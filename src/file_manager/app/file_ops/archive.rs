//! 檔案壓縮與解壓縮背景任務排程與結果定位。

use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::super::*;

impl App {
    /// 將目前選取或標記的項目壓成單一 zip 檔，並在完成後刷新所有 pane。
    pub(crate) fn compress_selected_entries(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to compress");
            return Ok(());
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to compress");
            return Ok(());
        }

        let target_dir = pane.cwd.clone();
        if entries_should_run_in_background(&entries) {
            return self.start_background_compress(self.focused_pane, target_dir, entries);
        }
        let archive_path = compress_entries_to_zip(&target_dir, &entries)?;
        self.reload_all_panes()?;
        let _ = self.reveal_path_and_track(self.focused_pane, &archive_path);

        self.status = if entries.len() == 1 {
            format!(
                "compressed {} -> {}",
                entries[0].display_name(),
                archive_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive.zip")
            )
        } else {
            format!(
                "compressed {} items -> {}",
                entries.len(),
                archive_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive.zip")
            )
        };
        Ok(())
    }

    /// 解開目前選取或標記的壓縮檔，並盡量把游標帶到第一個輸出結果。
    pub(crate) fn extract_selected_archives(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to extract");
            return Ok(());
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to extract");
            return Ok(());
        }

        let target_dir = pane.cwd.clone();
        if entries_should_run_in_background(&entries) {
            return self.start_background_extract(self.focused_pane, target_dir, entries);
        }
        let (extracted, skipped) = extract_entries(&target_dir, &entries)?;
        if extracted.is_empty() {
            self.status = if skipped == 0 {
                String::from("nothing selected to extract")
            } else {
                format!("no supported archives selected (skipped {skipped})")
            };
            return Ok(());
        }

        self.reload_all_panes()?;
        self.reveal_first_extracted_output(&extracted)?;

        self.status = extraction_status_label(&extracted, skipped);
        Ok(())
    }

    /// 把大型壓縮工作排入背景執行，避免 ZIP deflate 長時間占住 TUI 主執行緒。
    ///
    /// 參數：`pane_id: usize` 為來源 panel；`target_dir: PathBuf` 為輸出目錄；
    /// `entries: Vec<FileEntry>` 為本次要壓縮的項目。
    /// 回傳：`io::Result<()>`；成功代表工作已排入 task manager。
    pub(crate) fn start_background_compress(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        entries: Vec<crate::file_manager::entry::FileEntry>,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let first_name = entries
            .first()
            .map(|entry| entry.display_name())
            .unwrap_or_else(|| String::from("item"));
        let task_id = self.push_task(
            pane_id,
            "compress",
            format!("compress {entry_count} item(s)"),
            format!("output: {}", target_dir.display()),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let busy_paths = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        self.active_file_job_busy_paths.insert(task_id, busy_paths);
        let (sender, receiver) = std::sync::mpsc::channel();
        let total_bytes = entries
            .iter()
            .map(|entry| entry.size)
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes);
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result =
                compress_entries_to_zip_with_progress(&target_dir, &entries, &mut progress);
            let _ = sender.send(FileJobEvent::Compress {
                task_id,
                pane_id,
                entry_count,
                first_name,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("compressing {entry_count} item(s) in background [task {task_id}]");
        Ok(())
    }

    /// 把大型解壓工作排入背景執行，讓使用者可在其他 panel 繼續操作。
    ///
    /// 參數：`pane_id: usize` 為來源 panel；`target_dir: PathBuf` 為輸出目錄；
    /// `entries: Vec<FileEntry>` 為選取的壓縮檔。
    /// 回傳：`io::Result<()>`；成功代表工作已排入 task manager。
    pub(crate) fn start_background_extract(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        entries: Vec<crate::file_manager::entry::FileEntry>,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let task_id = self.push_task(
            pane_id,
            "extract",
            format!("extract {entry_count} item(s)"),
            format!("output: {}", target_dir.display()),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let mut busy_paths = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        for entry in &entries {
            if let Some(format) = detect_archive_format(&entry.path) {
                busy_paths.push(default_extract_output_path(
                    &target_dir,
                    &entry.path,
                    format,
                ));
            }
        }
        self.active_file_job_busy_paths.insert(task_id, busy_paths);
        let (sender, receiver) = mpsc::channel();
        // 解壓後資料通常比壓縮檔大，因此這是估算分母；完成事件會校正成最終 byte。
        let total_bytes = entries
            .iter()
            .map(|entry| entry.size)
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes);
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result = extract_entries_with_progress(&target_dir, &entries, &mut progress);
            let _ = sender.send(FileJobEvent::Extract {
                task_id,
                pane_id,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("extracting {entry_count} item(s) in background [task {task_id}]");
        Ok(())
    }

    /// 將目前焦點 pane 的游標帶到第一個解壓結果，方便使用者立刻繼續操作。
    pub(crate) fn reveal_first_extracted_output(
        &mut self,
        extracted: &[ExtractedArchive],
    ) -> io::Result<()> {
        let Some(first) = extracted.first() else {
            return Ok(());
        };
        let _ = self.reveal_path_and_track(self.focused_pane, &first.output_path);
        Ok(())
    }
}

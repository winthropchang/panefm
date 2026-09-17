//! 預覽核心型別、快取容器與非同步載入器定義。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ratatui::text::Line;

/// EasyMotion 全螢幕標籤優先使用的快速字鍵序列（Home-row 優先，其後為數字與大寫）。
pub(crate) const EASYMOTION_KEYS: &[char] = &[
    'j', 'k', 'd', 'f', 's', 'l', 'a', 'h', 'g', 'e', 'w', 'r', 'u', 'i', 'o', 'p', 'c', 'v', 'b',
    'n', 'm', 'x', 'z', 'q', 'y', 't', '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', 'J', 'K',
    'D', 'F', 'S', 'L', 'A', 'H', 'G', 'E', 'W', 'R', 'U', 'I', 'O', 'P',
];

/// 圖片解碼成功時回傳的縮圖列與原圖解析度。
pub type ImageDecodeResult = Result<(Vec<Line<'static>>, (u32, u32)), String>;

/// 圖片預覽快取，避免每一幀重新讀取磁碟與解碼像素。
#[derive(Clone, Debug)]
pub struct ImagePreviewCache {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub max_cols: usize,
    pub max_rows: usize,
    pub dimensions: Option<(u32, u32)>,
    pub lines: Vec<Line<'static>>,
}

/// 快取檔案與程式碼預覽行清單，避免每幀或 j/k 捲動時重複讀檔與語法解析。
#[derive(Clone, Debug)]
pub(crate) struct PreviewContentCache {
    pub(crate) path: PathBuf,
    pub(crate) modified: Option<SystemTime>,
    pub(crate) viewport_width: usize,
    pub(crate) total_lines: usize,
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) is_complete: bool,
}

/// 支援多項目 LRU 與鄰近預熱的檔案預覽快取容器。
#[derive(Clone, Debug)]
pub(crate) struct PreviewContentCacheMap {
    pub(crate) current: Option<PreviewContentCache>,
    pub(crate) entries: Vec<PreviewContentCache>,
    pub(crate) in_flight: std::collections::HashSet<PathBuf>,
    pub(crate) capacity: usize,
}

impl Default for PreviewContentCacheMap {
    fn default() -> Self {
        Self::new(32)
    }
}

impl PreviewContentCacheMap {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            current: None,
            entries: Vec::with_capacity(capacity.max(8)),
            in_flight: std::collections::HashSet::new(),
            capacity: capacity.max(8),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_some(&self) -> bool {
        self.current.is_some() || !self.entries.is_empty()
    }

    #[allow(dead_code)]
    pub(crate) fn as_ref(&self) -> Option<&PreviewContentCache> {
        self.current.as_ref().or_else(|| self.entries.first())
    }

    #[allow(dead_code)]
    pub(crate) fn as_mut(&mut self) -> Option<&mut PreviewContentCache> {
        self.current.as_mut().or_else(|| self.entries.first_mut())
    }

    #[allow(dead_code)]
    pub(crate) fn is_in_flight(&self, path: &Path) -> bool {
        self.in_flight.contains(path)
    }

    pub(crate) fn mark_in_flight(&mut self, path: PathBuf) {
        self.in_flight.insert(path);
    }

    pub(crate) fn clear_in_flight(&mut self, path: &Path) {
        self.in_flight.remove(path);
    }

    pub(crate) fn is_in_flight_or_cached(
        &self,
        path: &Path,
        modified: Option<SystemTime>,
        viewport_width: usize,
    ) -> bool {
        self.in_flight.contains(path) || self.contains(path, modified, viewport_width)
    }

    pub(crate) fn get(
        &self,
        path: &Path,
        modified: Option<SystemTime>,
        viewport_width: usize,
    ) -> Option<&PreviewContentCache> {
        if let Some(cur) = self.current.as_ref()
            && cur.path == path
            && (modified.is_none() || cur.modified == modified)
            && (cur.viewport_width == viewport_width || !path.is_dir())
        {
            return Some(cur);
        }
        self.entries.iter().find(|c| {
            c.path == path
                && (modified.is_none() || c.modified == modified)
                && (c.viewport_width == viewport_width || !path.is_dir())
        })
    }

    #[allow(dead_code)]
    pub(crate) fn get_by_path(&self, path: &Path) -> Option<&PreviewContentCache> {
        if let Some(cur) = self.current.as_ref()
            && cur.path == path
        {
            return Some(cur);
        }
        self.entries.iter().find(|c| c.path == path)
    }

    pub(crate) fn contains(
        &self,
        path: &Path,
        modified: Option<SystemTime>,
        viewport_width: usize,
    ) -> bool {
        self.get(path, modified, viewport_width).is_some()
    }

    pub(crate) fn put(&mut self, cache: PreviewContentCache) {
        self.in_flight.remove(&cache.path);
        if let Some(pos) = self.entries.iter().position(|c| c.path == cache.path) {
            self.entries.remove(pos);
        }
        self.entries.insert(0, cache.clone());
        if self.entries.len() > self.capacity {
            self.entries.pop();
        }
        // 若 current 尚未設定或傳入項目即為目前 current，才同步更新 current
        if self.current.as_ref().is_none_or(|c| c.path == cache.path) {
            self.current = Some(cache);
        }
    }

    pub(crate) fn put_current(&mut self, cache: PreviewContentCache) {
        self.in_flight.remove(&cache.path);
        if let Some(pos) = self.entries.iter().position(|c| c.path == cache.path) {
            self.entries.remove(pos);
        }
        self.entries.insert(0, cache.clone());
        if self.entries.len() > self.capacity {
            self.entries.pop();
        }
        self.current = Some(cache);
    }

    pub(crate) fn update_complete(
        &mut self,
        path: &Path,
        modified: Option<SystemTime>,
        full_lines: Vec<Line<'static>>,
    ) {
        let full_count = full_lines.len();
        if let Some(cur) = self.current.as_mut()
            && cur.path == path
            && (modified.is_none() || cur.modified == modified)
        {
            cur.total_lines = full_count.max(cur.total_lines);
            cur.lines = full_lines.clone();
            cur.is_complete = true;
        }
        if let Some(c) = self
            .entries
            .iter_mut()
            .find(|c| c.path == path && (modified.is_none() || c.modified == modified))
        {
            c.total_lines = full_count.max(c.total_lines);
            c.lines = full_lines;
            c.is_complete = true;
        }
    }
}

/// 背景非同步圖片解碼任務。
pub struct ImageLoader {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub max_cols: usize,
    pub max_rows: usize,
    pub receiver: std::sync::mpsc::Receiver<ImageDecodeResult>,
}

impl std::fmt::Debug for ImageLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageLoader")
            .field("path", &self.path)
            .field("modified", &self.modified)
            .field("max_cols", &self.max_cols)
            .field("max_rows", &self.max_rows)
            .finish_non_exhaustive()
    }
}

use std::{
    ffi::OsStr,
    fs, io,
    path::{Component, Path, PathBuf},
};

use super::PaneState;

impl PaneState {
    /// 依據輸入在目前目錄建立檔案或目錄。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要建立項目的目標 pane。
    /// - `input: &str`，使用者輸入的新檔案名稱或路徑。以 `/` 結尾代表建立目錄。
    ///
    /// 回傳：`io::Result<String>`。
    /// - 成功時回傳新項目的顯示名稱。
    /// - 失敗時回傳名稱無效、已存在或建立失敗的錯誤。
    pub(crate) fn create_entry(&mut self, input: &str) -> io::Result<String> {
        let request = parse_create_input(input)?;
        let new_path = self.cwd.join(&request.relative_path);

        if request.is_directory {
            fs::create_dir_all(
                new_path
                    .parent()
                    .filter(|parent| *parent != self.cwd.as_path())
                    .unwrap_or(&self.cwd),
            )?;
            fs::create_dir(&new_path)?;
        } else {
            if let Some(parent) = new_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&new_path)?;
        }

        self.reload()?;
        let focus_path = if request.relative_path.components().count() > 1 {
            match request.relative_path.components().next() {
                Some(Component::Normal(first)) => self.cwd.join(first),
                _ => new_path.clone(),
            }
        } else {
            new_path.clone()
        };
        self.select_path(&focus_path);

        Ok(request.display_name)
    }
}

pub(crate) fn unique_target_path(target_dir: &Path, original_name: &OsStr) -> PathBuf {
    let original_name = original_name.to_string_lossy();
    let initial_candidate = target_dir.join(original_name.as_ref());
    if !initial_candidate.exists() {
        return initial_candidate;
    }

    let (base_name, extension) = split_name_for_duplicate(&original_name);
    let mut duplicate_index = 1usize;

    loop {
        let candidate_name = if duplicate_index == 1 {
            duplicate_name(&base_name, extension.as_deref(), None)
        } else {
            duplicate_name(&base_name, extension.as_deref(), Some(duplicate_index))
        };
        let candidate_path = target_dir.join(candidate_name);
        if !candidate_path.exists() {
            return candidate_path;
        }
        duplicate_index += 1;
    }
}

/// 將檔名拆成「主名稱」與「副檔名」，方便產生重複貼上的新名稱。
pub(crate) fn split_name_for_duplicate(name: &str) -> (String, Option<String>) {
    if name.starts_with('.') && !name[1..].contains('.') {
        return (name.to_string(), None);
    }

    match name.rsplit_once('.') {
        Some((base_name, extension)) if !base_name.is_empty() => {
            (base_name.to_string(), Some(extension.to_string()))
        }
        _ => (name.to_string(), None),
    }
}

/// 依照主名稱、副檔名與重複次數產生新的不衝突檔名。
pub(crate) fn duplicate_name(
    base_name: &str,
    extension: Option<&str>,
    duplicate_index: Option<usize>,
) -> String {
    let mut candidate = String::from(base_name);
    candidate.push_str(" copy");

    if let Some(index) = duplicate_index {
        candidate.push(' ');
        candidate.push_str(&index.to_string());
    }

    if let Some(extension) = extension {
        candidate.push('.');
        candidate.push_str(extension);
    }

    candidate
}

/// 取出路徑最後一段作為顯示名稱，若缺少檔名則回傳空字串。
pub(crate) fn target_path_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 驗證新建立項目的名稱是否可用，避免空白名稱或直接包含路徑分隔符。
pub(crate) fn validate_new_entry_name(name: &str) -> io::Result<&str> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "name cannot be empty",
        ));
    }

    Ok(trimmed_name)
}

/// 描述一次建立請求解析後的結果。
struct CreateRequest {
    relative_path: PathBuf,
    display_name: String,
    is_directory: bool,
}

/// 解析建立輸入，決定要建立檔案還是資料夾，並驗證路徑是否安全。
fn parse_create_input(input: &str) -> io::Result<CreateRequest> {
    let trimmed = validate_new_entry_name(input)?;
    let is_directory = trimmed.ends_with('/');
    let normalized = trimmed.trim_end_matches('/');
    if normalized.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "name cannot be empty",
        ));
    }

    let relative_path = PathBuf::from(normalized);
    for component in relative_path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must stay inside current directory",
                ));
            }
        }
    }

    let display_name = if is_directory {
        format!("{normalized}/")
    } else {
        normalized.to_string()
    };

    Ok(CreateRequest {
        relative_path,
        display_name,
        is_directory,
    })
}

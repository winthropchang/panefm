use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    thread,
};

use super::types::SortMode;
use crate::file_manager::entry::FileEntry;

/// 依照指定的排序模式與種子對項目清單進行就地排序（可在背景執行緒執行）。
pub(crate) fn sort_file_entries(entries: &mut [FileEntry], sort_mode: SortMode, random_seed: u64) {
    if entries.len() <= 1 {
        return;
    }

    let comparator = |left: &FileEntry, right: &FileEntry| {
        if matches!(sort_mode, SortMode::Random) {
            match (left.is_dir, right.is_dir) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => random_key(left, random_seed).cmp(&random_key(right, random_seed)),
            }
        } else {
            compare_entries(left, right, sort_mode)
        }
    };

    let worker_count = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);

    if entries.len() < 1000 || worker_count <= 1 {
        entries.sort_unstable_by(comparator);
        return;
    }

    let chunk_size = entries.len().div_ceil(worker_count);
    thread::scope(|scope| {
        for chunk in entries.chunks_mut(chunk_size) {
            scope.spawn(move || {
                chunk.sort_unstable_by(comparator);
            });
        }
    });

    // 多路合併（Pairwise Merge）
    let mut step = chunk_size;
    let mut scratch = Vec::with_capacity(entries.len());
    while step < entries.len() {
        scratch.clear();
        let mut i = 0;
        while i < entries.len() {
            let mid = (i + step).min(entries.len());
            let end = (i + 2 * step).min(entries.len());
            if mid < end {
                let left_slice = &entries[i..mid];
                let right_slice = &entries[mid..end];
                let mut l = 0;
                let mut r = 0;
                while l < left_slice.len() && r < right_slice.len() {
                    if comparator(&left_slice[l], &right_slice[r]) != Ordering::Greater {
                        scratch.push(left_slice[l].clone());
                        l += 1;
                    } else {
                        scratch.push(right_slice[r].clone());
                        r += 1;
                    }
                }
                scratch.extend_from_slice(&left_slice[l..]);
                scratch.extend_from_slice(&right_slice[r..]);
            } else {
                scratch.extend_from_slice(&entries[i..mid]);
            }
            i = end;
        }
        entries.clone_from_slice(&scratch);
        step *= 2;
    }
}

/// 判斷檔名是否屬於隱藏檔或隱藏資料夾。
pub(crate) fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// 依照 pane 目前設定的排序模式重排清單，並維持資料夾優先。
pub(crate) fn compare_entries(
    left: &FileEntry,
    right: &FileEntry,
    sort_mode: SortMode,
) -> Ordering {
    match (left.is_dir, right.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => {
            let primary = match sort_mode {
                SortMode::Alphabetical { reverse } => compare_with_reverse(
                    compare_ascii_case_insensitive(&left.name, &right.name),
                    reverse,
                ),
                SortMode::Natural { reverse } => {
                    compare_with_reverse(natural_cmp(&left.name, &right.name), reverse)
                }
                SortMode::Size { reverse } => {
                    compare_with_reverse(left.size.cmp(&right.size), reverse)
                }
                SortMode::Modified { reverse } => {
                    compare_with_reverse(left.modified.cmp(&right.modified), reverse)
                }
                SortMode::Created { reverse } => {
                    compare_with_reverse(left.created.cmp(&right.created), reverse)
                }
                SortMode::Extension { reverse } => compare_with_reverse(
                    file_extension(left)
                        .cmp(&file_extension(right))
                        .then_with(|| compare_ascii_case_insensitive(&left.name, &right.name)),
                    reverse,
                ),
                SortMode::Random => Ordering::Equal,
            };

            if primary == Ordering::Equal {
                left.name.cmp(&right.name)
            } else {
                primary
            }
        }
    }
}

/// 零記憶體配置的 ASCII 不分大小寫字串比較。
pub(crate) fn compare_ascii_case_insensitive(left: &str, right: &str) -> Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let mut i = 0;
    while i < left_bytes.len() && i < right_bytes.len() {
        let bl = left_bytes[i].to_ascii_lowercase();
        let br = right_bytes[i].to_ascii_lowercase();
        if bl != br {
            return bl.cmp(&br);
        }
        i += 1;
    }
    left_bytes.len().cmp(&right_bytes.len())
}

/// 依照 reverse 旗標決定是否翻轉比較結果。
pub(crate) fn compare_with_reverse(ordering: Ordering, reverse: bool) -> Ordering {
    if reverse {
        ordering.reverse()
    } else {
        ordering
    }
}

/// 取出檔案副檔名作為小寫字串，資料夾則回傳空字串。
pub(crate) fn file_extension(entry: &FileEntry) -> String {
    if entry.is_dir {
        String::new()
    } else {
        entry
            .path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }
}

/// 用比較接近自然排序的方式比較兩個名稱，讓數字片段能按數值排序。
pub(crate) fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let (mut left_index, mut right_index) = (0usize, 0usize);

    while left_index < left_bytes.len() && right_index < right_bytes.len() {
        if left_bytes[left_index].is_ascii_digit() && right_bytes[right_index].is_ascii_digit() {
            let left_end = ascii_digit_run_end(left_bytes, left_index);
            let right_end = ascii_digit_run_end(right_bytes, right_index);
            let ordering = compare_ascii_number_runs(
                &left_bytes[left_index..left_end],
                &right_bytes[right_index..right_end],
            );
            if ordering != Ordering::Equal {
                return ordering;
            }
            left_index = left_end;
            right_index = right_end;
            continue;
        }

        // Fast path for ASCII characters (avoids Unicode lookup overhead)
        if left_bytes[left_index].is_ascii() && right_bytes[right_index].is_ascii() {
            let c_left = left_bytes[left_index].to_ascii_lowercase();
            let c_right = right_bytes[right_index].to_ascii_lowercase();
            if c_left != c_right {
                return c_left.cmp(&c_right);
            }
            left_index += 1;
            right_index += 1;
            continue;
        }

        let left_character = left[left_index..]
            .chars()
            .next()
            .expect("index is inside UTF-8 string");
        let right_character = right[right_index..]
            .chars()
            .next()
            .expect("index is inside UTF-8 string");
        let ordering = left_character
            .to_lowercase()
            .cmp(right_character.to_lowercase());
        if ordering != Ordering::Equal {
            return ordering;
        }
        left_index += left_character.len_utf8();
        right_index += right_character.len_utf8();
    }

    (left_bytes.len() - left_index).cmp(&(right_bytes.len() - right_index))
}

/// 找出 ASCII 數字片段的尾端索引，過程不建立暫存字串。
fn ascii_digit_run_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    end
}

/// 比較兩段任意長度的 ASCII 數字，不轉成整數，也不配置 heap 記憶體。
fn compare_ascii_number_runs(left: &[u8], right: &[u8]) -> Ordering {
    let left_trimmed = left
        .iter()
        .position(|byte| *byte != b'0')
        .map(|index| &left[index..])
        .unwrap_or(&[]);
    let right_trimmed = right
        .iter()
        .position(|byte| *byte != b'0')
        .map(|index| &right[index..])
        .unwrap_or(&[]);

    left_trimmed
        .len()
        .cmp(&right_trimmed.len())
        .then_with(|| left_trimmed.cmp(right_trimmed))
        .then_with(|| left.len().cmp(&right.len()))
}

/// 根據路徑內容產生一個基本種子，供隨機排序使用。
pub(crate) fn seed_from_path(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// 使用隨機排序時，依照目前種子為每個項目產生穩定但可變動的排序鍵。
pub(crate) fn random_key(entry: &FileEntry, seed: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    entry.path.hash(&mut hasher);
    hasher.finish()
}

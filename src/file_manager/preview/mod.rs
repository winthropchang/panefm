//! 終端檔案預覽模組：提供純 Rust 原生 Halfblock 圖片預覽、Syntect 語法高亮、壓縮檔樹與詳細資訊卡片。

pub(crate) mod archive;
pub(crate) mod card;
pub(crate) mod content;
pub(crate) mod directory;
pub(crate) mod halfblock;
pub(crate) mod markdown;
pub(crate) mod syntax;
pub(crate) mod types;

pub use archive::*;
pub use card::*;
pub(crate) use content::*;
pub use directory::*;
pub use halfblock::*;
pub use markdown::*;
pub use syntax::*;
pub use types::*;

#[allow(unused_imports)]
pub(crate) use std::path::Path;

use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

/// 全域延遲初始化的語法庫與主題庫，只有在首次預覽程式碼時載入，平日操作 0 延遲。
pub(crate) static SYNTAX_SET: std::sync::LazyLock<SyntaxSet> =
    std::sync::LazyLock::new(SyntaxSet::load_defaults_newlines);

pub(crate) static THEME_SET: std::sync::LazyLock<ThemeSet> =
    std::sync::LazyLock::new(ThemeSet::load_defaults);

/// 支援終端 Halfblock 預覽的單一圖片大小上限（30 MiB）。
pub(crate) const MAX_IMAGE_PREVIEW_SIZE: u64 = 30 * 1024 * 1024;

/// 一般文字檔案直接載入內容預覽的大小上限（2 MiB）。
pub(crate) const MAX_TEXT_PREVIEW_SIZE: u64 = 2 * 1024 * 1024;

#[cfg(test)]
#[path = "../tests/preview_test.rs"]
mod tests;

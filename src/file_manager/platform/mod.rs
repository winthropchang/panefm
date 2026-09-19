//! macOS、Windows 與未來 Linux 的系統整合命令邊界。
//!
//! Reveal、系統開啟、SMB 掛載等平台差異應集中在此層，其他模組只使用 `LaunchSpec`
//! 或抽象函數。這可讓命令建構在目前平台以單元測試驗證另一平台，而不必真的啟動。

pub(crate) mod clipboard;
pub(crate) mod process;
pub(crate) mod system;
pub(crate) mod terminal;

pub(crate) use clipboard::*;
pub(crate) use process::*;
pub(crate) use system::*;
pub(crate) use terminal::*;

/// 描述目前平台命令要針對哪一種作業系統產生。
///
/// 目前正式支援目標是 `Windows` 與 `MacOs`；
/// `LinuxLike` 先保留做為未來擴充點，避免之後要補 Unix / Linux 時重寫整層抽象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PlatformKind {
    Windows,
    MacOs,
    LinuxLike,
}

/// 回傳目前執行中的平台種類，供外部開啟、Reveal 等功能分流使用。
pub(crate) fn current_platform() -> PlatformKind {
    #[cfg(target_os = "windows")]
    {
        PlatformKind::Windows
    }

    #[cfg(target_os = "macos")]
    {
        PlatformKind::MacOs
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        PlatformKind::LinuxLike
    }
}

#[cfg(test)]
#[path = "../tests/platform_test.rs"]
mod tests;

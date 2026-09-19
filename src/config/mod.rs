//! 設定檔管理模組（載入、解析、外掛、範本、持久化）。

pub(crate) mod apply;
pub(crate) mod file_types;
pub(crate) mod paths;
pub(crate) mod persist;
pub(crate) mod plugins;
pub(crate) mod schema;
pub(crate) mod template;

pub use apply::load_config;
pub use paths::app_config_file;
pub use persist::persist_theme;
pub use schema::*;
pub use template::{DEFAULT_CONFIG_TEMPLATE, ensure_default_config_file};

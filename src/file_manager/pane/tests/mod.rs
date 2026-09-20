pub(crate) use super::*;
pub(crate) use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) use tempfile::tempdir;

pub(crate) use crate::file_manager::entry::FileEntry;
pub(crate) use crate::file_manager::search::GlobalSearchEntry;
pub(crate) use crate::theme::Theme;
pub(crate) use ratatui::style::Modifier;

mod copy;
mod navigation;
mod preview;
mod search_and_diff;
mod sort;

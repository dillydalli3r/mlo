//! mlo — native Rust music library engine + TUI/CLI.
//! See `docs/DEPENDENCIES.md` for the crate audit and `README.md` for install.

pub mod analysis;
pub mod atomic;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod grade;
pub mod images;
pub mod import;
pub mod jobs;
pub mod layout;
pub mod logging;
pub mod model;
pub mod naming;
pub mod net;
pub mod player;
pub mod scan;
pub mod scripts;
pub mod shellmenu;
pub mod tagkey;
pub mod tags;
pub mod trash;
pub mod tui;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

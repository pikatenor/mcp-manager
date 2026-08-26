//! Desktop shell. Business rules stay in the workspace crates.

pub mod session;
pub mod shell;

mod app;
#[cfg(target_os = "macos")]
mod tray;
mod ui;

pub use app::run;

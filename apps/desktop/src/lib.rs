//! Desktop shell. Business rules stay in the workspace crates.

pub mod session;
pub mod shell;

mod app;
#[cfg(target_os = "macos")]
mod tray;

pub use app::run;

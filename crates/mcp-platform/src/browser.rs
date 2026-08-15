use std::process::Command;

/// Opens an http(s) URL in the user's default browser (OAuth authorize).
pub trait BrowserOpener: Send + Sync {
    fn open_url(&self, url: &str) -> Result<(), BrowserError>;
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("failed to open url: {0}")]
    OpenFailed(String),
    #[error("refused url: {0}")]
    Refused(String),
}

pub struct NativeBrowserOpener;

impl BrowserOpener for NativeBrowserOpener {
    fn open_url(&self, url: &str) -> Result<(), BrowserError> {
        if !(url.starts_with("https://")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("http://localhost"))
        {
            return Err(BrowserError::Refused(url.to_string()));
        }
        let status = if cfg!(target_os = "macos") {
            Command::new("open").arg(url).status()
        } else if cfg!(target_os = "linux") {
            Command::new("xdg-open").arg(url).status()
        } else {
            return Err(BrowserError::OpenFailed("unsupported platform".into()));
        }
        .map_err(|e| BrowserError::OpenFailed(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(BrowserError::OpenFailed(format!("exit {status}")))
        }
    }
}

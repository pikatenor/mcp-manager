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

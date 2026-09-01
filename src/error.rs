use std::path::PathBuf;

use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("WinTerminal++ only supports Windows")]
    UnsupportedPlatform,

    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Windows Terminal is not installed or wt.exe is unavailable")]
    TerminalNotInstalled,

    #[error("another WinTerminal++ controller instance is already running")]
    ControllerAlreadyRunning,

    #[error("the controller must run elevated to control an elevated Windows Terminal")]
    ControllerRequiresElevation,

    #[error("Windows Terminal settings conflict: {0}")]
    SettingsConflict(String),

    #[error("Windows Terminal settings error at {path}: {message}")]
    Settings { path: PathBuf, message: String },

    #[error("native Windows operation failed: {0}")]
    Native(String),

    #[error("I/O operation {operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl AppError {
    #[must_use]
    pub fn io(operation: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}

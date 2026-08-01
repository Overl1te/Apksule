use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompatError {
    #[error("APPDATA and LOCALAPPDATA are not available")]
    MissingAppData,
    #[error("invalid Android package name: {0}")]
    InvalidPackageName(String),
    #[error("path must be relative and cannot contain parent traversal: {0}")]
    InvalidRelativePath(PathBuf),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("resource source rejected {path}: {message}")]
    Resource { path: String, message: String },
    #[error("unsupported API log is unavailable because its lock was poisoned")]
    ApiLogPoisoned,
}

pub type Result<T> = std::result::Result<T, CompatError>;

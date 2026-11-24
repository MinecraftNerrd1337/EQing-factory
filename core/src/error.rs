//! Errors returned to the UI (they become Swift `Error`s through uniFFI).

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("not found: {what}")]
    NotFound { what: String },
    #[error("the file no longer exists at this location: {path}")]
    FileMissing { path: String },
    #[error("{detail}")]
    Io { detail: String },
    #[error("{detail}")]
    Parse { detail: String },
    #[error("{detail}")]
    Invalid { detail: String },
    #[error("{detail}")]
    Engine { detail: String },
}

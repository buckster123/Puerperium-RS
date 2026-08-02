//! Error type for the core library.
//!
//! `thiserror` in the library, `anyhow` in binaries (house rule). Every variant names the
//! real cause — a failure that says only "error" costs the next session an hour.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("name is empty, hidden, or contains a path separator: {0:?}")]
    InvalidName(String),

    #[error("dataset already exists: {0} (datasets are immutable — pick another name)")]
    DatasetExists(PathBuf),

    #[error("no examples survived the quality gate ({rejected} rejected) — nothing was written")]
    NoExamples { rejected: usize },

    #[error("no record named {name:?} in {dir}")]
    RecordNotFound { dir: PathBuf, name: String },

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Attach a path to an io error, so the message names the file that actually failed.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

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

    #[error(
        "refusing to mine a live AGENTD_LOG ({0}) — copy the export first (D13 / docs/harvest.md)"
    )]
    LiveSessionsDir(PathBuf),

    #[error("no session-*.jsonl files in {0} — export with format jsonl first (docs/harvest.md)")]
    NoSessionFiles(PathBuf),

    #[error("{what} {name:?} already exists")]
    AlreadyExists { what: String, name: String },

    #[error("job {id} already exists ({reason}) — not resubmitted")]
    JobExists { id: String, reason: String },

    #[error(
        "training file {file_id} is not bound to a local dataset — \
         upload via `job upload` so the hash is pinned"
    )]
    UnboundTrainingFile { file_id: String },

    #[error("training file {file_id} was uploaded for dataset hash {bound}, not {requested}")]
    TrainingFileMismatch {
        file_id: String,
        bound: String,
        requested: String,
    },

    #[error("no record named {name:?} in {dir}")]
    RecordNotFound { dir: PathBuf, name: String },

    #[error("compute {requested:?} is not available (Puerperium never creates it — have: {})",         if available.is_empty() { "nothing".to_string() } else { available.join(", ") })]
    ComputeUnavailable {
        requested: String,
        available: Vec<String>,
    },

    #[error(
        "job {id} was recorded but the upstream never confirmed it ({reason}) — \
it may be running; check before resubmitting"
    )]
    SubmitUnconfirmed { id: String, reason: String },

    #[error("job {id} cannot be downloaded ({reason})")]
    NotDownloadable { id: String, reason: String },

    #[error("archive entry would escape the destination: {0}")]
    UnsafeArchiveEntry(String),

    #[error("provider refused: {0}")]
    ProviderRefused(String),

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

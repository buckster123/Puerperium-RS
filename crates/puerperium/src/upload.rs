//! Bind a Together `training_file_id` to the dataset that was actually uploaded.
//!
//! The job record names a dataset by hash. The upstream trains on whatever file id we send.
//! Those two must be the same bytes (after provider projection) or lineage would describe
//! a different run than the one that was paid for.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dataset::DatasetRef;
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::store;

/// A file we uploaded, pinned to the dataset it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileBinding {
    pub file_id: String,
    pub dataset: DatasetRef,
    pub projected_sha256: String,
    pub uploaded_at: DateTime<Utc>,
}

pub fn projected_sha256(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).as_slice())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn record_name(file_id: &str) -> String {
    match store::validate_name(file_id) {
        Ok(()) => file_id.to_string(),
        Err(_) => format!("sha-{}", projected_sha256(file_id.as_bytes())),
    }
}

pub fn save(paths: &Paths, binding: &FileBinding) -> Result<()> {
    store::save(&paths.uploads(), &record_name(&binding.file_id), binding)
}

/// Pin a freshly uploaded file to the dataset it was projected from.
pub fn bind(
    paths: &Paths,
    file_id: impl Into<String>,
    dataset: DatasetRef,
    projected: &[u8],
) -> Result<FileBinding> {
    let binding = FileBinding {
        file_id: file_id.into(),
        dataset,
        projected_sha256: projected_sha256(projected),
        uploaded_at: Utc::now(),
    };
    save(paths, &binding)?;
    Ok(binding)
}

pub fn load(paths: &Paths, file_id: &str) -> Result<FileBinding> {
    store::load(&paths.uploads(), &record_name(file_id))
}

/// Refuse a submit whose file id is unbound or was uploaded for a different dataset.
pub fn assert_bound(paths: &Paths, file_id: &str, dataset: &DatasetRef) -> Result<()> {
    let binding = match load(paths, file_id) {
        Ok(b) => b,
        Err(Error::RecordNotFound { .. }) => {
            return Err(Error::UnboundTrainingFile {
                file_id: file_id.to_string(),
            })
        }
        Err(e) => return Err(e),
    };
    if binding.dataset.sha256 != dataset.sha256 {
        return Err(Error::TrainingFileMismatch {
            file_id: file_id.to_string(),
            bound: binding.dataset.sha256,
            requested: dataset.sha256.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = Paths::new(dir.path());
        (dir, p)
    }

    fn dataset(hash: &str) -> DatasetRef {
        DatasetRef {
            name: "d".into(),
            sha256: hash.into(),
        }
    }

    fn binding(file_id: &str, hash: &str) -> FileBinding {
        FileBinding {
            file_id: file_id.into(),
            dataset: dataset(hash),
            projected_sha256: hash.into(),
            uploaded_at: Utc::now(),
        }
    }

    #[test]
    fn a_matching_bind_is_accepted() {
        let (_d, p) = paths();
        save(&p, &binding("file-abc", "aaa")).expect("save");
        assert_bound(&p, "file-abc", &dataset("aaa")).expect("match");
    }

    #[test]
    fn a_mismatched_hash_is_refused() {
        let (_d, p) = paths();
        save(&p, &binding("file-abc", "aaa")).expect("save");
        let err = assert_bound(&p, "file-abc", &dataset("bbb")).expect_err("mismatch");
        assert!(
            matches!(err, Error::TrainingFileMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn an_unbound_file_is_refused() {
        let (_d, p) = paths();
        let err = assert_bound(&p, "file-ghost", &dataset("aaa")).expect_err("unbound");
        assert!(
            matches!(err, Error::UnboundTrainingFile { .. }),
            "got {err:?}"
        );
    }
}

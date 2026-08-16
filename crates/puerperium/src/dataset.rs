//! Writing a dataset, and the metadata that makes it identifiable.
//!
//! Charter D12: every dataset is content-hashed. **The hash is the dataset's real identity** —
//! a name can be reused across a rebuild, a `sha256` cannot, so job records reference the
//! hash and lineage walks trust it.
//!
//! Datasets are immutable once written. Rebuilding under the same name is an error rather
//! than an overwrite, because a job record pointing at a silently-changed dataset is a
//! lineage lie.

use std::collections::BTreeMap;
use std::fs;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::convert::Converted;
use crate::error::{Error, Result};
use crate::store;

/// A handle to a written dataset. Carried by job and model records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRef {
    pub name: String,
    pub sha256: String,
}

/// What produced a dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpec {
    /// `"cerebro_query"`, `"export_file"`, `"synthetic"`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Whose memory space was mined — **not** the trainer (charter D6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// How many memories went in, before filtering.
    pub memories_in: usize,
}

/// The sidecar written next to every dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMeta {
    pub name: String,
    /// Hash of the JSONL bytes, hex-encoded.
    pub sha256: String,
    pub example_count: usize,
    pub memories_used: usize,
    /// Rejections by reason. Present even when empty — an absent field reads as "unknown",
    /// and the whole point is that the accounting is visible.
    pub rejections: BTreeMap<String, usize>,
    pub rejected_total: usize,
    /// Instruction framing split — `templated_heading` (strong) vs `templated_tag` (weak).
    /// Surfaced here so dataset quality is legible without re-reading the JSONL.
    ///
    /// `serde(default)`: a sidecar written before this field existed must still load. A
    /// dataset is a durable artifact — job records reference it by hash for the life of the
    /// registry, so *every* field added here has to be back-compatible or old lineage breaks.
    #[serde(default)]
    pub framing: BTreeMap<String, usize>,
    /// Per-chunk unframeable count. Distinct from `rejections.unframeable`, which
    /// is per-memory so the accounting stays total.
    #[serde(default)]
    pub unframeable_chunks: usize,
    pub source: SourceSpec,
    pub tool_version: String,
    pub created_at: DateTime<Utc>,
}

impl DatasetMeta {
    pub fn dataset_ref(&self) -> DatasetRef {
        DatasetRef {
            name: self.name.clone(),
            sha256: self.sha256.clone(),
        }
    }
}

pub fn jsonl_path(dir: &Path, name: &str) -> Result<PathBuf> {
    store::validate_name(name)?;
    Ok(dir.join(format!("{name}.jsonl")))
}

pub fn meta_path(dir: &Path, name: &str) -> Result<PathBuf> {
    store::validate_name(name)?;
    Ok(dir.join(format!("{name}.meta.json")))
}

/// Write a dataset and its sidecar. Returns the hash-bearing handle.
///
/// Fails rather than overwriting an existing dataset, and fails rather than writing an empty
/// one — an empty dataset is always a mistake, and the error carries the rejection count so
/// the cause is visible immediately.
pub fn write(
    dir: &Path,
    name: &str,
    converted: &Converted,
    source: SourceSpec,
) -> Result<DatasetMeta> {
    if converted.examples.is_empty() {
        return Err(Error::NoExamples {
            rejected: converted.rejections.total(),
        });
    }

    let data_path = jsonl_path(dir, name)?;
    if data_path.exists() {
        return Err(Error::DatasetExists(data_path));
    }

    store::ensure_dir(dir)?;

    // Build the whole body first so the hash covers exactly the bytes that land on disk.
    let mut body = String::new();
    for ex in &converted.examples {
        body.push_str(&ex.to_jsonl()?);
        body.push('\n');
    }

    let sha256 = hex(Sha256::digest(body.as_bytes()).as_slice());
    store::write_atomic(&data_path, body.as_bytes())?;

    let meta = DatasetMeta {
        name: name.to_string(),
        sha256,
        example_count: converted.examples.len(),
        memories_used: converted.memories_used,
        rejections: converted
            .rejections
            .counts()
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect(),
        rejected_total: converted.rejections.total(),
        framing: converted
            .framing
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), *v))
            .collect(),
        unframeable_chunks: converted.unframeable_chunks,
        source,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: Utc::now(),
    };

    let meta_json = serde_json::to_vec_pretty(&meta)?;
    store::write_atomic(&meta_path(dir, name)?, &meta_json)?;

    Ok(meta)
}

/// Load a sidecar.
///
/// A missing sidecar is [`Error::RecordNotFound`], not a raw io error — the same shape
/// `store::load` gives, so "there is no dataset called that" reads the same everywhere
/// instead of leaking an ENOENT chain at the caller.
pub fn read_meta(dir: &Path, name: &str) -> Result<DatasetMeta> {
    let p = meta_path(dir, name)?;
    let bytes = match fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::RecordNotFound {
                dir: dir.to_path_buf(),
                name: name.to_string(),
            })
        }
        Err(e) => return Err(Error::io(&p, e)),
    };
    Ok(serde_json::from_slice(&bytes)?)
}

/// Every dataset in `dir`, by sidecar, newest first. A missing directory is an empty list,
/// not an error — a fresh node has simply not made one yet.
pub fn list(dir: &Path) -> Result<Vec<DatasetMeta>> {
    let mut out: Vec<DatasetMeta> = store::list_with_suffix(dir, ".meta.json")?;
    out.sort_by_key(|m| std::cmp::Reverse(m.created_at));
    Ok(out)
}

/// Verify a dataset's bytes still hash to what its sidecar claims.
pub fn verify(dir: &Path, name: &str) -> Result<bool> {
    let meta = read_meta(dir, name)?;
    let p = jsonl_path(dir, name)?;
    let bytes = fs::read(&p).map_err(|e| Error::io(&p, e))?;
    Ok(hex(Sha256::digest(&bytes).as_slice()) == meta.sha256)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{convert, ConvertConfig};
    use crate::memory::{MemoryRecord, MemoryType};

    fn sample() -> Converted {
        let doc = "DEPLOY REFERENCE\n\n## Building\n\nAlways build on the target board; an x86 \
                   binary gives Exec format error, which reads like a corrupt file rather than \
                   a wrong architecture.\n";
        let m = MemoryRecord {
            id: "m1".into(),
            content: doc.into(),
            memory_type: MemoryType::Procedural,
            tags: vec!["deploy".into()],
            agent_id: Some("CLAUDE".into()),
            salience: 0.9,
        };
        convert(&[m], &ConvertConfig::new())
    }

    fn source() -> SourceSpec {
        SourceSpec {
            kind: "export_file".into(),
            query: None,
            agent_id: Some("CLAUDE".into()),
            memories_in: 1,
        }
    }

    #[test]
    fn writes_jsonl_and_sidecar_then_verifies() {
        let dir = tempfile::tempdir().expect("tempdir");
        let meta = write(dir.path(), "d1", &sample(), source()).expect("write");

        assert_eq!(meta.example_count, 1);
        assert_eq!(meta.memories_used, 1);
        assert_eq!(meta.sha256.len(), 64);
        assert!(verify(dir.path(), "d1").expect("verify"));

        let text = fs::read_to_string(jsonl_path(dir.path(), "d1").expect("path")).expect("read");
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with('\n'), "JSONL must be newline-terminated");
    }

    #[test]
    fn hash_is_deterministic_across_identical_content() {
        let a = tempfile::tempdir().expect("tempdir");
        let b = tempfile::tempdir().expect("tempdir");
        let m1 = write(a.path(), "d", &sample(), source()).expect("write a");
        let m2 = write(b.path(), "d", &sample(), source()).expect("write b");
        assert_eq!(m1.sha256, m2.sha256, "same examples must hash the same");
    }

    #[test]
    fn refuses_to_overwrite_an_existing_dataset() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "d1", &sample(), source()).expect("first write");
        let err = write(dir.path(), "d1", &sample(), source()).expect_err("must refuse");
        assert!(matches!(err, Error::DatasetExists(_)), "got {err:?}");
    }

    #[test]
    fn refuses_an_empty_dataset_and_says_how_many_were_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty = convert(
            &[MemoryRecord {
                id: "m".into(),
                content: "short".into(),
                memory_type: MemoryType::Semantic,
                tags: vec![],
                agent_id: None,
                salience: 0.9,
            }],
            &ConvertConfig::new(),
        );
        let err = write(dir.path(), "d", &empty, source()).expect_err("must refuse");
        assert!(
            matches!(err, Error::NoExamples { rejected: 1 }),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_names_that_would_escape_the_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        for bad in ["", "../evil", "a/b", ".hidden", "a\\b"] {
            let err = write(dir.path(), bad, &sample(), source()).expect_err("must reject");
            assert!(matches!(err, Error::InvalidName(_)), "{bad:?} gave {err:?}");
            assert!(
                matches!(read_meta(dir.path(), bad), Err(Error::InvalidName(_))),
                "reads must refuse {bad:?} before touching the filesystem"
            );
            assert!(
                matches!(jsonl_path(dir.path(), bad), Err(Error::InvalidName(_))),
                "path construction must refuse {bad:?}"
            );
        }
    }

    /// Regression: adding `framing` to the sidecar made every previously-written dataset
    /// unreadable — `puerperium data list` died with "missing field `framing`". Datasets are
    /// durable and referenced by hash for the life of the registry, so new metadata fields
    /// must always default.
    #[test]
    fn sidecar_written_before_a_field_existed_still_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = r#"{
            "name": "legacy",
            "sha256": "abc",
            "example_count": 3,
            "memories_used": 2,
            "rejections": {},
            "rejected_total": 0,
            "source": {"kind": "export_file", "memories_in": 5},
            "tool_version": "0.1.0",
            "created_at": "2026-08-02T00:00:00Z"
        }"#;
        fs::write(meta_path(dir.path(), "legacy").expect("path"), old)
            .expect("write legacy sidecar");

        let meta = read_meta(dir.path(), "legacy").expect("legacy sidecar must still load");
        assert_eq!(meta.example_count, 3);
        assert!(meta.framing.is_empty());
        assert_eq!(meta.unframeable_chunks, 0);
        assert_eq!(list(dir.path()).expect("list").len(), 1);
    }

    /// "There is no dataset called that" must read the same as it does for any other record,
    /// rather than leaking an ENOENT chain at whoever referenced it.
    #[test]
    fn missing_sidecar_is_record_not_found_not_a_raw_io_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = read_meta(dir.path(), "ghost").expect_err("must fail");
        assert!(matches!(err, Error::RecordNotFound { .. }), "got {err:?}");
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn list_is_empty_for_a_missing_directory_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let got = list(&dir.path().join("nope")).expect("must not error");
        assert!(got.is_empty());
    }

    #[test]
    fn meta_records_rejections_even_when_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let meta = write(dir.path(), "d1", &sample(), source()).expect("write");
        assert_eq!(meta.rejected_total, 0);
        let json = serde_json::to_string(&meta).expect("ser");
        assert!(
            json.contains("\"rejections\""),
            "field must be present even when empty"
        );
    }
}

//! Durable JSON records on disk.
//!
//! The shared mechanics behind datasets, models and apprentices: name validation, atomic
//! writes, and listing. Extracted so the three do not drift — the dataset writer grew these
//! first and the registry needs exactly the same guarantees.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{Error, Result};

/// Reject names that would escape their directory or collide with a sidecar suffix.
///
/// Records are addressed by *name*, never by a caller-supplied path — the same discipline
/// Prefrontal-RS applies to project-relative paths. A name with a separator, a `..`, a
/// leading dot or a control character is refused outright.
pub fn validate_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
        || name.chars().any(|c| c.is_control());
    if bad {
        return Err(Error::InvalidName(name.to_string()));
    }
    Ok(())
}

/// `tmp → fsync → rename`. A reader never sees a half-written record.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        f.write_all(bytes).map_err(|e| Error::io(&tmp, e))?;
        f.sync_all().map_err(|e| Error::io(&tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

pub fn record_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// Write a record, overwriting any existing one.
///
/// Unlike datasets (immutable — their hash is their identity), registry records are
/// **mutable**: an apprentice gains its model when training finishes, a model gains its
/// artifact path. Updating in place is the normal case, not an error.
pub fn save<T: Serialize>(dir: &Path, name: &str, record: &T) -> Result<()> {
    validate_name(name)?;
    let bytes = serde_json::to_vec_pretty(record)?;
    write_atomic(&record_path(dir, name), &bytes)
}

/// Load a record by name.
pub fn load<T: DeserializeOwned>(dir: &Path, name: &str) -> Result<T> {
    validate_name(name)?;
    let p = record_path(dir, name);
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

/// Is there a record under this name?
pub fn exists(dir: &Path, name: &str) -> bool {
    validate_name(name).is_ok() && record_path(dir, name).exists()
}

/// Every record in `dir` whose filename ends with `suffix`.
///
/// A missing directory is an **empty list, not an error** — a fresh node has simply not made
/// one yet, and an error there would make every `list` verb fail on first run.
pub fn list_with_suffix<T: DeserializeOwned>(dir: &Path, suffix: &str) -> Result<Vec<T>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(dir, e)),
    };

    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !fname.ends_with(suffix) {
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| Error::io(&path, e))?;
        out.push(serde_json::from_slice(&bytes)?);
    }
    Ok(out)
}

/// Every `*.json` record in `dir`.
pub fn list<T: DeserializeOwned>(dir: &Path) -> Result<Vec<T>> {
    list_with_suffix(dir, ".json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Rec {
        name: String,
        n: u32,
    }

    #[test]
    fn save_load_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let r = Rec {
            name: "a".into(),
            n: 7,
        };
        save(dir.path(), "a", &r).expect("save");
        let back: Rec = load(dir.path(), "a").expect("load");
        assert_eq!(back, r);
        assert!(exists(dir.path(), "a"));
        assert!(!exists(dir.path(), "nope"));
    }

    #[test]
    fn save_overwrites_because_registry_records_are_mutable() {
        let dir = tempfile::tempdir().expect("tempdir");
        save(
            dir.path(),
            "a",
            &Rec {
                name: "a".into(),
                n: 1,
            },
        )
        .expect("first");
        save(
            dir.path(),
            "a",
            &Rec {
                name: "a".into(),
                n: 2,
            },
        )
        .expect("second must not error");
        let back: Rec = load(dir.path(), "a").expect("load");
        assert_eq!(back.n, 2);
    }

    #[test]
    fn missing_record_is_a_named_error_not_a_bare_io_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = load::<Rec>(dir.path(), "ghost").expect_err("must fail");
        assert!(matches!(err, Error::RecordNotFound { .. }), "got {err:?}");
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn names_that_escape_the_directory_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        for bad in ["", "../evil", "a/b", ".hidden", "a\\b"] {
            let r = Rec {
                name: "x".into(),
                n: 1,
            };
            assert!(
                matches!(save(dir.path(), bad, &r), Err(Error::InvalidName(_))),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn list_is_empty_for_a_missing_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let got: Vec<Rec> = list(&dir.path().join("nope")).expect("must not error");
        assert!(got.is_empty());
    }

    #[test]
    fn list_ignores_non_matching_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        save(
            dir.path(),
            "a",
            &Rec {
                name: "a".into(),
                n: 1,
            },
        )
        .expect("save");
        fs::write(dir.path().join("notes.txt"), "ignore me").expect("write");
        fs::write(dir.path().join("a.jsonl"), "ignore me too").expect("write");
        let got: Vec<Rec> = list(dir.path()).expect("list");
        assert_eq!(got.len(), 1);
    }
}

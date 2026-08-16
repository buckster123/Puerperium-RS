//! The state directory layout.
//!
//! One place that knows where things live, so the library and every face agree. **Nothing is
//! ever written into a repo directory** — state lives under `$PUERPERIUM_STATE_DIR`, else
//! `~/.local/share/puerperium`.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::store;

/// Resolved state-directory layout.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$PUERPERIUM_STATE_DIR`, else `$HOME/.local/share/puerperium`.
    ///
    /// Returns `None` when neither is available rather than guessing a path — a tool that
    /// silently writes somewhere unexpected is worse than one that says it cannot.
    pub fn from_env() -> Option<Self> {
        if let Some(dir) = std::env::var_os("PUERPERIUM_STATE_DIR") {
            return Some(Self::new(dir));
        }
        std::env::var_os("HOME")
            .map(|h| Self::new(PathBuf::from(h).join(".local/share/puerperium")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create the layout and lock it to the owner (`0700` on Unix).
    pub fn ensure(&self) -> Result<()> {
        store::ensure_dir(&self.root)?;
        store::ensure_dir(&self.datasets())?;
        store::ensure_dir(&self.models())?;
        store::ensure_dir(&self.apprentices())?;
        Ok(())
    }

    pub fn datasets(&self) -> PathBuf {
        self.root.join("datasets")
    }

    pub fn models(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn apprentices(&self) -> PathBuf {
        self.root.join("apprentices")
    }

    /// Where a model's artifacts live, as distinct from its record.
    pub fn model_artifacts(&self, name: &str) -> PathBuf {
        self.models().join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_hangs_off_one_root() {
        let p = Paths::new("/tmp/x");
        assert_eq!(p.datasets(), Path::new("/tmp/x/datasets"));
        assert_eq!(p.models(), Path::new("/tmp/x/models"));
        assert_eq!(p.apprentices(), Path::new("/tmp/x/apprentices"));
        assert_eq!(p.model_artifacts("m1"), Path::new("/tmp/x/models/m1"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_locks_the_state_root() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let p = Paths::new(dir.path().join("state"));
        p.ensure().expect("ensure");
        let mode = std::fs::metadata(p.root())
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        assert!(p.datasets().is_dir());
    }
}

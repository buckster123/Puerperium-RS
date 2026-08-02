//! Loading credentials from an env file.
//!
//! House pattern (`Launchpad-RS/docs/deploy.md`): plain `KEY=VALUE`, no `export`, `0600`.
//! Deployed services read `/etc/<name>/env`; a laptop CLI reads
//! `~/.config/puerperium/env`. Both are searched, and `$PUERPERIUM_ENV_FILE` overrides.
//!
//! # Rules
//!
//! - **A real environment variable always wins.** Loading never overwrites what the shell
//!   already set, so a one-off `TOGETHER_API_KEY=… puerperium …` still works and the file is
//!   the default rather than the authority.
//! - **Values are never logged.** Not at any level, not on error. Names and counts only
//!   (doctrine #6).
//! - **A missing file is not an error.** Most invocations need no credential at all; only the
//!   operation that needs one should complain, and it should name the variable.
//! - **Loose permissions warn, they do not fail.** Refusing to run would push people toward
//!   pasting the key on a command line, where it lands in shell history — a worse outcome
//!   than a readable file on a single-user laptop.

use std::path::{Path, PathBuf};

/// What a load did. Carries no values — only what a human needs to see.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Loaded {
    pub file: Option<PathBuf>,
    /// Variable names set from the file. Never their values.
    pub set: Vec<String>,
    /// Names present in the file but already set in the environment, so left alone.
    pub skipped: Vec<String>,
    /// Non-fatal complaints, e.g. permissions.
    pub warnings: Vec<String>,
}

/// Where an env file may live, in search order.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(explicit) = std::env::var_os("PUERPERIUM_ENV_FILE") {
        out.push(PathBuf::from(explicit));
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(&home).join(".config/puerperium/env"));
    }
    out.push(PathBuf::from("/etc/puerperium/env"));
    out
}

/// Load the first env file that exists. A missing file is `Ok` with nothing set.
pub fn load() -> Loaded {
    for path in candidates() {
        if path.is_file() {
            return load_file(&path);
        }
    }
    Loaded::default()
}

/// Load one specific file.
pub fn load_file(path: &Path) -> Loaded {
    let mut out = Loaded {
        file: Some(path.to_path_buf()),
        ..Default::default()
    };

    let Ok(text) = std::fs::read_to_string(path) else {
        out.warnings
            .push(format!("could not read {}", path.display()));
        return out;
    };

    out.warnings.extend(permission_warning(path));

    for (key, value) in parse(&text) {
        if std::env::var_os(&key).is_some() {
            out.skipped.push(key);
            continue;
        }
        std::env::set_var(&key, value);
        out.set.push(key);
    }
    out
}

/// Parse `KEY=VALUE` lines. Blank lines and `#` comments are skipped, a leading `export` is
/// tolerated, and one layer of surrounding quotes is stripped.
///
/// Pure — the whole reason this is testable without touching a real file.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !is_env_name(key) {
            continue;
        }
        out.push((key.to_string(), unquote(value.trim()).to_string()));
    }
    out
}

/// A shell environment variable name: letter or underscore first, then alphanumerics and
/// underscores. The leading-character rule is the one that is easy to forget — checking only
/// the character set lets `1BAD=x` through, and it can never be read back as `$1BAD`.
fn is_env_name(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn unquote(v: &str) -> &str {
    let bytes = v.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// Warn when the file is readable by anyone but its owner.
#[cfg(unix)]
fn permission_warning(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then(|| {
        format!(
            "{} is mode {mode:04o} — group/other can read it; `chmod 600` it",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn permission_warning(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_house_format() {
        let text = "\
# a comment

TOGETHER_API_KEY=abc123
export OTHER=xyz
QUOTED=\"has spaces\"
SINGLE='also quoted'
";
        let got = parse(text);
        assert_eq!(
            got,
            vec![
                ("TOGETHER_API_KEY".into(), "abc123".into()),
                ("OTHER".into(), "xyz".into()),
                ("QUOTED".into(), "has spaces".into()),
                ("SINGLE".into(), "also quoted".into()),
            ]
        );
    }

    #[test]
    fn ignores_lines_that_are_not_assignments() {
        let got = parse("not an assignment\n=novalue\n1BAD=x\nGOOD=y\n");
        assert_eq!(got, vec![("GOOD".to_string(), "y".to_string())]);
    }

    #[test]
    fn a_value_containing_equals_survives_intact() {
        // Base64 and JWT-ish keys routinely end in '='.
        let got = parse("K=abc=def==\n");
        assert_eq!(got, vec![("K".to_string(), "abc=def==".to_string())]);
    }

    /// The report is shown to humans and may reach a log; it must carry names, never values.
    #[test]
    fn the_load_report_never_contains_a_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("env");
        std::fs::write(&path, "PUERPERIUM_TEST_SECRET=super-secret-value\n").expect("write");

        std::env::remove_var("PUERPERIUM_TEST_SECRET");
        let loaded = load_file(&path);

        let rendered = format!("{loaded:?}");
        assert!(
            !rendered.contains("super-secret-value"),
            "leaked: {rendered}"
        );
        assert!(loaded.set.contains(&"PUERPERIUM_TEST_SECRET".to_string()));
        assert_eq!(
            std::env::var("PUERPERIUM_TEST_SECRET").as_deref(),
            Ok("super-secret-value")
        );
        std::env::remove_var("PUERPERIUM_TEST_SECRET");
    }

    /// A one-off `KEY=… puerperium …` must still beat the file.
    #[test]
    fn an_existing_environment_variable_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("env");
        std::fs::write(&path, "PUERPERIUM_TEST_WINS=from-file\n").expect("write");

        std::env::set_var("PUERPERIUM_TEST_WINS", "from-shell");
        let loaded = load_file(&path);

        assert_eq!(
            std::env::var("PUERPERIUM_TEST_WINS").as_deref(),
            Ok("from-shell")
        );
        assert!(loaded.skipped.contains(&"PUERPERIUM_TEST_WINS".to_string()));
        assert!(loaded.set.is_empty());
        std::env::remove_var("PUERPERIUM_TEST_WINS");
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let loaded = load_file(Path::new("/nonexistent/puerperium/env"));
        assert!(loaded.set.is_empty());
        assert_eq!(
            loaded.warnings.len(),
            1,
            "says it could not read, and moves on"
        );
    }

    #[cfg(unix)]
    #[test]
    fn loose_permissions_warn_but_still_load() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("env");
        std::fs::write(&path, "PUERPERIUM_TEST_PERM=x\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        std::env::remove_var("PUERPERIUM_TEST_PERM");
        let loaded = load_file(&path);

        assert!(
            loaded.warnings.iter().any(|w| w.contains("chmod 600")),
            "should warn: {:?}",
            loaded.warnings
        );
        assert!(
            !loaded.set.is_empty(),
            "refusing would push people to the command line"
        );
        std::env::remove_var("PUERPERIUM_TEST_PERM");
    }

    #[cfg(unix)]
    #[test]
    fn a_correctly_locked_file_warns_about_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("env");
        std::fs::write(&path, "PUERPERIUM_TEST_OK=x\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        std::env::remove_var("PUERPERIUM_TEST_OK");
        let loaded = load_file(&path);
        assert!(loaded.warnings.is_empty(), "got {:?}", loaded.warnings);
        std::env::remove_var("PUERPERIUM_TEST_OK");
    }

    #[test]
    fn search_order_puts_the_explicit_override_first() {
        std::env::set_var("PUERPERIUM_ENV_FILE", "/tmp/explicit-env");
        let c = candidates();
        assert_eq!(c[0], PathBuf::from("/tmp/explicit-env"));
        assert!(c.iter().any(|p| p.ends_with(".config/puerperium/env")));
        assert!(c.contains(&PathBuf::from("/etc/puerperium/env")));
        std::env::remove_var("PUERPERIUM_ENV_FILE");
    }
}

//! Key-shaped tokens. Pure. Never logs a value — only a yes/no.

/// Does this round look like it contains a credential?
///
/// Minimum patterns from `docs/harvest.md`. Extend here when a live export
/// finds a new shape, not by guessing.
pub fn looks_like_secret(text: &str) -> bool {
    if text.contains("BEGIN ") && text.contains("PRIVATE KEY") {
        return true;
    }
    if has_sk_token(text) {
        return true;
    }
    if has_key_assignment(text) {
        return true;
    }
    if has_bearer_hex(text) {
        return true;
    }
    false
}

fn has_sk_token(text: &str) -> bool {
    // Longer prefixes first so `sk-ant-` is not counted as a short `sk-`.
    for prefix in ["sk-ant-", "sk-or-", "sk-"] {
        let mut rest = text;
        while let Some(i) = rest.find(prefix) {
            let after = &rest[i + prefix.len()..];
            let n = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .count();
            if n >= 16 {
                return true;
            }
            rest = &rest[i + prefix.len()..];
        }
    }
    false
}

const KEY_NAMES: &[&str] = &[
    "TOGETHER_API_KEY=",
    "OPENAI_API_KEY=",
    "OAI_API_KEY=",
    "ANTHROPIC_API_KEY=",
    "AGENTD_TOKEN=",
];

fn has_key_assignment(text: &str) -> bool {
    for name in KEY_NAMES {
        if let Some(i) = text.find(name) {
            let val = text[i + name.len()..]
                .chars()
                .take_while(|c| !c.is_whitespace())
                .count();
            if val >= 8 {
                return true;
            }
        }
    }
    false
}

fn has_bearer_hex(text: &str) -> bool {
    for needle in ["Bearer ", "bearer ", "token="] {
        let mut rest = text;
        while let Some(i) = rest.find(needle) {
            let after = rest[i + needle.len()..].trim_start();
            let hex: String = after
                .chars()
                .take(64)
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if hex.len() == 64 {
                return true;
            }
            rest = &rest[i + needle.len()..];
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_sk_and_assignments_and_bearers() {
        assert!(looks_like_secret(
            "export TOGETHER_API_KEY=sk-not-a-real-key-value-at-all"
        ));
        assert!(looks_like_secret(
            "Authorization: Bearer 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(looks_like_secret("-----BEGIN RSA PRIVATE KEY-----\nMIIB"));
        assert!(!looks_like_secret(
            "run cargo test — all pass. commit 0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!looks_like_secret(
            "the token length is 64 and the head is f6fb"
        ));
    }
}

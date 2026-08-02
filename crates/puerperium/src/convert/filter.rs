//! The quality gate.
//!
//! A Cerebro store holds **messages and chatter as well as knowledge** — agent-to-agent
//! pings, smoke tests, greetings. Those are legitimate memories and terrible training data.
//! This module is the reason a naive "mine everything" run does not teach a worker model to
//! greet people.
//!
//! Every rejection is **counted by reason** and reported. A filter that silently eats data
//! is worse than no filter: you cannot tell a well-curated 40-example dataset from a
//! catastrophically over-filtered one.

use std::collections::BTreeMap;

use crate::memory::{MemoryRecord, MemoryType};

/// Why a memory did not become training data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rejection {
    /// Memory type not in the include set (episodic is excluded by default).
    TypeExcluded,
    /// Too short to teach anything.
    TooShort,
    /// Conversational artifact — greeting, direct address, connectivity check.
    Chatter,
    /// Carries a denylisted tag.
    DeniedTag,
    /// Cerebro itself already judged it marginal.
    LowSalience,
    /// Mostly a bare URL, or mostly non-prose.
    NotProse,
    /// Survived the gate but could not be framed as a question — no heading trail, no tags.
    /// Recorded in the same ledger so a run's totals always add up.
    Unframeable,
}

impl Rejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Rejection::TypeExcluded => "type_excluded",
            Rejection::TooShort => "too_short",
            Rejection::Chatter => "chatter",
            Rejection::DeniedTag => "denied_tag",
            Rejection::LowSalience => "low_salience",
            Rejection::NotProse => "not_prose",
            Rejection::Unframeable => "unframeable",
        }
    }
}

/// Tallies rejections by reason so a run can report what it dropped and why.
#[derive(Debug, Default, Clone)]
pub struct RejectionLedger {
    counts: BTreeMap<&'static str, usize>,
    total: usize,
}

impl RejectionLedger {
    pub fn record(&mut self, r: Rejection) {
        *self.counts.entry(r.as_str()).or_insert(0) += 1;
        self.total += 1;
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn counts(&self) -> &BTreeMap<&'static str, usize> {
        &self.counts
    }

    pub fn count_of(&self, r: Rejection) -> usize {
        self.counts.get(r.as_str()).copied().unwrap_or(0)
    }
}

/// Tunable thresholds. Defaults were set against the real store (349 memories, 2026-08-02).
#[derive(Debug, Clone)]
pub struct FilterConfig {
    pub min_content: usize,
    pub min_salience: f32,
    pub include_types: Vec<MemoryType>,
    pub denied_tags: Vec<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            min_content: 120,
            min_salience: 0.3,
            include_types: MemoryType::DEFAULT_INCLUDED.to_vec(),
            denied_tags: ["a2a", "msg", "message", "test", "smoke", "chatter", "ping"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// Tag prefixes that mark routing metadata rather than subject matter.
///
/// A2A messages in the real store carry `msg`, `from:CLAUDE`, `to:HERMES-KRKN`. The bare
/// `message` denylist entry missed them — the tag is `msg`, and the routing pair is
/// structured. Both are covered now.
const ROUTING_TAG_PREFIXES: [&str; 2] = ["from:", "to:"];

/// Is this tag routing metadata (`from:CLAUDE`) rather than a subject?
pub fn is_routing_tag(tag: &str) -> bool {
    let t = tag.trim().to_lowercase();
    ROUTING_TAG_PREFIXES.iter().any(|p| t.starts_with(p))
}

/// Openers no knowledge memory begins with.
const GREETING_OPENERS: [&str; 7] = ["yo ", "hey ", "hi ", "hello ", "greetings", "sup ", "howdy"];

/// Phrases that address a reader directly. A reference document never asks these.
const DIRECT_ADDRESS: [&str; 8] = [
    "can you hear me",
    "are you there",
    "how's it going",
    "hows it going",
    "how are you",
    "testing 123",
    "just checking in",
    "first smoke test",
];

/// Decide whether a memory may become training data.
///
/// Pure. `Ok(())` means it passes.
pub fn assess(mem: &MemoryRecord, cfg: &FilterConfig) -> Result<(), Rejection> {
    if !cfg.include_types.contains(&mem.memory_type) {
        return Err(Rejection::TypeExcluded);
    }

    let content = mem.content.trim();
    if content.chars().count() < cfg.min_content {
        return Err(Rejection::TooShort);
    }

    if mem.salience < cfg.min_salience {
        return Err(Rejection::LowSalience);
    }

    let tags = mem.tags_lower();
    if tags
        .iter()
        .any(|t| cfg.denied_tags.iter().any(|d| t == d) || is_routing_tag(t))
    {
        return Err(Rejection::DeniedTag);
    }

    if is_chatter(content) {
        return Err(Rejection::Chatter);
    }

    if !is_prose(content) {
        return Err(Rejection::NotProse);
    }

    Ok(())
}

/// Conversational artifact detection.
///
/// Two decisive signals, chosen to avoid eating legitimate content:
///
/// 1. A **greeting opener** on the first line. A reference document does not begin "Yo X!".
/// 2. A **direct-address phrase** anywhere.
///
/// Notably *not* decisive on its own: the bare phrase "smoke test", which appears in
/// perfectly good operational procedures ("run the smoke test before deploying"). Only the
/// conversational form ("first smoke test") counts.
fn is_chatter(content: &str) -> bool {
    let first_line = content.lines().next().unwrap_or("").trim().to_lowercase();
    if GREETING_OPENERS.iter().any(|g| first_line.starts_with(g)) {
        return true;
    }

    let lower = content.to_lowercase();
    DIRECT_ADDRESS.iter().any(|p| lower.contains(p))
}

/// Reject content that is mostly a bare URL or mostly not words.
fn is_prose(content: &str) -> bool {
    let word_chars = content.chars().filter(|c| c.is_alphanumeric()).count();
    if word_chars * 2 < content.chars().count() {
        return false; // majority punctuation/symbols
    }

    // A lone URL with a few words around it is a bookmark, not a lesson.
    let non_url_len: usize = content
        .split_whitespace()
        .filter(|w| !w.starts_with("http://") && !w.starts_with("https://"))
        .map(|w| w.len())
        .sum();
    non_url_len >= 60
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(content: &str, ty: MemoryType) -> MemoryRecord {
        MemoryRecord {
            id: "m1".into(),
            content: content.into(),
            memory_type: ty,
            tags: vec![],
            agent_id: Some("CLAUDE".into()),
            salience: 0.8,
        }
    }

    /// The named regression case: a real memory from the live store, verbatim.
    /// If this ever passes the gate, a worker model learns to say hello instead of work.
    #[test]
    fn rejects_the_real_a2a_smoke_test_memory() {
        let real = "Yo HERMES-KRKN! 👋 This is Qwen-36 Harness here. Just doing a first \
                    smoke test on the A2A messaging system since we're all sharing the same \
                    Cerebro brain now. Can you hear me? How's the NPU work going? 🧠⚡";
        // Long enough to clear the length gate — the chatter rule is what must catch it.
        assert!(real.chars().count() > FilterConfig::default().min_content);
        assert_eq!(
            assess(&mem(real, MemoryType::Semantic), &FilterConfig::default()),
            Err(Rejection::Chatter)
        );
    }

    #[test]
    fn keeps_operational_prose_that_merely_mentions_a_smoke_test() {
        let legit = "Deploy procedure for the router: stop the service, copy the binary into \
                     /usr/local/bin, start it again, then run the smoke test against the \
                     control plane before declaring the deploy good. A running binary cannot \
                     be overwritten — 'text file busy' means the stop was skipped.";
        assert_eq!(
            assess(
                &mem(legit, MemoryType::Procedural),
                &FilterConfig::default()
            ),
            Ok(())
        );
    }

    #[test]
    fn episodic_is_excluded_by_default_but_admissible_on_request() {
        let long = "x ".repeat(200);
        let m = mem(&long, MemoryType::Episodic);
        assert_eq!(
            assess(&m, &FilterConfig::default()),
            Err(Rejection::TypeExcluded)
        );

        let cfg = FilterConfig {
            include_types: vec![MemoryType::Episodic],
            ..FilterConfig::default()
        };
        // Now only the prose gate applies — "x x x" is alphanumeric-majority, so it passes.
        assert_eq!(assess(&m, &cfg), Ok(()));
    }

    #[test]
    fn rejects_short_low_salience_and_denied_tags_distinctly() {
        let cfg = FilterConfig::default();

        assert_eq!(
            assess(&mem("too short", MemoryType::Semantic), &cfg),
            Err(Rejection::TooShort)
        );

        let body = "A genuinely useful and sufficiently long operational note about how the \
                    deployment pipeline actually behaves in practice on the target board.";

        let mut low = mem(body, MemoryType::Semantic);
        low.salience = 0.1;
        assert_eq!(assess(&low, &cfg), Err(Rejection::LowSalience));

        let mut tagged = mem(body, MemoryType::Semantic);
        tagged.tags = vec!["A2A".into(), "infra".into()]; // case-insensitive
        assert_eq!(assess(&tagged, &cfg), Err(Rejection::DeniedTag));
    }

    #[test]
    fn rejects_a_bare_bookmark() {
        let bookmark = format!("see {}", "https://example.com/".repeat(12));
        assert_eq!(
            assess(
                &mem(&bookmark, MemoryType::Semantic),
                &FilterConfig::default()
            ),
            Err(Rejection::NotProse)
        );
    }

    #[test]
    fn ledger_counts_by_reason() {
        let mut led = RejectionLedger::default();
        led.record(Rejection::Chatter);
        led.record(Rejection::Chatter);
        led.record(Rejection::TooShort);
        assert_eq!(led.total(), 3);
        assert_eq!(led.count_of(Rejection::Chatter), 2);
        assert_eq!(led.count_of(Rejection::LowSalience), 0);
    }
}

//! Framing a chunk as an instruction.
//!
//! S1 is **templates only**: deterministic, free, no LLM. The instruction is derived from
//! what the memory already carries — its heading trail, or failing that its tags.
//!
//! **Puerperium never rewrites the knowledge.** The response is the chunk body verbatim;
//! only the question is synthesised. LLM-assisted question generation is deferred (it costs
//! tokens, so charter D4 gates it) and template mode has to be the honest floor regardless:
//! a dataset must be buildable on a node with no key and no budget.

use crate::convert::chunk::Chunk;
use crate::convert::filter;
use crate::memory::MemoryRecord;
use crate::provenance::InstructionKind;

#[derive(Debug, Clone, Default)]
pub struct InstructConfig {
    /// Optional domain framing for the tag fallback, e.g. `Some("ApexOS")` →
    /// *"What do you know about mesh, federation in ApexOS?"*. Left unset by default —
    /// Puerperium is a general tool and should not assume whose knowledge it is mining.
    pub domain: Option<String>,
    /// How many tags the fallback names.
    pub max_tags: usize,
}

impl InstructConfig {
    pub fn new() -> Self {
        Self {
            domain: None,
            max_tags: 3,
        }
    }
}

/// Build the instruction half of an example.
///
/// Returns `None` when the chunk cannot be framed honestly — no heading trail and no tags.
/// The caller counts those rather than inventing a question, because a fabricated frame
/// ("Explain the following.") teaches a model nothing about when to say it.
pub fn instruction_for(
    chunk: &Chunk,
    mem: &MemoryRecord,
    cfg: &InstructConfig,
) -> Option<(String, InstructionKind)> {
    let path: Vec<String> = chunk.heading_path.iter().map(|h| tidy(h)).collect();

    match path.len() {
        0 => from_tags(mem, cfg).map(|q| (q, InstructionKind::TemplatedTag)),
        1 => {
            let q = if is_statement(&path[0]) {
                format!("Explain: {}", path[0])
            } else {
                format!("Explain {}.", path[0])
            };
            Some((q, InstructionKind::TemplatedHeading))
        }
        _ => {
            let doc = &path[0];
            let leaf = &path[path.len() - 1];
            let q = if leaf.eq_ignore_ascii_case(doc) {
                format!("Explain {doc}.")
            } else if is_statement(leaf) {
                // Some memories use a whole sentence as a heading ("Vast SSH-mode OVERRIDES
                // Docker ENTRYPOINT"). Inlining that after "Explain" reads as gibberish, so
                // the statement gets its own clause instead.
                format!("In {doc}, explain: {leaf}")
            } else {
                format!("Explain {leaf}, in the context of {doc}.")
            };
            Some((q, InstructionKind::TemplatedHeading))
        }
    }
}

/// A heading longer than this is treated as a statement, not a topic name.
const MAX_PHRASE_CHARS: usize = 40;
/// …as is one with more words than this.
const MAX_PHRASE_WORDS: usize = 4;

/// Does this heading read as a statement rather than a topic name?
///
/// Real-store examples that must be treated as statements:
/// `"Vast SSH-mode OVERRIDES Docker ENTRYPOINT"` (5 words),
/// `"hf download --local-dir + HF_HOME = DOUBLE CACHE"`,
/// `"When integrating NPU with CC, use zero-code approach"`.
///
/// Deliberately **conservative**: the clause form ("In <doc>, explain: <heading>") reads
/// acceptably for a noun phrase too, while the inline form reads as gibberish for a
/// statement. So only short, clearly-phrase-shaped headings take the inline form — a
/// false positive here costs a little verbosity, a false negative costs a broken sentence.
fn is_statement(heading: &str) -> bool {
    heading.chars().count() > MAX_PHRASE_CHARS
        || heading.split_whitespace().count() > MAX_PHRASE_WORDS
        || heading.contains(", ")
}

/// Bookkeeping tags that describe the *record*, not its subject.
///
/// Framing from these produced, on the real store: *"What do you know about phase-6,
/// completion-summary, and session-notes?"* — grammatically fine and semantically empty.
const BOOKKEEPING_TAGS: [&str; 12] = [
    "session-notes",
    "session",
    "notes",
    "summary",
    "completion-summary",
    "status",
    "update",
    "wip",
    "todo",
    "misc",
    "general",
    "project-state",
];

/// Does this tag name a subject someone could ask about?
///
/// Excludes routing metadata (`from:CLAUDE`), bare years and ranges (`2024`, `2024-2026`),
/// and record-bookkeeping terms.
fn is_topical(tag: &str) -> bool {
    let t = tag.trim().to_lowercase();
    if t.is_empty() || filter::is_routing_tag(&t) || BOOKKEEPING_TAGS.contains(&t.as_str()) {
        return false;
    }
    !t.chars()
        .all(|c| c.is_ascii_digit() || c == '-' || c == '/')
}

fn from_tags(mem: &MemoryRecord, cfg: &InstructConfig) -> Option<String> {
    let max = if cfg.max_tags == 0 { 3 } else { cfg.max_tags };
    let tags: Vec<String> = mem
        .tags
        .iter()
        .filter(|t| is_topical(t))
        .take(max)
        .map(|t| tidy(t))
        .collect();
    if tags.is_empty() {
        return None;
    }

    let subject = join_natural(&tags);
    Some(match &cfg.domain {
        Some(d) => format!("What do you know about {subject} in {d}?"),
        None => format!("What do you know about {subject}?"),
    })
}

/// Strip list numbering, trailing colons and surrounding backticks from a heading.
///
/// `"## 1. CORE CLI:"` has already lost its hashes and colon by the time it arrives here;
/// this removes the `1. ` so the instruction reads "Explain CORE CLI" rather than
/// "Explain 1. CORE CLI".
fn tidy(s: &str) -> String {
    let s = s.trim().trim_matches('`').trim();
    let stripped = match s.find(". ") {
        Some(i) if s[..i].chars().all(|c| c.is_ascii_digit()) && i > 0 => &s[i + 2..],
        _ => s,
    };
    stripped.trim().trim_end_matches(':').trim().to_string()
}

fn join_natural(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryType;

    fn mem(tags: &[&str]) -> MemoryRecord {
        MemoryRecord {
            id: "m1".into(),
            content: "body".into(),
            memory_type: MemoryType::Procedural,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            agent_id: None,
            salience: 0.8,
        }
    }

    fn chunk(path: &[&str]) -> Chunk {
        Chunk {
            heading_path: path.iter().map(|s| s.to_string()).collect(),
            body: "body".into(),
        }
    }

    /// Framing strategy is recorded, not just the text — a consumer must be able to tell a
    /// heading-derived example from a tag-derived one.
    #[test]
    fn sectioned_chunk_frames_leaf_within_document_and_reports_heading_kind() {
        let c = chunk(&[
            "VLLM SERVING CONFIGURATION REFERENCE",
            "1. CORE CLI",
            "Essential Flags",
        ]);
        let (got, kind) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(
            got,
            "Explain Essential Flags, in the context of VLLM SERVING CONFIGURATION REFERENCE."
        );
        assert_eq!(kind, InstructionKind::TemplatedHeading);
    }

    #[test]
    fn numbering_is_stripped_from_headings() {
        let c = chunk(&["Doc", "1. CORE CLI"]);
        let (got, _) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(got, "Explain CORE CLI, in the context of Doc.");
    }

    #[test]
    fn title_only_path_asks_about_the_title() {
        let c = chunk(&["Deploy procedure"]);
        let (got, kind) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(got, "Explain Deploy procedure.");
        assert_eq!(kind, InstructionKind::TemplatedHeading);
    }

    #[test]
    fn leaf_equal_to_doc_does_not_produce_a_tautology() {
        let c = chunk(&["Deploy", "deploy"]);
        let (got, _) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(got, "Explain Deploy.");
    }

    #[test]
    fn falls_back_to_tags_with_natural_joining_and_reports_tag_kind() {
        let c = chunk(&[]);
        let (got, kind) = instruction_for(
            &c,
            &mem(&["mesh", "federation", "beacon"]),
            &InstructConfig::new(),
        )
        .expect("framed");
        assert_eq!(got, "What do you know about mesh, federation, and beacon?");
        assert_eq!(kind, InstructionKind::TemplatedTag);
    }

    #[test]
    fn two_tags_join_with_and() {
        let c = chunk(&[]);
        let (got, _) =
            instruction_for(&c, &mem(&["mesh", "beacon"]), &InstructConfig::new()).expect("framed");
        assert_eq!(got, "What do you know about mesh and beacon?");
    }

    #[test]
    fn domain_framing_is_opt_in() {
        let c = chunk(&[]);
        let cfg = InstructConfig {
            domain: Some("ApexOS".into()),
            max_tags: 3,
        };
        let (got, _) = instruction_for(&c, &mem(&["mesh"]), &cfg).expect("framed");
        assert_eq!(got, "What do you know about mesh in ApexOS?");
    }

    #[test]
    fn unframeable_chunk_returns_none_rather_than_inventing_a_question() {
        let c = chunk(&[]);
        assert!(instruction_for(&c, &mem(&[]), &InstructConfig::new()).is_none());
    }

    /// Real-store regression: routing tags and bookkeeping terms are not subjects.
    /// These produced "What do you know about phase-6, completion-summary, and
    /// session-notes?" — a question nobody asks.
    #[test]
    fn non_topical_tags_do_not_frame_a_question() {
        let c = chunk(&[]);
        for tags in [
            vec!["from:CLAUDE", "to:HERMES-KRKN"],
            vec!["session-notes", "completion-summary"],
            vec!["2024", "2024-2026"],
        ] {
            assert!(
                instruction_for(&c, &mem(&tags), &InstructConfig::new()).is_none(),
                "{tags:?} should not frame a question"
            );
        }
    }

    /// Real-store regression: some memories use a whole claim as a section heading.
    /// "Explain <claim>, in the context of <doc>." reads as gibberish; it gets its own clause.
    #[test]
    fn statement_headings_get_a_clause_not_an_inline_noun_phrase() {
        let c = chunk(&[
            "PROCEDURE — Deploying to Vast",
            "Vast SSH-mode OVERRIDES Docker ENTRYPOINT",
        ]);
        let (got, kind) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(
            got,
            "In PROCEDURE — Deploying to Vast, explain: Vast SSH-mode OVERRIDES Docker ENTRYPOINT"
        );
        assert_eq!(kind, InstructionKind::TemplatedHeading);
    }

    /// A title-only path can be a statement too — the depth-1 branch needs the same guard.
    #[test]
    fn statement_title_alone_also_gets_the_clause_form() {
        let c = chunk(&["When integrating NPU with CC, use the zero-code approach"]);
        let (got, _) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(
            got,
            "Explain: When integrating NPU with CC, use the zero-code approach"
        );
    }

    #[test]
    fn short_noun_phrase_headings_keep_the_inline_form() {
        let c = chunk(&["Doc", "Essential Flags"]);
        let (got, _) = instruction_for(&c, &mem(&[]), &InstructConfig::new()).expect("framed");
        assert_eq!(got, "Explain Essential Flags, in the context of Doc.");
    }

    #[test]
    fn topical_tags_survive_alongside_bookkeeping_ones() {
        let c = chunk(&[]);
        let (got, _) = instruction_for(
            &c,
            &mem(&["session-notes", "mesh", "2026", "federation"]),
            &InstructConfig::new(),
        )
        .expect("framed");
        assert_eq!(got, "What do you know about mesh and federation?");
    }
}

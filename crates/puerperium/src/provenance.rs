//! Where an example came from.
//!
//! Charter D12: every example records its origin. Lineage is the product — an
//! unattributable dataset produces an unattributable model, and the registry stops being
//! able to answer "why is this specialist like this?".

use serde::{Deserialize, Serialize};

/// The origin of a single training example.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Derived from a Cerebro memory.
    CerebroMemory {
        memory_id: String,
        /// Whose memory space it was mined from — **not** the trainer (charter D6).
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        /// Heading path when the memory was section-chunked, e.g.
        /// `["VLLM SERVING REFERENCE", "Essential Flags"]`. Empty for whole-memory chunks.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        heading_path: Vec<String>,
    },
    /// Generated from a template over a tool schema.
    Synthetic { template: String },
}

impl Provenance {
    /// The originating memory id, when there is one. Used for dedup and lineage walks.
    pub fn memory_id(&self) -> Option<&str> {
        match self {
            Provenance::CerebroMemory { memory_id, .. } => Some(memory_id),
            Provenance::Synthetic { .. } => None,
        }
    }

    /// A stable key for histograms: the source kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Provenance::CerebroMemory { .. } => "cerebro_memory",
            Provenance::Synthetic { .. } => "synthetic",
        }
    }
}

/// How the instruction half of an example was produced.
///
/// The strong/weak split is **recorded, not hidden**. Measured against the real store, 38%
/// of a naive run framed from tags alone, and those read like *"What do you know about
/// phase-6, completion-summary, and session-notes?"* — a question nobody asks, which teaches
/// a model to answer nonsense. Rather than silently mixing those in with heading-derived
/// examples or silently dropping them, each example says which it is, so a consumer can
/// select or weight and the dataset's own metadata reports the ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionKind {
    /// Framed from a heading trail — the document said what this section is about. Strong.
    TemplatedHeading,
    /// Framed from topical tags, with no heading trail to draw on. Weaker: the tags describe
    /// the memory, not the question someone would ask of it.
    TemplatedTag,
    /// Written by a model. Costs tokens; gated by charter D4.
    LlmAssisted,
}

impl InstructionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InstructionKind::TemplatedHeading => "templated_heading",
            InstructionKind::TemplatedTag => "templated_tag",
            InstructionKind::LlmAssisted => "llm_assisted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cerebro_provenance_omits_empty_optionals_on_the_wire() {
        let p = Provenance::CerebroMemory {
            memory_id: "m1".into(),
            agent_id: None,
            heading_path: vec![],
        };
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(json, r#"{"kind":"cerebro_memory","memory_id":"m1"}"#);
    }

    #[test]
    fn provenance_roundtrips_with_a_heading_path() {
        let p = Provenance::CerebroMemory {
            memory_id: "m1".into(),
            agent_id: Some("CLAUDE".into()),
            heading_path: vec!["Doc".into(), "Section".into()],
        };
        let back: Provenance =
            serde_json::from_str(&serde_json::to_string(&p).expect("ser")).expect("de");
        assert_eq!(back, p);
        assert_eq!(back.memory_id(), Some("m1"));
    }
}

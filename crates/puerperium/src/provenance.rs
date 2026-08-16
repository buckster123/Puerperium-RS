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
    /// One ApexOS session round (D13). Not a Cerebro memory.
    SessionTurn {
        node_id: String,
        session_id: u64,
        turn_index: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        license_class: LicenseClass,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
}

/// Whether a round's thinking may be persisted into a training example (D13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseClass {
    OpenReasoning,
    ClosedHidden,
    AnswerOnly,
}

impl LicenseClass {
    pub fn as_str(self) -> &'static str {
        match self {
            LicenseClass::OpenReasoning => "open_reasoning",
            LicenseClass::ClosedHidden => "closed_hidden",
            LicenseClass::AnswerOnly => "answer_only",
        }
    }
}

impl Provenance {
    /// The originating memory id, when there is one. Used for dedup and lineage walks.
    pub fn memory_id(&self) -> Option<&str> {
        match self {
            Provenance::CerebroMemory { memory_id, .. } => Some(memory_id),
            Provenance::Synthetic { .. } | Provenance::SessionTurn { .. } => None,
        }
    }

    /// A stable key for histograms: the source kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Provenance::CerebroMemory { .. } => "cerebro_memory",
            Provenance::Synthetic { .. } => "synthetic",
            Provenance::SessionTurn { .. } => "session_turn",
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
    /// The user turn *is* the instruction; the rest of the round is the trajectory (D13).
    LivedTurn,
}

impl InstructionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InstructionKind::TemplatedHeading => "templated_heading",
            InstructionKind::TemplatedTag => "templated_tag",
            InstructionKind::LlmAssisted => "llm_assisted",
            InstructionKind::LivedTurn => "lived_turn",
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

    #[test]
    fn session_turn_omits_empty_optionals_and_has_no_memory_id() {
        let p = Provenance::SessionTurn {
            node_id: "apex1".into(),
            session_id: 22,
            turn_index: 0,
            agent_id: None,
            license_class: LicenseClass::AnswerOnly,
            model: None,
        };
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(
            json,
            r#"{"kind":"session_turn","node_id":"apex1","session_id":22,"turn_index":0,"license_class":"answer_only"}"#
        );
        assert_eq!(p.memory_id(), None);
        assert_eq!(p.kind(), "session_turn");
    }
}

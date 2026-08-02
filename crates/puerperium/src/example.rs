//! A training example and its serialized form.
//!
//! Wire shape is sharegpt-style `messages`, which is load-bearing (`docs/design.md`
//! §Types): the JSONL this produces is what a provider ingests. A representation change
//! must be proven equivalent on the wire.

use serde::{Deserialize, Serialize};

use crate::provenance::{InstructionKind, Provenance};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One training example.
///
/// `messages` is what the provider consumes; `provenance` and `instruction_kind` are
/// Puerperium's own bookkeeping and ride alongside it in the JSONL. A provider that
/// rejects unknown fields would need them stripped at submit time — noted in
/// `docs/design.md` §Open questions, to be settled against Together's schema at S3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    pub messages: Vec<Message>,
    pub provenance: Provenance,
    pub instruction_kind: InstructionKind,
}

impl Example {
    /// Build a two-turn instruction example.
    pub fn instruction(
        instruction: impl Into<String>,
        response: impl Into<String>,
        provenance: Provenance,
        instruction_kind: InstructionKind,
    ) -> Self {
        Self {
            messages: vec![Message::user(instruction), Message::assistant(response)],
            provenance,
            instruction_kind,
        }
    }

    /// Serialize to one JSONL line (no trailing newline).
    ///
    /// Fails only if the content is not representable as JSON, which for `String` fields
    /// cannot happen — but the error is surfaced rather than unwrapped, per house rule.
    pub fn to_jsonl(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov() -> Provenance {
        Provenance::CerebroMemory {
            memory_id: "m1".into(),
            agent_id: Some("CLAUDE".into()),
            heading_path: vec![],
        }
    }

    #[test]
    fn instruction_example_has_exactly_user_then_assistant() {
        let ex = Example::instruction("q", "a", prov(), InstructionKind::TemplatedHeading);
        assert_eq!(ex.messages.len(), 2);
        assert_eq!(ex.messages[0].role, Role::User);
        assert_eq!(ex.messages[1].role, Role::Assistant);
        assert_eq!(ex.messages[1].content, "a");
    }

    #[test]
    fn jsonl_line_is_single_line_and_roundtrips() {
        let ex = Example::instruction(
            "explain\tthings",
            "a body\nwith a newline",
            prov(),
            InstructionKind::TemplatedHeading,
        );
        let line = ex.to_jsonl().expect("serialize");
        assert!(
            !line.contains('\n'),
            "embedded newline would corrupt JSONL: {line}"
        );
        let back: Example = serde_json::from_str(&line).expect("deserialize");
        assert_eq!(back, ex);
        // The newline survives as data, escaped — it just must not break the line format.
        assert!(back.messages[1].content.contains('\n'));
    }
}

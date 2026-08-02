//! Projecting a stored dataset into what a provider will actually accept.
//!
//! # Why this exists
//!
//! Our JSONL carries `provenance` and `instruction_kind` beside `messages`, because lineage
//! is the product (D12). **Together's validator rejects unknown columns** — it raises
//! `InvalidFileFormatError: Found extra column` — so uploading a stored dataset verbatim
//! would be refused.
//!
//! So the stored file and the uploaded file are **different artifacts**. The stored one keeps
//! the bookkeeping and keeps its hash, which is the lineage identity; the uploaded one is a
//! projection down to the provider's schema. Nothing is mutated, and the projection is
//! reproducible from the original at any time.
//!
//! The validation here deliberately **mirrors the upstream's own rules** so a malformed
//! dataset fails locally, with a line number, instead of costing a round trip to learn
//! "Found extra column" with no idea which line.

use crate::example::{Example, Role};

/// The dataset shapes a provider accepts.
///
/// Only `Conversation` in v1. Instruction (`prompt`/`completion`) and preference
/// (`input`/`preferred_output`/`non_preferred_output`) are real Together formats, but nothing
/// here produces them yet — an unused variant would be a promise we have not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFormat {
    /// `{"messages": [{"role": ..., "content": ...}]}`
    Conversation,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExportError {
    #[error("line {line}: not a valid stored example ({reason})")]
    Unparsable { line: usize, reason: String },

    #[error("line {line}: {reason}")]
    Invalid { line: usize, reason: String },

    #[error("dataset is empty — nothing to upload")]
    Empty,
}

/// Project stored JSONL into provider JSONL.
///
/// Pure. Returns the bytes that should be uploaded; the caller decides what to do with them.
pub fn to_provider_jsonl(stored: &str, format: ProviderFormat) -> Result<String, ExportError> {
    let ProviderFormat::Conversation = format;

    let mut out = String::new();
    let mut count = 0usize;

    for (idx, raw) in stored.lines().enumerate() {
        let line = idx + 1;
        if raw.trim().is_empty() {
            continue;
        }

        let example: Example = serde_json::from_str(raw).map_err(|e| ExportError::Unparsable {
            line,
            reason: e.to_string(),
        })?;

        validate(&example, line)?;

        // The projection: messages only. Everything else stays behind in the stored file.
        let projected = serde_json::json!({ "messages": example.messages });
        out.push_str(
            &serde_json::to_string(&projected).map_err(|e| ExportError::Unparsable {
                line,
                reason: e.to_string(),
            })?,
        );
        out.push('\n');
        count += 1;
    }

    if count == 0 {
        return Err(ExportError::Empty);
    }
    Ok(out)
}

/// Mirror the upstream's own conversation rules, so failure is local and specific.
fn validate(example: &Example, line: usize) -> Result<(), ExportError> {
    if example.messages.is_empty() {
        return Err(ExportError::Invalid {
            line,
            reason: "no messages".into(),
        });
    }
    for (i, m) in example.messages.iter().enumerate() {
        if m.content.trim().is_empty() {
            return Err(ExportError::Invalid {
                line,
                reason: format!("message {i} has empty content"),
            });
        }
    }
    // A training example that never answers teaches nothing, and the upstream bills for it
    // either way.
    if !example.messages.iter().any(|m| m.role == Role::Assistant) {
        return Err(ExportError::Invalid {
            line,
            reason: "no assistant turn — nothing to learn from".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::example::Message;
    use crate::provenance::{InstructionKind, Provenance};

    fn stored_line(instruction: &str, response: &str) -> String {
        let ex = Example::instruction(
            instruction,
            response,
            Provenance::CerebroMemory {
                memory_id: "m1".into(),
                agent_id: Some("CLAUDE".into()),
                heading_path: vec!["Doc".into(), "Section".into()],
            },
            InstructionKind::TemplatedHeading,
        );
        ex.to_jsonl().expect("serialize")
    }

    /// The whole reason this module exists: Together raises "Found extra column".
    #[test]
    fn projection_strips_everything_the_upstream_would_reject() {
        let stored = stored_line("Explain X.", "X is a thing.");
        assert!(
            stored.contains("provenance"),
            "stored form carries bookkeeping"
        );
        assert!(stored.contains("instruction_kind"));

        let out = to_provider_jsonl(&stored, ProviderFormat::Conversation).expect("export");

        assert!(!out.contains("provenance"), "would be rejected: {out}");
        assert!(
            !out.contains("instruction_kind"),
            "would be rejected: {out}"
        );

        let parsed: serde_json::Value = serde_json::from_str(out.trim()).expect("valid json");
        let obj = parsed.as_object().expect("object");
        assert_eq!(obj.len(), 1, "exactly one column: {obj:?}");
        assert!(obj.contains_key("messages"));
    }

    #[test]
    fn message_objects_carry_only_role_and_content() {
        let out = to_provider_jsonl(&stored_line("q", "a"), ProviderFormat::Conversation)
            .expect("export");
        let parsed: serde_json::Value = serde_json::from_str(out.trim()).expect("json");
        for m in parsed["messages"].as_array().expect("array") {
            let keys: Vec<&String> = m.as_object().expect("obj").keys().collect();
            assert_eq!(keys.len(), 2, "got {keys:?}");
            assert!(m.get("role").is_some() && m.get("content").is_some());
        }
    }

    #[test]
    fn roles_serialize_as_the_upstream_spells_them() {
        let out = to_provider_jsonl(&stored_line("q", "a"), ProviderFormat::Conversation)
            .expect("export");
        assert!(out.contains(r#""role":"user""#), "got {out}");
        assert!(out.contains(r#""role":"assistant""#), "got {out}");
    }

    #[test]
    fn every_line_is_projected_and_blank_lines_are_skipped() {
        let stored = format!(
            "{}\n\n{}\n",
            stored_line("q1", "a1"),
            stored_line("q2", "a2")
        );
        let out = to_provider_jsonl(&stored, ProviderFormat::Conversation).expect("export");
        assert_eq!(out.lines().count(), 2);
    }

    /// Fail locally with a line number rather than paying a round trip to be told
    /// "Found extra column" with no idea where.
    #[test]
    fn a_malformed_line_names_its_line_number() {
        let stored = format!("{}\n{{ not json\n", stored_line("q", "a"));
        let err = to_provider_jsonl(&stored, ProviderFormat::Conversation).expect_err("must fail");
        assert!(
            matches!(err, ExportError::Unparsable { line: 2, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn an_example_with_no_assistant_turn_is_refused() {
        let ex = Example {
            messages: vec![Message::user("just a question")],
            provenance: Provenance::Synthetic {
                template: "t".into(),
            },
            instruction_kind: InstructionKind::TemplatedTag,
        };
        let err = to_provider_jsonl(&ex.to_jsonl().expect("ser"), ProviderFormat::Conversation)
            .expect_err("must fail");
        assert!(
            matches!(err, ExportError::Invalid { line: 1, ref reason } if reason.contains("assistant")),
            "got {err:?}"
        );
    }

    #[test]
    fn empty_content_is_refused_with_the_offending_index() {
        let ex = Example {
            messages: vec![Message::user("q"), Message::assistant("   ")],
            provenance: Provenance::Synthetic {
                template: "t".into(),
            },
            instruction_kind: InstructionKind::TemplatedTag,
        };
        let err = to_provider_jsonl(&ex.to_jsonl().expect("ser"), ProviderFormat::Conversation)
            .expect_err("must fail");
        assert!(
            matches!(err, ExportError::Invalid { ref reason, .. } if reason.contains("message 1")),
            "got {err:?}"
        );
    }

    #[test]
    fn an_empty_dataset_is_refused_rather_than_uploaded() {
        let err = to_provider_jsonl("\n\n  \n", ProviderFormat::Conversation).expect_err("fail");
        assert_eq!(err, ExportError::Empty);
    }
}

//! The input shape: a memory as Puerperium consumes it.
//!
//! Deliberately **source-agnostic** — a `MemoryRecord` may arrive from the Cerebro MCP
//! surface, a JSON export, or a test fixture. The conversion pipeline never knows which,
//! which is what keeps it pure and testable without a running Cerebro.
//!
//! Mirrors the fields of `cerebro::models::MemoryNode` that bear on training data, and
//! deliberately drops the rest (ACT-R timestamps, FSRS strength, links, embeddings).

use serde::{Deserialize, Serialize};

/// Cerebro's memory taxonomy, mirrored exactly.
///
/// The serialized form matches Cerebro's own lowercase strings so an export deserializes
/// without a translation layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    Affective,
    Prospective,
    Schematic,
}

impl MemoryType {
    /// Every variant, for defaults and histograms.
    pub const ALL: [MemoryType; 6] = [
        MemoryType::Episodic,
        MemoryType::Semantic,
        MemoryType::Procedural,
        MemoryType::Affective,
        MemoryType::Prospective,
        MemoryType::Schematic,
    ];

    /// The types mined by default.
    ///
    /// Episodic is the **largest** class in a real store (194 of 349 when this was set) and
    /// is deliberately excluded: it is session narrative, and training on it teaches a model
    /// to recite what happened rather than to do the work. Prospective (intentions) and
    /// affective are likewise not capability-bearing. See `docs/design.md` §Conversion.
    pub const DEFAULT_INCLUDED: [MemoryType; 3] = [
        MemoryType::Procedural,
        MemoryType::Semantic,
        MemoryType::Schematic,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Procedural => "procedural",
            MemoryType::Affective => "affective",
            MemoryType::Prospective => "prospective",
            MemoryType::Schematic => "schematic",
        }
    }
}

/// One memory, as Puerperium consumes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub content: String,
    pub memory_type: MemoryType,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default = "default_salience")]
    pub salience: f32,
}

fn default_salience() -> f32 {
    0.5
}

impl MemoryRecord {
    /// Lowercased tags, for case-insensitive matching.
    pub fn tags_lower(&self) -> Vec<String> {
        self.tags.iter().map(|t| t.to_lowercase()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_type_roundtrips_through_cerebros_lowercase_form() {
        for ty in MemoryType::ALL {
            let json = serde_json::to_string(&ty).expect("serialize");
            assert_eq!(json, format!("\"{}\"", ty.as_str()));
            let back: MemoryType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, ty);
        }
    }

    #[test]
    fn record_deserializes_from_a_minimal_export_row() {
        // Only id/content/memory_type are guaranteed; the rest must default.
        let row = r#"{"id":"m1","content":"hello","memory_type":"procedural"}"#;
        let rec: MemoryRecord = serde_json::from_str(row).expect("deserialize");
        assert_eq!(rec.memory_type, MemoryType::Procedural);
        assert!(rec.tags.is_empty());
        assert_eq!(rec.agent_id, None);
        assert_eq!(rec.salience, 0.5);
    }

    #[test]
    fn episodic_is_not_included_by_default() {
        assert!(!MemoryType::DEFAULT_INCLUDED.contains(&MemoryType::Episodic));
        assert!(MemoryType::DEFAULT_INCLUDED.contains(&MemoryType::Procedural));
    }
}

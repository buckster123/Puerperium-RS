//! Memory → training examples.
//!
//! The whole pipeline is **pure**: it takes already-materialised `MemoryRecord`s and returns
//! examples plus a rejection ledger. It never talks to Cerebro, a filesystem or a network,
//! which is what lets it be tested against real captured content with no running daemon.
//!
//! Stages: [`filter`] (quality gate) → [`chunk`] (teachable units) → [`instruct`] (framing).

pub mod chunk;
pub mod filter;
pub mod instruct;

use std::collections::BTreeMap;

use crate::example::Example;
use crate::memory::MemoryRecord;
use crate::provenance::{InstructionKind, Provenance};

use chunk::ChunkConfig;
use filter::{FilterConfig, Rejection, RejectionLedger};
use instruct::InstructConfig;

/// Everything the conversion needs to be reproducible.
#[derive(Debug, Clone, Default)]
pub struct ConvertConfig {
    pub filter: FilterConfig,
    pub chunk: ChunkConfig,
    pub instruct: InstructConfig,
}

impl ConvertConfig {
    pub fn new() -> Self {
        Self {
            filter: FilterConfig::default(),
            chunk: ChunkConfig::default(),
            instruct: InstructConfig::new(),
        }
    }
}

/// The result of a conversion run: what was produced, and everything that was not.
#[derive(Debug, Default)]
pub struct Converted {
    pub examples: Vec<Example>,
    pub rejections: RejectionLedger,
    /// Memories that contributed at least one example.
    pub memories_used: usize,
    /// How the instructions were framed. Reported in the dataset sidecar so the
    /// strong (heading-derived) / weak (tag-derived) ratio is visible without
    /// re-reading the JSONL.
    pub framing: BTreeMap<InstructionKind, usize>,
    /// Unframeable is per-chunk. The rejection ledger counts the *memory* once
    /// when nothing could be framed, so `memories_used + rejections.total()`
    /// stays equal to the input length.
    pub unframeable_chunks: usize,
}

/// Convert a batch of memories into training examples.
///
/// Pure. Total accounting: every input memory either contributes examples or is counted in
/// `rejections` — a memory can never vanish silently.
pub fn convert(memories: &[MemoryRecord], cfg: &ConvertConfig) -> Converted {
    let mut out = Converted::default();

    for mem in memories {
        if let Err(reason) = filter::assess(mem, &cfg.filter) {
            out.rejections.record(reason);
            continue;
        }

        let chunks = chunk::chunk(&mem.content, &cfg.chunk);
        let before = out.examples.len();
        let mut unframeable = 0usize;

        for c in chunks {
            let Some((instruction, kind)) = instruct::instruction_for(&c, mem, &cfg.instruct)
            else {
                unframeable += 1;
                continue;
            };

            *out.framing.entry(kind).or_insert(0) += 1;
            out.examples.push(Example::instruction(
                instruction,
                c.body.clone(),
                Provenance::CerebroMemory {
                    memory_id: mem.id.clone(),
                    agent_id: mem.agent_id.clone(),
                    heading_path: c.heading_path,
                },
                kind,
            ));
        }

        out.unframeable_chunks += unframeable;
        if out.examples.len() > before {
            out.memories_used += 1;
        } else {
            // The memory produced nothing teachable. Count it once, not once per chunk.
            out.rejections.record(Rejection::Unframeable);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryType;

    fn mem(id: &str, content: &str, ty: MemoryType, tags: &[&str]) -> MemoryRecord {
        MemoryRecord {
            id: id.into(),
            content: content.into(),
            memory_type: ty,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            agent_id: Some("CLAUDE".into()),
            salience: 0.8,
        }
    }

    #[test]
    fn a_sectioned_procedural_memory_yields_one_example_per_section() {
        let doc = "DEPLOY REFERENCE\n\
                   \n## Building\n\
                   \nAlways build on the target board; an x86 binary gives Exec format error \
                   and that failure reads like a corrupt file rather than a wrong architecture.\n\
                   \n## Swapping\n\
                   \nStop the service before copying the binary, or the copy fails with text \
                   file busy because a running binary cannot be overwritten in place.\n";

        let got = convert(
            &[mem("m1", doc, MemoryType::Procedural, &[])],
            &ConvertConfig::new(),
        );

        assert_eq!(got.examples.len(), 2);
        assert_eq!(got.memories_used, 1);
        assert_eq!(got.rejections.total(), 0);

        assert_eq!(
            got.examples[0].messages[0].content,
            "Explain Building, in the context of DEPLOY REFERENCE."
        );
        assert!(got.examples[0].messages[1]
            .content
            .starts_with("Always build"));

        // Provenance carries the section trail, not just the memory id.
        match &got.examples[0].provenance {
            Provenance::CerebroMemory {
                memory_id,
                heading_path,
                agent_id,
            } => {
                assert_eq!(memory_id, "m1");
                assert_eq!(agent_id.as_deref(), Some("CLAUDE"));
                assert_eq!(heading_path, &["DEPLOY REFERENCE", "Building"]);
            }
            other => panic!("wrong provenance: {other:?}"),
        }
    }

    #[test]
    fn accounting_is_total_nothing_vanishes() {
        let long = "A sufficiently long and genuinely useful operational note that clears the \
                    minimum content threshold without any trouble at all whatsoever.";
        let batch = vec![
            mem("keep", long, MemoryType::Procedural, &["deploy"]),
            mem("short", "nope", MemoryType::Semantic, &[]),
            mem("episodic", long, MemoryType::Episodic, &[]),
            mem(
                "chatty",
                &format!("Hey FORGE! {long} Can you hear me?"),
                MemoryType::Semantic,
                &[],
            ),
        ];

        let got = convert(&batch, &ConvertConfig::new());

        assert_eq!(got.memories_used, 1);
        assert_eq!(got.rejections.count_of(Rejection::TooShort), 1);
        assert_eq!(got.rejections.count_of(Rejection::TypeExcluded), 1);
        assert_eq!(got.rejections.count_of(Rejection::Chatter), 1);
        assert_eq!(got.rejections.total(), 3);
        assert_eq!(got.memories_used + got.rejections.total(), batch.len());
    }

    #[test]
    fn unframeable_chunk_is_counted_not_dropped_silently() {
        // No heading trail (first line ends with a period → not a title) and no tags.
        let body = "This is a plain paragraph of prose that ends in a period. It carries no \
                    headings and the memory carries no tags, so nothing can frame it.";
        let got = convert(
            &[mem("m1", body, MemoryType::Semantic, &[])],
            &ConvertConfig::new(),
        );

        assert!(got.examples.is_empty());
        assert_eq!(got.rejections.count_of(Rejection::Unframeable), 1);
        assert_eq!(got.unframeable_chunks, 1);
        assert_eq!(got.memories_used, 0);
        assert_eq!(got.memories_used + got.rejections.total(), 1);
    }

    #[test]
    fn two_unframeable_chunks_count_as_one_memory() {
        // First line ends with a period → no title → empty heading path → no tags
        // → unframeable. A small max_chunk splits the two paragraphs into two chunks.
        let p1 = "This is a plain paragraph of prose that ends in a period. It carries no \
                  headings and the memory carries no tags, so nothing can frame it at all.";
        let p2 = "A second paragraph of equally unframeable prose that also ends in a period. \
                  Together they are long enough to split, and neither can become an example.";
        let mut cfg = ConvertConfig::new();
        cfg.chunk.max_chunk = 180;
        let got = convert(
            &[mem(
                "m1",
                &format!("{p1}\n\n{p2}"),
                MemoryType::Semantic,
                &[],
            )],
            &cfg,
        );

        assert!(got.examples.is_empty());
        assert!(
            got.unframeable_chunks >= 2,
            "expected a split, got {} chunks",
            got.unframeable_chunks
        );
        assert_eq!(
            got.rejections.count_of(Rejection::Unframeable),
            1,
            "the memory is one rejection, not one per chunk"
        );
        assert_eq!(got.memories_used + got.rejections.total(), 1);
    }

    /// FORGE's lived procedures carry useful tags *and* a labelled banner.
    /// The banner is the stronger frame — tags are the fallback, not the winner.
    #[test]
    fn labelled_banner_frames_as_heading_not_tags() {
        let body = "PROCEDURE — Shipping a roadmap phase in the ApexOS Rust repos.\n\n\
                    WHEN: André approves a phase from a docs/build-roadmap.md row. \
                    Lock the design first if the arc is new, then one branch per \
                    slice, and do not start the next row until this one is merged.";
        let got = convert(
            &[mem(
                "m1",
                body,
                MemoryType::Procedural,
                &["apexos", "workflow", "forge"],
            )],
            &ConvertConfig::new(),
        );

        assert_eq!(got.examples.len(), 1);
        assert_eq!(
            got.framing.get(&InstructionKind::TemplatedHeading).copied(),
            Some(1)
        );
        assert!(!got.framing.contains_key(&InstructionKind::TemplatedTag));
        assert!(
            got.examples[0].messages[0]
                .content
                .starts_with("Explain: PROCEDURE — Shipping a roadmap phase"),
            "got {}",
            got.examples[0].messages[0].content
        );
    }

    #[test]
    fn tags_rescue_an_otherwise_unframeable_memory() {
        let body = "This is a plain paragraph of prose that ends in a period. It carries no \
                    headings, but the memory does carry tags to frame it with.";
        let got = convert(
            &[mem(
                "m1",
                body,
                MemoryType::Semantic,
                &["mesh", "federation"],
            )],
            &ConvertConfig::new(),
        );

        assert_eq!(got.examples.len(), 1);
        assert_eq!(
            got.examples[0].messages[0].content,
            "What do you know about mesh and federation?"
        );
    }
}

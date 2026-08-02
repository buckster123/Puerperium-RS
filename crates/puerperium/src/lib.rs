//! Puerperium-RS core library — the model nursery.
//!
//! Turns an agent's remembered experience into training data, with lineage good enough to
//! answer *"why is this specialist like this?"*.
//!
//! All logic lives here; the `-mcp` / `-cli` crates are thin faces over it. See
//! `docs/design.md` for the contract and `docs/CHARTER.md` for the binding decisions.
//!
//! # Shape
//!
//! - [`memory`] — the source-agnostic input record
//! - [`convert`] — the **pure** pipeline: quality gate → chunking → instruction framing
//! - [`example`] — the training example and its sharegpt wire form
//! - [`dataset`] — writing, hashing and listing datasets
//! - [`provenance`] — where every example came from (charter D12)
//!
//! # The pure core
//!
//! [`convert::convert`] takes already-materialised memories and returns examples plus a
//! rejection ledger. It performs no I/O, so it is testable against real captured content
//! with no running Cerebro — and the accounting is **total**: every input memory either
//! contributes examples or is counted as a rejection.
//!
//! ```
//! use puerperium::convert::{convert, ConvertConfig};
//! use puerperium::memory::{MemoryRecord, MemoryType};
//!
//! let doc = "DEPLOY REFERENCE\n\n## Building\n\nAlways build on the target board; an \
//!            x86 binary gives Exec format error, which reads like a corrupt file.\n";
//! let mem = MemoryRecord {
//!     id: "m1".into(),
//!     content: doc.into(),
//!     memory_type: MemoryType::Procedural,
//!     tags: vec!["deploy".into()],
//!     agent_id: Some("CLAUDE".into()),
//!     salience: 0.9,
//! };
//!
//! let out = convert(&[mem], &ConvertConfig::new());
//! assert_eq!(out.examples.len(), 1);
//! assert_eq!(
//!     out.examples[0].messages[0].content,
//!     "Explain Building, in the context of DEPLOY REFERENCE."
//! );
//! ```

pub mod apprentice;
pub mod convert;
pub mod dataset;
pub mod engine;
pub mod error;
pub mod estimate;
pub mod example;
pub mod export;
pub mod job;
pub mod memory;
pub mod paths;
pub mod provenance;
pub mod provider;
pub mod registry;
pub mod secrets;
pub mod source;
pub mod store;

pub use error::{Error, Result};

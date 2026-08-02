//! The registry: models, apprentices, and the lineage that joins them to their data.
//!
//! # Facts only (charter D3)
//!
//! A [`ModelRecord`] **never stores whether the model is deployed, live, or serving.**
//! Whether an alias actually answers is ApexRouter's truth, not ours — it depends on a
//! process we do not supervise, on a box we did not rent (D2/D4). A `deployed: true` on disk
//! is a lie the moment Router restarts, the tunnel drops, or the box is parked. The record
//! stores what Puerperium *did*: the artifact, the alias it **requested**, the dataset hash,
//! the trainer, the parent.
//!
//! Same shape for apprentices: there is no `trained: bool`, because that is `model.is_some()`.
//! Any boolean that restates another field is a chance for the two to disagree.
//!
//! # Lineage is the product
//!
//! [`lineage`] answers *"why is this specialist like this?"* — the ancestor chain, and at each
//! generation the dataset by hash and how many memories fed it. It **degrades honestly**: a
//! missing dataset, a missing parent, or a hand-edited cycle are all recorded and reported,
//! never silently skipped and never fatal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::dataset::{self, DatasetRef};
use crate::error::Result;
use crate::paths::Paths;
use crate::store;

/// A registered adapter. Facts only — no liveness, no deployment status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRecord {
    /// Registry key, and the candidate alias if it is ever registered with Router.
    pub name: String,
    pub base_model: String,
    /// Name + `sha256`. The hash is the identity — a name can be reused, a hash cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<DatasetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Who ordered the training (D6). **Never** `agent_id` — agentd stamps that field.
    pub trainer_agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PathBuf>,
    /// The model this one was trained from, for multi-generation lineage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// What we asked Router for. **Not** proof that it is live — see the module docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_requested: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ModelRecord {
    pub fn new(
        name: impl Into<String>,
        base_model: impl Into<String>,
        trainer: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_model: base_model.into(),
            dataset: None,
            job_id: None,
            trainer_agent: trainer.into(),
            artifact: None,
            parent: None,
            alias_requested: None,
            created_at: Utc::now(),
        }
    }
}

/// A specialist raised from a master agent's own remembered experience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprenticeRecord {
    pub id: String,
    /// Whose knowledge it was raised on — the mined memory space, not the trainer (D6).
    pub master_agent: String,
    pub name: String,
    pub specialization: String,
    pub base_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<DatasetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// The [`ModelRecord`] name, once trained. `None` means not yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ApprenticeRecord {
    /// Derived, never stored — a `trained: bool` field could disagree with `model`.
    pub fn is_trained(&self) -> bool {
        self.model.is_some()
    }
}

// ---------------------------------------------------------------- models

pub fn save_model(paths: &Paths, record: &ModelRecord) -> Result<()> {
    store::save(&paths.models(), &record.name, record)
}

pub fn load_model(paths: &Paths, name: &str) -> Result<ModelRecord> {
    store::load(&paths.models(), name)
}

pub fn model_exists(paths: &Paths, name: &str) -> bool {
    store::exists(&paths.models(), name)
}

/// All models, newest first.
pub fn list_models(paths: &Paths) -> Result<Vec<ModelRecord>> {
    let mut all: Vec<ModelRecord> = store::list(&paths.models())?;
    all.sort_by_key(|m| std::cmp::Reverse(m.created_at));
    Ok(all)
}

// ----------------------------------------------------------- apprentices

pub fn save_apprentice(paths: &Paths, record: &ApprenticeRecord) -> Result<()> {
    store::save(&paths.apprentices(), &record.id, record)
}

pub fn load_apprentice(paths: &Paths, id: &str) -> Result<ApprenticeRecord> {
    store::load(&paths.apprentices(), id)
}

/// All apprentices, newest first.
pub fn list_apprentices(paths: &Paths) -> Result<Vec<ApprenticeRecord>> {
    let mut all: Vec<ApprenticeRecord> = store::list(&paths.apprentices())?;
    all.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    Ok(all)
}

// --------------------------------------------------------------- lineage

/// One generation in a model's ancestry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEntry {
    /// 0 is the model asked about; 1 its parent, and so on.
    pub generation: usize,
    pub model: String,
    pub base_model: String,
    pub trainer_agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<DatasetRef>,
    /// From the dataset sidecar, when it is still on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_examples: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_memories: Option<usize>,
    /// The record names a dataset that is no longer here, or whose hash has changed.
    /// Recorded rather than hidden — a lineage that quietly omits a broken link is worse
    /// than one that says the link is broken.
    #[serde(default)]
    pub dataset_missing: bool,
    #[serde(default)]
    pub dataset_hash_mismatch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A model's full ancestry, as far as it could be walked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lineage {
    pub entries: Vec<LineageEntry>,
    /// Why the walk stopped early. `None` means it reached a root honestly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<String>,
}

/// Walk a model back through its ancestors.
///
/// Errors only if the *starting* model is missing — everything after that degrades into
/// `incomplete` so a partially-broken registry still answers as much as it can.
///
/// The cycle guard is not optional: records are plain JSON on disk and nothing stops a human
/// from pointing two models at each other.
pub fn lineage(paths: &Paths, model: &str) -> Result<Lineage> {
    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut incomplete = None;
    let mut cursor = Some(model.to_string());
    let mut generation = 0usize;

    // The starting model must exist; a typo should say so rather than return an empty walk.
    let mut record = load_model(paths, model)?;

    while let Some(name) = cursor.clone() {
        if !seen.insert(name.clone()) {
            incomplete = Some(format!(
                "parent cycle detected at {name:?} — a record points back at an ancestor"
            ));
            break;
        }

        let (examples, memories, missing, mismatch) =
            resolve_dataset(&paths.datasets(), record.dataset.as_ref());

        entries.push(LineageEntry {
            generation,
            model: record.name.clone(),
            base_model: record.base_model.clone(),
            trainer_agent: record.trainer_agent.clone(),
            dataset: record.dataset.clone(),
            dataset_examples: examples,
            dataset_memories: memories,
            dataset_missing: missing,
            dataset_hash_mismatch: mismatch,
            job_id: record.job_id.clone(),
            created_at: record.created_at,
        });

        let Some(parent) = record.parent.clone() else {
            break;
        };
        match load_model(paths, &parent) {
            Ok(next) => {
                record = next;
                cursor = Some(parent);
                generation += 1;
            }
            Err(_) => {
                incomplete = Some(format!(
                    "parent {parent:?} of {:?} is not in the registry",
                    record.name
                ));
                break;
            }
        }
    }

    Ok(Lineage {
        entries,
        incomplete,
    })
}

/// Look up a dataset sidecar: `(examples, memories, missing, hash_mismatch)`.
///
/// A hash mismatch means the dataset on disk is not the one this model was trained on. That
/// is exactly the case lineage exists to catch, so it is reported, never repaired.
fn resolve_dataset(
    dir: &Path,
    reference: Option<&DatasetRef>,
) -> (Option<usize>, Option<usize>, bool, bool) {
    let Some(r) = reference else {
        return (None, None, false, false);
    };
    match dataset::read_meta(dir, &r.name) {
        Ok(meta) => (
            Some(meta.example_count),
            Some(meta.memories_used),
            false,
            meta.sha256 != r.sha256,
        ),
        Err(_) => (None, None, true, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{convert, ConvertConfig};
    use crate::dataset::SourceSpec;
    use crate::memory::{MemoryRecord, MemoryType};

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = Paths::new(dir.path());
        (dir, p)
    }

    fn model(name: &str, parent: Option<&str>) -> ModelRecord {
        ModelRecord {
            parent: parent.map(|s| s.to_string()),
            ..ModelRecord::new(name, "Qwen/Qwen3.6-27B", "FORGE")
        }
    }

    /// Write a real dataset so lineage has something true to resolve against.
    fn write_dataset(p: &Paths, name: &str) -> DatasetRef {
        let doc = "DEPLOY REFERENCE\n\n## Building\n\nAlways build on the target board; an x86 \
                   binary gives Exec format error, which reads like a corrupt file rather than \
                   a wrong architecture.\n";
        let mem = MemoryRecord {
            id: "m1".into(),
            content: doc.into(),
            memory_type: MemoryType::Procedural,
            tags: vec!["deploy".into()],
            agent_id: Some("CLAUDE".into()),
            salience: 0.9,
        };
        let converted = convert(&[mem], &ConvertConfig::new());
        let source = SourceSpec {
            kind: "export_file".into(),
            query: None,
            agent_id: Some("CLAUDE".into()),
            memories_in: 1,
        };
        dataset::write(&p.datasets(), name, &converted, source)
            .expect("write dataset")
            .dataset_ref()
    }

    #[test]
    fn model_crud_roundtrips() {
        let (_d, p) = paths();
        let m = model("tool-forge-v1", None);
        save_model(&p, &m).expect("save");
        assert!(model_exists(&p, "tool-forge-v1"));
        assert_eq!(load_model(&p, "tool-forge-v1").expect("load"), m);
    }

    #[test]
    fn lists_are_valid_when_empty_and_when_populated() {
        let (_d, p) = paths();
        assert!(list_models(&p).expect("empty models").is_empty());
        assert!(list_apprentices(&p).expect("empty apprentices").is_empty());

        save_model(&p, &model("a", None)).expect("save a");
        save_model(&p, &model("b", None)).expect("save b");
        assert_eq!(list_models(&p).expect("list").len(), 2);
    }

    #[test]
    fn apprentice_trained_state_is_derived_not_stored() {
        let (_d, p) = paths();
        let mut a = ApprenticeRecord {
            id: "ap1".into(),
            master_agent: "FORGE".into(),
            name: "tool_forge".into(),
            specialization: "tool calling".into(),
            base_model: "Qwen/Qwen3.6-27B".into(),
            dataset: None,
            job_id: None,
            model: None,
            created_at: Utc::now(),
        };
        assert!(!a.is_trained());

        a.model = Some("tool-forge-v1".into());
        save_apprentice(&p, &a).expect("save");
        assert!(load_apprentice(&p, "ap1").expect("load").is_trained());

        // The serialized form carries no `trained` field to drift out of sync.
        let json = serde_json::to_string(&a).expect("ser");
        assert!(
            !json.contains("trained"),
            "derived state must not be persisted: {json}"
        );
    }

    /// A model record must never claim to be live — that is Router's truth, not ours.
    #[test]
    fn model_record_has_no_liveness_field_on_the_wire() {
        let mut m = model("x", None);
        m.alias_requested = Some("tool-forge".into());
        let json = serde_json::to_string(&m).expect("ser");
        for forbidden in ["deployed", "live", "serving", "\"status\""] {
            assert!(
                !json.contains(forbidden),
                "{forbidden} must not be persisted: {json}"
            );
        }
        // What we *asked* for is a fact and does ride along.
        assert!(json.contains("alias_requested"));
    }

    #[test]
    fn lineage_walks_generations_and_resolves_the_dataset() {
        let (_d, p) = paths();
        let dref = write_dataset(&p, "gen0-data");

        save_model(&p, &model("base-v1", None)).expect("save parent");
        let mut child = model("tool-forge-v2", Some("base-v1"));
        child.dataset = Some(dref.clone());
        save_model(&p, &child).expect("save child");

        let lin = lineage(&p, "tool-forge-v2").expect("lineage");
        assert_eq!(lin.incomplete, None, "should reach the root cleanly");
        assert_eq!(lin.entries.len(), 2);

        assert_eq!(lin.entries[0].generation, 0);
        assert_eq!(lin.entries[0].model, "tool-forge-v2");
        assert_eq!(lin.entries[0].dataset.as_ref(), Some(&dref));
        assert_eq!(lin.entries[0].dataset_examples, Some(1));
        assert_eq!(lin.entries[0].dataset_memories, Some(1));
        assert!(!lin.entries[0].dataset_missing);
        assert!(!lin.entries[0].dataset_hash_mismatch);

        assert_eq!(lin.entries[1].generation, 1);
        assert_eq!(lin.entries[1].model, "base-v1");
    }

    #[test]
    fn lineage_reports_a_missing_dataset_instead_of_hiding_it() {
        let (_d, p) = paths();
        let mut m = model("orphan", None);
        m.dataset = Some(DatasetRef {
            name: "deleted-set".into(),
            sha256: "abc".into(),
        });
        save_model(&p, &m).expect("save");

        let lin = lineage(&p, "orphan").expect("lineage");
        assert!(lin.entries[0].dataset_missing);
        assert_eq!(lin.entries[0].dataset_examples, None);
    }

    /// The case lineage exists to catch: the dataset on disk is not the one trained on.
    #[test]
    fn lineage_flags_a_dataset_whose_hash_no_longer_matches() {
        let (_d, p) = paths();
        write_dataset(&p, "gen0-data");

        let mut m = model("drifted", None);
        m.dataset = Some(DatasetRef {
            name: "gen0-data".into(),
            sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        });
        save_model(&p, &m).expect("save");

        let lin = lineage(&p, "drifted").expect("lineage");
        assert!(!lin.entries[0].dataset_missing, "the file is there");
        assert!(
            lin.entries[0].dataset_hash_mismatch,
            "but it is not the one trained on"
        );
    }

    #[test]
    fn lineage_reports_a_missing_parent_rather_than_erroring() {
        let (_d, p) = paths();
        save_model(&p, &model("child", Some("ghost"))).expect("save");

        let lin = lineage(&p, "child").expect("must not error");
        assert_eq!(lin.entries.len(), 1);
        let reason = lin.incomplete.expect("should say why it stopped");
        assert!(
            reason.contains("ghost"),
            "reason should name the missing parent: {reason}"
        );
    }

    /// Records are hand-editable JSON; nothing stops two models pointing at each other.
    #[test]
    fn lineage_survives_a_parent_cycle() {
        let (_d, p) = paths();
        save_model(&p, &model("a", Some("b"))).expect("save a");
        save_model(&p, &model("b", Some("a"))).expect("save b");

        let lin = lineage(&p, "a").expect("must terminate, not hang");
        assert_eq!(lin.entries.len(), 2);
        let reason = lin.incomplete.expect("should report the cycle");
        assert!(reason.contains("cycle"), "got {reason}");
    }

    #[test]
    fn lineage_of_an_unknown_model_is_an_error_not_an_empty_walk() {
        let (_d, p) = paths();
        assert!(lineage(&p, "nope").is_err());
    }
}

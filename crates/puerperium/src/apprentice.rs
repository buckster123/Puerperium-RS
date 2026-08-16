//! The apprentice protocol: raising a specialist from an agent's own remembered experience.
//!
//! This is the headline verb, and it deliberately **adds no new capability**. It composes
//! what the earlier slices built — mine, convert, write, register — and its whole value is
//! that the composition leaves a record you can trace afterwards.
//!
//! # It stops before spending
//!
//! Creating an apprentice mines, builds a dataset, and registers a record. It does **not**
//! submit a training job. Training costs money, so it stays a separate, explicit act
//! (charter D4) — the apprentice exists in an untrained state, honestly, and
//! [`attach_job`]/[`attach_model`] record what happens to it later.
//!
//! `ApprenticeRecord::is_trained()` is derived from `model.is_some()`, never stored (D3).

use chrono::Utc;

use crate::convert::{convert, ConvertConfig, Converted};
use crate::dataset::{self, SourceSpec};
use crate::error::{Error, Result};
use crate::job;
use crate::memory::MemoryRecord;
use crate::paths::Paths;
use crate::registry::{self, ApprenticeRecord};

/// What to raise, and from whose experience.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Registry key for the apprentice.
    pub id: String,
    /// Whose memory space was mined. **Not** the trainer (charter D6).
    pub master_agent: String,
    pub name: String,
    /// What this apprentice is for, in the operator's words. Recorded verbatim; it is the
    /// thing a future reader will actually search on.
    pub specialization: String,
    pub base_model: String,
    /// Name for the dataset this run produces. Datasets are immutable, so re-running under
    /// the same name is refused rather than silently rebuilt.
    pub dataset_name: String,
}

/// What a creation run produced, beyond the record itself.
#[derive(Debug)]
pub struct Created {
    pub apprentice: ApprenticeRecord,
    /// The conversion accounting, so a thin result is explainable on the spot.
    pub converted: Converted,
    pub memories_in: usize,
}

/// Mine → convert → dataset → apprentice record.
///
/// `memories` come from any source (see [`crate::source`]); this function does no I/O beyond
/// writing the dataset and the record, which is what keeps it testable.
///
/// Refuses when the conversion yields nothing, rather than registering an apprentice with no
/// data behind it — an untrainable record is worse than no record, because it looks like
/// progress.
pub fn create(
    paths: &Paths,
    spec: Spec,
    memories: &[MemoryRecord],
    cfg: &ConvertConfig,
) -> Result<Created> {
    if registry::apprentice_exists(paths, &spec.id) {
        return Err(Error::AlreadyExists {
            what: "apprentice".into(),
            name: spec.id,
        });
    }

    let converted = convert(memories, cfg);
    if converted.examples.is_empty() {
        return Err(Error::NoExamples {
            rejected: converted.rejections.total(),
        });
    }

    let source = SourceSpec {
        kind: "cerebro_query".into(),
        query: Some(spec.specialization.clone()),
        // Whose memories were mined — the master, not the trainer (D6).
        agent_id: Some(spec.master_agent.clone()),
        memories_in: memories.len(),
    };
    let meta = dataset::write(&paths.datasets(), &spec.dataset_name, &converted, source)?;

    let apprentice = ApprenticeRecord {
        id: spec.id,
        master_agent: spec.master_agent,
        name: spec.name,
        specialization: spec.specialization,
        base_model: spec.base_model,
        dataset: Some(meta.dataset_ref()),
        job_id: None,
        // Untrained, and honest about it. Training is a separate, explicit, paid act.
        model: None,
        created_at: Utc::now(),
    };
    registry::save_apprentice(paths, &apprentice)?;

    Ok(Created {
        apprentice,
        converted,
        memories_in: memories.len(),
    })
}

/// Record that a training job was started for this apprentice.
pub fn attach_job(paths: &Paths, id: &str, job_id: &str) -> Result<ApprenticeRecord> {
    // Refuse a dangling job the same way attach_model refuses a missing model —
    // lineage that points at work that never happened is a lie.
    job::load(paths.root(), job_id)?;
    let mut record = registry::load_apprentice(paths, id)?;
    record.job_id = Some(job_id.to_string());
    registry::save_apprentice(paths, &record)?;
    Ok(record)
}

/// Record the model an apprentice grew into. This is what makes it trained.
///
/// Refuses to point at a model that is not in the registry — a dangling reference would
/// break the lineage walk at exactly the generation someone is asking about.
pub fn attach_model(paths: &Paths, id: &str, model_name: &str) -> Result<ApprenticeRecord> {
    if !registry::model_exists(paths, model_name) {
        return Err(Error::RecordNotFound {
            dir: paths.models(),
            name: model_name.to_string(),
        });
    }
    let mut record = registry::load_apprentice(paths, id)?;
    record.model = Some(model_name.to_string());
    registry::save_apprentice(paths, &record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::DatasetRef;
    use crate::job::{self, ComputeRef, Hyperparams, JobRecord, Method, Provider};
    use crate::memory::MemoryType;
    use crate::registry::ModelRecord;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = Paths::new(dir.path());
        (dir, p)
    }

    fn spec(id: &str, dataset: &str) -> Spec {
        Spec {
            id: id.into(),
            master_agent: "FORGE".into(),
            name: "tool_forge".into(),
            specialization: "ApexOS deployment procedures".into(),
            base_model: "Qwen/Qwen3.6-27B".into(),
            dataset_name: dataset.into(),
        }
    }

    fn memories() -> Vec<MemoryRecord> {
        let doc = "DEPLOY REFERENCE\n\n## Building\n\nAlways build on the target board; an x86 \
                   binary gives Exec format error, which reads like a corrupt file rather than \
                   a wrong architecture.\n\n## Swapping\n\nStop the service before copying the \
                   binary, or the copy fails with text file busy.\n";
        vec![MemoryRecord {
            id: "m1".into(),
            content: doc.into(),
            memory_type: MemoryType::Procedural,
            tags: vec!["deploy".into()],
            agent_id: Some("FORGE".into()),
            salience: 0.9,
        }]
    }

    fn write_job(p: &Paths, id: &str) {
        job::append(
            p.root(),
            &JobRecord {
                id: id.into(),
                provider: Provider::Together,
                provider_job_id: Some("ft-1".into()),
                dataset: DatasetRef {
                    name: "d".into(),
                    sha256: "abc".into(),
                },
                base_model: "Qwen/Qwen3.6-35B-A3B".into(),
                output_name: "worker-v1".into(),
                method: Method::LoraSft,
                hyperparams: Hyperparams::default(),
                trainer_agent: "FORGE".into(),
                compute: ComputeRef::Managed,
                submitted_at: Utc::now(),
                terminal: None,
                cancel_requested_at: None,
                ledger_refs: vec![],
            },
        )
        .expect("write job");
    }

    #[test]
    fn creation_produces_a_lineage_complete_record() {
        let (_d, p) = paths();
        let got = create(
            &p,
            spec("ap1", "ap1-data"),
            &memories(),
            &ConvertConfig::new(),
        )
        .expect("create");

        let a = &got.apprentice;
        assert_eq!(a.master_agent, "FORGE");
        assert_eq!(a.specialization, "ApexOS deployment procedures");

        // The dataset hash is the link that makes lineage answerable (D12).
        let dref = a.dataset.as_ref().expect("dataset recorded");
        assert_eq!(dref.name, "ap1-data");
        assert_eq!(dref.sha256.len(), 64);

        // And it resolves: the dataset really is on disk with that hash.
        let meta = dataset::read_meta(&p.datasets(), "ap1-data").expect("sidecar");
        assert_eq!(meta.sha256, dref.sha256);
        assert_eq!(meta.example_count, 2, "two sections, two examples");
    }

    /// Creating an apprentice must not spend money.
    #[test]
    fn a_new_apprentice_is_untrained_and_has_no_job() {
        let (_d, p) = paths();
        let got = create(
            &p,
            spec("ap1", "ap1-data"),
            &memories(),
            &ConvertConfig::new(),
        )
        .expect("create");
        assert!(!got.apprentice.is_trained());
        assert_eq!(got.apprentice.job_id, None);
        assert_eq!(got.apprentice.model, None);
    }

    #[test]
    fn the_master_agent_is_recorded_as_the_mined_space_not_the_trainer() {
        let (_d, p) = paths();
        create(
            &p,
            spec("ap1", "ap1-data"),
            &memories(),
            &ConvertConfig::new(),
        )
        .expect("create");
        let meta = dataset::read_meta(&p.datasets(), "ap1-data").expect("sidecar");
        assert_eq!(meta.source.agent_id.as_deref(), Some("FORGE"));
        assert_eq!(meta.source.kind, "cerebro_query");
    }

    /// A record with no data behind it looks like progress and is not.
    #[test]
    fn refuses_when_nothing_survives_the_quality_gate() {
        let (_d, p) = paths();
        let chatter = vec![MemoryRecord {
            id: "m1".into(),
            content: "Hey FORGE! Just a quick smoke test, can you hear me? Testing the mesh \
                      here on this fine morning, hope all is well over there."
                .into(),
            memory_type: MemoryType::Semantic,
            tags: vec![],
            agent_id: Some("FORGE".into()),
            salience: 0.9,
        }];

        let err = create(&p, spec("ap1", "ap1-data"), &chatter, &ConvertConfig::new())
            .expect_err("must refuse");
        assert!(matches!(err, Error::NoExamples { .. }), "got {err:?}");
        assert!(
            registry::list_apprentices(&p).expect("list").is_empty(),
            "no record should survive a refused creation"
        );
    }

    #[test]
    fn creating_the_same_apprentice_twice_is_refused() {
        let (_d, p) = paths();
        create(&p, spec("ap1", "d1"), &memories(), &ConvertConfig::new()).expect("first");
        let err = create(&p, spec("ap1", "d2"), &memories(), &ConvertConfig::new())
            .expect_err("second must refuse");
        assert!(matches!(err, Error::AlreadyExists { .. }), "got {err:?}");
    }

    #[test]
    fn attaching_a_job_then_a_model_walks_it_to_trained() {
        let (_d, p) = paths();
        create(
            &p,
            spec("ap1", "ap1-data"),
            &memories(),
            &ConvertConfig::new(),
        )
        .expect("create");

        write_job(&p, "j1");
        let with_job = attach_job(&p, "ap1", "j1").expect("attach job");
        assert_eq!(with_job.job_id.as_deref(), Some("j1"));
        assert!(!with_job.is_trained(), "a job is not yet a model");

        registry::save_model(
            &p,
            &ModelRecord::new("worker-v1", "Qwen/Qwen3.6-27B", "FORGE"),
        )
        .expect("save model");
        let trained = attach_model(&p, "ap1", "worker-v1").expect("attach model");
        assert!(trained.is_trained());
        assert_eq!(trained.model.as_deref(), Some("worker-v1"));
    }

    #[test]
    fn attaching_a_job_that_is_not_recorded_is_refused() {
        let (_d, p) = paths();
        create(
            &p,
            spec("ap1", "ap1-data"),
            &memories(),
            &ConvertConfig::new(),
        )
        .expect("create");
        let err = attach_job(&p, "ap1", "ghost").expect_err("must refuse");
        assert!(matches!(err, Error::RecordNotFound { .. }), "got {err:?}");
        assert!(registry::load_apprentice(&p, "ap1")
            .expect("load")
            .job_id
            .is_none());
    }

    /// A dangling model reference breaks the lineage walk at the generation being asked about.
    #[test]
    fn attaching_a_model_that_is_not_registered_is_refused() {
        let (_d, p) = paths();
        create(
            &p,
            spec("ap1", "ap1-data"),
            &memories(),
            &ConvertConfig::new(),
        )
        .expect("create");
        let err = attach_model(&p, "ap1", "ghost").expect_err("must refuse");
        assert!(matches!(err, Error::RecordNotFound { .. }), "got {err:?}");
        assert!(!registry::load_apprentice(&p, "ap1")
            .expect("load")
            .is_trained());
    }
}

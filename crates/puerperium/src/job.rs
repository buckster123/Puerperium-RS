//! Training jobs: the records, the append-only log, and the phase that is never stored.
//!
//! # Facts only (charter D3)
//!
//! A [`JobRecord`] holds what we did and what we observed — never a running status. [`Phase`]
//! is **computed** by asking the provider, because a persisted `"running"` is a lie the moment
//! the box dies. The single exception is [`Terminal`]: an *observed end state*, which is a
//! fact, written **once** and never revised.
//!
//! # Append-only (D3, and the ledger pattern)
//!
//! `jobs.jsonl` appends a full snapshot on every change; current state is a fold by id with
//! last-write-wins. Jobs are the money-adjacent records, so this mirrors ApexRouter's
//! `ledger.jsonl` — no rewrite can lose the fact that a job was ever submitted, and the
//! progression stays legible after the fact.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::dataset::DatasetRef;
use crate::error::{Error, Result};

/// Where a job runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Together,
    Vast,
    Local,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Together => "together",
            Provider::Vast => "vast",
            Provider::Local => "local",
        }
    }
}

/// v1 trains LoRA adapters. Full-parameter and preference methods are Stage 2 (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    LoraSft,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hyperparams {
    pub n_epochs: u32,
    pub learning_rate: f64,
    pub lora_r: u32,
    pub lora_alpha: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
}

impl Default for Hyperparams {
    fn default() -> Self {
        Self {
            n_epochs: 3,
            learning_rate: 1e-5,
            lora_r: 16,
            lora_alpha: 32,
            batch_size: None,
        }
    }
}

/// What a job runs on.
///
/// The distinction that matters is whether anything needs to already exist (D4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComputeRef {
    /// A hosted API — Together. No box, so nothing to check before submitting.
    Managed,
    /// A Router-known backend or tunnel. Must already exist; Puerperium never creates it.
    Node { name: String },
}

impl ComputeRef {
    /// Does submitting require compute that already exists?
    pub fn requires_existing(&self) -> bool {
        matches!(self, ComputeRef::Node { .. })
    }
}

/// An observed end state. The only status ever persisted, and written exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terminal {
    pub outcome: Outcome,
    pub observed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// The real reason, never a generic. `error` and `user_error` both land in `Failed`, and
    /// this is where the difference survives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// A training job. Facts only — no phase, no status string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub provider: Provider,
    /// `None` until the upstream accepts it. A record with `None` after a crash is a job that
    /// may or may not exist upstream — visible and recoverable, which is the point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_job_id: Option<String>,
    /// Name + `sha256`. The hash is the identity (D12).
    pub dataset: DatasetRef,
    pub base_model: String,
    pub output_name: String,
    pub method: Method,
    pub hyperparams: Hyperparams,
    /// Who ordered the training (D6). **Never** `agent_id`.
    pub trainer_agent: String,
    pub compute: ComputeRef,
    pub submitted_at: DateTime<Utc>,
    /// Written once, when a terminal state is observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<Terminal>,
    /// ApexRouter ledger rows, for cost attribution. Referenced, never duplicated — Router
    /// stays the single source of truth for what money happened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ledger_refs: Vec<String>,
}

impl JobRecord {
    pub fn is_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    /// The phase implied by the record alone, without asking the provider.
    ///
    /// Terminal jobs answer from the record; everything else is `None`, meaning *you must
    /// poll* — deliberately not a guess.
    pub fn terminal_phase(&self) -> Option<Phase> {
        self.terminal.as_ref().map(|t| match t.outcome {
            Outcome::Succeeded => Phase::Succeeded,
            Outcome::Failed => Phase::Failed,
            Outcome::Cancelled => Phase::Cancelled,
        })
    }
}

/// Computed on read, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Submitted,
    Provisioning,
    Running,
    /// Cancel asked for; the upstream is still working. Collapsing this into `Running` would
    /// tell an operator their cancel had not registered.
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    /// The provider was unreachable, **or returned a state we do not recognise**. A
    /// first-class honest answer — never silently rendered as `Running`.
    Unknown,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Submitted => "submitted",
            Phase::Provisioning => "provisioning",
            Phase::Running => "running",
            Phase::Cancelling => "cancelling",
            Phase::Succeeded => "succeeded",
            Phase::Failed => "failed",
            Phase::Cancelled => "cancelled",
            Phase::Unknown => "unknown",
        }
    }

    /// Does this phase end the job's life?
    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Succeeded | Phase::Failed | Phase::Cancelled)
    }

    pub fn outcome(self) -> Option<Outcome> {
        match self {
            Phase::Succeeded => Some(Outcome::Succeeded),
            Phase::Failed => Some(Outcome::Failed),
            Phase::Cancelled => Some(Outcome::Cancelled),
            _ => None,
        }
    }
}

// ------------------------------------------------------------ the log

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("jobs.jsonl")
}

/// Append a snapshot. The current state of a job is the last snapshot bearing its id.
pub fn append(dir: &Path, record: &JobRecord) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let path = log_path(dir);
    let mut line = serde_json::to_string(record)?;
    line.push('\n');

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    f.write_all(line.as_bytes())
        .map_err(|e| Error::io(&path, e))?;
    f.sync_all().map_err(|e| Error::io(&path, e))
}

/// Fold the log into current state, newest first.
///
/// A missing log is an **empty list, not an error**. A malformed line is **skipped, not
/// fatal** — one bad append must never make every job unreadable, and the surviving records
/// are still the truth about real submitted work.
pub fn load_all(dir: &Path) -> Result<Vec<JobRecord>> {
    let path = log_path(dir);
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&path, e)),
    };

    let mut folded: BTreeMap<String, JobRecord> = BTreeMap::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| Error::io(&path, e))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<JobRecord>(&line) {
            folded.insert(rec.id.clone(), rec);
        }
    }

    let mut out: Vec<JobRecord> = folded.into_values().collect();
    out.sort_by_key(|j| std::cmp::Reverse(j.submitted_at));
    Ok(out)
}

/// One job by id.
pub fn load(dir: &Path, id: &str) -> Result<JobRecord> {
    load_all(dir)?
        .into_iter()
        .find(|j| j.id == id)
        .ok_or_else(|| Error::RecordNotFound {
            dir: dir.to_path_buf(),
            name: id.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> JobRecord {
        JobRecord {
            id: id.into(),
            provider: Provider::Together,
            provider_job_id: None,
            dataset: DatasetRef {
                name: "d".into(),
                sha256: "abc".into(),
            },
            base_model: "Qwen/Qwen3.6-27B".into(),
            output_name: "worker-v1".into(),
            method: Method::LoraSft,
            hyperparams: Hyperparams::default(),
            trainer_agent: "FORGE".into(),
            compute: ComputeRef::Managed,
            submitted_at: Utc::now(),
            terminal: None,
            ledger_refs: vec![],
        }
    }

    #[test]
    fn a_fresh_record_carries_no_phase_on_the_wire() {
        let json = serde_json::to_string(&record("j1")).expect("ser");
        for forbidden in ["\"phase\"", "\"status\"", "running", "pending"] {
            assert!(
                !json.contains(forbidden),
                "{forbidden} must not persist: {json}"
            );
        }
    }

    #[test]
    fn log_folds_to_last_write_per_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut j = record("j1");
        append(dir.path(), &j).expect("append 1");

        j.provider_job_id = Some("ft-123".into());
        append(dir.path(), &j).expect("append 2");

        let all = load_all(dir.path()).expect("load");
        assert_eq!(all.len(), 1, "one job, two snapshots");
        assert_eq!(all[0].provider_job_id.as_deref(), Some("ft-123"));

        // Both snapshots survive on disk — the progression stays legible.
        let text = fs::read_to_string(log_path(dir.path())).expect("read");
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn a_malformed_line_does_not_make_every_job_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        append(dir.path(), &record("j1")).expect("append");
        let path = log_path(dir.path());
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open");
        writeln!(f, "{{ this is not json").expect("write junk");
        append(dir.path(), &record("j2")).expect("append after junk");

        let all = load_all(dir.path()).expect("must not fail");
        assert_eq!(all.len(), 2, "the good records still load");
    }

    #[test]
    fn missing_log_is_empty_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_all(&dir.path().join("nope"))
            .expect("must not error")
            .is_empty());
    }

    #[test]
    fn terminal_phase_answers_from_the_record_but_only_when_terminal() {
        let mut j = record("j1");
        assert_eq!(
            j.terminal_phase(),
            None,
            "non-terminal must not guess — poll instead"
        );

        j.terminal = Some(Terminal {
            outcome: Outcome::Succeeded,
            observed_at: Utc::now(),
            artifact: Some("adapters/worker-v1".into()),
            error: None,
        });
        assert_eq!(j.terminal_phase(), Some(Phase::Succeeded));
        assert!(j.is_terminal());
    }

    #[test]
    fn managed_compute_needs_nothing_to_exist_first() {
        assert!(!ComputeRef::Managed.requires_existing());
        assert!(ComputeRef::Node {
            name: "gpu-1".into()
        }
        .requires_existing());
    }

    #[test]
    fn unknown_is_not_terminal_and_has_no_outcome() {
        assert!(!Phase::Unknown.is_terminal());
        assert_eq!(Phase::Unknown.outcome(), None);
        assert!(
            !Phase::Cancelling.is_terminal(),
            "cancel requested is not cancelled"
        );
    }
}

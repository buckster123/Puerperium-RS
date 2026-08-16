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

/// New fields MUST carry `#[serde(default)]` (or `default = "…"`). A snapshot
/// written before the field existed has to keep loading — jobs are money-adjacent
/// and a parse failure used to hide them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hyperparams {
    #[serde(default = "default_n_epochs")]
    pub n_epochs: u32,
    #[serde(default = "default_learning_rate")]
    pub learning_rate: f64,
    #[serde(default = "default_lora_r")]
    pub lora_r: u32,
    #[serde(default = "default_lora_alpha")]
    pub lora_alpha: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
}

fn default_n_epochs() -> u32 {
    3
}
fn default_learning_rate() -> f64 {
    1e-5
}
fn default_lora_r() -> u32 {
    16
}
fn default_lora_alpha() -> u32 {
    32
}

impl Default for Hyperparams {
    fn default() -> Self {
        Self {
            n_epochs: default_n_epochs(),
            learning_rate: default_learning_rate(),
            lora_r: default_lora_r(),
            lora_alpha: default_lora_alpha(),
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
    /// When we asked the upstream to stop. A fact about *us*, not an outcome — only an
    /// observed `cancelled` from the provider is [`Outcome::Cancelled`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_requested_at: Option<DateTime<Utc>>,
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

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Succeeded => "succeeded",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
        }
    }
}

// ------------------------------------------------------------ the log

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("jobs.jsonl")
}

/// Append a snapshot. The current state of a job is the last snapshot bearing its id.
pub fn append(dir: &Path, record: &JobRecord) -> Result<()> {
    crate::store::ensure_dir(dir)?;
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
    f.sync_all().map_err(|e| Error::io(&path, e))?;
    crate::store::lock_private(&path)
}

/// A snapshot line that could not be parsed. Reported, never fatal — one bad append
/// must not make every job unreadable, and a schema bump must not hide a paid run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedSnapshot {
    pub line: usize,
    pub reason: String,
}

/// Current jobs plus any snapshots the fold could not read.
#[derive(Debug, Clone)]
pub struct JobLog {
    pub jobs: Vec<JobRecord>,
    pub skipped: Vec<SkippedSnapshot>,
}

/// Fold the log into current state, newest first, and name every line that did not parse.
///
/// A missing log is an **empty list, not an error**.
pub fn load_log(dir: &Path) -> Result<JobLog> {
    let path = log_path(dir);
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JobLog {
                jobs: Vec::new(),
                skipped: Vec::new(),
            })
        }
        Err(e) => return Err(Error::io(&path, e)),
    };

    let mut folded: BTreeMap<String, JobRecord> = BTreeMap::new();
    let mut skipped = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| Error::io(&path, e))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JobRecord>(&line) {
            Ok(rec) => {
                folded.insert(rec.id.clone(), rec);
            }
            Err(e) => skipped.push(SkippedSnapshot {
                line: idx + 1,
                reason: e.to_string(),
            }),
        }
    }

    let mut jobs: Vec<JobRecord> = folded.into_values().collect();
    jobs.sort_by_key(|j| std::cmp::Reverse(j.submitted_at));
    Ok(JobLog { jobs, skipped })
}

/// Fold the log into current state, newest first.
///
/// A missing log is an **empty list, not an error**. A malformed line is **skipped, not
/// fatal** — one bad append must never make every job unreadable, and the surviving records
/// are still the truth about real submitted work. Call [`load_log`] when the skip list
/// itself must be shown.
pub fn load_all(dir: &Path) -> Result<Vec<JobRecord>> {
    Ok(load_log(dir)?.jobs)
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
            cancel_requested_at: None,
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

        let log = load_log(dir.path()).expect("must not fail");
        assert_eq!(log.jobs.len(), 2, "the good records still load");
        assert_eq!(
            log.skipped.len(),
            1,
            "the junk line is reported, not hidden"
        );
        assert_eq!(log.skipped[0].line, 2);
    }

    /// Regression: a snapshot written before `cancel_requested_at` (and before
    /// `batch_size` on hyperparams) must still load. Jobs are money-adjacent —
    /// a schema bump that fails to default hides a paid run.
    #[test]
    fn snapshot_written_before_a_field_existed_still_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = log_path(dir.path());
        fs::write(
            &path,
            r#"{"id":"j1","provider":"together","dataset":{"name":"d","sha256":"abc"},"base_model":"Qwen/Qwen3.6-35B-A3B","output_name":"worker-v1","method":"lora_sft","hyperparams":{"n_epochs":3,"learning_rate":1e-5,"lora_r":16,"lora_alpha":32},"trainer_agent":"FORGE","compute":{"kind":"managed"},"submitted_at":"2026-08-03T00:00:00Z"}
"#,
        )
        .expect("write legacy snapshot");

        let rec = load(dir.path(), "j1").expect("legacy snapshot must still load");
        assert_eq!(rec.id, "j1");
        assert_eq!(rec.hyperparams.n_epochs, 3);
        assert_eq!(rec.hyperparams.batch_size, None);
        assert!(rec.cancel_requested_at.is_none());
        assert!(rec.terminal.is_none());
    }

    #[test]
    fn a_partial_hyperparams_object_fills_the_documented_defaults() {
        let got: Hyperparams =
            serde_json::from_str(r#"{"n_epochs":5}"#).expect("missing fields default");
        assert_eq!(got.n_epochs, 5);
        assert_eq!(got.lora_r, 16);
        assert_eq!(got.lora_alpha, 32);
        assert!((got.learning_rate - 1e-5).abs() < 1e-15);
        assert_eq!(got.batch_size, None);
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

    #[cfg(unix)]
    #[test]
    fn the_job_log_is_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        append(dir.path(), &record("j1")).expect("append");
        let mode = fs::metadata(log_path(dir.path()))
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

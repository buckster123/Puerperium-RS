//! The job lifecycle: submit, refresh, cancel.
//!
//! This module is where the expensive invariants live. Each one exists because the failure it
//! prevents costs real money or real recoverability:
//!
//! 1. **The record is written before the upstream call.** A crash in between leaves a job
//!    with no `provider_job_id` — visible and recoverable, rather than a paid run nobody
//!    knows about.
//! 2. **An unreachable provider is not a failure** (doctrine #9). The job may well be running
//!    and billing; it stays non-terminal and resumable.
//! 3. **A rejected submit *is* a failure**, and terminal, with the upstream's real reason.
//!    The difference between "we could not ask" and "they said no" is the whole point.
//! 4. **`Terminal` is written once** and a terminal job is never polled again.
//! 5. **Compute is checked before anything is written** — Puerperium never creates compute
//!    (D4), so a job it cannot run must not leave a record implying it tried.

use chrono::Utc;

use crate::dataset::DatasetRef;
use crate::error::{Error, Result};
use crate::job::{
    self, ComputeRef, Hyperparams, JobRecord, Method, Outcome, Phase, Provider, Terminal,
};
use crate::paths::Paths;
use crate::provider::{ProviderError, SubmitRequest, TrainingProvider};

/// Everything needed to start a job.
#[derive(Debug, Clone)]
pub struct SubmitSpec {
    pub id: String,
    pub provider: Provider,
    pub dataset: DatasetRef,
    pub base_model: String,
    pub output_name: String,
    pub method: Method,
    pub hyperparams: Hyperparams,
    /// Who ordered it (D6). Never an `agent_id`.
    pub trainer_agent: String,
    pub compute: ComputeRef,
    /// The upstream's handle for the uploaded training data.
    pub training_file_id: String,
}

/// Is the requested compute already there?
///
/// Public and free to call, so a caller can gate **before** building a provider. That
/// ordering matters: a missing API key would otherwise mask the compute refusal, and the
/// operator would be told to set a key when the real problem is that the box does not exist.
/// [`submit`] calls this too — this is the check, not a preview of it.
pub fn check_compute(compute: &ComputeRef, available: &[String]) -> Result<()> {
    if let ComputeRef::Node { name } = compute {
        if !available.iter().any(|c| c == name) {
            return Err(Error::ComputeUnavailable {
                requested: name.clone(),
                available: available.to_vec(),
            });
        }
    }
    Ok(())
}

/// Submit a job.
///
/// `available_compute` is what Router already has — discovered, never created (D4). Ignored
/// for [`ComputeRef::Managed`], which needs no box.
///
/// Job ids are unique once they have a `provider_job_id` or a terminal. A crash-row (the
/// id exists, neither is set) is retried in place — last-write-wins must not mint a second
/// paid run under the same id.
pub fn submit(
    paths: &Paths,
    provider: &dyn TrainingProvider,
    spec: SubmitSpec,
    available_compute: &[String],
) -> Result<JobRecord> {
    // The gate comes first: refuse before writing anything, so a job that cannot run leaves
    // no record implying it tried.
    check_compute(&spec.compute, available_compute)?;
    crate::upload::assert_bound(paths, &spec.training_file_id, &spec.dataset)?;

    let dir = paths.root();

    if let Ok(existing) = job::load(dir, &spec.id) {
        if let Some(pid) = &existing.provider_job_id {
            return Err(Error::JobExists {
                id: spec.id,
                reason: format!("provider id {pid} — resubmitting would orphan a paid run"),
            });
        }
        if let Some(t) = &existing.terminal {
            return Err(Error::JobExists {
                id: spec.id,
                reason: format!("already ended ({})", t.outcome.as_str()),
            });
        }
        if !same_facts(&existing, &spec) {
            return Err(Error::JobExists {
                id: spec.id,
                reason: "unconfirmed submit with different facts — retry the original spec or pick a new id".into(),
            });
        }
        // Crash-row: the record is already the invariant-1 write. Retry, don't mint another.
        return finish_submit(dir, existing, spec.training_file_id, provider);
    }

    let record = JobRecord {
        id: spec.id,
        provider: spec.provider,
        provider_job_id: None,
        dataset: spec.dataset,
        base_model: spec.base_model.clone(),
        output_name: spec.output_name.clone(),
        method: spec.method,
        hyperparams: spec.hyperparams.clone(),
        trainer_agent: spec.trainer_agent,
        compute: spec.compute,
        submitted_at: Utc::now(),
        terminal: None,
        cancel_requested_at: None,
        total_price_nanodollars: None,
        ledger_refs: vec![],
    };

    // INVARIANT 1: on disk before the upstream is touched.
    job::append(dir, &record)?;
    finish_submit(dir, record, spec.training_file_id, provider)
}

fn same_facts(existing: &JobRecord, spec: &SubmitSpec) -> bool {
    existing.provider == spec.provider
        && existing.dataset == spec.dataset
        && existing.base_model == spec.base_model
        && existing.output_name == spec.output_name
        && existing.method == spec.method
        && existing.hyperparams == spec.hyperparams
        && existing.trainer_agent == spec.trainer_agent
        && existing.compute == spec.compute
}

fn finish_submit(
    dir: &std::path::Path,
    mut record: JobRecord,
    training_file_id: String,
    provider: &dyn TrainingProvider,
) -> Result<JobRecord> {
    let req = SubmitRequest {
        training_file_id,
        base_model: record.base_model.clone(),
        output_name: record.output_name.clone(),
        method: record.method,
        hyperparams: record.hyperparams.clone(),
    };

    match provider.submit(&req) {
        Ok(provider_job_id) => {
            record.provider_job_id = Some(provider_job_id);
            job::append(dir, &record)?;
            Ok(record)
        }
        // INVARIANT 3: they said no. Definite, terminal, with their reason.
        Err(e @ (ProviderError::Rejected(_) | ProviderError::NoKey { .. })) => {
            record.terminal = Some(Terminal {
                outcome: Outcome::Failed,
                observed_at: Utc::now(),
                artifact: None,
                error: Some(e.to_string()),
            });
            job::append(dir, &record)?;
            Ok(record)
        }
        // INVARIANT 2: we could not ask, or could not understand the answer. The job may
        // exist upstream and may be billing — leaving it non-terminal keeps it recoverable.
        // The crash-row is already on disk; do not append an identical blank snapshot.
        Err(e @ (ProviderError::Unreachable(_) | ProviderError::Malformed(_))) => {
            Err(Error::SubmitUnconfirmed {
                id: record.id,
                reason: e.to_string(),
            })
        }
    }
}

/// Ask where a job has got to, recording a terminal state the first time one is observed.
///
/// Returns the record and its phase. Never fails because the provider was unreachable — that
/// is [`Phase::Unknown`], which is an answer.
pub fn refresh(
    paths: &Paths,
    provider: &dyn TrainingProvider,
    id: &str,
) -> Result<(JobRecord, Phase)> {
    let dir = paths.root();
    let record = job::load(dir, id)?;

    // INVARIANT 4: a terminal job is never polled again.
    if let Some(phase) = record.terminal_phase() {
        return Ok((record, phase));
    }

    // Submitted but never acknowledged: there is nothing upstream to ask about.
    let Some(provider_job_id) = record.provider_job_id.clone() else {
        return Ok((record, Phase::Unknown));
    };

    match provider.poll(&provider_job_id) {
        Ok(status) => {
            let phase = status.phase;
            if let Some(outcome) = phase.outcome() {
                let mut updated = record;
                updated.terminal = Some(Terminal {
                    outcome,
                    observed_at: Utc::now(),
                    artifact: status.artifact,
                    error: status.error,
                });
                if status.total_price_nanodollars.is_some() {
                    updated.total_price_nanodollars = status.total_price_nanodollars;
                }
                job::append(dir, &updated)?;
                Ok((updated, phase))
            } else {
                // Non-terminal: nothing to persist. Phase is computed, never stored (D3).
                Ok((record, phase))
            }
        }
        // INVARIANT 2 again: unreachable is not failure.
        Err(_) => Ok((record, Phase::Unknown)),
    }
}

/// Ask the upstream to stop.
///
/// Best effort: the attempt is recorded either way, and the job is **not** marked cancelled
/// here — only an observed `cancelled` from the provider is terminal. Marking it locally
/// would claim an outcome we did not witness.
pub fn cancel(paths: &Paths, provider: &dyn TrainingProvider, id: &str) -> Result<JobRecord> {
    let dir = paths.root();
    let record = job::load(dir, id)?;

    if record.is_terminal() {
        return Ok(record);
    }
    let Some(provider_job_id) = record.provider_job_id.clone() else {
        return Err(Error::SubmitUnconfirmed {
            id: record.id,
            reason: "never acknowledged upstream, so there is nothing to cancel".into(),
        });
    };

    // The ask is a fact even when the upstream refuses it. Outcome stays unclaimed.
    let mut updated = record;
    updated.cancel_requested_at = Some(Utc::now());
    job::append(dir, &updated)?;

    provider
        .cancel(&provider_job_id)
        .map_err(|e| Error::ProviderRefused(e.to_string()))?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{status, ProviderStatus, Scripted};

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = Paths::new(dir.path());
        // spec() always uses file-abc / apexos-knowledge@abc123.
        crate::upload::save(
            &p,
            &crate::upload::FileBinding {
                file_id: "file-abc".into(),
                dataset: DatasetRef {
                    name: "apexos-knowledge".into(),
                    sha256: "abc123".into(),
                },
                projected_sha256: "abc123".into(),
                uploaded_at: Utc::now(),
            },
        )
        .expect("bind test file");
        (dir, p)
    }

    fn spec(id: &str, compute: ComputeRef) -> SubmitSpec {
        SubmitSpec {
            id: id.into(),
            provider: Provider::Together,
            dataset: DatasetRef {
                name: "apexos-knowledge".into(),
                sha256: "abc123".into(),
            },
            base_model: "Qwen/Qwen3.6-27B".into(),
            output_name: "worker-v1".into(),
            method: Method::LoraSft,
            hyperparams: Hyperparams::default(),
            trainer_agent: "FORGE".into(),
            compute,
            training_file_id: "file-abc".into(),
        }
    }

    #[test]
    fn happy_path_records_the_provider_id_then_reaches_succeeded() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1").then_polls(vec![
            Ok(status(Phase::Running, "running")),
            Ok(ProviderStatus {
                phase: Phase::Succeeded,
                artifact: Some("acct/worker-v1-adapter".into()),
                error: None,
                upstream_status: "completed".into(),
                total_price_nanodollars: Some(4_000_000_000),
            }),
        ]);

        let rec = submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("submit");
        assert_eq!(rec.provider_job_id.as_deref(), Some("ft-1"));

        let (_, phase) = refresh(&p, &prov, "j1").expect("first poll");
        assert_eq!(phase, Phase::Running);
        assert!(!job::load(p.root(), "j1").expect("load").is_terminal());

        let (rec, phase) = refresh(&p, &prov, "j1").expect("second poll");
        assert_eq!(phase, Phase::Succeeded);
        let t = rec.terminal.expect("terminal written");
        assert_eq!(t.outcome, Outcome::Succeeded);
        assert_eq!(t.artifact.as_deref(), Some("acct/worker-v1-adapter"));
        assert_eq!(rec.total_price_nanodollars, Some(4_000_000_000));
    }

    /// INVARIANT 1: the record must exist even when the upstream never answers.
    #[test]
    fn an_unreachable_submit_still_leaves_a_recoverable_record() {
        let (_d, p) = paths();
        let prov = Scripted {
            submit_result: None, // -> Unreachable
            ..Default::default()
        };

        let err = submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect_err("must fail");
        assert!(
            matches!(err, Error::SubmitUnconfirmed { .. }),
            "got {err:?}"
        );

        let rec = job::load(p.root(), "j1").expect("record must exist anyway");
        assert_eq!(
            rec.provider_job_id, None,
            "we never learned the upstream id"
        );
        assert!(
            !rec.is_terminal(),
            "it may be running and billing — not a failure"
        );
    }

    /// INVARIANT 3: "they said no" is definite, and terminal, with their reason.
    #[test]
    fn a_rejected_submit_is_terminal_with_the_real_reason() {
        let (_d, p) = paths();
        let prov = Scripted::failing_to_submit("base model not supported for fine-tuning");

        let rec = submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("returns record");
        let t = rec.terminal.expect("must be terminal");
        assert_eq!(t.outcome, Outcome::Failed);
        assert!(
            t.error
                .expect("reason")
                .contains("base model not supported"),
            "the upstream's reason must survive"
        );
    }

    /// INVARIANT 2: a poll timeout is not a failure — the job stays recoverable.
    #[test]
    fn an_unreachable_poll_yields_unknown_and_writes_no_terminal() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1").then_polls(vec![Err("connection reset".into())]);

        submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("submit");
        let (rec, phase) = refresh(&p, &prov, "j1").expect("must not error");

        assert_eq!(phase, Phase::Unknown);
        assert!(
            !rec.is_terminal(),
            "a paid run outliving our patience is still running"
        );
        assert!(job::load(p.root(), "j1").expect("load").terminal.is_none());
    }

    /// An upstream state we do not recognise must never be treated as finished.
    #[test]
    fn an_unrecognised_upstream_state_never_becomes_terminal() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1")
            .then_polls(vec![Ok(status(Phase::Unknown, "some_new_state"))]);

        submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("submit");
        let (rec, phase) = refresh(&p, &prov, "j1").expect("refresh");
        assert_eq!(phase, Phase::Unknown);
        assert!(!rec.is_terminal());
    }

    /// INVARIANT 4: terminal is written once and never re-polled.
    #[test]
    fn a_terminal_job_is_never_polled_again() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1")
            .then_polls(vec![Ok(status(Phase::Succeeded, "completed"))]);

        submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("submit");
        refresh(&p, &prov, "j1").expect("reaches terminal");
        let after_first = prov.poll_count();

        let (rec, phase) = refresh(&p, &prov, "j1").expect("second refresh");
        assert_eq!(phase, Phase::Succeeded);
        assert_eq!(
            prov.poll_count(),
            after_first,
            "must not re-poll a finished job"
        );
        assert!(rec.terminal.is_some());
    }

    /// INVARIANT 5: refuse before writing anything, and say what *is* available.
    #[test]
    fn missing_compute_refuses_without_leaving_a_record() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1");
        let node = ComputeRef::Node {
            name: "gpu-box-1".into(),
        };

        let err = submit(&p, &prov, spec("j1", node), &["other-box".into()]).expect_err("refuse");
        match err {
            Error::ComputeUnavailable {
                requested,
                available,
            } => {
                assert_eq!(requested, "gpu-box-1");
                assert_eq!(available, vec!["other-box".to_string()]);
            }
            other => panic!("wrong error: {other:?}"),
        }
        assert!(
            job::load_all(p.root()).expect("load").is_empty(),
            "a job that cannot run must leave no record implying it tried"
        );
    }

    /// The gate must be answerable without a provider, so a caller can check it before
    /// building one. A missing API key masking "that box does not exist" tells the operator
    /// to fix the wrong thing.
    #[test]
    fn compute_can_be_checked_without_a_provider() {
        assert!(check_compute(&ComputeRef::Managed, &[]).is_ok());
        let node = ComputeRef::Node {
            name: "gpu-box-1".into(),
        };
        assert!(check_compute(&node, &["gpu-box-1".into()]).is_ok());
        assert!(matches!(
            check_compute(&node, &["other".into()]),
            Err(Error::ComputeUnavailable { .. })
        ));
    }

    #[test]
    fn managed_compute_submits_with_nothing_provisioned() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1");
        submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("Together needs no box");
    }

    #[test]
    fn node_compute_submits_when_router_already_has_it() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1");
        let node = ComputeRef::Node {
            name: "gpu-box-1".into(),
        };
        submit(&p, &prov, spec("j1", node), &["gpu-box-1".into()]).expect("exists already");
    }

    /// Cancel claims nothing it did not witness.
    #[test]
    fn cancel_does_not_mark_the_job_cancelled_locally() {
        let (_d, p) = paths();
        let prov = Scripted::submitting("ft-1");
        submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]).expect("submit");

        let rec = cancel(&p, &prov, "j1").expect("cancel");
        assert!(
            !rec.is_terminal(),
            "only an observed `cancelled` from the provider is terminal"
        );
        assert!(
            rec.cancel_requested_at.is_some(),
            "the ask is a fact and must persist"
        );
        assert_eq!(
            job::load(p.root(), "j1").expect("load").cancel_requested_at,
            rec.cancel_requested_at
        );
    }

    #[test]
    fn a_second_submit_of_a_live_id_does_not_call_the_provider() {
        let (_d, p) = paths();
        let first = Scripted::submitting("ft-1");
        submit(&p, &first, spec("j1", ComputeRef::Managed), &[]).expect("first");

        let second = Scripted::submitting("ft-2");
        let err =
            submit(&p, &second, spec("j1", ComputeRef::Managed), &[]).expect_err("must refuse");
        assert!(matches!(err, Error::JobExists { .. }), "got {err:?}");
        assert!(
            err.to_string().contains("ft-1"),
            "the live id must be in the reason: {err}"
        );
        assert_eq!(
            second.submit_count(),
            0,
            "must not contact the provider a second time"
        );
        assert_eq!(
            job::load(p.root(), "j1")
                .expect("load")
                .provider_job_id
                .as_deref(),
            Some("ft-1"),
            "the live id must not be overwritten"
        );
    }

    #[test]
    fn a_rejected_id_is_not_reused() {
        let (_d, p) = paths();
        let reject = Scripted::failing_to_submit("base model not supported");
        submit(&p, &reject, spec("j1", ComputeRef::Managed), &[]).expect("rejected record");

        let again = Scripted::submitting("ft-2");
        let err =
            submit(&p, &again, spec("j1", ComputeRef::Managed), &[]).expect_err("must refuse");
        assert!(matches!(err, Error::JobExists { .. }), "got {err:?}");
        assert_eq!(again.submit_count(), 0);
        assert!(job::load(p.root(), "j1").expect("load").is_terminal());
    }

    #[test]
    fn a_crash_row_retries_without_minting_a_blank_snapshot() {
        let (_d, p) = paths();
        let fail = Scripted {
            submit_result: None,
            ..Default::default()
        };
        let _ = submit(&p, &fail, spec("j1", ComputeRef::Managed), &[]);
        let crash = job::load(p.root(), "j1").expect("crash-row");
        assert!(crash.provider_job_id.is_none());
        assert!(!crash.is_terminal());

        let ok = Scripted::submitting("ft-recovered");
        let rec = submit(&p, &ok, spec("j1", ComputeRef::Managed), &[]).expect("retry");
        assert_eq!(rec.provider_job_id.as_deref(), Some("ft-recovered"));
        assert_eq!(ok.submit_count(), 1);

        let text = std::fs::read_to_string(job::log_path(p.root())).expect("read log");
        assert_eq!(
            text.lines().count(),
            2,
            "crash-row + recovered id; no extra blank snapshot"
        );
    }

    #[test]
    fn an_unbound_training_file_refuses_without_writing_a_record() {
        let (_d, p) = paths();
        let mut s = spec("j1", ComputeRef::Managed);
        s.training_file_id = "file-ghost".into();
        let prov = Scripted::submitting("ft-1");
        let err = submit(&p, &prov, s, &[]).expect_err("must refuse");
        assert!(
            matches!(err, Error::UnboundTrainingFile { .. }),
            "got {err:?}"
        );
        assert!(
            job::load_all(p.root()).expect("load").is_empty(),
            "must not write a job for a file we cannot attribute"
        );
        assert_eq!(prov.submit_count(), 0);
    }

    #[test]
    fn a_crash_row_with_different_facts_is_refused() {
        let (_d, p) = paths();
        let fail = Scripted {
            submit_result: None,
            ..Default::default()
        };
        let _ = submit(&p, &fail, spec("j1", ComputeRef::Managed), &[]);

        let mut other = spec("j1", ComputeRef::Managed);
        other.output_name = "someone-else".into();
        let again = Scripted::submitting("ft-2");
        let err = submit(&p, &again, other, &[]).expect_err("must refuse");
        assert!(err.to_string().contains("different facts"), "got {err}");
        assert_eq!(again.submit_count(), 0);
    }

    #[test]
    fn a_refused_cancel_still_records_the_ask() {
        let (_d, p) = paths();
        let submitter = Scripted::submitting("ft-1");
        submit(&p, &submitter, spec("j1", ComputeRef::Managed), &[]).expect("submit");

        let refuser = Scripted {
            cancel_result: Some(Err("upstream said no".into())),
            ..Default::default()
        };
        let err = cancel(&p, &refuser, "j1").expect_err("must surface the refuse");
        assert!(err.to_string().contains("upstream said no"), "got {err}");
        let rec = job::load(p.root(), "j1").expect("load");
        assert!(
            rec.cancel_requested_at.is_some(),
            "the ask is a fact even when they refuse"
        );
        assert!(!rec.is_terminal());
    }

    #[test]
    fn cancelling_an_unacknowledged_job_says_why_it_cannot() {
        let (_d, p) = paths();
        let prov = Scripted {
            submit_result: None,
            ..Default::default()
        };
        let _ = submit(&p, &prov, spec("j1", ComputeRef::Managed), &[]);

        let err = cancel(&p, &prov, "j1").expect_err("nothing to cancel");
        assert!(err.to_string().contains("nothing to cancel"), "got {err}");
    }
}

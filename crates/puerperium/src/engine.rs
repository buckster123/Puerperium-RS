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
pub fn submit(
    paths: &Paths,
    provider: &dyn TrainingProvider,
    spec: SubmitSpec,
    available_compute: &[String],
) -> Result<JobRecord> {
    // The gate comes first: refuse before writing anything, so a job that cannot run leaves
    // no record implying it tried.
    check_compute(&spec.compute, available_compute)?;

    let dir = paths.root();
    let mut record = JobRecord {
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
        ledger_refs: vec![],
    };

    // INVARIANT 1: on disk before the upstream is touched.
    job::append(dir, &record)?;

    let req = SubmitRequest {
        training_file_id: spec.training_file_id,
        base_model: spec.base_model,
        output_name: spec.output_name,
        method: spec.method,
        hyperparams: spec.hyperparams,
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
        Err(e @ (ProviderError::Unreachable(_) | ProviderError::Malformed(_))) => {
            job::append(dir, &record)?;
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

    provider
        .cancel(&provider_job_id)
        .map_err(|e| Error::ProviderRefused(e.to_string()))?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{status, ProviderStatus, Scripted};

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = Paths::new(dir.path());
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

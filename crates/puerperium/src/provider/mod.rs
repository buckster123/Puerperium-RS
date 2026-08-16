//! Training providers: the seam between the job engine and an upstream.
//!
//! The trait is the mock point. Every test in this crate drives a [`Scripted`] provider —
//! nothing opens a socket (charter D5), because the suite next door once made live
//! authenticated calls with a real key and that lesson is inherited rather than repeated.

pub mod together;
pub mod together_http;

use crate::job::{Hyperparams, Method, Phase};

/// What a provider needs in order to submit.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmitRequest {
    /// The upstream's handle for the uploaded training data.
    pub training_file_id: String,
    pub base_model: String,
    pub output_name: String,
    pub method: Method,
    pub hyperparams: Hyperparams,
}

/// What a poll told us.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderStatus {
    pub phase: Phase,
    /// Where the trained adapter can be found, once there is one.
    pub artifact: Option<String>,
    /// The real reason, when the phase is a failure.
    pub error: Option<String>,
    /// The upstream's own word for its state, kept verbatim. When `phase` is
    /// [`Phase::Unknown`] this is the only thing that can explain why.
    pub upstream_status: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The upstream could not be reached, or did not answer in time.
    ///
    /// **This is not a job failure** (doctrine #9). A paid run that outlives our patience is
    /// still running.
    #[error("provider unreachable: {0}")]
    Unreachable(String),

    /// The upstream answered, but not in a shape we can trust.
    #[error("provider response malformed: {0}")]
    Malformed(String),

    /// The upstream refused, and said why.
    #[error("provider rejected the request: {0}")]
    Rejected(String),

    /// No credential configured. Stated plainly — "no key configured" beats a timeout.
    #[error("no API key configured for {provider} (set {env_var})")]
    NoKey {
        provider: &'static str,
        env_var: &'static str,
    },
}

/// An upstream that can train.
pub trait TrainingProvider {
    /// Submit, returning the upstream's job id.
    fn submit(&self, req: &SubmitRequest) -> Result<String, ProviderError>;

    /// Ask the upstream where a job has got to.
    fn poll(&self, provider_job_id: &str) -> Result<ProviderStatus, ProviderError>;

    /// Ask the upstream to stop. Best effort — the record keeps the attempt either way.
    fn cancel(&self, provider_job_id: &str) -> Result<(), ProviderError>;

    /// For messages and records.
    fn name(&self) -> &'static str;
}

/// A provider driven by a script, for tests and dry runs.
///
/// Lives outside `#[cfg(test)]` deliberately: the job engine's guarantees — record before
/// submit, timeout is not failure, terminal written once — are only demonstrable against a
/// provider whose behaviour can be dictated, and those demonstrations are the point.
#[derive(Debug, Default)]
pub struct Scripted {
    pub submit_result: Option<Result<String, String>>,
    pub poll_results: std::cell::RefCell<Vec<Result<ProviderStatus, String>>>,
    pub cancel_result: Option<Result<(), String>>,
    pub polls: std::cell::Cell<usize>,
    pub submits: std::cell::Cell<usize>,
}

impl Scripted {
    pub fn submitting(id: &str) -> Self {
        Self {
            submit_result: Some(Ok(id.to_string())),
            ..Default::default()
        }
    }

    pub fn failing_to_submit(reason: &str) -> Self {
        Self {
            submit_result: Some(Err(reason.to_string())),
            ..Default::default()
        }
    }

    /// Queue poll answers, consumed in order; the last repeats.
    pub fn then_polls(self, results: Vec<Result<ProviderStatus, String>>) -> Self {
        *self.poll_results.borrow_mut() = results;
        self
    }

    pub fn poll_count(&self) -> usize {
        self.polls.get()
    }

    pub fn submit_count(&self) -> usize {
        self.submits.get()
    }
}

impl TrainingProvider for Scripted {
    fn submit(&self, _req: &SubmitRequest) -> Result<String, ProviderError> {
        self.submits.set(self.submits.get() + 1);
        match &self.submit_result {
            Some(Ok(id)) => Ok(id.clone()),
            Some(Err(e)) => Err(ProviderError::Rejected(e.clone())),
            None => Err(ProviderError::Unreachable("no scripted submit".into())),
        }
    }

    fn poll(&self, _provider_job_id: &str) -> Result<ProviderStatus, ProviderError> {
        self.polls.set(self.polls.get() + 1);
        let mut queued = self.poll_results.borrow_mut();
        let next = if queued.len() > 1 {
            queued.remove(0)
        } else {
            queued
                .first()
                .cloned()
                .unwrap_or(Err("no scripted poll".into()))
        };
        next.map_err(ProviderError::Unreachable)
    }

    fn cancel(&self, _provider_job_id: &str) -> Result<(), ProviderError> {
        match &self.cancel_result {
            Some(Ok(())) => Ok(()),
            Some(Err(e)) => Err(ProviderError::Rejected(e.clone())),
            None => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        "scripted"
    }
}

/// Convenience for building scripted poll answers in tests.
pub fn status(phase: Phase, upstream: &str) -> ProviderStatus {
    ProviderStatus {
        phase,
        artifact: None,
        error: None,
        upstream_status: upstream.to_string(),
    }
}

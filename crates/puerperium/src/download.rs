//! Pull a finished Together adapter home and read its loss curve.
//!
//! A Together fine-tune leaves the artifact as a hosted name. Serving is on vast/local
//! (charter amendment 2026-08-03), so the weights have to come here. The download itself
//! is **free**. The default checkpoint is `adapter` — omitting it on the wire would
//! fetch the merged full model, which is the wrong default for a LoRA we will load
//! onto a base we already have.
//!
//! The archive is `.tar.zst`. `trainer_state.json` inside it is the free first gate:
//! compare **epoch means**, never first-step vs last-step.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::job::{self, JobRecord, Phase};
use crate::paths::Paths;
use crate::provider::together::Checkpoint;
use crate::provider::together_http::TogetherClient;
use crate::store;

const LOSS_CAVEAT: &str = "compare epoch means, never first-step vs last-step — \
     per-step stdev can dwarf the epoch-to-epoch trend";

/// Mean training loss for one epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpochMean {
    pub epoch: u32,
    pub mean_loss: f64,
    pub n: usize,
}

/// What `trainer_state.json` actually says, labelled so a reader cannot pick the
/// misleading first-vs-last number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LossCurve {
    pub steps: usize,
    pub epoch_means: Vec<EpochMean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_step_loss: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_step_loss: Option<f64>,
    pub caveat: String,
}

/// What a download produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub job_id: String,
    pub provider_job_id: String,
    pub checkpoint: Checkpoint,
    pub archive: PathBuf,
    pub extracted_dir: PathBuf,
    pub bytes: u64,
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss: Option<LossCurve>,
}

/// Who to download, and which checkpoint.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Local job id. When set, the record must already be `Succeeded`.
    pub job_id: Option<String>,
    /// Together `ft-…` id. Required when `job_id` is absent (recovery).
    pub provider_job_id: Option<String>,
    pub checkpoint: Checkpoint,
}

/// Can this record be downloaded?
///
/// Pure. A running or failed job has nothing we should fetch; an unacknowledged
/// one has no upstream id.
pub fn require_downloadable(record: &JobRecord) -> Result<String> {
    match record.terminal_phase() {
        Some(Phase::Succeeded) => {}
        Some(p) => {
            return Err(Error::NotDownloadable {
                id: record.id.clone(),
                reason: format!("phase is {}", p.as_str()),
            });
        }
        None => {
            return Err(Error::NotDownloadable {
                id: record.id.clone(),
                reason: "job is not terminal — poll it first".into(),
            });
        }
    }
    record
        .provider_job_id
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::NotDownloadable {
            id: record.id.clone(),
            reason: "no provider_job_id — the upstream never confirmed it".into(),
        })
}

/// Download from Together, write the archive, extract it, read the loss curve.
pub fn fetch(paths: &Paths, client: &TogetherClient, spec: Spec) -> Result<Report> {
    let (label, ft_id, dest_name) = resolve(paths, &spec)?;
    let dest = artifact_dir(paths, &dest_name)?;
    store::ensure_dir(&dest)?;

    let archive_name = format!("{}.tar.zst", spec.checkpoint.as_str());
    let archive = dest.join(&archive_name);
    let filename = client
        .download_checkpoint_to(&ft_id, spec.checkpoint, &archive)
        .map_err(|e| Error::ProviderRefused(e.to_string()))?;
    let renamed = dest.join(&filename);
    if renamed != archive && !renamed.exists() {
        fs::rename(&archive, &renamed).map_err(|e| Error::io(&renamed, e))?;
    }
    let archive = if renamed.exists() { renamed } else { archive };
    store::lock_private(&archive)?;

    let bytes = fs::metadata(&archive)
        .map_err(|e| Error::io(&archive, e))?
        .len();
    let extracted = dest.join("extracted");
    let files = extract_tar_zst(&archive, &extracted)?;
    let loss = files
        .iter()
        .find(|f| Path::new(f).file_name().and_then(|n| n.to_str()) == Some("trainer_state.json"))
        .and_then(|rel| read_loss(&extracted.join(rel)));

    Ok(Report {
        job_id: label,
        provider_job_id: ft_id,
        checkpoint: spec.checkpoint,
        archive,
        extracted_dir: extracted,
        bytes,
        files,
        loss,
    })
}

/// Write bytes that are already a `.tar.zst` and extract them. The test surface.
pub fn materialise(
    dest: &Path,
    checkpoint: Checkpoint,
    bytes: &[u8],
    job_id: &str,
    provider_job_id: &str,
) -> Result<Report> {
    store::ensure_dir(dest)?;
    let archive = dest.join(format!("{}.tar.zst", checkpoint.as_str()));
    store::write_atomic(&archive, bytes)?;
    let extracted = dest.join("extracted");
    let files = extract_tar_zst(&archive, &extracted)?;
    let loss = files
        .iter()
        .find(|f| Path::new(f).file_name().and_then(|n| n.to_str()) == Some("trainer_state.json"))
        .and_then(|rel| read_loss(&extracted.join(rel)));
    Ok(Report {
        job_id: job_id.into(),
        provider_job_id: provider_job_id.into(),
        checkpoint,
        archive,
        extracted_dir: extracted,
        bytes: bytes.len() as u64,
        files,
        loss,
    })
}

/// Unpack a `.tar.zst`. Entries that would escape `dest` are refused, not skipped.
pub fn extract_tar_zst(archive: &Path, dest: &Path) -> Result<Vec<String>> {
    let compressed = fs::read(archive).map_err(|e| Error::io(archive, e))?;
    let tar_bytes = zstd::decode_all(Cursor::new(compressed)).map_err(|e| Error::Io {
        path: archive.to_path_buf(),
        source: std::io::Error::other(format!("zstd decode failed: {e}")),
    })?;
    store::ensure_dir(dest)?;

    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut files = Vec::new();
    for entry in archive.entries().map_err(|e| Error::io(dest, e))? {
        let mut entry = entry.map_err(|e| Error::io(dest, e))?;
        let rel = entry.path().map_err(|e| Error::io(dest, e))?.into_owned();
        let safe = safe_entry(&rel)?;
        if entry.header().entry_type().is_dir() {
            store::ensure_dir(&dest.join(&safe))?;
            continue;
        }
        if let Some(parent) = dest.join(&safe).parent() {
            store::ensure_dir(parent)?;
        }
        let out = dest.join(&safe);
        let mut f = fs::File::create(&out).map_err(|e| Error::io(&out, e))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| Error::io(&out, e))?;
        f.flush().map_err(|e| Error::io(&out, e))?;
        store::lock_private(&out)?;
        files.push(safe.to_string_lossy().replace('\\', "/"));
    }
    Ok(files)
}

/// Parse HuggingFace `trainer_state.json`. Pure.
pub fn parse_trainer_state(json: &str) -> Result<LossCurve> {
    let state: TrainerState = serde_json::from_str(json)?;
    let mut losses: Vec<(u32, f64)> = Vec::new();
    for row in &state.log_history {
        let Some(loss) = row.loss else { continue };
        let epoch = row.epoch.unwrap_or(0.0).floor() as u32;
        losses.push((epoch, loss));
    }
    let first_step_loss = losses.first().map(|(_, l)| *l);
    let last_step_loss = losses.last().map(|(_, l)| *l);

    let mut by_epoch: std::collections::BTreeMap<u32, (f64, usize)> =
        std::collections::BTreeMap::new();
    for (epoch, loss) in &losses {
        let e = by_epoch.entry(*epoch).or_insert((0.0, 0));
        e.0 += *loss;
        e.1 += 1;
    }
    let epoch_means = by_epoch
        .into_iter()
        .map(|(epoch, (sum, n))| EpochMean {
            epoch,
            mean_loss: sum / n as f64,
            n,
        })
        .collect();

    Ok(LossCurve {
        steps: losses.len(),
        epoch_means,
        first_step_loss,
        last_step_loss,
        caveat: LOSS_CAVEAT.into(),
    })
}

fn read_loss(path: &Path) -> Option<LossCurve> {
    let text = fs::read_to_string(path).ok()?;
    parse_trainer_state(&text).ok()
}

fn resolve(paths: &Paths, spec: &Spec) -> Result<(String, String, String)> {
    match (&spec.job_id, &spec.provider_job_id) {
        (Some(id), _) => {
            let record = job::load(paths.root(), id)?;
            let ft = require_downloadable(&record)?;
            let dest = dest_name(&record);
            Ok((record.id, ft, dest))
        }
        (None, Some(ft)) if !ft.trim().is_empty() => {
            let dest = dest_name_from_ft(ft);
            Ok((ft.clone(), ft.clone(), dest))
        }
        _ => Err(Error::NotDownloadable {
            id: String::new(),
            reason: "give a local job id or a Together ft-… id".into(),
        }),
    }
}

fn dest_name(record: &JobRecord) -> String {
    if store::validate_name(&record.output_name).is_ok() {
        record.output_name.clone()
    } else {
        record.id.clone()
    }
}

fn dest_name_from_ft(ft: &str) -> String {
    if store::validate_name(ft).is_ok() {
        ft.to_string()
    } else {
        ft.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }
}

fn artifact_dir(paths: &Paths, name: &str) -> Result<PathBuf> {
    store::validate_name(name)?;
    Ok(paths.model_artifacts(name))
}

fn safe_entry(rel: &Path) -> Result<PathBuf> {
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::UnsafeArchiveEntry(rel.display().to_string()));
    }
    Ok(rel.to_path_buf())
}

#[derive(Debug, Deserialize)]
struct TrainerState {
    #[serde(default)]
    log_history: Vec<LogRow>,
}

#[derive(Debug, Deserialize)]
struct LogRow {
    #[serde(default)]
    loss: Option<f64>,
    #[serde(default)]
    epoch: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::DatasetRef;
    use crate::job::{ComputeRef, Hyperparams, JobRecord, Method, Outcome, Provider, Terminal};
    use chrono::Utc;

    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_buf);
            for (name, data) in files {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_cksum();
                b.append_data(&mut h, name, *data).expect("append");
            }
            b.finish().expect("finish");
        }
        zstd::encode_all(&tar_buf[..], 1).expect("zstd")
    }

    fn succeeded(id: &str, ft: &str) -> JobRecord {
        JobRecord {
            id: id.into(),
            provider: Provider::Together,
            provider_job_id: Some(ft.into()),
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
            terminal: Some(Terminal {
                outcome: Outcome::Succeeded,
                observed_at: Utc::now(),
                artifact: Some("acct/worker-v1-adapter".into()),
                error: None,
            }),
            cancel_requested_at: None,
            total_price_nanodollars: Some(4_000_000_000),
            ledger_refs: vec![],
        }
    }

    #[test]
    fn a_succeeded_job_with_a_provider_id_is_downloadable() {
        let rec = succeeded("j1", "ft-abc");
        assert_eq!(require_downloadable(&rec).expect("ok"), "ft-abc");
    }

    #[test]
    fn a_running_job_is_not_downloadable() {
        let mut rec = succeeded("j1", "ft-abc");
        rec.terminal = None;
        let err = require_downloadable(&rec).expect_err("no");
        assert!(matches!(err, Error::NotDownloadable { .. }), "{err}");
        assert!(err.to_string().contains("not terminal"));
    }

    #[test]
    fn a_failed_job_is_not_downloadable() {
        let mut rec = succeeded("j1", "ft-abc");
        rec.terminal = Some(Terminal {
            outcome: Outcome::Failed,
            observed_at: Utc::now(),
            artifact: None,
            error: Some("boom".into()),
        });
        let err = require_downloadable(&rec).expect_err("no");
        assert!(err.to_string().contains("failed"));
    }

    #[test]
    fn epoch_means_are_the_signal_not_first_vs_last() {
        let json = r#"{
            "log_history": [
                {"loss": 4.0, "epoch": 0.1, "step": 1},
                {"loss": 2.0, "epoch": 0.9, "step": 10},
                {"loss": 3.1, "epoch": 1.0, "step": 11},
                {"loss": 3.0, "epoch": 1.5, "step": 20},
                {"eval_loss": 9.9, "epoch": 2.0, "step": 21}
            ]
        }"#;
        let curve = parse_trainer_state(json).expect("parse");
        assert_eq!(curve.steps, 4, "eval_loss is not a training step");
        assert_eq!(curve.first_step_loss, Some(4.0));
        assert_eq!(curve.last_step_loss, Some(3.0));
        assert_eq!(curve.epoch_means.len(), 2);
        assert!(
            (curve.epoch_means[0].mean_loss - 3.0).abs() < 1e-9,
            "epoch 0 mean"
        );
        assert!(
            (curve.epoch_means[1].mean_loss - 3.05).abs() < 1e-9,
            "epoch 1 mean"
        );
        assert!(curve.caveat.contains("epoch means"));
    }

    #[test]
    fn materialise_extracts_and_reads_the_curve() {
        let dir = tempfile::tempdir().expect("tmp");
        let state = br#"{"log_history":[{"loss":3.096,"epoch":0.5,"step":1},{"loss":3.046,"epoch":2.0,"step":87}]}"#;
        let bytes = pack(&[
            ("adapter_config.json", b"{}"),
            ("trainer_state.json", state),
        ]);
        let report = materialise(dir.path(), Checkpoint::Adapter, &bytes, "j1", "ft-abc")
            .expect("materialise");
        assert!(report
            .files
            .iter()
            .any(|f| f.ends_with("trainer_state.json")));
        let loss = report.loss.expect("curve");
        assert_eq!(loss.steps, 2);
        assert_eq!(loss.epoch_means.len(), 2);
    }

    #[test]
    fn parent_and_absolute_paths_are_unsafe() {
        assert!(matches!(
            safe_entry(Path::new("../escape")),
            Err(Error::UnsafeArchiveEntry(_))
        ));
        assert!(matches!(
            safe_entry(Path::new("/tmp/evil")),
            Err(Error::UnsafeArchiveEntry(_))
        ));
        assert!(matches!(
            safe_entry(Path::new("ok/../../etc/passwd")),
            Err(Error::UnsafeArchiveEntry(_))
        ));
        assert_eq!(
            safe_entry(Path::new("adapter/config.json")).expect("ok"),
            PathBuf::from("adapter/config.json")
        );
    }
}

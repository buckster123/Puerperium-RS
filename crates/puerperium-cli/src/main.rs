//! `puerperium` — the human/ops face.
//!
//! Thin over the core library: parse arguments, call one function, render the result.
//! No logic lives here (house rule); if something here needs a test, it belongs in the lib.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use puerperium::convert::filter::FilterConfig;
use puerperium::convert::instruct::InstructConfig;
use puerperium::convert::{convert, ConvertConfig};
use puerperium::dataset::{self, SourceSpec};
use puerperium::engine::{self, SubmitSpec};
use puerperium::estimate;
use puerperium::job::{self, ComputeRef, Hyperparams, Method, Phase, Provider};
use puerperium::memory::{MemoryRecord, MemoryType};
use puerperium::paths::Paths;
use puerperium::registry::{self, ModelRecord};

#[derive(Parser)]
#[command(name = "puerperium", version, about = "The model nursery")]
struct Cli {
    /// State root. Defaults to $PUERPERIUM_STATE_DIR, else ~/.local/share/puerperium.
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Dataset garden — build and inspect training data.
    #[command(subcommand)]
    Data(DataCmd),
    /// Model registry — registered adapters and their provenance.
    #[command(subcommand)]
    Model(ModelCmd),
    /// Apprentices — specialists raised from an agent's own experience.
    #[command(subcommand)]
    Apprentice(ApprenticeCmd),
    /// Training jobs — submit, poll, cancel.
    #[command(subcommand)]
    Job(JobCmd),
    /// Estimate what a fine-tune would cost. Free; touches no upstream.
    Estimate(EstimateArgs),
    /// Trace a model back through its ancestors to the memories it came from.
    Lineage {
        model: String,
        /// Emit the full structure as JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DataCmd {
    /// Build a dataset from exported memories.
    Generate(GenerateArgs),
    /// List datasets, newest first.
    List,
    /// Show a dataset's metadata and its first examples.
    Inspect {
        name: String,
        #[arg(long, default_value_t = 3)]
        head: usize,
    },
    /// Re-hash a dataset and compare against its sidecar.
    Verify { name: String },
}

#[derive(Subcommand)]
enum ModelCmd {
    /// Record a trained adapter in the registry.
    ///
    /// This is the *registry* entry only. Registering it as a live Router alias is a
    /// separate, explicit act (charter D2) and lands with S5.
    Add(ModelAddArgs),
    /// List models, newest first.
    List,
    /// Show one model record as JSON.
    Show { name: String },
}

#[derive(Subcommand)]
enum ApprenticeCmd {
    /// List apprentices, newest first.
    List,
    /// Show one apprentice record as JSON.
    Show { id: String },
}

#[derive(Subcommand)]
enum JobCmd {
    /// Submit a training job.
    Submit(SubmitArgs),
    /// List jobs, newest first. Non-terminal jobs are polled.
    List,
    /// Show one job and its computed phase.
    Status { id: String },
    /// Ask the upstream to stop. Best effort; nothing is marked cancelled locally.
    Cancel { id: String },
}

#[derive(Args)]
struct SubmitArgs {
    /// Job id. Yours to choose; it is how you find the job again.
    #[arg(long)]
    id: String,
    /// Dataset name. Its hash is read from the sidecar and pinned into the record.
    #[arg(long)]
    dataset: String,
    #[arg(long, default_value = "Qwen/Qwen3.6-27B")]
    base_model: String,
    /// Name for the resulting adapter.
    #[arg(long)]
    output_name: String,
    /// Who ordered it. Never an `agent_id` (charter D6).
    #[arg(long, default_value = "FORGE")]
    trainer_agent: String,
    /// The upstream's handle for the already-uploaded training data.
    #[arg(long)]
    training_file_id: String,
    #[arg(long, default_value_t = 3)]
    epochs: u32,
    #[arg(long, default_value_t = 16)]
    lora_r: u32,
    #[arg(long, default_value_t = 32)]
    lora_alpha: u32,
    /// Router-known compute to run on. Omit for Together, which is a hosted API.
    #[arg(long)]
    compute: Option<String>,
    /// Compute Router already has. Puerperium never creates any (charter D4).
    #[arg(long, value_delimiter = ',')]
    available_compute: Vec<String>,
    /// Show the exact request body and write nothing. Touches no upstream.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct EstimateArgs {
    /// Dataset to price.
    #[arg(long)]
    dataset: String,
    /// Base model size in billions of parameters (27 for Qwen3.6-27B).
    #[arg(long, default_value_t = 27.0)]
    params_b: f64,
    #[arg(long, default_value_t = 3)]
    epochs: u32,
}

#[derive(Args)]
struct GenerateArgs {
    /// JSON array of memory records (a Cerebro export).
    #[arg(long)]
    from: PathBuf,
    /// Dataset name. Must be unique — datasets are immutable.
    #[arg(long)]
    name: String,
    /// Memory types to include. Defaults to procedural, semantic, schematic.
    #[arg(long, value_delimiter = ',')]
    include_types: Vec<String>,
    /// Optional domain for the tag fallback, e.g. "ApexOS".
    #[arg(long)]
    domain: Option<String>,
    /// Minimum content length in characters.
    #[arg(long)]
    min_content: Option<usize>,
    /// Convert and report, but write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct ModelAddArgs {
    /// Registry key, and the candidate Router alias.
    #[arg(long)]
    name: String,
    /// The base this was fine-tuned from.
    #[arg(long)]
    base_model: String,
    /// Who ordered the training. Never an `agent_id` (charter D6).
    #[arg(long, default_value = "FORGE")]
    trainer_agent: String,
    /// Dataset this was trained on. Its hash is read from the sidecar.
    #[arg(long)]
    dataset: Option<String>,
    /// The model this one was trained from, for multi-generation lineage.
    #[arg(long)]
    parent: Option<String>,
    /// Path to the adapter artifact.
    #[arg(long)]
    artifact: Option<PathBuf>,
    /// Job that produced it.
    #[arg(long)]
    job_id: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = match cli.state_dir {
        Some(p) => Paths::new(p),
        None => Paths::from_env()
            .context("no state dir: set $PUERPERIUM_STATE_DIR or $HOME, or pass --state-dir")?,
    };

    match cli.command {
        Command::Data(DataCmd::Generate(args)) => generate(&paths, args),
        Command::Data(DataCmd::List) => data_list(&paths),
        Command::Data(DataCmd::Inspect { name, head }) => data_inspect(&paths, &name, head),
        Command::Data(DataCmd::Verify { name }) => data_verify(&paths, &name),
        Command::Model(ModelCmd::Add(args)) => model_add(&paths, args),
        Command::Model(ModelCmd::List) => model_list(&paths),
        Command::Model(ModelCmd::Show { name }) => {
            print_json(&registry::load_model(&paths, &name)?)
        }
        Command::Apprentice(ApprenticeCmd::List) => apprentice_list(&paths),
        Command::Apprentice(ApprenticeCmd::Show { id }) => {
            print_json(&registry::load_apprentice(&paths, &id)?)
        }
        Command::Job(JobCmd::Submit(args)) => job_submit(&paths, args),
        Command::Job(JobCmd::List) => job_list(&paths),
        Command::Job(JobCmd::Status { id }) => job_status(&paths, &id),
        Command::Job(JobCmd::Cancel { id }) => job_cancel(&paths, &id),
        Command::Estimate(args) => estimate_cost(&paths, args),
        Command::Lineage { model, json } => lineage(&paths, &model, json),
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_type(s: &str) -> Result<MemoryType> {
    serde_json::from_value(serde_json::Value::String(s.trim().to_lowercase()))
        .with_context(|| format!("unknown memory type {s:?}"))
}

fn generate(paths: &Paths, args: GenerateArgs) -> Result<()> {
    let bytes =
        std::fs::read(&args.from).with_context(|| format!("reading {}", args.from.display()))?;
    let memories: Vec<MemoryRecord> = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} as a memory export", args.from.display()))?;

    let mut cfg = ConvertConfig::new();
    if !args.include_types.is_empty() {
        cfg.filter.include_types = args
            .include_types
            .iter()
            .map(|s| parse_type(s))
            .collect::<Result<_>>()?;
    }
    if let Some(n) = args.min_content {
        cfg.filter = FilterConfig {
            min_content: n,
            ..cfg.filter
        };
    }
    cfg.instruct = InstructConfig {
        domain: args.domain.clone(),
        ..InstructConfig::new()
    };

    let out = convert(&memories, &cfg);

    println!("memories in     {}", memories.len());
    println!("memories used   {}", out.memories_used);
    println!("examples        {}", out.examples.len());
    println!("rejected        {}", out.rejections.total());
    for (reason, n) in out.rejections.counts() {
        println!("  {reason:<16} {n}");
    }
    for (kind, n) in &out.framing {
        println!("  framing {:<8} {n}", kind.as_str());
    }

    if args.dry_run {
        println!("\ndry run — nothing written");
        return Ok(());
    }

    let source = SourceSpec {
        kind: "export_file".into(),
        query: Some(args.from.display().to_string()),
        agent_id: None,
        memories_in: memories.len(),
    };
    let meta = dataset::write(&paths.datasets(), &args.name, &out, source)?;

    println!(
        "\nwrote {}",
        dataset::jsonl_path(&paths.datasets(), &meta.name).display()
    );
    println!("sha256 {}", meta.sha256);
    Ok(())
}

fn data_list(paths: &Paths) -> Result<()> {
    let all = dataset::list(&paths.datasets())?;
    if all.is_empty() {
        println!("no datasets in {}", paths.datasets().display());
        return Ok(());
    }
    for m in all {
        println!(
            "{:<28} {:>5} ex  {:>4} used  {:>4} rej  {}  {}",
            m.name,
            m.example_count,
            m.memories_used,
            m.rejected_total,
            &m.sha256[..12],
            m.created_at.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

fn data_inspect(paths: &Paths, name: &str, head: usize) -> Result<()> {
    let meta = dataset::read_meta(&paths.datasets(), name)?;
    print_json(&meta)?;

    let path = dataset::jsonl_path(&paths.datasets(), name);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    println!("\n--- first {head} examples ---");
    for line in text.lines().take(head) {
        let v: serde_json::Value = serde_json::from_str(line)?;
        println!("{}", serde_json::to_string_pretty(&v)?);
    }
    Ok(())
}

fn data_verify(paths: &Paths, name: &str) -> Result<()> {
    if dataset::verify(&paths.datasets(), name)? {
        println!("{name}: hash matches");
        Ok(())
    } else {
        anyhow::bail!("{name}: HASH MISMATCH — the dataset has been modified since it was written")
    }
}

fn model_add(paths: &Paths, args: ModelAddArgs) -> Result<()> {
    // Resolve the dataset through its sidecar so the record carries the real hash. A
    // caller-supplied hash could be wrong; the sidecar cannot.
    let dataset_ref = match &args.dataset {
        Some(name) => Some(
            dataset::read_meta(&paths.datasets(), name)
                .with_context(|| format!("dataset {name:?} must exist to be referenced"))?
                .dataset_ref(),
        ),
        None => None,
    };

    if let Some(parent) = &args.parent {
        anyhow::ensure!(
            registry::model_exists(paths, parent),
            "parent {parent:?} is not in the registry — lineage would be broken from birth"
        );
    }

    let record = ModelRecord {
        dataset: dataset_ref,
        job_id: args.job_id,
        artifact: args.artifact,
        parent: args.parent,
        ..ModelRecord::new(&args.name, &args.base_model, &args.trainer_agent)
    };
    registry::save_model(paths, &record)?;
    print_json(&record)
}

fn model_list(paths: &Paths) -> Result<()> {
    let all = registry::list_models(paths)?;
    if all.is_empty() {
        println!("no models in {}", paths.models().display());
        return Ok(());
    }
    for m in all {
        println!(
            "{:<28} {:<26} {:<8} {:<20} {}",
            m.name,
            m.base_model,
            m.trainer_agent,
            m.dataset.as_ref().map(|d| d.name.as_str()).unwrap_or("—"),
            m.created_at.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

fn apprentice_list(paths: &Paths) -> Result<()> {
    let all = registry::list_apprentices(paths)?;
    if all.is_empty() {
        println!("no apprentices in {}", paths.apprentices().display());
        return Ok(());
    }
    for a in all {
        println!(
            "{:<20} {:<12} {:<28} {:<10} {}",
            a.id,
            a.master_agent,
            a.specialization,
            if a.is_trained() {
                "trained"
            } else {
                "untrained"
            },
            a.created_at.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

fn lineage(paths: &Paths, model: &str, json: bool) -> Result<()> {
    let lin = registry::lineage(paths, model)?;
    if json {
        return print_json(&lin);
    }

    for e in &lin.entries {
        let data = match (&e.dataset, e.dataset_missing, e.dataset_hash_mismatch) {
            (None, _, _) => "no dataset recorded".to_string(),
            (Some(d), true, _) => format!("{} — MISSING from disk", d.name),
            (Some(d), _, true) => format!("{} — HASH MISMATCH, not the data it trained on", d.name),
            (Some(d), _, _) => format!(
                "{} ({}) — {} examples from {} memories",
                d.name,
                &d.sha256[..12],
                e.dataset_examples.unwrap_or(0),
                e.dataset_memories.unwrap_or(0)
            ),
        };
        println!("gen {}  {}", e.generation, e.model);
        println!("        base    {}", e.base_model);
        println!("        trainer {}", e.trainer_agent);
        println!("        data    {data}");
    }

    if let Some(reason) = &lin.incomplete {
        println!("\nincomplete: {reason}");
    }
    Ok(())
}

// ------------------------------------------------------------------ jobs

/// The live provider, or an honest refusal naming what is missing.
fn together() -> Result<puerperium::provider::together_http::TogetherClient> {
    Ok(puerperium::provider::together_http::TogetherClient::from_env()?)
}

fn job_submit(paths: &Paths, args: SubmitArgs) -> Result<()> {
    let dataset = dataset::read_meta(&paths.datasets(), &args.dataset)
        .with_context(|| format!("dataset {:?} must exist to be trained on", args.dataset))?
        .dataset_ref();

    let hyperparams = Hyperparams {
        n_epochs: args.epochs,
        lora_r: args.lora_r,
        lora_alpha: args.lora_alpha,
        ..Hyperparams::default()
    };
    let compute = match &args.compute {
        Some(name) => ComputeRef::Node { name: name.clone() },
        None => ComputeRef::Managed,
    };

    if args.dry_run {
        let req = puerperium::provider::SubmitRequest {
            training_file_id: args.training_file_id,
            base_model: args.base_model,
            output_name: args.output_name,
            method: Method::LoraSft,
            hyperparams,
        };
        println!("would POST /v1/fine-tunes:");
        println!(
            "{}",
            serde_json::to_string_pretty(&puerperium::provider::together::build_submit_body(&req))?
        );
        println!("\ndry run — nothing written, no upstream contacted");
        return Ok(());
    }

    // Gate on compute BEFORE building a provider: a missing key must not mask the fact that
    // the requested box does not exist (charter D4).
    engine::check_compute(&compute, &args.available_compute)?;

    let spec = SubmitSpec {
        id: args.id,
        provider: Provider::Together,
        dataset,
        base_model: args.base_model,
        output_name: args.output_name,
        method: Method::LoraSft,
        hyperparams,
        trainer_agent: args.trainer_agent,
        compute,
        training_file_id: args.training_file_id,
    };

    let record = engine::submit(paths, &together()?, spec, &args.available_compute)?;
    print_json(&record)
}

fn job_list(paths: &Paths) -> Result<()> {
    let all = job::load_all(paths.root())?;
    if all.is_empty() {
        println!("no jobs in {}", job::log_path(paths.root()).display());
        return Ok(());
    }

    // Poll non-terminal jobs, but only build a client if one is actually needed — listing
    // must not demand a key just to show finished work.
    let live = all
        .iter()
        .any(|j| !j.is_terminal() && j.provider_job_id.is_some())
        .then(together)
        .transpose();

    for j in &all {
        let phase = match (j.terminal_phase(), &live) {
            (Some(p), _) => p,
            (None, Ok(Some(client))) => engine::refresh(paths, client, &j.id)
                .map(|(_, p)| p)
                .unwrap_or(Phase::Unknown),
            // No key, or the client could not be built: unknown is the honest answer.
            (None, _) => Phase::Unknown,
        };
        println!(
            "{:<16} {:<10} {:<12} {:<22} {}",
            j.id,
            j.provider.as_str(),
            phase.as_str(),
            j.output_name,
            j.dataset.name
        );
    }
    if let Err(e) = &live {
        println!("\n(non-terminal jobs shown as unknown: {e})");
    }
    Ok(())
}

fn job_status(paths: &Paths, id: &str) -> Result<()> {
    let record = job::load(paths.root(), id)?;
    let phase = match record.terminal_phase() {
        Some(p) => p,
        None => match together() {
            Ok(client) => engine::refresh(paths, &client, id)?.1,
            Err(e) => {
                println!("(cannot poll: {e})");
                Phase::Unknown
            }
        },
    };
    println!("phase: {}", phase.as_str());
    print_json(&job::load(paths.root(), id)?)
}

fn job_cancel(paths: &Paths, id: &str) -> Result<()> {
    let record = engine::cancel(paths, &together()?, id)?;
    println!("cancel requested for {id}; nothing marked terminal until the upstream says so");
    print_json(&record)
}

fn estimate_cost(paths: &Paths, args: EstimateArgs) -> Result<()> {
    let path = dataset::jsonl_path(&paths.datasets(), &args.dataset);
    let chars = std::fs::metadata(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();

    let est = estimate::together_lora(chars, args.epochs, args.params_b);
    println!("dataset tokens  ~{}", est.dataset_tokens);
    println!(
        "training tokens ~{} ({} epochs)",
        est.training_tokens, est.epochs
    );
    match (est.price_per_mtok_usd, est.training_usd) {
        (Some(p), Some(usd)) => println!("training cost   ~${usd:.2} (at ${p:.2}/Mtok)"),
        _ => println!("training cost   not priced"),
    }
    for c in &est.caveats {
        println!("  - {c}");
    }
    Ok(())
}

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
    /// Show which credentials are configured. Never prints a value.
    Keys,
    /// What compute ApexRouter already has. Read-only; Puerperium never creates any.
    Compute,
    /// Hand a trained adapter to ApexRouter as a routable alias, and record the lineage.
    Deploy(DeployArgs),
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
    /// Project a dataset into what a provider accepts, and validate it. Offline.
    Export {
        name: String,
        /// Where to write. Omit to check only and report.
        #[arg(long)]
        to: Option<PathBuf>,
    },
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
    /// Raise a specialist from an agent's own remembered experience.
    ///
    /// Mines, builds a dataset, registers the record. Does NOT train — that costs money
    /// and stays a separate explicit act (charter D4).
    ///
    /// Boxed: this variant is far larger than its siblings, and clap only ever holds one.
    Create(Box<ApprenticeCreateArgs>),
    /// Record the training job started for an apprentice.
    AttachJob { id: String, job_id: String },
    /// Record the model an apprentice grew into. This is what makes it trained.
    AttachModel { id: String, model: String },
    /// Show which agents a Cerebro snapshot holds memories for.
    Agents { db: PathBuf },
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
    /// Upload a dataset and print the training file id `submit` needs. Costs nothing.
    Upload { dataset: String },
    /// Together's OWN price estimate for an uploaded file. Free, and authoritative — it knows
    /// the real tokenizer and the minimum charge, which a local heuristic cannot.
    Quote {
        training_file_id: String,
        #[arg(long, default_value = "Qwen/Qwen3.6-35B-A3B")]
        base_model: String,
        #[arg(long, default_value_t = 3)]
        epochs: u32,
        #[arg(long, default_value_t = 16)]
        lora_r: u32,
        #[arg(long, default_value_t = 32)]
        lora_alpha: u32,
        /// Parameter count in billions, for the metered-vs-floor note only.
        #[arg(long, default_value_t = 35.0)]
        params_b: f64,
    },
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
struct DeployArgs {
    /// Registered model to hand over.
    #[arg(long)]
    model: String,
    /// Alias it becomes reachable as through Router.
    #[arg(long)]
    alias: String,
    /// Base URL of whatever serves it. Stored WITHOUT /v1.
    #[arg(long, default_value = "https://api.together.xyz")]
    base_url: String,
    /// The name the backend actually serves. Omit to pass the alias through unchanged.
    #[arg(long)]
    served_model: Option<String>,
    /// Env var holding the backend's credential. Router stores the NAME, never the value.
    #[arg(long, default_value = "TOGETHER_API_KEY")]
    credential_env: String,
    /// Skip the Cerebro lineage event.
    #[arg(long)]
    no_lineage: bool,
    /// Print exactly what would be sent, and contact nothing.
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
struct ApprenticeCreateArgs {
    /// Registry key for the apprentice.
    #[arg(long)]
    id: String,
    /// Cerebro snapshot to mine. Opened READ-ONLY; prefer a `.backup` snapshot over a live
    /// file. Repeatable — a colony's knowledge lives on several nodes.
    #[arg(long)]
    db: Vec<PathBuf>,
    /// Whose memory space to mine, paired positionally with `--db`. A single value applies to
    /// every snapshot.
    #[arg(long)]
    master_agent: Vec<String>,
    /// What this apprentice is for, in your words. Recorded verbatim.
    #[arg(long)]
    specialization: String,
    /// Human name for the apprentice.
    #[arg(long)]
    name: String,
    #[arg(long, default_value = "Qwen/Qwen3.6-27B")]
    base_model: String,
    /// Dataset name to create. Datasets are immutable; re-running under one is refused.
    #[arg(long)]
    dataset_name: String,
    /// Keep only memories carrying at least one of these tags.
    #[arg(long, value_delimiter = ',')]
    tags: Vec<String>,
    /// Cap memories mined, highest salience first.
    #[arg(long)]
    limit: Option<usize>,
    /// Memory types to include. Defaults to procedural, semantic, schematic.
    #[arg(long, value_delimiter = ',')]
    include_types: Vec<String>,
    /// Optional domain for the tag fallback, e.g. "ApexOS".
    #[arg(long)]
    domain: Option<String>,
    /// Admit dream-engine memories. Off by default — they are the agent's own abstractions,
    /// not lived experience, and training on them reinforces the abstraction.
    #[arg(long)]
    include_dream: bool,
    /// Mine and report what would be built, writing nothing.
    #[arg(long)]
    dry_run: bool,
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
    // Credentials before anything else, so every verb sees them. A real environment variable
    // always wins, so a one-off `TOGETHER_API_KEY=… puerperium …` still overrides the file.
    let loaded = puerperium::secrets::load();
    for w in &loaded.warnings {
        eprintln!("warning: {w}");
    }

    let cli = Cli::parse();
    let paths = match cli.state_dir {
        Some(p) => Paths::new(p),
        None => Paths::from_env()
            .context("no state dir: set $PUERPERIUM_STATE_DIR or $HOME, or pass --state-dir")?,
    };
    paths.ensure()?;

    match cli.command {
        Command::Data(DataCmd::Generate(args)) => generate(&paths, args),
        Command::Data(DataCmd::List) => data_list(&paths),
        Command::Data(DataCmd::Inspect { name, head }) => data_inspect(&paths, &name, head),
        Command::Data(DataCmd::Verify { name }) => data_verify(&paths, &name),
        Command::Data(DataCmd::Export { name, to }) => data_export(&paths, &name, to.as_deref()),
        Command::Model(ModelCmd::Add(args)) => model_add(&paths, args),
        Command::Model(ModelCmd::List) => model_list(&paths),
        Command::Model(ModelCmd::Show { name }) => {
            print_json(&registry::load_model(&paths, &name)?)
        }
        Command::Apprentice(ApprenticeCmd::Create(args)) => apprentice_create(&paths, *args),
        Command::Apprentice(ApprenticeCmd::AttachJob { id, job_id }) => {
            print_json(&puerperium::apprentice::attach_job(&paths, &id, &job_id)?)
        }
        Command::Apprentice(ApprenticeCmd::AttachModel { id, model }) => {
            print_json(&puerperium::apprentice::attach_model(&paths, &id, &model)?)
        }
        Command::Apprentice(ApprenticeCmd::Agents { db }) => agents(&db),
        Command::Apprentice(ApprenticeCmd::List) => apprentice_list(&paths),
        Command::Apprentice(ApprenticeCmd::Show { id }) => {
            print_json(&registry::load_apprentice(&paths, &id)?)
        }
        Command::Job(JobCmd::Submit(args)) => job_submit(&paths, args),
        Command::Job(JobCmd::List) => job_list(&paths),
        Command::Job(JobCmd::Status { id }) => job_status(&paths, &id),
        Command::Job(JobCmd::Cancel { id }) => job_cancel(&paths, &id),
        Command::Job(JobCmd::Upload { dataset }) => job_upload(&paths, &dataset),
        Command::Job(JobCmd::Quote {
            training_file_id,
            base_model,
            epochs,
            lora_r,
            lora_alpha,
            params_b,
        }) => job_quote(
            &training_file_id,
            &base_model,
            epochs,
            lora_r,
            lora_alpha,
            params_b,
        ),
        Command::Estimate(args) => estimate_cost(&paths, args),
        Command::Keys => keys(&loaded),
        Command::Compute => compute(),
        Command::Deploy(args) => deploy(&paths, args),
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
        dataset::jsonl_path(&paths.datasets(), &meta.name)?.display()
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

    let path = dataset::jsonl_path(&paths.datasets(), name)?;
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
    registry::add_model(paths, &record)?;
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
    print_json(&record)?;
    if let Some(t) = &record.terminal {
        anyhow::bail!(
            "submit rejected: {}",
            t.error.as_deref().unwrap_or(t.outcome.as_str())
        );
    }
    Ok(())
}

fn job_list(paths: &Paths) -> Result<()> {
    let log = job::load_log(paths.root())?;
    if log.jobs.is_empty() && log.skipped.is_empty() {
        println!("no jobs in {}", job::log_path(paths.root()).display());
        return Ok(());
    }

    // Poll non-terminal jobs, but only build a client if one is actually needed — listing
    // must not demand a key just to show finished work.
    let live = log
        .jobs
        .iter()
        .any(|j| !j.is_terminal() && j.provider_job_id.is_some())
        .then(together)
        .transpose();

    for j in &log.jobs {
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
    if !log.skipped.is_empty() {
        eprintln!(
            "\n{} unreadable job snapshot(s) skipped — a schema bump must not hide a paid run:",
            log.skipped.len()
        );
        for s in &log.skipped {
            eprintln!("  line {}: {}", s.line, s.reason);
        }
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
    let path = dataset::jsonl_path(&paths.datasets(), &args.dataset)?;
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

/// Report credential state. Lengths and heads only — never a value (doctrine #6).
fn keys(loaded: &puerperium::secrets::Loaded) -> Result<()> {
    match &loaded.file {
        Some(f) => println!("env file: {}", f.display()),
        None => {
            println!("env file: none found");
            for c in puerperium::secrets::candidates() {
                println!("  looked in {}", c.display());
            }
        }
    }
    if !loaded.set.is_empty() {
        println!("loaded from file: {}", loaded.set.join(", "));
    }
    if !loaded.skipped.is_empty() {
        println!(
            "already in environment (file ignored): {}",
            loaded.skipped.join(", ")
        );
    }

    println!();
    for (name, var) in [("together", "TOGETHER_API_KEY")] {
        match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => {
                let head: String = v.chars().take(4).collect();
                println!("{name:<10} configured  ({} chars, starts {head}…)", v.len());
            }
            _ => println!("{name:<10} not configured  (set {var})"),
        }
    }
    Ok(())
}

/// Read a stored dataset and project it to the provider's schema.
fn provider_bytes(paths: &Paths, name: &str) -> Result<String> {
    let path = dataset::jsonl_path(&paths.datasets(), name)?;
    let stored =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(puerperium::export::to_provider_jsonl(
        &stored,
        puerperium::export::ProviderFormat::Conversation,
    )?)
}

fn data_export(paths: &Paths, name: &str, to: Option<&std::path::Path>) -> Result<()> {
    let out = provider_bytes(paths, name)?;
    println!(
        "{} lines, {} bytes, provider-schema clean",
        out.lines().count(),
        out.len()
    );
    match to {
        Some(p) => {
            std::fs::write(p, &out).with_context(|| format!("writing {}", p.display()))?;
            println!("wrote {}", p.display());
        }
        None => println!("(validated only — pass --to <path> to write)"),
    }
    Ok(())
}

fn job_upload(paths: &Paths, dataset: &str) -> Result<()> {
    // Project and validate BEFORE contacting anything: a schema error should cost nothing.
    let body = provider_bytes(paths, dataset)?;
    println!(
        "{} lines, {} bytes — validated locally",
        body.lines().count(),
        body.len()
    );

    let client = together()?;
    let file_id = client.upload_jsonl(&format!("{dataset}.jsonl"), body.as_bytes())?;
    println!("training_file_id: {file_id}");
    println!("\nnext: puerperium job submit --id <id> --dataset {dataset} \\");
    println!("        --output-name <name> --training-file-id {file_id}");
    Ok(())
}

// ----------------------------------------------------------- apprentices

fn agents(db: &std::path::Path) -> Result<()> {
    let all = puerperium::source::cerebro_db::agents(db)?;
    if all.is_empty() {
        println!("no live memories in {}", db.display());
        return Ok(());
    }
    for (agent, n) in all {
        println!("{agent:<16} {n:>6} memories");
    }
    Ok(())
}

fn apprentice_create(paths: &Paths, args: ApprenticeCreateArgs) -> Result<()> {
    anyhow::ensure!(!args.db.is_empty(), "at least one --db is required");
    anyhow::ensure!(
        !args.master_agent.is_empty(),
        "at least one --master-agent is required"
    );
    anyhow::ensure!(
        args.master_agent.len() == 1 || args.master_agent.len() == args.db.len(),
        "give one --master-agent for all snapshots, or one per --db ({} given for {} snapshots)",
        args.master_agent.len(),
        args.db.len()
    );

    // Memory ids are only unique within a store; two nodes can reuse one. Prefixing by
    // source keeps provenance honest and stops a collision silently dropping a memory.
    let mut memories = Vec::new();
    for (i, db) in args.db.iter().enumerate() {
        let agent = args
            .master_agent
            .get(i)
            .unwrap_or(&args.master_agent[0])
            .clone();
        let query = puerperium::source::cerebro_db::Query {
            agent_id: Some(agent.clone()),
            any_tags: args.tags.clone(),
            limit: args.limit,
        };
        let mut got = puerperium::source::cerebro_db::read(db, &query)?;
        println!(
            "mined {:>5} memories  {agent:<14} {}",
            got.len(),
            db.display()
        );
        let stem = db.file_stem().and_then(|s| s.to_str()).unwrap_or("db");
        for m in &mut got {
            m.id = format!("{stem}:{}", m.id);
        }
        memories.append(&mut got);
    }
    println!("mined {} memories total", memories.len());

    let mut cfg = ConvertConfig::new();
    if !args.include_types.is_empty() {
        cfg.filter.include_types = args
            .include_types
            .iter()
            .map(|s| parse_type(s))
            .collect::<Result<_>>()?;
    }
    cfg.filter.include_dream_derived = args.include_dream;
    cfg.instruct = InstructConfig {
        domain: args.domain.clone(),
        ..InstructConfig::new()
    };

    if args.dry_run {
        let out = puerperium::convert::convert(&memories, &cfg);
        println!(
            "would produce {} examples from {} memories",
            out.examples.len(),
            out.memories_used
        );
        println!("rejected {}", out.rejections.total());
        for (reason, n) in out.rejections.counts() {
            println!("  {reason:<16} {n}");
        }
        for (kind, n) in &out.framing {
            println!("  framing {:<18} {n}", kind.as_str());
        }
        println!("\ndry run — nothing written");
        return Ok(());
    }

    let spec = puerperium::apprentice::Spec {
        id: args.id,
        master_agent: args.master_agent.join("+"),
        name: args.name,
        specialization: args.specialization,
        base_model: args.base_model,
        dataset_name: args.dataset_name,
    };
    let created = puerperium::apprentice::create(paths, spec, &memories, &cfg)?;

    println!(
        "examples {} from {} memories ({} rejected)",
        created.converted.examples.len(),
        created.converted.memories_used,
        created.converted.rejections.total()
    );
    println!();
    print_json(&created.apprentice)?;
    println!("\nuntrained by design — training costs money and is a separate act:");
    println!(
        "  puerperium job upload {}",
        created
            .apprentice
            .dataset
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("<dataset>")
    );
    Ok(())
}

// --------------------------------------------------------------- deploy

fn compute() -> Result<()> {
    let client = puerperium::router::RouterClient::from_env()?;
    let backends = client.backends()?;
    if backends.is_empty() {
        println!("ApexRouter has no backends. Puerperium never creates compute (charter D4) —");
        println!("use `apexrouter vast rent` or start a local endpoint first.");
        return Ok(());
    }
    for b in &backends {
        let shown: Vec<&str> = b.models.iter().take(3).map(|m| m.id.as_str()).collect();
        let more = b.models.len().saturating_sub(shown.len());
        // A catalogue backend declares hundreds of models; printing them all buries the
        // one line that matters.
        let tail = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        println!(
            "{:<20} {:<26} {:<10} {}{}",
            b.id,
            b.label,
            // CONFIGURATION, not liveness. A vast recipe can be configured-and-enabled while
            // the box is cold and unreachable.
            if b.enabled { "configured" } else { "disabled" },
            shown.join(", "),
            tail
        );
    }
    println!(
        "\n`configured` is Router's config state, NOT proof the box is up — a vast recipe reads\n\
         configured while cold. Probe before treating one as available compute."
    );

    println!("\nLoRA-capable bases advertised (fine-tune these, not the -Lora serving name):");
    let mut any = false;
    for b in &backends {
        for base in puerperium::router::lora_capable_bases(b) {
            println!("  {base}   [{}]", b.id);
            any = true;
        }
    }
    if !any {
        println!("  none advertised");
    }
    Ok(())
}

fn deploy(paths: &Paths, args: DeployArgs) -> Result<()> {
    // The model must be registered here first — handing Router an alias for something with no
    // provenance is exactly the untraceable outcome this project exists to prevent.
    let record = registry::load_model(paths, &args.model)?;

    let spec = puerperium::router::node_spec(
        &args.base_url,
        &format!("puerperium: {}", record.name),
        &args
            .served_model
            .clone()
            .into_iter()
            .collect::<Vec<String>>(),
        Some(&args.credential_env),
    );
    let route_preview =
        puerperium::router::model_route(&args.alias, "<backend-id>", args.served_model.as_deref());

    if args.dry_run {
        println!(
            "POST /v1/backends:\n{}",
            serde_json::to_string_pretty(&spec)?
        );
        println!(
            "\nPUT /v1/routes/{}:\n{}",
            args.alias,
            serde_json::to_string_pretty(&route_preview)?
        );
        println!("\ndry run — nothing sent, nothing recorded");
        return Ok(());
    }

    let client = puerperium::router::RouterClient::from_env()?;

    // Reuse a backend already pointing at this URL. Two rows for one URL disagree the moment
    // either is edited.
    let backend_id = match client.backend_for_base_url(&args.base_url)? {
        Some(existing) => {
            println!(
                "backend {} (reusing existing, not duplicating)",
                existing.id
            );
            existing.id
        }
        None => {
            let id = client.register_backend(&spec)?;
            println!("backend {id} (registered)");
            id
        }
    };

    let route =
        puerperium::router::model_route(&args.alias, &backend_id, args.served_model.as_deref());
    client.upsert_route(&args.alias, &route)?;
    println!("alias  {} -> {}", args.alias, backend_id);

    // Record what we asked for. NOT whether it is live — that stays Router's truth (D3).
    let mut updated = record.clone();
    updated.alias_requested = Some(args.alias.clone());
    registry::save_model(paths, &updated)?;

    if !args.no_lineage {
        let cerebro = puerperium::source::cerebro_mcp::CerebroMcp::from_env();
        let event = puerperium::source::cerebro_mcp::lineage_event_args(
            "model_registered",
            &record.name,
            record.dataset.as_ref().map(|d| d.name.as_str()),
            record.dataset.as_ref().map(|d| d.sha256.as_str()),
            &record.trainer_agent,
            &record.trainer_agent,
            &format!("registered with ApexRouter as alias {}", args.alias),
        );
        match cerebro.call_tool("remember", event) {
            Ok(_) => println!("lineage recorded in cerebro"),
            // A failed lineage write must not undo a successful deploy — say so and move on.
            Err(e) => println!(
                "lineage NOT recorded ({e}) — the deploy stands; re-run with the event later"
            ),
        }
    }

    println!(
        "\nverify:  curl -s 127.0.0.1:8888/v1/models | grep {}",
        args.alias
    );
    Ok(())
}

/// The authoritative quote. Free, and the only number that includes the minimum charge.
fn job_quote(
    training_file_id: &str,
    base_model: &str,
    epochs: u32,
    lora_r: u32,
    lora_alpha: u32,
    params_b: f64,
) -> Result<()> {
    let client = together()?;
    let limits = client.limits(base_model)?;
    let est = client.estimate_price(
        training_file_id,
        base_model,
        epochs,
        lora_r,
        lora_alpha,
        &limits.target_modules,
    )?;
    println!("train tokens  {}", est.train_tokens);
    println!("TOTAL         ${:.2}", est.total_usd);
    if !est.allowed_to_proceed {
        println!("REFUSED       this would exceed the account limit");
    }
    if let Some(band) = puerperium::provider::together::lora_price_per_mtok(params_b) {
        let metered = (est.train_tokens as f64 / 1_000_000.0) * band;
        if est.total_usd > metered * 2.0 {
            println!(
                "note          a MINIMUM CHARGE dominates: metered tokens are only ~${metered:.2}"
            );
        }
    }
    Ok(())
}

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
use puerperium::memory::{MemoryRecord, MemoryType};

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
        /// How many examples to show.
        #[arg(long, default_value_t = 3)]
        head: usize,
    },
    /// Re-hash a dataset and compare against its sidecar.
    Verify { name: String },
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let state = state_dir(cli.state_dir)?;
    let datasets = state.join("datasets");

    match cli.command {
        Command::Data(DataCmd::Generate(args)) => generate(&datasets, args),
        Command::Data(DataCmd::List) => list(&datasets),
        Command::Data(DataCmd::Inspect { name, head }) => inspect(&datasets, &name, head),
        Command::Data(DataCmd::Verify { name }) => verify(&datasets, &name),
    }
}

fn state_dir(flag: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = flag {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("PUERPERIUM_STATE_DIR") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").context("HOME is unset and --state-dir was not given")?;
    Ok(PathBuf::from(home).join(".local/share/puerperium"))
}

fn parse_type(s: &str) -> Result<MemoryType> {
    serde_json::from_value(serde_json::Value::String(s.trim().to_lowercase()))
        .with_context(|| format!("unknown memory type {s:?}"))
}

fn generate(datasets: &std::path::Path, args: GenerateArgs) -> Result<()> {
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
    let meta = dataset::write(datasets, &args.name, &out, source)?;

    println!(
        "\nwrote {}",
        dataset::jsonl_path(datasets, &meta.name).display()
    );
    println!("sha256 {}", meta.sha256);
    Ok(())
}

fn list(datasets: &std::path::Path) -> Result<()> {
    let all = dataset::list(datasets)?;
    if all.is_empty() {
        println!("no datasets in {}", datasets.display());
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

fn inspect(datasets: &std::path::Path, name: &str, head: usize) -> Result<()> {
    let meta = dataset::read_meta(datasets, name)?;
    println!("{}", serde_json::to_string_pretty(&meta)?);

    let path = dataset::jsonl_path(datasets, name);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    println!("\n--- first {head} examples ---");
    for line in text.lines().take(head) {
        let v: serde_json::Value = serde_json::from_str(line)?;
        println!("{}", serde_json::to_string_pretty(&v)?);
    }
    Ok(())
}

fn verify(datasets: &std::path::Path, name: &str) -> Result<()> {
    if dataset::verify(datasets, name)? {
        println!("{name}: hash matches");
        Ok(())
    } else {
        anyhow::bail!("{name}: HASH MISMATCH — the dataset has been modified since it was written")
    }
}

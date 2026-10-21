//! One run of the desktop's GPU bench (v0.6 step 88): the fixed questions
//! answered on this executable's backend, against an index built once by an
//! index-only run and reused by every other (nothing embedded again), with
//! one progress line per answer for the desktop.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::error::Result;
use crate::eval::answers::{AnswerProbe, ProbeResult, run_probe};
use crate::eval::retrieval::open_index;
use crate::gpu::Offload;
use crate::infer::{Embedder, Generator};
use crate::retrieval::answer::{ANSWER_SAMPLER, AnswerEngine};
use crate::retrieval::hybrid::{RetrievalConfig, Retriever};
use crate::store::search::SqliteVecIndex;

pub const SCHEMA: &str = "orag-bench-run/1";
/// A bench child whose stdin closed (the desktop cancelled or quit).
pub const EXIT_STDIN_CLOSED: i32 = 3;

/// What one bench process did; the desktop merges these per tier.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BenchRun {
    pub schema: &'static str,
    /// `ok`, or `not-installed` (then only `tier` and `missing` say more).
    pub status: &'static str,
    pub tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub ran: Option<Ran>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ran {
    /// `cpu`, `vulkan` or `metal`.
    pub build: &'static str,
    pub generator: String,
    pub embedder: String,
    pub sampler: String,
    pub index_built: bool,
    pub index_documents: usize,
    /// Both models' load time, apart from the answers' times.
    pub load_ms: f64,
    pub generator_offload: Option<Offload>,
    pub embedder_offload: Option<Offload>,
    pub results: Vec<ProbeResult>,
}

impl BenchRun {
    pub fn not_installed(tier: &str, missing: &str) -> BenchRun {
        BenchRun {
            schema: SCHEMA,
            status: "not-installed",
            tier: tier.to_string(),
            missing: Some(missing.to_string()),
            ran: None,
        }
    }
}

/// Which questions a run answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Questions {
    /// None: the run only builds the index.
    IndexOnly,
    /// The first, for another cold start.
    First,
    All,
}

pub struct BenchModels {
    pub embedder: Arc<dyn Embedder>,
    pub generator: Arc<dyn Generator>,
    pub load_ms: f64,
}

/// The corpus, the questions and the index of one run.
pub struct BenchInput<'a> {
    pub corpus: &'a Path,
    pub probes: &'a [AnswerProbe],
    pub index_dir: &'a Path,
    pub build_index: bool,
    pub questions: Questions,
}

/// Opens (`build_index`: builds) the index in `index_dir`, then answers the
/// chosen questions in order, calling `on_answer` after each.
pub fn run_bench(
    tier: &str,
    models: BenchModels,
    input: BenchInput<'_>,
    on_answer: &mut dyn FnMut(&ProbeResult) -> Result<()>,
) -> Result<BenchRun> {
    let BenchModels {
        embedder,
        generator,
        load_ms,
    } = models;
    let BenchInput {
        corpus,
        probes,
        index_dir,
        build_index,
        questions,
    } = input;
    std::fs::create_dir_all(index_dir)?;
    let (store, index_documents) = open_index(index_dir, corpus, embedder.clone(), !build_index)?;
    let engine = AnswerEngine {
        retriever: Retriever {
            index: Arc::new(SqliteVecIndex::new(store.clone())),
            store,
            embedder: embedder.clone(),
            config: RetrievalConfig::default(),
            reranker: None,
        },
        generator: generator.clone(),
        sampler: ANSWER_SAMPLER,
    };
    let chosen = match questions {
        Questions::IndexOnly => &probes[..0],
        Questions::First => &probes[..probes.len().min(1)],
        Questions::All => probes,
    };
    let mut results = Vec::with_capacity(chosen.len());
    for probe in chosen {
        let result = run_probe(&engine, probe, false)?;
        on_answer(&result)?;
        results.push(result);
    }
    Ok(BenchRun {
        schema: SCHEMA,
        status: "ok",
        tier: tier.to_string(),
        missing: None,
        ran: Some(Ran {
            build: crate::gpu::Build::current().name(),
            generator: generator.model_id().to_string(),
            embedder: embedder.descriptor().model_id.clone(),
            sampler: ANSWER_SAMPLER.name(),
            index_built: build_index,
            index_documents,
            load_ms,
            generator_offload: generator.offload(),
            embedder_offload: embedder.offload(),
            results,
        }),
    })
}

/// `orag eval bench`'s arguments.
#[derive(Debug, clap::Args)]
pub struct BenchArgs {
    #[arg(long)]
    pub corpus: std::path::PathBuf,
    #[arg(long)]
    pub dataset: std::path::PathBuf,
    /// Where the index is built (`--build-index`, an empty directory) or reused.
    #[arg(long)]
    pub index_dir: std::path::PathBuf,
    /// The generation tier: `default` or `large`.
    #[arg(long)]
    pub tier: String,
    #[arg(long)]
    pub build_index: bool,
    /// Only build or check the index; answer nothing.
    #[arg(long)]
    pub index_only: bool,
    /// Answer only the first question (another cold start).
    #[arg(long, conflicts_with = "index_only")]
    pub first_only: bool,
    /// The run's JSON; must not exist yet.
    #[arg(long)]
    pub out: std::path::PathBuf,
    /// Exit when stdin closes: the desktop app's way to stop its children.
    #[arg(long)]
    pub exit_on_stdin_close: bool,
    #[arg(long)]
    pub dev_fake_models: bool,
}

/// Cheap checks first (`--out`, tier, dataset), then the models, then the
/// run. A tier whose models are not installed is a `not-installed` run.
pub fn run_cli(args: &BenchArgs) -> anyhow::Result<()> {
    use anyhow::Context;
    use std::io::Write;
    if args.exit_on_stdin_close {
        exit_when_stdin_closes();
    }
    if args.out.exists() {
        anyhow::bail!(
            "{} already exists; choose a new --out file",
            args.out.display()
        );
    }
    let tier_id = tier_model(&args.tier)?;
    let probes = crate::eval::answers::load_probes(&args.dataset)
        .with_context(|| format!("dataset {}", args.dataset.display()))?;
    let run = match load_models(tier_id, args.dev_fake_models)? {
        Err(missing) => BenchRun::not_installed(&args.tier, &missing),
        Ok(models) => {
            let questions = if args.index_only {
                Questions::IndexOnly
            } else if args.first_only {
                Questions::First
            } else {
                Questions::All
            };
            let stdout = std::io::stdout();
            run_bench(
                &args.tier,
                models,
                BenchInput {
                    corpus: &args.corpus,
                    probes: &probes,
                    index_dir: &args.index_dir,
                    build_index: args.build_index,
                    questions,
                },
                &mut |result| {
                    let line =
                        serde_json::json!({"event": "answer", "id": result.id, "ms": result.ms});
                    let mut out = stdout.lock();
                    writeln!(out, "{line}")?;
                    out.flush()?;
                    Ok(())
                },
            )?
        }
    };
    let json = serde_json::to_string_pretty(&run)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.out)
        .with_context(|| format!("writing {}", args.out.display()))?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

/// The catalog id of an offered tier named `default` or `large`.
fn tier_model(name: &str) -> anyhow::Result<String> {
    use crate::infer::download::tiers::{TierSize, builtin_tiers};
    let size = match name {
        "default" => TierSize::Default,
        "large" => TierSize::Large,
        other => anyhow::bail!("--tier must be default or large, got {other:?}"),
    };
    builtin_tiers()
        .into_iter()
        .find(|tier| tier.size == size && tier.offered)
        .map(|tier| tier.id)
        .ok_or_else(|| anyhow::anyhow!("no offered {name} tier"))
}

/// The tier's models, or the id of the first one not installed. The bench
/// places by its own probe: `gpu = "off"` is set aside on this path only.
fn load_models(
    tier_id: String,
    fake: bool,
) -> anyhow::Result<std::result::Result<BenchModels, String>> {
    let started = std::time::Instant::now();
    if fake {
        return Ok(Ok(BenchModels {
            embedder: Arc::new(crate::infer::fake::FakeEmbedder::new()),
            generator: Arc::new(crate::infer::fake::FakeGenerator::new("fake answer [1]")),
            load_ms: started.elapsed().as_secs_f64() * 1000.0,
        }));
    }
    let home = crate::config::resolve_home(&|key| std::env::var_os(key))?;
    let mut config = crate::config::Config::load(&home)?;
    config.generation_model = tier_id;
    config.gpu = crate::gpu::GpuSetting::Auto;
    use crate::infer::models::ModelRole;
    for (id, role) in [
        (&config.embedding_model, ModelRole::Embedding),
        (&config.generation_model, ModelRole::Generation),
    ] {
        if let Err(crate::error::OragError::ModelNotInstalled { .. }) =
            crate::infer::models::find_model(&config.models_dir(), id, role)
        {
            return Ok(Err(id.clone()));
        }
    }
    let embedder = crate::app::load_embedder(&config)?;
    let generator = crate::app::load_generator(&config)?;
    Ok(Ok(BenchModels {
        embedder,
        generator,
        load_ms: started.elapsed().as_secs_f64() * 1000.0,
    }))
}

/// The desktop stops a bench child by closing its stdin, like the sidecar.
fn exit_when_stdin_closes() {
    std::thread::spawn(|| {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
        std::process::exit(EXIT_STDIN_CLOSED);
    });
}

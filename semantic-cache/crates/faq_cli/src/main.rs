use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use faq_core::{
    decide, evaluate_cases, load_entries_jsonl, save_entries_jsonl, CandleEmbeddingProvider,
    CandleEvaluationRun, Decision, EmbeddingProvider, EvalCase, FaqEntry, HashEmbeddingProvider,
    MiniLmEmbeddingProvider, OrchestrationStatus, Qwen3EmbeddingProvider, DEFAULT_EMBEDDING_DIM,
    DEFAULT_REQUIRED_PASS_RATE, DEFAULT_THRESHOLD,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "faq")]
#[command(about = "Semantic FAQ cache CLI")]
struct Cli {
    /// Path to the model file (.gguf or .safetensors). When provided with --tokenizer-path, uses neural embeddings.
    #[arg(long, global = true)]
    model_path: Option<PathBuf>,

    /// Path to the tokenizer.json file. Required when --model-path is set.
    #[arg(long, global = true)]
    tokenizer_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    BuildIndex {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Query {
        #[arg(long)]
        index: PathBuf,
        #[arg(long)]
        question: String,
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f32,
    },
    Eval {
        #[arg(long)]
        index: PathBuf,
        #[arg(long)]
        cases: PathBuf,
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f32,
        #[arg(long, default_value_t = DEFAULT_REQUIRED_PASS_RATE)]
        min_pass_rate: f32,
    },
}

#[derive(Debug, serde::Deserialize)]
struct RawFaq {
    id: String,
    question: String,
    answer: String,
}

fn read_raw_faq_jsonl(path: &Path) -> Result<Vec<RawFaq>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();

    for line in reader.lines() {
        let line = line.context("read input line")?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str::<RawFaq>(&line).context("parse raw faq json")?);
    }

    Ok(out)
}

fn read_eval_cases_json(path: &Path) -> Result<Vec<EvalCase>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let cases: Vec<EvalCase> = serde_json::from_reader(file).context("parse eval cases json")?;
    Ok(cases)
}

/// Detect the architecture of a safetensors file by reading its header JSON
/// and looking for known tensor names.
fn detect_safetensors_arch(path: &Path) -> Result<&'static str> {
    let mut file =
        File::open(path).with_context(|| format!("open safetensors: {}", path.display()))?;

    // First 8 bytes are a little-endian u64 giving the header size
    let mut size_buf = [0u8; 8];
    file.read_exact(&mut size_buf)
        .context("read safetensors header size")?;
    let header_size = u64::from_le_bytes(size_buf) as usize;

    // Cap at 10 MB to avoid reading the whole file
    let read_size = header_size.min(10 * 1024 * 1024);
    let mut header_buf = vec![0u8; read_size];
    file.read_exact(&mut header_buf)
        .context("read safetensors header JSON")?;

    let header = String::from_utf8_lossy(&header_buf);

    if header.contains("encoder.layer.0.attention.self.query.weight") {
        Ok("minilm")
    } else if header.contains("layers.0.self_attn.q_proj.weight") {
        Ok("qwen3")
    } else {
        anyhow::bail!(
            "unknown safetensors architecture in {}: header does not contain \
             known tensor names (expected BERT encoder.layer.* or Qwen3 layers.*)",
            path.display()
        )
    }
}

fn make_embedder(cli: &Cli) -> Result<Box<dyn EmbeddingProvider>> {
    match (&cli.model_path, &cli.tokenizer_path) {
        (Some(model), Some(tokenizer)) => {
            let ext = model.extension().and_then(|e| e.to_str()).unwrap_or("");
            eprintln!("Loading model from {} ...", model.display());
            let provider: Box<dyn EmbeddingProvider> = match ext {
                "gguf" => Box::new(CandleEmbeddingProvider::load(model, tokenizer)?),
                "safetensors" => match detect_safetensors_arch(model)? {
                    "minilm" => Box::new(MiniLmEmbeddingProvider::load(model, tokenizer)?),
                    "qwen3" => Box::new(Qwen3EmbeddingProvider::load(model, tokenizer)?),
                    other => anyhow::bail!("unknown safetensors architecture: {other}"),
                },
                other => anyhow::bail!(
                    "unsupported model format '.{other}' (expected .gguf or .safetensors)"
                ),
            };
            eprintln!("Model loaded.");
            Ok(provider)
        }
        (None, None) => Ok(Box::new(HashEmbeddingProvider::new(DEFAULT_EMBEDDING_DIM))),
        _ => anyhow::bail!("--model-path and --tokenizer-path must both be provided"),
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let model_name = cli
        .model_path
        .as_ref()
        .map(|p| {
            p.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        })
        .unwrap_or_else(|| "hash".to_string());

    match &cli.command {
        Commands::BuildIndex { input, output } => {
            let embedder = make_embedder(&cli)?;
            let raw = read_raw_faq_jsonl(input)?;
            let now = chrono::Utc::now();

            let mut entries = Vec::with_capacity(raw.len());
            for r in raw {
                entries.push(FaqEntry {
                    id: r.id,
                    question: r.question.clone(),
                    answer: r.answer,
                    embedding: embedder.embed(&r.question)?,
                    created_at: now,
                    updated_at: now,
                    expires_at: None,
                    product: None,
                    locale: None,
                    tags: Vec::new(),
                    version: None,
                    source: Some("human_curated".to_string()),
                    verified: Some(true),
                });
            }

            save_entries_jsonl(output, &entries)?;
            println!(
                "model={} indexed_entries={} output={}",
                model_name,
                entries.len(),
                output.display()
            );
        }
        Commands::Query {
            index,
            question,
            threshold,
        } => {
            let embedder = make_embedder(&cli)?;
            let entries = load_entries_jsonl(index)?;
            let q = embedder.embed(question)?;
            let result = decide(&q, &entries, *threshold);

            println!(
                "model={} decision={:?} score={:.4} entry_id={}",
                model_name,
                result.decision,
                result.score,
                result.entry_id.as_deref().unwrap_or("null")
            );
            if result.decision == Decision::Hit {
                println!("answer={}", result.answer.as_deref().unwrap_or(""));
            }
        }
        Commands::Eval {
            index,
            cases,
            threshold,
            min_pass_rate,
        } => {
            let run_id = format!("eval-{}", chrono::Utc::now().timestamp_millis());
            let mut run = CandleEvaluationRun::start(
                run_id,
                cases.to_string_lossy().into_owned(),
                Some(*threshold),
            );
            run.required_pass_rate = *min_pass_rate;

            // Check model file when using candle backend
            if cli.model_path.is_some() {
                if let Some(mp) = &cli.model_path {
                    if !mp.exists() {
                        run.on_runtime_boot_failed("runtime_boot_failed:model_file_not_found");
                    } else {
                        run.on_runtime_ready();
                    }
                }
            } else {
                run.on_runtime_ready();
            }

            if run.status == OrchestrationStatus::Failed {
                println!(
                    "run_id={} status={:?} required={:.4} error={}",
                    run.run_id,
                    run.status,
                    run.required_pass_rate,
                    run.error.as_deref().unwrap_or("unknown")
                );
                return Ok(());
            }

            let embedder = make_embedder(&cli)?;
            let entries = load_entries_jsonl(index)?;
            let cases = read_eval_cases_json(cases)?;
            let summary = evaluate_cases(&embedder, &entries, &cases, *threshold)?;
            run.on_eval_completed(&summary, *min_pass_rate);

            println!(
                "run_id={} model={} status={:?} total={} passed={} failed={} pass_rate={:.4} required={:.4} meets_threshold={}",
                run.run_id,
                model_name,
                run.status,
                summary.total,
                summary.passed,
                summary.failed,
                summary.pass_rate,
                run.required_pass_rate,
                run.meets_threshold()
            );

            for o in &summary.outcomes {
                println!(
                    "case={} passed={} decision={:?} faq_id={} score={:.4} latency={:.1}ms",
                    o.case_id,
                    o.passed,
                    o.actual_decision,
                    o.actual_faq_id.as_deref().unwrap_or("null"),
                    o.score,
                    o.latency_ms
                );
            }

            let total_ms: f64 = summary.outcomes.iter().map(|o| o.latency_ms).sum();
            let avg_ms = total_ms / summary.outcomes.len().max(1) as f64;
            println!(
                "total_latency={:.1}ms avg_latency={:.1}ms",
                total_ms, avg_ms
            );
        }
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

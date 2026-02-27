# Semantic FAQ Cache

A Rust CLI that matches user questions to a curated FAQ database using semantic
similarity. It runs a quantized
[nomic-embed-text-v2-moe](https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe)
model locally via [Candle](https://github.com/huggingface/candle) — no external
API calls needed.

## How it works

1. **Build an index** — embeds each FAQ question into a 768-dimensional vector
   and stores the result as JSONL.
2. **Query** — embeds the user's question, computes cosine similarity against
   all indexed FAQs, and returns the best match if it exceeds a threshold.
3. **Eval** — runs a set of test cases against the index, reporting pass/fail,
   similarity scores, and per-query latency.

## Prerequisites

- Rust toolchain (1.75+)
- Model files: download the GGUF model and tokenizer into the `models/` folder.
  See [models/README.md](models/README.md) for instructions.

## Build

```bash
cargo build --release
```

On Apple Silicon (macOS or Linux via Lima), the `.cargo/config.toml` enables the
`+fp16` target feature required by the `gemm-f16` crate. The first build
compiles ~214 crates and takes a few minutes.

## Usage

All commands accept optional `--model-path` and `--tokenizer-path` flags. When
provided, the CLI uses the Candle neural network backend for real semantic
embeddings. Without them, it falls back to a deterministic hash-based embedding
(useful for fast testing but not semantically meaningful).

### Build index

```bash
./target/release/faq_cli \
  --model-path ./models/nomic-embed-text-v2-moe.Q4_K_M.gguf \
  --tokenizer-path ./models/tokenizer.json \
  build-index --input data/faq_seed.jsonl --output /tmp/faq_index.jsonl
```

### Query

```bash
./target/release/faq_cli \
  --model-path ./models/nomic-embed-text-v2-moe.Q4_K_M.gguf \
  --tokenizer-path ./models/tokenizer.json \
  query --index /tmp/faq_index.jsonl \
  --question "I forgot my password"
```

Use `--threshold` to adjust hit/miss sensitivity (default: 0.55).

### Eval

```bash
./target/release/faq_cli \
  --model-path ./models/nomic-embed-text-v2-moe.Q4_K_M.gguf \
  --tokenizer-path ./models/tokenizer.json \
  eval --index /tmp/faq_index.jsonl --cases data/eval_cases.json
```

Output includes per-case pass/fail, similarity score, latency, and summary
statistics.

### Without the model (hash backend)

```bash
./target/release/faq_cli build-index --input data/faq_seed.jsonl --output /tmp/faq_hash.jsonl
./target/release/faq_cli eval --index /tmp/faq_hash.jsonl --cases data/eval_cases.json
```


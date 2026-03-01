# Semantic FAQ Cache

A Rust CLI that matches user questions to a curated FAQ database using semantic
similarity. It runs embedding models locally via
[Candle](https://github.com/huggingface/candle) — no external API calls needed.

Supported backends:

- [nomic-embed-text-v2-moe](https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe)
  (GGUF, 768-dim)
- [pplx-embed-v1-0.6b](https://huggingface.co/perplexity-ai/pplx-embed-v1-0.6b)
  (safetensors, 1024-dim)
- [all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2)
  (safetensors, 384-dim)

## How it works

1. **Build an index** — embeds each FAQ question into a vector (768 or 1024-dim
   depending on model) and stores the result as JSONL.
2. **Query** — embeds the user's question, computes cosine similarity against
   all indexed FAQs, and returns the best match if it exceeds a threshold.
3. **Eval** — runs a set of test cases against the index, reporting pass/fail,
   similarity scores, and per-query latency.
4. **Cluster** — reads a parquet dataset (e.g. SQuAD v2), embeds all questions,
   and groups them by cosine similarity to discover recurring FAQ patterns.

## Prerequisites

- Rust toolchain (1.75+)
- Model files: download at least one model and its tokenizer into the `models/`
  folder. See [models/README.md](models/README.md) for instructions.

## Build

```bash
cargo build --release
```

On Apple Silicon (macOS or Linux via Lima), the `.cargo/config.toml` enables the
`+fp16` target feature required by the `gemm-f16` crate. The first build
compiles ~214 crates and takes a few minutes.

## Usage

All commands accept optional `--model-path` and `--tokenizer-path` flags. The
CLI auto-detects the backend from the model file extension (`.gguf` for
nomic-bert-moe, `.safetensors` auto-detected as MiniLM or Qwen3). Without these
flags, it falls back to a deterministic hash-based embedding (useful for fast
testing but not semantically meaningful).

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
  eval --index /tmp/faq_index.jsonl --cases bench/eval_cases.json
```

Output includes per-case pass/fail, similarity score, latency, and summary
statistics.

### With pplx-embed (safetensors backend)

```bash
./target/release/faq_cli \
  --model-path ./models/pplx-embed-v1-0.6b.safetensors \
  --tokenizer-path ./models/pplx-embed-v1-0.6b-tokenizer.json \
  build-index --input data/faq_seed.jsonl --output /tmp/faq_pplx.jsonl

./target/release/faq_cli \
  --model-path ./models/pplx-embed-v1-0.6b.safetensors \
  --tokenizer-path ./models/pplx-embed-v1-0.6b-tokenizer.json \
  query --index /tmp/faq_pplx.jsonl --question "I forgot my password"
```

### With all-MiniLM-L6-v2 (safetensors backend)

```bash
./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  build-index --input data/faq_seed.jsonl --output /tmp/faq_minilm.jsonl

./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  query --index /tmp/faq_minilm.jsonl --question "I forgot my password"
```

### Cluster (FAQ discovery from datasets)

Reads a parquet dataset (e.g. SQuAD v2), embeds all questions, and groups them
by cosine similarity to surface recurring FAQ patterns.

```bash
# Download the dataset
pip install huggingface_hub
python -c "
from huggingface_hub import hf_hub_download
for split in ['train', 'validation']:
    hf_hub_download('rajpurkar/squad_v2', f'{split}.parquet',
                    repo_type='dataset', local_dir='data/squad_v2')
"
```

#### With neural embeddings (recommended)

```bash
./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  cluster \
  --input data/squad_v2/validation.parquet \
  --threshold 0.90 \
  --min-size 3 \
  --top 20
```

#### With hash embeddings (fast, for testing)

```bash
./target/release/faq_cli cluster \
  --input data/squad_v2/validation.parquet \
  --threshold 0.80 \
  --min-size 3 \
  --top 20
```

#### Options

| Flag | Default | Description |
|---|---|---|
| `--input` | (required) | Path to a `.parquet` file with a `question` column |
| `--threshold` | `0.80` | Cosine similarity threshold for grouping |
| `--min-size` | `3` | Minimum cluster size to report |
| `--top` | `10` | Number of top clusters to display |

#### Threshold guide

| Threshold | Use case |
|---|---|
| `0.80` | Broad topic mapping |
| `0.90` | FAQ extraction (recommended) |
| `0.95` | Near-duplicate / adversarial pair detection |

### Without the model (hash backend)

```bash
./target/release/faq_cli build-index --input data/faq_seed.jsonl --output /tmp/faq_hash.jsonl
./target/release/faq_cli eval --index /tmp/faq_hash.jsonl --cases bench/eval_cases.json
```

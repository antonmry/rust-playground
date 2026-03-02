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
4. **Cluster** — reads a dataset (SQuAD v2 parquet or Bitext CSV), embeds every
   question, and groups them by cosine similarity to surface recurring FAQ
   patterns. Optionally exports a 2D scatter plot (HTML) using PCA or t-SNE
   projection, plus structured JSON.

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
  build-index --input data/faq_seed.jsonl --output bench/index_nomic.jsonl
```

### Query

```bash
./target/release/faq_cli \
  --model-path ./models/nomic-embed-text-v2-moe.Q4_K_M.gguf \
  --tokenizer-path ./models/tokenizer.json \
  query --index bench/index_nomic.jsonl \
  --question "I forgot my password"
```

Use `--threshold` to adjust hit/miss sensitivity (default: 0.55).

### Eval

```bash
./target/release/faq_cli \
  --model-path ./models/nomic-embed-text-v2-moe.Q4_K_M.gguf \
  --tokenizer-path ./models/tokenizer.json \
  eval --index bench/index_nomic.jsonl --cases data/eval_cases.json
```

Output includes per-case pass/fail, similarity score, latency, and summary
statistics.

### With pplx-embed (safetensors backend)

```bash
./target/release/faq_cli \
  --model-path ./models/pplx-embed-v1-0.6b.safetensors \
  --tokenizer-path ./models/pplx-embed-v1-0.6b-tokenizer.json \
  build-index --input data/faq_seed.jsonl --output bench/index_pplx.jsonl

./target/release/faq_cli \
  --model-path ./models/pplx-embed-v1-0.6b.safetensors \
  --tokenizer-path ./models/pplx-embed-v1-0.6b-tokenizer.json \
  query --index bench/index_pplx.jsonl --question "I forgot my password"
```

### With all-MiniLM-L6-v2 (safetensors backend)

```bash
./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  build-index --input data/faq_seed.jsonl --output bench/index_minilm.jsonl

./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  query --index bench/index_minilm.jsonl --question "I forgot my password"
```

### Without the model (hash backend)

```bash
./target/release/faq_cli build-index --input data/faq_seed.jsonl --output bench/index_hash.jsonl
./target/release/faq_cli eval --index bench/index_hash.jsonl --cases data/eval_cases.json
```

### Cluster

Embeds every question in a dataset and groups them by cosine similarity to
identify recurring FAQ patterns. Supports two input formats:

- **Parquet** — [SQuAD v2](https://huggingface.co/datasets/rajpurkar/squad_v2)
  schema (`id`, `title`, `context`, `question`, `answers`)
- **CSV** —
  [Bitext customer support](https://huggingface.co/datasets/bitext/Bitext-customer-support-llm-chatbot-training-dataset)
  schema (`flags`, `instruction`, `category`, `intent`, `response`)

The format is auto-detected from the file extension.

**Download a dataset:**

```bash
# SQuAD v2 (parquet)
mkdir -p data/squad_v2
huggingface-cli download rajpurkar/squad_v2 --repo-type dataset --local-dir data/squad_v2

# Bitext customer support (CSV)
mkdir -p data/bitext-support
huggingface-cli download bitext/Bitext-customer-support-llm-chatbot-training-dataset \
  --repo-type dataset --local-dir data/bitext-support
```

**Basic usage (hash backend, fast but not semantically meaningful):**

```bash
./target/release/faq_cli cluster \
  --input data/squad_v2/validation.parquet \
  --threshold 0.80 --min-size 3 --top 20
```

**With neural embeddings (recommended):**

```bash
./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  cluster --input data/bitext-support/Bitext_Sample_Customer_Support_Training_Dataset_27K_responses-v11.csv \
  --threshold 0.95 --max-points 1000 --min-size 3 --top 20
```

**With JSON export and HTML scatter plot (t-SNE):**

```bash
./target/release/faq_cli \
  --model-path ./models/all-MiniLM-L6-v2.safetensors \
  --tokenizer-path ./models/all-MiniLM-L6-v2-tokenizer.json \
  cluster --input data/bitext-support/Bitext_Sample_Customer_Support_Training_Dataset_27K_responses-v11.csv \
  --threshold 0.95 --max-points 1000 \
  --projection tsne \
  --json-out clusters.json \
  --plot-out clusters.html
```

Open `clusters.html` in a browser to see an interactive scatter plot of
the clustered questions with hover tooltips.

**Cluster options:**

| Flag           | Default    | Description                                          |
| -------------- | ---------- | ---------------------------------------------------- |
| `--input`      | (required) | Path to input file (`.parquet` or `.csv`)            |
| `--threshold`  | `0.80`     | Cosine similarity threshold for grouping             |
| `--min-size`   | `2`        | Minimum cluster size to display                      |
| `--top`        | `50`       | Maximum number of clusters to show                   |
| `--json-out`   | —          | Write structured JSON with clusters + 2D projections |
| `--plot-out`   | —          | Write standalone HTML scatter plot                   |
| `--projection` | `pca`      | 2D projection method: `pca` or `tsne`                |
| `--max-points` | —          | Downsample to N points before embedding              |

**Projection methods:**

| Method | Best for                      | Notes                                         |
| ------ | ----------------------------- | --------------------------------------------- |
| `pca`  | Fast overview, large datasets | Linear; clusters may overlap visually         |
| `tsne` | Cluster visualization         | Non-linear; clusters appear as distinct blobs |

**Threshold guide:**

| Threshold | Use case                                                              |
| --------- | --------------------------------------------------------------------- |
| `0.80`    | Topic mapping — broad semantic categories                             |
| `0.90`    | FAQ extraction — semantically coherent groups (SQuAD v2)              |
| `0.95`    | FAQ extraction for homogeneous datasets (Bitext), duplicate detection |

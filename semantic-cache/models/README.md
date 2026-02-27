# Models

This folder contains model and tokenizer files for the embedding backends.

## Available models

| File                                   | Size     | Description                              |
| -------------------------------------- | -------- | ---------------------------------------- |
| `nomic-embed-text-v2-moe.Q4_K_M.gguf` | ~329 MB  | Quantized nomic-bert-moe (Q4_K_M, GGUF) |
| `tokenizer.json`                       | ~17 MB   | Nomic tokenizer vocabulary               |
| `pplx-embed-v1-0.6b.safetensors`      | ~1.2 GB  | Perplexity Qwen3 embedding (safetensors) |
| `pplx-embed-v1-0.6b-tokenizer.json`   | ~7 MB    | Perplexity/Qwen3 tokenizer vocabulary    |
| `all-MiniLM-L6-v2.safetensors`        | ~91 MB   | all-MiniLM-L6-v2 BERT embedding          |
| `all-MiniLM-L6-v2-tokenizer.json`     | ~466 KB  | MiniLM tokenizer vocabulary              |

## Download

### Nomic (GGUF)

```bash
cd models/

curl -L -O https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe/resolve/main/nomic-embed-text-v2-moe.Q4_K_M.gguf
curl -L -O https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe/resolve/main/tokenizer.json
```

### Perplexity pplx-embed (safetensors)

```bash
cd models/

# Download the safetensors model
curl -L -o pplx-embed-v1-0.6b.safetensors \
  https://huggingface.co/perplexity-ai/pplx-embed-v1-0.6b/resolve/main/model.safetensors

# Download the tokenizer
curl -L -o pplx-embed-v1-0.6b-tokenizer.json \
  https://huggingface.co/perplexity-ai/pplx-embed-v1-0.6b/resolve/main/tokenizer.json
```

### all-MiniLM-L6-v2 (safetensors)

```bash
cd models/

# Download the safetensors model
curl -L -o all-MiniLM-L6-v2.safetensors \
  https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/model.safetensors

# Download the tokenizer
curl -L -o all-MiniLM-L6-v2-tokenizer.json \
  https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json
```

### Using huggingface-cli

```bash
pip install huggingface_hub

# Nomic
huggingface-cli download nomic-ai/nomic-embed-text-v2-moe \
  nomic-embed-text-v2-moe.Q4_K_M.gguf tokenizer.json \
  --local-dir models/

# Perplexity (rename files after download)
huggingface-cli download perplexity-ai/pplx-embed-v1-0.6b \
  model.safetensors tokenizer.json \
  --local-dir /tmp/pplx-embed/
cp /tmp/pplx-embed/model.safetensors models/pplx-embed-v1-0.6b.safetensors
cp /tmp/pplx-embed/tokenizer.json models/pplx-embed-v1-0.6b-tokenizer.json

# MiniLM (rename files after download)
huggingface-cli download sentence-transformers/all-MiniLM-L6-v2 \
  model.safetensors tokenizer.json \
  --local-dir /tmp/minilm/
cp /tmp/minilm/model.safetensors models/all-MiniLM-L6-v2.safetensors
cp /tmp/minilm/tokenizer.json models/all-MiniLM-L6-v2-tokenizer.json
```

## Verify

```bash
ls -lh models/
# nomic-embed-text-v2-moe.Q4_K_M.gguf   ~329 MB
# tokenizer.json                          ~17 MB
# pplx-embed-v1-0.6b.safetensors         ~1.2 GB
# pplx-embed-v1-0.6b-tokenizer.json      ~7 MB
# all-MiniLM-L6-v2.safetensors           ~91 MB
# all-MiniLM-L6-v2-tokenizer.json        ~466 KB
```

## Embedding backends

| Backend                 | Flag                                | Dim  | Speed        | Quality                            |
| ----------------------- | ----------------------------------- | ---- | ------------ | ---------------------------------- |
| Candle (nomic-bert-moe) | `--model-path` + `--tokenizer-path` | 768  | ~600ms/query | Real semantic similarity           |
| Qwen3 (pplx-embed)     | `--model-path` + `--tokenizer-path` | 1024 | TBD          | Real semantic similarity           |
| MiniLM (all-MiniLM-L6-v2) | `--model-path` + `--tokenizer-path` | 384 | TBD          | Real semantic similarity           |
| Hash                    | *(none)*                            | 768  | <0.1ms/query | Deterministic, no semantic meaning |

The CLI auto-detects the backend from the model file extension:
- `.gguf` files use the nomic-bert-moe backend
- `.safetensors` files are auto-detected by reading the header (BERT → MiniLM, Qwen3 → pplx-embed)

## Model details

### nomic-bert-moe (GGUF)

- 12 transformer layers with BERT-style post-norm (LayerNorm after residual)
- Mixture of Experts (8 experts, top-2 routing) on odd-indexed layers
- RoPE positional encoding (NeoX half-split variant)
- Mean pooling + L2 normalization
- Q4_K_M quantization (~329 MB vs ~1.4 GB unquantized)

### pplx-embed-v1-0.6b (safetensors)

- 28 transformer layers with pre-norm (RMSNorm before attention/MLP)
- Grouped Query Attention (16 Q heads, 8 KV heads, head_dim=128)
- SwiGLU MLP (gate + up projection with SiLU activation)
- Per-head Q/K RMSNorm before RoPE (theta=1M)
- Mean pooling + L2 normalization
- Full precision (~1.2 GB, 0.6B parameters)

### all-MiniLM-L6-v2 (safetensors)

- 6 transformer layers with BERT-style post-norm (LayerNorm after residual)
- 12 attention heads (head_dim=32), separate Q/K/V projections with biases
- FFN: 384→1536 + GELU(erf) + 1536→384 with biases
- Absolute position embeddings (max 512 tokens)
- Mean pooling + L2 normalization
- Full precision (~91 MB, 22M parameters, 384-dim)

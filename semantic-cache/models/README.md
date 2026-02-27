# Models

This folder contains the model and tokenizer files required by the Candle
embedding backend.

## Required files

| File                                  | Size    | Description                           |
| ------------------------------------- | ------- | ------------------------------------- |
| `nomic-embed-text-v2-moe.Q4_K_M.gguf` | ~329 MB | Quantized embedding model (Q4_K_M)    |
| `tokenizer.json`                      | ~17 MB  | SentencePiece/T5 tokenizer vocabulary |

## Download

### Using curl

```bash
cd models/

# Download the GGUF model
curl -L -O https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe/resolve/main/nomic-embed-text-v2-moe.Q4_K_M.gguf

# Download the tokenizer
curl -L -O https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe/resolve/main/tokenizer.json
```

### Using wget

```bash
cd models/

wget https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe/resolve/main/nomic-embed-text-v2-moe.Q4_K_M.gguf
wget https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe/resolve/main/tokenizer.json
```

### Using huggingface-cli

```bash
pip install huggingface_hub
huggingface-cli download nomic-ai/nomic-embed-text-v2-moe \
  nomic-embed-text-v2-moe.Q4_K_M.gguf tokenizer.json \
  --local-dir models/
```

## Verify

After downloading, the folder should contain:

```text
models/
  README.md
  nomic-embed-text-v2-moe.Q4_K_M.gguf
  tokenizer.json
```

You can verify the files with:

```bash
ls -lh models/
# nomic-embed-text-v2-moe.Q4_K_M.gguf  ~329 MB
# tokenizer.json                         ~17 MB
```

## Embedding backends

| Backend                 | Flag                                | Dim | Speed        | Quality                            |
| ----------------------- | ----------------------------------- | --- | ------------ | ---------------------------------- |
| Candle (nomic-bert-moe) | `--model-path` + `--tokenizer-path` | 768 | ~600ms/query | Real semantic similarity           |
| Hash                    | *(none)*                            | 768 | <0.1ms/query | Deterministic, no semantic meaning |

## Model details

The Candle backend loads `nomic-embed-text-v2-moe.Q4_K_M.gguf` and implements
the full nomic-bert-moe architecture:

- 12 transformer layers with BERT-style post-norm (LayerNorm after residual)
- Mixture of Experts (8 experts, top-2 routing) on odd-indexed layers
- RoPE positional encoding (NeoX half-split variant)
- Mean pooling + L2 normalization
- Q4_K_M quantization (~329 MB vs ~1.4 GB unquantized)

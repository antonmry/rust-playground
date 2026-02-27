use anyhow::{bail, Context, Result};
use candle_core::quantized::{gguf_file, QMatMul, QTensor};
use candle_core::{DType, Device, IndexOp, Module, Tensor};
use std::path::Path;

use crate::embed::EmbeddingProvider;

// ---------------------------------------------------------------------------
// Config derived from GGUF metadata
// ---------------------------------------------------------------------------

struct ModelConfig {
    num_heads: usize,
    head_dim: usize,
    num_layers: usize,
    num_experts: usize,
    num_active_experts: usize,
    moe_every_n_layers: usize,
    rope_freq_base: f32,
    layer_norm_eps: f64,
    max_seq_len: usize,
}

impl ModelConfig {
    fn from_gguf(content: &gguf_file::Content) -> Result<Self> {
        let get_u32 = |key: &str| -> Result<u32> {
            match content.metadata.get(key) {
                Some(gguf_file::Value::U32(v)) => Ok(*v),
                _ => bail!("missing or invalid GGUF metadata: {key}"),
            }
        };
        let get_f32 = |key: &str| -> Result<f32> {
            match content.metadata.get(key) {
                Some(gguf_file::Value::F32(v)) => Ok(*v),
                _ => bail!("missing or invalid GGUF metadata: {key}"),
            }
        };

        let hidden_size = get_u32("nomic-bert-moe.embedding_length")? as usize;
        let num_heads = get_u32("nomic-bert-moe.attention.head_count")? as usize;
        let head_dim = hidden_size / num_heads;
        let num_layers = get_u32("nomic-bert-moe.block_count")? as usize;
        let num_experts = get_u32("nomic-bert-moe.expert_count")? as usize;
        let num_active_experts = get_u32("nomic-bert-moe.expert_used_count")? as usize;
        let moe_every_n_layers = get_u32("nomic-bert-moe.moe_every_n_layers")? as usize;
        let rope_freq_base = get_f32("nomic-bert-moe.rope.freq_base")?;
        let layer_norm_eps = get_f32("nomic-bert-moe.attention.layer_norm_epsilon")? as f64;
        let max_seq_len = get_u32("nomic-bert-moe.context_length")? as usize;

        Ok(Self {
            num_heads,
            head_dim,
            num_layers,
            num_experts,
            num_active_experts,
            moe_every_n_layers,
            rope_freq_base,
            layer_norm_eps,
            max_seq_len,
        })
    }
}

// ---------------------------------------------------------------------------
// Layer norm
// ---------------------------------------------------------------------------

struct LayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f64,
}

impl LayerNorm {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x_dtype = x.dtype();
        let x = x.to_dtype(DType::F32)?;
        let mean = x.mean_keepdim(candle_core::D::Minus1)?;
        let diff = x.broadcast_sub(&mean)?;
        let var = diff.sqr()?.mean_keepdim(candle_core::D::Minus1)?;
        let std = (var + self.eps)?.sqrt()?;
        let normed = diff.broadcast_div(&std)?;
        let out = normed
            .broadcast_mul(&self.weight)?
            .broadcast_add(&self.bias)?;
        out.to_dtype(x_dtype).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Feed-forward variants
// ---------------------------------------------------------------------------

enum FeedForward {
    Regular {
        up_w: QMatMul,
        up_b: Tensor,
        down_w: QMatMul,
        down_b: Tensor,
    },
    MoE {
        gate: Tensor,
        up_exps: QTensor,
        down_exps: QTensor,
        _num_experts: usize,
        num_active: usize,
    },
}

fn gelu(x: &Tensor) -> Result<Tensor> {
    x.gelu_erf().map_err(Into::into)
}

impl FeedForward {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        match self {
            FeedForward::Regular {
                up_w,
                up_b,
                down_w,
                down_b,
            } => {
                let h = up_w.forward(x)?.broadcast_add(up_b)?;
                let h = gelu(&h)?;
                let out = down_w.forward(&h)?.broadcast_add(down_b)?;
                Ok(out)
            }
            FeedForward::MoE {
                gate,
                up_exps,
                down_exps,
                num_active,
                ..
            } => moe_forward(x, gate, up_exps, down_exps, *num_active),
        }
    }
}

fn moe_forward(
    x: &Tensor,
    gate: &Tensor,
    up_exps: &QTensor,
    down_exps: &QTensor,
    num_active: usize,
) -> Result<Tensor> {
    let device = x.device();
    let (batch, seq_len, hidden) = x.dims3()?;
    let flat = x.reshape((batch * seq_len, hidden))?;

    let router_logits = flat.matmul(&gate.t()?)?;
    let router_probs = candle_nn::ops::softmax(&router_logits, candle_core::D::Minus1)?;

    let up_all = up_exps.dequantize(device)?;
    let down_all = down_exps.dequantize(device)?;

    let num_tokens = batch * seq_len;
    let mut output = Tensor::zeros((num_tokens, hidden), DType::F32, device)?;

    let probs_data = router_probs.to_vec2::<f32>()?;

    for (token_idx, probs) in probs_data.iter().enumerate().take(num_tokens) {
        let mut indexed: Vec<(usize, f32)> = probs.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_k: Vec<(usize, f32)> = indexed.into_iter().take(num_active).collect();
        let weight_sum: f32 = top_k.iter().map(|(_, w)| w).sum();

        let token_vec = flat.i(token_idx)?;
        let mut expert_sum = Tensor::zeros(&[hidden], DType::F32, device)?;

        for &(expert_idx, raw_weight) in &top_k {
            let w = raw_weight / weight_sum;
            let up_w = up_all.i(expert_idx)?;
            let h = token_vec.unsqueeze(0)?.matmul(&up_w.t()?)?.squeeze(0)?;
            let h = gelu(&h)?;
            let down_w = down_all.i(expert_idx)?;
            let out = h.unsqueeze(0)?.matmul(&down_w.t()?)?.squeeze(0)?;
            expert_sum = (expert_sum + (out * w as f64)?)?;
        }

        output = output.slice_assign(
            &[token_idx..token_idx + 1, 0..hidden],
            &expert_sum.unsqueeze(0)?,
        )?;
    }

    output.reshape((batch, seq_len, hidden)).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Transformer layer (BERT-style post-norm)
// ---------------------------------------------------------------------------

struct TransformerLayer {
    attn_norm: LayerNorm,
    attn_qkv_w: QMatMul,
    attn_qkv_b: Tensor,
    attn_out_w: QMatMul,
    attn_out_b: Tensor,
    ffn_norm: LayerNorm,
    ffn: FeedForward,
}

impl TransformerLayer {
    fn forward(
        &self,
        x: &Tensor,
        cos: &Tensor,
        sin: &Tensor,
        num_heads: usize,
        head_dim: usize,
    ) -> Result<Tensor> {
        let (batch, seq_len, _hidden) = x.dims3()?;

        // Self-attention on raw input
        let qkv = self
            .attn_qkv_w
            .forward(x)?
            .broadcast_add(&self.attn_qkv_b)?;

        let qkv = qkv.reshape((batch, seq_len, 3, num_heads, head_dim))?;
        let q = qkv.i((.., .., 0))?.contiguous()?;
        let k = qkv.i((.., .., 1))?.contiguous()?;
        let v = qkv.i((.., .., 2))?.contiguous()?;

        let q = q.transpose(1, 2)?.contiguous()?;
        let k = k.transpose(1, 2)?.contiguous()?;
        let v = v.transpose(1, 2)?.contiguous()?;

        let q = apply_rope(&q, cos, sin)?;
        let k = apply_rope(&k, cos, sin)?;

        let scale = (head_dim as f64).sqrt();
        let attn_weights = q.matmul(&k.t()?)?.affine(1.0 / scale, 0.0)?;
        let attn_weights = candle_nn::ops::softmax(&attn_weights, candle_core::D::Minus1)?;
        let attn_out = attn_weights.matmul(&v)?;

        let attn_out = attn_out.transpose(1, 2)?.contiguous()?.reshape((
            batch,
            seq_len,
            num_heads * head_dim,
        ))?;

        let attn_out = self
            .attn_out_w
            .forward(&attn_out)?
            .broadcast_add(&self.attn_out_b)?;

        // Residual + post-norm
        let x = (x + attn_out)?;
        let x = self.attn_norm.forward(&x)?;

        // FFN + residual + post-norm
        let ffn_out = self.ffn.forward(&x)?;
        let x = (x + ffn_out)?;
        self.ffn_norm.forward(&x)
    }
}

// ---------------------------------------------------------------------------
// RoPE (NeoX half-split variant, matching llama.cpp LLAMA_ROPE_TYPE_NEOX)
// ---------------------------------------------------------------------------

fn precompute_rope(
    head_dim: usize,
    max_len: usize,
    freq_base: f32,
    device: &Device,
) -> Result<(Tensor, Tensor)> {
    let half_dim = head_dim / 2;
    let mut freqs = Vec::with_capacity(half_dim);
    for i in 0..half_dim {
        freqs.push(1.0f32 / freq_base.powf(i as f32 / half_dim as f32));
    }
    let freqs = Tensor::new(freqs.as_slice(), device)?;

    let positions: Vec<f32> = (0..max_len).map(|i| i as f32).collect();
    let positions = Tensor::new(positions.as_slice(), device)?;

    let angles = positions
        .unsqueeze(1)?
        .broadcast_mul(&freqs.unsqueeze(0)?)?;
    let cos = angles.cos()?;
    let sin = angles.sin()?;

    Ok((cos, sin))
}

fn apply_rope(x: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
    let (_b, _h, seq_len, head_dim) = x.dims4()?;
    let half = head_dim / 2;

    let cos = cos.i(..seq_len)?;
    let sin = sin.i(..seq_len)?;

    let x1 = x.narrow(3, 0, half)?;
    let x2 = x.narrow(3, half, half)?;

    let cos = cos.unsqueeze(0)?.unsqueeze(0)?;
    let sin = sin.unsqueeze(0)?.unsqueeze(0)?;

    let rotated_x1 = (x1.broadcast_mul(&cos)? - x2.broadcast_mul(&sin)?)?;
    let rotated_x2 = (x1.broadcast_mul(&sin)? + x2.broadcast_mul(&cos)?)?;

    Tensor::cat(&[&rotated_x1, &rotated_x2], 3).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Full model
// ---------------------------------------------------------------------------

struct NomicBertMoeModel {
    token_embeddings: Tensor,
    token_type_embedding: Tensor,
    embedding_norm: LayerNorm,
    layers: Vec<TransformerLayer>,
    rope_cos: Tensor,
    rope_sin: Tensor,
    config: ModelConfig,
}

impl NomicBertMoeModel {
    fn load(path: &Path, device: &Device) -> Result<Self> {
        let mut file =
            std::fs::File::open(path).with_context(|| format!("open GGUF: {}", path.display()))?;
        let content = gguf_file::Content::read(&mut file).context("parse GGUF")?;
        let config = ModelConfig::from_gguf(&content)?;

        let mut get_tensor = |name: &str| -> Result<QTensor> {
            content
                .tensor(&mut file, name, device)
                .with_context(|| format!("load tensor: {name}"))
        };

        let token_embeddings = get_tensor("token_embd.weight")?.dequantize(device)?;
        let token_type_embedding = get_tensor("token_types.weight")?.dequantize(device)?;

        let embedding_norm = LayerNorm {
            weight: get_tensor("token_embd_norm.weight")?.dequantize(device)?,
            bias: get_tensor("token_embd_norm.bias")?.dequantize(device)?,
            eps: config.layer_norm_eps,
        };

        let (rope_cos, rope_sin) = precompute_rope(
            config.head_dim,
            config.max_seq_len,
            config.rope_freq_base,
            device,
        )?;

        let mut layers = Vec::with_capacity(config.num_layers);
        for i in 0..config.num_layers {
            let prefix = format!("blk.{i}");

            let attn_norm = LayerNorm {
                weight: get_tensor(&format!("{prefix}.attn_output_norm.weight"))?
                    .dequantize(device)?,
                bias: get_tensor(&format!("{prefix}.attn_output_norm.bias"))?.dequantize(device)?,
                eps: config.layer_norm_eps,
            };

            let attn_qkv_w =
                QMatMul::from_qtensor(get_tensor(&format!("{prefix}.attn_qkv.weight"))?)?;
            let attn_qkv_b = get_tensor(&format!("{prefix}.attn_qkv.bias"))?.dequantize(device)?;

            let attn_out_w =
                QMatMul::from_qtensor(get_tensor(&format!("{prefix}.attn_output.weight"))?)?;
            let attn_out_b =
                get_tensor(&format!("{prefix}.attn_output.bias"))?.dequantize(device)?;

            let ffn_norm = LayerNorm {
                weight: get_tensor(&format!("{prefix}.layer_output_norm.weight"))?
                    .dequantize(device)?,
                bias: get_tensor(&format!("{prefix}.layer_output_norm.bias"))?
                    .dequantize(device)?,
                eps: config.layer_norm_eps,
            };

            let is_moe = (i % config.moe_every_n_layers) != 0;
            let ffn = if is_moe {
                let gate =
                    get_tensor(&format!("{prefix}.ffn_gate_inp.weight"))?.dequantize(device)?;
                FeedForward::MoE {
                    gate,
                    up_exps: get_tensor(&format!("{prefix}.ffn_up_exps.weight"))?,
                    down_exps: get_tensor(&format!("{prefix}.ffn_down_exps.weight"))?,
                    _num_experts: config.num_experts,
                    num_active: config.num_active_experts,
                }
            } else {
                FeedForward::Regular {
                    up_w: QMatMul::from_qtensor(get_tensor(&format!("{prefix}.ffn_up.weight"))?)?,
                    up_b: get_tensor(&format!("{prefix}.ffn_up.bias"))?.dequantize(device)?,
                    down_w: QMatMul::from_qtensor(get_tensor(&format!(
                        "{prefix}.ffn_down.weight"
                    ))?)?,
                    down_b: get_tensor(&format!("{prefix}.ffn_down.bias"))?.dequantize(device)?,
                }
            };

            layers.push(TransformerLayer {
                attn_norm,
                attn_qkv_w,
                attn_qkv_b,
                attn_out_w,
                attn_out_b,
                ffn_norm,
                ffn,
            });
        }

        Ok(Self {
            token_embeddings,
            token_type_embedding,
            embedding_norm,
            layers,
            rope_cos,
            rope_sin,
            config,
        })
    }

    fn forward(&self, token_ids: &[u32]) -> Result<Vec<f32>> {
        let device = self.token_embeddings.device();
        let seq_len = token_ids.len();

        if seq_len > self.config.max_seq_len {
            bail!(
                "input length {seq_len} exceeds max {}",
                self.config.max_seq_len
            );
        }

        let ids = Tensor::new(token_ids, device)?;
        let mut hidden = self.token_embeddings.index_select(&ids, 0)?;
        hidden = hidden.broadcast_add(&self.token_type_embedding)?;
        hidden = self.embedding_norm.forward(&hidden)?;
        hidden = hidden.unsqueeze(0)?;

        for layer in &self.layers {
            hidden = layer.forward(
                &hidden,
                &self.rope_cos,
                &self.rope_sin,
                self.config.num_heads,
                self.config.head_dim,
            )?;
        }

        // Mean pooling + L2 normalize
        let pooled = hidden.mean(1)?.squeeze(0)?;
        let norm_val: f32 = pooled.sqr()?.sum_all()?.sqrt()?.to_scalar()?;
        let normalized = if norm_val > 0.0 {
            pooled.affine(1.0 / norm_val as f64, 0.0)?
        } else {
            pooled
        };

        normalized.to_vec1::<f32>().map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Public CandleEmbeddingProvider
// ---------------------------------------------------------------------------

pub struct CandleEmbeddingProvider {
    model: NomicBertMoeModel,
    tokenizer: tokenizers::Tokenizer,
    query_prefix: String,
}

impl CandleEmbeddingProvider {
    pub fn load(model_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let model = NomicBertMoeModel::load(model_path, &device)?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;

        Ok(Self {
            model,
            tokenizer,
            query_prefix: "search_query: ".to_string(),
        })
    }
}

impl EmbeddingProvider for CandleEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let prefixed = format!("{}{}", self.query_prefix, text);
        let encoding = self
            .tokenizer
            .encode(prefixed.as_str(), true)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let token_ids: Vec<u32> = encoding.get_ids().to_vec();
        self.model.forward(&token_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn test_candle_embed_basic() {
        let base = model_dir();
        let model_path = base.join("models/nomic-embed-text-v2-moe.Q4_K_M.gguf");
        let tokenizer_path = base.join("models/tokenizer.json");
        if !model_path.exists() || !tokenizer_path.exists() {
            eprintln!("Skipping: model or tokenizer not found");
            return;
        }

        let provider = CandleEmbeddingProvider::load(&model_path, &tokenizer_path).unwrap();
        let embedding = provider.embed("How do I reset my password?").unwrap();

        assert_eq!(embedding.len(), 768);

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "L2 norm should be ~1.0, got {norm}"
        );
    }

    #[test]
    fn test_candle_embed_similarity() {
        let base = model_dir();
        let model_path = base.join("models/nomic-embed-text-v2-moe.Q4_K_M.gguf");
        let tokenizer_path = base.join("models/tokenizer.json");
        if !model_path.exists() || !tokenizer_path.exists() {
            eprintln!("Skipping: model or tokenizer not found");
            return;
        }

        let provider = CandleEmbeddingProvider::load(&model_path, &tokenizer_path).unwrap();

        let e1 = provider.embed("How do I reset my password?").unwrap();
        let e2 = provider
            .embed("I forgot my password, how can I sign in again?")
            .unwrap();
        let e3 = provider
            .embed("What is the weather like in Tokyo?")
            .unwrap();

        let sim_related: f32 = e1.iter().zip(e2.iter()).map(|(a, b)| a * b).sum();
        let sim_unrelated: f32 = e1.iter().zip(e3.iter()).map(|(a, b)| a * b).sum();

        println!("sim(password_reset, forgot_password) = {sim_related:.4}");
        println!("sim(password_reset, tokyo_weather)   = {sim_unrelated:.4}");

        assert!(
            sim_related > sim_unrelated,
            "related questions should have higher similarity"
        );
        assert!(sim_related > 0.6, "related questions should be > 0.6");
        assert!(sim_unrelated < 0.7, "unrelated questions should be < 0.7");
    }
}

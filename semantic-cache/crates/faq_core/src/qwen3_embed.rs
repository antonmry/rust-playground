use anyhow::{bail, Result};
use candle_core::{DType, Device, IndexOp, Module, Tensor};
use candle_nn::{Embedding, Linear, RmsNorm, VarBuilder};
use std::path::Path;

use crate::embed::EmbeddingProvider;

// ---------------------------------------------------------------------------
// Config (hardcoded for pplx-embed-v1-0.6b)
// ---------------------------------------------------------------------------

struct Qwen3Config {
    hidden_size: usize,
    intermediate_size: usize,
    num_attention_heads: usize,
    num_key_value_heads: usize,
    head_dim: usize,
    num_hidden_layers: usize,
    vocab_size: usize,
    rms_norm_eps: f64,
    rope_theta: f32,
    max_position_embeddings: usize,
}

impl Qwen3Config {
    fn pplx_embed_v1() -> Self {
        Self {
            hidden_size: 1024,
            intermediate_size: 3072,
            num_attention_heads: 16,
            num_key_value_heads: 8,
            head_dim: 128,
            num_hidden_layers: 28,
            vocab_size: 151936,
            rms_norm_eps: 1e-6,
            rope_theta: 1_000_000.0,
            max_position_embeddings: 32768,
        }
    }
}

// ---------------------------------------------------------------------------
// SwiGLU MLP
// ---------------------------------------------------------------------------

struct Qwen3Mlp {
    gate_proj: Linear,
    up_proj: Linear,
    down_proj: Linear,
}

impl Qwen3Mlp {
    fn load(vb: VarBuilder, config: &Qwen3Config) -> Result<Self> {
        let gate_proj = candle_nn::linear_no_bias(
            config.hidden_size,
            config.intermediate_size,
            vb.pp("gate_proj"),
        )?;
        let up_proj = candle_nn::linear_no_bias(
            config.hidden_size,
            config.intermediate_size,
            vb.pp("up_proj"),
        )?;
        let down_proj = candle_nn::linear_no_bias(
            config.intermediate_size,
            config.hidden_size,
            vb.pp("down_proj"),
        )?;
        Ok(Self {
            gate_proj,
            up_proj,
            down_proj,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let gate = candle_nn::ops::silu(&self.gate_proj.forward(x)?)?;
        let up = self.up_proj.forward(x)?;
        self.down_proj.forward(&(gate * up)?).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// GQA Attention with per-head Q/K RMSNorm
// ---------------------------------------------------------------------------

struct Qwen3Attention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    o_proj: Linear,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    num_heads: usize,
    num_kv_heads: usize,
    head_dim: usize,
}

impl Qwen3Attention {
    fn load(vb: VarBuilder, config: &Qwen3Config) -> Result<Self> {
        let q_dim = config.num_attention_heads * config.head_dim;
        let kv_dim = config.num_key_value_heads * config.head_dim;

        let q_proj = candle_nn::linear_no_bias(config.hidden_size, q_dim, vb.pp("q_proj"))?;
        let k_proj = candle_nn::linear_no_bias(config.hidden_size, kv_dim, vb.pp("k_proj"))?;
        let v_proj = candle_nn::linear_no_bias(config.hidden_size, kv_dim, vb.pp("v_proj"))?;
        let o_proj = candle_nn::linear_no_bias(q_dim, config.hidden_size, vb.pp("o_proj"))?;

        let q_norm = candle_nn::rms_norm(config.head_dim, config.rms_norm_eps, vb.pp("q_norm"))?;
        let k_norm = candle_nn::rms_norm(config.head_dim, config.rms_norm_eps, vb.pp("k_norm"))?;

        Ok(Self {
            q_proj,
            k_proj,
            v_proj,
            o_proj,
            q_norm,
            k_norm,
            num_heads: config.num_attention_heads,
            num_kv_heads: config.num_key_value_heads,
            head_dim: config.head_dim,
        })
    }

    fn forward(&self, x: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
        let (batch, seq_len, _) = x.dims3()?;

        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;

        // Reshape: (batch, seq, num_heads, head_dim) -> (batch, num_heads, seq, head_dim)
        let q = q
            .reshape((batch, seq_len, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        let k = k
            .reshape((batch, seq_len, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;
        let v = v
            .reshape((batch, seq_len, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;

        // Per-head Q/K RMSNorm before RoPE
        let q = self.q_norm.forward(&q)?;
        let k = self.k_norm.forward(&k)?;

        // RoPE
        let q = apply_rope(&q, cos, sin)?;
        let k = apply_rope(&k, cos, sin)?;

        // GQA: repeat K,V heads to match Q heads
        let n_rep = self.num_heads / self.num_kv_heads;
        let k = repeat_kv(k, n_rep)?;
        let v = repeat_kv(v, n_rep)?;

        // Bidirectional attention (no causal mask)
        let scale = (self.head_dim as f64).sqrt();
        let attn_weights = q.matmul(&k.t()?)?.affine(1.0 / scale, 0.0)?;
        let attn_weights = candle_nn::ops::softmax(&attn_weights, candle_core::D::Minus1)?;
        let attn_out = attn_weights.matmul(&v)?;

        let attn_out = attn_out.transpose(1, 2)?.contiguous()?.reshape((
            batch,
            seq_len,
            self.num_heads * self.head_dim,
        ))?;

        self.o_proj.forward(&attn_out).map_err(Into::into)
    }
}

fn repeat_kv(x: Tensor, n_rep: usize) -> Result<Tensor> {
    if n_rep == 1 {
        return Ok(x);
    }
    let (batch, num_kv_heads, seq_len, head_dim) = x.dims4()?;
    x.unsqueeze(2)?
        .expand((batch, num_kv_heads, n_rep, seq_len, head_dim))?
        .reshape((batch, num_kv_heads * n_rep, seq_len, head_dim))
        .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Transformer layer (pre-norm)
// ---------------------------------------------------------------------------

struct Qwen3Layer {
    input_layernorm: RmsNorm,
    self_attn: Qwen3Attention,
    post_attention_layernorm: RmsNorm,
    mlp: Qwen3Mlp,
}

impl Qwen3Layer {
    fn load(vb: VarBuilder, config: &Qwen3Config) -> Result<Self> {
        let input_layernorm = candle_nn::rms_norm(
            config.hidden_size,
            config.rms_norm_eps,
            vb.pp("input_layernorm"),
        )?;
        let self_attn = Qwen3Attention::load(vb.pp("self_attn"), config)?;
        let post_attention_layernorm = candle_nn::rms_norm(
            config.hidden_size,
            config.rms_norm_eps,
            vb.pp("post_attention_layernorm"),
        )?;
        let mlp = Qwen3Mlp::load(vb.pp("mlp"), config)?;
        Ok(Self {
            input_layernorm,
            self_attn,
            post_attention_layernorm,
            mlp,
        })
    }

    fn forward(&self, x: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
        // Pre-norm attention + residual
        let residual = x.clone();
        let hidden = self.input_layernorm.forward(x)?;
        let hidden = self.self_attn.forward(&hidden, cos, sin)?;
        let x = (residual + hidden)?;

        // Pre-norm MLP + residual
        let residual = x.clone();
        let hidden = self.post_attention_layernorm.forward(&x)?;
        let hidden = self.mlp.forward(&hidden)?;
        (residual + hidden).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// RoPE (NeoX half-split variant, same as candle_embed.rs, theta=1M)
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

struct Qwen3EmbeddingModel {
    embed_tokens: Embedding,
    layers: Vec<Qwen3Layer>,
    norm: RmsNorm,
    rope_cos: Tensor,
    rope_sin: Tensor,
    config: Qwen3Config,
}

impl Qwen3EmbeddingModel {
    fn load(path: &Path, device: &Device) -> Result<Self> {
        let config = Qwen3Config::pplx_embed_v1();

        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[path], DType::F32, device)? };

        let embed_tokens =
            candle_nn::embedding(config.vocab_size, config.hidden_size, vb.pp("embed_tokens"))?;

        let mut layers = Vec::with_capacity(config.num_hidden_layers);
        for i in 0..config.num_hidden_layers {
            layers.push(Qwen3Layer::load(vb.pp(format!("layers.{i}")), &config)?);
        }

        let norm = candle_nn::rms_norm(config.hidden_size, config.rms_norm_eps, vb.pp("norm"))?;

        let (rope_cos, rope_sin) = precompute_rope(
            config.head_dim,
            config.max_position_embeddings,
            config.rope_theta,
            device,
        )?;

        Ok(Self {
            embed_tokens,
            layers,
            norm,
            rope_cos,
            rope_sin,
            config,
        })
    }

    fn forward(&self, token_ids: &[u32]) -> Result<Vec<f32>> {
        let device = self.rope_cos.device();
        let seq_len = token_ids.len();

        if seq_len > self.config.max_position_embeddings {
            bail!(
                "input length {seq_len} exceeds max {}",
                self.config.max_position_embeddings
            );
        }

        let ids = Tensor::new(token_ids, device)?;
        let mut hidden = self.embed_tokens.forward(&ids)?;
        hidden = hidden.unsqueeze(0)?;

        for layer in &self.layers {
            hidden = layer.forward(&hidden, &self.rope_cos, &self.rope_sin)?;
        }

        hidden = self.norm.forward(&hidden)?;

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
// Public Qwen3EmbeddingProvider
// ---------------------------------------------------------------------------

pub struct Qwen3EmbeddingProvider {
    model: Qwen3EmbeddingModel,
    tokenizer: tokenizers::Tokenizer,
}

impl Qwen3EmbeddingProvider {
    pub fn load(model_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let model = Qwen3EmbeddingModel::load(model_path, &device)?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;

        Ok(Self { model, tokenizer })
    }
}

impl EmbeddingProvider for Qwen3EmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
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
    fn test_qwen3_embed_basic() {
        let base = model_dir();
        let model_path = base.join("models/pplx-embed-v1-0.6b.safetensors");
        let tokenizer_path = base.join("models/pplx-embed-v1-0.6b-tokenizer.json");
        if !model_path.exists() || !tokenizer_path.exists() {
            eprintln!("Skipping: pplx-embed model or tokenizer not found");
            return;
        }

        let provider = Qwen3EmbeddingProvider::load(&model_path, &tokenizer_path).unwrap();
        let embedding = provider.embed("How do I reset my password?").unwrap();

        assert_eq!(embedding.len(), 1024);

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "L2 norm should be ~1.0, got {norm}"
        );
    }

    #[test]
    fn test_qwen3_embed_similarity() {
        let base = model_dir();
        let model_path = base.join("models/pplx-embed-v1-0.6b.safetensors");
        let tokenizer_path = base.join("models/pplx-embed-v1-0.6b-tokenizer.json");
        if !model_path.exists() || !tokenizer_path.exists() {
            eprintln!("Skipping: pplx-embed model or tokenizer not found");
            return;
        }

        let provider = Qwen3EmbeddingProvider::load(&model_path, &tokenizer_path).unwrap();

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

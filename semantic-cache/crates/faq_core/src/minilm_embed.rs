use anyhow::{bail, Result};
use candle_core::{DType, Device, Module, Tensor};
use candle_nn::{Linear, VarBuilder};
use std::path::Path;

use crate::embed::EmbeddingProvider;

// ---------------------------------------------------------------------------
// Config (hardcoded for all-MiniLM-L6-v2)
// ---------------------------------------------------------------------------

struct MiniLmConfig {
    hidden_size: usize,
    intermediate_size: usize,
    num_attention_heads: usize,
    head_dim: usize,
    num_hidden_layers: usize,
    vocab_size: usize,
    max_position_embeddings: usize,
    type_vocab_size: usize,
    layer_norm_eps: f64,
}

impl MiniLmConfig {
    fn all_minilm_l6_v2() -> Self {
        Self {
            hidden_size: 384,
            intermediate_size: 1536,
            num_attention_heads: 12,
            head_dim: 32,
            num_hidden_layers: 6,
            vocab_size: 30522,
            max_position_embeddings: 512,
            type_vocab_size: 2,
            layer_norm_eps: 1e-12,
        }
    }
}

// ---------------------------------------------------------------------------
// Layer norm (with bias)
// ---------------------------------------------------------------------------

struct LayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f64,
}

impl LayerNorm {
    fn load(vb: VarBuilder, hidden_size: usize, eps: f64) -> Result<Self> {
        let weight = vb.get(hidden_size, "weight")?;
        let bias = vb.get(hidden_size, "bias")?;
        Ok(Self { weight, bias, eps })
    }

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
// Self-attention (separate Q/K/V with biases, no RoPE)
// ---------------------------------------------------------------------------

struct BertSelfAttention {
    query: Linear,
    key: Linear,
    value: Linear,
    output: Linear,
    output_norm: LayerNorm,
    num_heads: usize,
    head_dim: usize,
}

impl BertSelfAttention {
    fn load(vb: VarBuilder, config: &MiniLmConfig) -> Result<Self> {
        let h = config.hidden_size;
        let attn_vb = vb.pp("attention");

        let query = candle_nn::linear(h, h, attn_vb.pp("self").pp("query"))?;
        let key = candle_nn::linear(h, h, attn_vb.pp("self").pp("key"))?;
        let value = candle_nn::linear(h, h, attn_vb.pp("self").pp("value"))?;
        let output = candle_nn::linear(h, h, attn_vb.pp("output").pp("dense"))?;
        let output_norm = LayerNorm::load(
            attn_vb.pp("output").pp("LayerNorm"),
            h,
            config.layer_norm_eps,
        )?;

        Ok(Self {
            query,
            key,
            value,
            output,
            output_norm,
            num_heads: config.num_attention_heads,
            head_dim: config.head_dim,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let (batch, seq_len, _) = x.dims3()?;

        let q = self
            .query
            .forward(x)?
            .reshape((batch, seq_len, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        let k = self
            .key
            .forward(x)?
            .reshape((batch, seq_len, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        let v = self
            .value
            .forward(x)?
            .reshape((batch, seq_len, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;

        let scale = (self.head_dim as f64).sqrt();
        let attn_weights = q.matmul(&k.t()?)?.affine(1.0 / scale, 0.0)?;
        let attn_weights = candle_nn::ops::softmax(&attn_weights, candle_core::D::Minus1)?;
        let attn_out = attn_weights.matmul(&v)?;

        let attn_out = attn_out.transpose(1, 2)?.contiguous()?.reshape((
            batch,
            seq_len,
            self.num_heads * self.head_dim,
        ))?;

        let attn_out = self.output.forward(&attn_out)?;

        // Residual + post-norm
        let x = (x + attn_out)?;
        self.output_norm.forward(&x)
    }
}

// ---------------------------------------------------------------------------
// FFN (up + GELU + down, with biases) + post-norm
// ---------------------------------------------------------------------------

struct BertFfn {
    up: Linear,
    down: Linear,
    output_norm: LayerNorm,
}

impl BertFfn {
    fn load(vb: VarBuilder, config: &MiniLmConfig) -> Result<Self> {
        let up = candle_nn::linear(
            config.hidden_size,
            config.intermediate_size,
            vb.pp("intermediate").pp("dense"),
        )?;
        let down = candle_nn::linear(
            config.intermediate_size,
            config.hidden_size,
            vb.pp("output").pp("dense"),
        )?;
        let output_norm = LayerNorm::load(
            vb.pp("output").pp("LayerNorm"),
            config.hidden_size,
            config.layer_norm_eps,
        )?;
        Ok(Self {
            up,
            down,
            output_norm,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let h = self.up.forward(x)?.gelu_erf()?;
        let h = self.down.forward(&h)?;

        // Residual + post-norm
        let x = (x + h)?;
        self.output_norm.forward(&x)
    }
}

// ---------------------------------------------------------------------------
// Transformer layer
// ---------------------------------------------------------------------------

struct BertLayer {
    attention: BertSelfAttention,
    ffn: BertFfn,
}

impl BertLayer {
    fn load(vb: VarBuilder, config: &MiniLmConfig) -> Result<Self> {
        let attention = BertSelfAttention::load(vb.clone(), config)?;
        let ffn = BertFfn::load(vb, config)?;
        Ok(Self { attention, ffn })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x = self.attention.forward(x)?;
        self.ffn.forward(&x)
    }
}

// ---------------------------------------------------------------------------
// Full model
// ---------------------------------------------------------------------------

struct MiniLmModel {
    word_embeddings: Tensor,
    position_embeddings: Tensor,
    token_type_embeddings: Tensor,
    embedding_norm: LayerNorm,
    layers: Vec<BertLayer>,
    config: MiniLmConfig,
}

impl MiniLmModel {
    fn load(path: &Path, device: &Device) -> Result<Self> {
        let config = MiniLmConfig::all_minilm_l6_v2();

        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[path], DType::F32, device)? };

        let emb_vb = vb.pp("embeddings");
        let word_embeddings = emb_vb
            .pp("word_embeddings")
            .get((config.vocab_size, config.hidden_size), "weight")?;
        let position_embeddings = emb_vb.pp("position_embeddings").get(
            (config.max_position_embeddings, config.hidden_size),
            "weight",
        )?;
        let token_type_embeddings = emb_vb
            .pp("token_type_embeddings")
            .get((config.type_vocab_size, config.hidden_size), "weight")?;
        let embedding_norm = LayerNorm::load(
            emb_vb.pp("LayerNorm"),
            config.hidden_size,
            config.layer_norm_eps,
        )?;

        let mut layers = Vec::with_capacity(config.num_hidden_layers);
        for i in 0..config.num_hidden_layers {
            layers.push(BertLayer::load(
                vb.pp("encoder").pp("layer").pp(i.to_string()),
                &config,
            )?);
        }

        Ok(Self {
            word_embeddings,
            position_embeddings,
            token_type_embeddings,
            embedding_norm,
            layers,
            config,
        })
    }

    fn forward(&self, token_ids: &[u32]) -> Result<Vec<f32>> {
        let device = self.word_embeddings.device();
        let seq_len = token_ids.len();

        if seq_len > self.config.max_position_embeddings {
            bail!(
                "input length {seq_len} exceeds max {}",
                self.config.max_position_embeddings
            );
        }

        let ids = Tensor::new(token_ids, device)?;
        let word_emb = self.word_embeddings.index_select(&ids, 0)?;

        let position_ids: Vec<u32> = (0..seq_len as u32).collect();
        let position_ids = Tensor::new(position_ids.as_slice(), device)?;
        let pos_emb = self.position_embeddings.index_select(&position_ids, 0)?;

        let token_type_ids = Tensor::zeros(seq_len, DType::U32, device)?;
        let type_emb = self
            .token_type_embeddings
            .index_select(&token_type_ids, 0)?;

        let mut hidden = ((word_emb + pos_emb)? + type_emb)?;
        hidden = self.embedding_norm.forward(&hidden)?;
        hidden = hidden.unsqueeze(0)?;

        for layer in &self.layers {
            hidden = layer.forward(&hidden)?;
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
// Public MiniLmEmbeddingProvider
// ---------------------------------------------------------------------------

pub struct MiniLmEmbeddingProvider {
    model: MiniLmModel,
    tokenizer: tokenizers::Tokenizer,
}

impl MiniLmEmbeddingProvider {
    pub fn load(model_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let model = MiniLmModel::load(model_path, &device)?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;

        Ok(Self { model, tokenizer })
    }
}

impl EmbeddingProvider for MiniLmEmbeddingProvider {
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
    fn test_minilm_embed_basic() {
        let base = model_dir();
        let model_path = base.join("models/all-MiniLM-L6-v2.safetensors");
        let tokenizer_path = base.join("models/all-MiniLM-L6-v2-tokenizer.json");
        if !model_path.exists() || !tokenizer_path.exists() {
            eprintln!("Skipping: all-MiniLM-L6-v2 model or tokenizer not found");
            return;
        }

        let provider = MiniLmEmbeddingProvider::load(&model_path, &tokenizer_path).unwrap();
        let embedding = provider.embed("How do I reset my password?").unwrap();

        assert_eq!(embedding.len(), 384);

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "L2 norm should be ~1.0, got {norm}"
        );
    }

    #[test]
    fn test_minilm_embed_similarity() {
        let base = model_dir();
        let model_path = base.join("models/all-MiniLM-L6-v2.safetensors");
        let tokenizer_path = base.join("models/all-MiniLM-L6-v2-tokenizer.json");
        if !model_path.exists() || !tokenizer_path.exists() {
            eprintln!("Skipping: all-MiniLM-L6-v2 model or tokenizer not found");
            return;
        }

        let provider = MiniLmEmbeddingProvider::load(&model_path, &tokenizer_path).unwrap();

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

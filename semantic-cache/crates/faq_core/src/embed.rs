use anyhow::Result;

pub trait EmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

impl EmbeddingProvider for Box<dyn EmbeddingProvider> {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        (**self).embed(text)
    }
}

#[derive(Debug, Clone)]
pub struct HashEmbeddingProvider {
    dim: usize,
}

impl HashEmbeddingProvider {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(8) }
    }
}

impl Default for HashEmbeddingProvider {
    fn default() -> Self {
        Self { dim: 768 }
    }
}

impl EmbeddingProvider for HashEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut v = vec![0.0f32; self.dim];

        for token in text
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let mut h: u64 = 1469598103934665603;
            for b in token.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            let idx = (h as usize) % self.dim;
            v[idx] += 1.0;
        }

        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }

        Ok(v)
    }
}

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::memory::types::EmbeddingProvider;

/// OpenAI text embedding provider.
/// Uses the existing reqwest client to call the OpenAI Embeddings API.
pub struct OpenAIEmbedder {
    client: reqwest::Client,
    api_key: String,
    model: String,
    dimensions: usize,
}

impl OpenAIEmbedder {
    pub fn new(client: reqwest::Client, api_key: String, model: &str) -> Self {
        let dimensions = match model {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        };
        Self {
            client,
            api_key,
            model: model.to_string(),
            dimensions,
        }
    }

    /// Truncate text to max chars (matching agentmemory's EMBED_MAX_CHARS = 16_000).
    fn clip(text: &str) -> &str {
        const MAX: usize = 16_000;
        if text.len() <= MAX { text } else { &text[..MAX] }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbedder {
    fn name(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let clipped = Self::clip(text);
        let body = serde_json::json!({
            "model": self.model,
            "input": clipped,
        });

        let resp = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("OpenAI embeddings request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI embeddings returned {}: {}", status, text);
        }

        let data: serde_json::Value = resp.json().await?;
        let embedding = data["data"][0]["embedding"]
            .as_array()
            .context("invalid embeddings response: missing data[0].embedding")?;

        let vec: Vec<f32> = embedding
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        if vec.len() != self.dimensions {
            anyhow::bail!(
                "embedding dimension mismatch: expected {}, got {}",
                self.dimensions,
                vec.len()
            );
        }

        Ok(vec)
    }
}

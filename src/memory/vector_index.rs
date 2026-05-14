use std::collections::HashMap;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// In-memory vector index with cosine similarity search.
/// Replicates agentmemory's `VectorIndex`.
#[derive(Debug)]
pub struct VectorIndex {
    vectors: HashMap<String, VectorEntry>,
    dimensions: usize,
}

#[derive(Debug)]
struct VectorEntry {
    embedding: Vec<f32>,
    session_id: String,
}

impl VectorIndex {
    pub fn new(dimensions: usize) -> Self {
        Self {
            vectors: HashMap::new(),
            dimensions,
        }
    }

    /// Add a vector to the index. Returns error on dimension mismatch.
    pub fn add(&mut self, id: &str, session_id: &str, embedding: Vec<f32>) -> anyhow::Result<()> {
        if embedding.len() != self.dimensions {
            anyhow::bail!(
                "Embedding dimension mismatch: expected {}, got {}",
                self.dimensions,
                embedding.len()
            );
        }
        self.vectors.insert(
            id.to_string(),
            VectorEntry {
                embedding,
                session_id: session_id.to_string(),
            },
        );
        Ok(())
    }

    /// Remove a vector from the index.
    pub fn remove(&mut self, id: &str) {
        self.vectors.remove(id);
    }

    /// Search for top-K similar vectors by cosine similarity.
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(String, f64)> {
        if self.vectors.is_empty() || query.len() != self.dimensions {
            return vec![];
        }

        // Use binary heap to keep top-K
        let mut heap: BinaryHeap<Reverse<(i64, String)>> = BinaryHeap::new();

        for (id, entry) in &self.vectors {
            let sim = cosine_similarity(query, &entry.embedding);
            // Use negative score for max-heap via Reverse
            let score_int = (sim * 1_000_000.0) as i64;
            heap.push(Reverse((score_int, id.clone())));
            if heap.len() > limit {
                heap.pop();
            }
        }

        let mut results: Vec<(String, f64)> = heap
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse((s, id))| (id, s as f64 / 1_000_000.0))
            .rev()
            .collect();

        results.truncate(limit);
        results
    }

    /// Number of vectors in the index.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Clear all vectors for a session.
    pub fn clear_session(&mut self, session_id: &str) {
        self.vectors.retain(|_, v| v.session_id != session_id);
    }
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum();
    let norm_b: f32 = b.iter().map(|x| x * x).sum();
    let denom = (norm_a * norm_b).sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom) as f64
    }
}

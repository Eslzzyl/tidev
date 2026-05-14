use std::collections::HashMap;

use crate::memory::types::HybridSearchResult;
use crate::memory::search_index::Bm25Index;
use crate::memory::vector_index::VectorIndex;

/// Hybrid search using RRF (Reciprocal Rank Fusion) to merge
/// BM25 and vector search results.
/// Replicates agentmemory's `HybridSearch` with RRF.
#[derive(Debug)]
pub struct HybridSearch {
    bm25_weight: f64,
    vector_weight: f64,
    rrf_k: f64,
}

impl HybridSearch {
    pub fn new() -> Self {
        Self {
            bm25_weight: 0.4,
            vector_weight: 0.6,
            rrf_k: 60.0,
        }
    }

    /// Run hybrid search with BM25 + vector embedding.
    /// `bm25` and `vector` are the indexes; `query_embedding` is optional
    /// (pass None for BM25-only search).
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        bm25: &Bm25Index,
        vector: &VectorIndex,
        query_embedding: Option<&[f32]>,
    ) -> Vec<HybridSearchResult> {
        // 1. BM25 search (fetch 2x limit for fusion)
        let bm25_results = bm25.search(query, limit * 2);

        // 2. Vector search (fetch 2x limit for fusion)
        let vector_results = query_embedding
            .map(|emb| vector.search(emb, limit * 2))
            .unwrap_or_default();

        // 3. RRF fusion
        let mut scores: HashMap<String, HybridScore> = HashMap::new();

        for (i, (id, score)) in bm25_results.iter().enumerate() {
            let rrf = 1.0 / (self.rrf_k + i as f64);
            let entry = scores.entry(id.clone()).or_default();
            entry.combined += rrf * self.bm25_weight;
            entry.bm25 = Some(*score);
        }

        for (i, (id, score)) in vector_results.iter().enumerate() {
            let rrf = 1.0 / (self.rrf_k + i as f64);
            let entry = scores.entry(id.clone()).or_default();
            entry.combined += rrf * self.vector_weight;
            entry.vector = Some(*score);
        }

        // 4. Sort by combined score, take top-K
        let mut results: Vec<HybridSearchResult> = scores
            .into_iter()
            .map(|(id, s)| HybridSearchResult {
                id,
                combined_score: s.combined,
                bm25_score: s.bm25,
                vector_score: s.vector,
            })
            .collect();

        results.sort_by(|a, b| {
            b.combined_score
                .partial_cmp(&a.combined_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }
}

impl Default for HybridSearch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct HybridScore {
    combined: f64,
    bm25: Option<f64>,
    vector: Option<f64>,
}

use lru::LruCache;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

const DEDUP_TTL: Duration = Duration::from_secs(300); // 5 minutes
const DEDUP_CAPACITY: usize = 10_000;
const INPUT_TRUNCATE: usize = 500;

/// SHA256-based dedup map with 5-minute TTL.
/// Exact replica of agentmemory's `DedupMap`.
#[derive(Debug)]
pub struct DedupMap {
    entries: LruCache<String, Instant>,
}

impl DedupMap {
    pub fn new() -> Self {
        Self {
            entries: LruCache::new(NonZeroUsize::new(DEDUP_CAPACITY).unwrap()),
        }
    }

    /// Compute dedup hash from session + tool name + input.
    pub fn compute_hash(&self, session_id: &str, tool_name: &str, input: &str) -> String {
        let input = if input.len() > INPUT_TRUNCATE {
            &input[..INPUT_TRUNCATE]
        } else {
            input
        };
        let raw = format!("{}:{}:{}", session_id, tool_name, input);
        let hash_bytes = blake3::hash(raw.as_bytes());
        hex::encode(&hash_bytes.as_bytes()[..16])
    }

    /// Check if hash is a duplicate (not expired).
    pub fn is_duplicate(&mut self, hash: &str) -> bool {
        let hash = hash.to_string();
        if let Some(&expires) = self.entries.peek(&hash) {
            if expires > Instant::now() {
                return true;
            }
            self.entries.pop(&hash);
        }
        false
    }

    /// Record a hash with TTL.
    pub fn record(&mut self, hash: String) {
        self.entries.put(hash, Instant::now() + DEDUP_TTL);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for DedupMap {
    fn default() -> Self {
        Self::new()
    }
}

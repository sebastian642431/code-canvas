//! Per-page cache of tokenized chunks.
//!
//! Tokens are not kept for the whole repository (that would be ~12 bytes per
//! token, gigabytes for big repos). Instead, chunks of
//! [`CHUNK_LINES`](crate::syntax::CHUNK_LINES) lines are tokenized when they
//! become visible and evicted when they have not been used for a while.

use std::collections::HashMap;

use crate::scan::SourceFile;
use crate::syntax::Tokens;

struct Entry {
    tokens: Tokens,
    last_used: u64,
}

pub struct TokenCache {
    map: HashMap<(u32, u32), Entry>,
    frame: u64,
    /// Soft limit; above it, chunks unused for a few frames are evicted.
    pub max_chunks: usize,
    pub tokenized_this_frame: usize,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self::new(16_384)
    }
}

impl TokenCache {
    pub fn new(max_chunks: usize) -> Self {
        Self { map: HashMap::new(), frame: 0, max_chunks, tokenized_this_frame: 0 }
    }

    /// Call once per frame before using [`TokenCache::chunk`].
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.tokenized_this_frame = 0;
        if self.map.len() > self.max_chunks {
            // Evict least recently used chunks down to 3/4 of the limit.
            let mut ages: Vec<u64> = self.map.values().map(|e| e.last_used).collect();
            ages.sort_unstable();
            let cut = ages[self.map.len() - self.max_chunks * 3 / 4];
            let frame = self.frame;
            self.map.retain(|_, e| e.last_used >= cut || e.last_used + 2 >= frame);
        }
    }

    /// Tokens of chunk `chunk` of file `file_index`, tokenizing it if needed.
    pub fn chunk(&mut self, file_index: usize, file: &SourceFile, chunk: usize) -> &Tokens {
        let frame = self.frame;
        let mut made = false;
        let e = self.map.entry((file_index as u32, chunk as u32)).or_insert_with(|| {
            made = true;
            Entry { tokens: file.tokenize_chunk(chunk), last_used: frame }
        });
        e.last_used = frame;
        if made {
            self.tokenized_this_frame += 1;
        }
        &e.tokens
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

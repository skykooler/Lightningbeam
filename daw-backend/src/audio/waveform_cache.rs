//! Waveform chunk cache for scalable multi-resolution waveform generation
//!
//! This module provides a chunk-based waveform caching system that generates
//! waveform data progressively at multiple detail levels, avoiding the limitations
//! of the old fixed 20,000-peak approach.

use crate::io::{WaveformChunk, WaveformChunkKey, WaveformPeak};
use crate::audio::pool::AudioFile;
use std::collections::HashMap;

/// Detail levels for multi-resolution waveform storage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailLevel {
    Overview = 0,   // 1 peak per second
    Low = 1,        // 10 peaks per second
    Medium = 2,     // 100 peaks per second
    High = 3,       // 1000 peaks per second
    Max = 4,        // Full resolution (sample-accurate)
}

impl DetailLevel {
    /// Get peaks per second for this detail level
    pub fn peaks_per_second(self) -> usize {
        match self {
            DetailLevel::Overview => 1,
            DetailLevel::Low => 10,
            DetailLevel::Medium => 100,
            DetailLevel::High => 1000,
            DetailLevel::Max => 48000, // Approximate max for sample-accurate
        }
    }

    /// Create from u8 value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(DetailLevel::Overview),
            1 => Some(DetailLevel::Low),
            2 => Some(DetailLevel::Medium),
            3 => Some(DetailLevel::High),
            4 => Some(DetailLevel::Max),
            _ => None,
        }
    }
}

/// Priority for chunk generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChunkPriority {
    Low = 0,      // Background generation
    Medium = 1,   // Precache adjacent to viewport
    High = 2,     // Visible in current viewport
}

/// Chunk generation request
#[derive(Debug, Clone)]
pub struct ChunkGenerationRequest {
    pub key: WaveformChunkKey,
    pub priority: ChunkPriority,
}

/// Waveform chunk cache with multi-resolution support
pub struct WaveformCache {
    /// Cached chunks indexed by key
    chunks: HashMap<WaveformChunkKey, Vec<WaveformPeak>>,

    /// Maximum memory usage in MB (for future LRU eviction)
    max_memory_mb: usize,

    /// Current memory usage estimate in bytes
    current_memory_bytes: usize,
}

impl WaveformCache {
    /// Create a new waveform cache with the specified memory limit
    pub fn new(max_memory_mb: usize) -> Self {
        Self {
            chunks: HashMap::new(),
            max_memory_mb,
            current_memory_bytes: 0,
        }
    }

    /// Get a chunk from the cache
    pub fn get_chunk(&self, key: &WaveformChunkKey) -> Option<&Vec<WaveformPeak>> {
        self.chunks.get(key)
    }

    /// Store a chunk in the cache
    pub fn store_chunk(&mut self, key: WaveformChunkKey, peaks: Vec<WaveformPeak>) {
        let chunk_size = peaks.len() * std::mem::size_of::<WaveformPeak>();
        self.current_memory_bytes += chunk_size;
        self.chunks.insert(key, peaks);

        // TODO: Implement LRU eviction if memory exceeds limit
    }

    /// Check if a chunk exists in the cache
    pub fn has_chunk(&self, key: &WaveformChunkKey) -> bool {
        self.chunks.contains_key(key)
    }

    /// Clear all chunks for a specific pool index (when file is unloaded)
    pub fn clear_pool(&mut self, pool_index: usize) {
        self.chunks.retain(|key, peaks| {
            if key.pool_index == pool_index {
                let chunk_size = peaks.len() * std::mem::size_of::<WaveformPeak>();
                self.current_memory_bytes = self.current_memory_bytes.saturating_sub(chunk_size);
                false
            } else {
                true
            }
        });
    }

    /// Generate a single waveform chunk for an audio file
    ///
    /// This generates peaks for a specific time range at a specific detail level.
    /// The chunk covers a time range based on the detail level and chunk index.
    pub fn generate_chunk(
        audio_file: &AudioFile,
        detail_level: u8,
        chunk_index: u32,
    ) -> Option<WaveformChunk> {
        let level = DetailLevel::from_u8(detail_level)?;
        let peaks_per_second = level.peaks_per_second();

        // Calculate time range for this chunk based on detail level
        // Each chunk covers a varying amount of time depending on detail level
        let chunk_duration_seconds = match level {
            DetailLevel::Overview => 60.0,    // 60 seconds per chunk (60 peaks)
            DetailLevel::Low => 30.0,         // 30 seconds per chunk (300 peaks)
            DetailLevel::Medium => 10.0,      // 10 seconds per chunk (1000 peaks)
            DetailLevel::High => 5.0,         // 5 seconds per chunk (5000 peaks)
            DetailLevel::Max => 1.0,          // 1 second per chunk (48000 peaks)
        };

        let start_time = chunk_index as f64 * chunk_duration_seconds;
        let end_time = start_time + chunk_duration_seconds;

        // Check if this chunk is within the audio file duration
        let audio_duration = audio_file.duration_seconds();
        if start_time >= audio_duration {
            return None; // Chunk is completely beyond file end
        }

        // Clamp end_time to file duration
        let end_time = end_time.min(audio_duration);

        // Calculate frame range
        let start_frame = (start_time * audio_file.sample_rate as f64) as usize;
        let end_frame = (end_time * audio_file.sample_rate as f64) as usize;

        // Calculate number of peaks for this time range
        let duration = end_time - start_time;
        let target_peaks = (duration * peaks_per_second as f64).ceil() as usize;

        if target_peaks == 0 {
            return None;
        }

        // Generate peaks using the existing method
        let peaks = audio_file.generate_waveform_overview_range(
            start_frame,
            end_frame,
            target_peaks,
        );

        Some(WaveformChunk {
            audio_pool_index: 0, // Will be set by caller
            detail_level,
            chunk_index,
            time_range: (start_time, end_time),
            peaks,
        })
    }

    /// Generate multiple chunks for an audio file
    ///
    /// This is a convenience method for generating several chunks at once.
    pub fn generate_chunks(
        audio_file: &AudioFile,
        pool_index: usize,
        detail_level: u8,
        chunk_indices: &[u32],
    ) -> Vec<WaveformChunk> {
        chunk_indices
            .iter()
            .filter_map(|&chunk_index| {
                let mut chunk = Self::generate_chunk(audio_file, detail_level, chunk_index)?;
                chunk.audio_pool_index = pool_index;
                Some(chunk)
            })
            .collect()
    }

    /// Calculate how many chunks are needed for a file at a given detail level
    pub fn calculate_chunk_count(duration_seconds: f64, detail_level: u8) -> u32 {
        let level = match DetailLevel::from_u8(detail_level) {
            Some(l) => l,
            None => return 0,
        };

        let chunk_duration_seconds = match level {
            DetailLevel::Overview => 60.0,
            DetailLevel::Low => 30.0,
            DetailLevel::Medium => 10.0,
            DetailLevel::High => 5.0,
            DetailLevel::Max => 1.0,
        };

        ((duration_seconds / chunk_duration_seconds).ceil() as u32).max(1)
    }

    /// Generate all Level 0 (overview) chunks for a file
    ///
    /// This should be called immediately when a file is imported to provide
    /// instant thumbnail display.
    pub fn generate_overview_chunks(
        &mut self,
        audio_file: &AudioFile,
        pool_index: usize,
    ) -> Vec<WaveformChunk> {
        let duration = audio_file.duration_seconds();
        let chunk_count = Self::calculate_chunk_count(duration, 0);

        let chunk_indices: Vec<u32> = (0..chunk_count).collect();
        let chunks = Self::generate_chunks(audio_file, pool_index, 0, &chunk_indices);

        // Store chunks in cache
        for chunk in &chunks {
            let key = WaveformChunkKey {
                pool_index,
                detail_level: chunk.detail_level,
                chunk_index: chunk.chunk_index,
            };
            self.store_chunk(key, chunk.peaks.clone());
        }

        chunks
    }

    /// Get current memory usage in bytes
    pub fn memory_usage_bytes(&self) -> usize {
        self.current_memory_bytes
    }

    /// Get current memory usage in megabytes
    pub fn memory_usage_mb(&self) -> f64 {
        self.current_memory_bytes as f64 / 1_000_000.0
    }

    /// Get number of cached chunks
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

impl Default for WaveformCache {
    fn default() -> Self {
        Self::new(100) // Default 100MB cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detail_level_peaks_per_second() {
        assert_eq!(DetailLevel::Overview.peaks_per_second(), 1);
        assert_eq!(DetailLevel::Low.peaks_per_second(), 10);
        assert_eq!(DetailLevel::Medium.peaks_per_second(), 100);
        assert_eq!(DetailLevel::High.peaks_per_second(), 1000);
    }

    #[test]
    fn test_chunk_count_calculation() {
        // 60 second file, Overview level (60s chunks) = 1 chunk
        assert_eq!(WaveformCache::calculate_chunk_count(60.0, 0), 1);

        // 120 second file, Overview level (60s chunks) = 2 chunks
        assert_eq!(WaveformCache::calculate_chunk_count(120.0, 0), 2);

        // 10 second file, Medium level (10s chunks) = 1 chunk
        assert_eq!(WaveformCache::calculate_chunk_count(10.0, 2), 1);

        // 25 second file, Medium level (10s chunks) = 3 chunks
        assert_eq!(WaveformCache::calculate_chunk_count(25.0, 2), 3);
    }
}

use eframe::egui;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Tile width is constant at 1024 pixels per tile
pub const TILE_WIDTH_PIXELS: usize = 1024;

/// Unique identifier for a cached waveform image tile
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaveformCacheKey {
    /// Audio pool index from backend
    pub audio_pool_index: usize,
    /// Zoom bucket (power of 2: 1, 2, 4, 8, 16, etc.)
    pub zoom_bucket: u32,
    /// Tile index (which tile in the sequence for this audio clip)
    pub tile_index: u32,
    /// Clip height in pixels (for cache invalidation on resize)
    pub height: u32,
}

/// Cached waveform image with metadata
pub struct CachedWaveform {
    /// The rendered texture handle
    pub texture: egui::TextureHandle,
    /// Size in bytes (for memory tracking)
    pub size_bytes: usize,
    /// Last access time (for LRU eviction)
    pub last_accessed: Instant,
    /// Width of the image in pixels
    pub width_pixels: u32,
    /// Height of the image in pixels
    pub height_pixels: u32,
}

/// Main cache structure
pub struct WaveformImageCache {
    /// Map from cache key to rendered texture
    cache: HashMap<WaveformCacheKey, CachedWaveform>,
    /// LRU queue (most recent at back)
    lru_queue: VecDeque<WaveformCacheKey>,
    /// Current total memory usage in bytes
    total_bytes: usize,
    /// Maximum memory usage (100 MB default)
    max_bytes: usize,
    /// Statistics
    hits: u64,
    misses: u64,
}

impl WaveformImageCache {
    /// Create a new waveform image cache with 100 MB limit
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            lru_queue: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 100 * 1024 * 1024, // 100 MB
            hits: 0,
            misses: 0,
        }
    }

    /// Clear all cached textures
    pub fn clear(&mut self) {
        self.cache.clear();
        self.lru_queue.clear();
        self.total_bytes = 0;
        // Note: hits/misses preserved for debugging
    }

    /// Get cache statistics: (hits, misses, total_bytes, num_entries)
    pub fn stats(&self) -> (u64, u64, usize, usize) {
        (self.hits, self.misses, self.total_bytes, self.cache.len())
    }

    /// Evict least recently used entries until under memory limit
    fn evict_lru(&mut self) {
        while self.total_bytes > self.max_bytes && !self.lru_queue.is_empty() {
            if let Some(key) = self.lru_queue.pop_front() {
                if let Some(cached) = self.cache.remove(&key) {
                    self.total_bytes -= cached.size_bytes;
                    // Texture automatically freed when CachedWaveform dropped
                }
            }
        }
    }

    /// Update LRU queue when a key is accessed
    fn touch(&mut self, key: WaveformCacheKey) {
        // Remove key from its current position in LRU queue
        self.lru_queue.retain(|&k| k != key);
        // Add to back (most recent)
        self.lru_queue.push_back(key);
    }

    /// Get cached texture or generate new one
    pub fn get_or_create(
        &mut self,
        key: WaveformCacheKey,
        ctx: &egui::Context,
        waveform: &[daw_backend::WaveformPeak],
        audio_file_duration: f64,
        trim_start: f64,
    ) -> egui::TextureHandle {
        // Check if already cached
        let texture = if let Some(cached) = self.cache.get_mut(&key) {
            // Cache hit
            self.hits += 1;
            cached.last_accessed = Instant::now();
            Some(cached.texture.clone())
        } else {
            None
        };

        if let Some(texture) = texture {
            self.touch(key);
            return texture;
        }

        // Cache miss - generate new tile
        self.misses += 1;

        // Render waveform to image
        let color_image = render_waveform_to_image(
            waveform,
            key.tile_index,
            audio_file_duration,
            key.zoom_bucket,
            key.height,
            trim_start,
        );

        // Upload to GPU as texture
        let texture_name = format!(
            "waveform_{}_{}_{}",
            key.audio_pool_index, key.zoom_bucket, key.tile_index
        );
        let texture = ctx.load_texture(
            texture_name,
            color_image,
            egui::TextureOptions::LINEAR,
        );

        // Calculate memory usage
        let size_bytes = TILE_WIDTH_PIXELS * key.height as usize * 4;

        // Store in cache
        let cached = CachedWaveform {
            texture: texture.clone(),
            size_bytes,
            last_accessed: Instant::now(),
            width_pixels: TILE_WIDTH_PIXELS as u32,
            height_pixels: key.height,
        };

        self.total_bytes += size_bytes;
        self.cache.insert(key, cached);
        self.touch(key);

        // Evict if over limit
        self.evict_lru();

        texture
    }

    /// Pre-cache tiles for smooth scrolling
    pub fn precache_tiles(
        &mut self,
        keys: &[WaveformCacheKey],
        ctx: &egui::Context,
        waveform_peak_cache: &HashMap<usize, Vec<daw_backend::WaveformPeak>>,
        audio_file_duration: f64,
        trim_start: f64,
    ) {
        // Limit pre-caching to avoid frame time spike
        const MAX_PRECACHE_PER_FRAME: usize = 2;

        let mut precached = 0;

        for key in keys {
            if precached >= MAX_PRECACHE_PER_FRAME {
                break;
            }

            // Skip if already cached
            if self.cache.contains_key(key) {
                continue;
            }

            // Get waveform peaks
            if let Some(waveform) = waveform_peak_cache.get(&key.audio_pool_index) {
                // Generate and cache
                let _ = self.get_or_create(*key, ctx, waveform, audio_file_duration, trim_start);
                precached += 1;
            }
        }
    }

    /// Remove all entries for a specific audio file
    pub fn invalidate_audio(&mut self, audio_pool_index: usize) {
        let keys_to_remove: Vec<WaveformCacheKey> = self
            .cache
            .keys()
            .filter(|k| k.audio_pool_index == audio_pool_index)
            .copied()
            .collect();

        for key in keys_to_remove {
            if let Some(cached) = self.cache.remove(&key) {
                self.total_bytes -= cached.size_bytes;
            }
        }

        // Also clean up LRU queue
        self.lru_queue.retain(|key| key.audio_pool_index != audio_pool_index);
    }

    /// Remove all entries with a specific height (for window resize)
    pub fn invalidate_height(&mut self, old_height: u32) {
        let keys_to_remove: Vec<WaveformCacheKey> = self
            .cache
            .keys()
            .filter(|k| k.height == old_height)
            .copied()
            .collect();

        for key in keys_to_remove {
            if let Some(cached) = self.cache.remove(&key) {
                self.total_bytes -= cached.size_bytes;
            }
        }

        // Also clean up LRU queue
        self.lru_queue.retain(|key| key.height != old_height);
    }
}

impl Default for WaveformImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate zoom bucket from pixels_per_second
/// Rounds to nearest power of 2: 1, 2, 4, 8, 16, 32, 64, 128, 256
pub fn calculate_zoom_bucket(pixels_per_second: f64) -> u32 {
    if pixels_per_second <= 1.0 {
        return 1;
    }

    // Round to nearest power of 2
    let log2 = pixels_per_second.log2();
    let rounded = log2.round();
    2u32.pow(rounded as u32)
}

/// Render a waveform tile to a ColorImage
fn render_waveform_to_image(
    waveform: &[daw_backend::WaveformPeak],
    tile_index: u32,
    audio_file_duration: f64,
    zoom_bucket: u32,
    height: u32,
    trim_start: f64,
) -> egui::ColorImage {
    let width = TILE_WIDTH_PIXELS;
    let height = height as usize;

    // Create RGBA buffer (transparent background)
    let mut pixels = vec![0u8; width * height * 4];

    // Render as white - will be tinted at render time with clip background color
    let waveform_color = egui::Color32::WHITE;

    // Calculate time range for this tile
    // Each pixel represents (1.0 / zoom_bucket) seconds
    let seconds_per_pixel = 1.0 / zoom_bucket as f64;
    let tile_start_in_clip = tile_index as f64 * TILE_WIDTH_PIXELS as f64 * seconds_per_pixel;
    let tile_end_in_clip = tile_start_in_clip + width as f64 * seconds_per_pixel;

    // Add trim_start offset to get position in source audio file
    let tile_start_time = trim_start + tile_start_in_clip;
    let tile_end_time = (trim_start + tile_end_in_clip).min(audio_file_duration);

    // Calculate which waveform peaks correspond to this tile
    let peak_start_idx = ((tile_start_time / audio_file_duration) * waveform.len() as f64) as usize;
    let peak_end_idx = ((tile_end_time / audio_file_duration) * waveform.len() as f64) as usize;
    let peak_end_idx = peak_end_idx.min(waveform.len());

    if peak_start_idx >= waveform.len() {
        // Tile is beyond the end of the audio clip - return transparent image
        return egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
    }

    let tile_peaks = &waveform[peak_start_idx..peak_end_idx];
    if tile_peaks.is_empty() {
        return egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
    }

    // Calculate the actual time range this tile covers in the audio file
    // This may be less than the full tile width if the audio file is shorter than the tile's time span
    let actual_time_covered = tile_end_time - tile_start_time;
    let actual_pixel_width = (actual_time_covered / seconds_per_pixel).min(width as f64);

    // Render waveform to pixel buffer
    // Distribute peaks only across the valid pixel range, not the entire tile width
    let pixels_per_peak = actual_pixel_width / tile_peaks.len() as f64;

    for (peak_idx, peak) in tile_peaks.iter().enumerate() {
        let x_start = (peak_idx as f64 * pixels_per_peak).floor() as usize;
        let x_end = ((peak_idx + 1) as f64 * pixels_per_peak).ceil() as usize;
        let x_end = x_end.min(width);

        // Calculate Y range for this peak
        let center_y = height as f64 / 2.0;
        let max_y = (center_y + (peak.max as f64 * height as f64 * 0.45)).round() as usize;
        let min_y = (center_y + (peak.min as f64 * height as f64 * 0.45)).round() as usize;
        let min_y = min_y.min(height - 1);
        let max_y = max_y.min(height - 1);

        // Fill vertical span for this peak
        for x in x_start..x_end {
            for y in min_y..=max_y {
                let pixel_idx = (y * width + x) * 4;
                pixels[pixel_idx] = waveform_color.r();
                pixels[pixel_idx + 1] = waveform_color.g();
                pixels[pixel_idx + 2] = waveform_color.b();
                pixels[pixel_idx + 3] = waveform_color.a();
            }
        }
    }

    egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels)
}

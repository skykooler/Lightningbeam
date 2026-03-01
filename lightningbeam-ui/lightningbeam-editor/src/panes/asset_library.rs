//! Asset Library pane - browse and manage project assets
//!
//! Displays all clips in the document organized by category:
//! - Vector Clips (animations)
//! - Video Clips (imported video files)
//! - Audio Clips (sampled audio and MIDI)
//! - Image Assets (static images)

use eframe::egui;
use lightningbeam_core::clip::{AudioClipType, VectorClip};
use lightningbeam_core::document::Document;
use lightningbeam_core::layer::AnyLayer;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::{DragClipType, DraggingAsset, NodePath, PaneRenderer, SharedPaneState};
use crate::widgets::ImeTextField;

/// Derive min/max peak pairs from raw audio samples for thumbnail rendering.
/// Downsamples to `num_peaks` (min, max) pairs by scanning chunks of samples.
fn peaks_from_raw_audio(
    raw: &(std::sync::Arc<Vec<f32>>, u32, u32), // (samples, sample_rate, channels)
    num_peaks: usize,
) -> Vec<(f32, f32)> {
    let (samples, _sr, channels) = raw;
    let ch = (*channels as usize).max(1);
    let total_frames = samples.len() / ch;
    if total_frames == 0 || num_peaks == 0 {
        return vec![];
    }
    let frames_per_peak = (total_frames as f64 / num_peaks as f64).max(1.0);
    let mut peaks = Vec::with_capacity(num_peaks);
    for i in 0..num_peaks {
        let start = (i as f64 * frames_per_peak) as usize;
        let end = (((i + 1) as f64 * frames_per_peak) as usize).min(total_frames);
        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;
        for frame in start..end {
            // Mix all channels together for the thumbnail
            let mut sample = 0.0f32;
            for c in 0..ch {
                sample += samples[frame * ch + c];
            }
            sample /= ch as f32;
            min_val = min_val.min(sample);
            max_val = max_val.max(sample);
        }
        if min_val <= max_val {
            peaks.push((min_val, max_val));
        }
    }
    peaks
}

// Thumbnail constants
const THUMBNAIL_SIZE: u32 = 64;
const THUMBNAIL_PREVIEW_SECONDS: f64 = 10.0;

// Layout constants
const SEARCH_BAR_HEIGHT: f32 = 30.0;
const CATEGORY_TAB_HEIGHT: f32 = 28.0;
const BREADCRUMB_HEIGHT: f32 = 24.0;
const ITEM_HEIGHT: f32 = 40.0;
#[allow(dead_code)]
const ITEM_PADDING: f32 = 4.0;
const LIST_THUMBNAIL_SIZE: f32 = 32.0;
const GRID_ITEM_SIZE: f32 = 80.0;
const GRID_SPACING: f32 = 8.0;

/// View mode for the asset library
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetViewMode {
    #[default]
    List,
    Grid,
}

/// Cache for thumbnail textures
pub struct ThumbnailCache {
    /// Cached egui textures keyed by asset UUID
    textures: HashMap<Uuid, egui::TextureHandle>,
    /// Track which assets need regeneration
    dirty: HashSet<Uuid>,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            dirty: HashSet::new(),
        }
    }

    /// Get a cached thumbnail or create one using the provided generator
    pub fn get_or_create<F>(
        &mut self,
        ctx: &egui::Context,
        asset_id: Uuid,
        generator: F,
    ) -> Option<&egui::TextureHandle>
    where
        F: FnOnce() -> Option<Vec<u8>>,
    {
        // Check if we need to regenerate
        if self.dirty.contains(&asset_id) {
            self.textures.remove(&asset_id);
            self.dirty.remove(&asset_id);
        }

        // Return cached texture if available
        if self.textures.contains_key(&asset_id) {
            return self.textures.get(&asset_id);
        }

        // Generate new thumbnail
        if let Some(rgba_data) = generator() {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize],
                &rgba_data,
            );
            let texture = ctx.load_texture(
                format!("thumbnail_{}", asset_id),
                color_image,
                egui::TextureOptions::LINEAR,
            );
            self.textures.insert(asset_id, texture);
            return self.textures.get(&asset_id);
        }

        None
    }

    /// Check if a thumbnail is already cached (and not dirty)
    #[allow(dead_code)]
    pub fn has(&self, asset_id: &Uuid) -> bool {
        self.textures.contains_key(asset_id) && !self.dirty.contains(asset_id)
    }

    /// Mark an asset's thumbnail as needing regeneration
    pub fn invalidate(&mut self, asset_id: &Uuid) {
        self.dirty.insert(*asset_id);
    }

}

// ============================================================================
// Thumbnail Generation Functions
// ============================================================================

/// Generate a 64x64 RGBA thumbnail for an image asset
fn generate_image_thumbnail(asset: &lightningbeam_core::clip::ImageAsset) -> Option<Vec<u8>> {
    let data = asset.data.as_ref()?;

    // Decode the image
    let img = image::load_from_memory(data).ok()?;

    // Resize to thumbnail size using Lanczos3 filter for quality
    let thumbnail = img.resize_exact(
        THUMBNAIL_SIZE,
        THUMBNAIL_SIZE,
        image::imageops::FilterType::Lanczos3,
    );

    // Convert to RGBA8
    Some(thumbnail.to_rgba8().into_raw())
}

/// Generate a placeholder thumbnail with a solid color and optional icon indication
fn generate_placeholder_thumbnail(category: AssetCategory, bg_alpha: u8) -> Vec<u8> {
    let size = THUMBNAIL_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];

    // Get category color for the placeholder
    let color = category.color();

    // Fill with semi-transparent background
    for pixel in rgba.chunks_mut(4) {
        pixel[0] = 40;
        pixel[1] = 40;
        pixel[2] = 40;
        pixel[3] = bg_alpha;
    }

    // Draw a simple icon/indicator in the center based on category
    let center = size / 2;
    let icon_size = size / 3;

    match category {
        AssetCategory::Video => {
            // Draw a play triangle
            for y in 0..icon_size {
                let row_width = (y * icon_size / icon_size).max(1);
                for x in 0..row_width {
                    let px = center - icon_size / 4 + x;
                    let py = center - icon_size / 2 + y;
                    if px < size && py < size {
                        let idx = (py * size + px) * 4;
                        rgba[idx] = color.r();
                        rgba[idx + 1] = color.g();
                        rgba[idx + 2] = color.b();
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
        _ => {
            // Draw a simple rectangle
            let half = icon_size / 2;
            for y in (center - half)..(center + half) {
                for x in (center - half)..(center + half) {
                    if x < size && y < size {
                        let idx = (y * size + x) * 4;
                        rgba[idx] = color.r();
                        rgba[idx + 1] = color.g();
                        rgba[idx + 2] = color.b();
                        rgba[idx + 3] = 200;
                    }
                }
            }
        }
    }

    rgba
}

/// Helper function to fill a thumbnail buffer with a background color
fn fill_thumbnail_background(rgba: &mut [u8], color: egui::Color32) {
    for pixel in rgba.chunks_mut(4) {
        pixel[0] = color.r();
        pixel[1] = color.g();
        pixel[2] = color.b();
        pixel[3] = color.a();
    }
}

/// Helper function to draw a rectangle on the thumbnail buffer
fn draw_thumbnail_rect(
    rgba: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: egui::Color32,
) {
    for dy in 0..h {
        for dx in 0..w {
            let px = x + dx;
            let py = y + dy;
            if px < width && py < width {
                let idx = (py * width + px) * 4;
                if idx + 3 < rgba.len() {
                    rgba[idx] = color.r();
                    rgba[idx + 1] = color.g();
                    rgba[idx + 2] = color.b();
                    rgba[idx + 3] = color.a();
                }
            }
        }
    }
}

/// Generate a waveform thumbnail for sampled audio
/// Shows the first THUMBNAIL_PREVIEW_SECONDS of audio to avoid solid blobs for long clips
fn generate_waveform_thumbnail(
    waveform_peaks: &[(f32, f32)], // (min, max) pairs
    bg_color: egui::Color32,
    wave_color: egui::Color32,
) -> Vec<u8> {
    let size = THUMBNAIL_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];

    // Fill background
    fill_thumbnail_background(&mut rgba, bg_color);

    // Draw waveform
    let center_y = size / 2;
    let _num_peaks = waveform_peaks.len().min(size);

    for (x, &(min_val, max_val)) in waveform_peaks.iter().take(size).enumerate() {
        // Scale peaks to pixel range (center ± half height)
        let min_y = (center_y as f32 + min_val * center_y as f32) as usize;
        let max_y = (center_y as f32 + max_val * center_y as f32) as usize;

        let y_start = min_y.min(max_y).min(size - 1);
        let y_end = min_y.max(max_y).min(size - 1);

        for y in y_start..=y_end {
            let idx = (y * size + x) * 4;
            if idx + 3 < rgba.len() {
                rgba[idx] = wave_color.r();
                rgba[idx + 1] = wave_color.g();
                rgba[idx + 2] = wave_color.b();
                rgba[idx + 3] = 255;
            }
        }
    }

    rgba
}

/// Generate a video thumbnail by decoding the first frame
/// Returns a 64x64 RGBA thumbnail with letterboxing to maintain aspect ratio
fn generate_video_thumbnail(
    clip_id: &uuid::Uuid,
    video_manager: &std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>,
) -> Option<Vec<u8>> {
    // Get a frame from the video (at 1 second to skip potential black intros)
    let timestamp = 1.0;

    let frame = {
        let mut video_mgr = video_manager.lock().ok()?;
        video_mgr.get_frame(clip_id, timestamp)?
    };

    let src_width = frame.width as usize;
    let src_height = frame.height as usize;
    let dst_size = THUMBNAIL_SIZE as usize;

    // Calculate letterboxing dimensions to maintain aspect ratio
    let src_aspect = src_width as f32 / src_height as f32;
    let (scaled_width, scaled_height, offset_x, offset_y) = if src_aspect > 1.0 {
        // Wide video - letterbox top and bottom
        let scaled_width = dst_size;
        let scaled_height = (dst_size as f32 / src_aspect) as usize;
        let offset_y = (dst_size - scaled_height) / 2;
        (scaled_width, scaled_height, 0, offset_y)
    } else {
        // Tall video - letterbox left and right
        let scaled_height = dst_size;
        let scaled_width = (dst_size as f32 * src_aspect) as usize;
        let offset_x = (dst_size - scaled_width) / 2;
        (scaled_width, scaled_height, offset_x, 0)
    };

    // Create thumbnail with black letterbox bars
    let mut rgba = vec![0u8; dst_size * dst_size * 4];

    let x_ratio = src_width as f32 / scaled_width as f32;
    let y_ratio = src_height as f32 / scaled_height as f32;

    // Fill the scaled region
    for dst_y in 0..scaled_height {
        for dst_x in 0..scaled_width {
            let src_x = (dst_x as f32 * x_ratio) as usize;
            let src_y = (dst_y as f32 * y_ratio) as usize;
            let src_idx = (src_y * src_width + src_x) * 4;

            let final_x = dst_x + offset_x;
            let final_y = dst_y + offset_y;
            let dst_idx = (final_y * dst_size + final_x) * 4;

            // Copy RGBA bytes
            if src_idx + 3 < frame.rgba_data.len() && dst_idx + 3 < rgba.len() {
                rgba[dst_idx] = frame.rgba_data[src_idx];
                rgba[dst_idx + 1] = frame.rgba_data[src_idx + 1];
                rgba[dst_idx + 2] = frame.rgba_data[src_idx + 2];
                rgba[dst_idx + 3] = frame.rgba_data[src_idx + 3];
            }
        }
    }

    Some(rgba)
}

/// Generate a piano roll thumbnail for MIDI clips
/// Shows notes as horizontal bars with Y position = note % 12 (one octave)
fn generate_midi_thumbnail(
    events: &[(f64, u8, u8, bool)], // (timestamp, note_number, velocity, is_note_on)
    duration: f64,
    bg_color: egui::Color32,
    note_color: egui::Color32,
) -> Vec<u8> {
    let size = THUMBNAIL_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];

    // Fill background
    fill_thumbnail_background(&mut rgba, bg_color);

    // Limit to first 10 seconds
    let preview_duration = duration.min(THUMBNAIL_PREVIEW_SECONDS);
    if preview_duration <= 0.0 {
        return rgba;
    }

    // Draw note events
    for &(timestamp, note_number, _velocity, is_note_on) in events {
        if !is_note_on || timestamp > preview_duration {
            continue;
        }

        let x = ((timestamp / preview_duration) * size as f64) as usize;

        // Note position: modulo 12 (one octave), mapped to full height
        // Note 0 (C) at bottom, Note 11 (B) at top
        let note_in_octave = note_number % 12;
        let y = size - 1 - (note_in_octave as usize * size / 12);

        // Draw a small rectangle for the note
        draw_thumbnail_rect(&mut rgba, size, x.min(size - 2), y.saturating_sub(2), 2, 4, note_color);
    }

    rgba
}

/// Generate a 64x64 RGBA thumbnail for a vector clip
/// Renders frame 0 of the clip using tiny-skia for software rendering
fn generate_vector_thumbnail(clip: &VectorClip, bg_color: egui::Color32) -> Vec<u8> {
    use tiny_skia::Pixmap;

    let size = THUMBNAIL_SIZE as usize;
    let mut pixmap = Pixmap::new(THUMBNAIL_SIZE, THUMBNAIL_SIZE)
        .unwrap_or_else(|| Pixmap::new(1, 1).unwrap());

    // Fill background
    pixmap.fill(tiny_skia::Color::from_rgba8(
        bg_color.r(),
        bg_color.g(),
        bg_color.b(),
        bg_color.a(),
    ));

    // Calculate scale to fit clip dimensions into thumbnail
    let scale_x = THUMBNAIL_SIZE as f64 / clip.width.max(1.0);
    let scale_y = THUMBNAIL_SIZE as f64 / clip.height.max(1.0);
    let _scale = scale_x.min(scale_y) * 0.9; // 90% to leave a small margin

    // Iterate through layers and render shapes
    for layer_node in clip.layers.iter() {
        if let AnyLayer::Vector(vector_layer) = &layer_node.data {
            // TODO: DCEL - thumbnail shape rendering disabled during migration
            // (was: shapes_at_time(0.0) to render shape fills/strokes into thumbnail)
            let _ = vector_layer;
        }
    }

    // Convert to RGBA bytes
    let data = pixmap.data();
    // tiny-skia uses premultiplied RGBA, need to convert to straight alpha for egui
    let mut rgba = Vec::with_capacity(size * size * 4);
    for chunk in data.chunks(4) {
        let a = chunk[3] as f32 / 255.0;
        if a > 0.0 {
            // Unpremultiply
            rgba.push((chunk[0] as f32 / a).min(255.0) as u8);
            rgba.push((chunk[1] as f32 / a).min(255.0) as u8);
            rgba.push((chunk[2] as f32 / a).min(255.0) as u8);
            rgba.push(chunk[3]);
        } else {
            rgba.extend_from_slice(chunk);
        }
    }
    rgba
}

/// Generate a simple effect thumbnail with a pink gradient
#[allow(dead_code)]
fn generate_effect_thumbnail() -> Vec<u8> {
    let size = THUMBNAIL_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];

    // Pink gradient background with "FX" visual indicator
    for y in 0..size {
        for x in 0..size {
            let brightness = 1.0 - (y as f32 / size as f32) * 0.3;
            let idx = (y * size + x) * 4;
            rgba[idx] = (220.0 * brightness) as u8;     // R
            rgba[idx + 1] = (80.0 * brightness) as u8;  // G
            rgba[idx + 2] = (160.0 * brightness) as u8; // B
            rgba[idx + 3] = 200;                         // A
        }
    }

    // Draw a simple "FX" pattern in the center using darker pixels
    let center = size / 2;
    let letter_size = size / 4;

    // Draw "F" - vertical bar
    for y in (center - letter_size)..(center + letter_size) {
        let x = center - letter_size;
        let idx = (y * size + x) * 4;
        rgba[idx] = 255;
        rgba[idx + 1] = 255;
        rgba[idx + 2] = 255;
        rgba[idx + 3] = 255;
    }
    // Draw "F" - top horizontal
    for x in (center - letter_size)..(center - 2) {
        let y = center - letter_size;
        let idx = (y * size + x) * 4;
        rgba[idx] = 255;
        rgba[idx + 1] = 255;
        rgba[idx + 2] = 255;
        rgba[idx + 3] = 255;
    }
    // Draw "F" - middle horizontal
    for x in (center - letter_size)..(center - 4) {
        let y = center;
        let idx = (y * size + x) * 4;
        rgba[idx] = 255;
        rgba[idx + 1] = 255;
        rgba[idx + 2] = 255;
        rgba[idx + 3] = 255;
    }

    // Draw "X" - diagonal lines
    for i in 0..letter_size {
        // Top-left to bottom-right
        let x1 = center + 2 + i;
        let y1 = center - letter_size + i * 2;
        if x1 < size && y1 < size {
            let idx = (y1 * size + x1) * 4;
            rgba[idx] = 255;
            rgba[idx + 1] = 255;
            rgba[idx + 2] = 255;
            rgba[idx + 3] = 255;
        }
        // Top-right to bottom-left
        let x2 = center + letter_size - i;
        let y2 = center - letter_size + i * 2;
        if x2 < size && y2 < size {
            let idx = (y2 * size + x2) * 4;
            rgba[idx] = 255;
            rgba[idx + 1] = 255;
            rgba[idx + 2] = 255;
            rgba[idx + 3] = 255;
        }
    }

    rgba
}

/// Ellipsize a string to fit within a maximum character count
#[allow(dead_code)]
fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Asset category for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCategory {
    All,
    Vector,
    Video,
    Audio,
    Images,
    Effects,
}

impl AssetCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            AssetCategory::All => "All",
            AssetCategory::Vector => "Vector",
            AssetCategory::Video => "Video",
            AssetCategory::Audio => "Audio",
            AssetCategory::Images => "Images",
            AssetCategory::Effects => "Effects",
        }
    }

    pub fn all() -> &'static [AssetCategory] {
        &[
            AssetCategory::All,
            AssetCategory::Vector,
            AssetCategory::Video,
            AssetCategory::Audio,
            AssetCategory::Images,
            AssetCategory::Effects,
        ]
    }

    /// Get the color associated with this category
    pub fn color(&self) -> egui::Color32 {
        match self {
            AssetCategory::All => egui::Color32::from_gray(150),
            AssetCategory::Vector => egui::Color32::from_rgb(100, 150, 255), // Blue
            AssetCategory::Video => egui::Color32::from_rgb(255, 150, 100),  // Orange
            AssetCategory::Audio => egui::Color32::from_rgb(100, 255, 150),  // Green
            AssetCategory::Images => egui::Color32::from_rgb(255, 200, 100), // Yellow/Gold
            AssetCategory::Effects => egui::Color32::from_rgb(220, 80, 160), // Pink
        }
    }
}

/// Unified asset entry for display
#[derive(Debug, Clone)]
pub struct AssetEntry {
    pub id: Uuid,
    pub name: String,
    pub category: AssetCategory,
    /// More specific clip type for drag-and-drop compatibility
    pub drag_clip_type: DragClipType,
    pub duration: f64,
    pub dimensions: Option<(f64, f64)>,
    pub extra_info: String,
    /// True for built-in effects from the registry (not editable/deletable)
    pub is_builtin: bool,
    /// Folder this asset belongs to (None = root)
    pub folder_id: Option<Uuid>,
}

/// Folder entry for display
#[derive(Debug, Clone)]
pub struct FolderEntry {
    pub id: Uuid,
    pub name: String,
    #[allow(dead_code)]
    pub category: AssetCategory,
    pub item_count: usize,
}

/// Library item - either a folder or an asset
#[derive(Debug, Clone)]
pub enum LibraryItem {
    Folder(FolderEntry),
    Asset(AssetEntry),
}

impl LibraryItem {
    #[allow(dead_code)]
    pub fn id(&self) -> Uuid {
        match self {
            LibraryItem::Folder(f) => f.id,
            LibraryItem::Asset(a) => a.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            LibraryItem::Folder(f) => &f.name,
            LibraryItem::Asset(a) => &a.name,
        }
    }
}

/// Pending delete confirmation state
#[derive(Debug, Clone)]
struct PendingDelete {
    asset_id: Uuid,
    asset_name: String,
    category: AssetCategory,
    in_use: bool,
}

/// Inline rename editing state
#[derive(Debug, Clone)]
struct RenameState {
    asset_id: Uuid,
    category: AssetCategory,
    edit_text: String,
}

/// Inline folder rename editing state
#[derive(Debug, Clone)]
struct FolderRenameState {
    folder_id: Uuid,
    category: AssetCategory,
    edit_text: String,
}

/// Context menu state with position
#[derive(Debug, Clone)]
struct ContextMenuState {
    asset_id: Uuid,
    position: egui::Pos2,
}

#[derive(Debug, Clone)]
struct FolderContextMenuState {
    folder_id: Uuid,
    position: egui::Pos2,
}

pub struct AssetLibraryPane {
    /// Current search filter text
    search_filter: String,

    /// Currently selected category tab
    selected_category: AssetCategory,

    /// Currently selected asset ID (for future drag-to-timeline)
    selected_asset: Option<Uuid>,

    /// Context menu state with position (for assets)
    context_menu: Option<ContextMenuState>,

    /// Folder context menu state (for folders)
    folder_context_menu: Option<FolderContextMenuState>,

    /// Pane context menu position (for background right-click)
    pane_context_menu: Option<egui::Pos2>,

    /// Pending delete confirmation
    pending_delete: Option<PendingDelete>,

    /// Active rename state (for assets)
    rename_state: Option<RenameState>,

    /// Active folder rename state
    folder_rename_state: Option<FolderRenameState>,

    /// Current view mode (list or grid)
    view_mode: AssetViewMode,

    /// Thumbnail texture cache
    thumbnail_cache: ThumbnailCache,

    /// Current folder navigation per category (category index -> current folder ID)
    /// None means at root level
    current_folders: HashMap<u8, Option<Uuid>>,

    /// Set of expanded folder IDs (for tree view - future enhancement)
    #[allow(dead_code)]
    expanded_folders: HashSet<Uuid>,

    /// Cached folder icon texture
    folder_icon: Option<egui::TextureHandle>,
}

// Embedded folder icon SVG
const FOLDER_ICON_SVG: &[u8] = include_bytes!("../../../../src/assets/folder.svg");

impl AssetLibraryPane {
    pub fn new() -> Self {
        Self {
            search_filter: String::new(),
            selected_category: AssetCategory::All,
            selected_asset: None,
            context_menu: None,
            folder_context_menu: None,
            pane_context_menu: None,
            pending_delete: None,
            rename_state: None,
            folder_rename_state: None,
            view_mode: AssetViewMode::default(),
            thumbnail_cache: ThumbnailCache::new(),
            current_folders: HashMap::new(),
            expanded_folders: HashSet::new(),
            folder_icon: None,
        }
    }

    /// Get or load the folder icon texture
    fn get_folder_icon(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if self.folder_icon.is_none() {
            // Rasterize the embedded SVG
            let render_size = 32; // Render at 32px for list/grid views

            if let Ok(tree) = resvg::usvg::Tree::from_data(FOLDER_ICON_SVG, &resvg::usvg::Options::default()) {
                let pixmap_size = tree.size().to_int_size();
                let scale_x = render_size as f32 / pixmap_size.width() as f32;
                let scale_y = render_size as f32 / pixmap_size.height() as f32;
                let scale = scale_x.min(scale_y);

                if let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(render_size, render_size) {
                    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
                    resvg::render(&tree, transform, &mut pixmap.as_mut());

                    let rgba_data = pixmap.data();
                    let size = [pixmap.width() as usize, pixmap.height() as usize];
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba_data);

                    let texture = ctx.load_texture(
                        "folder_icon",
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );

                    self.folder_icon = Some(texture);
                }
            }
        }

        self.folder_icon.as_ref()
    }

    /// Get the current folder for the selected category
    fn get_current_folder(&self) -> Option<Uuid> {
        let category_index = match self.selected_category {
            AssetCategory::All => return None, // All category doesn't have folders
            AssetCategory::Vector => 1,
            AssetCategory::Video => 2,
            AssetCategory::Audio => 3,
            AssetCategory::Images => 4,
            AssetCategory::Effects => 5,
        };

        self.current_folders.get(&category_index).copied().flatten()
    }

    /// Set the current folder for the selected category
    fn set_current_folder(&mut self, folder_id: Option<Uuid>) {
        let category_index = match self.selected_category {
            AssetCategory::All => return, // All category doesn't have folders
            AssetCategory::Vector => 1,
            AssetCategory::Video => 2,
            AssetCategory::Audio => 3,
            AssetCategory::Images => 4,
            AssetCategory::Effects => 5,
        };

        self.current_folders.insert(category_index, folder_id);
    }

    /// Convert UI AssetCategory to core AssetCategory
    fn to_core_category(category: AssetCategory) -> Option<lightningbeam_core::document::AssetCategory> {
        match category {
            AssetCategory::All => None,
            AssetCategory::Vector => Some(lightningbeam_core::document::AssetCategory::Vector),
            AssetCategory::Video => Some(lightningbeam_core::document::AssetCategory::Video),
            AssetCategory::Audio => Some(lightningbeam_core::document::AssetCategory::Audio),
            AssetCategory::Images => Some(lightningbeam_core::document::AssetCategory::Images),
            AssetCategory::Effects => Some(lightningbeam_core::document::AssetCategory::Effects),
        }
    }

    /// Convert DragClipType to core AssetCategory
    fn drag_clip_type_to_core_category(clip_type: DragClipType) -> lightningbeam_core::document::AssetCategory {
        match clip_type {
            DragClipType::Vector => lightningbeam_core::document::AssetCategory::Vector,
            DragClipType::Video => lightningbeam_core::document::AssetCategory::Video,
            DragClipType::AudioSampled | DragClipType::AudioMidi => lightningbeam_core::document::AssetCategory::Audio,
            DragClipType::Image => lightningbeam_core::document::AssetCategory::Images,
            DragClipType::Effect => lightningbeam_core::document::AssetCategory::Effects,
        }
    }

    /// Convert DragClipType to UI AssetCategory
    fn drag_clip_type_to_category(clip_type: DragClipType) -> AssetCategory {
        match clip_type {
            DragClipType::Vector => AssetCategory::Vector,
            DragClipType::Video => AssetCategory::Video,
            DragClipType::AudioSampled | DragClipType::AudioMidi => AssetCategory::Audio,
            DragClipType::Image => AssetCategory::Images,
            DragClipType::Effect => AssetCategory::Effects,
        }
    }

    /// Collect all assets from the document into a unified list
    fn collect_assets(&self, document: &Document) -> Vec<AssetEntry> {
        let mut assets = Vec::new();

        // Collect vector clips
        for (id, clip) in &document.vector_clips {
            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Vector,
                drag_clip_type: DragClipType::Vector,
                duration: clip.duration,
                dimensions: Some((clip.width, clip.height)),
                extra_info: format!("{}x{}", clip.width as u32, clip.height as u32),
                is_builtin: false,
                folder_id: clip.folder_id,
            });
        }

        // Collect video clips
        for (id, clip) in &document.video_clips {
            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Video,
                drag_clip_type: DragClipType::Video,
                duration: clip.duration,
                dimensions: Some((clip.width, clip.height)),
                extra_info: format!("{:.0}fps", clip.frame_rate),
                is_builtin: false,
                folder_id: clip.folder_id,
            });
        }

        // Build set of audio clip IDs that are linked to videos
        let linked_audio_ids: std::collections::HashSet<uuid::Uuid> = document.video_clips.values()
            .filter_map(|video| video.linked_audio_clip_id)
            .collect();

        // Collect audio clips (skip those linked to videos)
        for (id, clip) in &document.audio_clips {
            // Skip if this audio clip is linked to a video
            if linked_audio_ids.contains(id) {
                continue;
            }

            let (extra_info, drag_clip_type) = match &clip.clip_type {
                AudioClipType::Sampled { .. } => ("Sampled".to_string(), DragClipType::AudioSampled),
                AudioClipType::Midi { .. } => ("MIDI".to_string(), DragClipType::AudioMidi),
                AudioClipType::Recording => {
                    // Skip recording-in-progress clips from asset library
                    continue;
                }
            };

            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Audio,
                drag_clip_type,
                duration: clip.duration,
                dimensions: None,
                extra_info,
                is_builtin: false,
                folder_id: clip.folder_id,
            });
        }

        // Collect image assets
        for (id, asset) in &document.image_assets {
            assets.push(AssetEntry {
                id: *id,
                name: asset.name.clone(),
                category: AssetCategory::Images,
                drag_clip_type: DragClipType::Image,
                duration: 0.0, // Images don't have duration
                dimensions: Some((asset.width as f64, asset.height as f64)),
                extra_info: format!("{}x{}", asset.width, asset.height),
                is_builtin: false,
                folder_id: asset.folder_id,
            });
        }

        // Collect built-in effects from registry
        for effect_def in lightningbeam_core::effect_registry::EffectRegistry::get_all() {
            assets.push(AssetEntry {
                id: effect_def.id,
                name: effect_def.name.clone(),
                category: AssetCategory::Effects,
                drag_clip_type: DragClipType::Effect,
                duration: 5.0, // Default duration when dropped
                dimensions: None,
                extra_info: format!("{:?}", effect_def.category),
                is_builtin: true, // Built-in from registry
                folder_id: None, // Built-in effects are at root
            });
        }

        // Collect user-edited effects from document (that aren't in registry)
        let registry_ids: HashSet<Uuid> = lightningbeam_core::effect_registry::EffectRegistry::get_all()
            .iter()
            .map(|e| e.id)
            .collect();

        for effect_def in document.effect_definitions.values() {
            if !registry_ids.contains(&effect_def.id) {
                // User-created/modified effect
                assets.push(AssetEntry {
                    id: effect_def.id,
                    name: effect_def.name.clone(),
                    category: AssetCategory::Effects,
                    drag_clip_type: DragClipType::Effect,
                    duration: 5.0,
                    dimensions: None,
                    extra_info: format!("{:?}", effect_def.category),
                    is_builtin: false, // User effect
                    folder_id: effect_def.folder_id,
                });
            }
        }

        // Sort alphabetically by name
        assets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        assets
    }

    /// Collect folders and assets for the current view (folder-aware)
    fn collect_items(&self, document: &Document) -> Vec<LibraryItem> {
        let mut items = Vec::new();

        // For "All" category, return all assets except built-in effects (no folders)
        if self.selected_category == AssetCategory::All {
            let assets = self.collect_assets(document);
            return assets.into_iter()
                .filter(|asset| {
                    // Exclude built-in effects from "All" category
                    !(asset.category == AssetCategory::Effects && asset.is_builtin)
                })
                .map(LibraryItem::Asset)
                .collect();
        }

        // Get the core category and folder tree
        let Some(core_category) = Self::to_core_category(self.selected_category) else {
            return items;
        };

        let folder_tree = document.get_folder_tree(core_category);
        let current_folder = self.get_current_folder();

        // Collect folders at the current level
        let folders = if let Some(parent_id) = current_folder {
            folder_tree.children_of(&parent_id)
        } else {
            folder_tree.root_folders()
        };

        for folder in folders {
            // Count items in this folder (subfolders + assets)
            let subfolder_count = folder_tree.children_of(&folder.id).len();

            // Count assets in this folder
            let asset_count = match self.selected_category {
                AssetCategory::Vector => document
                    .vector_clips
                    .values()
                    .filter(|c| c.folder_id == Some(folder.id))
                    .count(),
                AssetCategory::Video => document
                    .video_clips
                    .values()
                    .filter(|c| c.folder_id == Some(folder.id))
                    .count(),
                AssetCategory::Audio => document
                    .audio_clips
                    .values()
                    .filter(|c| c.folder_id == Some(folder.id))
                    .count(),
                AssetCategory::Images => document
                    .image_assets
                    .values()
                    .filter(|a| a.folder_id == Some(folder.id))
                    .count(),
                AssetCategory::Effects => document
                    .effect_definitions
                    .values()
                    .filter(|e| e.folder_id == Some(folder.id))
                    .count(),
                AssetCategory::All => 0,
            };

            items.push(LibraryItem::Folder(FolderEntry {
                id: folder.id,
                name: folder.name.clone(),
                category: self.selected_category,
                item_count: subfolder_count + asset_count,
            }));
        }

        // Collect assets at the current level
        match self.selected_category {
            AssetCategory::Vector => {
                for (id, clip) in &document.vector_clips {
                    if clip.folder_id == current_folder {
                        items.push(LibraryItem::Asset(AssetEntry {
                            id: *id,
                            name: clip.name.clone(),
                            category: AssetCategory::Vector,
                            drag_clip_type: DragClipType::Vector,
                            duration: clip.duration,
                            dimensions: Some((clip.width, clip.height)),
                            extra_info: format!("{}x{}", clip.width as u32, clip.height as u32),
                            is_builtin: false,
                            folder_id: clip.folder_id,
                        }));
                    }
                }
            }
            AssetCategory::Video => {
                for (id, clip) in &document.video_clips {
                    if clip.folder_id == current_folder {
                        items.push(LibraryItem::Asset(AssetEntry {
                            id: *id,
                            name: clip.name.clone(),
                            category: AssetCategory::Video,
                            drag_clip_type: DragClipType::Video,
                            duration: clip.duration,
                            dimensions: Some((clip.width, clip.height)),
                            extra_info: format!("{:.0}fps", clip.frame_rate),
                            is_builtin: false,
                            folder_id: clip.folder_id,
                        }));
                    }
                }
            }
            AssetCategory::Audio => {
                // Build set of linked audio IDs to skip
                let linked_audio_ids: HashSet<Uuid> = document
                    .video_clips
                    .values()
                    .filter_map(|v| v.linked_audio_clip_id)
                    .collect();

                for (id, clip) in &document.audio_clips {
                    if !linked_audio_ids.contains(id) && clip.folder_id == current_folder {
                        let (extra_info, drag_clip_type) = match &clip.clip_type {
                            AudioClipType::Sampled { .. } => {
                                ("Sampled".to_string(), DragClipType::AudioSampled)
                            }
                            AudioClipType::Midi { .. } => {
                                ("MIDI".to_string(), DragClipType::AudioMidi)
                            }
                            AudioClipType::Recording => {
                                // Skip recording-in-progress clips
                                continue;
                            }
                        };

                        items.push(LibraryItem::Asset(AssetEntry {
                            id: *id,
                            name: clip.name.clone(),
                            category: AssetCategory::Audio,
                            drag_clip_type,
                            duration: clip.duration,
                            dimensions: None,
                            extra_info,
                            is_builtin: false,
                            folder_id: clip.folder_id,
                        }));
                    }
                }
            }
            AssetCategory::Images => {
                for (id, asset) in &document.image_assets {
                    if asset.folder_id == current_folder {
                        items.push(LibraryItem::Asset(AssetEntry {
                            id: *id,
                            name: asset.name.clone(),
                            category: AssetCategory::Images,
                            drag_clip_type: DragClipType::Image,
                            duration: 0.0,
                            dimensions: Some((asset.width as f64, asset.height as f64)),
                            extra_info: format!("{}x{}", asset.width, asset.height),
                            is_builtin: false,
                            folder_id: asset.folder_id,
                        }));
                    }
                }
            }
            AssetCategory::Effects => {
                // Built-in effects always appear at root level
                if current_folder.is_none() {
                    for effect_def in lightningbeam_core::effect_registry::EffectRegistry::get_all() {
                        items.push(LibraryItem::Asset(AssetEntry {
                            id: effect_def.id,
                            name: effect_def.name.clone(),
                            category: AssetCategory::Effects,
                            drag_clip_type: DragClipType::Effect,
                            duration: 0.0,
                            dimensions: None,
                            extra_info: format!("{:?}", effect_def.category),
                            is_builtin: true,
                            folder_id: None, // Built-in effects are always at root
                        }));
                    }
                }

                // User effects
                for (id, effect) in &document.effect_definitions {
                    if effect.folder_id == current_folder {
                        items.push(LibraryItem::Asset(AssetEntry {
                            id: *id,
                            name: effect.name.clone(),
                            category: AssetCategory::Effects,
                            drag_clip_type: DragClipType::Effect,
                            duration: 0.0,
                            dimensions: None,
                            extra_info: format!("{:?}", effect.category),
                            is_builtin: false,
                            folder_id: effect.folder_id,
                        }));
                    }
                }
            }
            AssetCategory::All => {
                // Already handled above
            }
        }

        // Sort: folders first (alphabetically), then assets (alphabetically)
        items.sort_by(|a, b| {
            match (a, b) {
                (LibraryItem::Folder(f1), LibraryItem::Folder(f2)) => {
                    f1.name.to_lowercase().cmp(&f2.name.to_lowercase())
                }
                (LibraryItem::Asset(a1), LibraryItem::Asset(a2)) => {
                    a1.name.to_lowercase().cmp(&a2.name.to_lowercase())
                }
                (LibraryItem::Folder(_), LibraryItem::Asset(_)) => std::cmp::Ordering::Less,
                (LibraryItem::Asset(_), LibraryItem::Folder(_)) => std::cmp::Ordering::Greater,
            }
        });

        items
    }

    /// Filter assets based on current category and search text
    #[allow(dead_code)]
    fn filter_assets<'a>(&self, assets: &'a [AssetEntry]) -> Vec<&'a AssetEntry> {
        let search_lower = self.search_filter.to_lowercase();

        assets
            .iter()
            .filter(|asset| {
                // Category filter
                let category_matches = if self.selected_category == AssetCategory::All {
                    // "All" tab: show everything EXCEPT built-in effects
                    // (built-in effects only appear in the Effects tab)
                    !(asset.category == AssetCategory::Effects && asset.is_builtin)
                } else {
                    asset.category == self.selected_category
                };

                // Search filter
                let search_matches =
                    search_lower.is_empty() || asset.name.to_lowercase().contains(&search_lower);

                category_matches && search_matches
            })
            .collect()
    }

    /// Check if an asset is currently in use (has clip instances on layers)
    fn is_asset_in_use(document: &Document, asset_id: Uuid, category: AssetCategory) -> bool {
        // Check all layers (root + inside movie clips) for clip instances referencing this asset
        for layer in document.all_layers() {
            match layer {
                lightningbeam_core::layer::AnyLayer::Vector(vl) => {
                    if category == AssetCategory::Vector {
                        for instance in &vl.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
                lightningbeam_core::layer::AnyLayer::Video(vl) => {
                    if category == AssetCategory::Video {
                        for instance in &vl.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
                lightningbeam_core::layer::AnyLayer::Audio(al) => {
                    if category == AssetCategory::Audio {
                        for instance in &al.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
                lightningbeam_core::layer::AnyLayer::Effect(el) => {
                    if category == AssetCategory::Effects {
                        for instance in &el.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
                lightningbeam_core::layer::AnyLayer::Group(_) => {
                    // Group layers don't have their own clip instances
                }
            }
        }
        false
    }

    /// Delete an asset from the document
    fn delete_asset(document: &mut Document, asset_id: Uuid, category: AssetCategory) {
        match category {
            AssetCategory::Vector => {
                document.remove_vector_clip(&asset_id);
            }
            AssetCategory::Video => {
                document.remove_video_clip(&asset_id);
            }
            AssetCategory::Audio => {
                document.remove_audio_clip(&asset_id);
            }
            AssetCategory::Images => {
                document.remove_image_asset(&asset_id);
            }
            AssetCategory::Effects => {
                document.effect_definitions.remove(&asset_id);
            }
            AssetCategory::All => {} // Not a real category for deletion
        }
    }

    /// Rename an asset in the document
    fn rename_asset(document: &mut Document, asset_id: Uuid, category: AssetCategory, new_name: &str) {
        match category {
            AssetCategory::Vector => {
                if let Some(clip) = document.get_vector_clip_mut(&asset_id) {
                    clip.name = new_name.to_string();
                }
            }
            AssetCategory::Video => {
                if let Some(clip) = document.get_video_clip_mut(&asset_id) {
                    clip.name = new_name.to_string();
                }
            }
            AssetCategory::Audio => {
                if let Some(clip) = document.get_audio_clip_mut(&asset_id) {
                    clip.name = new_name.to_string();
                }
            }
            AssetCategory::Images => {
                if let Some(asset) = document.get_image_asset_mut(&asset_id) {
                    asset.name = new_name.to_string();
                }
            }
            AssetCategory::Effects => {
                if let Some(effect) = document.effect_definitions.get_mut(&asset_id) {
                    effect.name = new_name.to_string();
                }
            }
            AssetCategory::All => {} // Not a real category for renaming
        }
    }

    /// Render the search bar at the top with view toggle buttons
    fn render_search_bar(&mut self, ui: &mut egui::Ui, rect: egui::Rect, shared: &SharedPaneState) {
        let search_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), SEARCH_BAR_HEIGHT));

        // Background
        let bg_style = shared.theme.style(".panel-header", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(30, 30, 30));
        ui.painter().rect_filled(search_rect, 0.0, bg_color);

        // View toggle buttons on the right (list and grid icons)
        let button_size = 20.0;
        let button_padding = 4.0;
        let buttons_width = button_size * 2.0 + button_padding * 3.0;

        // Grid view button (rightmost)
        let grid_button_rect = egui::Rect::from_min_size(
            egui::pos2(
                search_rect.max.x - button_size - button_padding,
                search_rect.min.y + (SEARCH_BAR_HEIGHT - button_size) / 2.0,
            ),
            egui::vec2(button_size, button_size),
        );

        // List view button
        let list_button_rect = egui::Rect::from_min_size(
            egui::pos2(
                grid_button_rect.min.x - button_size - button_padding,
                search_rect.min.y + (SEARCH_BAR_HEIGHT - button_size) / 2.0,
            ),
            egui::vec2(button_size, button_size),
        );

        // Draw and handle list button
        let list_selected = self.view_mode == AssetViewMode::List;
        let list_response = ui.allocate_rect(list_button_rect, egui::Sense::click());
        let list_bg = if list_selected {
            egui::Color32::from_rgb(70, 90, 110)
        } else if list_response.hovered() {
            egui::Color32::from_rgb(50, 50, 50)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(list_button_rect, 3.0, list_bg);

        // Draw list icon (three horizontal lines)
        let list_icon_color = if list_selected {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(150)
        };
        let line_spacing = 4.0;
        let line_width = 10.0;
        let line_x = list_button_rect.center().x - line_width / 2.0;
        for i in 0..3 {
            let line_y = list_button_rect.center().y - line_spacing + (i as f32 * line_spacing);
            ui.painter().line_segment(
                [
                    egui::pos2(line_x, line_y),
                    egui::pos2(line_x + line_width, line_y),
                ],
                egui::Stroke::new(1.5, list_icon_color),
            );
        }

        if list_response.clicked() {
            self.view_mode = AssetViewMode::List;
        }

        // Draw and handle grid button
        let grid_selected = self.view_mode == AssetViewMode::Grid;
        let grid_response = ui.allocate_rect(grid_button_rect, egui::Sense::click());
        let grid_bg = if grid_selected {
            egui::Color32::from_rgb(70, 90, 110)
        } else if grid_response.hovered() {
            egui::Color32::from_rgb(50, 50, 50)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(grid_button_rect, 3.0, grid_bg);

        // Draw grid icon (2x2 squares)
        let grid_icon_color = if grid_selected {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(150)
        };
        let square_size = 4.0;
        let square_gap = 2.0;
        let grid_start_x = grid_button_rect.center().x - square_size - square_gap / 2.0;
        let grid_start_y = grid_button_rect.center().y - square_size - square_gap / 2.0;
        for row in 0..2 {
            for col in 0..2 {
                let square_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        grid_start_x + col as f32 * (square_size + square_gap),
                        grid_start_y + row as f32 * (square_size + square_gap),
                    ),
                    egui::vec2(square_size, square_size),
                );
                ui.painter().rect_filled(square_rect, 1.0, grid_icon_color);
            }
        }

        if grid_response.clicked() {
            self.view_mode = AssetViewMode::Grid;
        }

        // Label position
        let label_pos = search_rect.min + egui::vec2(8.0, (SEARCH_BAR_HEIGHT - 14.0) / 2.0);
        ui.painter().text(
            label_pos,
            egui::Align2::LEFT_TOP,
            "Search:",
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(180),
        );

        // Text field using IME-safe widget (leave room for view toggle buttons)
        let text_edit_rect = egui::Rect::from_min_size(
            search_rect.min + egui::vec2(65.0, 4.0),
            egui::vec2(search_rect.width() - 75.0 - buttons_width, SEARCH_BAR_HEIGHT - 8.0),
        );

        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(text_edit_rect));
        ImeTextField::new(&mut self.search_filter)
            .placeholder("Filter assets...")
            .desired_width(text_edit_rect.width())
            .show(&mut child_ui);
    }

    /// Render category tabs
    fn render_category_tabs(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &SharedPaneState,
    ) {
        let tabs_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), CATEGORY_TAB_HEIGHT));

        // Background
        let bg_style = shared.theme.style(".panel-content", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(40, 40, 40));
        ui.painter().rect_filled(tabs_rect, 0.0, bg_color);

        // Tab buttons
        let tab_width = tabs_rect.width() / AssetCategory::all().len() as f32;

        for (i, category) in AssetCategory::all().iter().enumerate() {
            let tab_rect = egui::Rect::from_min_size(
                tabs_rect.min + egui::vec2(i as f32 * tab_width, 0.0),
                egui::vec2(tab_width, CATEGORY_TAB_HEIGHT),
            );

            let is_selected = self.selected_category == *category;

            // Tab background
            let tab_bg = if is_selected {
                egui::Color32::from_rgb(60, 60, 60)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(tab_rect, 0.0, tab_bg);

            // Handle click
            let response = ui.allocate_rect(tab_rect, egui::Sense::click());
            if response.clicked() {
                self.selected_category = *category;
            }

            // Category color indicator
            let indicator_color = category.color();

            let text_color = if is_selected {
                indicator_color
            } else {
                egui::Color32::from_gray(150)
            };

            ui.painter().text(
                tab_rect.center(),
                egui::Align2::CENTER_CENTER,
                category.display_name(),
                egui::FontId::proportional(12.0),
                text_color,
            );

            // Underline for selected tab
            if is_selected {
                ui.painter().line_segment(
                    [
                        egui::pos2(tab_rect.min.x + 4.0, tab_rect.max.y - 2.0),
                        egui::pos2(tab_rect.max.x - 4.0, tab_rect.max.y - 2.0),
                    ],
                    egui::Stroke::new(2.0, indicator_color),
                );
            }
        }
    }

    /// Render breadcrumb navigation showing current folder path
    fn render_breadcrumbs(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        document: &Document,
        shared: &SharedPaneState,
    ) {
        // Only show breadcrumbs for specific categories (not "All")
        if self.selected_category == AssetCategory::All {
            return;
        }

        let Some(core_category) = Self::to_core_category(self.selected_category) else {
            return;
        };

        // Background
        let bg_style = shared.theme.style(".panel-header", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(25, 25, 25));
        ui.painter().rect_filled(rect, 0.0, bg_color);

        // Get folder tree and build path
        let folder_tree = document.get_folder_tree(core_category);
        let current_folder = self.get_current_folder();

        // Build path: category name -> folder1 -> folder2 -> ...
        let mut path_items = vec![self.selected_category.display_name().to_string()];
        let mut path_folder_ids = Vec::new();

        if let Some(folder_id) = current_folder {
            let folder_path_ids = folder_tree.path_to_folder(&folder_id);
            for fid in &folder_path_ids {
                if let Some(folder) = folder_tree.folders.get(fid) {
                    path_items.push(folder.name.clone());
                    path_folder_ids.push(*fid);
                }
            }
        }

        // Render breadcrumb items
        let mut x_offset = rect.min.x + 8.0;
        let y_center = rect.min.y + BREADCRUMB_HEIGHT / 2.0;

        for (i, item_name) in path_items.iter().enumerate() {
            let is_last = i == path_items.len() - 1;

            // Calculate text size
            let font_id = egui::FontId::proportional(12.0);
            let text_galley = ui.painter().layout_no_wrap(
                item_name.clone(),
                font_id.clone(),
                egui::Color32::WHITE,
            );

            let text_width = text_galley.size().x;
            let item_rect = egui::Rect::from_min_size(
                egui::pos2(x_offset, rect.min.y),
                egui::vec2(text_width + 8.0, BREADCRUMB_HEIGHT),
            );

            // Make clickable if not the last item
            let response = ui.allocate_rect(item_rect, egui::Sense::click());

            // Determine color based on state
            let text_color = if is_last {
                egui::Color32::WHITE
            } else if response.hovered() {
                egui::Color32::from_rgb(100, 150, 255)
            } else {
                egui::Color32::from_rgb(150, 150, 150)
            };

            // Draw text
            ui.painter().text(
                egui::pos2(x_offset, y_center),
                egui::Align2::LEFT_CENTER,
                item_name,
                font_id,
                text_color,
            );

            // Handle click to navigate up the hierarchy
            if response.clicked() && !is_last {
                if i == 0 {
                    // Clicked on category root - go to root
                    self.set_current_folder(None);
                } else {
                    // Clicked on a folder - navigate to it
                    // Get the folder at this index (i-1 because category is at 0)
                    if i - 1 < path_folder_ids.len() {
                        self.set_current_folder(Some(path_folder_ids[i - 1]));
                    }
                }
            }

            x_offset += text_width + 8.0;

            // Draw separator (>) if not last
            if !is_last {
                ui.painter().text(
                    egui::pos2(x_offset, y_center),
                    egui::Align2::LEFT_CENTER,
                    ">",
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_rgb(100, 100, 100),
                );
                x_offset += 16.0;
            }
        }
    }

    /// Render a section header for effect categories
    #[allow(dead_code)] // Part of List/Grid view rendering subsystem, not yet wired
    fn render_section_header(ui: &mut egui::Ui, label: &str, color: egui::Color32) {
        ui.add_space(4.0);
        let (header_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 20.0),
            egui::Sense::hover(),
        );
        ui.painter().text(
            header_rect.min + egui::vec2(8.0, 2.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(11.0),
            color,
        );
        ui.add_space(2.0);
    }

    /// Render a grid of asset items
    #[allow(clippy::too_many_arguments, dead_code)]
    fn render_grid_items(
        &mut self,
        ui: &mut egui::Ui,
        assets: &[&AssetEntry],
        columns: usize,
        item_height: f32,
        content_width: f32,
        shared: &mut SharedPaneState,
        document: &Document,
        text_color: egui::Color32,
        _secondary_text_color: egui::Color32,
    ) {
        if assets.is_empty() {
            return;
        }

        let rows = (assets.len() + columns - 1) / columns;
        // Grid height: matches the positioning formula used below
        // Items are at: GRID_SPACING + row * (item_height + GRID_SPACING)
        // Last item bottom: GRID_SPACING + (rows-1) * (item_height + GRID_SPACING) + item_height
        //                 = GRID_SPACING + rows * item_height + (rows-1) * GRID_SPACING
        //                 = rows * (item_height + GRID_SPACING) + GRID_SPACING - GRID_SPACING (for last row)
        // Simplified: GRID_SPACING + rows * (item_height + GRID_SPACING)
        let grid_height = GRID_SPACING + rows as f32 * (item_height + GRID_SPACING);

        // Reserve space for this grid section
        // We need to use allocate_space to properly advance the cursor by the full height,
        // then calculate the rect ourselves
        let cursor_before = ui.cursor().min;
        let _ = ui.allocate_space(egui::vec2(content_width, grid_height));
        let grid_rect = egui::Rect::from_min_size(cursor_before, egui::vec2(content_width, grid_height));

        for (idx, asset) in assets.iter().enumerate() {
            let col = idx % columns;
            let row = idx / columns;

            let item_x = grid_rect.min.x + GRID_SPACING + col as f32 * (GRID_ITEM_SIZE + GRID_SPACING);
            let item_y = grid_rect.min.y + GRID_SPACING + row as f32 * (item_height + GRID_SPACING);

            let item_rect = egui::Rect::from_min_size(
                egui::pos2(item_x, item_y),
                egui::vec2(GRID_ITEM_SIZE, item_height),
            );

            // Use interact() instead of allocate_rect() because we've already allocated the
            // entire grid space via allocate_exact_size above - allocate_rect would double-count
            let response = ui.interact(item_rect, egui::Id::new(("grid_item", asset.id)), egui::Sense::click_and_drag());

            let is_selected = self.selected_asset == Some(asset.id);
            let is_being_dragged = shared.dragging_asset.as_ref().map(|d| d.clip_id == asset.id).unwrap_or(false);

            // Item background
            let item_bg = if is_being_dragged {
                egui::Color32::from_rgb(80, 100, 120)
            } else if is_selected {
                egui::Color32::from_rgb(60, 80, 100)
            } else if response.hovered() {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::from_rgb(35, 35, 35)
            };
            ui.painter().rect_filled(item_rect, 4.0, item_bg);

            // Thumbnail area
            let thumbnail_rect = egui::Rect::from_min_size(
                egui::pos2(
                    item_rect.min.x + (GRID_ITEM_SIZE - THUMBNAIL_SIZE as f32) / 2.0,
                    item_rect.min.y + 4.0,
                ),
                egui::vec2(THUMBNAIL_SIZE as f32, THUMBNAIL_SIZE as f32),
            );

            // Generate and display thumbnail
            let asset_id = asset.id;
            let asset_category = asset.category;
            let ctx = ui.ctx().clone();

            let prefetched_waveform: Option<Vec<(f32, f32)>> =
                if asset_category == AssetCategory::Audio && !self.thumbnail_cache.has(&asset_id) {
                    if let Some(clip) = document.audio_clips.get(&asset_id) {
                        if let AudioClipType::Sampled { audio_pool_index } = &clip.clip_type {
                            shared.raw_audio_cache.get(audio_pool_index)
                                .map(|raw| peaks_from_raw_audio(raw, THUMBNAIL_SIZE as usize))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

            let texture = self.thumbnail_cache.get_or_create(&ctx, asset_id, || {
                match asset_category {
                    AssetCategory::Images => document.image_assets.get(&asset_id).and_then(generate_image_thumbnail),
                    AssetCategory::Vector => {
                        let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                        document.vector_clips.get(&asset_id).map(|clip| generate_vector_thumbnail(clip, bg_color))
                    }
                    AssetCategory::Video => generate_video_thumbnail(&asset_id, &shared.video_manager)
                        .or_else(|| Some(generate_placeholder_thumbnail(AssetCategory::Video, 200))),
                    AssetCategory::Audio => {
                        if let Some(clip) = document.audio_clips.get(&asset_id) {
                            let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                            match &clip.clip_type {
                                AudioClipType::Sampled { .. } => {
                                    let wave_color = egui::Color32::from_rgb(100, 200, 100);
                                    if let Some(ref peaks) = prefetched_waveform {
                                        Some(generate_waveform_thumbnail(peaks, bg_color, wave_color))
                                    } else {
                                        Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                    }
                                }
                                AudioClipType::Midi { midi_clip_id } => {
                                    let note_color = egui::Color32::from_rgb(100, 200, 100);
                                    if let Some(events) = shared.midi_event_cache.get(midi_clip_id) {
                                        Some(generate_midi_thumbnail(events, clip.duration, bg_color, note_color))
                                    } else {
                                        Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                    }
                                }
                                AudioClipType::Recording => {
                                    // Recording in progress - show placeholder
                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                }
                            }
                        } else {
                            Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                        }
                    }
                    AssetCategory::Effects => {
                        // Use GPU-rendered effect thumbnail if available
                        if let Some(rgba) = shared.effect_thumbnail_cache.get(&asset_id) {
                            Some(rgba.clone())
                        } else {
                            // Request GPU thumbnail generation
                            shared.effect_thumbnail_requests.push(asset_id);
                            // Return None to avoid caching placeholder - will retry next frame
                            None
                        }
                    }
                    AssetCategory::All => None,
                }
            });

            // Either use cached texture or render placeholder directly for effects
            // Use painter().image() instead of ui.put() to avoid affecting the cursor
            if let Some(texture) = texture {
                ui.painter().image(
                    texture.id(),
                    thumbnail_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else if asset_category == AssetCategory::Effects {
                // Render effect placeholder directly (not cached) until GPU thumbnail ready
                let placeholder_rgba = generate_effect_thumbnail();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize],
                    &placeholder_rgba,
                );
                let texture = ctx.load_texture(
                    format!("effect_placeholder_{}", asset_id),
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                ui.painter().image(
                    texture.id(),
                    thumbnail_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            // Category color indicator
            let indicator_rect = egui::Rect::from_min_size(
                egui::pos2(thumbnail_rect.min.x, thumbnail_rect.max.y - 3.0),
                egui::vec2(THUMBNAIL_SIZE as f32, 3.0),
            );
            ui.painter().rect_filled(indicator_rect, 0.0, asset.category.color());

            // Asset name
            let name_display = ellipsize(&asset.name, 12);
            ui.painter().text(
                egui::pos2(item_rect.center().x, thumbnail_rect.max.y + 8.0),
                egui::Align2::CENTER_TOP,
                &name_display,
                egui::FontId::proportional(10.0),
                text_color,
            );

            // Handle interactions
            if response.clicked() {
                self.selected_asset = Some(asset.id);
            }

            if response.secondary_clicked() {
                if let Some(pos) = ui.ctx().pointer_interact_pos() {
                    self.context_menu = Some(ContextMenuState { asset_id: asset.id, position: pos });
                }
            }

            if response.double_clicked() {
                if asset.category == AssetCategory::Effects {
                    *shared.effect_to_load = Some(asset.id);
                } else if !asset.is_builtin {
                    self.rename_state = Some(RenameState {
                        asset_id: asset.id,
                        category: asset.category,
                        edit_text: asset.name.clone(),
                    });
                }
            }

            if response.drag_started() {
                let linked_audio_clip_id = if asset.drag_clip_type == DragClipType::Video {
                    document.video_clips.get(&asset.id).and_then(|video| video.linked_audio_clip_id)
                } else {
                    None
                };
                *shared.dragging_asset = Some(DraggingAsset {
                    clip_id: asset.id,
                    clip_type: asset.drag_clip_type,
                    name: asset.name.clone(),
                    duration: asset.duration,
                    dimensions: asset.dimensions,
                    linked_audio_clip_id,
                });
            }
        }
    }

    /// Render items (folders and assets) based on current view mode
    fn render_items(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
        items: &[&LibraryItem],
        document: &Document,
    ) {
        match self.view_mode {
            AssetViewMode::List => {
                self.render_items_list_view(ui, rect, path, shared, items, document);
            }
            AssetViewMode::Grid => {
                self.render_items_grid_view(ui, rect, path, shared, items, document);
            }
        }
    }

    /// Render items in list view (folders + assets)
    fn render_items_list_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
        items: &[&LibraryItem],
        document: &Document,
    ) {
        // Load folder icon if needed
        let folder_icon = self.get_folder_icon(ui.ctx()).cloned();

        let _scroll_area = egui::ScrollArea::vertical()
            .id_salt("asset_library_scroll")
            .show_viewport(ui, |ui, viewport| {
                ui.set_min_width(rect.width());

                for item in items {
                    match item {
                        LibraryItem::Folder(folder) => {
                            // Render folder item
                            let item_rect = egui::Rect::from_min_size(
                                egui::pos2(rect.min.x, ui.cursor().top()),
                                egui::vec2(rect.width(), ITEM_HEIGHT),
                            );

                            if viewport.intersects(item_rect) {
                                let response = ui.allocate_rect(item_rect, egui::Sense::click());

                                // Check if an asset is being dragged and matches this folder's category
                                let is_valid_drop_target = shared.dragging_asset.as_ref().map(|drag| {
                                    let drag_category = Self::drag_clip_type_to_category(drag.clip_type);
                                    drag_category == self.selected_category
                                }).unwrap_or(false);

                                let is_drop_hover = is_valid_drop_target && response.hovered();

                                // Background
                                let bg_color = if is_drop_hover {
                                    // Highlight as drop target
                                    egui::Color32::from_rgb(60, 100, 140)
                                } else if response.hovered() {
                                    egui::Color32::from_rgb(50, 50, 50)
                                } else {
                                    egui::Color32::from_rgb(35, 35, 35)
                                };
                                ui.painter().rect_filled(item_rect, 0.0, bg_color);

                                // Draw drop target indicator border
                                if is_drop_hover {
                                    ui.painter().rect_stroke(
                                        item_rect,
                                        0.0,
                                        egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 180, 255)),
                                        egui::StrokeKind::Middle,
                                    );
                                }

                                // Folder icon
                                if let Some(ref icon) = folder_icon {
                                    let icon_size = LIST_THUMBNAIL_SIZE;
                                    let icon_rect = egui::Rect::from_min_size(
                                        item_rect.min + egui::vec2(4.0, (ITEM_HEIGHT - icon_size) / 2.0),
                                        egui::vec2(icon_size, icon_size),
                                    );
                                    ui.painter().image(
                                        icon.id(),
                                        icon_rect,
                                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                        egui::Color32::WHITE,
                                    );
                                }

                                // Folder name (or inline edit field)
                                let is_renaming = self.folder_rename_state.as_ref().map(|s| s.folder_id == folder.id).unwrap_or(false);

                                if is_renaming {
                                    // Inline rename text field
                                    let name_rect = egui::Rect::from_min_size(
                                        item_rect.min + egui::vec2(LIST_THUMBNAIL_SIZE + 8.0, (ITEM_HEIGHT - 22.0) / 2.0),
                                        egui::vec2(200.0, 22.0),
                                    );

                                    if let Some(ref mut state) = self.folder_rename_state {
                                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
                                        ImeTextField::new(&mut state.edit_text)
                                            .font_size(13.0)
                                            .desired_width(name_rect.width())
                                            .request_focus()
                                            .show(&mut child_ui);
                                    }
                                } else {
                                    ui.painter().text(
                                        item_rect.min + egui::vec2(LIST_THUMBNAIL_SIZE + 12.0, ITEM_HEIGHT / 2.0),
                                        egui::Align2::LEFT_CENTER,
                                        &folder.name,
                                        egui::FontId::proportional(13.0),
                                        egui::Color32::WHITE,
                                    );
                                }

                                // Item count
                                let count_text = format!("{} items", folder.item_count);
                                ui.painter().text(
                                    item_rect.max - egui::vec2(8.0, ITEM_HEIGHT / 2.0),
                                    egui::Align2::RIGHT_CENTER,
                                    count_text,
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::from_rgb(150, 150, 150),
                                );

                                // Handle drop: move asset to folder
                                if is_drop_hover && ui.input(|i| i.pointer.any_released()) {
                                    if let Some(ref drag) = shared.dragging_asset.clone() {
                                        let core_category = Self::drag_clip_type_to_core_category(drag.clip_type);
                                        let action = lightningbeam_core::actions::MoveAssetToFolderAction::new(
                                            core_category,
                                            drag.clip_id,
                                            Some(folder.id),
                                        );
                                        let _ = shared.action_executor.execute(Box::new(action));
                                        *shared.dragging_asset = None;
                                    }
                                }

                                // Handle double-click to navigate into folder
                                if response.double_clicked() {
                                    self.set_current_folder(Some(folder.id));
                                }

                                // Handle right-click for context menu
                                if response.secondary_clicked() {
                                    self.folder_context_menu = Some(FolderContextMenuState {
                                        folder_id: folder.id,
                                        position: ui.ctx().pointer_interact_pos().unwrap_or(egui::pos2(0.0, 0.0)),
                                    });
                                }
                            } else {
                                ui.allocate_space(egui::vec2(rect.width(), ITEM_HEIGHT));
                            }
                        }
                        LibraryItem::Asset(asset) => {
                            // Render asset item
                            let item_rect = egui::Rect::from_min_size(
                                egui::pos2(rect.min.x, ui.cursor().top()),
                                egui::vec2(rect.width(), ITEM_HEIGHT),
                            );

                            if viewport.intersects(item_rect) {
                                self.render_single_asset_list(ui, asset, item_rect, document, shared);
                            } else {
                                ui.allocate_space(egui::vec2(rect.width(), ITEM_HEIGHT));
                            }
                        }
                    }
                }
            });
    }

    /// Render items in grid view (folders + assets)
    fn render_items_grid_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
        items: &[&LibraryItem],
        document: &Document,
    ) {
        // Load folder icon if needed
        let folder_icon = self.get_folder_icon(ui.ctx()).cloned();

        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("asset_library_grid_scroll", path))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(rect.width() - 16.0); // Account for scrollbar

                let items_per_row =
                    ((rect.width() - GRID_SPACING) / (GRID_ITEM_SIZE + GRID_SPACING)).floor() as usize;
                let items_per_row = items_per_row.max(1);

                for row_start in (0..items.len()).step_by(items_per_row) {
                    ui.horizontal(|ui| {
                        for i in 0..items_per_row {
                            let index = row_start + i;
                            if index >= items.len() {
                                break;
                            }

                            let item = items[index];
                            match item {
                                LibraryItem::Folder(folder) => {
                                    // Render folder in grid (with space for name and count below)
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(GRID_ITEM_SIZE, GRID_ITEM_SIZE + 20.0),
                                        egui::Sense::click(),
                                    );

                                    // Check if an asset is being dragged and matches this folder's category
                                    let is_valid_drop_target = shared.dragging_asset.as_ref().map(|drag| {
                                        let drag_category = Self::drag_clip_type_to_category(drag.clip_type);
                                        drag_category == self.selected_category
                                    }).unwrap_or(false);

                                    let is_drop_hover = is_valid_drop_target && response.hovered();

                                    // Background
                                    let bg_color = if is_drop_hover {
                                        // Highlight as drop target
                                        egui::Color32::from_rgb(60, 100, 140)
                                    } else if response.hovered() {
                                        egui::Color32::from_rgb(50, 50, 50)
                                    } else {
                                        egui::Color32::from_rgb(35, 35, 35)
                                    };
                                    ui.painter().rect_filled(rect, 4.0, bg_color);

                                    // Draw drop target indicator border
                                    if is_drop_hover {
                                        ui.painter().rect_stroke(
                                            rect,
                                            4.0,
                                            egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 180, 255)),
                                            egui::StrokeKind::Middle,
                                        );
                                    }

                                    // Folder icon (centered)
                                    if let Some(ref icon) = folder_icon {
                                        let icon_size = 48.0;
                                        let icon_rect = egui::Rect::from_center_size(
                                            rect.center() - egui::vec2(0.0, 8.0),
                                            egui::vec2(icon_size, icon_size),
                                        );
                                        ui.painter().image(
                                            icon.id(),
                                            icon_rect,
                                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                            egui::Color32::WHITE,
                                        );
                                    }

                                    // Folder name (bottom, truncated)
                                    let name = if folder.name.len() > 12 {
                                        format!("{}...", &folder.name[..9])
                                    } else {
                                        folder.name.clone()
                                    };
                                    ui.painter().text(
                                        rect.center() + egui::vec2(0.0, 20.0),
                                        egui::Align2::CENTER_CENTER,
                                        name,
                                        egui::FontId::proportional(10.0),
                                        egui::Color32::WHITE,
                                    );

                                    // Item count
                                    ui.painter().text(
                                        rect.center() + egui::vec2(0.0, 32.0),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{} items", folder.item_count),
                                        egui::FontId::proportional(9.0),
                                        egui::Color32::from_rgb(150, 150, 150),
                                    );

                                    // Handle drop: move asset to folder
                                    if is_drop_hover && ui.input(|i| i.pointer.any_released()) {
                                        if let Some(ref drag) = shared.dragging_asset.clone() {
                                            let core_category = Self::drag_clip_type_to_core_category(drag.clip_type);
                                            let action = lightningbeam_core::actions::MoveAssetToFolderAction::new(
                                                core_category,
                                                drag.clip_id,
                                                Some(folder.id),
                                            );
                                            let _ = shared.action_executor.execute(Box::new(action));
                                            *shared.dragging_asset = None;
                                        }
                                    }

                                    // Handle double-click to navigate into folder
                                    if response.double_clicked() {
                                        self.set_current_folder(Some(folder.id));
                                    }

                                    // Handle right-click for context menu
                                    if response.secondary_clicked() {
                                        self.folder_context_menu = Some(FolderContextMenuState {
                                            folder_id: folder.id,
                                            position: ui.ctx().pointer_interact_pos().unwrap_or(egui::pos2(0.0, 0.0)),
                                        });
                                    }
                                }
                                LibraryItem::Asset(asset) => {
                                    // Allocate rect for asset grid item (with space for name below)
                                    let (item_rect, _response) = ui.allocate_exact_size(
                                        egui::vec2(GRID_ITEM_SIZE, GRID_ITEM_SIZE + 20.0),
                                        egui::Sense::hover(),
                                    );
                                    self.render_single_asset_grid(ui, asset, item_rect, document, shared);
                                }
                            }
                        }
                    });
                    ui.add_space(GRID_SPACING);
                }
            });
        });
    }

    /// Helper to render a single asset in list view
    fn render_single_asset_list(
        &mut self,
        ui: &mut egui::Ui,
        asset: &AssetEntry,
        item_rect: egui::Rect,
        document: &Document,
        shared: &mut SharedPaneState,
    ) -> egui::Response {
        let response = ui.allocate_rect(item_rect, egui::Sense::click_and_drag());

        let is_selected = self.selected_asset == Some(asset.id);
        let is_being_dragged = shared
            .dragging_asset
            .as_ref()
            .map(|d| d.clip_id == asset.id)
            .unwrap_or(false);

        // Text colors
        let text_color = egui::Color32::from_gray(200);
        let secondary_text_color = egui::Color32::from_gray(120);

        // Item background
        let item_bg = if is_being_dragged {
            egui::Color32::from_rgb(80, 100, 120)
        } else if is_selected {
            egui::Color32::from_rgb(60, 80, 100)
        } else if response.hovered() {
            egui::Color32::from_rgb(45, 45, 45)
        } else {
            egui::Color32::from_rgb(35, 35, 35)
        };
        ui.painter().rect_filled(item_rect, 3.0, item_bg);

        // Category color indicator bar
        let indicator_color = asset.category.color();
        let indicator_rect = egui::Rect::from_min_size(
            item_rect.min,
            egui::vec2(4.0, ITEM_HEIGHT),
        );
        ui.painter().rect_filled(indicator_rect, 0.0, indicator_color);

        // Asset name
        ui.painter().text(
            item_rect.min + egui::vec2(12.0, 8.0),
            egui::Align2::LEFT_TOP,
            &asset.name,
            egui::FontId::proportional(13.0),
            text_color,
        );

        // Metadata line
        let metadata = if asset.category == AssetCategory::Images {
            asset.extra_info.clone()
        } else if let Some((w, h)) = asset.dimensions {
            format!(
                "{:.1}s | {}x{} | {}",
                asset.duration, w as u32, h as u32, asset.extra_info
            )
        } else {
            format!("{:.1}s | {}", asset.duration, asset.extra_info)
        };

        ui.painter().text(
            item_rect.min + egui::vec2(12.0, 24.0),
            egui::Align2::LEFT_TOP,
            &metadata,
            egui::FontId::proportional(10.0),
            secondary_text_color,
        );

        // Thumbnail on the right side
        let thumbnail_rect = egui::Rect::from_min_size(
            egui::pos2(
                item_rect.max.x - LIST_THUMBNAIL_SIZE - 4.0,
                item_rect.min.y + (ITEM_HEIGHT - LIST_THUMBNAIL_SIZE) / 2.0,
            ),
            egui::vec2(LIST_THUMBNAIL_SIZE, LIST_THUMBNAIL_SIZE),
        );

        // Generate and display thumbnail
        let asset_id = asset.id;
        let asset_category = asset.category;
        let ctx = ui.ctx().clone();

        let texture = self.thumbnail_cache.get_or_create(&ctx, asset_id, || {
            match asset_category {
                AssetCategory::Images => {
                    document.image_assets.get(&asset_id)
                        .and_then(generate_image_thumbnail)
                }
                AssetCategory::Vector => {
                    let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                    document.vector_clips.get(&asset_id)
                        .map(|clip| generate_vector_thumbnail(clip, bg_color))
                }
                AssetCategory::Video => {
                    generate_video_thumbnail(&asset_id, &shared.video_manager)
                        .or_else(|| Some(generate_placeholder_thumbnail(AssetCategory::Video, 200)))
                }
                AssetCategory::Audio => {
                    if let Some(clip) = document.audio_clips.get(&asset_id) {
                        let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                        match &clip.clip_type {
                            AudioClipType::Sampled { audio_pool_index } => {
                                let wave_color = egui::Color32::from_rgb(100, 200, 100);
                                let waveform: Option<Vec<(f32, f32)>> = shared.raw_audio_cache.get(audio_pool_index)
                                    .map(|raw| peaks_from_raw_audio(raw, THUMBNAIL_SIZE as usize));
                                if let Some(ref peaks) = waveform {
                                    Some(generate_waveform_thumbnail(peaks, bg_color, wave_color))
                                } else {
                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                }
                            }
                            AudioClipType::Midi { midi_clip_id } => {
                                let note_color = egui::Color32::from_rgb(100, 200, 100);
                                if let Some(events) = shared.midi_event_cache.get(midi_clip_id) {
                                    Some(generate_midi_thumbnail(events, clip.duration, bg_color, note_color))
                                } else {
                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                }
                            }
                            AudioClipType::Recording => {
                                Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                            }
                        }
                    } else {
                        None
                    }
                }
                AssetCategory::Effects => {
                    if let Some(rgba) = shared.effect_thumbnail_cache.get(&asset_id) {
                        Some(rgba.clone())
                    } else {
                        shared.effect_thumbnail_requests.push(asset_id);
                        None
                    }
                }
                AssetCategory::All => None,
            }
        });

        if let Some(texture) = texture {
            ui.painter().image(
                texture.id(),
                thumbnail_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Handle drag start
        if response.drag_started() {
            let linked_audio_clip_id = if asset_category == AssetCategory::Video {
                document.video_clips.get(&asset_id)
                    .and_then(|clip| clip.linked_audio_clip_id)
            } else {
                None
            };

            *shared.dragging_asset = Some(DraggingAsset {
                clip_type: asset.drag_clip_type,
                clip_id: asset_id,
                name: asset.name.clone(),
                duration: asset.duration,
                dimensions: asset.dimensions,
                linked_audio_clip_id,
            });
        }

        // Handle right-click for context menu
        if response.secondary_clicked() {
            self.context_menu = Some(ContextMenuState {
                asset_id: asset.id,
                position: ui.ctx().pointer_interact_pos().unwrap_or(egui::pos2(0.0, 0.0)),
            });
        }

        // Handle selection
        if response.clicked() {
            self.selected_asset = Some(asset.id);
        }

        response
    }

    /// Helper to render a single asset in grid view
    fn render_single_asset_grid(
        &mut self,
        ui: &mut egui::Ui,
        asset: &AssetEntry,
        rect: egui::Rect,
        document: &Document,
        shared: &mut SharedPaneState,
    ) -> egui::Response {
        let response = ui.interact(rect, egui::Id::new(("grid_asset", asset.id)), egui::Sense::click_and_drag());

        let is_selected = self.selected_asset == Some(asset.id);
        let is_being_dragged = shared
            .dragging_asset
            .as_ref()
            .map(|d| d.clip_id == asset.id)
            .unwrap_or(false);

        // Background
        let bg_color = if is_being_dragged {
            egui::Color32::from_rgb(80, 100, 120)
        } else if is_selected {
            egui::Color32::from_rgb(60, 80, 100)
        } else if response.hovered() {
            egui::Color32::from_rgb(50, 50, 50)
        } else {
            egui::Color32::from_rgb(35, 35, 35)
        };
        ui.painter().rect_filled(rect, 4.0, bg_color);

        // Thumbnail
        let thumbnail_size = 64.0;
        let thumbnail_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.center().x - thumbnail_size / 2.0,
                rect.min.y + 8.0,
            ),
            egui::vec2(thumbnail_size, thumbnail_size),
        );

        let asset_id = asset.id;
        let asset_category = asset.category;
        let ctx = ui.ctx().clone();

        let texture = self.thumbnail_cache.get_or_create(&ctx, asset_id, || {
            match asset_category {
                AssetCategory::Images => {
                    document.image_assets.get(&asset_id)
                        .and_then(generate_image_thumbnail)
                }
                AssetCategory::Vector => {
                    let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                    document.vector_clips.get(&asset_id)
                        .map(|clip| generate_vector_thumbnail(clip, bg_color))
                }
                AssetCategory::Video => {
                    generate_video_thumbnail(&asset_id, &shared.video_manager)
                        .or_else(|| Some(generate_placeholder_thumbnail(AssetCategory::Video, 200)))
                }
                AssetCategory::Audio => {
                    if let Some(clip) = document.audio_clips.get(&asset_id) {
                        let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                        match &clip.clip_type {
                            AudioClipType::Sampled { audio_pool_index } => {
                                let wave_color = egui::Color32::from_rgb(100, 200, 100);
                                let waveform: Option<Vec<(f32, f32)>> = shared.raw_audio_cache.get(audio_pool_index)
                                    .map(|raw| peaks_from_raw_audio(raw, THUMBNAIL_SIZE as usize));
                                if let Some(ref peaks) = waveform {
                                    Some(generate_waveform_thumbnail(peaks, bg_color, wave_color))
                                } else {
                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                }
                            }
                            AudioClipType::Midi { midi_clip_id } => {
                                let note_color = egui::Color32::from_rgb(100, 200, 100);
                                if let Some(events) = shared.midi_event_cache.get(midi_clip_id) {
                                    Some(generate_midi_thumbnail(events, clip.duration, bg_color, note_color))
                                } else {
                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                }
                            }
                            AudioClipType::Recording => {
                                Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                            }
                        }
                    } else {
                        None
                    }
                }
                AssetCategory::Effects => {
                    if let Some(rgba) = shared.effect_thumbnail_cache.get(&asset_id) {
                        Some(rgba.clone())
                    } else {
                        shared.effect_thumbnail_requests.push(asset_id);
                        None
                    }
                }
                AssetCategory::All => None,
            }
        });

        if let Some(texture) = texture {
            ui.painter().image(
                texture.id(),
                thumbnail_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Category indicator
        let indicator_rect = egui::Rect::from_min_size(
            egui::pos2(thumbnail_rect.min.x, thumbnail_rect.max.y - 3.0),
            egui::vec2(thumbnail_size, 3.0),
        );
        ui.painter().rect_filled(indicator_rect, 0.0, asset.category.color());

        // Asset name
        let name = if asset.name.len() > 12 {
            format!("{}...", &asset.name[..9])
        } else {
            asset.name.clone()
        };
        ui.painter().text(
            rect.center() + egui::vec2(0.0, 40.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );

        // Handle interactions
        if response.clicked() {
            self.selected_asset = Some(asset.id);
        }

        if response.secondary_clicked() {
            self.context_menu = Some(ContextMenuState {
                asset_id: asset.id,
                position: ui.ctx().pointer_interact_pos().unwrap_or(egui::pos2(0.0, 0.0)),
            });
        }

        if response.drag_started() {
            let linked_audio_clip_id = if asset_category == AssetCategory::Video {
                document.video_clips.get(&asset_id)
                    .and_then(|clip| clip.linked_audio_clip_id)
            } else {
                None
            };

            *shared.dragging_asset = Some(DraggingAsset {
                clip_type: asset.drag_clip_type,
                clip_id: asset_id,
                name: asset.name.clone(),
                duration: asset.duration,
                dimensions: asset.dimensions,
                linked_audio_clip_id,
            });
        }

        response
    }

    /// Render assets based on current view mode
    #[allow(dead_code)]
    fn render_assets(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
        assets: &[&AssetEntry],
        document: &Document,
    ) {
        match self.view_mode {
            AssetViewMode::List => {
                self.render_asset_list_view(ui, rect, path, shared, assets, document);
            }
            AssetViewMode::Grid => {
                self.render_asset_grid_view(ui, rect, path, shared, assets, document);
            }
        }
    }

    /// Render the asset list view
    #[allow(dead_code)]
    fn render_asset_list_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
        assets: &[&AssetEntry],
        document: &Document,
    ) {
        // Background
        let bg_style = shared.theme.style(".panel-content", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(25, 25, 25));
        ui.painter().rect_filled(rect, 0.0, bg_color);

        // Text colors
        let text_style = shared.theme.style(".text-primary", ui.ctx());
        let text_color = text_style
            .text_color
            .unwrap_or(egui::Color32::from_gray(200));
        let secondary_text_color = egui::Color32::from_gray(120);

        // Show empty state message if no assets
        if assets.is_empty() {
            let message = if !self.search_filter.is_empty() {
                "No assets match your search"
            } else {
                "No assets in this category"
            };

            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::FontId::proportional(14.0),
                secondary_text_color,
            );
            return;
        }

        // Use egui's built-in ScrollArea for scrolling
        let scroll_area_rect = rect;
        ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_area_rect), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("asset_list_scroll", path))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(scroll_area_rect.width() - 16.0); // Account for scrollbar

                    // For Effects tab, reorder: built-in first, then custom, with headers
                    let ordered_assets: Vec<&AssetEntry>;
                    let show_effects_sections = self.selected_category == AssetCategory::Effects;

                    let assets_to_render = if show_effects_sections {
                        let builtin: Vec<_> = assets.iter().filter(|a| a.is_builtin).copied().collect();
                        let custom: Vec<_> = assets.iter().filter(|a| !a.is_builtin).copied().collect();
                        ordered_assets = builtin.into_iter().chain(custom.into_iter()).collect();
                        &ordered_assets[..]
                    } else {
                        assets
                    };

                    // Track whether we need to render section headers
                    let builtin_count = if show_effects_sections {
                        assets.iter().filter(|a| a.is_builtin).count()
                    } else {
                        0
                    };
                    let custom_count = if show_effects_sections {
                        assets.iter().filter(|a| !a.is_builtin).count()
                    } else {
                        0
                    };
                    let mut rendered_builtin_header = false;
                    let mut rendered_custom_header = false;
                    let mut _builtin_rendered = 0;

                    for asset in assets_to_render {
                        // Render section headers for Effects tab
                        if show_effects_sections {
                            if asset.is_builtin && !rendered_builtin_header && builtin_count > 0 {
                                Self::render_section_header(ui, "Built-in Effects", secondary_text_color);
                                rendered_builtin_header = true;
                            }
                            if !asset.is_builtin && !rendered_custom_header && custom_count > 0 {
                                // Add separator before custom section if there were built-in effects
                                if builtin_count > 0 {
                                    ui.add_space(8.0);
                                    let separator_rect = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 1.0),
                                        egui::Sense::hover(),
                                    ).0;
                                    ui.painter().rect_filled(separator_rect, 0.0, egui::Color32::from_gray(60));
                                    ui.add_space(8.0);
                                }
                                Self::render_section_header(ui, "Custom Effects", secondary_text_color);
                                rendered_custom_header = true;
                            }
                            if asset.is_builtin {
                                _builtin_rendered += 1;
                            }
                        }

                        let (item_rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ITEM_HEIGHT),
                            egui::Sense::click_and_drag(),
                        );

                        let is_selected = self.selected_asset == Some(asset.id);
                        let is_being_dragged = shared
                            .dragging_asset
                            .as_ref()
                            .map(|d| d.clip_id == asset.id)
                            .unwrap_or(false);

                        // Item background
                        let item_bg = if is_being_dragged {
                            egui::Color32::from_rgb(80, 100, 120) // Highlight when dragging
                        } else if is_selected {
                            egui::Color32::from_rgb(60, 80, 100)
                        } else if response.hovered() {
                            egui::Color32::from_rgb(45, 45, 45)
                        } else {
                            egui::Color32::from_rgb(35, 35, 35)
                        };
                        ui.painter().rect_filled(item_rect, 3.0, item_bg);

                        // Category color indicator bar
                        let indicator_color = asset.category.color();
                        let indicator_rect = egui::Rect::from_min_size(
                            item_rect.min,
                            egui::vec2(4.0, ITEM_HEIGHT),
                        );
                        ui.painter().rect_filled(indicator_rect, 0.0, indicator_color);

                        // Asset name (or inline edit field)
                        let is_renaming = self.rename_state.as_ref().map(|s| s.asset_id == asset.id).unwrap_or(false);

                        if is_renaming {
                            // Inline rename text field using IME-safe widget
                            let name_rect = egui::Rect::from_min_size(
                                item_rect.min + egui::vec2(10.0, 4.0),
                                egui::vec2(item_rect.width() - 20.0, 18.0),
                            );

                            if let Some(ref mut state) = self.rename_state {
                                let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
                                ImeTextField::new(&mut state.edit_text)
                                    .font_size(13.0)
                                    .desired_width(name_rect.width())
                                    .request_focus()
                                    .show(&mut child_ui);
                            }
                        } else {
                            // Normal asset name display
                            ui.painter().text(
                                item_rect.min + egui::vec2(12.0, 8.0),
                                egui::Align2::LEFT_TOP,
                                &asset.name,
                                egui::FontId::proportional(13.0),
                                text_color,
                            );
                        }

                        // Metadata line (images don't have duration)
                        let metadata = if asset.category == AssetCategory::Images {
                            // For images, just show dimensions
                            asset.extra_info.clone()
                        } else if let Some((w, h)) = asset.dimensions {
                            format!(
                                "{:.1}s | {}x{} | {}",
                                asset.duration, w as u32, h as u32, asset.extra_info
                            )
                        } else {
                            format!("{:.1}s | {}", asset.duration, asset.extra_info)
                        };

                        ui.painter().text(
                            item_rect.min + egui::vec2(12.0, 24.0),
                            egui::Align2::LEFT_TOP,
                            &metadata,
                            egui::FontId::proportional(10.0),
                            secondary_text_color,
                        );

                        // Thumbnail on the right side
                        let thumbnail_rect = egui::Rect::from_min_size(
                            egui::pos2(
                                item_rect.max.x - LIST_THUMBNAIL_SIZE - 4.0,
                                item_rect.min.y + (ITEM_HEIGHT - LIST_THUMBNAIL_SIZE) / 2.0,
                            ),
                            egui::vec2(LIST_THUMBNAIL_SIZE, LIST_THUMBNAIL_SIZE),
                        );

                        // Generate and display thumbnail based on asset type
                        let asset_id = asset.id;
                        let asset_category = asset.category;
                        let ctx = ui.ctx().clone();

                        // Get waveform data from cache if thumbnail not already cached
                        let prefetched_waveform: Option<Vec<(f32, f32)>> =
                            if asset_category == AssetCategory::Audio && !self.thumbnail_cache.has(&asset_id) {
                                if let Some(clip) = document.audio_clips.get(&asset_id) {
                                    if let AudioClipType::Sampled { audio_pool_index } = &clip.clip_type {
                                        let waveform: Option<Vec<(f32, f32)>> = shared.raw_audio_cache.get(audio_pool_index)
                                            .map(|raw| peaks_from_raw_audio(raw, THUMBNAIL_SIZE as usize));
                                        if waveform.is_some() {
                                            println!("🎵 Found waveform for pool {} (asset {})", audio_pool_index, asset_id);
                                        } else {
                                            println!("⚠️  No waveform yet for pool {} (asset {})", audio_pool_index, asset_id);
                                        }
                                        waveform
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                        let texture = self.thumbnail_cache.get_or_create(&ctx, asset_id, || {
                            match asset_category {
                                AssetCategory::Images => {
                                    document.image_assets.get(&asset_id)
                                        .and_then(generate_image_thumbnail)
                                }
                                AssetCategory::Vector => {
                                    // Render frame 0 of vector clip using tiny-skia
                                    let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                                    document.vector_clips.get(&asset_id)
                                        .map(|clip| generate_vector_thumbnail(clip, bg_color))
                                }
                                AssetCategory::Video => {
                                    // Generate video thumbnail from first frame
                                    generate_video_thumbnail(&asset_id, &shared.video_manager)
                                        .or_else(|| Some(generate_placeholder_thumbnail(AssetCategory::Video, 200)))
                                }
                                AssetCategory::Audio => {
                                    // Check if it's sampled or MIDI
                                    if let Some(clip) = document.audio_clips.get(&asset_id) {
                                        let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                                        match &clip.clip_type {
                                            AudioClipType::Sampled { .. } => {
                                                let wave_color = egui::Color32::from_rgb(100, 200, 100);
                                                if let Some(ref peaks) = prefetched_waveform {
                                                    println!("✅ Generating waveform thumbnail with {} peaks for asset {}", peaks.len(), asset_id);
                                                    Some(generate_waveform_thumbnail(peaks, bg_color, wave_color))
                                                } else {
                                                    println!("📦 Generating placeholder thumbnail for asset {}", asset_id);
                                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                                }
                                            }
                                            AudioClipType::Midi { midi_clip_id } => {
                                                let bg_color = egui::Color32::from_rgba_unmultiplied(40, 40, 40, 200);
                                                let note_color = egui::Color32::from_rgb(100, 200, 100);

                                                if let Some(events) = shared.midi_event_cache.get(midi_clip_id) {
                                                    Some(generate_midi_thumbnail(events, clip.duration, bg_color, note_color))
                                                } else {
                                                    Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                                }
                                            }
                                            AudioClipType::Recording => {
                                                Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                            }
                                        }
                                    } else {
                                        Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                    }
                                }
                                AssetCategory::Effects => {
                                    // Use GPU-rendered effect thumbnail if available
                                    if let Some(rgba) = shared.effect_thumbnail_cache.get(&asset.id) {
                                        Some(rgba.clone())
                                    } else {
                                        // Request GPU thumbnail generation
                                        shared.effect_thumbnail_requests.push(asset.id);
                                        // Return None to avoid caching placeholder - will retry next frame
                                        None
                                    }
                                }
                                AssetCategory::All => None,
                            }
                        });

                        // Either use cached texture or render placeholder directly for effects
                        if let Some(texture) = texture {
                            let image = egui::Image::new(texture)
                                .fit_to_exact_size(egui::vec2(LIST_THUMBNAIL_SIZE, LIST_THUMBNAIL_SIZE));
                            ui.put(thumbnail_rect, image);
                        } else if asset.category == AssetCategory::Effects {
                            // Render effect placeholder directly (not cached) until GPU thumbnail ready
                            let placeholder_rgba = generate_effect_thumbnail();
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize],
                                &placeholder_rgba,
                            );
                            let texture = ctx.load_texture(
                                format!("effect_placeholder_{}", asset.id),
                                color_image,
                                egui::TextureOptions::LINEAR,
                            );
                            let image = egui::Image::new(&texture)
                                .fit_to_exact_size(egui::vec2(LIST_THUMBNAIL_SIZE, LIST_THUMBNAIL_SIZE));
                            ui.put(thumbnail_rect, image);
                        }

                        // Handle click (selection)
                        if response.clicked() {
                            self.selected_asset = Some(asset.id);
                        }

                        // Handle right-click (context menu)
                        if response.secondary_clicked() {
                            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                                self.context_menu = Some(ContextMenuState {
                                    asset_id: asset.id,
                                    position: pos,
                                });
                            }
                        }

                        // Handle double-click
                        if response.double_clicked() {
                            // For effects, open in shader editor
                            if asset.category == AssetCategory::Effects {
                                *shared.effect_to_load = Some(asset.id);
                            } else if !asset.is_builtin {
                                // For other non-builtin assets, start rename
                                self.rename_state = Some(RenameState {
                                    asset_id: asset.id,
                                    category: asset.category,
                                    edit_text: asset.name.clone(),
                                });
                            }
                        }

                        // Handle drag start
                        if response.drag_started() {
                            // For video clips, get the linked audio clip ID
                            let linked_audio_clip_id = if asset.drag_clip_type == DragClipType::Video {
                                let result = document.video_clips.get(&asset.id)
                                    .and_then(|video| video.linked_audio_clip_id);
                                eprintln!("DEBUG DRAG: Video clip {} has linked audio: {:?}", asset.id, result);
                                result
                            } else {
                                None
                            };

                            *shared.dragging_asset = Some(DraggingAsset {
                                clip_id: asset.id,
                                clip_type: asset.drag_clip_type,
                                name: asset.name.clone(),
                                duration: asset.duration,
                                dimensions: asset.dimensions,
                                linked_audio_clip_id,
                            });
                        }

                        // Add small spacing between items
                        ui.add_space(ITEM_PADDING);
                    }
                });
        });

        // Draw drag preview at cursor when dragging
        if let Some(dragging) = shared.dragging_asset.as_ref() {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                // Draw a semi-transparent preview
                let preview_rect = egui::Rect::from_min_size(
                    pos + egui::vec2(10.0, 10.0), // Offset from cursor
                    egui::vec2(150.0, 30.0),
                );

                // Use top layer for drag preview
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("drag_preview"),
                ));

                painter.rect_filled(
                    preview_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(60, 60, 60, 220),
                );

                painter.text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &dragging.name,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Clear drag state when mouse is released (if not dropped on valid target)
        // Note: Valid drop targets (Timeline, Stage) will clear this themselves
        if ui.input(|i| i.pointer.any_released()) {
            // Only clear if we're still within this pane (dropped back on library)
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                if rect.contains(pos) {
                    *shared.dragging_asset = None;
                }
            }
        }
    }

    /// Render the asset grid view
    #[allow(dead_code)]
    fn render_asset_grid_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
        assets: &[&AssetEntry],
        document: &Document,
    ) {
        // Background
        let bg_style = shared.theme.style(".panel-content", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(25, 25, 25));
        ui.painter().rect_filled(rect, 0.0, bg_color);

        // Text color
        let text_style = shared.theme.style(".text-primary", ui.ctx());
        let text_color = text_style
            .text_color
            .unwrap_or(egui::Color32::from_gray(200));
        let secondary_text_color = egui::Color32::from_gray(120);

        // Show empty state message if no assets
        if assets.is_empty() {
            let message = if !self.search_filter.is_empty() {
                "No assets match your search"
            } else {
                "No assets in this category"
            };

            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::FontId::proportional(14.0),
                secondary_text_color,
            );
            return;
        }

        // Calculate grid layout
        let content_width = rect.width() - 16.0; // Account for scrollbar
        let columns = ((content_width + GRID_SPACING) / (GRID_ITEM_SIZE + GRID_SPACING))
            .floor()
            .max(1.0) as usize;
        let item_height = GRID_ITEM_SIZE + 20.0; // 20 for name below thumbnail

        // For Effects tab, reorder: built-in first, then custom
        let ordered_assets: Vec<&AssetEntry>;
        let show_effects_sections = self.selected_category == AssetCategory::Effects;

        let assets_to_render: &[&AssetEntry] = if show_effects_sections {
            let builtin: Vec<_> = assets.iter().filter(|a| a.is_builtin).copied().collect();
            let custom: Vec<_> = assets.iter().filter(|a| !a.is_builtin).copied().collect();
            ordered_assets = builtin.into_iter().chain(custom.into_iter()).collect();
            &ordered_assets[..]
        } else {
            assets
        };

        let builtin_count = if show_effects_sections {
            assets.iter().filter(|a| a.is_builtin).count()
        } else {
            0
        };
        let custom_count = if show_effects_sections {
            assets.iter().filter(|a| !a.is_builtin).count()
        } else {
            0
        };

        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("asset_grid_scroll", path))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(content_width);

                    // Render built-in section header
                    if show_effects_sections && builtin_count > 0 {
                        Self::render_section_header(ui, "Built-in Effects", secondary_text_color);
                    }

                    // First pass: render built-in items
                    let builtin_items: Vec<_> = assets_to_render.iter().filter(|a| a.is_builtin).copied().collect();
                    if !builtin_items.is_empty() {
                        self.render_grid_items(ui, &builtin_items, columns, item_height, content_width, shared, document, text_color, secondary_text_color);
                    }

                    // Separator between sections
                    if show_effects_sections && builtin_count > 0 && custom_count > 0 {
                        ui.add_space(8.0);
                        let separator_rect = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        ).0;
                        ui.painter().rect_filled(separator_rect, 0.0, egui::Color32::from_gray(60));
                        ui.add_space(8.0);
                    }

                    // Render custom section header
                    if show_effects_sections && custom_count > 0 {
                        Self::render_section_header(ui, "Custom Effects", secondary_text_color);
                    }

                    // Second pass: render custom items
                    let custom_items: Vec<_> = assets_to_render.iter().filter(|a| !a.is_builtin).copied().collect();
                    if !custom_items.is_empty() {
                        self.render_grid_items(ui, &custom_items, columns, item_height, content_width, shared, document, text_color, secondary_text_color);
                    }

                    // For non-Effects tabs, just render all items
                    if !show_effects_sections {
                        self.render_grid_items(ui, assets_to_render, columns, item_height, content_width, shared, document, text_color, secondary_text_color);
                    }
                });
        });

        // Draw drag preview at cursor when dragging
        if let Some(dragging) = shared.dragging_asset.as_ref() {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                let preview_rect = egui::Rect::from_min_size(
                    pos + egui::vec2(10.0, 10.0),
                    egui::vec2(150.0, 30.0),
                );

                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("drag_preview"),
                ));

                painter.rect_filled(
                    preview_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(60, 60, 60, 220),
                );

                painter.text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &dragging.name,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Clear drag state when mouse is released
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                if rect.contains(pos) {
                    *shared.dragging_asset = None;
                }
            }
        }
    }
}

impl PaneRenderer for AssetLibraryPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Get an Arc clone of the document for thumbnail generation
        // This allows us to pass &mut shared to render functions while still accessing document
        let document_arc = shared.action_executor.document_arc();

        // Invalidate thumbnails for audio clips that got new waveform data
        if !shared.audio_pools_with_new_waveforms.is_empty() {
            println!("🎨 [ASSET_LIB] Checking for thumbnails to invalidate (pools: {:?})", shared.audio_pools_with_new_waveforms);
            let mut invalidated_any = false;
            for (asset_id, clip) in &document_arc.audio_clips {
                if let lightningbeam_core::clip::AudioClipType::Sampled { audio_pool_index } = &clip.clip_type {
                    if shared.audio_pools_with_new_waveforms.contains(audio_pool_index) {
                        println!("❌ [ASSET_LIB] Invalidating thumbnail for asset {} (pool {})", asset_id, audio_pool_index);
                        self.thumbnail_cache.invalidate(asset_id);
                        invalidated_any = true;
                    }
                }
            }
            // Force a repaint if we invalidated any thumbnails
            if invalidated_any {
                println!("🔄 [ASSET_LIB] Requesting repaint after invalidating thumbnails");
                ui.ctx().request_repaint();
            }
        }

        // Invalidate thumbnails for effects that were edited (shader code changed)
        if !shared.effect_thumbnails_to_invalidate.is_empty() {
            for effect_id in shared.effect_thumbnails_to_invalidate.iter() {
                self.thumbnail_cache.invalidate(effect_id);
            }
            // Clear after processing - we've handled these
            shared.effect_thumbnails_to_invalidate.clear();
            ui.ctx().request_repaint();
        }

        // Collect items (folders and assets)
        let all_items = self.collect_items(&document_arc);

        // Filter items by search text
        let search_lower = self.search_filter.to_lowercase();
        let filtered_items: Vec<&LibraryItem> = all_items
            .iter()
            .filter(|item| {
                if search_lower.is_empty() {
                    true
                } else {
                    item.name().to_lowercase().contains(&search_lower)
                }
            })
            .collect();

        // Layout: Search bar -> Category tabs -> Breadcrumbs -> Asset list
        let search_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), SEARCH_BAR_HEIGHT));

        let tabs_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT),
            egui::vec2(rect.width(), CATEGORY_TAB_HEIGHT),
        );

        let breadcrumb_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT + CATEGORY_TAB_HEIGHT),
            egui::vec2(rect.width(), BREADCRUMB_HEIGHT),
        );

        let list_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT + CATEGORY_TAB_HEIGHT + BREADCRUMB_HEIGHT),
            rect.max,
        );

        // Render components
        self.render_search_bar(ui, search_rect, shared);
        self.render_category_tabs(ui, tabs_rect, shared);
        self.render_breadcrumbs(ui, breadcrumb_rect, &document_arc, shared);
        self.render_items(ui, list_rect, path, shared, &filtered_items, &document_arc);

        // Detect right-click on pane background (not on items)
        // Only allow folder creation in categories with folder support (not "All")
        // Don't trigger if we already opened a folder or asset context menu
        if self.selected_category != AssetCategory::All {
            if ui.input(|i| i.pointer.secondary_clicked()) {
                if self.folder_context_menu.is_none() && self.context_menu.is_none() {
                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                        if list_rect.contains(pos) {
                            self.pane_context_menu = Some(pos);
                        }
                    }
                }
            }
        }

        // Context menu handling
        if let Some(ref context_state) = self.context_menu.clone() {
            let context_asset_id = context_state.asset_id;
            let menu_pos = context_state.position;

            // Find the asset info from all_items
            let asset_opt = all_items.iter().find_map(|item| {
                match item {
                    LibraryItem::Asset(asset) if asset.id == context_asset_id => Some(asset),
                    _ => None,
                }
            });

            if let Some(asset) = asset_opt {
                let asset_name = asset.name.clone();
                let asset_category = asset.category;
                let asset_is_builtin = asset.is_builtin;
                let asset_folder_id = asset.folder_id;
                let in_use = Self::is_asset_in_use(
                    shared.action_executor.document(),
                    context_asset_id,
                    asset_category,
                );

                // Get folders for this category (for Move to Folder submenu)
                let folders: Vec<(Uuid, String)> = if let Some(core_cat) = Self::to_core_category(asset_category) {
                    let tree = document_arc.get_folder_tree(core_cat);
                    tree.folders.iter()
                        .map(|(id, f)| (*id, f.name.clone()))
                        .collect()
                } else {
                    Vec::new()
                };

                // Show context menu popup at the stored position
                let menu_id = egui::Id::new("asset_context_menu");
                let menu_response = egui::Area::new(menu_id)
                    .order(egui::Order::Foreground)
                    .fixed_pos(menu_pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(120.0);

                            // Add "Edit in Shader Editor" for effects
                            if asset_category == AssetCategory::Effects {
                                if ui.button("Edit in Shader Editor").clicked() {
                                    *shared.effect_to_load = Some(context_asset_id);
                                    self.context_menu = None;
                                }
                                ui.separator();
                            }

                            // Built-in effects cannot be renamed or deleted
                            if asset_is_builtin {
                                ui.label(egui::RichText::new("Built-in effect")
                                    .color(egui::Color32::from_gray(120))
                                    .italics());
                            } else {
                                if ui.button("Rename").clicked() {
                                    // Start inline rename
                                    self.rename_state = Some(RenameState {
                                        asset_id: context_asset_id,
                                        category: asset_category,
                                        edit_text: asset_name.clone(),
                                    });
                                    self.context_menu = None;
                                }

                                if ui.button("Delete").clicked() {
                                    // Set up pending delete confirmation
                                    self.pending_delete = Some(PendingDelete {
                                        asset_id: context_asset_id,
                                        asset_name: asset_name.clone(),
                                        category: asset_category,
                                        in_use,
                                    });
                                    self.context_menu = None;
                                }

                                // Move to Folder submenu (only show if there are folders or asset is not at root)
                                if !folders.is_empty() || asset_folder_id.is_some() {
                                    ui.separator();
                                    ui.menu_button("Move to Folder", |ui| {
                                        // Move to Root option (if not already at root)
                                        if asset_folder_id.is_some() {
                                            if ui.button("Root").clicked() {
                                                if let Some(core_cat) = Self::to_core_category(asset_category) {
                                                    let action = lightningbeam_core::actions::MoveAssetToFolderAction::new(
                                                        core_cat,
                                                        context_asset_id,
                                                        None,
                                                    );
                                                    let _ = shared.action_executor.execute(Box::new(action));
                                                }
                                                self.context_menu = None;
                                            }
                                            if !folders.is_empty() {
                                                ui.separator();
                                            }
                                        }

                                        // List all folders (except current folder)
                                        for (folder_id, folder_name) in &folders {
                                            if asset_folder_id != Some(*folder_id) {
                                                if ui.button(folder_name).clicked() {
                                                    if let Some(core_cat) = Self::to_core_category(asset_category) {
                                                        let action = lightningbeam_core::actions::MoveAssetToFolderAction::new(
                                                            core_cat,
                                                            context_asset_id,
                                                            Some(*folder_id),
                                                        );
                                                        let _ = shared.action_executor.execute(Box::new(action));
                                                    }
                                                    self.context_menu = None;
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        });
                    });

                // Close menu on click outside (using primary button release)
                let menu_rect = menu_response.response.rect;
                if ui.input(|i| i.pointer.primary_released()) {
                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                        if !menu_rect.contains(pos) {
                            self.context_menu = None;
                        }
                    }
                }

                // Also close on Escape
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.context_menu = None;
                }
            } else {
                self.context_menu = None;
            }
        }

        // Pane context menu (for creating folders)
        if let Some(menu_pos) = self.pane_context_menu {
            let menu_id = egui::Id::new("pane_context_menu");
            let menu_response = egui::Area::new(menu_id)
                .order(egui::Order::Foreground)
                .fixed_pos(menu_pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(150.0);

                        if ui.button("New Folder").clicked() {
                            // Get the current folder for this category
                            let parent_folder_id = self.get_current_folder();

                            // Get the core category
                            if let Some(core_category) = Self::to_core_category(self.selected_category) {
                                // Create folder action
                                let action = lightningbeam_core::actions::CreateFolderAction::new(
                                    core_category,
                                    "New Folder",
                                    parent_folder_id,
                                );

                                if shared.action_executor.execute(Box::new(action)).is_ok() {
                                    // Successfully created folder
                                }
                            }

                            self.pane_context_menu = None;
                        }
                    })
                });

            // Close menu on click outside (using primary button release to avoid first-frame issue)
            let menu_rect = menu_response.response.rect;
            if ui.input(|i| i.pointer.primary_released()) {
                if let Some(pos) = ui.ctx().pointer_interact_pos() {
                    if !menu_rect.contains(pos) {
                        self.pane_context_menu = None;
                    }
                }
            }

            // Also close on Escape
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.pane_context_menu = None;
            }
        }

        // Folder context menu (for rename/delete/etc)
        if let Some(ref folder_state) = self.folder_context_menu.clone() {
            let folder_id = folder_state.folder_id;
            let menu_pos = folder_state.position;

            // Get the folder from the document
            if let Some(core_category) = Self::to_core_category(self.selected_category) {
                let folder_tree = document_arc.get_folder_tree(core_category);

                if let Some(folder) = folder_tree.folders.get(&folder_id) {
                    let folder_name = folder.name.clone();

                    let menu_id = egui::Id::new("folder_context_menu");
                    let menu_response = egui::Area::new(menu_id)
                        .order(egui::Order::Foreground)
                        .fixed_pos(menu_pos)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(150.0);

                                if ui.button("Rename").clicked() {
                                    // Enter rename mode for folder
                                    self.folder_rename_state = Some(FolderRenameState {
                                        folder_id,
                                        category: self.selected_category,
                                        edit_text: folder_name.clone(),
                                    });
                                    self.folder_context_menu = None;
                                }

                                if ui.button("Delete").clicked() {
                                    // Execute delete folder action
                                    let action = lightningbeam_core::actions::DeleteFolderAction::new(
                                        core_category,
                                        folder_id,
                                        lightningbeam_core::actions::DeleteStrategy::MoveToParent,
                                    );

                                    let _ = shared.action_executor.execute(Box::new(action));
                                    self.folder_context_menu = None;
                                }
                            })
                        });

                    // Close menu on click outside (using primary button release to avoid first-frame issue)
                    let menu_rect = menu_response.response.rect;
                    if ui.input(|i| i.pointer.primary_released()) {
                        if let Some(pos) = ui.ctx().pointer_interact_pos() {
                            if !menu_rect.contains(pos) {
                                self.folder_context_menu = None;
                            }
                        }
                    }

                    // Also close on Escape
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.folder_context_menu = None;
                    }
                } else {
                    self.folder_context_menu = None;
                }
            } else {
                self.folder_context_menu = None;
            }
        }

        // Draw drag preview at cursor when dragging an asset
        if let Some(dragging) = shared.dragging_asset.as_ref() {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                // Draw a semi-transparent preview near the cursor
                let preview_rect = egui::Rect::from_min_size(
                    pos + egui::vec2(12.0, 12.0), // Offset from cursor
                    egui::vec2(160.0, 32.0),
                );

                // Use top layer for drag preview so it appears above everything
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("asset_drag_preview"),
                ));

                // Background with rounded corners
                painter.rect_filled(
                    preview_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(50, 80, 120, 230),
                );

                // Border
                painter.rect_stroke(
                    preview_rect,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 160, 220)),
                    egui::StrokeKind::Inside,
                );

                // Asset name
                painter.text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &dragging.name,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Clear drag state when mouse is released within the asset library
        // (dropped back on library without hitting a valid folder target)
        if ui.input(|i| i.pointer.any_released()) {
            if shared.dragging_asset.is_some() {
                if let Some(pos) = ui.ctx().pointer_interact_pos() {
                    if rect.contains(pos) {
                        *shared.dragging_asset = None;
                    }
                }
            }
        }

        // Delete confirmation dialog
        if let Some(ref pending) = self.pending_delete.clone() {
            let window_id = egui::Id::new("delete_confirm_dialog");
            let mut should_close = false;
            let mut should_delete = false;

            egui::Window::new("Confirm Delete")
                .id(window_id)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(300.0);

                    if pending.in_use {
                        ui.label(egui::RichText::new("Warning: This asset is currently in use!")
                            .color(egui::Color32::from_rgb(255, 180, 100)));
                        ui.add_space(4.0);
                        ui.label("Deleting it will remove all clip instances that reference it.");
                        ui.add_space(8.0);
                    }

                    ui.label(format!("Are you sure you want to delete \"{}\"?", pending.asset_name));
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let delete_text = if pending.in_use { "Delete Anyway" } else { "Delete" };
                            if ui.button(delete_text).clicked() {
                                should_delete = true;
                                should_close = true;
                            }
                        });
                    });
                });

            if should_delete {
                // Perform the delete
                Self::delete_asset(
                    shared.action_executor.document_mut(),
                    pending.asset_id,
                    pending.category,
                );
            }

            if should_close {
                self.pending_delete = None;
            }
        }

        // Handle rename state (Enter to confirm, Escape to cancel, click outside to confirm)
        if let Some(ref state) = self.rename_state.clone() {
            let mut should_confirm = false;
            let mut should_cancel = false;

            // Check for Enter or Escape
            ui.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    should_confirm = true;
                } else if i.key_pressed(egui::Key::Escape) {
                    should_cancel = true;
                }
            });

            if should_confirm {
                let new_name = state.edit_text.trim();
                if !new_name.is_empty() {
                    Self::rename_asset(
                        shared.action_executor.document_mut(),
                        state.asset_id,
                        state.category,
                        new_name,
                    );
                }
                self.rename_state = None;
            } else if should_cancel {
                self.rename_state = None;
            }
        }

        // Handle folder rename state (Enter to confirm, Escape to cancel)
        if let Some(ref state) = self.folder_rename_state.clone() {
            let mut should_confirm = false;
            let mut should_cancel = false;

            // Check for Enter or Escape
            ui.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    should_confirm = true;
                } else if i.key_pressed(egui::Key::Escape) {
                    should_cancel = true;
                }
            });

            if should_confirm {
                let new_name = state.edit_text.trim();
                if !new_name.is_empty() {
                    // Execute rename folder action
                    if let Some(core_category) = Self::to_core_category(state.category) {
                        let action = lightningbeam_core::actions::RenameFolderAction::new(
                            core_category,
                            state.folder_id,
                            new_name.to_string(),
                        );
                        let _ = shared.action_executor.execute(Box::new(action));
                    }
                }
                self.folder_rename_state = None;
            } else if should_cancel {
                self.folder_rename_state = None;
            }
        }
    }

    fn name(&self) -> &str {
        "Asset Library"
    }
}

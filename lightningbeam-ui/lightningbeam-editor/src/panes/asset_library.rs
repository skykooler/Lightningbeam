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
use lightningbeam_core::shape::ShapeColor;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::{DragClipType, DraggingAsset, NodePath, PaneRenderer, SharedPaneState};
use crate::widgets::ImeTextField;

// Thumbnail constants
const THUMBNAIL_SIZE: u32 = 64;
const THUMBNAIL_PREVIEW_SECONDS: f64 = 10.0;

// Layout constants
const SEARCH_BAR_HEIGHT: f32 = 30.0;
const CATEGORY_TAB_HEIGHT: f32 = 28.0;
const ITEM_HEIGHT: f32 = 40.0;
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
    pub fn has(&self, asset_id: &Uuid) -> bool {
        self.textures.contains_key(asset_id) && !self.dirty.contains(asset_id)
    }

    /// Mark an asset's thumbnail as needing regeneration
    pub fn invalidate(&mut self, asset_id: &Uuid) {
        self.dirty.insert(*asset_id);
    }

    /// Clear all cached thumbnails
    pub fn clear(&mut self) {
        self.textures.clear();
        self.dirty.clear();
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
    let num_peaks = waveform_peaks.len().min(size);

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

/// Generate a piano roll thumbnail for MIDI clips
/// Shows notes as horizontal bars with Y position = note % 12 (one octave)
fn generate_midi_thumbnail(
    events: &[(f64, u8, bool)], // (timestamp, note_number, is_note_on)
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
    for &(timestamp, note_number, is_note_on) in events {
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
    use kurbo::PathEl;
    use tiny_skia::{Paint, PathBuilder, Pixmap, Transform as TsTransform};

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
    let scale = scale_x.min(scale_y) * 0.9; // 90% to leave a small margin

    // Center offset
    let offset_x = (THUMBNAIL_SIZE as f64 - clip.width * scale) / 2.0;
    let offset_y = (THUMBNAIL_SIZE as f64 - clip.height * scale) / 2.0;

    // Iterate through layers and render shapes
    for layer_node in clip.layers.iter() {
        if let AnyLayer::Vector(vector_layer) = &layer_node.data {
            // Render each shape instance
            for shape_instance in &vector_layer.shape_instances {
                if let Some(shape) = vector_layer.shapes.get(&shape_instance.shape_id) {
                    // Get the path (frame 0)
                    let kurbo_path = shape.path();

                    // Convert kurbo BezPath to tiny-skia PathBuilder
                    let mut path_builder = PathBuilder::new();
                    for el in kurbo_path.iter() {
                        match el {
                            PathEl::MoveTo(p) => {
                                let x = (p.x * scale + offset_x) as f32;
                                let y = (p.y * scale + offset_y) as f32;
                                path_builder.move_to(x, y);
                            }
                            PathEl::LineTo(p) => {
                                let x = (p.x * scale + offset_x) as f32;
                                let y = (p.y * scale + offset_y) as f32;
                                path_builder.line_to(x, y);
                            }
                            PathEl::QuadTo(p1, p2) => {
                                let x1 = (p1.x * scale + offset_x) as f32;
                                let y1 = (p1.y * scale + offset_y) as f32;
                                let x2 = (p2.x * scale + offset_x) as f32;
                                let y2 = (p2.y * scale + offset_y) as f32;
                                path_builder.quad_to(x1, y1, x2, y2);
                            }
                            PathEl::CurveTo(p1, p2, p3) => {
                                let x1 = (p1.x * scale + offset_x) as f32;
                                let y1 = (p1.y * scale + offset_y) as f32;
                                let x2 = (p2.x * scale + offset_x) as f32;
                                let y2 = (p2.y * scale + offset_y) as f32;
                                let x3 = (p3.x * scale + offset_x) as f32;
                                let y3 = (p3.y * scale + offset_y) as f32;
                                path_builder.cubic_to(x1, y1, x2, y2, x3, y3);
                            }
                            PathEl::ClosePath => {
                                path_builder.close();
                            }
                        }
                    }

                    if let Some(ts_path) = path_builder.finish() {
                        // Draw fill if present
                        if let Some(fill_color) = &shape.fill_color {
                            let mut paint = Paint::default();
                            paint.set_color(shape_color_to_tiny_skia(fill_color));
                            paint.anti_alias = true;
                            pixmap.fill_path(
                                &ts_path,
                                &paint,
                                tiny_skia::FillRule::Winding,
                                TsTransform::identity(),
                                None,
                            );
                        }

                        // Draw stroke if present
                        if let Some(stroke_color) = &shape.stroke_color {
                            if let Some(stroke_style) = &shape.stroke_style {
                                let mut paint = Paint::default();
                                paint.set_color(shape_color_to_tiny_skia(stroke_color));
                                paint.anti_alias = true;

                                let stroke = tiny_skia::Stroke {
                                    width: (stroke_style.width * scale) as f32,
                                    ..Default::default()
                                };

                                pixmap.stroke_path(
                                    &ts_path,
                                    &paint,
                                    &stroke,
                                    TsTransform::identity(),
                                    None,
                                );
                            }
                        }
                    }
                }
            }
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

/// Convert ShapeColor to tiny_skia Color
fn shape_color_to_tiny_skia(color: &ShapeColor) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(color.r, color.g, color.b, color.a)
}

/// Ellipsize a string to fit within a maximum character count
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
}

impl AssetCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            AssetCategory::All => "All",
            AssetCategory::Vector => "Vector",
            AssetCategory::Video => "Video",
            AssetCategory::Audio => "Audio",
            AssetCategory::Images => "Images",
        }
    }

    pub fn all() -> &'static [AssetCategory] {
        &[
            AssetCategory::All,
            AssetCategory::Vector,
            AssetCategory::Video,
            AssetCategory::Audio,
            AssetCategory::Images,
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

/// Context menu state with position
#[derive(Debug, Clone)]
struct ContextMenuState {
    asset_id: Uuid,
    position: egui::Pos2,
}

pub struct AssetLibraryPane {
    /// Current search filter text
    search_filter: String,

    /// Currently selected category tab
    selected_category: AssetCategory,

    /// Currently selected asset ID (for future drag-to-timeline)
    selected_asset: Option<Uuid>,

    /// Context menu state with position
    context_menu: Option<ContextMenuState>,

    /// Pending delete confirmation
    pending_delete: Option<PendingDelete>,

    /// Active rename state
    rename_state: Option<RenameState>,

    /// Current view mode (list or grid)
    view_mode: AssetViewMode,

    /// Thumbnail texture cache
    thumbnail_cache: ThumbnailCache,
}

impl AssetLibraryPane {
    pub fn new() -> Self {
        Self {
            search_filter: String::new(),
            selected_category: AssetCategory::All,
            selected_asset: None,
            context_menu: None,
            pending_delete: None,
            rename_state: None,
            view_mode: AssetViewMode::default(),
            thumbnail_cache: ThumbnailCache::new(),
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
            });
        }

        // Collect audio clips
        for (id, clip) in &document.audio_clips {
            let (extra_info, drag_clip_type) = match &clip.clip_type {
                AudioClipType::Sampled { .. } => ("Sampled".to_string(), DragClipType::AudioSampled),
                AudioClipType::Midi { .. } => ("MIDI".to_string(), DragClipType::AudioMidi),
            };

            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Audio,
                drag_clip_type,
                duration: clip.duration,
                dimensions: None,
                extra_info,
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
            });
        }

        // Sort alphabetically by name
        assets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        assets
    }

    /// Filter assets based on current category and search text
    fn filter_assets<'a>(&self, assets: &'a [AssetEntry]) -> Vec<&'a AssetEntry> {
        let search_lower = self.search_filter.to_lowercase();

        assets
            .iter()
            .filter(|asset| {
                // Category filter
                let category_matches = self.selected_category == AssetCategory::All
                    || asset.category == self.selected_category;

                // Search filter
                let search_matches =
                    search_lower.is_empty() || asset.name.to_lowercase().contains(&search_lower);

                category_matches && search_matches
            })
            .collect()
    }

    /// Check if an asset is currently in use (has clip instances on layers)
    fn is_asset_in_use(document: &Document, asset_id: Uuid, category: AssetCategory) -> bool {
        // Check all layers for clip instances referencing this asset
        for layer in &document.root.children {
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

    /// Render assets based on current view mode
    fn render_assets(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &mut SharedPaneState,
        assets: &[&AssetEntry],
        document: &Document,
    ) {
        match self.view_mode {
            AssetViewMode::List => {
                self.render_asset_list_view(ui, rect, shared, assets, document);
            }
            AssetViewMode::Grid => {
                self.render_asset_grid_view(ui, rect, shared, assets, document);
            }
        }
    }

    /// Render the asset list view
    fn render_asset_list_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
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
        ui.allocate_ui_at_rect(scroll_area_rect, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(scroll_area_rect.width() - 16.0); // Account for scrollbar

                    for asset in assets {
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

                        // Only pre-fetch waveform data if thumbnail not already cached
                        // (get_pool_waveform is expensive - it blocks waiting for audio thread)
                        let prefetched_waveform: Option<Vec<(f32, f32)>> =
                            if asset_category == AssetCategory::Audio && !self.thumbnail_cache.has(&asset_id) {
                                if let Some(clip) = document.audio_clips.get(&asset_id) {
                                    if let AudioClipType::Sampled { audio_pool_index } = &clip.clip_type {
                                        if let Some(audio_controller) = shared.audio_controller.as_mut() {
                                            audio_controller.get_pool_waveform(*audio_pool_index, THUMBNAIL_SIZE as usize)
                                                .ok()
                                                .map(|peaks| peaks.iter().map(|p| (p.min, p.max)).collect())
                                        } else {
                                            None
                                        }
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
                                    // Video backend not implemented yet - use placeholder
                                    Some(generate_placeholder_thumbnail(AssetCategory::Video, 200))
                                }
                                AssetCategory::Audio => {
                                    // Check if it's sampled or MIDI
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
                                            AudioClipType::Midi { events, .. } => {
                                                let note_color = egui::Color32::from_rgb(100, 150, 255);
                                                // Convert MIDI events to (timestamp, note, is_note_on) tuples
                                                // Note on: 0x90-0x9F, Note off: 0x80-0x8F
                                                let midi_events: Vec<(f64, u8, bool)> = events.iter()
                                                    .filter_map(|e| {
                                                        let msg_type = e.status & 0xF0;
                                                        let is_note_on = msg_type == 0x90 && e.data2 > 0;
                                                        let is_note_off = msg_type == 0x80 || (msg_type == 0x90 && e.data2 == 0);
                                                        if is_note_on || is_note_off {
                                                            Some((e.timestamp, e.data1, is_note_on))
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .collect();
                                                Some(generate_midi_thumbnail(&midi_events, clip.duration, bg_color, note_color))
                                            }
                                        }
                                    } else {
                                        Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                    }
                                }
                                AssetCategory::All => None,
                            }
                        });

                        if let Some(texture) = texture {
                            let image = egui::Image::new(texture)
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

                        // Handle double-click (start rename)
                        if response.double_clicked() {
                            self.rename_state = Some(RenameState {
                                asset_id: asset.id,
                                category: asset.category,
                                edit_text: asset.name.clone(),
                            });
                        }

                        // Handle drag start
                        if response.drag_started() {
                            *shared.dragging_asset = Some(DraggingAsset {
                                clip_id: asset.id,
                                clip_type: asset.drag_clip_type,
                                name: asset.name.clone(),
                                duration: asset.duration,
                                dimensions: asset.dimensions,
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
    fn render_asset_grid_view(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
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
        let rows = (assets.len() + columns - 1) / columns;
        let total_height = GRID_SPACING + rows as f32 * (item_height + GRID_SPACING);

        // Use egui's built-in ScrollArea for scrolling
        ui.allocate_ui_at_rect(rect, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Reserve space for the entire grid
                    let (grid_rect, _) = ui.allocate_exact_size(
                        egui::vec2(content_width, total_height),
                        egui::Sense::hover(),
                    );

                    for (idx, asset) in assets.iter().enumerate() {
                        let col = idx % columns;
                        let row = idx / columns;

                        // Calculate item position with proper spacing
                        let item_x = grid_rect.min.x + GRID_SPACING + col as f32 * (GRID_ITEM_SIZE + GRID_SPACING);
                        let item_y = grid_rect.min.y + GRID_SPACING + row as f32 * (item_height + GRID_SPACING);

                        let item_rect = egui::Rect::from_min_size(
                            egui::pos2(item_x, item_y),
                            egui::vec2(GRID_ITEM_SIZE, item_height),
                        );

                        // Allocate the response for this item
                        let response = ui.allocate_rect(item_rect, egui::Sense::click_and_drag());

                        let is_selected = self.selected_asset == Some(asset.id);
                        let is_being_dragged = shared
                            .dragging_asset
                            .as_ref()
                            .map(|d| d.clip_id == asset.id)
                            .unwrap_or(false);

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

                        // Thumbnail area (64x64 centered in 80px width)
                        let thumbnail_rect = egui::Rect::from_min_size(
                            egui::pos2(
                                item_rect.min.x + (GRID_ITEM_SIZE - THUMBNAIL_SIZE as f32) / 2.0,
                                item_rect.min.y + 4.0,
                            ),
                            egui::vec2(THUMBNAIL_SIZE as f32, THUMBNAIL_SIZE as f32),
                        );

                        // Generate and display thumbnail based on asset type
                        let asset_id = asset.id;
                        let asset_category = asset.category;
                        let ctx = ui.ctx().clone();

                        // Only pre-fetch waveform data if thumbnail not already cached
                        // (get_pool_waveform is expensive - it blocks waiting for audio thread)
                        let prefetched_waveform: Option<Vec<(f32, f32)>> =
                            if asset_category == AssetCategory::Audio && !self.thumbnail_cache.has(&asset_id) {
                                if let Some(clip) = document.audio_clips.get(&asset_id) {
                                    if let AudioClipType::Sampled { audio_pool_index } = &clip.clip_type {
                                        if let Some(audio_controller) = shared.audio_controller.as_mut() {
                                            audio_controller.get_pool_waveform(*audio_pool_index, THUMBNAIL_SIZE as usize)
                                                .ok()
                                                .map(|peaks| peaks.iter().map(|p| (p.min, p.max)).collect())
                                        } else {
                                            None
                                        }
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
                                    Some(generate_placeholder_thumbnail(AssetCategory::Video, 200))
                                }
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
                                            AudioClipType::Midi { events, .. } => {
                                                let note_color = egui::Color32::from_rgb(100, 150, 255);
                                                let midi_events: Vec<(f64, u8, bool)> = events.iter()
                                                    .filter_map(|e| {
                                                        let msg_type = e.status & 0xF0;
                                                        let is_note_on = msg_type == 0x90 && e.data2 > 0;
                                                        let is_note_off = msg_type == 0x80 || (msg_type == 0x90 && e.data2 == 0);
                                                        if is_note_on || is_note_off {
                                                            Some((e.timestamp, e.data1, is_note_on))
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .collect();
                                                Some(generate_midi_thumbnail(&midi_events, clip.duration, bg_color, note_color))
                                            }
                                        }
                                    } else {
                                        Some(generate_placeholder_thumbnail(AssetCategory::Audio, 200))
                                    }
                                }
                                AssetCategory::All => None,
                            }
                        });

                        if let Some(texture) = texture {
                            let image = egui::Image::new(texture)
                                .fit_to_exact_size(egui::vec2(THUMBNAIL_SIZE as f32, THUMBNAIL_SIZE as f32));
                            ui.put(thumbnail_rect, image);
                        }

                        // Category color indicator (small bar at bottom of thumbnail)
                        let indicator_rect = egui::Rect::from_min_size(
                            egui::pos2(thumbnail_rect.min.x, thumbnail_rect.max.y - 3.0),
                            egui::vec2(THUMBNAIL_SIZE as f32, 3.0),
                        );
                        ui.painter().rect_filled(indicator_rect, 0.0, asset.category.color());

                        // Asset name below thumbnail (ellipsized)
                        let name_display = ellipsize(&asset.name, 12);
                        let name_pos = egui::pos2(
                            item_rect.center().x,
                            thumbnail_rect.max.y + 8.0,
                        );
                        ui.painter().text(
                            name_pos,
                            egui::Align2::CENTER_TOP,
                            &name_display,
                            egui::FontId::proportional(10.0),
                            text_color,
                        );

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

                        // Handle double-click (start rename)
                        if response.double_clicked() {
                            self.rename_state = Some(RenameState {
                                asset_id: asset.id,
                                category: asset.category,
                                edit_text: asset.name.clone(),
                            });
                        }

                        // Handle drag start
                        if response.drag_started() {
                            *shared.dragging_asset = Some(DraggingAsset {
                                clip_id: asset.id,
                                clip_type: asset.drag_clip_type,
                                name: asset.name.clone(),
                                duration: asset.duration,
                                dimensions: asset.dimensions,
                            });
                        }
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
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Get an Arc clone of the document for thumbnail generation
        // This allows us to pass &mut shared to render functions while still accessing document
        let document_arc = shared.action_executor.document_arc();

        // Collect and filter assets
        let all_assets = self.collect_assets(&document_arc);
        let filtered_assets = self.filter_assets(&all_assets);

        // Layout: Search bar -> Category tabs -> Asset list
        let search_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), SEARCH_BAR_HEIGHT));

        let tabs_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT),
            egui::vec2(rect.width(), CATEGORY_TAB_HEIGHT),
        );

        let list_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT + CATEGORY_TAB_HEIGHT),
            rect.max,
        );

        // Render components
        self.render_search_bar(ui, search_rect, shared);
        self.render_category_tabs(ui, tabs_rect, shared);
        self.render_assets(ui, list_rect, shared, &filtered_assets, &document_arc);

        // Context menu handling
        if let Some(ref context_state) = self.context_menu.clone() {
            let context_asset_id = context_state.asset_id;
            let menu_pos = context_state.position;

            // Find the asset info
            if let Some(asset) = all_assets.iter().find(|a| a.id == context_asset_id) {
                let asset_name = asset.name.clone();
                let asset_category = asset.category;
                let in_use = Self::is_asset_in_use(
                    shared.action_executor.document(),
                    context_asset_id,
                    asset_category,
                );

                // Show context menu popup at the stored position
                let menu_id = egui::Id::new("asset_context_menu");
                let menu_response = egui::Area::new(menu_id)
                    .order(egui::Order::Foreground)
                    .fixed_pos(menu_pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(120.0);

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
    }

    fn name(&self) -> &str {
        "Asset Library"
    }
}

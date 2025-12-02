/// Timeline pane - Modern GarageBand-style timeline
///
/// Phase 1 Implementation: Time Ruler & Playhead
/// - Time-based ruler (seconds, not frames)
/// - Playhead for current time
/// - Zoom/pan controls
/// - Basic layer visualization

use eframe::egui;
use lightningbeam_core::clip::ClipInstance;
use lightningbeam_core::layer::{AnyLayer, AudioLayerType, LayerTrait};
use super::{DragClipType, NodePath, PaneRenderer, SharedPaneState};

const RULER_HEIGHT: f32 = 30.0;
const LAYER_HEIGHT: f32 = 60.0;
const LAYER_HEADER_WIDTH: f32 = 200.0;
const MIN_PIXELS_PER_SECOND: f32 = 1.0;  // Allow zooming out to see 10+ minutes
const MAX_PIXELS_PER_SECOND: f32 = 500.0;
const EDGE_DETECTION_PIXELS: f32 = 8.0; // Distance from edge to detect trim handles

/// Type of clip drag operation
#[derive(Debug, Clone, Copy, PartialEq)]
enum ClipDragType {
    Move,
    TrimLeft,
    TrimRight,
}

pub struct TimelinePane {
    /// Horizontal zoom level (pixels per second)
    pixels_per_second: f32,

    /// Horizontal scroll offset (in seconds)
    viewport_start_time: f64,

    /// Vertical scroll offset (in pixels)
    viewport_scroll_y: f32,

    /// Total duration of the animation
    duration: f64,

    /// Is the user currently dragging the playhead?
    is_scrubbing: bool,

    /// Is the user panning the timeline?
    is_panning: bool,
    last_pan_pos: Option<egui::Pos2>,

    /// Clip drag state (None if not dragging)
    clip_drag_state: Option<ClipDragType>,
    drag_offset: f64,  // Time offset being applied during drag (for preview)

    /// Cached mouse position from mousedown (used for edge detection when drag starts)
    mousedown_pos: Option<egui::Pos2>,

    /// Track if a layer control widget was clicked this frame
    layer_control_clicked: bool,
}

/// Check if a clip type can be dropped on a layer type
fn can_drop_on_layer(layer: &AnyLayer, clip_type: DragClipType) -> bool {
    match (layer, clip_type) {
        (AnyLayer::Vector(_), DragClipType::Vector) => true,
        (AnyLayer::Video(_), DragClipType::Video) => true,
        (AnyLayer::Audio(audio), DragClipType::AudioSampled) => {
            audio.audio_layer_type == AudioLayerType::Sampled
        }
        (AnyLayer::Audio(audio), DragClipType::AudioMidi) => {
            audio.audio_layer_type == AudioLayerType::Midi
        }
        _ => false,
    }
}

impl TimelinePane {
    pub fn new() -> Self {
        Self {
            pixels_per_second: 100.0,
            viewport_start_time: 0.0,
            viewport_scroll_y: 0.0,
            duration: 10.0,  // Default 10 seconds
            is_scrubbing: false,
            is_panning: false,
            last_pan_pos: None,
            clip_drag_state: None,
            drag_offset: 0.0,
            mousedown_pos: None,
            layer_control_clicked: false,
        }
    }

    /// Execute a view action with the given parameters
    /// Called from main.rs after determining this is the best handler
    pub fn execute_view_action(&mut self, action: &crate::menu::MenuAction, zoom_center: egui::Vec2) {
        use crate::menu::MenuAction;
        match action {
            MenuAction::ZoomIn => self.zoom_in(zoom_center.x),
            MenuAction::ZoomOut => self.zoom_out(zoom_center.x),
            MenuAction::ActualSize => self.actual_size(),
            MenuAction::RecenterView => self.recenter(),
            _ => {} // Not a view action we handle
        }
    }

    /// Detect which clip is under the pointer and what type of drag would occur
    ///
    /// Returns (drag_type, clip_id) if pointer is over a clip, None otherwise
    fn detect_clip_at_pointer(
        &self,
        pointer_pos: egui::Pos2,
        document: &lightningbeam_core::document::Document,
        content_rect: egui::Rect,
        header_rect: egui::Rect,
    ) -> Option<(ClipDragType, uuid::Uuid)> {
        let layer_count = document.root.children.len();

        // Check if pointer is in valid area
        if pointer_pos.y < header_rect.min.y {
            return None;
        }
        if pointer_pos.x < content_rect.min.x {
            return None;
        }

        let hover_time = self.x_to_time(pointer_pos.x - content_rect.min.x);
        let relative_y = pointer_pos.y - header_rect.min.y + self.viewport_scroll_y;
        let hovered_layer_index = (relative_y / LAYER_HEIGHT) as usize;

        if hovered_layer_index >= layer_count {
            return None;
        }

        let layers: Vec<_> = document.root.children.iter().rev().collect();
        let layer = layers.get(hovered_layer_index)?;
        let layer_data = layer.layer();

        let clip_instances = match layer {
            lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
            lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
            lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
        };

        // Check each clip instance
        for clip_instance in clip_instances {
            let clip_duration = match layer {
                lightningbeam_core::layer::AnyLayer::Vector(_) => {
                    document.get_vector_clip(&clip_instance.clip_id).map(|c| c.duration)
                }
                lightningbeam_core::layer::AnyLayer::Audio(_) => {
                    document.get_audio_clip(&clip_instance.clip_id).map(|c| c.duration)
                }
                lightningbeam_core::layer::AnyLayer::Video(_) => {
                    document.get_video_clip(&clip_instance.clip_id).map(|c| c.duration)
                }
            }?;

            let instance_duration = clip_instance.effective_duration(clip_duration);
            let instance_start = clip_instance.timeline_start;
            let instance_end = instance_start + instance_duration;

            if hover_time >= instance_start && hover_time <= instance_end {
                let start_x = self.time_to_x(instance_start);
                let end_x = self.time_to_x(instance_end);
                let mouse_x = pointer_pos.x - content_rect.min.x;

                // Determine drag type based on edge proximity (check both sides of edge)
                let drag_type = if (mouse_x - start_x).abs() <= EDGE_DETECTION_PIXELS {
                    ClipDragType::TrimLeft
                } else if (end_x - mouse_x).abs() <= EDGE_DETECTION_PIXELS {
                    ClipDragType::TrimRight
                } else {
                    ClipDragType::Move
                };

                return Some((drag_type, clip_instance.id));
            }
        }

        None
    }

    /// Zoom in by a fixed increment
    pub fn zoom_in(&mut self, center_x: f32) {
        self.apply_zoom_at_point(0.2, center_x);
    }

    /// Zoom out by a fixed increment
    pub fn zoom_out(&mut self, center_x: f32) {
        self.apply_zoom_at_point(-0.2, center_x);
    }

    /// Reset zoom to 100 pixels per second
    pub fn actual_size(&mut self) {
        self.pixels_per_second = 100.0;
    }

    /// Reset pan to start and zoom to default
    pub fn recenter(&mut self) {
        self.viewport_start_time = 0.0;
        self.viewport_scroll_y = 0.0;
        self.pixels_per_second = 100.0;
    }

    /// Apply zoom while keeping the time under the cursor stationary
    fn apply_zoom_at_point(&mut self, zoom_delta: f32, mouse_x: f32) {
        let old_zoom = self.pixels_per_second;

        // Calculate time position under mouse before zoom
        let time_at_mouse = self.x_to_time(mouse_x);

        // Apply zoom
        let new_zoom = (old_zoom * (1.0 + zoom_delta)).clamp(MIN_PIXELS_PER_SECOND, MAX_PIXELS_PER_SECOND);
        self.pixels_per_second = new_zoom;

        // Adjust viewport so the same time stays under the mouse
        let new_mouse_x = self.time_to_x(time_at_mouse);
        let time_delta = (new_mouse_x - mouse_x) / new_zoom;
        self.viewport_start_time = (self.viewport_start_time + time_delta as f64).max(0.0);
    }

    /// Convert time (seconds) to pixel x-coordinate
    fn time_to_x(&self, time: f64) -> f32 {
        ((time - self.viewport_start_time) * self.pixels_per_second as f64) as f32
    }

    /// Convert pixel x-coordinate to time (seconds)
    fn x_to_time(&self, x: f32) -> f64 {
        self.viewport_start_time + (x / self.pixels_per_second) as f64
    }

    /// Calculate appropriate interval for time ruler based on zoom level
    fn calculate_ruler_interval(&self) -> f64 {
        // Target: 50-100px between major ticks
        let target_px = 75.0;
        let target_seconds = target_px / self.pixels_per_second;

        // Standard intervals: 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100...
        let intervals = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];

        // Find the interval closest to our target
        intervals.iter()
            .min_by_key(|&&interval| ((interval - target_seconds as f64).abs() * 1000.0) as i32)
            .copied()
            .unwrap_or(1.0)
    }

    /// Render the time ruler at the top
    fn render_ruler(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme) {
        let painter = ui.painter();

        // Background
        let bg_style = theme.style(".timeline-background", ui.ctx());
        let bg_color = bg_style.background_color.unwrap_or(egui::Color32::from_rgb(34, 34, 34));
        painter.rect_filled(
            rect,
            0.0,
            bg_color,
        );

        // Get text color from theme
        let text_style = theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));

        // Calculate interval for tick marks
        let interval = self.calculate_ruler_interval();

        // Draw tick marks and labels
        let start_time = (self.viewport_start_time / interval).floor() * interval;
        let end_time = self.x_to_time(rect.width());

        let mut time = start_time;
        while time <= end_time {
            let x = self.time_to_x(time);

            if x >= 0.0 && x <= rect.width() {
                // Major tick mark
                painter.line_segment(
                    [
                        rect.min + egui::vec2(x, rect.height() - 10.0),
                        rect.min + egui::vec2(x, rect.height()),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                );

                // Time label
                let label = format!("{:.1}s", time);
                painter.text(
                    rect.min + egui::vec2(x + 2.0, 5.0),
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(12.0),
                    text_color,
                );
            }

            // Minor tick marks (subdivisions)
            let minor_interval = interval / 5.0;
            for i in 1..5 {
                let minor_time = time + minor_interval * i as f64;
                let minor_x = self.time_to_x(minor_time);

                if minor_x >= 0.0 && minor_x <= rect.width() {
                    painter.line_segment(
                        [
                            rect.min + egui::vec2(minor_x, rect.height() - 5.0),
                            rect.min + egui::vec2(minor_x, rect.height()),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    );
                }
            }

            time += interval;
        }
    }

    /// Render the playhead (current time indicator)
    fn render_playhead(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme, playback_time: f64) {
        let x = self.time_to_x(playback_time);

        if x >= 0.0 && x <= rect.width() {
            let painter = ui.painter();
            let scrubber_style = theme.style(".timeline-scrubber", ui.ctx());
            let scrubber_color = scrubber_style.background_color.unwrap_or(egui::Color32::from_rgb(204, 34, 34));

            // Red vertical line
            painter.line_segment(
                [
                    rect.min + egui::vec2(x, 0.0),
                    egui::pos2(rect.min.x + x, rect.max.y),
                ],
                egui::Stroke::new(2.0, scrubber_color),
            );

            // Playhead handle (triangle at top)
            let handle_size = 8.0;
            let points = vec![
                rect.min + egui::vec2(x, 0.0),
                rect.min + egui::vec2(x - handle_size / 2.0, handle_size),
                rect.min + egui::vec2(x + handle_size / 2.0, handle_size),
            ];
            painter.add(egui::Shape::convex_polygon(
                points,
                scrubber_color,
                egui::Stroke::NONE,
            ));
        }
    }

    /// Render mini piano roll visualization for MIDI clips on timeline
    /// Shows notes modulo 12 (one octave) matching the JavaScript reference implementation
    #[allow(clippy::too_many_arguments)]
    fn render_midi_piano_roll(
        painter: &egui::Painter,
        clip_rect: egui::Rect,
        rect_min_x: f32, // Timeline panel left edge (for proper viewport-relative positioning)
        events: &[(f64, u8, bool)], // (timestamp, note_number, is_note_on)
        trim_start: f64,
        visible_duration: f64,
        timeline_start: f64,
        viewport_start_time: f64,
        pixels_per_second: f32,
        theme: &crate::theme::Theme,
        ctx: &egui::Context,
    ) {
        let clip_height = clip_rect.height();
        let note_height = clip_height / 12.0; // 12 semitones per octave

        // Get note color from theme CSS (fallback to black)
        let note_style = theme.style(".timeline-midi-note", ctx);
        let note_color = note_style.background_color.unwrap_or(egui::Color32::BLACK);

        // Build a map of active notes (note_number -> note_on_timestamp)
        // to calculate durations when we encounter note-offs
        let mut active_notes: std::collections::HashMap<u8, f64> = std::collections::HashMap::new();
        let mut note_rectangles: Vec<(egui::Rect, u8)> = Vec::new();

        // First pass: pair note-ons with note-offs to calculate durations
        for &(timestamp, note_number, is_note_on) in events {
            if is_note_on {
                // Store note-on timestamp
                active_notes.insert(note_number, timestamp);
            } else {
                // Note-off: find matching note-on and calculate duration
                if let Some(&note_on_time) = active_notes.get(&note_number) {
                    let duration = timestamp - note_on_time;

                    // Skip notes outside visible trim range
                    if note_on_time < trim_start || note_on_time > trim_start + visible_duration {
                        active_notes.remove(&note_number);
                        continue;
                    }

                    // Calculate X position and width
                    // Convert note position to absolute timeline position
                    let note_timeline_pos = timeline_start + (note_on_time - trim_start);
                    // Convert to screen X using same formula as clip positioning (time_to_x)
                    let note_x = rect_min_x + ((note_timeline_pos - viewport_start_time) * pixels_per_second as f64) as f32;

                    // Calculate note width from duration (minimum 2px for visibility)
                    let note_width = (duration as f32 * pixels_per_second).max(2.0);

                    // Calculate Y position (modulo 12 for octave wrapping)
                    let pitch_class = note_number % 12;
                    let note_y = clip_rect.min.y + ((11 - pitch_class) as f32 * note_height);

                    let note_rect = egui::Rect::from_min_size(
                        egui::pos2(note_x, note_y),
                        egui::vec2(note_width, note_height - 1.0), // -1 for spacing between notes
                    );

                    // Store for rendering (only if visible)
                    if note_rect.right() >= clip_rect.left() && note_rect.left() <= clip_rect.right() {
                        note_rectangles.push((note_rect, note_number));
                    }

                    active_notes.remove(&note_number);
                }
            }
        }

        // Handle any notes that didn't get a note-off (still active at end of clip)
        for (&note_number, &note_on_time) in &active_notes {
            // Skip notes outside visible trim range
            if note_on_time < trim_start || note_on_time > trim_start + visible_duration {
                continue;
            }

            // Use a default duration (extend to end of visible area or 0.5 seconds, whichever is shorter)
            let max_end_time = (trim_start + visible_duration).min(note_on_time + 0.5);
            let duration = max_end_time - note_on_time;

            // Convert note position to absolute timeline position
            let note_timeline_pos = timeline_start + (note_on_time - trim_start);
            // Convert to screen X using same formula as clip positioning (time_to_x)
            let note_x = rect_min_x + ((note_timeline_pos - viewport_start_time) * pixels_per_second as f64) as f32;

            let note_width = (duration as f32 * pixels_per_second).max(2.0);

            let pitch_class = note_number % 12;
            let note_y = clip_rect.min.y + ((11 - pitch_class) as f32 * note_height);

            let note_rect = egui::Rect::from_min_size(
                egui::pos2(note_x, note_y),
                egui::vec2(note_width, note_height - 1.0),
            );

            if note_rect.right() >= clip_rect.left() && note_rect.left() <= clip_rect.right() {
                note_rectangles.push((note_rect, note_number));
            }
        }

        // Second pass: render all note rectangles
        for (note_rect, _note_number) in note_rectangles {
            painter.rect_filled(note_rect, 1.0, note_color);
        }
    }

    /// Render waveform visualization for audio clips on timeline
    /// Uses peak-based rendering: each waveform sample has a fixed pixel width that scales with zoom
    #[allow(clippy::too_many_arguments)]
    fn render_audio_waveform(
        painter: &egui::Painter,
        clip_rect: egui::Rect,
        clip_start_x: f32, // Absolute screen x where clip starts (can be offscreen)
        clip_bg_color: egui::Color32, // Background color of the clip
        waveform: &[daw_backend::WaveformPeak],
        clip_duration: f64,
        pixels_per_second: f32,
        trim_start: f64,
        theme: &crate::theme::Theme,
        ctx: &egui::Context,
    ) {
        if waveform.is_empty() {
            return;
        }

        let clip_height = clip_rect.height();
        let center_y = clip_rect.center().y;

        // Calculate waveform color: lighten the clip background color
        // Blend clip background with white (70% white + 30% clip color) for subtle tint
        // Use full opacity to prevent overlapping lines from blending lighter when zoomed out
        let r = ((255.0 * 0.7) + (clip_bg_color.r() as f32 * 0.3)) as u8;
        let g = ((255.0 * 0.7) + (clip_bg_color.g() as f32 * 0.3)) as u8;
        let b = ((255.0 * 0.7) + (clip_bg_color.b() as f32 * 0.3)) as u8;
        let waveform_color = egui::Color32::from_rgb(r, g, b);

        // Calculate how wide each peak should be at current zoom (mirrors JavaScript)
        // fullSourceWidth = sourceDuration * pixelsPerSecond
        // pixelsPerPeak = fullSourceWidth / waveformData.length
        let full_source_width = clip_duration * pixels_per_second as f64;
        let pixels_per_peak = full_source_width / waveform.len() as f64;

        // Calculate which peak corresponds to the clip's offset (trimmed left edge)
        let offset_peak_index = ((trim_start / clip_duration) * waveform.len() as f64).floor() as usize;
        let offset_peak_index = offset_peak_index.min(waveform.len().saturating_sub(1));

        // Calculate visible peak range
        // firstVisiblePeak = max(offsetPeakIndex, floor((visibleStart - startX) / pixelsPerPeak) + offsetPeakIndex)
        let visible_start = clip_rect.min.x;
        let visible_end = clip_rect.max.x;

        let first_visible_peak_from_viewport = if pixels_per_peak > 0.0 {
            (((visible_start - clip_start_x) as f64 / pixels_per_peak).floor() as isize + offset_peak_index as isize).max(0)
        } else {
            offset_peak_index as isize
        };
        let first_visible_peak = (first_visible_peak_from_viewport as usize).max(offset_peak_index);

        let last_visible_peak_from_viewport = if pixels_per_peak > 0.0 {
            ((visible_end - clip_start_x) as f64 / pixels_per_peak).ceil() as isize + offset_peak_index as isize
        } else {
            offset_peak_index as isize
        };
        let last_visible_peak = (last_visible_peak_from_viewport as usize)
            .min(waveform.len().saturating_sub(1));

        if first_visible_peak > last_visible_peak || first_visible_peak >= waveform.len() {
            return;
        }

        println!("\n🎵 WAVEFORM RENDER:");
        println!("  Waveform total peaks: {}", waveform.len());
        println!("  Clip duration: {:.2}s", clip_duration);
        println!("  Pixels per second: {}", pixels_per_second);
        println!("  Pixels per peak: {:.4}", pixels_per_peak);
        println!("  Trim start: {:.2}s", trim_start);
        println!("  Offset peak index: {}", offset_peak_index);
        println!("  Clip start X: {:.1}", clip_start_x);
        println!("  Clip rect: x=[{:.1}, {:.1}], y=[{:.1}, {:.1}]",
                 clip_rect.min.x, clip_rect.max.x, clip_rect.min.y, clip_rect.max.y);
        println!("  Visible start: {:.1}, end: {:.1}", visible_start, visible_end);
        println!("  First visible peak: {} (time: {:.2}s)",
                 first_visible_peak, first_visible_peak as f64 * clip_duration / waveform.len() as f64);
        println!("  Last visible peak: {} (time: {:.2}s)",
                 last_visible_peak, last_visible_peak as f64 * clip_duration / waveform.len() as f64);
        println!("  Peak range size: {}", last_visible_peak - first_visible_peak + 1);

        // Draw waveform as vertical lines from min to max
        // Line width scales with zoom to avoid gaps between peaks
        let line_width = if pixels_per_peak > 1.0 {
            pixels_per_peak.ceil() as f32
        } else {
            1.0
        };

        let mut peaks_drawn = 0;
        let mut lines = Vec::new();

        for i in first_visible_peak..=last_visible_peak {
            if i >= waveform.len() {
                break;
            }

            let peak_x = clip_start_x + ((i as isize - offset_peak_index as isize) as f64 * pixels_per_peak) as f32;
            let peak = &waveform[i];

            // Calculate Y positions for min and max
            let max_y = center_y + (peak.max * clip_height * 0.45);
            let min_y = center_y + (peak.min * clip_height * 0.45);

            if peaks_drawn < 3 {
                println!("  PEAK[{}]: x={:.1}, min={:.3} (y={:.1}), max={:.3} (y={:.1})",
                         i, peak_x, peak.min, min_y, peak.max, max_y);
            }

            // Draw vertical line from min to max
            lines.push((
                egui::pos2(peak_x, max_y),
                egui::pos2(peak_x, min_y),
            ));

            peaks_drawn += 1;
        }

        println!("  Peaks drawn: {}, line width: {:.1}px", peaks_drawn, line_width);

        // Draw all lines with clipping
        for (start, end) in lines {
            painter.with_clip_rect(clip_rect).line_segment(
                [start, end],
                egui::Stroke::new(line_width, waveform_color),
            );
        }
    }

    /// Render layer header column (left side with track names and controls)
    fn render_layer_headers(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        theme: &crate::theme::Theme,
        active_layer_id: &Option<uuid::Uuid>,
        pending_actions: &mut Vec<Box<dyn lightningbeam_core::action::Action>>,
        document: &lightningbeam_core::document::Document,
    ) {
        // Background for header column
        let header_style = theme.style(".timeline-header", ui.ctx());
        let header_bg = header_style.background_color.unwrap_or(egui::Color32::from_rgb(17, 17, 17));
        ui.painter().rect_filled(
            rect,
            0.0,
            header_bg,
        );

        // Theme colors for active/inactive layers
        let active_style = theme.style(".timeline-layer-active", ui.ctx());
        let inactive_style = theme.style(".timeline-layer-inactive", ui.ctx());
        let active_color = active_style.background_color.unwrap_or(egui::Color32::from_rgb(79, 79, 79));
        let inactive_color = inactive_style.background_color.unwrap_or(egui::Color32::from_rgb(51, 51, 51));

        // Get text color from theme
        let text_style = theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));
        let secondary_text_color = egui::Color32::from_gray(150);

        // Draw layer headers from document (reversed so newest layers appear on top)
        for (i, layer) in document.root.children.iter().rev().enumerate() {
            let y = rect.min.y + i as f32 * LAYER_HEIGHT - self.viewport_scroll_y;

            // Skip if layer is outside visible area
            if y + LAYER_HEIGHT < rect.min.y || y > rect.max.y {
                continue;
            }

            let header_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, y),
                egui::vec2(LAYER_HEADER_WIDTH, LAYER_HEIGHT),
            );

            // Active vs inactive background colors
            let is_active = active_layer_id.map_or(false, |id| id == layer.id());
            let bg_color = if is_active {
                active_color
            } else {
                inactive_color
            };

            ui.painter().rect_filled(header_rect, 0.0, bg_color);

            // Get layer info
            let layer_data = layer.layer();
            let layer_name = &layer_data.name;
            let (layer_type, type_color) = match layer {
                lightningbeam_core::layer::AnyLayer::Vector(_) => ("Vector", egui::Color32::from_rgb(255, 180, 100)), // Orange
                lightningbeam_core::layer::AnyLayer::Audio(audio_layer) => {
                    match audio_layer.audio_layer_type {
                        lightningbeam_core::layer::AudioLayerType::Midi => ("MIDI", egui::Color32::from_rgb(100, 255, 150)), // Green
                        lightningbeam_core::layer::AudioLayerType::Sampled => ("Audio", egui::Color32::from_rgb(100, 180, 255)), // Blue
                    }
                }
                lightningbeam_core::layer::AnyLayer::Video(_) => ("Video", egui::Color32::from_rgb(255, 150, 100)), // Orange/Red
            };

            // Color indicator bar on the left edge
            let indicator_rect = egui::Rect::from_min_size(
                header_rect.min,
                egui::vec2(4.0, LAYER_HEIGHT),
            );
            ui.painter().rect_filled(indicator_rect, 0.0, type_color);

            // Layer name
            ui.painter().text(
                header_rect.min + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                layer_name,
                egui::FontId::proportional(14.0),
                text_color,
            );

            // Layer type (smaller text below name with colored background)
            let type_text_pos = header_rect.min + egui::vec2(10.0, 28.0);
            let type_text_galley = ui.painter().layout_no_wrap(
                layer_type.to_string(),
                egui::FontId::proportional(11.0),
                secondary_text_color,
            );

            // Draw colored background for type label
            let type_bg_rect = egui::Rect::from_min_size(
                type_text_pos + egui::vec2(-2.0, -1.0),
                egui::vec2(type_text_galley.size().x + 4.0, type_text_galley.size().y + 2.0),
            );
            ui.painter().rect_filled(
                type_bg_rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(type_color.r(), type_color.g(), type_color.b(), 60),
            );

            ui.painter().text(
                type_text_pos,
                egui::Align2::LEFT_TOP,
                layer_type,
                egui::FontId::proportional(11.0),
                secondary_text_color,
            );

            // Layer controls (mute, solo, lock, volume)
            let controls_top = header_rect.min.y + 4.0;
            let controls_right = header_rect.max.x - 8.0;
            let button_size = egui::vec2(20.0, 20.0);
            let slider_width = 60.0;

            // Position controls from right to left
            let volume_slider_rect = egui::Rect::from_min_size(
                egui::pos2(controls_right - slider_width, controls_top),
                egui::vec2(slider_width, 20.0),
            );

            let lock_button_rect = egui::Rect::from_min_size(
                egui::pos2(volume_slider_rect.min.x - button_size.x - 4.0, controls_top),
                button_size,
            );

            let solo_button_rect = egui::Rect::from_min_size(
                egui::pos2(lock_button_rect.min.x - button_size.x - 4.0, controls_top),
                button_size,
            );

            let mute_button_rect = egui::Rect::from_min_size(
                egui::pos2(solo_button_rect.min.x - button_size.x - 4.0, controls_top),
                button_size,
            );

            // Get layer ID and current property values from the layer we already have
            let layer_id = layer.id();
            let current_volume = layer.volume();
            let is_muted = layer.muted();
            let is_soloed = layer.soloed();
            let is_locked = layer.locked();

            // Mute button
            // TODO: Replace with SVG icon (volume-up-fill.svg / volume-mute.svg)
            let mute_response = ui.allocate_ui_at_rect(mute_button_rect, |ui| {
                let mute_text = if is_muted { "🔇" } else { "🔊" };
                let button = egui::Button::new(mute_text)
                    .fill(if is_muted {
                        egui::Color32::from_rgba_unmultiplied(255, 100, 100, 100)
                    } else {
                        egui::Color32::from_gray(40)
                    })
                    .stroke(egui::Stroke::NONE);
                ui.add(button)
            }).inner;

            if mute_response.clicked() {
                self.layer_control_clicked = true;
                pending_actions.push(Box::new(
                    lightningbeam_core::actions::SetLayerPropertiesAction::new(
                        layer_id,
                        lightningbeam_core::actions::LayerProperty::Muted(!is_muted),
                    )
                ));
            }

            // Solo button
            // TODO: Replace with SVG headphones icon
            let solo_response = ui.allocate_ui_at_rect(solo_button_rect, |ui| {
                let button = egui::Button::new("🎧")
                    .fill(if is_soloed {
                        egui::Color32::from_rgba_unmultiplied(100, 200, 100, 100)
                    } else {
                        egui::Color32::from_gray(40)
                    })
                    .stroke(egui::Stroke::NONE);
                ui.add(button)
            }).inner;

            if solo_response.clicked() {
                self.layer_control_clicked = true;
                pending_actions.push(Box::new(
                    lightningbeam_core::actions::SetLayerPropertiesAction::new(
                        layer_id,
                        lightningbeam_core::actions::LayerProperty::Soloed(!is_soloed),
                    )
                ));
            }

            // Lock button
            // TODO: Replace with SVG lock/lock-open icons
            let lock_response = ui.allocate_ui_at_rect(lock_button_rect, |ui| {
                let lock_text = if is_locked { "🔒" } else { "🔓" };
                let button = egui::Button::new(lock_text)
                    .fill(if is_locked {
                        egui::Color32::from_rgba_unmultiplied(200, 150, 100, 100)
                    } else {
                        egui::Color32::from_gray(40)
                    })
                    .stroke(egui::Stroke::NONE);
                ui.add(button)
            }).inner;

            if lock_response.clicked() {
                self.layer_control_clicked = true;
                pending_actions.push(Box::new(
                    lightningbeam_core::actions::SetLayerPropertiesAction::new(
                        layer_id,
                        lightningbeam_core::actions::LayerProperty::Locked(!is_locked),
                    )
                ));
            }

            // Volume slider (nonlinear: 0-70% slider = 0-100% volume, 70-100% slider = 100-200% volume)
            let volume_response = ui.allocate_ui_at_rect(volume_slider_rect, |ui| {
                // Map volume (0.0-2.0) to slider position (0.0-1.0)
                let slider_value = if current_volume <= 1.0 {
                    // 0.0-1.0 volume maps to 0.0-0.7 slider (70%)
                    current_volume * 0.7
                } else {
                    // 1.0-2.0 volume maps to 0.7-1.0 slider (30%)
                    0.7 + (current_volume - 1.0) * 0.3
                };

                let mut temp_slider_value = slider_value;
                let slider = egui::Slider::new(&mut temp_slider_value, 0.0..=1.0)
                    .show_value(false);

                let response = ui.add(slider);
                (response, temp_slider_value)
            }).inner;

            if volume_response.0.changed() {
                self.layer_control_clicked = true;
                // Map slider position (0.0-1.0) back to volume (0.0-2.0)
                let new_volume = if volume_response.1 <= 0.7 {
                    // 0.0-0.7 slider maps to 0.0-1.0 volume
                    volume_response.1 / 0.7
                } else {
                    // 0.7-1.0 slider maps to 1.0-2.0 volume
                    1.0 + (volume_response.1 - 0.7) / 0.3
                };

                pending_actions.push(Box::new(
                    lightningbeam_core::actions::SetLayerPropertiesAction::new(
                        layer_id,
                        lightningbeam_core::actions::LayerProperty::Volume(new_volume),
                    )
                ));
            }

            // Separator line at bottom
            ui.painter().line_segment(
                [
                    egui::pos2(header_rect.min.x, header_rect.max.y),
                    egui::pos2(header_rect.max.x, header_rect.max.y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
            );
        }

        // Right border for header column
        ui.painter().line_segment(
            [
                egui::pos2(rect.max.x, rect.min.y),
                egui::pos2(rect.max.x, rect.max.y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
        );
    }

    /// Render layer rows (timeline content area)
    fn render_layers(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        theme: &crate::theme::Theme,
        document: &lightningbeam_core::document::Document,
        active_layer_id: &Option<uuid::Uuid>,
        selection: &lightningbeam_core::selection::Selection,
        midi_event_cache: &std::collections::HashMap<u32, Vec<(f64, u8, bool)>>,
        waveform_cache: &std::collections::HashMap<usize, Vec<daw_backend::WaveformPeak>>,
    ) {
        let painter = ui.painter();

        // Theme colors for active/inactive layers
        let active_style = theme.style(".timeline-row-active", ui.ctx());
        let inactive_style = theme.style(".timeline-row-inactive", ui.ctx());
        let active_color = active_style.background_color.unwrap_or(egui::Color32::from_rgb(85, 85, 85));
        let inactive_color = inactive_style.background_color.unwrap_or(egui::Color32::from_rgb(136, 136, 136));

        // Draw layer rows from document (reversed so newest layers appear on top)
        for (i, layer) in document.root.children.iter().rev().enumerate() {
            let y = rect.min.y + i as f32 * LAYER_HEIGHT - self.viewport_scroll_y;

            // Skip if layer is outside visible area
            if y + LAYER_HEIGHT < rect.min.y || y > rect.max.y {
                continue;
            }

            let layer_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, y),
                egui::vec2(rect.width(), LAYER_HEIGHT),
            );

            // Active vs inactive background colors
            let is_active = active_layer_id.map_or(false, |id| id == layer.id());
            let bg_color = if is_active {
                active_color
            } else {
                inactive_color
            };

            painter.rect_filled(layer_rect, 0.0, bg_color);

            // Grid lines matching ruler
            let interval = self.calculate_ruler_interval();
            let start_time = (self.viewport_start_time / interval).floor() * interval;
            let end_time = self.x_to_time(rect.width());

            let mut time = start_time;
            while time <= end_time {
                let x = self.time_to_x(time);

                if x >= 0.0 && x <= rect.width() {
                    painter.line_segment(
                        [
                            egui::pos2(rect.min.x + x, y),
                            egui::pos2(rect.min.x + x, y + LAYER_HEIGHT),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                    );
                }

                time += interval;
            }

            // Draw clip instances for this layer
            let clip_instances = match layer {
                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
            };

            for clip_instance in clip_instances {
                // Get the clip to determine duration
                let clip_duration = match layer {
                    lightningbeam_core::layer::AnyLayer::Vector(_) => {
                        document.get_vector_clip(&clip_instance.clip_id)
                            .map(|c| c.duration)
                    }
                    lightningbeam_core::layer::AnyLayer::Audio(_) => {
                        document.get_audio_clip(&clip_instance.clip_id)
                            .map(|c| c.duration)
                    }
                    lightningbeam_core::layer::AnyLayer::Video(_) => {
                        document.get_video_clip(&clip_instance.clip_id)
                            .map(|c| c.duration)
                    }
                };

                if let Some(clip_duration) = clip_duration {
                    // Calculate effective duration accounting for trimming
                    let mut instance_duration = clip_instance.effective_duration(clip_duration);

                    // Instance positioned on the layer's timeline using timeline_start
                    // The layer itself has start_time, so the absolute timeline position is:
                    // layer.start_time + instance.timeline_start
                    let layer_data = layer.layer();
                    let mut instance_start = clip_instance.timeline_start;

                    // Apply drag offset preview for selected clips
                    let is_selected = selection.contains_clip_instance(&clip_instance.id);

                    if let Some(drag_type) = self.clip_drag_state {
                        if is_selected {
                            match drag_type {
                                ClipDragType::Move => {
                                    // Move: shift the entire clip along the timeline
                                    instance_start += self.drag_offset;
                                }
                                ClipDragType::TrimLeft => {
                                    // Trim left: calculate new trim_start and clamp to valid range
                                    let new_trim_start = (clip_instance.trim_start + self.drag_offset)
                                        .max(0.0)
                                        .min(clip_duration);
                                    let actual_offset = new_trim_start - clip_instance.trim_start;

                                    // Move start and reduce duration by actual clamped offset
                                    instance_start = (clip_instance.timeline_start + actual_offset)
                                        .max(0.0);
                                    instance_duration = (clip_duration - new_trim_start).max(0.0);

                                    // Adjust for existing trim_end
                                    if let Some(trim_end) = clip_instance.trim_end {
                                        instance_duration = (trim_end - new_trim_start).max(0.0);
                                    }
                                }
                                ClipDragType::TrimRight => {
                                    // Trim right: extend or reduce duration, clamped to available content
                                    let max_duration = clip_duration - clip_instance.trim_start;
                                    instance_duration = (instance_duration + self.drag_offset)
                                        .max(0.0)
                                        .min(max_duration);
                                }
                            }
                        }
                    }

                    let instance_end = instance_start + instance_duration;

                    let start_x = self.time_to_x(instance_start);
                    let end_x = self.time_to_x(instance_end);

                    // Only draw if any part is visible in viewport
                    if end_x >= 0.0 && start_x <= rect.width() {
                        let visible_start_x = start_x.max(0.0);
                        let visible_end_x = end_x.min(rect.width());

                        // Choose color based on layer type
                        let (clip_color, bright_color) = match layer {
                            lightningbeam_core::layer::AnyLayer::Vector(_) => (
                                egui::Color32::from_rgb(220, 150, 80), // Orange
                                egui::Color32::from_rgb(255, 210, 150), // Bright orange
                            ),
                            lightningbeam_core::layer::AnyLayer::Audio(audio_layer) => {
                                match audio_layer.audio_layer_type {
                                    lightningbeam_core::layer::AudioLayerType::Midi => (
                                        egui::Color32::from_rgb(100, 200, 150), // Green
                                        egui::Color32::from_rgb(150, 255, 200), // Bright green
                                    ),
                                    lightningbeam_core::layer::AudioLayerType::Sampled => (
                                        egui::Color32::from_rgb(80, 150, 220), // Blue
                                        egui::Color32::from_rgb(150, 210, 255), // Bright blue
                                    ),
                                }
                            }
                            lightningbeam_core::layer::AnyLayer::Video(_) => (
                                egui::Color32::from_rgb(255, 150, 100), // Orange/Red
                                egui::Color32::from_rgb(255, 200, 150), // Bright orange/red
                            ),
                        };

                        let clip_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x + visible_start_x, y + 10.0),
                            egui::pos2(rect.min.x + visible_end_x, y + LAYER_HEIGHT - 10.0),
                        );

                        // Draw the clip instance
                        painter.rect_filled(
                            clip_rect,
                            3.0, // Rounded corners
                            clip_color,
                        );

                        // AUDIO VISUALIZATION: Draw piano roll or waveform overlay
                        if let lightningbeam_core::layer::AnyLayer::Audio(_) = layer {
                            if let Some(clip) = document.get_audio_clip(&clip_instance.clip_id) {
                                match &clip.clip_type {
                                    // MIDI: Draw piano roll
                                    lightningbeam_core::clip::AudioClipType::Midi { midi_clip_id } => {
                                        if let Some(events) = midi_event_cache.get(midi_clip_id) {
                                            Self::render_midi_piano_roll(
                                                painter,
                                                clip_rect,
                                                rect.min.x, // Pass timeline panel left edge for proper positioning
                                                events,
                                                clip_instance.trim_start,
                                                instance_duration,
                                                instance_start,
                                                self.viewport_start_time,
                                                self.pixels_per_second,
                                                theme,
                                                ui.ctx(),
                                            );
                                        }
                                    }
                                    // Sampled Audio: Draw waveform
                                    lightningbeam_core::clip::AudioClipType::Sampled { audio_pool_index } => {
                                        if let Some(waveform) = waveform_cache.get(audio_pool_index) {
                                            // Calculate absolute screen x where clip starts (can be offscreen)
                                            let clip_start_x = rect.min.x + start_x;

                                            Self::render_audio_waveform(
                                                painter,
                                                clip_rect,
                                                clip_start_x,
                                                clip_color, // Pass clip background color for tinting
                                                waveform,
                                                clip.duration,
                                                self.pixels_per_second,
                                                clip_instance.trim_start,
                                                theme,
                                                ui.ctx(),
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        // Draw border only if selected (brighter version of clip color)
                        if selection.contains_clip_instance(&clip_instance.id) {
                            painter.rect_stroke(
                                clip_rect,
                                3.0,
                                egui::Stroke::new(3.0, bright_color),
                                egui::StrokeKind::Middle,
                            );
                        }

                        // Draw clip name if there's space
                        if let Some(name) = &clip_instance.name {
                            if clip_rect.width() > 50.0 {
                                painter.text(
                                    clip_rect.min + egui::vec2(5.0, 5.0),
                                    egui::Align2::LEFT_TOP,
                                    name,
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::WHITE,
                                );
                            }
                        }
                    }
                }
            }

            // Separator line at bottom
            painter.line_segment(
                [
                    egui::pos2(layer_rect.min.x, layer_rect.max.y),
                    egui::pos2(layer_rect.max.x, layer_rect.max.y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
            );
        }
    }

    /// Handle mouse input for scrubbing, panning, zooming, layer selection, and clip instance selection
    fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        full_timeline_rect: egui::Rect,
        ruler_rect: egui::Rect,
        content_rect: egui::Rect,
        header_rect: egui::Rect,
        layer_count: usize,
        document: &lightningbeam_core::document::Document,
        active_layer_id: &mut Option<uuid::Uuid>,
        selection: &mut lightningbeam_core::selection::Selection,
        pending_actions: &mut Vec<Box<dyn lightningbeam_core::action::Action>>,
        playback_time: &mut f64,
        is_playing: &mut bool,
        audio_controller: Option<&std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>>,
    ) {
        // Don't allocate the header area for input - let widgets handle it directly
        // Only allocate content area (ruler + layers) with click and drag
        let content_response = ui.allocate_rect(
            egui::Rect::from_min_size(
                egui::pos2(content_rect.min.x, ruler_rect.min.y),
                egui::vec2(
                    content_rect.width(),
                    ruler_rect.height() + content_rect.height()
                )
            ),
            egui::Sense::click_and_drag()
        );

        let response = content_response;

        // Check if mouse is over either area
        let header_hovered = ui.rect_contains_pointer(header_rect);
        let any_hovered = response.hovered() || header_hovered;

        // Only process input if mouse is over the timeline pane
        if !any_hovered {
            self.is_panning = false;
            self.last_pan_pos = None;
            self.is_scrubbing = false;
            return;
        }

        let alt_held = ui.input(|i| i.modifiers.alt);
        let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        let shift_held = ui.input(|i| i.modifiers.shift);

        // Handle clip instance selection by clicking on clip rectangles
        let mut clicked_clip_instance = false;
        if response.clicked() && !alt_held {
            if let Some(pos) = response.interact_pointer_pos() {
                // Check if click is in content area (not ruler or header column)
                if pos.y >= header_rect.min.y && pos.x >= content_rect.min.x {
                    let relative_y = pos.y - header_rect.min.y + self.viewport_scroll_y;
                    let clicked_layer_index = (relative_y / LAYER_HEIGHT) as usize;
                    let click_time = self.x_to_time(pos.x - content_rect.min.x);

                    // Get the layer at this index (accounting for reversed display order)
                    if clicked_layer_index < layer_count {
                        let layers: Vec<_> = document.root.children.iter().rev().collect();
                        if let Some(layer) = layers.get(clicked_layer_index) {
                            let layer_data = layer.layer();

                            // Get clip instances for this layer
                            let clip_instances = match layer {
                                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                            };

                            // Check if click is within any clip instance
                            for clip_instance in clip_instances {
                                // Get the clip to determine duration
                                let clip_duration = match layer {
                                    lightningbeam_core::layer::AnyLayer::Vector(_) => {
                                        document.get_vector_clip(&clip_instance.clip_id)
                                            .map(|c| c.duration)
                                    }
                                    lightningbeam_core::layer::AnyLayer::Audio(_) => {
                                        document.get_audio_clip(&clip_instance.clip_id)
                                            .map(|c| c.duration)
                                    }
                                    lightningbeam_core::layer::AnyLayer::Video(_) => {
                                        document.get_video_clip(&clip_instance.clip_id)
                                            .map(|c| c.duration)
                                    }
                                };

                                if let Some(clip_duration) = clip_duration {
                                    let instance_duration = clip_instance.effective_duration(clip_duration);
                                    let instance_start = clip_instance.timeline_start;
                                    let instance_end = instance_start + instance_duration;

                                    // Check if click is within this clip instance's time range
                                    if click_time >= instance_start && click_time <= instance_end {
                                        // Found a clicked clip instance!
                                        if shift_held {
                                            // Shift+click: add to selection
                                            selection.add_clip_instance(clip_instance.id);
                                        } else {
                                            // Regular click: select only this clip
                                            selection.select_only_clip_instance(clip_instance.id);
                                        }
                                        clicked_clip_instance = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Handle layer header selection (only if no control widget was clicked)
        // Check for clicks in header area using direct input query
        let header_clicked = ui.input(|i| {
            i.pointer.button_clicked(egui::PointerButton::Primary) &&
            i.pointer.interact_pos().map_or(false, |pos| header_rect.contains(pos))
        });

        if header_clicked && !alt_held && !clicked_clip_instance && !self.layer_control_clicked {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let relative_y = pos.y - header_rect.min.y + self.viewport_scroll_y;
                let clicked_layer_index = (relative_y / LAYER_HEIGHT) as usize;

                // Get the layer at this index (accounting for reversed display order)
                if clicked_layer_index < layer_count {
                    let layers: Vec<_> = document.root.children.iter().rev().collect();
                    if let Some(layer) = layers.get(clicked_layer_index) {
                        *active_layer_id = Some(layer.id());
                    }
                }
            }
        }

        // Cache mouse position on mousedown (before any dragging)
        if response.hovered() && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
            if let Some(pos) = response.hover_pos() {
                self.mousedown_pos = Some(pos);
            }
        }

        // Handle clip dragging (only if not panning or scrubbing)
        if !alt_held && !self.is_scrubbing && !self.is_panning {
            if response.drag_started() {
                // Use cached mousedown position for edge detection
                if let Some(mousedown_pos) = self.mousedown_pos {
                    if let Some((drag_type, clip_id)) = self.detect_clip_at_pointer(
                        mousedown_pos,
                        document,
                        content_rect,
                        header_rect,
                    ) {
                        // If this clip is not selected, select it (respecting shift key)
                        if !selection.contains_clip_instance(&clip_id) {
                            if shift_held {
                                selection.add_clip_instance(clip_id);
                            } else {
                                selection.select_only_clip_instance(clip_id);
                            }
                        }

                        // Start dragging with the detected drag type
                        self.clip_drag_state = Some(drag_type);
                        self.drag_offset = 0.0;
                    }
                }
            }

            // Update drag offset during drag
            if self.clip_drag_state.is_some() && response.dragged() {
                let drag_delta = response.drag_delta();
                let time_delta = drag_delta.x / self.pixels_per_second;
                self.drag_offset += time_delta as f64;
            }

            // End drag - create action based on drag type
            if let Some(drag_type) = self.clip_drag_state {
                if response.drag_stopped() {
                // Build layer_moves map for the action
                use std::collections::HashMap;
                let mut layer_moves: HashMap<uuid::Uuid, Vec<(uuid::Uuid, f64, f64)>> =
                    HashMap::new();

                // Iterate through all layers to find selected clip instances
                for layer in &document.root.children {
                    let layer_id = layer.id();

                    // Get clip instances for this layer
                    let clip_instances = match layer {
                        lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                        lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                        lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                    };

                    // Find selected clip instances in this layer
                    for clip_instance in clip_instances {
                        if selection.contains_clip_instance(&clip_instance.id) {
                            let old_timeline_start = clip_instance.timeline_start;
                            let new_timeline_start = old_timeline_start + self.drag_offset;

                            // Add to layer_moves
                            layer_moves
                                .entry(layer_id)
                                .or_insert_with(Vec::new)
                                .push((clip_instance.id, old_timeline_start, new_timeline_start));
                        }
                    }
                }

                    // Create and add the action based on drag type
                    match drag_type {
                        ClipDragType::Move => {
                            if !layer_moves.is_empty() {
                                let action = Box::new(
                                    lightningbeam_core::actions::MoveClipInstancesAction::new(
                                        layer_moves,
                                    ),
                                );
                                pending_actions.push(action);
                            }
                        }
                        ClipDragType::TrimLeft | ClipDragType::TrimRight => {
                            // Build layer_trims map for trim action
                            let mut layer_trims: HashMap<
                                uuid::Uuid,
                                Vec<(
                                    uuid::Uuid,
                                    lightningbeam_core::actions::TrimType,
                                    lightningbeam_core::actions::TrimData,
                                    lightningbeam_core::actions::TrimData,
                                )>,
                            > = HashMap::new();

                            // Iterate through all layers to find selected clip instances
                            for layer in &document.root.children {
                                let layer_id = layer.id();
                                let layer_data = layer.layer();

                                let clip_instances = match layer {
                                    lightningbeam_core::layer::AnyLayer::Vector(vl) => {
                                        &vl.clip_instances
                                    }
                                    lightningbeam_core::layer::AnyLayer::Audio(al) => {
                                        &al.clip_instances
                                    }
                                    lightningbeam_core::layer::AnyLayer::Video(vl) => {
                                        &vl.clip_instances
                                    }
                                };

                                // Find selected clip instances in this layer
                                for clip_instance in clip_instances {
                                    if selection.contains_clip_instance(&clip_instance.id) {
                                        // Get clip duration to validate trim bounds
                                        let clip_duration = match layer {
                                            lightningbeam_core::layer::AnyLayer::Vector(_) => {
                                                document
                                                    .get_vector_clip(&clip_instance.clip_id)
                                                    .map(|c| c.duration)
                                            }
                                            lightningbeam_core::layer::AnyLayer::Audio(_) => {
                                                document
                                                    .get_audio_clip(&clip_instance.clip_id)
                                                    .map(|c| c.duration)
                                            }
                                            lightningbeam_core::layer::AnyLayer::Video(_) => {
                                                document
                                                    .get_video_clip(&clip_instance.clip_id)
                                                    .map(|c| c.duration)
                                            }
                                        };

                                        if let Some(clip_duration) = clip_duration {
                                            match drag_type {
                                                ClipDragType::TrimLeft => {
                                                    let old_trim_start = clip_instance.trim_start;
                                                    let old_timeline_start =
                                                        clip_instance.timeline_start;

                                                    // New trim_start is clamped to valid range
                                                    let new_trim_start = (old_trim_start
                                                        + self.drag_offset)
                                                        .max(0.0)
                                                        .min(clip_duration);

                                                    // Calculate actual offset after clamping
                                                    let actual_offset = new_trim_start - old_trim_start;
                                                    let new_timeline_start =
                                                        old_timeline_start + actual_offset;

                                                    layer_trims
                                                        .entry(layer_id)
                                                        .or_insert_with(Vec::new)
                                                        .push((
                                                            clip_instance.id,
                                                            lightningbeam_core::actions::TrimType::TrimLeft,
                                                            lightningbeam_core::actions::TrimData::left(
                                                                old_trim_start,
                                                                old_timeline_start,
                                                            ),
                                                            lightningbeam_core::actions::TrimData::left(
                                                                new_trim_start,
                                                                new_timeline_start,
                                                            ),
                                                        ));
                                                }
                                                ClipDragType::TrimRight => {
                                                    let old_trim_end = clip_instance.trim_end;

                                                    // Calculate new trim_end based on current duration
                                                    let current_duration =
                                                        clip_instance.effective_duration(clip_duration);
                                                    let new_duration =
                                                        (current_duration + self.drag_offset).max(0.0);

                                                    // Convert new duration back to trim_end value
                                                    let new_trim_end = if new_duration >= clip_duration {
                                                        None // Use full clip duration
                                                    } else {
                                                        Some((clip_instance.trim_start + new_duration).min(clip_duration))
                                                    };

                                                    layer_trims
                                                        .entry(layer_id)
                                                        .or_insert_with(Vec::new)
                                                        .push((
                                                            clip_instance.id,
                                                            lightningbeam_core::actions::TrimType::TrimRight,
                                                            lightningbeam_core::actions::TrimData::right(
                                                                old_trim_end,
                                                            ),
                                                            lightningbeam_core::actions::TrimData::right(
                                                                new_trim_end,
                                                            ),
                                                        ));
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }

                            // Create and add the trim action if there are any trims
                            if !layer_trims.is_empty() {
                                let action = Box::new(
                                    lightningbeam_core::actions::TrimClipInstancesAction::new(
                                        layer_trims,
                                    ),
                                );
                                pending_actions.push(action);
                            }
                        }
                    }

                    // Reset drag state
                    self.clip_drag_state = None;
                    self.drag_offset = 0.0;
                    self.mousedown_pos = None;
                }
            }
        }

        // Handle layer selection by clicking on layer header or content (only if no clip was clicked)
        if response.clicked() && !alt_held && !clicked_clip_instance {
            if let Some(pos) = response.interact_pointer_pos() {
                // Check if click is in header or content area (not ruler)
                if pos.y >= header_rect.min.y {
                    let relative_y = pos.y - header_rect.min.y + self.viewport_scroll_y;
                    let clicked_layer_index = (relative_y / LAYER_HEIGHT) as usize;

                    // Get the layer at this index (accounting for reversed display order)
                    if clicked_layer_index < layer_count {
                        let layers: Vec<_> = document.root.children.iter().rev().collect();
                        if let Some(layer) = layers.get(clicked_layer_index) {
                            *active_layer_id = Some(layer.id());
                            // Clear clip instance selection when clicking on empty layer area
                            if !shift_held {
                                selection.clear_clip_instances();
                            }
                        }
                    }
                }
            }
        }

        // Get mouse position relative to content area
        let mouse_pos = response.hover_pos().unwrap_or(content_rect.center());
        let mouse_x = (mouse_pos.x - content_rect.min.x).max(0.0);

        // Calculate max vertical scroll based on number of layers
        let total_content_height = layer_count as f32 * LAYER_HEIGHT;
        let visible_height = content_rect.height();
        let max_scroll_y = (total_content_height - visible_height).max(0.0);

        // Scrubbing (clicking/dragging on ruler, but only when not panning)
        let cursor_over_ruler = ruler_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()));

        // Start scrubbing if cursor is over ruler and we click/drag
        if cursor_over_ruler && !alt_held && (response.clicked() || (response.dragged() && !self.is_panning)) {
            if let Some(pos) = response.interact_pointer_pos() {
                let x = (pos.x - content_rect.min.x).max(0.0);
                let new_time = self.x_to_time(x).max(0.0);
                *playback_time = new_time;
                self.is_scrubbing = true;
            }
        }
        // Continue scrubbing while dragging, even if cursor leaves ruler
        else if self.is_scrubbing && response.dragged() && !self.is_panning {
            if let Some(pos) = response.interact_pointer_pos() {
                let x = (pos.x - content_rect.min.x).max(0.0);
                let new_time = self.x_to_time(x).max(0.0);
                *playback_time = new_time;
            }
        }
        // Stop scrubbing when drag ends - seek the audio engine
        else if !response.dragged() && self.is_scrubbing {
            self.is_scrubbing = false;
            // Seek the audio engine to the new position
            if let Some(controller_arc) = audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                controller.seek(*playback_time);
            }
        }

        // Distinguish between mouse wheel (discrete) and trackpad (smooth)
        let mut handled = false;
        ui.input(|i| {
            for event in &i.raw.events {
                if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
                    match unit {
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                            // Real mouse wheel (discrete clicks) -> always zoom horizontally
                            let zoom_delta = if ctrl_held || modifiers.ctrl {
                                delta.y * 0.01 // Ctrl+wheel: faster zoom
                            } else {
                                delta.y * 0.005 // Normal zoom
                            };
                            self.apply_zoom_at_point(zoom_delta, mouse_x);
                            handled = true;
                        }
                        egui::MouseWheelUnit::Point => {
                            // Trackpad (smooth scrolling)
                            if ctrl_held || modifiers.ctrl {
                                // Ctrl held: zoom
                                let zoom_delta = delta.y * 0.005;
                                self.apply_zoom_at_point(zoom_delta, mouse_x);
                                handled = true;
                            }
                            // Otherwise let scroll_delta handle panning (below)
                        }
                    }
                }
            }
        });

        // Handle scroll_delta for trackpad panning (when Ctrl not held)
        if !handled {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            if scroll_delta.x.abs() > 0.0 || scroll_delta.y.abs() > 0.0 {
                // Horizontal scroll: pan timeline (inverted: positive delta scrolls left/earlier in time)
                let delta_time = scroll_delta.x / self.pixels_per_second;
                self.viewport_start_time = (self.viewport_start_time - delta_time as f64).max(0.0);

                // Vertical scroll: scroll layers vertically (clamped to content bounds)
                self.viewport_scroll_y = (self.viewport_scroll_y - scroll_delta.y).clamp(0.0, max_scroll_y);
            }
        }

        // Handle panning with Alt+Drag (timeline scrolls left/right, layers scroll up/down)
        if alt_held && response.dragged() && !self.is_scrubbing {
            if let Some(last_pos) = self.last_pan_pos {
                if let Some(current_pos) = response.interact_pointer_pos() {
                    let delta = current_pos - last_pos;

                    // Horizontal pan: timeline
                    let delta_time = delta.x / self.pixels_per_second;
                    self.viewport_start_time = (self.viewport_start_time - delta_time as f64).max(0.0);

                    // Vertical pan: layers (clamped to content bounds)
                    self.viewport_scroll_y = (self.viewport_scroll_y - delta.y).clamp(0.0, max_scroll_y);
                }
            }
            self.last_pan_pos = response.interact_pointer_pos();
            self.is_panning = true;
        } else {
            if !response.dragged() {
                self.is_panning = false;
                self.last_pan_pos = None;
            }
        }

        // Update cursor based on hover position (only if not scrubbing or panning)
        if !self.is_scrubbing && !self.is_panning {
            // If dragging a clip with trim, keep the resize cursor
            if let Some(drag_type) = self.clip_drag_state {
                if drag_type != ClipDragType::Move {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            } else if let Some(hover_pos) = response.hover_pos() {
                // Not dragging - detect hover for cursor feedback
                if let Some((drag_type, _clip_id)) = self.detect_clip_at_pointer(
                    hover_pos,
                    document,
                    content_rect,
                    header_rect,
                ) {
                    // Set cursor for trim operations
                    if drag_type != ClipDragType::Move {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                }
            }
        }
    }
}

impl PaneRenderer for TimelinePane {
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        ui.spacing_mut().item_spacing.x = 2.0; // Small spacing between button groups

        // Main playback controls group
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0; // No spacing between buttons
                let button_size = egui::vec2(32.0, 28.0); // Larger buttons

                // Go to start
                if ui.add_sized(button_size, egui::Button::new("|◀")).clicked() {
                    *shared.playback_time = 0.0;
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.seek(0.0);
                    }
                }

                // Rewind (step backward)
                if ui.add_sized(button_size, egui::Button::new("◀◀")).clicked() {
                    *shared.playback_time = (*shared.playback_time - 0.1).max(0.0);
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.seek(*shared.playback_time);
                    }
                }

                // Play/Pause toggle
                let play_pause_text = if *shared.is_playing { "⏸" } else { "▶" };
                if ui.add_sized(button_size, egui::Button::new(play_pause_text)).clicked() {
                    *shared.is_playing = !*shared.is_playing;
                    println!("🔘 Play/Pause button clicked! is_playing = {}", *shared.is_playing);

                    // Send play/pause command to audio engine
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        if *shared.is_playing {
                            controller.play();
                            println!("▶ Started playback");
                        } else {
                            controller.pause();
                            println!("⏸ Paused playback");
                        }
                    } else {
                        println!("⚠️  No audio controller available (audio system failed to initialize)");
                    }
                }

                // Fast forward (step forward)
                if ui.add_sized(button_size, egui::Button::new("▶▶")).clicked() {
                    *shared.playback_time = (*shared.playback_time + 0.1).min(self.duration);
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.seek(*shared.playback_time);
                    }
                }

                // Go to end
                if ui.add_sized(button_size, egui::Button::new("▶|")).clicked() {
                    *shared.playback_time = self.duration;
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.seek(self.duration);
                    }
                }
            });
        });

        ui.separator();

        // Get text color from theme
        let text_style = shared.theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));

        // Time display
        ui.colored_label(text_color, format!("Time: {:.2}s / {:.2}s", *shared.playback_time, self.duration));

        ui.separator();

        // Zoom display
        ui.colored_label(text_color, format!("Zoom: {:.0}px/s", self.pixels_per_second));

        true
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Reset layer control click flag at start of frame
        self.layer_control_clicked = false;

        // Sync playback_time to document
        shared.action_executor.document_mut().current_time = *shared.playback_time;

        // Get document from action executor
        let document = shared.action_executor.document();
        let layer_count = document.root.children.len();

        // Calculate project duration from last clip endpoint across all layers
        let mut max_endpoint: f64 = 10.0; // Default minimum duration
        for layer in &document.root.children {
            let clip_instances = match layer {
                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
            };

            for clip_instance in clip_instances {
                // Get clip duration
                let clip_duration = match layer {
                    lightningbeam_core::layer::AnyLayer::Vector(_) => {
                        document.get_vector_clip(&clip_instance.clip_id)
                            .map(|c| c.duration)
                    }
                    lightningbeam_core::layer::AnyLayer::Audio(_) => {
                        document.get_audio_clip(&clip_instance.clip_id)
                            .map(|c| c.duration)
                    }
                    lightningbeam_core::layer::AnyLayer::Video(_) => {
                        document.get_video_clip(&clip_instance.clip_id)
                            .map(|c| c.duration)
                    }
                };

                if let Some(clip_duration) = clip_duration {
                    let instance_duration = clip_instance.effective_duration(clip_duration);
                    let instance_end = clip_instance.timeline_start + instance_duration;
                    max_endpoint = max_endpoint.max(instance_end);
                }
            }
        }
        self.duration = max_endpoint;

        // Split into layer header column (left) and timeline content (right)
        let header_column_rect = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(LAYER_HEADER_WIDTH, rect.height()),
        );

        let timeline_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(LAYER_HEADER_WIDTH, 0.0),
            egui::vec2(rect.width() - LAYER_HEADER_WIDTH, rect.height()),
        );

        // Split timeline into ruler and content areas
        let ruler_rect = egui::Rect::from_min_size(
            timeline_rect.min,
            egui::vec2(timeline_rect.width(), RULER_HEIGHT),
        );

        let content_rect = egui::Rect::from_min_size(
            timeline_rect.min + egui::vec2(0.0, RULER_HEIGHT),
            egui::vec2(timeline_rect.width(), timeline_rect.height() - RULER_HEIGHT),
        );

        // Split header column into ruler area (top) and layer headers (bottom)
        let header_ruler_spacer = egui::Rect::from_min_size(
            header_column_rect.min,
            egui::vec2(LAYER_HEADER_WIDTH, RULER_HEIGHT),
        );

        let layer_headers_rect = egui::Rect::from_min_size(
            header_column_rect.min + egui::vec2(0.0, RULER_HEIGHT),
            egui::vec2(LAYER_HEADER_WIDTH, header_column_rect.height() - RULER_HEIGHT),
        );

        // Save original clip rect to restore at the end
        let original_clip_rect = ui.clip_rect();

        // Render spacer above layer headers (same height as ruler)
        let spacer_style = shared.theme.style(".timeline-spacer", ui.ctx());
        let spacer_bg = spacer_style.background_color.unwrap_or(egui::Color32::from_rgb(17, 17, 17));
        ui.painter().rect_filled(
            header_ruler_spacer,
            0.0,
            spacer_bg,
        );

        // Render layer header column with clipping
        ui.set_clip_rect(layer_headers_rect.intersect(original_clip_rect));
        self.render_layer_headers(ui, layer_headers_rect, shared.theme, shared.active_layer_id, &mut shared.pending_actions, document);

        // Render time ruler (clip to ruler rect)
        ui.set_clip_rect(ruler_rect.intersect(original_clip_rect));
        self.render_ruler(ui, ruler_rect, shared.theme);

        // Render layer rows with clipping
        ui.set_clip_rect(content_rect.intersect(original_clip_rect));
        self.render_layers(ui, content_rect, shared.theme, document, shared.active_layer_id, shared.selection, shared.midi_event_cache, shared.waveform_cache);

        // Render playhead on top (clip to timeline area)
        ui.set_clip_rect(timeline_rect.intersect(original_clip_rect));
        self.render_playhead(ui, timeline_rect, shared.theme, *shared.playback_time);

        // Restore original clip rect
        ui.set_clip_rect(original_clip_rect);

        // Handle input (use full rect including header column)
        self.handle_input(
            ui,
            rect,
            ruler_rect,
            content_rect,
            layer_headers_rect,
            layer_count,
            document,
            shared.active_layer_id,
            shared.selection,
            shared.pending_actions,
            shared.playback_time,
            shared.is_playing,
            shared.audio_controller,
        );

        // Handle asset drag-and-drop from Asset Library
        if let Some(dragging) = shared.dragging_asset.as_ref() {
            if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                // Check if pointer is in content area (not ruler or header column)
                if content_rect.contains(pointer_pos) {
                    // Calculate which layer the pointer is over
                    let relative_y = pointer_pos.y - content_rect.min.y + self.viewport_scroll_y;
                    let hovered_layer_index = (relative_y / LAYER_HEIGHT) as usize;

                    // Get the layer at this index (accounting for reversed display order)
                    let layers: Vec<_> = document.root.children.iter().rev().collect();

                    if let Some(layer) = layers.get(hovered_layer_index) {
                        let is_compatible = can_drop_on_layer(layer, dragging.clip_type);

                        // Visual feedback: highlight compatible tracks
                        let layer_y = content_rect.min.y + hovered_layer_index as f32 * LAYER_HEIGHT - self.viewport_scroll_y;
                        let highlight_rect = egui::Rect::from_min_size(
                            egui::pos2(content_rect.min.x, layer_y),
                            egui::vec2(content_rect.width(), LAYER_HEIGHT),
                        );

                        let highlight_color = if is_compatible {
                            egui::Color32::from_rgba_unmultiplied(100, 255, 100, 40) // Green
                        } else {
                            egui::Color32::from_rgba_unmultiplied(255, 100, 100, 40) // Red
                        };

                        ui.painter().rect_filled(highlight_rect, 0.0, highlight_color);

                        // Show drop time indicator
                        let drop_time = self.x_to_time(pointer_pos.x - content_rect.min.x);
                        let drop_x = self.time_to_x(drop_time);
                        if drop_x >= 0.0 && drop_x <= content_rect.width() {
                            ui.painter().line_segment(
                                [
                                    egui::pos2(content_rect.min.x + drop_x, layer_y),
                                    egui::pos2(content_rect.min.x + drop_x, layer_y + LAYER_HEIGHT),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::WHITE),
                            );
                        }

                        // Handle drop on mouse release
                        if ui.input(|i| i.pointer.any_released()) && is_compatible {
                            let layer_id = layer.id();
                            let drop_time = self.x_to_time(pointer_pos.x - content_rect.min.x).max(0.0);

                            // Get document dimensions for centering
                            let doc = shared.action_executor.document();
                            let center_x = doc.width / 2.0;
                            let center_y = doc.height / 2.0;

                            // Create clip instance centered on stage, at drop time
                            let clip_instance = ClipInstance::new(dragging.clip_id)
                                .with_timeline_start(drop_time)
                                .with_position(center_x, center_y);

                            // Create and queue action
                            let action = lightningbeam_core::actions::AddClipInstanceAction::new(
                                layer_id,
                                clip_instance,
                            );
                            shared.pending_actions.push(Box::new(action));

                            // Clear drag state
                            *shared.dragging_asset = None;
                        }
                    }
                }
            }
        }

        // Register handler for pending view actions (two-phase dispatch)
        // Priority: Mouse-over (0-99) > Fallback Timeline(1001)
        const TIMELINE_MOUSE_OVER_PRIORITY: u32 = 0;
        const TIMELINE_FALLBACK_PRIORITY: u32 = 1001;

        let mouse_over = ui.rect_contains_pointer(rect);

        // Determine our priority for this action
        let our_priority = if mouse_over {
            TIMELINE_MOUSE_OVER_PRIORITY  // High priority - mouse is over this pane
        } else {
            TIMELINE_FALLBACK_PRIORITY    // Low priority - just a fallback option
        };

        // Check if we should register as a handler (better priority than current best)
        let should_register = shared.pending_view_action.is_some() &&
            shared.fallback_pane_priority.map_or(true, |p| our_priority < p);

        if should_register {
            // Update fallback priority tracker
            *shared.fallback_pane_priority = Some(our_priority);

            // Register as a handler (don't execute yet - that happens after all panes render)
            if let Some(action) = &shared.pending_view_action {
                use crate::menu::MenuAction;

                // Determine zoom center point (use x-position only for timeline horizontal zoom)
                let center = if mouse_over {
                    // Use mouse position for zoom-to-cursor
                    let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                    mouse_pos - rect.min
                } else {
                    // Use center of viewport for fallback
                    rect.size() / 2.0
                };

                // Only register for actions we can handle
                match action {
                    MenuAction::ZoomIn | MenuAction::ZoomOut |
                    MenuAction::ActualSize | MenuAction::RecenterView => {
                        shared.pending_handlers.push(super::ViewActionHandler {
                            priority: our_priority,
                            pane_path: _path.clone(),
                            zoom_center: center,
                        });
                    }
                    _ => {
                        // Not a view action we handle - reset priority so others can try
                        *shared.fallback_pane_priority = None;
                    }
                }
            }
        }
    }

    fn name(&self) -> &str {
        "Timeline"
    }
}

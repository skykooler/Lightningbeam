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
const LOOP_CORNER_SIZE: f32 = 12.0; // Size of loop corner hotzone at top-right of clip
const MIN_CLIP_WIDTH_PX: f32 = 8.0; // Minimum visible width for very short clips (e.g. groups)

/// Calculate vertical bounds for a clip instance within a layer row.
/// For vector layers with multiple clip instances, stacks them vertically.
/// Returns (y_min, y_max) relative to the layer top.
fn clip_instance_y_bounds(
    layer: &AnyLayer,
    clip_index: usize,
    clip_count: usize,
) -> (f32, f32) {
    if matches!(layer, AnyLayer::Vector(_)) && clip_count > 1 {
        let usable_height = LAYER_HEIGHT - 20.0; // 10px padding top/bottom
        let row_height = (usable_height / clip_count as f32).min(20.0);
        let top = 10.0 + clip_index as f32 * row_height;
        (top, top + row_height - 1.0)
    } else {
        (10.0, LAYER_HEIGHT - 10.0)
    }
}

/// Get the effective clip duration for a clip instance on a given layer.
/// For groups on vector layers, the duration spans all consecutive keyframes
/// where the group is present. For regular clips, returns the clip's internal duration.
fn effective_clip_duration(
    document: &lightningbeam_core::document::Document,
    layer: &AnyLayer,
    clip_instance: &ClipInstance,
) -> Option<f64> {
    match layer {
        AnyLayer::Vector(vl) => {
            let vc = document.get_vector_clip(&clip_instance.clip_id)?;
            if vc.is_group {
                let frame_duration = 1.0 / document.framerate;
                let end = vl.group_visibility_end(&clip_instance.id, clip_instance.timeline_start, frame_duration);
                Some((end - clip_instance.timeline_start).max(0.0))
            } else {
                Some(vc.duration)
            }
        }
        AnyLayer::Audio(_) => document.get_audio_clip(&clip_instance.clip_id).map(|c| c.duration),
        AnyLayer::Video(_) => document.get_video_clip(&clip_instance.clip_id).map(|c| c.duration),
        AnyLayer::Effect(_) => Some(lightningbeam_core::effect::EFFECT_DURATION),
    }
}

/// Type of clip drag operation
#[derive(Debug, Clone, Copy, PartialEq)]
enum ClipDragType {
    Move,
    TrimLeft,
    TrimRight,
    LoopExtendRight,
    LoopExtendLeft,
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

    /// Context menu state: Some((optional_clip_instance_id, position)) when a right-click menu is open
    /// clip_id is None when right-clicking on empty timeline space
    context_menu_clip: Option<(Option<uuid::Uuid>, egui::Pos2)>,
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
        (AnyLayer::Effect(_), DragClipType::Effect) => true,
        _ => false,
    }
}

/// Find an existing sampled audio track in the document where a clip can be placed without overlap
/// Returns the layer ID if found, None otherwise
fn find_sampled_audio_track_for_clip(
    document: &lightningbeam_core::document::Document,
    clip_id: uuid::Uuid,
    timeline_start: f64,
) -> Option<uuid::Uuid> {
    // Get the clip duration
    let clip_duration = document.get_clip_duration(&clip_id)?;
    let clip_end = timeline_start + clip_duration;

    // Check each sampled audio layer
    for layer in &document.root.children {
        if let AnyLayer::Audio(audio_layer) = layer {
            if audio_layer.audio_layer_type == AudioLayerType::Sampled {
                // Check if there's any overlap with existing clips on this layer
                let (overlaps, _) = document.check_overlap_on_layer(
                    &audio_layer.layer.id,
                    timeline_start,
                    clip_end,
                    &[], // Don't exclude any instances
                );

                if !overlaps {
                    // Found a suitable layer
                    return Some(audio_layer.layer.id);
                }
            }
        }
    }
    None
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
            context_menu_clip: None,
        }
    }

    /// Execute a view action with the given parameters
    /// Called from main.rs after determining this is the best handler
    #[allow(dead_code)] // Mirrors StagePane; wiring in main.rs pending (see TODO at view action dispatch)
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

    /// Toggle recording on/off
    /// In Auto mode, records to the active audio layer
    fn toggle_recording(&mut self, shared: &mut SharedPaneState) {
        if *shared.is_recording {
            // Stop recording
            self.stop_recording(shared);
        } else {
            // Start recording on active layer
            self.start_recording(shared);
        }
    }

    /// Start recording on the active audio layer
    fn start_recording(&mut self, shared: &mut SharedPaneState) {
        use lightningbeam_core::clip::{AudioClip, ClipInstance};

        let Some(active_layer_id) = *shared.active_layer_id else {
            println!("⚠️  No active layer selected for recording");
            return;
        };

        // Get layer type (copy it so we can drop the document borrow before mutating)
        let layer_type = {
            let document = shared.action_executor.document();
            let Some(layer) = document.root.children.iter().find(|l| l.id() == active_layer_id) else {
                println!("⚠️  Active layer not found in document");
                return;
            };
            let AnyLayer::Audio(audio_layer) = layer else {
                println!("⚠️  Active layer is not an audio layer - cannot record");
                return;
            };
            audio_layer.audio_layer_type
        };

        // Get the backend track ID for this layer
        let Some(&track_id) = shared.layer_to_track_map.get(&active_layer_id) else {
            println!("⚠️  No backend track mapped for layer {}", active_layer_id);
            return;
        };

        let start_time = *shared.playback_time;

        // Start recording based on layer type
        if let Some(controller_arc) = shared.audio_controller {
            let mut controller = controller_arc.lock().unwrap();

            match layer_type {
                AudioLayerType::Midi => {
                    // Create backend MIDI clip and start recording
                    let clip_id = controller.create_midi_clip(track_id, start_time, 0.0);
                    controller.start_midi_recording(track_id, clip_id, start_time);
                    shared.recording_clips.insert(active_layer_id, clip_id);
                    println!("🎹 Started MIDI recording on track {:?} at {:.2}s, clip_id={}",
                             track_id, start_time, clip_id);

                    // Drop controller lock before document mutation
                    drop(controller);

                    // Create document clip + clip instance immediately (clip_id is known synchronously)
                    let doc_clip = AudioClip::new_midi("Recording...", clip_id, 0.0);
                    let doc_clip_id = shared.action_executor.document_mut().add_audio_clip(doc_clip);

                    let clip_instance = ClipInstance::new(doc_clip_id)
                        .with_timeline_start(start_time);

                    if let Some(layer) = shared.action_executor.document_mut().root.children.iter_mut()
                        .find(|l| l.id() == active_layer_id)
                    {
                        if let lightningbeam_core::layer::AnyLayer::Audio(audio_layer) = layer {
                            audio_layer.clip_instances.push(clip_instance);
                        }
                    }

                    // Initialize empty cache entry for this clip
                    shared.midi_event_cache.insert(clip_id, Vec::new());
                }
                AudioLayerType::Sampled => {
                    // For audio recording, backend creates the clip
                    controller.start_recording(track_id, start_time);
                    println!("🎤 Started audio recording on track {:?} at {:.2}s", track_id, start_time);
                    drop(controller);
                }
            }

            // Re-acquire lock for playback start
            if !*shared.is_playing {
                let mut controller = controller_arc.lock().unwrap();
                controller.play();
                *shared.is_playing = true;
                println!("▶ Auto-started playback for recording");
            }

            // Store recording state
            *shared.is_recording = true;
            *shared.recording_start_time = start_time;
            *shared.recording_layer_id = Some(active_layer_id);
        } else {
            println!("⚠️  No audio controller available");
        }
    }

    /// Stop the current recording
    fn stop_recording(&mut self, shared: &mut SharedPaneState) {
        // Determine if this is MIDI or audio recording by checking the layer type
        let is_midi_recording = if let Some(layer_id) = *shared.recording_layer_id {
            shared.action_executor.document().root.children.iter()
                .find(|l| l.id() == layer_id)
                .map(|layer| {
                    if let lightningbeam_core::layer::AnyLayer::Audio(audio_layer) = layer {
                        matches!(audio_layer.audio_layer_type, lightningbeam_core::layer::AudioLayerType::Midi)
                    } else {
                        false
                    }
                })
                .unwrap_or(false)
        } else {
            false
        };

        if let Some(controller_arc) = shared.audio_controller {
            let mut controller = controller_arc.lock().unwrap();

            if is_midi_recording {
                controller.stop_midi_recording();
                println!("🎹 Stopped MIDI recording");
            } else {
                controller.stop_recording();
                println!("🎤 Stopped audio recording");
            }
        }

        // Note: Don't clear recording_layer_id here!
        // The RecordingStopped/MidiRecordingStopped event handler in main.rs
        // needs it to finalize the clip. It will clear the state after processing.
        // Only clear is_recording to update UI state immediately.
        *shared.is_recording = false;
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

        let relative_y = pointer_pos.y - header_rect.min.y + self.viewport_scroll_y;
        let hovered_layer_index = (relative_y / LAYER_HEIGHT) as usize;

        if hovered_layer_index >= layer_count {
            return None;
        }

        let layers: Vec<_> = document.root.children.iter().rev().collect();
        let layer = layers.get(hovered_layer_index)?;
        let _layer_data = layer.layer();

        let clip_instances = match layer {
            lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
            lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
            lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
            lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
        };

        // Check each clip instance
        let clip_count = clip_instances.len();
        for (ci_idx, clip_instance) in clip_instances.iter().enumerate() {
            let clip_duration = effective_clip_duration(document, layer, clip_instance)?;

            let instance_start = clip_instance.effective_start();
            let instance_duration = clip_instance.total_duration(clip_duration);
            let instance_end = instance_start + instance_duration;

            let start_x = self.time_to_x(instance_start);
            let end_x = self.time_to_x(instance_end).max(start_x + MIN_CLIP_WIDTH_PX);
            let mouse_x = pointer_pos.x - content_rect.min.x;

            if mouse_x >= start_x && mouse_x <= end_x {
                // Check vertical bounds for stacked vector layer clips
                let layer_top = header_rect.min.y + (hovered_layer_index as f32 * LAYER_HEIGHT) - self.viewport_scroll_y;
                let (cy_min, cy_max) = clip_instance_y_bounds(layer, ci_idx, clip_count);
                let mouse_rel_y = pointer_pos.y - layer_top;
                if mouse_rel_y < cy_min || mouse_rel_y > cy_max {
                    continue;
                }

                // Determine drag type based on edge proximity (check both sides of edge)
                let is_audio_layer = matches!(layer, lightningbeam_core::layer::AnyLayer::Audio(_));
                let mouse_in_top_corner = pointer_pos.y < layer_top + LOOP_CORNER_SIZE;

                let is_looping = clip_instance.timeline_duration.is_some() || clip_instance.loop_before.is_some();
                let drag_type = if (mouse_x - start_x).abs() <= EDGE_DETECTION_PIXELS {
                    // Left edge: loop extend left for audio clips that are looping or top-left corner
                    let mouse_in_top_left_corner = pointer_pos.y < layer_top + LOOP_CORNER_SIZE;
                    if is_audio_layer && (is_looping || mouse_in_top_left_corner) {
                        ClipDragType::LoopExtendLeft
                    } else {
                        ClipDragType::TrimLeft
                    }
                } else if (end_x - mouse_x).abs() <= EDGE_DETECTION_PIXELS {
                    // If already looping, right edge is always loop extend
                    // Otherwise, top-right corner of audio clips = loop extend
                    if is_audio_layer && (is_looping || mouse_in_top_corner) {
                        ClipDragType::LoopExtendRight
                    } else {
                        ClipDragType::TrimRight
                    }
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
        events: &[(f64, u8, u8, bool)], // (timestamp, note_number, velocity, is_note_on)
        trim_start: f64,
        visible_duration: f64,
        timeline_start: f64,
        viewport_start_time: f64,
        pixels_per_second: f32,
        theme: &crate::theme::Theme,
        ctx: &egui::Context,
        faded: bool,
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
        for &(timestamp, note_number, _velocity, is_note_on) in events {
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
        let render_color = if faded {
            egui::Color32::from_rgba_unmultiplied(note_color.r(), note_color.g(), note_color.b(), note_color.a() / 2)
        } else {
            note_color
        };
        for (note_rect, _note_number) in note_rectangles {
            painter.rect_filled(note_rect, 1.0, render_color);
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
                lightningbeam_core::layer::AnyLayer::Video(_) => ("Video", egui::Color32::from_rgb(180, 100, 255)), // Purple
                lightningbeam_core::layer::AnyLayer::Effect(_) => ("Effect", egui::Color32::from_rgb(255, 100, 180)), // Pink
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
            let mute_response = ui.scope_builder(egui::UiBuilder::new().max_rect(mute_button_rect), |ui| {
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
            let solo_response = ui.scope_builder(egui::UiBuilder::new().max_rect(solo_button_rect), |ui| {
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
            let lock_response = ui.scope_builder(egui::UiBuilder::new().max_rect(lock_button_rect), |ui| {
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
            let volume_response = ui.scope_builder(egui::UiBuilder::new().max_rect(volume_slider_rect), |ui| {
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
    /// Returns video clip hover data for processing after input handling
    fn render_layers(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        theme: &crate::theme::Theme,
        document: &lightningbeam_core::document::Document,
        active_layer_id: &Option<uuid::Uuid>,
        selection: &lightningbeam_core::selection::Selection,
        midi_event_cache: &std::collections::HashMap<u32, Vec<(f64, u8, u8, bool)>>,
        raw_audio_cache: &std::collections::HashMap<usize, (std::sync::Arc<Vec<f32>>, u32, u32)>,
        waveform_gpu_dirty: &mut std::collections::HashSet<usize>,
        target_format: wgpu::TextureFormat,
        waveform_stereo: bool,
    ) -> Vec<(egui::Rect, uuid::Uuid, f64, f64)> {
        let painter = ui.painter();

        // Collect video clip rects for hover detection (to avoid borrow conflicts)
        let mut video_clip_hovers: Vec<(egui::Rect, uuid::Uuid, f64, f64)> = Vec::new();

        // Theme colors for active/inactive layers
        let active_style = theme.style(".timeline-row-active", ui.ctx());
        let inactive_style = theme.style(".timeline-row-inactive", ui.ctx());
        let active_color = active_style.background_color.unwrap_or(egui::Color32::from_rgb(85, 85, 85));
        let inactive_color = inactive_style.background_color.unwrap_or(egui::Color32::from_rgb(136, 136, 136));

        // Build a map of clip_instance_id -> InstanceGroup for linked clip previews
        let mut instance_to_group: std::collections::HashMap<uuid::Uuid, &lightningbeam_core::instance_group::InstanceGroup> = std::collections::HashMap::new();
        for group in document.instance_groups.values() {
            for (_, instance_id) in &group.members {
                instance_to_group.insert(*instance_id, group);
            }
        }

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
                lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
            };

            // For moves, precompute the clamped offset so all selected clips move uniformly
            let group_move_offset = if self.clip_drag_state == Some(ClipDragType::Move) {
                let group: Vec<(uuid::Uuid, f64, f64)> = clip_instances.iter()
                    .filter(|ci| selection.contains_clip_instance(&ci.id))
                    .filter_map(|ci| {
                        let dur = document.get_clip_duration(&ci.clip_id)?;
                        Some((ci.id, ci.effective_start(), ci.total_duration(dur)))
                    })
                    .collect();
                if !group.is_empty() {
                    Some(document.clamp_group_move_offset(&layer.id(), &group, self.drag_offset))
                } else {
                    None
                }
            } else {
                None
            };

            let clip_instance_count = clip_instances.len();
            for (clip_instance_index, clip_instance) in clip_instances.iter().enumerate() {
                // Get the clip to determine duration
                let clip_duration = effective_clip_duration(document, layer, clip_instance);

                if let Some(clip_duration) = clip_duration {
                    // Calculate effective duration accounting for trimming
                    let mut instance_duration = clip_instance.total_duration(clip_duration);

                    // Instance positioned on the layer's timeline using timeline_start
                    // The layer itself has start_time, so the absolute timeline position is:
                    // layer.start_time + instance.timeline_start
                    let _layer_data = layer.layer();
                    let mut instance_start = clip_instance.effective_start();

                    // Apply drag offset preview for selected clips with snapping
                    let is_selected = selection.contains_clip_instance(&clip_instance.id);

                    // Check if this clip is linked to a selected clip being dragged
                    let is_linked_to_dragged = if self.clip_drag_state.is_some() {
                        if let Some(group) = instance_to_group.get(&clip_instance.id) {
                            // Check if any OTHER member of this group is selected
                            group.members.iter().any(|(_, member_id)| {
                                *member_id != clip_instance.id && selection.contains_clip_instance(member_id)
                            })
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // Content origin: where the first "real" content iteration starts
                    // Loop iterations tile outward from this point
                    let mut content_origin = clip_instance.timeline_start;

                    // Track preview trim values for waveform rendering
                    let mut preview_trim_start = clip_instance.trim_start;
                    let mut preview_clip_duration = clip_duration;

                    if let Some(drag_type) = self.clip_drag_state {
                        if is_selected || is_linked_to_dragged {
                            match drag_type {
                                ClipDragType::Move => {
                                    if let Some(offset) = group_move_offset {
                                        instance_start = (clip_instance.effective_start() + offset).max(0.0);
                                        content_origin = instance_start + clip_instance.loop_before.unwrap_or(0.0);
                                    }
                                }
                                ClipDragType::TrimLeft => {
                                    // Trim left: calculate new trim_start with snap to adjacent clips
                                    let desired_trim_start = (clip_instance.trim_start + self.drag_offset)
                                        .max(0.0)
                                        .min(clip_duration);

                                    let new_trim_start = if desired_trim_start < clip_instance.trim_start {
                                        // Extending left - check for adjacent clips
                                        let max_extend = document.find_max_trim_extend_left(
                                            &layer.id(),
                                            &clip_instance.id,
                                            clip_instance.effective_start(),
                                        );

                                        let desired_extend = clip_instance.trim_start - desired_trim_start;
                                        let actual_extend = desired_extend.min(max_extend);
                                        clip_instance.trim_start - actual_extend
                                    } else {
                                        // Shrinking - no snap needed
                                        desired_trim_start
                                    };

                                    let actual_offset = new_trim_start - clip_instance.trim_start;

                                    // Move start and reduce duration by actual clamped offset
                                    instance_start = (clip_instance.timeline_start + actual_offset)
                                        .max(0.0);

                                    instance_duration = (clip_duration - new_trim_start).max(0.0);

                                    // Adjust for existing trim_end
                                    if let Some(trim_end) = clip_instance.trim_end {
                                        instance_duration = (trim_end - new_trim_start).max(0.0);
                                    }

                                    // Update preview trim for waveform rendering
                                    preview_trim_start = new_trim_start;
                                }
                                ClipDragType::TrimRight => {
                                    // Trim right: extend or reduce duration with snap to adjacent clips
                                    let old_trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                    let desired_change = self.drag_offset;
                                    let desired_trim_end = (old_trim_end + desired_change)
                                        .max(clip_instance.trim_start)
                                        .min(clip_duration);

                                    let new_trim_end = if desired_trim_end > old_trim_end {
                                        // Extending right - check for adjacent clips
                                        let current_duration = old_trim_end - clip_instance.trim_start;
                                        let max_extend = document.find_max_trim_extend_right(
                                            &layer.id(),
                                            &clip_instance.id,
                                            clip_instance.timeline_start,
                                            current_duration,
                                        );

                                        let desired_extend = desired_trim_end - old_trim_end;
                                        let actual_extend = desired_extend.min(max_extend);
                                        old_trim_end + actual_extend
                                    } else {
                                        // Shrinking - no snap needed
                                        desired_trim_end
                                    };

                                    instance_duration = (new_trim_end - clip_instance.trim_start).max(0.0);

                                    // Update preview clip duration for waveform rendering
                                    // (the waveform system uses clip_duration to determine visible range)
                                    preview_clip_duration = new_trim_end - preview_trim_start;
                                }
                                ClipDragType::LoopExtendRight => {
                                    // Loop extend right: extend clip beyond content window
                                    let trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                    let content_window = (trim_end - clip_instance.trim_start).max(0.0);
                                    let current_right = clip_instance.timeline_duration.unwrap_or(content_window);
                                    let desired_right = (current_right + self.drag_offset).max(content_window);

                                    let new_right = if desired_right > current_right {
                                        let max_extend = document.find_max_trim_extend_right(
                                            &layer.id(),
                                            &clip_instance.id,
                                            clip_instance.timeline_start,
                                            current_right,
                                        );
                                        let extend_amount = (desired_right - current_right).min(max_extend);
                                        current_right + extend_amount
                                    } else {
                                        desired_right
                                    };

                                    // Total duration = loop_before + right duration
                                    let loop_before = clip_instance.loop_before.unwrap_or(0.0);
                                    instance_duration = loop_before + new_right;
                                }
                                ClipDragType::LoopExtendLeft => {
                                    // Loop extend left: extend loop_before (pre-loop region)
                                    // Snap to multiples of content_window so iterations align with backend
                                    let trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                    let content_window = (trim_end - clip_instance.trim_start).max(0.001);
                                    let current_loop_before = clip_instance.loop_before.unwrap_or(0.0);
                                    // Invert: dragging left (negative offset) = extend
                                    let desired_loop_before = (current_loop_before - self.drag_offset).max(0.0);
                                    // Snap to whole iterations
                                    let desired_iters = (desired_loop_before / content_window).round();
                                    let snapped_loop_before = desired_iters * content_window;

                                    let new_loop_before = if snapped_loop_before > current_loop_before {
                                        // Extending left - check for adjacent clips
                                        let max_extend = document.find_max_loop_extend_left(
                                            &layer.id(),
                                            &clip_instance.id,
                                            clip_instance.effective_start(),
                                        );
                                        let extend_amount = (snapped_loop_before - current_loop_before).min(max_extend);
                                        // Re-snap after clamping
                                        let clamped = current_loop_before + extend_amount;
                                        (clamped / content_window).floor() * content_window
                                    } else {
                                        snapped_loop_before
                                    };

                                    // Recompute instance_start and instance_duration
                                    let right_duration = clip_instance.effective_duration(clip_duration);
                                    instance_start = clip_instance.timeline_start - new_loop_before;
                                    instance_duration = new_loop_before + right_duration;
                                    content_origin = clip_instance.timeline_start;
                                }
                            }
                        }
                    }

                    let instance_end = instance_start + instance_duration;

                    let start_x = self.time_to_x(instance_start);
                    let end_x = self.time_to_x(instance_end).max(start_x + MIN_CLIP_WIDTH_PX);

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
                                egui::Color32::from_rgb(150, 80, 220), // Purple
                                egui::Color32::from_rgb(200, 150, 255), // Bright purple
                            ),
                            lightningbeam_core::layer::AnyLayer::Effect(_) => (
                                egui::Color32::from_rgb(220, 80, 160), // Pink
                                egui::Color32::from_rgb(255, 120, 200), // Bright pink
                            ),
                        };

                        let (cy_min, cy_max) = clip_instance_y_bounds(layer, clip_instance_index, clip_instance_count);

                        let clip_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x + visible_start_x, y + cy_min),
                            egui::pos2(rect.min.x + visible_end_x, y + cy_max),
                        );

                        // Draw the clip instance background(s)
                        // For looping clips, draw each iteration as a separate rounded rect
                        let trim_end_for_bg = clip_instance.trim_end.unwrap_or(clip_duration);
                        let content_window_for_bg = (trim_end_for_bg - clip_instance.trim_start).max(0.0);
                        let is_looping_bg = instance_duration > content_window_for_bg + 0.001 && content_window_for_bg > 0.0;

                        if is_looping_bg {
                            // Compute iterations aligned to content_origin
                            let loop_before_val = content_origin - instance_start;
                            let pre_iters = if loop_before_val > 0.001 {
                                (loop_before_val / content_window_for_bg).ceil() as usize
                            } else {
                                0
                            };
                            let right_duration = instance_duration - loop_before_val;
                            let post_iters = if right_duration > 0.001 {
                                (right_duration / content_window_for_bg).ceil() as usize
                            } else {
                                1
                            };
                            let total_iters = pre_iters + post_iters;

                            let faded_color = egui::Color32::from_rgba_unmultiplied(
                                clip_color.r(), clip_color.g(), clip_color.b(),
                                (clip_color.a() as f32 * 0.55) as u8,
                            );
                            for i in 0..total_iters {
                                let signed_i = i as i64 - pre_iters as i64;
                                let iter_time_start_raw = content_origin + signed_i as f64 * content_window_for_bg;
                                let iter_time_end_raw = iter_time_start_raw + content_window_for_bg;
                                let iter_time_start = iter_time_start_raw.max(instance_start);
                                let iter_time_end = iter_time_end_raw.min(instance_start + instance_duration);
                                if iter_time_end <= iter_time_start { continue; }

                                let ix0 = (rect.min.x + ((iter_time_start - self.viewport_start_time) * self.pixels_per_second as f64) as f32).max(clip_rect.min.x);
                                let ix1 = (rect.min.x + ((iter_time_end - self.viewport_start_time) * self.pixels_per_second as f64) as f32).min(clip_rect.max.x);
                                if ix1 > ix0 {
                                    let iter_rect = egui::Rect::from_min_max(
                                        egui::pos2(ix0, clip_rect.min.y),
                                        egui::pos2(ix1, clip_rect.max.y),
                                    );
                                    let color = if signed_i == 0 { clip_color } else { faded_color };
                                    painter.rect_filled(iter_rect, 3.0, color);
                                }
                            }
                        } else {
                            painter.rect_filled(
                                clip_rect,
                                3.0,
                                clip_color,
                            );
                        }

                        // AUDIO VISUALIZATION: Draw piano roll or waveform overlay
                        if let lightningbeam_core::layer::AnyLayer::Audio(_) = layer {
                            if let Some(clip) = document.get_audio_clip(&clip_instance.clip_id) {
                                match &clip.clip_type {
                                    // MIDI: Draw piano roll (with loop iterations)
                                    lightningbeam_core::clip::AudioClipType::Midi { midi_clip_id } => {
                                        if let Some(events) = midi_event_cache.get(midi_clip_id) {
                                            // Calculate content window for loop detection
                                            let preview_trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                            let content_window = (preview_trim_end - preview_trim_start).max(0.0);
                                            let is_looping = instance_duration > content_window + 0.001;

                                            if is_looping && content_window > 0.0 {
                                                // Compute iterations aligned to content_origin
                                                let lb_val = content_origin - instance_start;
                                                let pre = if lb_val > 0.001 { (lb_val / content_window).ceil() as usize } else { 0 };
                                                let right_dur = instance_duration - lb_val;
                                                let post = if right_dur > 0.001 { (right_dur / content_window).ceil() as usize } else { 1 };

                                                for i in 0..(pre + post) {
                                                    let si = i as i64 - pre as i64;
                                                    let iter_start_raw = content_origin + si as f64 * content_window;
                                                    let iter_end_raw = iter_start_raw + content_window;
                                                    let iter_start = iter_start_raw.max(instance_start);
                                                    let iter_end = iter_end_raw.min(instance_start + instance_duration);
                                                    let iter_duration = iter_end - iter_start;
                                                    if iter_duration <= 0.0 { continue; }

                                                    Self::render_midi_piano_roll(
                                                        painter,
                                                        clip_rect,
                                                        rect.min.x,
                                                        events,
                                                        clip_instance.trim_start,
                                                        iter_duration,
                                                        iter_start,
                                                        self.viewport_start_time,
                                                        self.pixels_per_second,
                                                        theme,
                                                        ui.ctx(),
                                                        si != 0, // fade non-content iterations
                                                    );
                                                }
                                            } else {
                                                Self::render_midi_piano_roll(
                                                    painter,
                                                    clip_rect,
                                                    rect.min.x,
                                                    events,
                                                    clip_instance.trim_start,
                                                    instance_duration,
                                                    instance_start,
                                                    self.viewport_start_time,
                                                    self.pixels_per_second,
                                                    theme,
                                                    ui.ctx(),
                                                    false,
                                                );
                                            }
                                        }
                                    }
                                    // Sampled Audio: Draw waveform via GPU
                                    lightningbeam_core::clip::AudioClipType::Sampled { audio_pool_index } => {
                                        if let Some((samples, sr, ch)) = raw_audio_cache.get(audio_pool_index) {
                                            let total_frames = samples.len() / (*ch).max(1) as usize;
                                            let audio_file_duration = total_frames as f64 / *sr as f64;
                                            let screen_size = ui.ctx().content_rect().size();

                                            let pending_upload = if waveform_gpu_dirty.contains(audio_pool_index) {
                                                waveform_gpu_dirty.remove(audio_pool_index);
                                                Some(crate::waveform_gpu::PendingUpload {
                                                    samples: samples.clone(),
                                                    sample_rate: *sr,
                                                    channels: *ch,
                                                })
                                            } else {
                                                None
                                            };

                                            let tint = [
                                                bright_color.r() as f32 / 255.0,
                                                bright_color.g() as f32 / 255.0,
                                                bright_color.b() as f32 / 255.0,
                                                bright_color.a() as f32 / 255.0,
                                            ];

                                            // Calculate content window for loop detection
                                            // Use trimmed content window (preview_trim_start accounts for TrimLeft drag)
                                            let preview_trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                            let content_window = (preview_trim_end - preview_trim_start).max(0.0);
                                            let is_looping = instance_duration > content_window + 0.001;

                                            // Compute iterations aligned to content_origin
                                            let lb_val = content_origin - instance_start;
                                            let pre_w = if is_looping && lb_val > 0.001 { (lb_val / content_window).ceil() as usize } else { 0 };
                                            let right_dur_w = instance_duration - lb_val;
                                            let post_w = if is_looping && content_window > 0.0 {
                                                (right_dur_w / content_window).ceil() as usize
                                            } else {
                                                1
                                            };
                                            let total_w = pre_w + post_w;

                                            for wi in 0..total_w {
                                                let si_w = wi as i64 - pre_w as i64;
                                                let (iter_start, iter_duration) = if is_looping {
                                                    let raw_start = content_origin + si_w as f64 * content_window;
                                                    let raw_end = raw_start + content_window;
                                                    let s = raw_start.max(instance_start);
                                                    let e = raw_end.min(instance_start + instance_duration);
                                                    (s, (e - s).max(0.0))
                                                } else {
                                                    (instance_start, instance_duration)
                                                };

                                                if iter_duration <= 0.0 { continue; }

                                                let iter_screen_start = rect.min.x + ((iter_start - self.viewport_start_time) * self.pixels_per_second as f64) as f32;
                                                let iter_screen_end = iter_screen_start + (iter_duration * self.pixels_per_second as f64) as f32;
                                                let waveform_rect = egui::Rect::from_min_max(
                                                    egui::pos2(iter_screen_start.max(clip_rect.min.x), clip_rect.min.y),
                                                    egui::pos2(iter_screen_end.min(clip_rect.max.x), clip_rect.max.y),
                                                );

                                                if waveform_rect.width() > 0.0 && waveform_rect.height() > 0.0 {
                                                    let instance_id = clip_instance.id.as_u128() as u64 + wi as u64;
                                                    let is_loop_iter = si_w != 0;
                                                    let callback = crate::waveform_gpu::WaveformCallback {
                                                        pool_index: *audio_pool_index,
                                                        segment_index: 0,
                                                        params: crate::waveform_gpu::WaveformParams {
                                                            clip_rect: [waveform_rect.min.x, waveform_rect.min.y, waveform_rect.max.x, waveform_rect.max.y],
                                                            viewport_start_time: self.viewport_start_time as f32,
                                                            pixels_per_second: self.pixels_per_second as f32,
                                                            audio_duration: audio_file_duration as f32,
                                                            sample_rate: *sr as f32,
                                                            clip_start_time: iter_screen_start,
                                                            trim_start: preview_trim_start as f32,
                                                            tex_width: crate::waveform_gpu::tex_width() as f32,
                                                            total_frames: total_frames as f32,
                                                            segment_start_frame: 0.0,
                                                            display_mode: if waveform_stereo { 1.0 } else { 0.0 },
                                                            _pad1: [0.0, 0.0],
                                                            tint_color: if is_loop_iter {
                                                                [tint[0], tint[1], tint[2], tint[3] * 0.5]
                                                            } else {
                                                                tint
                                                            },
                                                            screen_size: [screen_size.x, screen_size.y],
                                                            _pad: [0.0, 0.0],
                                                        },
                                                        target_format,
                                                        pending_upload: if wi == 0 { pending_upload.clone() } else { None },
                                                        instance_id,
                                                    };

                                                    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                                                        waveform_rect,
                                                        callback,
                                                    ));
                                                }

                                            }
                                        }
                                    }
                                    // Recording in progress: show live waveform
                                    lightningbeam_core::clip::AudioClipType::Recording => {
                                        let rec_pool_idx = usize::MAX;
                                        if let Some((samples, sr, ch)) = raw_audio_cache.get(&rec_pool_idx) {
                                            let total_frames = samples.len() / (*ch).max(1) as usize;
                                            if total_frames > 0 {
                                                let audio_file_duration = total_frames as f64 / *sr as f64;
                                                let screen_size = ui.ctx().content_rect().size();

                                                let pending_upload = if waveform_gpu_dirty.contains(&rec_pool_idx) {
                                                    waveform_gpu_dirty.remove(&rec_pool_idx);
                                                    Some(crate::waveform_gpu::PendingUpload {
                                                        samples: samples.clone(),
                                                        sample_rate: *sr,
                                                        channels: *ch,
                                                    })
                                                } else {
                                                    None
                                                };

                                                let tint = [
                                                    bright_color.r() as f32 / 255.0,
                                                    bright_color.g() as f32 / 255.0,
                                                    bright_color.b() as f32 / 255.0,
                                                    bright_color.a() as f32 / 255.0,
                                                ];

                                                let clip_screen_start = rect.min.x + ((instance_start - self.viewport_start_time) * self.pixels_per_second as f64) as f32;
                                                let clip_screen_end = clip_screen_start + (preview_clip_duration * self.pixels_per_second as f64) as f32;
                                                let waveform_rect = egui::Rect::from_min_max(
                                                    egui::pos2(clip_screen_start.max(clip_rect.min.x), clip_rect.min.y),
                                                    egui::pos2(clip_screen_end.min(clip_rect.max.x), clip_rect.max.y),
                                                );

                                                if waveform_rect.width() > 0.0 && waveform_rect.height() > 0.0 {
                                                    let instance_id = clip_instance.id.as_u128() as u64;
                                                    let callback = crate::waveform_gpu::WaveformCallback {
                                                        pool_index: rec_pool_idx,
                                                        segment_index: 0,
                                                        params: crate::waveform_gpu::WaveformParams {
                                                            clip_rect: [waveform_rect.min.x, waveform_rect.min.y, waveform_rect.max.x, waveform_rect.max.y],
                                                            viewport_start_time: self.viewport_start_time as f32,
                                                            pixels_per_second: self.pixels_per_second as f32,
                                                            audio_duration: audio_file_duration as f32,
                                                            sample_rate: *sr as f32,
                                                            clip_start_time: clip_screen_start,
                                                            trim_start: preview_trim_start as f32,
                                                            tex_width: crate::waveform_gpu::tex_width() as f32,
                                                            total_frames: total_frames as f32,
                                                            segment_start_frame: 0.0,
                                                            display_mode: if waveform_stereo { 1.0 } else { 0.0 },
                                                            _pad1: [0.0, 0.0],
                                                            tint_color: tint,
                                                            screen_size: [screen_size.x, screen_size.y],
                                                            _pad: [0.0, 0.0],
                                                        },
                                                        target_format,
                                                        pending_upload,
                                                        instance_id,
                                                    };

                                                    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                                                        waveform_rect,
                                                        callback,
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // VIDEO PREVIEW: Collect clip rect for hover detection
                        if let lightningbeam_core::layer::AnyLayer::Video(_) = layer {
                            video_clip_hovers.push((clip_rect, clip_instance.clip_id, clip_instance.trim_start, instance_start));
                        }

                        // Draw border per segment (per loop iteration for looping clips)
                        {
                            let is_selected = selection.contains_clip_instance(&clip_instance.id);
                            let border_stroke = if is_selected {
                                egui::Stroke::new(3.0, bright_color)
                            } else {
                                let dark_border = egui::Color32::from_rgb(
                                    clip_color.r() / 2,
                                    clip_color.g() / 2,
                                    clip_color.b() / 2,
                                );
                                egui::Stroke::new(1.0, dark_border)
                            };

                            if is_looping_bg {
                                // Aligned to content_origin (same as bg rendering)
                                let lb_border = content_origin - instance_start;
                                let pre_b = if lb_border > 0.001 { (lb_border / content_window_for_bg).ceil() as usize } else { 0 };
                                let right_b = instance_duration - lb_border;
                                let post_b = if right_b > 0.001 { (right_b / content_window_for_bg).ceil() as usize } else { 1 };
                                for i in 0..(pre_b + post_b) {
                                    let si_b = i as i64 - pre_b as i64;
                                    let iter_time_start_raw = content_origin + si_b as f64 * content_window_for_bg;
                                    let iter_time_end_raw = iter_time_start_raw + content_window_for_bg;
                                    let iter_time_start = iter_time_start_raw.max(instance_start);
                                    let iter_time_end = iter_time_end_raw.min(instance_start + instance_duration);
                                    if iter_time_end <= iter_time_start { continue; }
                                    let ix0 = (rect.min.x + ((iter_time_start - self.viewport_start_time) * self.pixels_per_second as f64) as f32).max(clip_rect.min.x);
                                    let ix1 = (rect.min.x + ((iter_time_end - self.viewport_start_time) * self.pixels_per_second as f64) as f32).min(clip_rect.max.x);
                                    if ix1 > ix0 {
                                        let iter_rect = egui::Rect::from_min_max(
                                            egui::pos2(ix0, clip_rect.min.y),
                                            egui::pos2(ix1, clip_rect.max.y),
                                        );
                                        painter.rect_stroke(iter_rect, 3.0, border_stroke, egui::StrokeKind::Middle);
                                    }
                                }
                            } else {
                                painter.rect_stroke(clip_rect, 3.0, border_stroke, egui::StrokeKind::Middle);
                            }
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

            // Draw shape keyframe markers for vector layers
            if let lightningbeam_core::layer::AnyLayer::Vector(vl) = layer {
                for kf in &vl.keyframes {
                    let x = self.time_to_x(kf.time);
                    if x >= 0.0 && x <= rect.width() {
                        let cx = rect.min.x + x;
                        let cy = y + LAYER_HEIGHT - 8.0;
                        let size = 5.0;
                        // Draw diamond shape
                        let diamond = [
                            egui::pos2(cx, cy - size),
                            egui::pos2(cx + size, cy),
                            egui::pos2(cx, cy + size),
                            egui::pos2(cx - size, cy),
                        ];
                        let color = egui::Color32::from_rgb(255, 220, 100);
                        painter.add(egui::Shape::convex_polygon(
                            diamond.to_vec(),
                            color,
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 150, 50)),
                        ));
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

        // Return video clip hover data for processing after input handling
        video_clip_hovers
    }

    /// Handle mouse input for scrubbing, panning, zooming, layer selection, and clip instance selection
    fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        _full_timeline_rect: egui::Rect,
        ruler_rect: egui::Rect,
        content_rect: egui::Rect,
        header_rect: egui::Rect,
        layer_count: usize,
        document: &lightningbeam_core::document::Document,
        active_layer_id: &mut Option<uuid::Uuid>,
        selection: &mut lightningbeam_core::selection::Selection,
        pending_actions: &mut Vec<Box<dyn lightningbeam_core::action::Action>>,
        playback_time: &mut f64,
        _is_playing: &mut bool,
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
                    // Get the layer at this index (accounting for reversed display order)
                    if clicked_layer_index < layer_count {
                        let layers: Vec<_> = document.root.children.iter().rev().collect();
                        if let Some(layer) = layers.get(clicked_layer_index) {
                            let _layer_data = layer.layer();

                            // Get clip instances for this layer
                            let clip_instances = match layer {
                                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
                            };

                            // Check if click is within any clip instance
                            let click_clip_count = clip_instances.len();
                            let click_layer_top = pos.y - (relative_y % LAYER_HEIGHT);
                            for (ci_idx, clip_instance) in clip_instances.iter().enumerate() {
                                let clip_duration = effective_clip_duration(document, layer, clip_instance);

                                if let Some(clip_duration) = clip_duration {
                                    let instance_duration = clip_instance.total_duration(clip_duration);
                                    let instance_start = clip_instance.effective_start();
                                    let instance_end = instance_start + instance_duration;

                                    // Check if click is within this clip instance's pixel range and vertical bounds
                                    let ci_start_x = self.time_to_x(instance_start);
                                    let ci_end_x = self.time_to_x(instance_end).max(ci_start_x + MIN_CLIP_WIDTH_PX);
                                    let click_x = pos.x - content_rect.min.x;
                                    let (cy_min, cy_max) = clip_instance_y_bounds(layer, ci_idx, click_clip_count);
                                    let click_rel_y = pos.y - click_layer_top;
                                    if click_x >= ci_start_x && click_x <= ci_end_x
                                        && click_rel_y >= cy_min && click_rel_y <= cy_max
                                    {
                                        // Found a clicked clip instance!
                                        if shift_held {
                                            // Shift+click: add to selection
                                            selection.add_clip_instance(clip_instance.id);
                                        } else {
                                            // Regular click: select only this clip
                                            selection.select_only_clip_instance(clip_instance.id);
                                        }
                                        // Also set this layer as the active layer
                                        *active_layer_id = Some(layer.id());
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
                        lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
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
                                let _layer_data = layer.layer();

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
                                    lightningbeam_core::layer::AnyLayer::Effect(el) => {
                                        &el.clip_instances
                                    }
                                };

                                // Find selected clip instances in this layer
                                for clip_instance in clip_instances {
                                    if selection.contains_clip_instance(&clip_instance.id) {
                                        let clip_duration = effective_clip_duration(document, layer, clip_instance);

                                        if let Some(clip_duration) = clip_duration {
                                            match drag_type {
                                                ClipDragType::TrimLeft => {
                                                    let old_trim_start = clip_instance.trim_start;
                                                    let old_timeline_start =
                                                        clip_instance.timeline_start;

                                                    // New trim_start is clamped to valid range
                                                    let desired_trim_start = (old_trim_start
                                                        + self.drag_offset)
                                                        .max(0.0)
                                                        .min(clip_duration);

                                                    // Apply overlap prevention when extending left
                                                    let new_trim_start = if desired_trim_start < old_trim_start {
                                                        let max_extend = document.find_max_trim_extend_left(
                                                            &layer_id,
                                                            &clip_instance.id,
                                                            old_timeline_start,
                                                        );
                                                        let desired_extend = old_trim_start - desired_trim_start;
                                                        let actual_extend = desired_extend.min(max_extend);
                                                        old_trim_start - actual_extend
                                                    } else {
                                                        desired_trim_start
                                                    };

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
                                                    let old_trim_end_val = clip_instance.trim_end.unwrap_or(clip_duration);
                                                    let desired_trim_end = (old_trim_end_val + self.drag_offset)
                                                        .max(clip_instance.trim_start)
                                                        .min(clip_duration);

                                                    // Apply overlap prevention when extending right
                                                    let new_trim_end_val = if desired_trim_end > old_trim_end_val {
                                                        let max_extend = document.find_max_trim_extend_right(
                                                            &layer_id,
                                                            &clip_instance.id,
                                                            clip_instance.timeline_start,
                                                            current_duration,
                                                        );
                                                        let desired_extend = desired_trim_end - old_trim_end_val;
                                                        let actual_extend = desired_extend.min(max_extend);
                                                        old_trim_end_val + actual_extend
                                                    } else {
                                                        desired_trim_end
                                                    };

                                                    let new_duration = (new_trim_end_val - clip_instance.trim_start).max(0.0);

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
                        ClipDragType::LoopExtendRight => {
                            let mut layer_loops: HashMap<uuid::Uuid, Vec<lightningbeam_core::actions::loop_clip_instances::LoopEntry>> = HashMap::new();

                            for layer in &document.root.children {
                                let layer_id = layer.id();
                                let clip_instances = match layer {
                                    lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                                    lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                                    lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                                    lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
                                };

                                for clip_instance in clip_instances {
                                    if selection.contains_clip_instance(&clip_instance.id) {
                                        let clip_duration = match layer {
                                            lightningbeam_core::layer::AnyLayer::Audio(_) => {
                                                document.get_audio_clip(&clip_instance.clip_id).map(|c| c.duration)
                                            }
                                            _ => continue,
                                        };

                                        if let Some(clip_duration) = clip_duration {
                                            let trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                            let content_window = (trim_end - clip_instance.trim_start).max(0.0);
                                            let current_right = clip_instance.timeline_duration.unwrap_or(content_window);
                                            let desired_right = current_right + self.drag_offset;

                                            let new_right = if desired_right > current_right {
                                                let max_extend = document.find_max_trim_extend_right(
                                                    &layer_id,
                                                    &clip_instance.id,
                                                    clip_instance.timeline_start,
                                                    current_right,
                                                );
                                                let extend_amount = (desired_right - current_right).min(max_extend);
                                                current_right + extend_amount
                                            } else {
                                                desired_right
                                            };

                                            let old_timeline_duration = clip_instance.timeline_duration;
                                            let new_timeline_duration = if new_right > content_window + 0.001 {
                                                Some(new_right)
                                            } else {
                                                None
                                            };

                                            if old_timeline_duration != new_timeline_duration {
                                                layer_loops
                                                    .entry(layer_id)
                                                    .or_insert_with(Vec::new)
                                                    .push((
                                                        clip_instance.id,
                                                        old_timeline_duration,
                                                        new_timeline_duration,
                                                        clip_instance.loop_before,
                                                        clip_instance.loop_before, // loop_before unchanged
                                                    ));
                                            }
                                        }
                                    }
                                }
                            }

                            if !layer_loops.is_empty() {
                                let action = Box::new(
                                    lightningbeam_core::actions::LoopClipInstancesAction::new(layer_loops),
                                );
                                pending_actions.push(action);
                            }
                        }
                        ClipDragType::LoopExtendLeft => {
                            // Extend loop_before (pre-loop region)
                            let mut layer_loops: HashMap<uuid::Uuid, Vec<lightningbeam_core::actions::loop_clip_instances::LoopEntry>> = HashMap::new();

                            for layer in &document.root.children {
                                let layer_id = layer.id();
                                let clip_instances = match layer {
                                    lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                                    lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                                    lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                                    lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
                                };

                                for clip_instance in clip_instances {
                                    if selection.contains_clip_instance(&clip_instance.id) {
                                        let clip_duration = match layer {
                                            lightningbeam_core::layer::AnyLayer::Audio(_) => {
                                                document.get_audio_clip(&clip_instance.clip_id).map(|c| c.duration)
                                            }
                                            _ => continue,
                                        };

                                        if let Some(clip_duration) = clip_duration {
                                            let trim_end = clip_instance.trim_end.unwrap_or(clip_duration);
                                            let content_window = (trim_end - clip_instance.trim_start).max(0.001);
                                            let current_loop_before = clip_instance.loop_before.unwrap_or(0.0);
                                            // Invert: dragging left (negative offset) = extend
                                            let desired_loop_before = (current_loop_before - self.drag_offset).max(0.0);
                                            // Snap to whole iterations so backend modulo aligns
                                            let desired_iters = (desired_loop_before / content_window).round();
                                            let snapped = desired_iters * content_window;

                                            let new_loop_before = if snapped > current_loop_before {
                                                let max_extend = document.find_max_loop_extend_left(
                                                    &layer_id,
                                                    &clip_instance.id,
                                                    clip_instance.effective_start(),
                                                );
                                                let extend_amount = (snapped - current_loop_before).min(max_extend);
                                                let clamped = current_loop_before + extend_amount;
                                                (clamped / content_window).floor() * content_window
                                            } else {
                                                snapped
                                            };

                                            let old_loop_before = clip_instance.loop_before;
                                            let new_lb = if new_loop_before > 0.001 {
                                                Some(new_loop_before)
                                            } else {
                                                None
                                            };

                                            if old_loop_before != new_lb {
                                                layer_loops
                                                    .entry(layer_id)
                                                    .or_insert_with(Vec::new)
                                                    .push((
                                                        clip_instance.id,
                                                        clip_instance.timeline_duration,
                                                        clip_instance.timeline_duration, // timeline_duration unchanged
                                                        old_loop_before,
                                                        new_lb,
                                                    ));
                                            }
                                        }
                                    }
                                }
                            }

                            if !layer_loops.is_empty() {
                                let action = Box::new(
                                    lightningbeam_core::actions::LoopClipInstancesAction::new(layer_loops),
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
                // Seek immediately so it works while playing
                if let Some(controller_arc) = audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.seek(new_time);
                }
            }
        }
        // Continue scrubbing while dragging, even if cursor leaves ruler
        else if self.is_scrubbing && response.dragged() && !self.is_panning {
            if let Some(pos) = response.interact_pointer_pos() {
                let x = (pos.x - content_rect.min.x).max(0.0);
                let new_time = self.x_to_time(x).max(0.0);
                *playback_time = new_time;
                if let Some(controller_arc) = audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.seek(new_time);
                }
            }
        }
        // Stop scrubbing when drag ends
        else if !response.dragged() && self.is_scrubbing {
            self.is_scrubbing = false;
        }

        // Distinguish between mouse wheel (discrete) and trackpad (smooth)
        // Only handle scroll when mouse is over the timeline area
        let mut handled = false;
        let pointer_over_timeline = response.hovered() || ui.rect_contains_pointer(header_rect);
        if pointer_over_timeline { ui.input(|i| {
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
        }); }

        // Handle scroll_delta for trackpad panning (when Ctrl not held)
        if pointer_over_timeline && !handled {
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
            // If dragging a clip with trim/loop, keep the appropriate cursor
            if let Some(drag_type) = self.clip_drag_state {
                match drag_type {
                    ClipDragType::TrimLeft | ClipDragType::TrimRight => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    ClipDragType::LoopExtendRight | ClipDragType::LoopExtendLeft => {
                        crate::custom_cursor::set(ui.ctx(), crate::custom_cursor::CustomCursor::LoopExtend);
                    }
                    ClipDragType::Move => {}
                }
            } else if let Some(hover_pos) = response.hover_pos() {
                // Not dragging - detect hover for cursor feedback
                if let Some((drag_type, _clip_id)) = self.detect_clip_at_pointer(
                    hover_pos,
                    document,
                    content_rect,
                    header_rect,
                ) {
                    match drag_type {
                        ClipDragType::TrimLeft | ClipDragType::TrimRight => {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        ClipDragType::LoopExtendRight | ClipDragType::LoopExtendLeft => {
                            crate::custom_cursor::set(ui.ctx(), crate::custom_cursor::CustomCursor::LoopExtend);
                        }
                        ClipDragType::Move => {}
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
                    // If pausing while recording, stop recording first
                    if *shared.is_playing && *shared.is_recording {
                        self.stop_recording(shared);
                        println!("⏹ Stopped recording (playback paused)");
                    }

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

                // Small separator before record button
                ui.add_space(8.0);

                // Record button - red circle, pulsing when recording
                let record_color = if *shared.is_recording {
                    // Pulsing red when recording (vary alpha based on time)
                    let pulse = (ui.ctx().input(|i| i.time) * 2.0).sin() * 0.3 + 0.7;
                    egui::Color32::from_rgba_unmultiplied(220, 50, 50, (pulse * 255.0) as u8)
                } else {
                    egui::Color32::from_rgb(180, 60, 60)
                };

                let record_button = egui::Button::new(
                    egui::RichText::new("⏺").color(record_color).size(16.0)
                );

                if ui.add_sized(button_size, record_button).clicked() {
                    self.toggle_recording(shared);
                }

                // Request repaint while recording for pulse animation
                if *shared.is_recording {
                    ui.ctx().request_repaint();
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
                lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
            };

            for clip_instance in clip_instances {
                let clip_duration = effective_clip_duration(document, layer, clip_instance);

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
        let video_clip_hovers = self.render_layers(ui, content_rect, shared.theme, document, shared.active_layer_id, shared.selection, shared.midi_event_cache, shared.raw_audio_cache, shared.waveform_gpu_dirty, shared.target_format, shared.waveform_stereo);

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

        // Context menu: detect right-click on clips or empty timeline space
        let mut just_opened_menu = false;
        let secondary_clicked = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
        if secondary_clicked {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if content_rect.contains(pos) {
                    if let Some((_drag_type, clip_id)) = self.detect_clip_at_pointer(pos, document, content_rect, layer_headers_rect) {
                        // Right-clicked on a clip
                        if !shared.selection.contains_clip_instance(&clip_id) {
                            shared.selection.select_only_clip_instance(clip_id);
                        }
                        self.context_menu_clip = Some((Some(clip_id), pos));
                    } else {
                        // Right-clicked on empty timeline space
                        self.context_menu_clip = Some((None, pos));
                    }
                    just_opened_menu = true;
                }
            }
        }

        // Render context menu
        if let Some((ctx_clip_id, menu_pos)) = self.context_menu_clip {
            let has_clip = ctx_clip_id.is_some();
            // Determine which items are enabled
            let playback_time = *shared.playback_time;
            let min_split_px = 4.0_f32;

            // Split: playhead must be over a selected clip, at least min_split_px from edges
            let split_enabled = has_clip && {
                let mut enabled = false;
                if let Some(layer_id) = *shared.active_layer_id {
                    if let Some(layer) = document.get_layer(&layer_id) {
                        let instances: &[ClipInstance] = match layer {
                            AnyLayer::Vector(vl) => &vl.clip_instances,
                            AnyLayer::Audio(al) => &al.clip_instances,
                            AnyLayer::Video(vl) => &vl.clip_instances,
                            AnyLayer::Effect(el) => &el.clip_instances,
                        };
                        for inst in instances {
                            if !shared.selection.contains_clip_instance(&inst.id) { continue; }
                            if let Some(dur) = document.get_clip_duration(&inst.clip_id) {
                                let eff = inst.effective_duration(dur);
                                let start = inst.timeline_start;
                                let end = start + eff;
                                let min_dist = min_split_px as f64 / self.pixels_per_second as f64;
                                if playback_time > start + min_dist && playback_time < end - min_dist {
                                    enabled = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                enabled
            };

            // Duplicate: check if there's room to the right of each selected clip
            let duplicate_enabled = has_clip && {
                let mut enabled = false;
                if let Some(layer_id) = *shared.active_layer_id {
                    if let Some(layer) = document.get_layer(&layer_id) {
                        let instances: &[ClipInstance] = match layer {
                            AnyLayer::Vector(vl) => &vl.clip_instances,
                            AnyLayer::Audio(al) => &al.clip_instances,
                            AnyLayer::Video(vl) => &vl.clip_instances,
                            AnyLayer::Effect(el) => &el.clip_instances,
                        };
                        // Check each selected clip
                        enabled = instances.iter()
                            .filter(|ci| shared.selection.contains_clip_instance(&ci.id))
                            .all(|ci| {
                                if let Some(dur) = document.get_clip_duration(&ci.clip_id) {
                                    let eff = ci.effective_duration(dur);
                                    let max_extend = document.find_max_trim_extend_right(
                                        &layer_id, &ci.id, ci.timeline_start, eff,
                                    );
                                    max_extend >= eff
                                } else {
                                    false
                                }
                            })
                            && instances.iter().any(|ci| shared.selection.contains_clip_instance(&ci.id));
                    }
                }
                enabled
            };

            // Paste: check if clipboard has content and there's room at playhead
            let paste_enabled = {
                let mut enabled = false;
                if shared.clipboard_manager.has_content() {
                    if let Some(layer_id) = *shared.active_layer_id {
                        if let Some(content) = shared.clipboard_manager.paste() {
                            if let lightningbeam_core::clipboard::ClipboardContent::ClipInstances {
                                ref layer_type,
                                ref instances,
                                ..
                            } = content
                            {
                                if let Some(layer) = document.get_layer(&layer_id) {
                                    if layer_type.is_compatible(layer) && !instances.is_empty() {
                                        // Check if each pasted clip would fit at playhead
                                        let min_start = instances
                                            .iter()
                                            .map(|i| i.timeline_start)
                                            .fold(f64::INFINITY, f64::min);
                                        let offset = *shared.playback_time - min_start;

                                        enabled = instances.iter().all(|ci| {
                                            let paste_start = (ci.timeline_start + offset).max(0.0);
                                            if let Some(dur) = document.get_clip_duration(&ci.clip_id) {
                                                let eff = ci.effective_duration(dur);
                                                document
                                                    .find_nearest_valid_position(
                                                        &layer_id,
                                                        paste_start,
                                                        eff,
                                                        &[],
                                                    )
                                                    .is_some()
                                            } else {
                                                // Clip def not in document yet (from external paste) — allow
                                                true
                                            }
                                        });
                                    }
                                }
                            } else {
                                // Shapes paste — always enabled if layer is vector
                                if let Some(layer) = document.get_layer(&layer_id) {
                                    enabled = matches!(layer, AnyLayer::Vector(_));
                                }
                            }
                        }
                    }
                }
                enabled
            };

            let area_id = ui.id().with("clip_context_menu");
            let mut item_clicked = false;
            let area_response = egui::Area::new(area_id)
                .order(egui::Order::Foreground)
                .fixed_pos(menu_pos)
                .interactable(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(160.0);

                        // Helper: full-width menu item with optional enabled state
                        let menu_item = |ui: &mut egui::Ui, label: &str, enabled: bool| -> bool {
                            let desired_width = ui.available_width();
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(desired_width, ui.spacing().interact_size.y),
                                if enabled { egui::Sense::click() } else { egui::Sense::hover() },
                            );
                            if ui.is_rect_visible(rect) {
                                if enabled && response.hovered() {
                                    ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                                }
                                let text_color = if !enabled {
                                    ui.visuals().weak_text_color()
                                } else if response.hovered() {
                                    ui.visuals().widgets.hovered.text_color()
                                } else {
                                    ui.visuals().widgets.inactive.text_color()
                                };
                                ui.painter().text(
                                    rect.min + egui::vec2(4.0, (rect.height() - 14.0) / 2.0),
                                    egui::Align2::LEFT_TOP,
                                    label,
                                    egui::FontId::proportional(14.0),
                                    text_color,
                                );
                            }
                            enabled && response.clicked()
                        };

                        if menu_item(ui, "Split Clip", split_enabled) {
                            shared.pending_menu_actions.push(crate::menu::MenuAction::SplitClip);
                            item_clicked = true;
                        }
                        if menu_item(ui, "Duplicate Clip", duplicate_enabled) {
                            shared.pending_menu_actions.push(crate::menu::MenuAction::DuplicateClip);
                            item_clicked = true;
                        }
                        ui.separator();
                        if menu_item(ui, "Cut", has_clip) {
                            shared.pending_menu_actions.push(crate::menu::MenuAction::Cut);
                            item_clicked = true;
                        }
                        if menu_item(ui, "Copy", has_clip) {
                            shared.pending_menu_actions.push(crate::menu::MenuAction::Copy);
                            item_clicked = true;
                        }
                        if menu_item(ui, "Paste", paste_enabled) {
                            shared.pending_menu_actions.push(crate::menu::MenuAction::Paste);
                            item_clicked = true;
                        }
                        ui.separator();
                        if menu_item(ui, "Delete", has_clip) {
                            shared.pending_menu_actions.push(crate::menu::MenuAction::Delete);
                            item_clicked = true;
                        }
                    });
                });

            // Close on item click or click outside (skip on the frame we just opened)
            if !just_opened_menu {
                let any_click = ui.input(|i| {
                    i.pointer.button_clicked(egui::PointerButton::Primary)
                        || i.pointer.button_clicked(egui::PointerButton::Secondary)
                });
                if item_clicked || (any_click && !area_response.response.contains_pointer()) {
                    self.context_menu_clip = None;
                }
            }
        }

        // VIDEO HOVER DETECTION: Handle video clip hover tooltips AFTER input handling
        // This ensures hover events aren't consumed by the main input handler
        for (clip_rect, clip_id, trim_start, instance_start) in video_clip_hovers {
            let hover_response = ui.allocate_rect(clip_rect, egui::Sense::hover());

            if hover_response.hovered() {
                if let Some(hover_pos) = hover_response.hover_pos() {
                    // Calculate timestamp at hover position
                    let hover_offset_pixels = hover_pos.x - clip_rect.min.x;
                    let hover_offset_time = (hover_offset_pixels as f64) / (self.pixels_per_second as f64);
                    let hover_timestamp = instance_start + hover_offset_time;

                    // Remap to clip content time accounting for trim
                    let clip_content_time = trim_start + (hover_timestamp - instance_start);

                    // Try to get thumbnail from video manager
                    let thumbnail_data: Option<(u32, u32, std::sync::Arc<Vec<u8>>)> = {
                        let video_mgr = shared.video_manager.lock().unwrap();
                        video_mgr.get_thumbnail_at(&clip_id, clip_content_time)
                    };

                    if let Some((thumb_width, thumb_height, ref thumb_data)) = thumbnail_data {
                        // Create texture from thumbnail
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(
                            [thumb_width as usize, thumb_height as usize],
                            &thumb_data,
                        );
                        let texture = ui.ctx().load_texture(
                            format!("video_hover_{}", clip_id),
                            color_image,
                            egui::TextureOptions::LINEAR,
                        );

                        // Show tooltip with thumbnail positioned near cursor
                        let tooltip_pos = hover_pos + egui::vec2(10.0, 10.0);
                        egui::Area::new(egui::Id::new(format!("video_hover_tooltip_{}", clip_id)))
                            .fixed_pos(tooltip_pos)
                            .order(egui::Order::Tooltip)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style())
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.image(&texture);
                                            ui.label(format!("Time: {:.2}s", clip_content_time));
                                        });
                                    });
                            });
                    } else {
                        // Show simple tooltip if no thumbnail available
                        let tooltip_pos = hover_pos + egui::vec2(10.0, 10.0);
                        egui::Area::new(egui::Id::new(format!("video_tooltip_{}", clip_id)))
                            .fixed_pos(tooltip_pos)
                            .order(egui::Order::Tooltip)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style())
                                    .show(ui, |ui| {
                                        ui.label(format!("Video clip\nTime: {:.2}s\n(Thumbnails generating...)", clip_content_time));
                                    });
                            });
                    }
                }
            }
        }

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

                        // Show drop time indicator with snap preview
                        let raw_drop_time = self.x_to_time(pointer_pos.x - content_rect.min.x).max(0.0);

                        // Calculate snapped drop time for preview
                        let drop_time = if is_compatible {
                            // Get clip duration to calculate snapped position
                            let clip_duration = {
                                let doc = shared.action_executor.document();
                                doc.get_clip_duration(&dragging.clip_id).unwrap_or(1.0)
                            };

                            // Find nearest valid position (auto-snap for preview)
                            let snapped = shared.action_executor.document()
                                .find_nearest_valid_position(
                                    &layer.id(),
                                    raw_drop_time,
                                    clip_duration,
                                    &[],
                                );

                            snapped.unwrap_or(raw_drop_time)
                        } else {
                            raw_drop_time
                        };

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

                            // Handle effect drops specially
                            if dragging.clip_type == DragClipType::Effect {
                                // Get effect definition from registry or document
                                let effect_def = lightningbeam_core::effect_registry::EffectRegistry::get_by_id(&dragging.clip_id)
                                    .or_else(|| shared.action_executor.document().get_effect_definition(&dragging.clip_id).cloned());

                                if let Some(def) = effect_def {
                                    // Ensure effect definition is in document (copy from registry if built-in)
                                    if shared.action_executor.document().get_effect_definition(&def.id).is_none() {
                                        shared.action_executor.document_mut().add_effect_definition(def.clone());
                                    }

                                    // Create clip instance for effect with 5 second default duration
                                    let clip_instance = ClipInstance::new(def.id)
                                        .with_timeline_start(drop_time)
                                        .with_timeline_duration(5.0);

                                    // Use AddEffectAction for effect layers
                                    let action = lightningbeam_core::actions::AddEffectAction::new(
                                        layer_id,
                                        clip_instance,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }

                                // Clear drag state
                                *shared.dragging_asset = None;
                            } else {
                                // Get document dimensions for centering and create clip instance
                                let (_center_x, _center_y, clip_instance) = {
                                    let doc = shared.action_executor.document();
                                    let center_x = doc.width / 2.0;
                                    let center_y = doc.height / 2.0;

                                    let mut clip_instance = ClipInstance::new(dragging.clip_id)
                                        .with_timeline_start(drop_time);

                                    // For video clips, scale to fill document dimensions
                                    if dragging.clip_type == DragClipType::Video {
                                        if let Some((video_width, video_height)) = dragging.dimensions {
                                            // Calculate scale to fill document
                                            let scale_x = doc.width / video_width;
                                            let scale_y = doc.height / video_height;

                                            clip_instance.transform.scale_x = scale_x;
                                            clip_instance.transform.scale_y = scale_y;

                                            // Position at (0, 0) to center the scaled video
                                            // (scaled dimensions = document dimensions, so top-left at origin centers it)
                                            clip_instance.transform.x = 0.0;
                                            clip_instance.transform.y = 0.0;
                                        } else {
                                            // No dimensions available, use document center
                                            clip_instance.transform.x = center_x;
                                            clip_instance.transform.y = center_y;
                                        }
                                    } else {
                                        // Non-video clips: center at document center
                                        clip_instance.transform.x = center_x;
                                        clip_instance.transform.y = center_y;
                                    }

                                    (center_x, center_y, clip_instance)
                                }; // doc is dropped here

                                // Save instance ID for potential grouping
                                let video_instance_id = clip_instance.id;

                                // Create and queue action for video
                                let action = lightningbeam_core::actions::AddClipInstanceAction::new(
                                    layer_id,
                                    clip_instance,
                                );
                                shared.pending_actions.push(Box::new(action));

                                // If video has linked audio, auto-place it and create group
                                if let Some(linked_audio_clip_id) = dragging.linked_audio_clip_id {
                                    eprintln!("DEBUG: Video has linked audio clip: {}", linked_audio_clip_id);

                                    // Find or create sampled audio track where the audio won't overlap
                                    let audio_layer_id = {
                                        let doc = shared.action_executor.document();
                                        let result = find_sampled_audio_track_for_clip(doc, linked_audio_clip_id, drop_time);
                                        if let Some(id) = result {
                                            eprintln!("DEBUG: Found existing audio track without overlap: {}", id);
                                        } else {
                                            eprintln!("DEBUG: No suitable audio track found, will create new one");
                                        }
                                        result
                                    }.unwrap_or_else(|| {
                                        eprintln!("DEBUG: Creating new audio track");
                                        // Create new sampled audio layer
                                        let audio_layer = lightningbeam_core::layer::AudioLayer::new_sampled("Audio Track");
                                        let layer_id = shared.action_executor.document_mut().root.add_child(
                                            lightningbeam_core::layer::AnyLayer::Audio(audio_layer)
                                        );
                                        eprintln!("DEBUG: Created audio layer with ID: {}", layer_id);
                                        layer_id
                                    });

                                    eprintln!("DEBUG: Using audio layer ID: {}", audio_layer_id);

                                    // Create audio clip instance at same timeline position
                                    let audio_instance = ClipInstance::new(linked_audio_clip_id)
                                        .with_timeline_start(drop_time);
                                    let audio_instance_id = audio_instance.id;

                                    eprintln!("DEBUG: Created audio instance: {} for clip: {}", audio_instance_id, linked_audio_clip_id);

                                    // Queue audio action
                                    let audio_action = lightningbeam_core::actions::AddClipInstanceAction::new(
                                        audio_layer_id,
                                        audio_instance,
                                    );
                                    shared.pending_actions.push(Box::new(audio_action));
                                    eprintln!("DEBUG: Queued audio action, total pending: {}", shared.pending_actions.len());

                                    // Create instance group linking video and audio
                                    let mut group = lightningbeam_core::instance_group::InstanceGroup::new();
                                    group.add_member(layer_id, video_instance_id);
                                    group.add_member(audio_layer_id, audio_instance_id);
                                    shared.action_executor.document_mut().add_instance_group(group);
                                    eprintln!("DEBUG: Created instance group");
                                } else {
                                    eprintln!("DEBUG: Video has NO linked audio clip!");
                                }

                                // Clear drag state
                                *shared.dragging_asset = None;
                            }
                        }
                    } else {
                        // No existing layer at this position - show "create new layer" indicator
                        // and handle drop to create a new layer
                        let layer_y = content_rect.min.y + hovered_layer_index as f32 * LAYER_HEIGHT - self.viewport_scroll_y;
                        let highlight_rect = egui::Rect::from_min_size(
                            egui::pos2(content_rect.min.x, layer_y),
                            egui::vec2(content_rect.width(), LAYER_HEIGHT),
                        );

                        // Blue highlight for "will create new layer"
                        ui.painter().rect_filled(
                            highlight_rect,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(100, 150, 255, 40),
                        );

                        // Show drop time indicator
                        let drop_time = self.x_to_time(pointer_pos.x - content_rect.min.x).max(0.0);
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

                        // Handle drop on mouse release - create new layer
                        if ui.input(|i| i.pointer.any_released()) {
                            let drop_time = self.x_to_time(pointer_pos.x - content_rect.min.x).max(0.0);

                            // Create the appropriate layer type
                            let layer_name = format!("{} Layer", match dragging.clip_type {
                                DragClipType::Vector => "Vector",
                                DragClipType::Video => "Video",
                                DragClipType::AudioSampled => "Audio",
                                DragClipType::AudioMidi => "MIDI",
                                DragClipType::Image => "Image",
                                DragClipType::Effect => "Effect",
                            });
                            let new_layer = super::create_layer_for_clip_type(dragging.clip_type, &layer_name);
                            let new_layer_id = new_layer.id();

                            // Add the layer
                            shared.action_executor.document_mut().root.add_child(new_layer);

                            // Now add the clip to the new layer
                            if dragging.clip_type == DragClipType::Effect {
                                // Handle effect drops
                                let effect_def = lightningbeam_core::effect_registry::EffectRegistry::get_by_id(&dragging.clip_id)
                                    .or_else(|| shared.action_executor.document().get_effect_definition(&dragging.clip_id).cloned());

                                if let Some(def) = effect_def {
                                    if shared.action_executor.document().get_effect_definition(&def.id).is_none() {
                                        shared.action_executor.document_mut().add_effect_definition(def.clone());
                                    }

                                    let clip_instance = ClipInstance::new(def.id)
                                        .with_timeline_start(drop_time)
                                        .with_timeline_duration(5.0);

                                    let action = lightningbeam_core::actions::AddEffectAction::new(
                                        new_layer_id,
                                        clip_instance,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            } else {
                                // Handle other clip types
                                let clip_instance = ClipInstance::new(dragging.clip_id)
                                    .with_timeline_start(drop_time);

                                let action = lightningbeam_core::actions::AddClipInstanceAction::new(
                                    new_layer_id,
                                    clip_instance,
                                );
                                shared.pending_actions.push(Box::new(action));
                            }

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

/// Timeline pane - Modern GarageBand-style timeline
///
/// Phase 1 Implementation: Time Ruler & Playhead
/// - Time-based ruler (seconds, not frames)
/// - Playhead for current time
/// - Zoom/pan controls
/// - Basic layer visualization

use eframe::egui;
use lightningbeam_core::clip::ClipInstance;
use lightningbeam_core::layer::{AnyLayer, AudioLayerType, GroupLayer, LayerTrait};
use super::{DragClipType, NodePath, PaneRenderer, SharedPaneState};

const RULER_HEIGHT: f32 = 30.0;
const LAYER_HEIGHT: f32 = 60.0;
const LAYER_HEADER_WIDTH: f32 = 200.0;
const MIN_PIXELS_PER_SECOND: f32 = 1.0;  // Allow zooming out to see 10+ minutes
const MAX_PIXELS_PER_SECOND: f32 = 500.0;
const EDGE_DETECTION_PIXELS: f32 = 8.0; // Distance from edge to detect trim handles
const LOOP_CORNER_SIZE: f32 = 12.0; // Size of loop corner hotzone at top-right of clip
const MIN_CLIP_WIDTH_PX: f32 = 8.0; // Minimum visible width for very short clips (e.g. groups)

/// Compute stacking row assignments for clip instances on a vector layer.
/// Only clips that overlap in time are stacked; non-overlapping clips share row 0.
/// Returns a Vec of (row, total_rows) for each clip instance.
fn compute_clip_stacking_from_ranges(
    ranges: &[(f64, f64)],
) -> Vec<(usize, usize)> {
    if ranges.len() <= 1 {
        return vec![(0, 1); ranges.len()];
    }

    // Greedy row assignment: assign each clip to the first row where it doesn't overlap
    let mut row_assignments = vec![0usize; ranges.len()];
    let mut row_ends: Vec<f64> = Vec::new(); // track the end time of the last clip in each row

    // Sort indices by start time for greedy packing
    let mut sorted_indices: Vec<usize> = (0..ranges.len()).collect();
    sorted_indices.sort_by(|&a, &b| ranges[a].0.partial_cmp(&ranges[b].0).unwrap_or(std::cmp::Ordering::Equal));

    for &idx in &sorted_indices {
        let (start, end) = ranges[idx];
        // Find first row where this clip fits (no overlap)
        let mut assigned_row = None;
        for (row, row_end) in row_ends.iter_mut().enumerate() {
            if start >= *row_end {
                *row_end = end;
                assigned_row = Some(row);
                break;
            }
        }
        if let Some(row) = assigned_row {
            row_assignments[idx] = row;
        } else {
            row_assignments[idx] = row_ends.len();
            row_ends.push(end);
        }
    }

    let total_rows = row_ends.len().max(1);
    row_assignments.iter().map(|&row| (row, total_rows)).collect()
}

fn compute_clip_stacking(
    document: &lightningbeam_core::document::Document,
    layer: &AnyLayer,
    clip_instances: &[lightningbeam_core::clip::ClipInstance],
) -> Vec<(usize, usize)> {
    if !matches!(layer, AnyLayer::Vector(_)) || clip_instances.len() <= 1 {
        return vec![(0, 1); clip_instances.len()];
    }

    let ranges: Vec<(f64, f64)> = clip_instances.iter().map(|ci| {
        let clip_dur = effective_clip_duration(document, layer, ci).unwrap_or(0.0);
        let start = ci.effective_start();
        let end = start + ci.total_duration(clip_dur);
        (start, end)
    }).collect();

    compute_clip_stacking_from_ranges(&ranges)
}

/// Calculate vertical bounds for a clip instance within a layer row.
/// `row` is the stacking row (0-based), `total_rows` is the total number of rows needed.
/// Returns (y_min, y_max) relative to the layer top.
fn clip_instance_y_bounds(row: usize, total_rows: usize) -> (f32, f32) {
    if total_rows > 1 {
        let usable_height = LAYER_HEIGHT - 20.0; // 10px padding top/bottom
        let row_height = (usable_height / total_rows as f32).min(20.0);
        let top = 10.0 + row as f32 * row_height;
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
                // Movie clips: duration based on all internal content (keyframes + clip instances)
                document.get_clip_duration(&clip_instance.clip_id)
            }
        }
        AnyLayer::Audio(_) => document.get_audio_clip(&clip_instance.clip_id).map(|c| c.duration),
        AnyLayer::Video(_) => document.get_video_clip(&clip_instance.clip_id).map(|c| c.duration),
        AnyLayer::Effect(_) => Some(lightningbeam_core::effect::EFFECT_DURATION),
        AnyLayer::Group(_) => None,
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

/// How time is displayed in the ruler and header
#[derive(Debug, Clone, Copy, PartialEq)]
enum TimeDisplayFormat {
    Seconds,
    Measures,
}

/// Type of recording in progress (for stop logic dispatch)
enum RecordingType {
    Audio,
    Midi,
    Webcam,
}

/// State for an in-progress layer header drag-to-reorder operation.
struct LayerDragState {
    /// IDs of the layers being dragged (in visual order, top to bottom)
    layer_ids: Vec<uuid::Uuid>,
    /// Original parent group IDs for each dragged layer (parallel to layer_ids)
    source_parent_ids: Vec<Option<uuid::Uuid>>,
    /// Current gap position in the filtered (dragged-layers-removed) row list
    gap_row_index: usize,
    /// Current mouse Y in screen coordinates (for floating header rendering)
    current_mouse_y: f32,
    /// Y offset from the top of the topmost dragged row to the mousedown point
    grab_offset_y: f32,
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

    /// Whether to display time as seconds or measures
    time_display_format: TimeDisplayFormat,

    /// Waveform upload progress: pool_index -> frames uploaded so far.
    /// Tracks chunked GPU uploads across frames to avoid hitches.
    waveform_upload_progress: std::collections::HashMap<usize, usize>,

    /// Cached egui textures for video thumbnail strip rendering.
    /// Key: (clip_id, thumbnail_timestamp_millis) → TextureHandle
    video_thumbnail_textures: std::collections::HashMap<(uuid::Uuid, i64), egui::TextureHandle>,

    /// Layer header drag-to-reorder state (None if not dragging a layer)
    layer_drag: Option<LayerDragState>,

    /// Cached mousedown position in header area (for drag threshold detection)
    header_mousedown_pos: Option<egui::Pos2>,
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

/// Represents a single row in the timeline's virtual layer list.
/// Expanded groups show their children directly (no separate header row).
/// Collapsed groups show as a single row with merged clips.
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum TimelineRow<'a> {
    /// A normal standalone layer (not in any group)
    Normal(&'a AnyLayer),
    /// A collapsed group -- single row with expand triangle and merged clips
    CollapsedGroup { group: &'a GroupLayer, depth: u32 },
    /// A child layer inside an expanded group
    GroupChild {
        child: &'a AnyLayer,
        group: &'a GroupLayer,  // the immediate parent group (for collapse action)
        depth: u32,             // nesting depth (1 = direct child of root group)
        show_collapse: bool,    // true for first visible child -- shows collapse triangle
    },
}

impl<'a> TimelineRow<'a> {
    fn layer_id(&self) -> uuid::Uuid {
        match self {
            TimelineRow::Normal(l) => l.id(),
            TimelineRow::CollapsedGroup { group, .. } => group.layer.id,
            TimelineRow::GroupChild { child, .. } => child.id(),
        }
    }

    fn as_any_layer(&self) -> Option<&'a AnyLayer> {
        match self {
            TimelineRow::Normal(l) => Some(l),
            TimelineRow::CollapsedGroup { .. } => None,
            TimelineRow::GroupChild { child, .. } => Some(child),
        }
    }

    /// Returns the parent group ID, or None if this row is at root level.
    fn parent_id(&self) -> Option<uuid::Uuid> {
        match self {
            TimelineRow::GroupChild { group, .. } => Some(group.layer.id),
            _ => None,
        }
    }
}

/// Build a flattened list of timeline rows from the reversed context_layers.
/// Expanded groups emit their children directly (no header row).
/// Collapsed groups emit a single CollapsedGroup row.
fn build_timeline_rows<'a>(context_layers: &[&'a AnyLayer]) -> Vec<TimelineRow<'a>> {
    let mut rows = Vec::new();
    for &layer in context_layers.iter().rev() {
        flatten_layer(layer, 0, None, &mut rows);
    }
    rows
}

fn flatten_layer<'a>(
    layer: &'a AnyLayer,
    depth: u32,
    parent_group: Option<&'a GroupLayer>,
    rows: &mut Vec<TimelineRow<'a>>,
) {
    match layer {
        AnyLayer::Group(g) if !g.expanded => {
            rows.push(TimelineRow::CollapsedGroup { group: g, depth });
        }
        AnyLayer::Group(g) => {
            // Expanded group: no header row, emit children directly.
            // The first emitted row gets the collapse triangle for this group.
            let mut first_emitted = true;
            for child in &g.children {
                let before_len = rows.len();
                flatten_layer(child, depth + 1, Some(g), rows);
                // Mark the first emitted GroupChild row with the collapse triangle
                if first_emitted && rows.len() > before_len {
                    if let Some(TimelineRow::GroupChild { show_collapse, group, .. }) = rows.get_mut(before_len) {
                        *show_collapse = true;
                        *group = g; // point to THIS group for the collapse action
                    }
                    first_emitted = false;
                }
            }
        }
        _ => {
            if depth > 0 {
                if let Some(group) = parent_group {
                    rows.push(TimelineRow::GroupChild {
                        child: layer,
                        group,
                        depth,
                        show_collapse: false,
                    });
                }
            } else {
                rows.push(TimelineRow::Normal(layer));
            }
        }
    }
}

/// Paint a soft drop shadow around a rect using gradient meshes (bottom + right + corner).
/// Three non-overlapping quads so alpha doesn't double up.
fn paint_drop_shadow(painter: &egui::Painter, rect: egui::Rect, shadow_size: f32, alpha: u8) {
    let c = egui::Color32::from_black_alpha(alpha);
    let t = egui::Color32::TRANSPARENT;
    let mut mesh = egui::Mesh::default();

    // Bottom edge: straight down, stops at right edge
    let idx = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_bottom(), c);                                      // 0
    mesh.colored_vertex(rect.right_bottom(), c);                                     // 1
    mesh.colored_vertex(egui::pos2(rect.right(), rect.bottom() + shadow_size), t);   // 2
    mesh.colored_vertex(egui::pos2(rect.left(), rect.bottom() + shadow_size), t);    // 3
    mesh.add_triangle(idx, idx + 1, idx + 2);
    mesh.add_triangle(idx, idx + 2, idx + 3);

    // Right edge: rightward, stops at bottom edge
    let idx = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.right_top(), c);                                        // 0
    mesh.colored_vertex(egui::pos2(rect.right() + shadow_size, rect.top()), t);      // 1
    mesh.colored_vertex(egui::pos2(rect.right() + shadow_size, rect.bottom()), t);   // 2
    mesh.colored_vertex(rect.right_bottom(), c);                                     // 3
    mesh.add_triangle(idx, idx + 1, idx + 2);
    mesh.add_triangle(idx, idx + 2, idx + 3);

    // Bottom-right corner: dark at inner corner, transparent at other three
    let idx = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.right_bottom(), c);                                                  // 0
    mesh.colored_vertex(egui::pos2(rect.right() + shadow_size, rect.bottom()), t);                // 1
    mesh.colored_vertex(egui::pos2(rect.right() + shadow_size, rect.bottom() + shadow_size), t); // 2
    mesh.colored_vertex(egui::pos2(rect.right(), rect.bottom() + shadow_size), t);                // 3
    mesh.add_triangle(idx, idx + 1, idx + 2);
    mesh.add_triangle(idx, idx + 2, idx + 3);

    painter.add(egui::Shape::mesh(mesh));
}

/// Shift+click layer selection: toggle a layer in/out of the focus selection,
/// enforcing the sibling constraint (all selected layers must share the same parent).
fn shift_toggle_layer(
    focus: &mut lightningbeam_core::selection::FocusSelection,
    layer_id: uuid::Uuid,
    clicked_parent: Option<uuid::Uuid>,
    rows: &[TimelineRow],
) {
    use lightningbeam_core::selection::FocusSelection;

    if let FocusSelection::Layers(ids) = focus {
        // Check if existing selection shares the same parent as the clicked layer
        let existing_parent = ids.first().and_then(|first_id| {
            rows.iter()
                .find(|r| r.layer_id() == *first_id)
                .and_then(|r| r.parent_id())
        });
        // For root-level layers, existing_parent is None; for group children, it's Some(group_id)
        // We need to compare them properly: both None means same parent (root)
        let same_parent = if ids.is_empty() {
            true
        } else {
            existing_parent == clicked_parent
        };

        if same_parent {
            // Toggle the clicked layer in/out
            if let Some(pos) = ids.iter().position(|id| *id == layer_id) {
                ids.remove(pos);
                if ids.is_empty() {
                    *focus = FocusSelection::None;
                }
            } else {
                ids.push(layer_id);
            }
            return;
        }
    }

    // Different parent or focus wasn't Layers — start fresh
    *focus = lightningbeam_core::selection::FocusSelection::Layers(vec![layer_id]);
}

/// Collect all (layer_ref, clip_instances) tuples from context_layers,
/// recursively descending into group children.
/// Returns (&AnyLayer, &[ClipInstance]) so callers have access to both layer info and clips.
fn all_layer_clip_instances<'a>(context_layers: &[&'a AnyLayer]) -> Vec<(&'a AnyLayer, &'a [ClipInstance])> {
    let mut result = Vec::new();
    for &layer in context_layers {
        collect_clip_instances(layer, &mut result);
    }
    result
}

fn collect_clip_instances<'a>(layer: &'a AnyLayer, result: &mut Vec<(&'a AnyLayer, &'a [ClipInstance])>) {
    match layer {
        AnyLayer::Vector(l) => result.push((layer, &l.clip_instances)),
        AnyLayer::Audio(l) => result.push((layer, &l.clip_instances)),
        AnyLayer::Video(l) => result.push((layer, &l.clip_instances)),
        AnyLayer::Effect(l) => result.push((layer, &l.clip_instances)),
        AnyLayer::Group(g) => {
            for child in &g.children {
                collect_clip_instances(child, result);
            }
        }
    }
}

/// Find an existing sampled audio track in the document where a clip can be placed without overlap
/// Returns the layer ID if found, None otherwise
fn find_sampled_audio_track_for_clip(
    document: &lightningbeam_core::document::Document,
    clip_id: uuid::Uuid,
    timeline_start: f64,
    editing_clip_id: Option<&uuid::Uuid>,
) -> Option<uuid::Uuid> {
    // Get the clip duration
    let clip_duration = document.get_clip_duration(&clip_id)?;
    let clip_end = timeline_start + clip_duration;

    // Check each sampled audio layer
    let context_layers = document.context_layers(editing_clip_id);
    for &layer in &context_layers {
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
            time_display_format: TimeDisplayFormat::Seconds,
            waveform_upload_progress: std::collections::HashMap::new(),
            video_thumbnail_textures: std::collections::HashMap::new(),
            layer_drag: None,
            header_mousedown_pos: None,
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
    /// In Auto mode, records to the active layer (audio or video with camera)
    fn toggle_recording(&mut self, shared: &mut SharedPaneState) {
        if *shared.is_recording {
            // Stop recording
            self.stop_recording(shared);
        } else {
            // Start recording on active layer
            self.start_recording(shared);
        }
    }

    /// Start recording on the active layer (audio or video with camera)
    fn start_recording(&mut self, shared: &mut SharedPaneState) {
        use lightningbeam_core::clip::{AudioClip, ClipInstance};

        let Some(active_layer_id) = *shared.active_layer_id else {
            println!("⚠️  No active layer selected for recording");
            return;
        };

        // Check if this is a video layer with camera enabled
        let is_video_camera = {
            let document = shared.action_executor.document();
            let context_layers = document.context_layers(shared.editing_clip_id.as_ref());
            context_layers.iter().copied()
                .find(|l| l.id() == active_layer_id)
                .map(|layer| {
                    if let AnyLayer::Video(v) = layer {
                        v.camera_enabled
                    } else {
                        false
                    }
                })
                .unwrap_or(false)
        };

        if is_video_camera {
            // Issue webcam recording start command (processed by main.rs)
            *shared.webcam_record_command = Some(super::WebcamRecordCommand::Start {
                layer_id: active_layer_id,
            });
            *shared.is_recording = true;
            *shared.recording_start_time = *shared.playback_time;
            *shared.recording_layer_id = Some(active_layer_id);

            // Auto-start playback for recording
            if !*shared.is_playing {
                if let Some(controller_arc) = shared.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.play();
                    *shared.is_playing = true;
                    println!("▶ Auto-started playback for webcam recording");
                }
            }
            println!("📹 Started webcam recording on layer {}", active_layer_id);
            return;
        }

        // Get layer type (copy it so we can drop the document borrow before mutating)
        let layer_type = {
            let document = shared.action_executor.document();
            let context_layers = document.context_layers(shared.editing_clip_id.as_ref());
            let Some(layer) = context_layers.iter().copied().find(|l| l.id() == active_layer_id) else {
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

                    if let Some(layer) = shared.action_executor.document_mut().get_layer_mut(&active_layer_id) {
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
        // Determine recording type by checking the layer
        let recording_type = if let Some(layer_id) = *shared.recording_layer_id {
            let context_layers = shared.action_executor.document().context_layers(shared.editing_clip_id.as_ref());
            context_layers.iter().copied()
                .find(|l| l.id() == layer_id)
                .map(|layer| {
                    match layer {
                        lightningbeam_core::layer::AnyLayer::Audio(audio_layer) => {
                            if matches!(audio_layer.audio_layer_type, lightningbeam_core::layer::AudioLayerType::Midi) {
                                RecordingType::Midi
                            } else {
                                RecordingType::Audio
                            }
                        }
                        lightningbeam_core::layer::AnyLayer::Video(v) if v.camera_enabled => {
                            RecordingType::Webcam
                        }
                        _ => RecordingType::Audio,
                    }
                })
                .unwrap_or(RecordingType::Audio)
        } else {
            RecordingType::Audio
        };

        match recording_type {
            RecordingType::Webcam => {
                // Issue webcam stop command (processed by main.rs)
                *shared.webcam_record_command = Some(super::WebcamRecordCommand::Stop);
                println!("📹 Stopped webcam recording");
            }
            _ => {
                if let Some(controller_arc) = shared.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();

                    if matches!(recording_type, RecordingType::Midi) {
                        controller.stop_midi_recording();
                        println!("🎹 Stopped MIDI recording");
                    } else {
                        controller.stop_recording();
                        println!("🎤 Stopped audio recording");
                    }
                }
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
        editing_clip_id: Option<&uuid::Uuid>,
    ) -> Option<(ClipDragType, uuid::Uuid)> {
        let context_layers = document.context_layers(editing_clip_id);
        let rows = build_timeline_rows(&context_layers);
        let layer_count = rows.len();

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

        let row = &rows[hovered_layer_index];
        // Collapsed groups have no directly clickable clips
        let layer: &AnyLayer = match row {
            TimelineRow::Normal(l) => l,
            TimelineRow::GroupChild { child, .. } => child,
            TimelineRow::CollapsedGroup { .. } => return None,
        };
        let _layer_data = layer.layer();

        let clip_instances: &[ClipInstance] = match layer {
            lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
            lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
            lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
            lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
            lightningbeam_core::layer::AnyLayer::Group(_) => &[],
        };

        // Check each clip instance
        let stacking = compute_clip_stacking(document, layer, clip_instances);
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
                let (row, total_rows) = stacking[ci_idx];
                let (cy_min, cy_max) = clip_instance_y_bounds(row, total_rows);
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

    /// Detect if the pointer is over a merged span in a collapsed group row.
    /// Returns all child clip instance IDs that contribute to the hit span.
    fn detect_collapsed_group_at_pointer(
        &self,
        pointer_pos: egui::Pos2,
        document: &lightningbeam_core::document::Document,
        content_rect: egui::Rect,
        header_rect: egui::Rect,
        editing_clip_id: Option<&uuid::Uuid>,
    ) -> Option<Vec<uuid::Uuid>> {
        let context_layers = document.context_layers(editing_clip_id);
        let rows = build_timeline_rows(&context_layers);

        if pointer_pos.y < header_rect.min.y || pointer_pos.x < content_rect.min.x {
            return None;
        }

        let relative_y = pointer_pos.y - header_rect.min.y + self.viewport_scroll_y;
        let hovered_index = (relative_y / LAYER_HEIGHT) as usize;
        if hovered_index >= rows.len() {
            return None;
        }

        let TimelineRow::CollapsedGroup { group, .. } = &rows[hovered_index] else {
            return None;
        };

        // Compute merged spans with the child clip IDs that contribute to each
        let child_clips = group.all_child_clip_instances();
        let mut spans: Vec<(f64, f64, Vec<uuid::Uuid>)> = Vec::new(); // (start, end, clip_ids)

        for (_child_layer_id, ci) in &child_clips {
            let clip_dur = document.get_clip_duration(&ci.clip_id).unwrap_or_else(|| {
                ci.trim_end.unwrap_or(1.0) - ci.trim_start
            });
            let start = ci.effective_start();
            let end = start + ci.total_duration(clip_dur);
            spans.push((start, end, vec![ci.id]));
        }

        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // Merge overlapping spans
        let mut merged: Vec<(f64, f64, Vec<uuid::Uuid>)> = Vec::new();
        for (s, e, ids) in spans {
            if let Some(last) = merged.last_mut() {
                if s <= last.1 {
                    last.1 = last.1.max(e);
                    last.2.extend(ids);
                } else {
                    merged.push((s, e, ids));
                }
            } else {
                merged.push((s, e, ids));
            }
        }

        // Check which merged span the pointer is over
        let mouse_x = pointer_pos.x - content_rect.min.x;
        for (s, e, ids) in merged {
            let sx = self.time_to_x(s);
            let ex = self.time_to_x(e).max(sx + MIN_CLIP_WIDTH_PX);
            if mouse_x >= sx && mouse_x <= ex {
                return Some(ids);
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
    fn render_ruler(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme,
                    bpm: f64, time_sig: &lightningbeam_core::document::TimeSignature) {
        let painter = ui.painter();

        // Background
        let bg_style = theme.style(".timeline-background", ui.ctx());
        let bg_color = bg_style.background_color.unwrap_or(egui::Color32::from_rgb(34, 34, 34));
        painter.rect_filled(rect, 0.0, bg_color);

        let text_style = theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));

        match self.time_display_format {
            TimeDisplayFormat::Seconds => {
                let interval = self.calculate_ruler_interval();
                let start_time = (self.viewport_start_time / interval).floor() * interval;
                let end_time = self.x_to_time(rect.width());

                let mut time = start_time;
                while time <= end_time {
                    let x = self.time_to_x(time);
                    if x >= 0.0 && x <= rect.width() {
                        painter.line_segment(
                            [rect.min + egui::vec2(x, rect.height() - 10.0),
                             rect.min + egui::vec2(x, rect.height())],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                        );
                        painter.text(
                            rect.min + egui::vec2(x + 2.0, 5.0), egui::Align2::LEFT_TOP,
                            format!("{:.1}s", time), egui::FontId::proportional(12.0), text_color,
                        );
                    }
                    let minor_interval = interval / 5.0;
                    for i in 1..5 {
                        let minor_x = self.time_to_x(time + minor_interval * i as f64);
                        if minor_x >= 0.0 && minor_x <= rect.width() {
                            painter.line_segment(
                                [rect.min + egui::vec2(minor_x, rect.height() - 5.0),
                                 rect.min + egui::vec2(minor_x, rect.height())],
                                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                            );
                        }
                    }
                    time += interval;
                }
            }
            TimeDisplayFormat::Measures => {
                let beats_per_second = bpm / 60.0;
                let beat_dur = lightningbeam_core::beat_time::beat_duration(bpm);
                let bpm_count = time_sig.numerator;
                let px_per_beat = beat_dur as f32 * self.pixels_per_second;

                let start_beat = (self.viewport_start_time.max(0.0) * beats_per_second).floor() as i64;
                let end_beat = (self.x_to_time(rect.width()) * beats_per_second).ceil() as i64;

                // Adaptive: how often to label measures
                let measure_px = px_per_beat * bpm_count as f32;
                let label_every = if measure_px > 60.0 { 1u32 } else if measure_px > 20.0 { 4 } else { 16 };

                for beat_idx in start_beat..=end_beat {
                    if beat_idx < 0 { continue; }
                    let x = self.time_to_x(beat_idx as f64 / beats_per_second);
                    if x < 0.0 || x > rect.width() { continue; }

                    let beat_in_measure = (beat_idx as u32) % bpm_count;
                    let measure = (beat_idx as u32) / bpm_count + 1;
                    let is_measure_boundary = beat_in_measure == 0;

                    // Tick height, stroke width, and brightness based on beat importance
                    let (tick_h, stroke_w, gray) = if is_measure_boundary {
                        (12.0, 2.0, 140u8)
                    } else if beat_in_measure % 2 == 0 {
                        (8.0, 1.0, 80)
                    } else {
                        (5.0, 1.0, 50)
                    };

                    painter.line_segment(
                        [rect.min + egui::vec2(x, rect.height() - tick_h),
                         rect.min + egui::vec2(x, rect.height())],
                        egui::Stroke::new(stroke_w, egui::Color32::from_gray(gray)),
                    );

                    // Labels: measure numbers at boundaries, beat numbers when zoomed in
                    if is_measure_boundary && (label_every == 1 || measure % label_every == 1) {
                        painter.text(
                            rect.min + egui::vec2(x + 3.0, 3.0), egui::Align2::LEFT_TOP,
                            format!("{}", measure), egui::FontId::proportional(12.0), text_color,
                        );
                    } else if !is_measure_boundary && px_per_beat > 40.0 {
                        let alpha = if beat_in_measure % 2 == 0 { 0.5 } else if px_per_beat > 80.0 { 0.25 } else { continue };
                        painter.text(
                            rect.min + egui::vec2(x + 2.0, 5.0), egui::Align2::LEFT_TOP,
                            format!("{}.{}", measure, beat_in_measure + 1),
                            egui::FontId::proportional(10.0), text_color.gamma_multiply(alpha),
                        );
                    }
                }
            }
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
        focus: &lightningbeam_core::selection::FocusSelection,
        pending_actions: &mut Vec<Box<dyn lightningbeam_core::action::Action>>,
        _document: &lightningbeam_core::document::Document,
        context_layers: &[&lightningbeam_core::layer::AnyLayer],
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

        // Build virtual row list (accounts for group expansion)
        let all_rows = build_timeline_rows(context_layers);

        // When dragging layers, filter them out and compute gap-adjusted positions
        let drag_layer_ids: Vec<uuid::Uuid> = self.layer_drag.as_ref()
            .map(|d| d.layer_ids.clone()).unwrap_or_default();
        let drag_count = drag_layer_ids.len();
        let gap_row_index = self.layer_drag.as_ref().map(|d| d.gap_row_index);

        // Build filtered row list (excluding dragged layers)
        let rows: Vec<&TimelineRow> = all_rows.iter()
            .filter(|r| !drag_layer_ids.contains(&r.layer_id()))
            .collect();

        // Draw layer headers from virtual row list
        for (filtered_i, row) in rows.iter().enumerate() {
            // Compute Y with gap offset: rows at or after the gap shift down by drag_count * LAYER_HEIGHT
            let visual_i = match gap_row_index {
                Some(gap) if filtered_i >= gap => filtered_i + drag_count,
                _ => filtered_i,
            };
            let y = rect.min.y + visual_i as f32 * LAYER_HEIGHT - self.viewport_scroll_y;

            // Skip if layer is outside visible area
            if y + LAYER_HEIGHT < rect.min.y || y > rect.max.y {
                continue;
            }

            // Indent for group children and collapsed groups based on depth
            let indent = match row {
                TimelineRow::GroupChild { depth, .. } => *depth as f32 * 16.0,
                TimelineRow::CollapsedGroup { depth, .. } => *depth as f32 * 16.0,
                _ => 0.0,
            };

            let header_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, y),
                egui::vec2(LAYER_HEADER_WIDTH, LAYER_HEIGHT),
            );

            // Determine the AnyLayer or GroupLayer for this row
            let (layer_id, layer_name, layer_type, type_color) = match row {
                TimelineRow::Normal(layer) => {
                    let data = layer.layer();
                    let (lt, tc) = match layer {
                        AnyLayer::Vector(_) => ("Vector", egui::Color32::from_rgb(255, 180, 100)),
                        AnyLayer::Audio(al) => match al.audio_layer_type {
                            AudioLayerType::Midi => ("MIDI", egui::Color32::from_rgb(100, 255, 150)),
                            AudioLayerType::Sampled => ("Audio", egui::Color32::from_rgb(100, 180, 255)),
                        },
                        AnyLayer::Video(_) => ("Video", egui::Color32::from_rgb(180, 100, 255)),
                        AnyLayer::Effect(_) => ("Effect", egui::Color32::from_rgb(255, 100, 180)),
                        AnyLayer::Group(_) => ("Group", egui::Color32::from_rgb(0, 180, 180)),
                    };
                    (layer.id(), data.name.clone(), lt, tc)
                }
                TimelineRow::CollapsedGroup { group, .. } => {
                    (group.layer.id, group.layer.name.clone(), "Group", egui::Color32::from_rgb(0, 180, 180))
                }
                TimelineRow::GroupChild { child, .. } => {
                    let data = child.layer();
                    let (lt, tc) = match child {
                        AnyLayer::Vector(_) => ("Vector", egui::Color32::from_rgb(255, 180, 100)),
                        AnyLayer::Audio(al) => match al.audio_layer_type {
                            AudioLayerType::Midi => ("MIDI", egui::Color32::from_rgb(100, 255, 150)),
                            AudioLayerType::Sampled => ("Audio", egui::Color32::from_rgb(100, 180, 255)),
                        },
                        AnyLayer::Video(_) => ("Video", egui::Color32::from_rgb(180, 100, 255)),
                        AnyLayer::Effect(_) => ("Effect", egui::Color32::from_rgb(255, 100, 180)),
                        AnyLayer::Group(_) => ("Group", egui::Color32::from_rgb(0, 180, 180)),
                    };
                    (child.id(), data.name.clone(), lt, tc)
                }
            };

            // Active vs inactive background colors
            let is_active = active_layer_id.map_or(false, |id| id == layer_id);
            let is_selected = match focus {
                lightningbeam_core::selection::FocusSelection::Layers(ids) => ids.contains(&layer_id),
                _ => false,
            };
            let bg_color = if is_active || is_selected {
                active_color
            } else {
                inactive_color
            };

            ui.painter().rect_filled(header_rect, 0.0, bg_color);

            // Gutter area (left of indicator) — solid group color, with collapse chevron
            if indent > 0.0 {
                let gutter_rect = egui::Rect::from_min_size(
                    header_rect.min,
                    egui::vec2(indent, LAYER_HEIGHT),
                );
                // Solid dark group color for the gutter strip
                let group_color = match row {
                    TimelineRow::GroupChild { .. } | TimelineRow::CollapsedGroup { .. } => {
                        // Solid dark teal for the group gutter
                        egui::Color32::from_rgb(0, 50, 50)
                    }
                    _ => header_bg,
                };
                ui.painter().rect_filled(gutter_rect, 0.0, group_color);

                // Thin colored accent line at right edge of gutter (group color)
                let accent_rect = egui::Rect::from_min_size(
                    egui::pos2(header_rect.min.x + indent - 2.0, y),
                    egui::vec2(2.0, LAYER_HEIGHT),
                );
                ui.painter().rect_filled(accent_rect, 0.0, egui::Color32::from_rgb(0, 180, 180));

                // Draw collapse triangle on first child row (painted, not text)
                if let TimelineRow::GroupChild { show_collapse: true, .. } = row {
                    let cx = header_rect.min.x + indent * 0.5;
                    let cy = y + LAYER_HEIGHT * 0.5;
                    let s = 4.0; // half-size of triangle
                    // Down-pointing triangle (▼) for collapse
                    let tri = vec![
                        egui::pos2(cx - s, cy - s * 0.6),
                        egui::pos2(cx + s, cy - s * 0.6),
                        egui::pos2(cx, cy + s * 0.6),
                    ];
                    ui.painter().add(egui::Shape::convex_polygon(tri, egui::Color32::from_gray(180), egui::Stroke::NONE));
                }

                // Make the ENTIRE gutter clickable for collapse on any GroupChild row
                if let TimelineRow::GroupChild { group, .. } = row {
                    let gutter_response = ui.scope_builder(egui::UiBuilder::new().max_rect(gutter_rect), |ui| {
                        ui.allocate_rect(gutter_rect, egui::Sense::click())
                    }).inner;
                    if gutter_response.clicked() {
                        self.layer_control_clicked = true;
                        pending_actions.push(Box::new(
                            lightningbeam_core::actions::ToggleGroupExpansionAction::new(group.layer.id, false),
                        ));
                    }
                }
            }

            // Color indicator bar on the left edge (after gutter)
            let indicator_rect = egui::Rect::from_min_size(
                header_rect.min + egui::vec2(indent, 0.0),
                egui::vec2(4.0, LAYER_HEIGHT),
            );
            ui.painter().rect_filled(indicator_rect, 0.0, type_color);

            // Expand triangle in the header for collapsed groups
            let mut name_x_offset = 10.0 + indent;
            if let TimelineRow::CollapsedGroup { group, .. } = row {
                // Right-pointing triangle (▶) for expand, painted manually
                let cx = header_rect.min.x + indent + 14.0;
                let cy = y + 17.0;
                let s = 4.0;
                let tri = vec![
                    egui::pos2(cx - s * 0.6, cy - s),
                    egui::pos2(cx - s * 0.6, cy + s),
                    egui::pos2(cx + s * 0.6, cy),
                ];
                ui.painter().add(egui::Shape::convex_polygon(tri, egui::Color32::from_gray(180), egui::Stroke::NONE));

                // Clickable area for expand
                let chevron_rect = egui::Rect::from_min_size(
                    egui::pos2(header_rect.min.x + indent + 4.0, y + 4.0),
                    egui::vec2(20.0, 24.0),
                );
                let chevron_response = ui.scope_builder(egui::UiBuilder::new().max_rect(chevron_rect), |ui| {
                    ui.allocate_rect(chevron_rect, egui::Sense::click())
                }).inner;
                if chevron_response.clicked() {
                    self.layer_control_clicked = true;
                    pending_actions.push(Box::new(
                        lightningbeam_core::actions::ToggleGroupExpansionAction::new(group.layer.id, true),
                    ));
                }
                name_x_offset = 10.0 + indent + 18.0;
            }

            // Layer name
            ui.painter().text(
                header_rect.min + egui::vec2(name_x_offset, 10.0),
                egui::Align2::LEFT_TOP,
                &layer_name,
                egui::FontId::proportional(14.0),
                text_color,
            );

            // Layer type (smaller text below name with colored background)
            let type_text_pos = header_rect.min + egui::vec2(name_x_offset, 28.0);
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

            // Get the AnyLayer reference for controls
            let any_layer_for_controls: Option<&AnyLayer> = match row {
                TimelineRow::Normal(l) => Some(l),
                TimelineRow::CollapsedGroup { group, .. } => {
                    // We need an AnyLayer ref - find it from context_layers
                    context_layers.iter().rev().copied().find(|l| l.id() == group.layer.id)
                }
                TimelineRow::GroupChild { child, .. } => Some(child),
            };

            let Some(layer_for_controls) = any_layer_for_controls else { continue; };

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
            let current_volume = layer_for_controls.volume();
            let is_muted = layer_for_controls.muted();
            let is_soloed = layer_for_controls.soloed();
            let is_locked = layer_for_controls.locked();

            // Mute button — or camera toggle for video layers
            let is_video_layer = matches!(layer_for_controls, lightningbeam_core::layer::AnyLayer::Video(_));
            let camera_enabled = if let lightningbeam_core::layer::AnyLayer::Video(v) = layer_for_controls {
                v.camera_enabled
            } else {
                false
            };

            let first_btn_response = ui.scope_builder(egui::UiBuilder::new().max_rect(mute_button_rect), |ui| {
                if is_video_layer {
                    // Camera toggle for video layers
                    let cam_text = if camera_enabled { "📹" } else { "📷" };
                    let button = egui::Button::new(cam_text)
                        .fill(if camera_enabled {
                            egui::Color32::from_rgba_unmultiplied(100, 200, 100, 100)
                        } else {
                            egui::Color32::from_gray(40)
                        })
                        .stroke(egui::Stroke::NONE);
                    ui.add(button)
                } else {
                    // Mute button for non-video layers
                    let mute_text = if is_muted { "🔇" } else { "🔊" };
                    let button = egui::Button::new(mute_text)
                        .fill(if is_muted {
                            egui::Color32::from_rgba_unmultiplied(255, 100, 100, 100)
                        } else {
                            egui::Color32::from_gray(40)
                        })
                        .stroke(egui::Stroke::NONE);
                    ui.add(button)
                }
            }).inner;

            if first_btn_response.clicked() {
                self.layer_control_clicked = true;
                if is_video_layer {
                    pending_actions.push(Box::new(
                        lightningbeam_core::actions::SetLayerPropertiesAction::new(
                            layer_id,
                            lightningbeam_core::actions::LayerProperty::CameraEnabled(!camera_enabled),
                        )
                    ));
                } else {
                    pending_actions.push(Box::new(
                        lightningbeam_core::actions::SetLayerPropertiesAction::new(
                            layer_id,
                            lightningbeam_core::actions::LayerProperty::Muted(!is_muted),
                        )
                    ));
                }
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

        // Draw floating dragged layer headers at mouse position with drop shadow
        if let Some(ref drag_state) = self.layer_drag {
            // Collect the dragged rows in order
            let dragged_rows: Vec<&TimelineRow> = drag_state.layer_ids.iter()
                .filter_map(|did| all_rows.iter().find(|r| r.layer_id() == *did))
                .collect();

            let float_top_y = drag_state.current_mouse_y - drag_state.grab_offset_y;

            for (di, dragged_row) in dragged_rows.iter().enumerate() {
                let float_y = float_top_y + di as f32 * LAYER_HEIGHT;
                let float_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, float_y),
                    egui::vec2(LAYER_HEADER_WIDTH, LAYER_HEIGHT),
                );

                // Gradient drop shadow
                paint_drop_shadow(ui.painter(), float_rect, 8.0, 60);

                // Background (active/selected color)
                ui.painter().rect_filled(float_rect, 0.0, active_color);

                // Layer info
                let drag_indent = match dragged_row {
                    TimelineRow::GroupChild { depth, .. } => *depth as f32 * 16.0,
                    TimelineRow::CollapsedGroup { depth, .. } => *depth as f32 * 16.0,
                    _ => 0.0,
                };
                let (drag_name, drag_type_str, drag_type_color) = match dragged_row {
                    TimelineRow::Normal(layer) => {
                        let (lt, tc) = match layer {
                            AnyLayer::Vector(_) => ("Vector", egui::Color32::from_rgb(255, 180, 100)),
                            AnyLayer::Audio(al) => match al.audio_layer_type {
                                AudioLayerType::Midi => ("MIDI", egui::Color32::from_rgb(100, 255, 150)),
                                AudioLayerType::Sampled => ("Audio", egui::Color32::from_rgb(100, 180, 255)),
                            },
                            AnyLayer::Video(_) => ("Video", egui::Color32::from_rgb(180, 100, 255)),
                            AnyLayer::Effect(_) => ("Effect", egui::Color32::from_rgb(255, 100, 180)),
                            AnyLayer::Group(_) => ("Group", egui::Color32::from_rgb(0, 180, 180)),
                        };
                        (layer.layer().name.clone(), lt, tc)
                    }
                    TimelineRow::CollapsedGroup { group, .. } => {
                        (group.layer.name.clone(), "Group", egui::Color32::from_rgb(0, 180, 180))
                    }
                    TimelineRow::GroupChild { child, .. } => {
                        let (lt, tc) = match child {
                            AnyLayer::Vector(_) => ("Vector", egui::Color32::from_rgb(255, 180, 100)),
                            AnyLayer::Audio(al) => match al.audio_layer_type {
                                AudioLayerType::Midi => ("MIDI", egui::Color32::from_rgb(100, 255, 150)),
                                AudioLayerType::Sampled => ("Audio", egui::Color32::from_rgb(100, 180, 255)),
                            },
                            AnyLayer::Video(_) => ("Video", egui::Color32::from_rgb(180, 100, 255)),
                            AnyLayer::Effect(_) => ("Effect", egui::Color32::from_rgb(255, 100, 180)),
                            AnyLayer::Group(_) => ("Group", egui::Color32::from_rgb(0, 180, 180)),
                        };
                        (child.layer().name.clone(), lt, tc)
                    }
                };

                // Color indicator bar
                let indicator_rect = egui::Rect::from_min_size(
                    float_rect.min + egui::vec2(drag_indent, 0.0),
                    egui::vec2(4.0, LAYER_HEIGHT),
                );
                ui.painter().rect_filled(indicator_rect, 0.0, drag_type_color);

                // Layer name
                let name_x = 10.0 + drag_indent;
                ui.painter().text(
                    float_rect.min + egui::vec2(name_x, 10.0),
                    egui::Align2::LEFT_TOP,
                    &drag_name,
                    egui::FontId::proportional(14.0),
                    text_color,
                );

                // Type label
                ui.painter().text(
                    float_rect.min + egui::vec2(name_x, 28.0),
                    egui::Align2::LEFT_TOP,
                    drag_type_str,
                    egui::FontId::proportional(11.0),
                    secondary_text_color,
                );

                // Separator line at bottom
                ui.painter().line_segment(
                    [egui::pos2(float_rect.min.x, float_rect.max.y), egui::pos2(float_rect.max.x, float_rect.max.y)],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
                );
            }
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
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        theme: &crate::theme::Theme,
        document: &lightningbeam_core::document::Document,
        active_layer_id: &Option<uuid::Uuid>,
        focus: &lightningbeam_core::selection::FocusSelection,
        selection: &lightningbeam_core::selection::Selection,
        midi_event_cache: &std::collections::HashMap<u32, Vec<(f64, u8, u8, bool)>>,
        raw_audio_cache: &std::collections::HashMap<usize, (std::sync::Arc<Vec<f32>>, u32, u32)>,
        waveform_gpu_dirty: &mut std::collections::HashSet<usize>,
        target_format: wgpu::TextureFormat,
        waveform_stereo: bool,
        context_layers: &[&lightningbeam_core::layer::AnyLayer],
        video_manager: &std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>,
    ) -> Vec<(egui::Rect, uuid::Uuid, f64, f64)> {
        let painter = ui.painter();

        // Collect video clip rects for hover detection (to avoid borrow conflicts)
        let mut video_clip_hovers: Vec<(egui::Rect, uuid::Uuid, f64, f64)> = Vec::new();

        // Track visible video clip IDs for texture cache cleanup
        let mut visible_video_clip_ids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

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

        // Build virtual row list (accounts for group expansion)
        let all_rows = build_timeline_rows(context_layers);

        // When dragging layers, compute remapped Y positions:
        // - Dragged rows render at the gap position
        // - Non-dragged rows shift around the gap
        let drag_layer_ids_content: Vec<uuid::Uuid> = self.layer_drag.as_ref()
            .map(|d| d.layer_ids.clone()).unwrap_or_default();
        let drag_count_content = drag_layer_ids_content.len();
        let gap_row_index_content = self.layer_drag.as_ref().map(|d| d.gap_row_index);

        // Pre-compute Y position for each row.
        // Dragged rows follow the mouse continuously (matching the floating header);
        // non-dragged rows snap to discrete positions shifted around the gap.
        let drag_float_top_y: Option<f32> = self.layer_drag.as_ref()
            .map(|d| d.current_mouse_y - d.grab_offset_y);

        let row_y_positions: Vec<f32> = {
            let mut positions = Vec::with_capacity(all_rows.len());
            let mut filtered_i = 0usize;
            let mut drag_offset = 0usize;
            for row in all_rows.iter() {
                if drag_layer_ids_content.contains(&row.layer_id()) {
                    // Dragged row: continuous Y from mouse position
                    let base_y = drag_float_top_y.unwrap_or(0.0);
                    positions.push(base_y + drag_offset as f32 * LAYER_HEIGHT);
                    drag_offset += 1;
                } else {
                    // Non-dragged row: discrete position, shifted around gap
                    let visual = match gap_row_index_content {
                        Some(gap) if filtered_i >= gap => filtered_i + drag_count_content,
                        _ => filtered_i,
                    };
                    positions.push(rect.min.y + visual as f32 * LAYER_HEIGHT - self.viewport_scroll_y);
                    filtered_i += 1;
                }
            }
            positions
        };

        // Draw non-dragged rows first, then dragged rows on top (so shadow/content overlaps correctly)
        let draw_order: Vec<usize> = {
            let mut non_dragged: Vec<usize> = Vec::new();
            let mut dragged: Vec<usize> = Vec::new();
            for (i, row) in all_rows.iter().enumerate() {
                if drag_layer_ids_content.contains(&row.layer_id()) {
                    dragged.push(i);
                } else {
                    non_dragged.push(i);
                }
            }
            non_dragged.extend(dragged);
            non_dragged
        };

        for &i in &draw_order {
            let row = &all_rows[i];
            let y = row_y_positions[i];
            let is_being_dragged = drag_layer_ids_content.contains(&row.layer_id());

            // Skip if layer is outside visible area
            if y + LAYER_HEIGHT < rect.min.y || y > rect.max.y {
                continue;
            }

            let layer_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, y),
                egui::vec2(rect.width(), LAYER_HEIGHT),
            );

            // Drop shadow for dragged rows
            if is_being_dragged {
                paint_drop_shadow(painter, layer_rect, 8.0, 60);
            }

            let row_layer_id = row.layer_id();

            // Active vs inactive background colors
            let is_active = active_layer_id.map_or(false, |id| id == row_layer_id);
            let is_selected = match focus {
                lightningbeam_core::selection::FocusSelection::Layers(ids) => ids.contains(&row_layer_id),
                _ => false,
            };
            let bg_color = if is_active || is_selected {
                active_color
            } else {
                inactive_color
            };

            painter.rect_filled(layer_rect, 0.0, bg_color);

            // Grid lines matching ruler
            match self.time_display_format {
                TimeDisplayFormat::Seconds => {
                    let interval = self.calculate_ruler_interval();
                    let start_time = (self.viewport_start_time / interval).floor() * interval;
                    let end_time = self.x_to_time(rect.width());
                    let mut time = start_time;
                    while time <= end_time {
                        let x = self.time_to_x(time);
                        if x >= 0.0 && x <= rect.width() {
                            painter.line_segment(
                                [egui::pos2(rect.min.x + x, y),
                                 egui::pos2(rect.min.x + x, y + LAYER_HEIGHT)],
                                egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                            );
                        }
                        time += interval;
                    }
                }
                TimeDisplayFormat::Measures => {
                    let beats_per_second = document.bpm / 60.0;
                    let bpm_count = document.time_signature.numerator;
                    let start_beat = (self.viewport_start_time.max(0.0) * beats_per_second).floor() as i64;
                    let end_beat = (self.x_to_time(rect.width()) * beats_per_second).ceil() as i64;
                    for beat_idx in start_beat..=end_beat {
                        if beat_idx < 0 { continue; }
                        let x = self.time_to_x(beat_idx as f64 / beats_per_second);
                        if x < 0.0 || x > rect.width() { continue; }
                        let is_measure_boundary = (beat_idx as u32) % bpm_count == 0;
                        let gray = if is_measure_boundary { 45 } else { 25 };
                        painter.line_segment(
                            [egui::pos2(rect.min.x + x, y),
                             egui::pos2(rect.min.x + x, y + LAYER_HEIGHT)],
                            egui::Stroke::new(if is_measure_boundary { 1.5 } else { 1.0 }, egui::Color32::from_gray(gray)),
                        );
                    }
                }
            }

            // For collapsed groups, render merged clip spans and skip normal clip rendering
            if let TimelineRow::CollapsedGroup { group: g, .. } = row {
                // Collect all child clip time ranges (with drag preview offset)
                let child_clips = g.all_child_clip_instances();
                let is_move_drag = self.clip_drag_state == Some(ClipDragType::Move);
                let mut ranges: Vec<(f64, f64)> = Vec::new();
                for (_child_layer_id, ci) in &child_clips {
                    let clip_dur = document.get_clip_duration(&ci.clip_id).unwrap_or_else(|| {
                        ci.trim_end.unwrap_or(1.0) - ci.trim_start
                    });
                    let mut start = ci.effective_start();
                    let dur = ci.total_duration(clip_dur);
                    // Apply drag offset for selected clips during move
                    if is_move_drag && selection.contains_clip_instance(&ci.id) {
                        start = (start + self.drag_offset).max(0.0);
                    }
                    ranges.push((start, start + dur));
                }

                // Sort and merge overlapping ranges
                ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let mut merged: Vec<(f64, f64)> = Vec::new();
                for (s, e) in ranges {
                    if let Some(last) = merged.last_mut() {
                        if s <= last.1 {
                            last.1 = last.1.max(e);
                        } else {
                            merged.push((s, e));
                        }
                    } else {
                        merged.push((s, e));
                    }
                }

                // Check if any child clips are selected (for highlight)
                let any_selected = child_clips.iter().any(|(_, ci)| selection.contains_clip_instance(&ci.id));
                // Draw each merged span as a teal bar (brighter when selected)
                let teal = if any_selected {
                    egui::Color32::from_rgb(30, 190, 190)
                } else {
                    egui::Color32::from_rgb(0, 150, 150)
                };
                let bright_teal = if any_selected {
                    egui::Color32::from_rgb(150, 255, 255)
                } else {
                    egui::Color32::from_rgb(100, 220, 220)
                };
                for (s, e) in &merged {
                    let sx = self.time_to_x(*s);
                    let ex = self.time_to_x(*e).max(sx + MIN_CLIP_WIDTH_PX);
                    if ex >= 0.0 && sx <= rect.width() {
                        let vsx = sx.max(0.0);
                        let vex = ex.min(rect.width());
                        let span_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x + vsx, y + 10.0),
                            egui::pos2(rect.min.x + vex, y + LAYER_HEIGHT - 10.0),
                        );
                        painter.rect_filled(span_rect, 3.0, teal);
                        painter.rect_stroke(
                            span_rect,
                            3.0,
                            egui::Stroke::new(1.0, bright_teal),
                            egui::StrokeKind::Middle,
                        );
                    }
                }

                // Render video thumbnails from the top video child layer
                // and waveforms from audio child layers inside the collapsed group row
                {
                    let span_y_min = y + 10.0;
                    let span_y_max = y + LAYER_HEIGHT - 10.0;
                    let span_height = span_y_max - span_y_min;
                    let thumb_y_max = span_y_min + span_height * (2.0 / 3.0);
                    let wave_y_min = thumb_y_max;

                    // Find the first (top) video child and draw thumbnails for its clips
                    if let Some(video_child) = g.children.iter().find(|c| matches!(c, AnyLayer::Video(_))) {
                        if let AnyLayer::Video(vl) = video_child {
                            for ci in &vl.clip_instances {
                                let clip_dur = document.get_clip_duration(&ci.clip_id)
                                    .unwrap_or_else(|| ci.trim_end.unwrap_or(1.0) - ci.trim_start);
                                let mut ci_start = ci.effective_start();
                                if is_move_drag && selection.contains_clip_instance(&ci.id) {
                                    ci_start = (ci_start + self.drag_offset).max(0.0);
                                }
                                let ci_duration = ci.total_duration(clip_dur);
                                let ci_end = ci_start + ci_duration;

                                let sx = self.time_to_x(ci_start);
                                let ex = self.time_to_x(ci_end);
                                if ex < 0.0 || sx > rect.width() { continue; }

                                let ci_rect = egui::Rect::from_min_max(
                                    egui::pos2((rect.min.x + sx).max(rect.min.x), span_y_min),
                                    egui::pos2((rect.min.x + ex).min(rect.max.x), thumb_y_max),
                                );

                                visible_video_clip_ids.insert(ci.clip_id);

                                // Collect for hover tooltip (use full span height as hover target)
                                let hover_rect = egui::Rect::from_min_max(
                                    egui::pos2(ci_rect.min.x, span_y_min),
                                    egui::pos2(ci_rect.max.x, span_y_max),
                                );
                                video_clip_hovers.push((hover_rect, ci.clip_id, ci.trim_start, ci_start));

                                let thumb_display_height = (thumb_y_max - span_y_min) - 4.0;
                                if thumb_display_height > 8.0 {
                                    let video_mgr = video_manager.lock().unwrap();
                                    if let Some((tw, th, _)) = video_mgr.get_thumbnail_at(&ci.clip_id, 0.0) {
                                        let aspect = tw as f32 / th as f32;
                                        let thumb_display_width = thumb_display_height * aspect;
                                        let ci_width = ci_rect.width();
                                        let num_thumbs = ((ci_width / thumb_display_width).ceil() as usize).max(1);

                                        for ti in 0..num_thumbs {
                                            let x_offset = ti as f32 * thumb_display_width;
                                            if x_offset >= ci_width { break; }

                                            let time_offset = (x_offset as f64 + thumb_display_width as f64 * 0.5)
                                                / self.pixels_per_second as f64;
                                            let content_time = ci.trim_start + time_offset;

                                            if let Some((tw, th, rgba_data)) = video_mgr.get_thumbnail_at(&ci.clip_id, content_time) {
                                                let ts_key = (content_time * 1000.0) as i64;
                                                let cache_key = (ci.clip_id, ts_key);

                                                let texture = self.video_thumbnail_textures
                                                    .entry(cache_key)
                                                    .or_insert_with(|| {
                                                        let image = egui::ColorImage::from_rgba_unmultiplied(
                                                            [tw as usize, th as usize],
                                                            &rgba_data,
                                                        );
                                                        ui.ctx().load_texture(
                                                            format!("vthumb_{}_{}", ci.clip_id, ts_key),
                                                            image,
                                                            egui::TextureOptions::LINEAR,
                                                        )
                                                    });

                                                let full_rect = egui::Rect::from_min_size(
                                                    egui::pos2(ci_rect.min.x + x_offset, ci_rect.min.y + 2.0),
                                                    egui::vec2(thumb_display_width, thumb_display_height),
                                                );
                                                let thumb_rect = full_rect.intersect(ci_rect);

                                                if thumb_rect.width() > 2.0 && thumb_rect.height() > 2.0 {
                                                    let uv_min = egui::pos2(
                                                        (thumb_rect.min.x - full_rect.min.x) / full_rect.width(),
                                                        (thumb_rect.min.y - full_rect.min.y) / full_rect.height(),
                                                    );
                                                    let uv_max = egui::pos2(
                                                        (thumb_rect.max.x - full_rect.min.x) / full_rect.width(),
                                                        (thumb_rect.max.y - full_rect.min.y) / full_rect.height(),
                                                    );

                                                    painter.image(
                                                        texture.id(),
                                                        thumb_rect,
                                                        egui::Rect::from_min_max(uv_min, uv_max),
                                                        egui::Color32::WHITE,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Draw waveforms from audio child layers
                    let screen_size = ui.ctx().content_rect().size();
                    let waveform_tint = [
                        bright_teal.r() as f32 / 255.0,
                        bright_teal.g() as f32 / 255.0,
                        bright_teal.b() as f32 / 255.0,
                        bright_teal.a() as f32 / 255.0,
                    ];
                    for child in &g.children {
                        if let AnyLayer::Audio(al) = child {
                            for ci in &al.clip_instances {
                                let audio_clip = match document.get_audio_clip(&ci.clip_id) {
                                    Some(c) => c,
                                    None => continue,
                                };
                                let audio_pool_index = match audio_clip.audio_pool_index() {
                                    Some(idx) => idx,
                                    None => continue,
                                };
                                let (samples, sr, ch) = match raw_audio_cache.get(&audio_pool_index) {
                                    Some(v) => v,
                                    None => continue,
                                };

                                let total_frames = samples.len() / (*ch).max(1) as usize;
                                let audio_file_duration = total_frames as f64 / *sr as f64;

                                let clip_dur = audio_clip.duration;
                                let mut ci_start = ci.effective_start();
                                if is_move_drag && selection.contains_clip_instance(&ci.id) {
                                    ci_start = (ci_start + self.drag_offset).max(0.0);
                                }
                                let ci_duration = ci.total_duration(clip_dur);

                                let ci_screen_start = rect.min.x + self.time_to_x(ci_start);
                                let ci_screen_end = ci_screen_start + (ci_duration * self.pixels_per_second as f64) as f32;

                                let waveform_rect = egui::Rect::from_min_max(
                                    egui::pos2(ci_screen_start.max(rect.min.x), wave_y_min),
                                    egui::pos2(ci_screen_end.min(rect.max.x), span_y_max),
                                );

                                if waveform_rect.width() > 0.0 && waveform_rect.height() > 0.0 {
                                    let pending_upload = if waveform_gpu_dirty.contains(&audio_pool_index) {
                                        let chunk = crate::waveform_gpu::UPLOAD_CHUNK_FRAMES;
                                        let progress = self.waveform_upload_progress.get(&audio_pool_index).copied().unwrap_or(0);
                                        let next_end = (progress + chunk).min(total_frames);
                                        let frame_limit = Some(next_end);
                                        if next_end >= total_frames {
                                            waveform_gpu_dirty.remove(&audio_pool_index);
                                            self.waveform_upload_progress.remove(&audio_pool_index);
                                        } else {
                                            self.waveform_upload_progress.insert(audio_pool_index, next_end);
                                            ui.ctx().request_repaint();
                                        }
                                        Some(crate::waveform_gpu::PendingUpload {
                                            samples: samples.clone(),
                                            sample_rate: *sr,
                                            channels: *ch,
                                            frame_limit,
                                        })
                                    } else {
                                        None
                                    };

                                    let instance_id = ci.id.as_u128() as u64;
                                    let callback = crate::waveform_gpu::WaveformCallback {
                                        pool_index: audio_pool_index,
                                        segment_index: 0,
                                        params: crate::waveform_gpu::WaveformParams {
                                            clip_rect: [waveform_rect.min.x, waveform_rect.min.y, waveform_rect.max.x, waveform_rect.max.y],
                                            viewport_start_time: self.viewport_start_time as f32,
                                            pixels_per_second: self.pixels_per_second as f32,
                                            audio_duration: audio_file_duration as f32,
                                            sample_rate: *sr as f32,
                                            clip_start_time: ci_screen_start,
                                            trim_start: ci.trim_start as f32,
                                            tex_width: crate::waveform_gpu::tex_width() as f32,
                                            total_frames: total_frames as f32,
                                            segment_start_frame: 0.0,
                                            display_mode: if waveform_stereo { 1.0 } else { 0.0 },
                                            _pad1: [0.0, 0.0],
                                            tint_color: waveform_tint,
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

                // Separator line at bottom
                painter.line_segment(
                    [
                        egui::pos2(layer_rect.min.x, layer_rect.max.y),
                        egui::pos2(layer_rect.max.x, layer_rect.max.y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
                );
                continue; // Skip normal clip rendering for collapsed groups
            }

            // Get the AnyLayer for normal rendering (Normal or GroupChild rows)
            let layer: &AnyLayer = match row {
                TimelineRow::Normal(l) => l,
                TimelineRow::GroupChild { child, .. } => child,
                TimelineRow::CollapsedGroup { .. } => unreachable!(), // handled above
            };

            // Draw clip instances for this layer
            let clip_instances: &[ClipInstance] = match layer {
                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
                lightningbeam_core::layer::AnyLayer::Group(_) => &[],
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

            // Compute stacking using preview positions (with drag offsets) for vector layers
            let clip_stacking = if matches!(layer, AnyLayer::Vector(_)) && clip_instances.len() > 1 {
                let preview_ranges: Vec<(f64, f64)> = clip_instances.iter().map(|ci| {
                    let clip_dur = effective_clip_duration(document, layer, ci).unwrap_or(0.0);
                    let mut start = ci.effective_start();
                    let mut duration = ci.total_duration(clip_dur);

                    let is_selected = selection.contains_clip_instance(&ci.id);
                    let is_linked = if self.clip_drag_state.is_some() {
                        instance_to_group.get(&ci.id).map_or(false, |group| {
                            group.members.iter().any(|(_, mid)| *mid != ci.id && selection.contains_clip_instance(mid))
                        })
                    } else {
                        false
                    };

                    if let Some(drag_type) = self.clip_drag_state {
                        if is_selected || is_linked {
                            match drag_type {
                                ClipDragType::Move => {
                                    if let Some(offset) = group_move_offset {
                                        start = (ci.effective_start() + offset).max(0.0);
                                    }
                                }
                                ClipDragType::TrimLeft => {
                                    let new_trim = (ci.trim_start + self.drag_offset).max(0.0).min(clip_dur);
                                    let offset = new_trim - ci.trim_start;
                                    start = (ci.timeline_start + offset).max(0.0);
                                    duration = (clip_dur - new_trim).max(0.0);
                                    if let Some(trim_end) = ci.trim_end {
                                        duration = (trim_end - new_trim).max(0.0);
                                    }
                                }
                                ClipDragType::TrimRight => {
                                    let old_trim_end = ci.trim_end.unwrap_or(clip_dur);
                                    let new_trim_end = (old_trim_end + self.drag_offset).max(ci.trim_start).min(clip_dur);
                                    duration = (new_trim_end - ci.trim_start).max(0.0);
                                }
                                ClipDragType::LoopExtendRight => {
                                    let trim_end = ci.trim_end.unwrap_or(clip_dur);
                                    let content_window = (trim_end - ci.trim_start).max(0.0);
                                    let current_right = ci.timeline_duration.unwrap_or(content_window);
                                    let new_right = (current_right + self.drag_offset).max(content_window);
                                    let loop_before = ci.loop_before.unwrap_or(0.0);
                                    duration = loop_before + new_right;
                                }
                                ClipDragType::LoopExtendLeft => {
                                    let trim_end = ci.trim_end.unwrap_or(clip_dur);
                                    let content_window = (trim_end - ci.trim_start).max(0.001);
                                    let current_loop_before = ci.loop_before.unwrap_or(0.0);
                                    let desired = (current_loop_before - self.drag_offset).max(0.0);
                                    let snapped = (desired / content_window).round() * content_window;
                                    start = ci.timeline_start - snapped;
                                    duration = snapped + ci.effective_duration(clip_dur);
                                }
                            }
                        }
                    }

                    (start, start + duration)
                }).collect();
                compute_clip_stacking_from_ranges(&preview_ranges)
            } else {
                compute_clip_stacking(document, layer, clip_instances)
            };
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
                    let preview_trim_end_default = clip_instance.trim_end.unwrap_or(clip_duration);
                    let mut preview_clip_duration = (preview_trim_end_default - preview_trim_start).max(0.0);

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
                                    preview_clip_duration = instance_duration;
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
                            lightningbeam_core::layer::AnyLayer::Group(_) => (
                                egui::Color32::from_rgb(0, 150, 150), // Teal
                                egui::Color32::from_rgb(100, 220, 220), // Bright teal
                            ),
                        };

                        let (row, total_rows) = clip_stacking[clip_instance_index];
                        let (cy_min, cy_max) = clip_instance_y_bounds(row, total_rows);

                        let clip_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x + visible_start_x, y + cy_min),
                            egui::pos2(rect.min.x + visible_end_x, y + cy_max),
                        );

                        // Draw the clip instance background(s)
                        // For looping clips, draw each iteration as a separate rounded rect
                        // Use preview_clip_duration so trim drag previews don't show false loop iterations
                        let content_window_for_bg = preview_clip_duration.max(0.0);
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
                                            // preview_clip_duration accounts for TrimLeft/TrimRight drag previews
                                            let content_window = preview_clip_duration.max(0.0);
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
                                                        preview_trim_start,
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
                                                    preview_trim_start,
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
                                                // Chunked upload: track progress across frames
                                                let chunk = crate::waveform_gpu::UPLOAD_CHUNK_FRAMES;
                                                let progress = self.waveform_upload_progress.get(audio_pool_index).copied().unwrap_or(0);
                                                let next_end = (progress + chunk).min(total_frames);
                                                let frame_limit = Some(next_end);

                                                if next_end >= total_frames {
                                                    // Final chunk — done
                                                    waveform_gpu_dirty.remove(audio_pool_index);
                                                    self.waveform_upload_progress.remove(audio_pool_index);
                                                } else {
                                                    // More chunks needed
                                                    self.waveform_upload_progress.insert(*audio_pool_index, next_end);
                                                    ui.ctx().request_repaint();
                                                }

                                                Some(crate::waveform_gpu::PendingUpload {
                                                    samples: samples.clone(),
                                                    sample_rate: *sr,
                                                    channels: *ch,
                                                    frame_limit,
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
                                                        frame_limit: None, // recording uses incremental path
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

                        // VIDEO THUMBNAIL STRIP: Draw sequence of thumbnails inside clip rect
                        if let lightningbeam_core::layer::AnyLayer::Video(_) = layer {
                            visible_video_clip_ids.insert(clip_instance.clip_id);
                            let thumb_display_height = clip_rect.height() - 4.0;
                            if thumb_display_height > 8.0 {
                                let video_mgr = video_manager.lock().unwrap();
                                if let Some((tw, th, _)) = video_mgr.get_thumbnail_at(&clip_instance.clip_id, 0.0) {
                                    let aspect = tw as f32 / th as f32;
                                    let thumb_display_width = thumb_display_height * aspect;
                                    let thumb_step_px = thumb_display_width;

                                    let clip_width = clip_rect.width();
                                    let num_thumbs = ((clip_width / thumb_step_px).ceil() as usize).max(1);

                                    for i in 0..num_thumbs {
                                        let x_offset = i as f32 * thumb_step_px;
                                        if x_offset >= clip_width { break; }

                                        // Map pixel position to content time
                                        let time_offset = (x_offset as f64 + thumb_display_width as f64 * 0.5)
                                            / self.pixels_per_second as f64;
                                        let content_time = clip_instance.trim_start + time_offset;

                                        if let Some((tw, th, rgba_data)) = video_mgr.get_thumbnail_at(
                                            &clip_instance.clip_id, content_time
                                        ) {
                                            let ts_key = (content_time * 1000.0) as i64;
                                            let cache_key = (clip_instance.clip_id, ts_key);

                                            let texture = self.video_thumbnail_textures
                                                .entry(cache_key)
                                                .or_insert_with(|| {
                                                    let image = egui::ColorImage::from_rgba_unmultiplied(
                                                        [tw as usize, th as usize],
                                                        &rgba_data,
                                                    );
                                                    ui.ctx().load_texture(
                                                        format!("vthumb_{}_{}", clip_instance.clip_id, ts_key),
                                                        image,
                                                        egui::TextureOptions::LINEAR,
                                                    )
                                                });

                                            let full_rect = egui::Rect::from_min_size(
                                                egui::pos2(clip_rect.min.x + x_offset, clip_rect.min.y + 2.0),
                                                egui::vec2(thumb_display_width, thumb_display_height),
                                            );
                                            let thumb_rect = full_rect.intersect(clip_rect);

                                            if thumb_rect.width() > 2.0 && thumb_rect.height() > 2.0 {
                                                let uv_min = egui::pos2(
                                                    (thumb_rect.min.x - full_rect.min.x) / full_rect.width(),
                                                    (thumb_rect.min.y - full_rect.min.y) / full_rect.height(),
                                                );
                                                let uv_max = egui::pos2(
                                                    (thumb_rect.max.x - full_rect.min.x) / full_rect.width(),
                                                    (thumb_rect.max.y - full_rect.min.y) / full_rect.height(),
                                                );

                                                painter.image(
                                                    texture.id(),
                                                    thumb_rect,
                                                    egui::Rect::from_min_max(uv_min, uv_max),
                                                    egui::Color32::WHITE,
                                                );
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

        // Clean up stale video thumbnail textures for clips no longer visible
        self.video_thumbnail_textures.retain(|&(clip_id, _), _| visible_video_clip_ids.contains(&clip_id));

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
        focus: &mut lightningbeam_core::selection::FocusSelection,
        pending_actions: &mut Vec<Box<dyn lightningbeam_core::action::Action>>,
        playback_time: &mut f64,
        _is_playing: &mut bool,
        audio_controller: Option<&std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>>,
        context_layers: &[&lightningbeam_core::layer::AnyLayer],
        editing_clip_id: Option<&uuid::Uuid>,
    ) {
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
                    // Get the layer at this index (using virtual rows for group support)
                    let click_rows = build_timeline_rows(context_layers);
                    if clicked_layer_index < click_rows.len() {
                        let click_row = &click_rows[clicked_layer_index];
                        // Check collapsed groups first (merged spans)
                        if matches!(click_row, TimelineRow::CollapsedGroup { .. }) {
                            if let Some(child_ids) = self.detect_collapsed_group_at_pointer(
                                pos, document, content_rect, header_rect, editing_clip_id,
                            ) {
                                if !child_ids.is_empty() {
                                    if shift_held {
                                        for id in &child_ids {
                                            selection.add_clip_instance(*id);
                                        }
                                    } else {
                                        selection.clear_clip_instances();
                                        for id in &child_ids {
                                            selection.add_clip_instance(*id);
                                        }
                                    }
                                    *active_layer_id = Some(click_row.layer_id());
                                    *focus = lightningbeam_core::selection::FocusSelection::ClipInstances(selection.clip_instances().to_vec());
                                    clicked_clip_instance = true;
                                }
                            }
                        } else if let Some(layer) = click_row.as_any_layer() {
                            // Normal or GroupChild rows: check individual clips
                            let _layer_data = layer.layer();

                            // Get clip instances for this layer
                            let clip_instances: &[ClipInstance] = match layer {
                                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
                                lightningbeam_core::layer::AnyLayer::Group(_) => &[],
                            };

                            // Check if click is within any clip instance
                            let click_stacking = compute_clip_stacking(document, layer, clip_instances);
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
                                    let (row, total_rows) = click_stacking[ci_idx];
                                    let (cy_min, cy_max) = clip_instance_y_bounds(row, total_rows);
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
                                        *focus = lightningbeam_core::selection::FocusSelection::ClipInstances(selection.clip_instances().to_vec());
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

        // Layer header drag-to-reorder (manual pointer tracking, no allocate_rect)
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

        // Handle layer header selection on mousedown (immediate, not on release)
        if primary_pressed && !alt_held && !self.layer_control_clicked {
            if let Some(pos) = pointer_pos {
                if header_rect.contains(pos) {
                    let relative_y = pos.y - header_rect.min.y + self.viewport_scroll_y;
                    let clicked_layer_index = (relative_y / LAYER_HEIGHT) as usize;
                    let header_rows = build_timeline_rows(context_layers);
                    if clicked_layer_index < header_rows.len() {
                        let layer_id = header_rows[clicked_layer_index].layer_id();
                        let clicked_parent = header_rows[clicked_layer_index].parent_id();
                        *active_layer_id = Some(layer_id);
                        if shift_held {
                            shift_toggle_layer(focus, layer_id, clicked_parent, &header_rows);
                        } else {
                            // Only change selection if the clicked layer isn't already selected
                            let already_selected = match focus {
                                lightningbeam_core::selection::FocusSelection::Layers(ids) => ids.contains(&layer_id),
                                _ => false,
                            };
                            if !already_selected {
                                *focus = lightningbeam_core::selection::FocusSelection::Layers(vec![layer_id]);
                            }
                        }
                    }
                    // Also record for potential drag
                    self.header_mousedown_pos = Some(pos);
                }
            }
        }

        // Start drag after movement threshold (4px)
        const LAYER_DRAG_THRESHOLD: f32 = 4.0;
        if self.layer_drag.is_none() && !self.layer_control_clicked {
            if let (Some(down_pos), Some(cur_pos)) = (self.header_mousedown_pos, pointer_pos) {
                if primary_down && (cur_pos - down_pos).length() > LAYER_DRAG_THRESHOLD {
                    let relative_y = down_pos.y - header_rect.min.y + self.viewport_scroll_y;
                    let clicked_index = (relative_y / LAYER_HEIGHT) as usize;
                    let drag_rows = build_timeline_rows(context_layers);
                    if clicked_index < drag_rows.len() {
                        // Collect all selected layer IDs (in visual order)
                        let selected_ids: Vec<uuid::Uuid> = match focus {
                            lightningbeam_core::selection::FocusSelection::Layers(ids) => {
                                // Filter to only IDs present in the row list, in visual order
                                drag_rows.iter()
                                    .filter(|r| ids.contains(&r.layer_id()))
                                    .map(|r| r.layer_id())
                                    .collect()
                            }
                            _ => vec![drag_rows[clicked_index].layer_id()],
                        };
                        // If clicked layer isn't in selection, just drag that one
                        let clicked_id = drag_rows[clicked_index].layer_id();
                        let layer_ids = if selected_ids.contains(&clicked_id) {
                            selected_ids
                        } else {
                            vec![clicked_id]
                        };

                        // Find source parent IDs for each dragged layer
                        let source_parent_ids: Vec<Option<uuid::Uuid>> = layer_ids.iter()
                            .map(|lid| drag_rows.iter().find(|r| r.layer_id() == *lid).and_then(|r| r.parent_id()))
                            .collect();

                        // Find the visual index of the first dragged layer
                        let first_drag_visual_idx = drag_rows.iter()
                            .position(|r| r.layer_id() == layer_ids[0])
                            .unwrap_or(0);

                        // Compute gap index in the filtered list
                        let gap_index = drag_rows.iter()
                            .take(first_drag_visual_idx)
                            .filter(|r| !layer_ids.contains(&r.layer_id()))
                            .count();

                        // Grab offset: ensure the clicked layer stays under the cursor
                        // in the stacked floating header view
                        let clicked_row_y = header_rect.min.y + clicked_index as f32 * LAYER_HEIGHT - self.viewport_scroll_y;
                        let clicked_within_drag = layer_ids.iter().position(|id| *id == clicked_id).unwrap_or(0);
                        let grab_offset = down_pos.y - clicked_row_y + clicked_within_drag as f32 * LAYER_HEIGHT;

                        self.layer_drag = Some(LayerDragState {
                            layer_ids,
                            source_parent_ids,
                            gap_row_index: gap_index,
                            current_mouse_y: cur_pos.y,
                            grab_offset_y: grab_offset,
                        });
                    }
                    self.header_mousedown_pos = None; // consumed
                }
            }
        }

        // Update gap position and mouse Y during layer drag
        if let Some(ref mut drag) = self.layer_drag {
            if primary_down {
                if let Some(pos) = pointer_pos {
                    drag.current_mouse_y = pos.y;
                    let relative_y = pos.y - drag.grab_offset_y - header_rect.min.y + self.viewport_scroll_y + LAYER_HEIGHT * 0.5;
                    let all_rows = build_timeline_rows(context_layers);
                    let filtered_count = all_rows.iter()
                        .filter(|r| !drag.layer_ids.contains(&r.layer_id()))
                        .count();
                    let target = ((relative_y / LAYER_HEIGHT) as usize).min(filtered_count);
                    drag.gap_row_index = target;
                }
                ui.ctx().request_repaint();
            }
        }

        // Drop layers on mouse release
        if self.layer_drag.is_some() && primary_released {
            let drag = self.layer_drag.take().unwrap();

            // Build the row list to determine where the gap lands
            let drop_rows = build_timeline_rows(context_layers);
            let filtered_rows: Vec<&TimelineRow> = drop_rows.iter()
                .filter(|r| !drag.layer_ids.contains(&r.layer_id()))
                .collect();

            // Determine target parent from the row above the gap
            let new_parent_id = if drag.gap_row_index == 0 {
                None // top of list = root
            } else {
                let row_above = &filtered_rows[drag.gap_row_index.min(filtered_rows.len()) - 1];
                row_above.parent_id()
            };

            // Compute insertion index in new parent's children vec AFTER dragged layers are removed.
            // Get the new parent's children, filter out all dragged layers, find where the
            // row-above falls in that filtered list.
            let new_children: Vec<uuid::Uuid> = match new_parent_id {
                None => context_layers.iter().map(|l| l.id()).collect(),
                Some(pid) => {
                    if let Some(AnyLayer::Group(g)) = document.root.get_child(&pid) {
                        g.children.iter().map(|l| l.id()).collect()
                    } else {
                        vec![]
                    }
                }
            };
            let new_children_filtered: Vec<uuid::Uuid> = new_children.iter()
                .filter(|id| !drag.layer_ids.contains(id))
                .copied()
                .collect();

            let new_base_index = if drag.gap_row_index == 0 {
                // Gap at top = visually topmost position.
                // Since timeline reverses children, this is the end of the children vec.
                new_children_filtered.len()
            } else {
                let row_above = &filtered_rows[drag.gap_row_index.min(filtered_rows.len()) - 1];
                let above_id = row_above.layer_id();
                if let Some(pos) = new_children_filtered.iter().position(|&id| id == above_id) {
                    // Insert before it in children vec (visually below = lower children index)
                    pos
                } else {
                    new_children_filtered.len()
                }
            };

            // Build layer list: (layer_id, old_parent_id) in visual order
            let layers: Vec<(uuid::Uuid, Option<uuid::Uuid>)> = drag.layer_ids.iter()
                .zip(drag.source_parent_ids.iter())
                .map(|(id, pid)| (*id, *pid))
                .collect();

            // Only create action if something actually changed
            let anything_changed = layers.iter().enumerate().any(|(i, (lid, old_pid))| {
                if *old_pid != new_parent_id {
                    return true;
                }
                // Check if position changed within same parent
                let old_idx = new_children.iter().position(|id| id == lid);
                let target_idx_in_original = if new_base_index < new_children_filtered.len() {
                    // Find where new_children_filtered[new_base_index] sits in original
                    new_children.iter().position(|id| *id == new_children_filtered[new_base_index])
                        .map(|p| p + i)
                } else {
                    Some(0 + i) // inserting at start of children (end of filtered = start of original)
                };
                old_idx != target_idx_in_original
            });

            if anything_changed {
                pending_actions.push(Box::new(
                    lightningbeam_core::actions::MoveLayerAction::new(
                        layers,
                        new_parent_id,
                        new_base_index,
                    ),
                ));
            }
        }

        // Clear header mousedown if released without starting a drag
        if primary_released {
            self.header_mousedown_pos = None;
        }
        // Cancel layer drag if pointer is no longer down
        if self.layer_drag.is_some() && !primary_down {
            self.layer_drag = None;
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
                        editing_clip_id,
                    ) {
                        // If this clip is not selected, select it (respecting shift key)
                        if !selection.contains_clip_instance(&clip_id) {
                            if shift_held {
                                selection.add_clip_instance(clip_id);
                            } else {
                                selection.select_only_clip_instance(clip_id);
                            }
                            *focus = lightningbeam_core::selection::FocusSelection::ClipInstances(selection.clip_instances().to_vec());
                        }

                        // Start dragging with the detected drag type
                        self.clip_drag_state = Some(drag_type);
                        self.drag_offset = 0.0;
                    } else if let Some(child_ids) = self.detect_collapsed_group_at_pointer(
                        mousedown_pos,
                        document,
                        content_rect,
                        header_rect,
                        editing_clip_id,
                    ) {
                        // Collapsed group merged span — select all child clips and start Move drag
                        if !child_ids.is_empty() {
                            if !shift_held {
                                selection.clear_clip_instances();
                            }
                            for id in &child_ids {
                                selection.add_clip_instance(*id);
                            }
                            *focus = lightningbeam_core::selection::FocusSelection::ClipInstances(selection.clip_instances().to_vec());
                            self.clip_drag_state = Some(ClipDragType::Move);
                            self.drag_offset = 0.0;
                        }
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

                // Iterate through all layers (including group children) to find selected clip instances
                for (layer, clip_instances) in all_layer_clip_instances(context_layers) {
                    let layer_id = layer.id();
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

                            // Iterate through all layers (including group children) to find selected clip instances
                            for (layer, clip_instances) in all_layer_clip_instances(context_layers) {
                                let layer_id = layer.id();

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

                            for (layer, clip_instances) in all_layer_clip_instances(context_layers) {
                                let layer_id = layer.id();

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

                            for (layer, clip_instances) in all_layer_clip_instances(context_layers) {
                                let layer_id = layer.id();

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

                    // Get the layer at this index (using virtual rows for group support)
                    let empty_click_rows = build_timeline_rows(context_layers);
                    if clicked_layer_index < empty_click_rows.len() {
                        let layer_id = empty_click_rows[clicked_layer_index].layer_id();
                        let clicked_parent = empty_click_rows[clicked_layer_index].parent_id();
                        *active_layer_id = Some(layer_id);
                        if shift_held {
                            shift_toggle_layer(focus, layer_id, clicked_parent, &empty_click_rows);
                        } else {
                            selection.clear_clip_instances();
                            *focus = lightningbeam_core::selection::FocusSelection::Layers(vec![layer_id]);
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
                    editing_clip_id,
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

        // Time display (format-dependent)
        {
            let (bpm, time_sig_num, time_sig_den) = {
                let doc = shared.action_executor.document();
                (doc.bpm, doc.time_signature.numerator, doc.time_signature.denominator)
            };

            match self.time_display_format {
                TimeDisplayFormat::Seconds => {
                    ui.colored_label(text_color, format!("Time: {:.2}s / {:.2}s", *shared.playback_time, self.duration));
                }
                TimeDisplayFormat::Measures => {
                    let time_sig = lightningbeam_core::document::TimeSignature { numerator: time_sig_num, denominator: time_sig_den };
                    let pos = lightningbeam_core::beat_time::time_to_measure(
                        *shared.playback_time, bpm, &time_sig,
                    );
                    ui.colored_label(text_color, format!(
                        "BAR: {}.{}  |  BPM: {:.0}  |  {}/{}",
                        pos.measure, pos.beat, bpm,
                        time_sig_num, time_sig_den,
                    ));
                }
            }

            ui.separator();

            // Zoom display
            ui.colored_label(text_color, format!("Zoom: {:.0}px/s", self.pixels_per_second));

            ui.separator();

            // Time display format toggle
            egui::ComboBox::from_id_salt("time_format")
                .selected_text(match self.time_display_format {
                    TimeDisplayFormat::Seconds => "Seconds",
                    TimeDisplayFormat::Measures => "Measures",
                })
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.time_display_format, TimeDisplayFormat::Seconds, "Seconds");
                    ui.selectable_value(&mut self.time_display_format, TimeDisplayFormat::Measures, "Measures");
                });

            ui.separator();

            // BPM control
            let mut bpm_val = bpm;
            ui.label("BPM:");
            let bpm_response = ui.add(egui::DragValue::new(&mut bpm_val)
                .range(20.0..=300.0)
                .speed(0.5)
                .fixed_decimals(1));
            if bpm_response.changed() {
                shared.action_executor.document_mut().bpm = bpm_val;
                if let Some(controller_arc) = shared.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.set_tempo(bpm_val as f32, (time_sig_num, time_sig_den));
                }
            }

            ui.separator();

            // Time signature selector
            let time_sig_presets: [(u32, u32); 8] = [
                (2, 4), (3, 4), (4, 4), (5, 4),
                (6, 8), (7, 8), (9, 8), (12, 8),
            ];
            let current_ts_label = format!("{}/{}", time_sig_num, time_sig_den);
            egui::ComboBox::from_id_salt("time_sig")
                .selected_text(&current_ts_label)
                .width(60.0)
                .show_ui(ui, |ui| {
                    for (num, den) in &time_sig_presets {
                        let label = format!("{}/{}", num, den);
                        if ui.selectable_label(
                            time_sig_num == *num && time_sig_den == *den,
                            &label,
                        ).clicked() {
                            let doc = shared.action_executor.document_mut();
                            doc.time_signature.numerator = *num;
                            doc.time_signature.denominator = *den;
                            if let Some(controller_arc) = shared.audio_controller {
                                let mut controller = controller_arc.lock().unwrap();
                                controller.set_tempo(doc.bpm as f32, (*num, *den));
                            }
                        }
                    }
                });
        }

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
        let editing_clip_id = shared.editing_clip_id;
        let context_layers = document.context_layers(editing_clip_id.as_ref());
        // Use virtual row count (includes expanded group children) for height calculations
        let layer_count = build_timeline_rows(&context_layers).len();

        // Calculate project duration from last clip endpoint across all layers
        let mut max_endpoint: f64 = 10.0; // Default minimum duration
        for &layer in &context_layers {
            let clip_instances: &[ClipInstance] = match layer {
                lightningbeam_core::layer::AnyLayer::Vector(vl) => &vl.clip_instances,
                lightningbeam_core::layer::AnyLayer::Audio(al) => &al.clip_instances,
                lightningbeam_core::layer::AnyLayer::Video(vl) => &vl.clip_instances,
                lightningbeam_core::layer::AnyLayer::Effect(el) => &el.clip_instances,
                lightningbeam_core::layer::AnyLayer::Group(_) => &[],
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
        self.render_layer_headers(ui, layer_headers_rect, shared.theme, shared.active_layer_id, shared.focus, &mut shared.pending_actions, document, &context_layers);

        // Render time ruler (clip to ruler rect)
        ui.set_clip_rect(ruler_rect.intersect(original_clip_rect));
        self.render_ruler(ui, ruler_rect, shared.theme, document.bpm, &document.time_signature);

        // Render layer rows with clipping
        ui.set_clip_rect(content_rect.intersect(original_clip_rect));
        let video_clip_hovers = self.render_layers(ui, content_rect, shared.theme, document, shared.active_layer_id, shared.focus, shared.selection, shared.midi_event_cache, shared.raw_audio_cache, shared.waveform_gpu_dirty, shared.target_format, shared.waveform_stereo, &context_layers, shared.video_manager);

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
            shared.focus,
            shared.pending_actions,
            shared.playback_time,
            shared.is_playing,
            shared.audio_controller,
            &context_layers,
            editing_clip_id.as_ref(),
        );

        // Context menu: detect right-click on clips or empty timeline space
        let mut just_opened_menu = false;
        let secondary_clicked = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
        if secondary_clicked {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if content_rect.contains(pos) {
                    if let Some((_drag_type, clip_id)) = self.detect_clip_at_pointer(pos, document, content_rect, layer_headers_rect, editing_clip_id.as_ref()) {
                        // Right-clicked on a clip
                        if !shared.selection.contains_clip_instance(&clip_id) {
                            shared.selection.select_only_clip_instance(clip_id);
                        }
                        *shared.focus = lightningbeam_core::selection::FocusSelection::ClipInstances(shared.selection.clip_instances().to_vec());
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
                            AnyLayer::Group(_) => &[],
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
                            AnyLayer::Group(_) => &[],
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

                    // Get the layer at this index (using virtual rows for group support)
                    let drop_rows = build_timeline_rows(&context_layers);

                    let drop_layer = drop_rows.get(hovered_layer_index).and_then(|r| r.as_any_layer());
                    if let Some(layer) = drop_layer {
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
                                        let result = find_sampled_audio_track_for_clip(doc, linked_audio_clip_id, drop_time, editing_clip_id.as_ref());
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

                            // Add the layer to the current editing context
                            if let Some(clip_id) = shared.editing_clip_id {
                                if let Some(clip) = shared.action_executor.document_mut().vector_clips.get_mut(&clip_id) {
                                    clip.layers.add_root(new_layer);
                                }
                                shared.action_executor.document_mut().layer_to_clip_map.insert(new_layer_id, clip_id);
                            } else {
                                shared.action_executor.document_mut().root.add_child(new_layer);
                            }

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

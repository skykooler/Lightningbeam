//! Clip system for Lightningbeam
//!
//! Clips are reusable compositions that can contain layers and be instantiated multiple times.
//! Similar to MovieClips in Flash or Compositions in After Effects.
//!
//! ## Architecture
//!
//! - **Clip**: The reusable definition (VectorClip, VideoClip, AudioClip)
//! - **ClipInstance**: An instance of a clip with its own transform, timing, and playback properties
//!
//! Multiple ClipInstances can reference the same Clip, each with different positions,
//! timing windows, and playback speeds.

use crate::layer::AnyLayer;
use crate::layer_tree::LayerTree;
use crate::object::Transform;
use daw_backend::{Beats, ContentTime, Seconds};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use vello::kurbo::Rect;

/// Vector clip containing nested layers
///
/// A VectorClip is a composition that contains its own layer hierarchy.
/// Multiple ClipInstances can reference the same VectorClip, each with
/// their own transform and timing properties.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VectorClip {
    /// Unique identifier
    pub id: Uuid,

    /// Clip name
    pub name: String,

    /// Canvas width in pixels
    pub width: f64,

    /// Canvas height in pixels
    pub height: f64,

    /// Duration in seconds
    pub duration: f64,

    /// Nested layer hierarchy
    pub layers: LayerTree<AnyLayer>,

    /// Whether this clip is a group (static collection) rather than an animated clip.
    /// Groups have their timeline extent determined by keyframe spans on the containing layer,
    /// not by their internal duration.
    #[serde(default)]
    pub is_group: bool,

    /// Folder this clip belongs to (None = root of category)
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

impl VectorClip {
    /// Create a new vector clip
    pub fn new(name: impl Into<String>, width: f64, height: f64, duration: f64) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            width,
            height,
            duration,
            layers: LayerTree::new(),
            is_group: false,
            folder_id: None,
        }
    }

    /// Create with a specific ID
    pub fn with_id(
        id: Uuid,
        name: impl Into<String>,
        width: f64,
        height: f64,
        duration: f64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            width,
            height,
            duration,
            layers: LayerTree::new(),
            is_group: false,
            folder_id: None,
        }
    }

    /// Calculate the duration of this clip based on its internal content.
    ///
    /// Considers:
    /// - Vector layer keyframes (last keyframe time + one frame)
    /// - Audio/video/effect layer clip instances (timeline_start + effective duration)
    ///
    /// The `clip_duration_fn` resolves referenced clip durations for non-vector layers.
    /// Falls back to the stored `duration` field if no content exists.
    pub fn content_duration(&self, framerate: f64, tempo_map: &crate::tempo_map::TempoMap) -> f64 {
        self.content_duration_with(framerate, tempo_map, |_| None)
    }

    /// Like `content_duration`, but with a closure that resolves clip durations
    /// for audio/video/effect clip instances inside this movie clip.
    ///
    /// `clip_duration_fn` returns clip content duration **in seconds**.
    /// Result is in **seconds**.
    pub fn content_duration_with(&self, framerate: f64, tempo_map: &crate::tempo_map::TempoMap, clip_duration_fn: impl Fn(&Uuid) -> Option<f64>) -> f64 {
        let frame_duration = 1.0 / framerate;
        // Work in beats, convert to seconds at the end.
        let mut last_beats: Option<Beats> = None;
        let mut last_secs: Option<f64> = None;

        for layer_node in self.layers.iter() {
            // Check clip instances on ALL layer types (vector, audio, video, effect)
            let clip_instances: &[ClipInstance] = match &layer_node.data {
                AnyLayer::Vector(vl) => &vl.clip_instances,
                AnyLayer::Audio(al) => &al.clip_instances,
                AnyLayer::Video(vl) => &vl.clip_instances,
                AnyLayer::Effect(el) => &el.clip_instances,
                AnyLayer::Group(_) => &[],
                AnyLayer::Raster(_) => &[],
                AnyLayer::Text(_) => &[],
            };
            for ci in clip_instances {
                // Compute end position of this clip instance in beats
                let end_beats: Beats = if let Some(td_beats) = ci.timeline_duration {
                    ci.timeline_start + td_beats
                } else if let Some(te) = ci.trim_end {
                    // `clip_duration_fn` hands back seconds, so this whole path treats nested
                    // content as wall-clock. That's right for the vector/video/audio clips a vector
                    // clip actually nests; a nested MIDI clip (beats content) would need resolving
                    // against its clip, which this callback can't do. Pre-existing limitation.
                    let secs = (te - ci.trim_start).raw().max(0.0);
                    tempo_map.seconds_to_beats(tempo_map.beats_to_seconds(ci.timeline_start) + Seconds(secs))
                } else if let Some(clip_dur_secs) = clip_duration_fn(&ci.clip_id) {
                    let secs = (clip_dur_secs - ci.trim_start.raw()).max(0.0);
                    tempo_map.seconds_to_beats(tempo_map.beats_to_seconds(ci.timeline_start) + Seconds(secs))
                } else {
                    continue;
                };
                last_beats = Some(last_beats.map_or(end_beats, |t: Beats| t.max(end_beats)));
            }

            // Vector layer keyframes are in seconds
            if let AnyLayer::Vector(vector_layer) = &layer_node.data {
                if let Some(last_kf) = vector_layer.keyframes.last() {
                    last_secs = Some(last_secs.map_or(last_kf.time, |t: f64| t.max(last_kf.time)));
                }
            }
        }

        let from_clips = last_beats.map(|b| tempo_map.beats_to_seconds(b).seconds_to_f64());
        let combined = match (from_clips, last_secs) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        match combined {
            Some(t) => t + frame_duration,
            None => self.duration,
        }
    }

    /// Calculate the bounding box of all content in this clip at a specific time
    ///
    /// This recursively calculates the union of all shape and nested clip bounding boxes
    /// across all layers, evaluating animations at the specified clip-local time.
    ///
    /// # Arguments
    /// * `document` - The document containing all clip definitions (for resolving nested clips)
    /// * `clip_time` - The time within this clip (already converted from timeline time)
    ///
    /// # Returns
    /// The bounding box of all visible content at the specified time
    pub fn calculate_content_bounds(&self, document: &crate::document::Document, clip_time: f64) -> Rect {
        let mut combined_bounds: Option<Rect> = None;

        // Iterate through all layers in the layer tree
        for layer_node in self.layers.iter() {
            // Only process vector layers (skip other layer types)
            if let AnyLayer::Vector(vector_layer) = &layer_node.data {
                // Calculate bounds from DCEL edges
                if let Some(dcel) = vector_layer.graph_at_time(clip_time) {
                    use kurbo::Shape as KurboShape;
                    for edge in &dcel.edges {
                        if edge.deleted {
                            continue;
                        }
                        let edge_bbox = edge.curve.bounding_box();
                        combined_bounds = Some(match combined_bounds {
                            None => edge_bbox,
                            Some(existing) => existing.union(edge_bbox),
                        });
                    }
                }

                // Handle nested clip instances recursively
                for clip_instance in &vector_layer.clip_instances {
                    // Convert parent clip time (seconds) to nested clip local time (seconds).
                    // timeline_start is in beats; convert to seconds using document BPM.
                    let start_secs = document.tempo_map().beats_to_seconds(clip_instance.timeline_start).seconds_to_f64();
                    // Nested clips here are vector clips, whose content is wall-clock seconds.
                    let nested_clip_time =
                        ((clip_time - start_secs) * clip_instance.playback_speed) + clip_instance.trim_start.raw();

                    // Look up the nested clip definition
                    let nested_bounds = if let Some(nested_clip) = document.get_vector_clip(&clip_instance.clip_id) {
                        // Recursively calculate bounds for nested clip at its local time
                        nested_clip.calculate_content_bounds(document, nested_clip_time)
                    } else if let Some(video_clip) = document.get_video_clip(&clip_instance.clip_id) {
                        // Video clips have fixed dimensions
                        Rect::new(0.0, 0.0, video_clip.width, video_clip.height)
                    } else {
                        // Clip not found or is audio (no spatial representation)
                        continue;
                    };

                    // Apply clip instance transform to the nested bounds
                    let instance_transform = clip_instance.transform.to_affine();
                    let transformed_bounds = instance_transform.transform_rect_bbox(nested_bounds);

                    // Union with combined bounds
                    combined_bounds = Some(match combined_bounds {
                        None => transformed_bounds,
                        Some(existing) => existing.union(transformed_bounds),
                    });
                }
            } else if let AnyLayer::Text(text_layer) = &layer_node.data {
                // Text layers contribute their box bounds (so a text-only clip is
                // selectable/draggable, not a degenerate point).
                let r = Rect::from_origin_size(
                    text_layer.box_origin,
                    (text_layer.box_width, text_layer.box_height),
                );
                combined_bounds = Some(match combined_bounds {
                    None => r,
                    Some(existing) => existing.union(r),
                });
            }
        }

        // If no content found, return a small rect at origin
        combined_bounds.unwrap_or_else(|| Rect::new(0.0, 0.0, 1.0, 1.0))
    }

    /// Get the width of the content bounds at a specific time
    pub fn content_width(&self, document: &crate::document::Document, clip_time: f64) -> f64 {
        self.calculate_content_bounds(document, clip_time).width()
    }

    /// Get the height of the content bounds at a specific time
    pub fn content_height(&self, document: &crate::document::Document, clip_time: f64) -> f64 {
        self.calculate_content_bounds(document, clip_time).height()
    }
}

/// Image asset for static images
///
/// Images can be used as fill textures for shapes or (in the future)
/// added to video tracks as still frames. Unlike clips, images don't
/// have a duration or timeline properties.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageAsset {
    /// Unique identifier
    pub id: Uuid,

    /// Asset name (usually derived from filename)
    pub name: String,

    /// Original file path
    pub path: PathBuf,

    /// Image width in pixels
    pub width: u32,

    /// Image height in pixels
    pub height: u32,

    /// Raw image file bytes. NOT serialized to project JSON — persisted as a
    /// `MediaKind::ImageAsset` row in the `.beam` container (chunked, pageable) and
    /// read back on load. `default` so new projects (bytes in the container, not JSON)
    /// deserialize; old projects with base64-embedded `data` still load via deserialize.
    #[serde(default, skip_serializing)]
    pub data: Option<Vec<u8>>,

    /// Folder this asset belongs to (None = root of category)
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

impl ImageAsset {
    /// Create a new image asset
    pub fn new(
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            path: path.into(),
            width,
            height,
            data: None,
            folder_id: None,
        }
    }

    /// Create with embedded data
    pub fn with_data(
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        width: u32,
        height: u32,
        data: Vec<u8>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            path: path.into(),
            width,
            height,
            data: Some(data),
            folder_id: None,
        }
    }
}

/// Video clip referencing an external video file
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoClip {
    /// Unique identifier
    pub id: Uuid,

    /// Clip name
    pub name: String,

    /// Path to video file
    pub file_path: String,

    /// Video width in pixels
    pub width: f64,

    /// Video height in pixels
    pub height: f64,

    /// Duration in seconds (from video metadata)
    pub duration: f64,

    /// Frame rate (from video metadata)
    pub frame_rate: f64,

    /// Optional linked audio clip (extracted from video file)
    /// When set, the audio clip should be moved/trimmed in sync with this video clip
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_audio_clip_id: Option<Uuid>,

    /// When set, the video bytes are packed into the `.beam` container under this
    /// media id (== the clip id) and decoded by streaming from the SQLite blob.
    /// `None` means the video is referenced externally via [`Self::file_path`].
    /// Reconstructed from the archive on load (see `load_beam_sqlite`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<Uuid>,

    /// Folder this clip belongs to (None = root of category)
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

impl VideoClip {
    /// Create a new video clip
    pub fn new(
        name: impl Into<String>,
        file_path: impl Into<String>,
        width: f64,
        height: f64,
        duration: f64,
        frame_rate: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            file_path: file_path.into(),
            width,
            height,
            duration,
            frame_rate,
            linked_audio_clip_id: None,
            media_id: None,
            folder_id: None,
        }
    }
}

/// MIDI event representing a single MIDI message
///
/// Compatible with daw-backend's MidiEvent structure
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MidiEvent {
    /// Time position within the clip in seconds
    pub timestamp: f64,
    /// MIDI status byte (includes channel)
    pub status: u8,
    /// First data byte (note number, CC number, etc.)
    pub data1: u8,
    /// Second data byte (velocity, CC value, etc.)
    pub data2: u8,
}

impl MidiEvent {
    /// Create a new MIDI event
    pub fn new(timestamp: f64, status: u8, data1: u8, data2: u8) -> Self {
        Self {
            timestamp,
            status,
            data1,
            data2,
        }
    }

    /// Create a note on event
    pub fn note_on(timestamp: f64, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            timestamp,
            status: 0x90 | (channel & 0x0F),
            data1: note,
            data2: velocity,
        }
    }

    /// Create a note off event
    pub fn note_off(timestamp: f64, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            timestamp,
            status: 0x80 | (channel & 0x0F),
            data1: note,
            data2: velocity,
        }
    }
}

/// Audio clip type
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AudioClipType {
    /// Sampled audio from a file
    ///
    /// References audio data in a shared AudioPool (managed by daw-backend).
    /// Compatible with daw-backend's Clip structure.
    Sampled {
        /// Index into the audio pool (references AudioFile)
        /// This allows sharing audio data between multiple clip instances
        audio_pool_index: usize,
    },
    /// MIDI sequence
    ///
    /// References MIDI data in the backend's MidiClipPool.
    /// The clip content is stored in daw-backend, not duplicated here.
    Midi {
        /// Backend MIDI clip ID (references MidiClip in backend pool)
        /// This allows sharing MIDI data between multiple clip instances
        midi_clip_id: u32,
    },
    /// Recording in progress
    ///
    /// Placeholder for a clip that is currently being recorded.
    /// The audio_pool_index will be assigned when recording stops.
    Recording,
}

/// One take of a cycle recording — see [`ClipInstance::takes`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioTake {
    /// Display name, e.g. "Take 1". User-editable.
    pub name: String,
    /// The recorded content this take points at.
    pub content: TakeContent,
}

/// What a take actually holds. An instance's takes are all the same kind — one cycle-record session
/// captures either audio or MIDI, never a mix.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TakeContent {
    /// Sampled audio: index into the audio pool.
    Audio { audio_pool_index: usize },
    /// MIDI: backend MIDI clip ID.
    Midi { midi_clip_id: u32 },
}

/// A clip's content duration, tagged by its native unit.
///
/// Sampled/recording audio and video measure content in wall-clock **seconds**; MIDI measures
/// it in **beats** (tempo-independent musical length). Carrying the domain in the type means a
/// duration can't be silently read in the wrong unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipDuration {
    Seconds(Seconds),
    Beats(Beats),
}

impl ClipDuration {
    /// Wall-clock seconds. Beats are converted as a length from beat 0 (exact under constant
    /// tempo; a reasonable approximation otherwise — durations are position-independent here).
    pub fn to_seconds(self, tempo_map: &daw_backend::TempoMap) -> Seconds {
        match self {
            ClipDuration::Seconds(s) => s,
            ClipDuration::Beats(b) => tempo_map.beats_to_seconds(b),
        }
    }

    /// The raw magnitude in the clip's native unit. Use only in code that already works in that
    /// domain (e.g. trim math, whose values share the clip's native domain).
    pub fn native(self) -> f64 {
        match self {
            ClipDuration::Seconds(s) => s.seconds_to_f64(),
            ClipDuration::Beats(b) => b.beats_to_f64(),
        }
    }

    /// Tag a [`ContentTime`] with *this* duration's domain.
    ///
    /// Handy when you already hold a clip's content duration (so you know the domain) and need to
    /// resolve one of its trim bounds, without going back to the clip.
    pub fn same_domain(self, t: ContentTime) -> ClipDuration {
        match self {
            ClipDuration::Seconds(_) => ClipDuration::Seconds(Seconds(t.raw())),
            ClipDuration::Beats(_) => ClipDuration::Beats(Beats(t.raw())),
        }
    }
}

/// Audio clip
///
/// This is compatible with daw-backend's audio system:
/// - Sampled audio references data in AudioPool (managed externally)
/// - MIDI audio stores events directly in the clip
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioClip {
    /// Unique identifier
    pub id: Uuid,

    /// Clip name
    pub name: String,

    /// Raw content duration in the clip's **native domain** — SECONDS for sampled/recording
    /// audio, BEATS for MIDI (musical length, tempo-independent). Private on purpose: the domain
    /// depends on `clip_type`, so all access goes through [`AudioClip::content_duration`] /
    /// [`AudioClip::set_content_duration`], which keep it type-safe. Stored as a bare `f64`
    /// because the `.beam` format serializes it as a plain number (serde derives over private
    /// fields fine); a domain-tagged newtype would change the on-disk shape.
    duration: f64,

    /// Audio clip type (sampled or MIDI)
    pub clip_type: AudioClipType,

    /// Folder this clip belongs to (None = root of category)
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

impl AudioClip {
    /// The clip's content duration, tagged with its native domain (seconds for sampled/recording,
    /// beats for MIDI). This is the only sanctioned way to read the raw `duration` field.
    pub fn content_duration(&self) -> ClipDuration {
        if self.is_midi_domain() {
            ClipDuration::Beats(Beats(self.duration))
        } else {
            ClipDuration::Seconds(Seconds(self.duration))
        }
    }

    /// Set the content duration. Debug-asserts the value's domain matches the clip type so a
    /// beats duration can't be stored on a seconds clip (or vice-versa).
    pub fn set_content_duration(&mut self, duration: ClipDuration) {
        debug_assert!(
            matches!(
                (self.is_midi_domain(), duration),
                (true, ClipDuration::Beats(_)) | (false, ClipDuration::Seconds(_))
            ),
            "clip duration domain must match clip type",
        );
        self.duration = duration.native();
    }

    /// Whether this clip's `duration` is measured in beats (MIDI) rather than seconds.
    fn is_midi_domain(&self) -> bool {
        matches!(self.clip_type, AudioClipType::Midi { .. })
    }

    /// Create a new sampled audio clip
    ///
    /// # Arguments
    /// * `name` - Clip name
    /// * `audio_pool_index` - Index into the AudioPool (from daw-backend)
    /// * `duration` - Clip duration (can be shorter than source file for trimming)
    pub fn new_sampled(name: impl Into<String>, audio_pool_index: usize, duration: f64) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            duration,
            clip_type: AudioClipType::Sampled { audio_pool_index },
            folder_id: None,
        }
    }

    /// Create a new MIDI clip
    ///
    /// # Arguments
    /// * `name` - Clip name
    /// * `midi_clip_id` - Backend MIDI clip ID (from daw-backend MidiClipPool)
    /// * `duration` - Clip duration
    pub fn new_midi(
        name: impl Into<String>,
        midi_clip_id: u32,
        duration: daw_backend::Beats,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            duration: duration.beats_to_f64(),
            clip_type: AudioClipType::Midi { midi_clip_id },
            folder_id: None,
        }
    }

    /// Create a new recording-in-progress clip
    ///
    /// This is a placeholder clip for audio currently being recorded.
    /// Call `finalize_recording` when recording stops to set the pool index.
    pub fn new_recording(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            duration: 0.0, // Will be updated as recording progresses
            clip_type: AudioClipType::Recording,
            folder_id: None,
        }
    }

    /// Finalize a recording clip with the actual audio pool index and duration
    ///
    /// Returns true if the clip was a Recording type and was successfully finalized.
    pub fn finalize_recording(&mut self, audio_pool_index: usize, duration: f64) -> bool {
        if matches!(self.clip_type, AudioClipType::Recording) {
            self.clip_type = AudioClipType::Sampled { audio_pool_index };
            self.duration = duration;
            true
        } else {
            false
        }
    }

    /// Check if this clip is a recording in progress
    pub fn is_recording(&self) -> bool {
        matches!(self.clip_type, AudioClipType::Recording)
    }

    /// Get the audio pool index if this is a sampled audio clip
    pub fn audio_pool_index(&self) -> Option<usize> {
        match &self.clip_type {
            AudioClipType::Sampled { audio_pool_index } => Some(*audio_pool_index),
            _ => None,
        }
    }

    /// Get backend MIDI clip ID if this is a MIDI clip
    pub fn midi_clip_id(&self) -> Option<u32> {
        match &self.clip_type {
            AudioClipType::Midi { midi_clip_id } => Some(*midi_clip_id),
            _ => None,
        }
    }

    /// The clip's own content, ignoring takes.
    ///
    /// Callers that are handing content to the backend want [`ClipInstance::resolve`] instead — an
    /// instance with takes overrides the clip's content with whichever take is active.
    pub fn resolve(&self) -> ResolvedContent {
        match &self.clip_type {
            AudioClipType::Sampled { audio_pool_index } => ResolvedContent::Audio {
                audio_pool_index: *audio_pool_index,
            },
            AudioClipType::Midi { midi_clip_id } => ResolvedContent::Midi {
                midi_clip_id: *midi_clip_id,
            },
            AudioClipType::Recording => ResolvedContent::Recording,
        }
    }

    /// Resolve a content time against this clip's domain.
    ///
    /// This is the ONLY sanctioned way to turn a [`ContentTime`] into a real duration — the type has
    /// no `.to_seconds()` of its own precisely so that the clip, which is the one thing that knows
    /// whether its content is measured in seconds or beats, has to be consulted.
    pub fn resolve_content_time(&self, t: ContentTime) -> ClipDuration {
        if self.is_midi_domain() {
            ClipDuration::Beats(Beats(t.raw()))
        } else {
            ClipDuration::Seconds(Seconds(t.raw()))
        }
    }

    /// Tag a pair of trim bounds with this clip's content domain, ready for the backend.
    ///
    /// Building the [`TrimRange`] from the clip means a caller can't reach for the wrong variant:
    /// the clip is the one thing that knows the domain.
    pub fn trim_range(&self, start: ContentTime, end: ContentTime) -> daw_backend::command::TrimRange {
        if self.is_midi_domain() {
            daw_backend::command::TrimRange::Beats {
                start: Beats(start.raw()),
                end: Beats(end.raw()),
            }
        } else {
            daw_backend::command::TrimRange::Seconds {
                start: Seconds(start.raw()),
                end: Seconds(end.raw()),
            }
        }
    }

    /// Whether this clip's own content is the given audio pool index.
    pub fn owns_audio_pool_index(&self, pool_index: usize) -> bool {
        self.audio_pool_index() == Some(pool_index)
    }

    /// Whether this clip's own content is the given backend MIDI clip ID.
    pub fn owns_midi_clip_id(&self, id: u32) -> bool {
        self.midi_clip_id() == Some(id)
    }
}

/// What a clip instance actually plays, once takes are resolved to the active one.
/// Produced by [`ClipInstance::resolve`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedContent {
    Audio { audio_pool_index: usize },
    Midi { midi_clip_id: u32 },
    /// A recording in progress (or an empty take folder) — no backend content yet.
    Recording,
}

/// Unified clip enum for polymorphic handling
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnyClip {
    Vector(VectorClip),
    Video(VideoClip),
    Audio(AudioClip),
}

impl AnyClip {
    /// Get the clip ID
    pub fn id(&self) -> Uuid {
        match self {
            AnyClip::Vector(c) => c.id,
            AnyClip::Audio(c) => c.id,
            AnyClip::Video(c) => c.id,
        }
    }

    /// Get the clip name
    pub fn name(&self) -> &str {
        match self {
            AnyClip::Vector(c) => &c.name,
            AnyClip::Audio(c) => &c.name,
            AnyClip::Video(c) => &c.name,
        }
    }

    /// Get the clip duration
    pub fn duration(&self) -> f64 {
        match self {
            AnyClip::Vector(c) => c.duration,
            AnyClip::Audio(c) => c.duration,
            AnyClip::Video(c) => c.duration,
        }
    }
}

/// Clip instance with transform, timing, and playback properties
///
/// References a clip and provides instance-specific properties:
/// - Transform (position, rotation, scale)
/// - Timeline placement (when this instance appears on the parent layer's timeline)
/// - Trimming (trim_start, trim_end within the clip's internal content)
/// - Playback speed (time remapping)
///
/// ## Coordinate systems
/// - `timeline_start` / `timeline_duration` are in **beats** (quarter-note beats).
///   Use [`crate::tempo_map::TempoMap::beats_to_seconds`] to convert to seconds.
/// - `trim_start` / `trim_end` are in **seconds** (audio/video file seek offsets;
///   not affected by BPM changes).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipInstance {
    /// Unique identifier for this instance
    pub id: Uuid,

    /// The clip this instance references
    pub clip_id: Uuid,

    /// Transform (position, rotation, scale, skew)
    pub transform: Transform,

    /// Opacity (0.0 to 1.0)
    pub opacity: f64,

    /// Optional name for this instance
    pub name: Option<String>,

    /// When this instance starts on the timeline, in **beats**.
    /// Default: 0.0
    pub timeline_start: Beats,

    /// How long this instance appears on the timeline, in **beats**.
    /// If set and longer than the trimmed content, the content will loop.
    /// Default: None (use trimmed clip duration, no looping)
    pub timeline_duration: Option<Beats>,

    /// Trim start: offset into the clip's internal content.
    ///
    /// A [`ContentTime`] — measured in the CLIP's content domain, which is seconds for sampled
    /// audio/video/vector but BEATS for MIDI. Resolve it against the clip
    /// ([`Document::resolve_content_time`]) before combining it with anything on the timeline.
    /// Default: 0.0
    pub trim_start: ContentTime,

    /// Trim end: offset into the clip's internal content. See [`Self::trim_start`].
    /// Default: None (use full clip duration)
    pub trim_end: Option<ContentTime>,

    /// Playback speed multiplier
    /// 1.0 = normal speed, 0.5 = half speed, 2.0 = double speed
    /// Default: 1.0
    pub playback_speed: f64,

    /// Clip-level gain/volume (for audio clips)
    /// Compatible with daw-backend's Clip.gain
    /// Default: 1.0
    pub gain: f32,

    /// How far (in beats) the looped content extends before timeline_start.
    /// When set, loop iterations are drawn/played before the content start.
    /// Default: None (no pre-loop)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_before: Option<Beats>,

    /// Alternate takes from cycle recording. Empty = an ordinary instance with no takes.
    ///
    /// The takes live on the INSTANCE, not the clip, so managing them is per-instance: deleting or
    /// renaming a take on one instance leaves every other instance alone. Splitting clones the list
    /// along with the rest of the instance, so the two halves get independent take lists — and,
    /// since they can each select a different take, comping still falls out for free.
    ///
    /// Every take spans the full cycle region (partial passes are padded with silence at capture
    /// time), so they're all the same length as the clip's own content. That uniformity is what lets
    /// a take switch leave the instance's geometry untouched.
    ///
    /// When non-empty, these OVERRIDE the clip's own content — see [`Self::resolve`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<AudioTake>,

    /// Which of [`Self::takes`] plays. `None` (or a stale index) means take 0.
    /// Default: None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_take: Option<usize>,

    /// The cycle region's length in beats when these takes were recorded.
    ///
    /// Audio takes are cut geometrically (by sample count), so they're only meaningful against the
    /// tempo they were recorded at. Keeping the recorded length lets a future time-stretch/conform
    /// feature reconcile them if the tempo moves underneath, and lets a new recording tell whether
    /// it belongs in this take list or a fresh one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_loop_beats: Option<Beats>,
}

/// High 64-bit sentinel used to identify UUIDs that encode a backend audio clip instance ID.
/// Using a sentinel that would never appear in a v4 random UUID (which has specific version bits).
const AUDIO_BACKEND_UUID_HIGH: u64 = 0xDEAD_BEEF_CAFE_BABE;

/// Convert a backend `AudioClipInstanceId` (u32) to a synthetic UUID for use in selection/hit-testing.
/// These UUIDs are distinct from real document UUIDs and can be round-tripped via `audio_backend_id_from_uuid`.
pub fn audio_backend_uuid(backend_id: u32) -> Uuid {
    Uuid::from_u64_pair(AUDIO_BACKEND_UUID_HIGH, backend_id as u64)
}

/// Extract a backend `AudioClipInstanceId` from a synthetic UUID created by `audio_backend_uuid`.
/// Returns `None` if this is a regular document UUID.
pub fn audio_backend_id_from_uuid(uuid: Uuid) -> Option<u32> {
    let (high, low) = uuid.as_u64_pair();
    if high == AUDIO_BACKEND_UUID_HIGH {
        Some(low as u32)
    } else {
        None
    }
}

/// High 64-bit sentinel used to identify UUIDs that encode a backend MIDI clip instance ID.
const MIDI_BACKEND_UUID_HIGH: u64 = 0xDEAD_BEEF_CAFE_BEEF;

/// Convert a backend `MidiClipInstanceId` (u32) to a synthetic UUID for use in selection/hit-testing.
pub fn midi_backend_uuid(backend_id: u32) -> Uuid {
    Uuid::from_u64_pair(MIDI_BACKEND_UUID_HIGH, backend_id as u64)
}

/// Extract a backend `MidiClipInstanceId` from a synthetic UUID created by `midi_backend_uuid`.
/// Returns `None` if this is a regular document UUID.
pub fn midi_backend_id_from_uuid(uuid: Uuid) -> Option<u32> {
    let (high, low) = uuid.as_u64_pair();
    if high == MIDI_BACKEND_UUID_HIGH {
        Some(low as u32)
    } else {
        None
    }
}

impl ClipInstance {
    /// Create a new clip instance
    pub fn new(clip_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            clip_id,
            transform: Transform::default(),
            opacity: 1.0,
            name: None,
            timeline_start: Beats::ZERO,
            timeline_duration: None,
            trim_start: ContentTime::ZERO,
            trim_end: None,
            playback_speed: 1.0,
            gain: 1.0,
            loop_before: None,
            takes: Vec::new(),
            active_take: None,
            recorded_loop_beats: None,
        }
    }

    /// Create with a specific ID
    pub fn with_id(id: Uuid, clip_id: Uuid) -> Self {
        Self {
            id,
            clip_id,
            transform: Transform::default(),
            opacity: 1.0,
            name: None,
            timeline_start: Beats::ZERO,
            timeline_duration: None,
            trim_start: ContentTime::ZERO,
            trim_end: None,
            playback_speed: 1.0,
            gain: 1.0,
            loop_before: None,
            takes: Vec::new(),
            active_take: None,
            recorded_loop_beats: None,
        }
    }

    /// Set the transform
    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// Set the position
    pub fn with_position(mut self, x: f64, y: f64) -> Self {
        self.transform.x = x;
        self.transform.y = y;
        self
    }

    /// Set the opacity
    pub fn with_opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    /// Set the name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set timeline position (beats).
    pub fn with_timeline_start(mut self, timeline_start: Beats) -> Self {
        self.timeline_start = timeline_start;
        self
    }

    /// Set trimming (start and end time within the clip's internal content)
    pub fn with_trimming(mut self, trim_start: ContentTime, trim_end: Option<ContentTime>) -> Self {
        self.trim_start = trim_start;
        self.trim_end = trim_end;
        self
    }

    /// Set playback speed
    pub fn with_playback_speed(mut self, speed: f64) -> Self {
        self.playback_speed = speed;
        self
    }

    /// Set gain/volume (for audio)
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Set explicit timeline duration (in beats) by directly setting `timeline_duration`.
    pub fn with_timeline_duration(mut self, duration_beats: Beats) -> Self {
        self.timeline_duration = Some(duration_beats);
        self
    }

    /// The take this instance plays, if it has any.
    ///
    /// A `None`/stale `active_take` falls back to take 0 — an index can go stale (an undo that
    /// shrank the list, an old `.beam`), and silently playing the first take beats playing nothing.
    pub fn active_take(&self) -> Option<&AudioTake> {
        self.takes
            .get(self.active_take.unwrap_or(0))
            .or_else(|| self.takes.first())
    }

    /// The index [`Self::active_take`] actually resolves to, clamped into range.
    pub fn active_take_index(&self) -> usize {
        let i = self.active_take.unwrap_or(0);
        if i < self.takes.len() { i } else { 0 }
    }

    /// What this instance plays: its active take if it has takes, otherwise the clip's own content.
    ///
    /// This is the sanctioned way to ask "what content do I hand the backend for this instance?".
    /// Takes are never a distinct *case* at the call site — an instance with takes is just an audio
    /// or MIDI instance whose identity depends on which take is live.
    pub fn resolve(&self, clip: &AudioClip) -> ResolvedContent {
        match self.active_take().map(|t| &t.content) {
            Some(TakeContent::Audio { audio_pool_index }) => ResolvedContent::Audio {
                audio_pool_index: *audio_pool_index,
            },
            Some(TakeContent::Midi { midi_clip_id }) => ResolvedContent::Midi {
                midi_clip_id: *midi_clip_id,
            },
            None => clip.resolve(),
        }
    }

    /// The audio pool index this instance plays. See [`Self::resolve`].
    pub fn resolved_audio_pool_index(&self, clip: &AudioClip) -> Option<usize> {
        match self.resolve(clip) {
            ResolvedContent::Audio { audio_pool_index } => Some(audio_pool_index),
            _ => None,
        }
    }

    /// The backend MIDI clip ID this instance plays. See [`Self::resolve`].
    pub fn resolved_midi_clip_id(&self, clip: &AudioClip) -> Option<u32> {
        match self.resolve(clip) {
            ResolvedContent::Midi { midi_clip_id } => Some(midi_clip_id),
            _ => None,
        }
    }

    /// Content window (`trim_end - trim_start`) in the clip's own content domain.
    /// Used for internal looping calculations.
    pub fn content_window(&self, clip_content: ClipDuration) -> ClipDuration {
        let end = self.trim_end.map_or(clip_content.native(), |t| t.raw());
        let window = (end - self.trim_start.raw()).max(0.0);
        match clip_content {
            ClipDuration::Beats(_) => ClipDuration::Beats(Beats(window)),
            ClipDuration::Seconds(_) => ClipDuration::Seconds(Seconds(window)),
        }
    }

    /// How long this instance appears on the timeline, in **beats**.
    ///
    /// If `timeline_duration` is set, returns that (enabling content looping). Otherwise the clip
    /// occupies its content window — converted to beats *in the clip's own domain*:
    ///
    /// - MIDI content is already beats and is tempo-invariant, so it carries over directly.
    /// - Wall-clock content (audio/video/vector) is a seconds span, so it converts at the clip's
    ///   position on the timeline.
    ///
    /// Taking a `ClipDuration` rather than a bare `Seconds` is what keeps those apart: this used to
    /// take seconds and subtract `trim_start` from it, which for a TRIMMED MIDI clip subtracted a
    /// beats offset from a seconds duration and got the clip's length wrong.
    pub fn effective_duration_beats(&self, clip_content: ClipDuration, tempo_map: &crate::tempo_map::TempoMap) -> Beats {
        if let Some(td) = self.timeline_duration {
            return td;
        }
        match self.content_window(clip_content) {
            ClipDuration::Beats(b) => b,
            ClipDuration::Seconds(s) => {
                let start_secs = tempo_map.beats_to_seconds(self.timeline_start);
                tempo_map.seconds_to_beats(start_secs + s) - self.timeline_start
            }
        }
    }

    /// Left edge of the clip's visual extent on the timeline, in **beats**.
    pub fn effective_start(&self) -> Beats {
        self.timeline_start - self.loop_before.unwrap_or(Beats::ZERO)
    }

    /// Total visual duration (loop_before + effective_duration), in **beats**.
    pub fn total_duration(&self, clip_content: ClipDuration, tempo_map: &crate::tempo_map::TempoMap) -> Beats {
        self.loop_before.unwrap_or(Beats::ZERO) + self.effective_duration_beats(clip_content, tempo_map)
    }

    /// Map a playback time (in **seconds**) to clip-local content time (in **seconds**).
    ///
    /// The trim bounds are resolved through `clip_content`'s domain first, so a MIDI clip's beats
    /// trims are converted rather than read as seconds. Callers are the wall-clock consumers (video
    /// seek, vector/raster rendering), which want seconds regardless of how the clip stores content.
    ///
    /// Returns `None` if the clip instance is not active at `time`.
    pub fn remap_time_secs(&self, time: Seconds, clip_content: ClipDuration, tempo_map: &crate::tempo_map::TempoMap) -> Option<Seconds> {
        let start_secs = tempo_map.beats_to_seconds(self.timeline_start);
        let dur_beats = self.effective_duration_beats(clip_content, tempo_map);
        let end_secs = tempo_map.beats_to_seconds(self.timeline_start + dur_beats);

        if time < start_secs || time >= end_secs {
            return None;
        }

        let trim_start_secs = clip_content.same_domain(self.trim_start).to_seconds(tempo_map);
        let content_time = (time - start_secs) * self.playback_speed;
        let content_window = self.content_window(clip_content).to_seconds(tempo_map);

        if content_window == Seconds::ZERO {
            return Some(trim_start_secs);
        }

        let looped = if content_time > content_window {
            content_time % content_window
        } else {
            content_time
        };

        Some(trim_start_secs + looped)
    }

    /// Alias for `remap_time_secs`.
    #[inline]
    pub fn remap_time(&self, time: Seconds, clip_content: ClipDuration, tempo_map: &crate::tempo_map::TempoMap) -> Option<Seconds> {
        self.remap_time_secs(time, clip_content, tempo_map)
    }

    /// Alias for `effective_duration_beats`.
    #[inline]
    pub fn effective_duration(&self, clip_content: ClipDuration, tempo_map: &crate::tempo_map::TempoMap) -> Beats {
        self.effective_duration_beats(clip_content, tempo_map)
    }

    /// Convert to affine transform
    pub fn to_affine(&self) -> vello::kurbo::Affine {
        self.transform.to_affine()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_clip_creation() {
        let clip = VectorClip::new("My Composition", 1920.0, 1080.0, 10.0);
        assert_eq!(clip.name, "My Composition");
        assert_eq!(clip.width, 1920.0);
        assert_eq!(clip.height, 1080.0);
        assert_eq!(clip.duration, 10.0);
    }

    #[test]
    fn test_video_clip_creation() {
        let clip = VideoClip::new("My Video", "/path/to/video.mp4", 1920.0, 1080.0, 30.0, 24.0);
        assert_eq!(clip.name, "My Video");
        assert_eq!(clip.file_path, "/path/to/video.mp4");
        assert_eq!(clip.duration, 30.0);
        assert_eq!(clip.frame_rate, 24.0);
    }

    #[test]
    fn test_audio_clip_sampled() {
        let clip = AudioClip::new_sampled("Background Music", 0, 180.0);
        assert_eq!(clip.name, "Background Music");
        assert_eq!(clip.duration, 180.0);
        assert_eq!(clip.audio_pool_index(), Some(0));
    }

    #[test]
    fn test_audio_clip_midi() {
        let clip = AudioClip::new_midi("Piano Melody", 1, daw_backend::Beats(60.0));
        assert_eq!(clip.name, "Piano Melody");
        assert_eq!(clip.duration, 60.0);
        match &clip.clip_type {
            AudioClipType::Midi { midi_clip_id } => assert_eq!(*midi_clip_id, 1),
            _ => panic!("Expected Midi clip type"),
        }
    }

    #[test]
    fn test_midi_event_creation() {
        let event = MidiEvent::note_on(1.5, 0, 60, 100);
        assert_eq!(event.timestamp, 1.5);
        assert_eq!(event.status, 0x90); // Note on, channel 0
        assert_eq!(event.data1, 60); // Middle C
        assert_eq!(event.data2, 100); // Velocity
    }

    #[test]
    fn test_any_clip_enum() {
        let vector_clip = VectorClip::new("Comp", 1920.0, 1080.0, 10.0);
        let any_clip = AnyClip::Vector(vector_clip.clone());

        assert_eq!(any_clip.id(), vector_clip.id);
        assert_eq!(any_clip.name(), "Comp");
        assert_eq!(any_clip.duration(), 10.0);
    }

    #[test]
    fn test_clip_instance_creation() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id);

        assert_eq!(instance.clip_id, clip_id);
        assert_eq!(instance.opacity, 1.0);
        assert_eq!(instance.timeline_start, Beats::ZERO);
        assert_eq!(instance.trim_start, ContentTime::ZERO);
        assert_eq!(instance.trim_end, None);
        assert_eq!(instance.playback_speed, 1.0);
        assert_eq!(instance.gain, 1.0);
    }

    #[test]
    fn test_clip_instance_trimming() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_trimming(ContentTime(2.0), Some(ContentTime(8.0)));

        assert_eq!(instance.trim_start, ContentTime(2.0));
        assert_eq!(instance.trim_end, Some(ContentTime(8.0)));
        // At 60 BPM the tempo map is identity (1 beat == 1 second), so the
        // beats-domain effective duration equals the seconds content window.
        let tempo_map = crate::tempo_map::TempoMap::constant(60.0);
        let content = ClipDuration::Seconds(Seconds(10.0));
        assert_eq!(instance.effective_duration(content, &tempo_map), Beats(6.0));
    }

    #[test]
    fn test_clip_instance_no_end_trim() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_trimming(ContentTime(2.0), None);

        assert_eq!(instance.trim_start, ContentTime(2.0));
        assert_eq!(instance.trim_end, None);
        // At 60 BPM the tempo map is identity (1 beat == 1 second).
        let tempo_map = crate::tempo_map::TempoMap::constant(60.0);
        let content = ClipDuration::Seconds(Seconds(10.0));
        assert_eq!(instance.effective_duration(content, &tempo_map), Beats(8.0));
    }

    #[test]
    fn trimmed_midi_clip_keeps_its_beats_length_across_tempo() {
        // Regression: `effective_duration_beats` used to take a SECONDS clip duration and subtract
        // `trim_start` from it. For a TRIMMED MIDI clip that subtracted a beats offset from a
        // seconds duration, so the clip's timeline length came out wrong at any tempo but 60 BPM.
        //
        // MIDI content is beats and tempo-invariant: a clip trimmed to beats 2..6 is 4 beats long
        // whatever the tempo says.
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_trimming(ContentTime(2.0), Some(ContentTime(6.0)));
        let content = ClipDuration::Beats(Beats(8.0));

        for bpm in [60.0, 120.0, 90.0] {
            let tempo_map = crate::tempo_map::TempoMap::constant(bpm);
            assert_eq!(
                instance.effective_duration(content, &tempo_map),
                Beats(4.0),
                "a MIDI clip trimmed to beats 2..6 is 4 beats long at {bpm} BPM",
            );
        }
    }

    #[test]
    fn test_clip_instance_builder() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_position(100.0, 200.0)
            .with_opacity(0.5)
            .with_name("My Instance")
            .with_playback_speed(2.0)
            .with_gain(0.8);

        assert_eq!(instance.transform.x, 100.0);
        assert_eq!(instance.transform.y, 200.0);
        assert_eq!(instance.opacity, 0.5);
        assert_eq!(instance.name, Some("My Instance".to_string()));
        assert_eq!(instance.playback_speed, 2.0);
        assert_eq!(instance.gain, 0.8);
    }

    /// A clip whose own content is pool 10, plus an instance carrying takes over the given pools.
    fn with_takes(pool_indices: &[usize]) -> (AudioClip, ClipInstance) {
        let clip = AudioClip::new_sampled("Cycle rec", pool_indices[0], 2.0);
        let mut instance = ClipInstance::new(clip.id);
        instance.takes = pool_indices
            .iter()
            .enumerate()
            .map(|(i, &audio_pool_index)| AudioTake {
                name: format!("Take {}", i + 1),
                content: TakeContent::Audio { audio_pool_index },
            })
            .collect();
        instance.recorded_loop_beats = Some(Beats(8.0));
        (clip, instance)
    }

    #[test]
    fn active_take_selects_the_pool_file() {
        let (clip, mut instance) = with_takes(&[10, 11, 12]);
        instance.active_take = Some(0);
        assert_eq!(instance.resolved_audio_pool_index(&clip), Some(10));
        instance.active_take = Some(2);
        assert_eq!(instance.resolved_audio_pool_index(&clip), Some(12));
        // None means take 0.
        instance.active_take = None;
        assert_eq!(instance.resolved_audio_pool_index(&clip), Some(10));
    }

    #[test]
    fn out_of_range_take_falls_back_to_the_first() {
        // An index can go stale (an undo that shrank the list, an old .beam). Playing the first take
        // beats playing nothing.
        let (clip, mut instance) = with_takes(&[10, 11]);
        instance.active_take = Some(99);
        assert_eq!(instance.resolved_audio_pool_index(&clip), Some(10));
        assert_eq!(instance.active_take_index(), 0);
    }

    #[test]
    fn an_instance_without_takes_plays_the_clips_own_content() {
        let clip = AudioClip::new_sampled("Plain", 42, 2.0);
        let instance = ClipInstance::new(clip.id);
        assert!(instance.takes.is_empty());
        assert_eq!(instance.resolved_audio_pool_index(&clip), Some(42));
    }

    #[test]
    fn midi_takes_resolve_to_midi_content() {
        let clip = AudioClip::new_midi("Cycle rec", 7, Beats(4.0));
        let mut instance = ClipInstance::new(clip.id);
        instance.takes = vec![AudioTake {
            name: "Take 1".into(),
            content: TakeContent::Midi { midi_clip_id: 7 },
        }];
        assert_eq!(clip.content_duration(), ClipDuration::Beats(Beats(4.0)));
        assert_eq!(instance.resolved_midi_clip_id(&clip), Some(7));
        assert_eq!(instance.resolved_audio_pool_index(&clip), None);
    }

    #[test]
    fn splitting_gives_each_half_an_independent_take_list() {
        // Takes live on the INSTANCE, so a split (which clones the instance) hands each half its own
        // list. Two consequences, both wanted: the halves can select different takes (comping), and
        // deleting a take from one leaves the other alone.
        let (clip, left_src) = with_takes(&[10, 11, 12]);
        let mut left = left_src.clone();
        let mut right = left_src.clone();
        right.id = Uuid::new_v4();

        left.active_take = Some(0);
        right.active_take = Some(2);
        assert_eq!(left.resolved_audio_pool_index(&clip), Some(10));
        assert_eq!(right.resolved_audio_pool_index(&clip), Some(12));

        // Delete take 2 (pool 11) from the left half only.
        left.takes.remove(1);
        assert_eq!(left.takes.len(), 2);
        assert_eq!(right.takes.len(), 3, "the other half keeps its own takes");
        assert_eq!(
            right.resolved_audio_pool_index(&clip),
            Some(12),
            "and its selection still points where it did",
        );
    }

    #[test]
    fn clip_instance_without_active_take_deserializes() {
        // Back-compat: .beam files written before take folders have no `active_take` field.
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "clip_id": "550e8400-e29b-41d4-a716-446655440001",
            "transform": {"x": 0.0, "y": 0.0, "rotation": 0.0, "scale_x": 1.0, "scale_y": 1.0, "skew_x": 0.0, "skew_y": 0.0},
            "opacity": 1.0,
            "name": null,
            "timeline_start": 0.0,
            "timeline_duration": null,
            "trim_start": 0.0,
            "trim_end": null,
            "playback_speed": 1.0,
            "gain": 1.0
        }"#;
        let instance: ClipInstance = serde_json::from_str(json).expect("old instances must load");
        assert_eq!(instance.active_take, None);
    }
}

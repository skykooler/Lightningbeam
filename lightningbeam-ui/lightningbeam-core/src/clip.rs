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
use daw_backend::{Beats, Seconds};
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
                    let secs = (te - ci.trim_start).max(0.0);
                    tempo_map.seconds_to_beats(tempo_map.beats_to_seconds(ci.timeline_start) + Seconds(secs))
                } else if let Some(clip_dur_secs) = clip_duration_fn(&ci.clip_id) {
                    let secs = (clip_dur_secs - ci.trim_start).max(0.0);
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
                    let nested_clip_time = ((clip_time - start_secs) * clip_instance.playback_speed) + clip_instance.trim_start;

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
    /// A folder of alternate takes, produced by cycle recording.
    ///
    /// Each pass of the transport around the cycle region becomes one take. Every take spans the
    /// **full** cycle region (partial passes are padded with silence at capture time), so all takes
    /// are the same length and share this clip's `duration` — which in turn means switching takes
    /// never changes the clip's geometry, and splitting a take-folder instance yields two halves
    /// whose takes still line up. Which take actually sounds is per-*instance*
    /// ([`ClipInstance::active_take`]), not per-clip, so a split can play take 1 on the left and
    /// take 3 on the right. That's comping.
    TakeFolder {
        /// The takes, in the order they were recorded. Never empty in practice.
        takes: Vec<AudioTake>,
        /// The cycle region's length in beats at the time of recording.
        ///
        /// Audio takes are segmented geometrically (by sample count), so they're only meaningful
        /// against the tempo they were cut at. Keeping the recorded length lets a future
        /// time-stretch/conform feature reconcile the takes if the tempo changes underneath them.
        recorded_loop_beats: Beats,
    },
}

/// One take in a [`AudioClipType::TakeFolder`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioTake {
    /// Display name, e.g. "Take 1".
    pub name: String,
    /// The recorded content this take points at.
    pub content: TakeContent,
}

/// What a take actually holds. A folder's takes are all the same kind — one cycle-record session
/// captures either audio or MIDI, never a mix.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    ///
    /// A take folder inherits the domain of its takes, which are all the same kind — one
    /// cycle-record session captures either audio or MIDI, never a mix. An empty folder can't
    /// happen in practice; call it seconds so the fallback is the common case.
    fn is_midi_domain(&self) -> bool {
        match &self.clip_type {
            AudioClipType::Midi { .. } => true,
            AudioClipType::Sampled { .. } | AudioClipType::Recording => false,
            AudioClipType::TakeFolder { takes, .. } => {
                matches!(takes.first().map(|t| &t.content), Some(TakeContent::Midi { .. }))
            }
        }
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

    /// The clip's takes, if it's a take folder.
    pub fn takes(&self) -> Option<&[AudioTake]> {
        match &self.clip_type {
            AudioClipType::TakeFolder { takes, .. } => Some(takes),
            _ => None,
        }
    }

    /// The take an instance's `active_take` actually selects.
    ///
    /// `None` means take 0, which is also what an out-of-range index falls back to — an index can
    /// go stale (an old `.beam`, an undo that shrank the folder), and silently playing the first
    /// take beats refusing to play anything.
    fn take_for(&self, active_take: Option<usize>) -> Option<&AudioTake> {
        let takes = self.takes()?;
        takes
            .get(active_take.unwrap_or(0))
            .or_else(|| takes.first())
    }

    /// What this clip plays *for a given instance*, with take folders collapsed to the instance's
    /// active take.
    ///
    /// This is the sanctioned way to ask "what content do I hand the backend for this instance?".
    /// Matching on `clip_type` directly will see a `TakeFolder` and have to handle it separately;
    /// matching on this won't, because a folder is never a distinct case here — it's just an audio
    /// or MIDI clip whose identity depends on which take is active.
    pub fn resolve(&self, active_take: Option<usize>) -> ResolvedContent {
        match &self.clip_type {
            AudioClipType::Sampled { audio_pool_index } => ResolvedContent::Audio {
                audio_pool_index: *audio_pool_index,
            },
            AudioClipType::Midi { midi_clip_id } => ResolvedContent::Midi {
                midi_clip_id: *midi_clip_id,
            },
            AudioClipType::Recording => ResolvedContent::Recording,
            AudioClipType::TakeFolder { .. } => match self.take_for(active_take).map(|t| &t.content) {
                Some(TakeContent::Audio { audio_pool_index }) => ResolvedContent::Audio {
                    audio_pool_index: *audio_pool_index,
                },
                Some(TakeContent::Midi { midi_clip_id }) => ResolvedContent::Midi {
                    midi_clip_id: *midi_clip_id,
                },
                // An empty folder has nothing to play. Treat it like a recording placeholder:
                // the backend gets nothing, rather than a bogus pool index.
                None => ResolvedContent::Recording,
            },
        }
    }

    /// Tag a pair of raw trim bounds with this clip's content domain, ready for the backend.
    ///
    /// `ClipInstance::trim_start`/`trim_end` are bare `f64`s whose unit depends on the clip —
    /// SECONDS for sampled audio, BEATS for MIDI. Building the [`TrimRange`] from the clip means a
    /// caller can't reach for the wrong variant: the clip is the one thing that knows.
    pub fn trim_range(&self, start: f64, end: f64) -> daw_backend::command::TrimRange {
        if self.is_midi_domain() {
            daw_backend::command::TrimRange::Beats {
                start: Beats(start),
                end: Beats(end),
            }
        } else {
            daw_backend::command::TrimRange::Seconds {
                start: Seconds(start),
                end: Seconds(end),
            }
        }
    }

    /// Whether this clip owns the given audio pool index — either as a plain sampled clip, or as
    /// *any* take of a take folder. Reverse lookups (backend resource → document clip) must use
    /// this: a folder owns one pool file per take, not just the active one.
    pub fn owns_audio_pool_index(&self, pool_index: usize) -> bool {
        match &self.clip_type {
            AudioClipType::Sampled { audio_pool_index } => *audio_pool_index == pool_index,
            AudioClipType::TakeFolder { takes, .. } => takes.iter().any(|t| {
                matches!(t.content, TakeContent::Audio { audio_pool_index } if audio_pool_index == pool_index)
            }),
            _ => false,
        }
    }

    /// Whether this clip owns the given backend MIDI clip ID. See [`Self::owns_audio_pool_index`].
    pub fn owns_midi_clip_id(&self, id: u32) -> bool {
        match &self.clip_type {
            AudioClipType::Midi { midi_clip_id } => *midi_clip_id == id,
            AudioClipType::TakeFolder { takes, .. } => takes.iter().any(|t| {
                matches!(t.content, TakeContent::Midi { midi_clip_id } if midi_clip_id == id)
            }),
            _ => false,
        }
    }

    /// The audio pool index this *instance* should play. See [`Self::resolve`].
    pub fn resolved_audio_pool_index(&self, active_take: Option<usize>) -> Option<usize> {
        match self.resolve(active_take) {
            ResolvedContent::Audio { audio_pool_index } => Some(audio_pool_index),
            _ => None,
        }
    }

    /// The backend MIDI clip ID this *instance* should play. See [`Self::resolve`].
    pub fn resolved_midi_clip_id(&self, active_take: Option<usize>) -> Option<u32> {
        match self.resolve(active_take) {
            ResolvedContent::Midi { midi_clip_id } => Some(midi_clip_id),
            _ => None,
        }
    }
}

/// What a clip instance actually plays, once take folders are resolved to their active take.
/// Produced by [`AudioClip::resolve`].
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

    /// Trim start: offset into the clip's internal content, in **seconds**.
    /// - For audio: byte-offset into the audio file
    /// - For video: seek position in the video file
    /// - For vector: time offset into the animation
    /// Default: 0.0
    pub trim_start: f64,

    /// Trim end: offset into the clip's internal content, in **seconds**.
    /// Default: None (use full clip duration)
    pub trim_end: Option<f64>,

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

    /// Which take of a [`AudioClipType::TakeFolder`] clip this instance plays.
    ///
    /// Per-instance rather than per-clip so two instances of the same folder — e.g. the two halves
    /// of a split — can play different takes. That's how comping works. `None` means take 0;
    /// meaningless (and ignored) on non-folder clips.
    /// Default: None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_take: Option<usize>,
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
            trim_start: 0.0,
            trim_end: None,
            playback_speed: 1.0,
            gain: 1.0,
            loop_before: None,
            active_take: None,
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
            trim_start: 0.0,
            trim_end: None,
            playback_speed: 1.0,
            gain: 1.0,
            loop_before: None,
            active_take: None,
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
    pub fn with_trimming(mut self, trim_start: f64, trim_end: Option<f64>) -> Self {
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

    /// Content window size in seconds: `trim_end - trim_start`.
    /// Used for internal looping calculations.
    pub fn content_window_secs(&self, clip_duration_secs: Seconds) -> Seconds {
        let end = self.trim_end.unwrap_or(clip_duration_secs.seconds_to_f64());
        Seconds((end - self.trim_start).max(0.0))
    }

    /// How long this instance appears on the timeline, in **beats**.
    ///
    /// If `timeline_duration` is set, returns that (enabling content looping).
    /// Otherwise converts the content window from seconds to beats using the tempo map.
    pub fn effective_duration_beats(&self, clip_duration_secs: Seconds, tempo_map: &crate::tempo_map::TempoMap) -> Beats {
        if let Some(td) = self.timeline_duration {
            return td;
        }
        let window = self.content_window_secs(clip_duration_secs);
        let start_secs = tempo_map.beats_to_seconds(self.timeline_start);
        tempo_map.seconds_to_beats(start_secs + window) - self.timeline_start
    }

    /// Left edge of the clip's visual extent on the timeline, in **beats**.
    pub fn effective_start(&self) -> Beats {
        self.timeline_start - self.loop_before.unwrap_or(Beats::ZERO)
    }

    /// Total visual duration (loop_before + effective_duration), in **beats**.
    pub fn total_duration(&self, clip_duration_secs: Seconds, tempo_map: &crate::tempo_map::TempoMap) -> Beats {
        self.loop_before.unwrap_or(Beats::ZERO) + self.effective_duration_beats(clip_duration_secs, tempo_map)
    }

    /// Map a playback time (in **seconds**) to clip-local content time (in **seconds**).
    ///
    /// Returns `None` if the clip instance is not active at `time_secs`.
    pub fn remap_time_secs(&self, time: Seconds, clip_duration_secs: Seconds, tempo_map: &crate::tempo_map::TempoMap) -> Option<Seconds> {
        let start_secs = tempo_map.beats_to_seconds(self.timeline_start);
        let dur_beats = self.effective_duration_beats(clip_duration_secs, tempo_map);
        let end_secs = tempo_map.beats_to_seconds(self.timeline_start + dur_beats);

        if time < start_secs || time >= end_secs {
            return None;
        }

        let content_time = (time - start_secs) * self.playback_speed;
        let content_window = self.content_window_secs(clip_duration_secs);

        if content_window == Seconds::ZERO {
            return Some(Seconds(self.trim_start));
        }

        let looped = if content_time > content_window {
            content_time % content_window
        } else {
            content_time
        };

        Some(Seconds(self.trim_start) + looped)
    }

    /// Alias for `remap_time_secs`.
    #[inline]
    pub fn remap_time(&self, time: Seconds, clip_duration_secs: Seconds, tempo_map: &crate::tempo_map::TempoMap) -> Option<Seconds> {
        self.remap_time_secs(time, clip_duration_secs, tempo_map)
    }

    /// Alias for `effective_duration_beats`.
    #[inline]
    pub fn effective_duration(&self, clip_duration_secs: Seconds, tempo_map: &crate::tempo_map::TempoMap) -> Beats {
        self.effective_duration_beats(clip_duration_secs, tempo_map)
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
        assert_eq!(instance.trim_start, 0.0);
        assert_eq!(instance.trim_end, None);
        assert_eq!(instance.playback_speed, 1.0);
        assert_eq!(instance.gain, 1.0);
    }

    #[test]
    fn test_clip_instance_trimming() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_trimming(2.0, Some(8.0));

        assert_eq!(instance.trim_start, 2.0);
        assert_eq!(instance.trim_end, Some(8.0));
        // At 60 BPM the tempo map is identity (1 beat == 1 second), so the
        // beats-domain effective duration equals the seconds content window.
        let tempo_map = crate::tempo_map::TempoMap::constant(60.0);
        assert_eq!(instance.effective_duration(Seconds(10.0), &tempo_map), Beats(6.0));
    }

    #[test]
    fn test_clip_instance_no_end_trim() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_trimming(2.0, None);

        assert_eq!(instance.trim_start, 2.0);
        assert_eq!(instance.trim_end, None);
        // At 60 BPM the tempo map is identity (1 beat == 1 second).
        let tempo_map = crate::tempo_map::TempoMap::constant(60.0);
        assert_eq!(instance.effective_duration(Seconds(10.0), &tempo_map), Beats(8.0));
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

    /// Build a take folder of `n` audio takes with the given pool indices.
    fn take_folder(pool_indices: &[usize]) -> AudioClip {
        let mut clip = AudioClip::new_sampled("Cycle rec", 0, 2.0);
        clip.clip_type = AudioClipType::TakeFolder {
            takes: pool_indices
                .iter()
                .enumerate()
                .map(|(i, &audio_pool_index)| AudioTake {
                    name: format!("Take {}", i + 1),
                    content: TakeContent::Audio { audio_pool_index },
                })
                .collect(),
            recorded_loop_beats: Beats(8.0),
        };
        clip
    }

    #[test]
    fn active_take_selects_the_pool_file() {
        let clip = take_folder(&[10, 11, 12]);
        assert_eq!(clip.resolved_audio_pool_index(Some(0)), Some(10));
        assert_eq!(clip.resolved_audio_pool_index(Some(2)), Some(12));
        // None means take 0.
        assert_eq!(clip.resolved_audio_pool_index(None), Some(10));
    }

    #[test]
    fn out_of_range_take_falls_back_to_the_first() {
        // An index can go stale (an old .beam, an undo that shrank the folder). Playing the first
        // take beats playing nothing.
        let clip = take_folder(&[10, 11]);
        assert_eq!(clip.resolved_audio_pool_index(Some(99)), Some(10));
    }

    #[test]
    fn take_folder_owns_every_takes_pool_file() {
        // Reverse lookups (backend resource -> document clip) must find the folder via ANY take,
        // not just the active one.
        let clip = take_folder(&[10, 11, 12]);
        assert!(clip.owns_audio_pool_index(10));
        assert!(clip.owns_audio_pool_index(12));
        assert!(!clip.owns_audio_pool_index(13));
    }

    #[test]
    fn midi_take_folder_measures_duration_in_beats() {
        // A folder inherits its takes' domain: MIDI takes mean the duration is beats, not seconds.
        let mut clip = AudioClip::new_sampled("Cycle rec", 0, 4.0);
        clip.clip_type = AudioClipType::TakeFolder {
            takes: vec![AudioTake {
                name: "Take 1".into(),
                content: TakeContent::Midi { midi_clip_id: 7 },
            }],
            recorded_loop_beats: Beats(4.0),
        };
        assert_eq!(clip.content_duration(), ClipDuration::Beats(Beats(4.0)));
        assert_eq!(clip.resolved_midi_clip_id(Some(0)), Some(7));
        assert_eq!(clip.resolved_audio_pool_index(Some(0)), None);
    }

    #[test]
    fn takes_are_per_instance_so_a_split_can_comp() {
        // The whole point of putting active_take on the instance: two instances of the same folder
        // (which is what a split produces) can play different takes.
        let clip = take_folder(&[10, 11, 12]);
        let mut left = ClipInstance::new(clip.id);
        let mut right = left.clone();
        right.id = Uuid::new_v4();
        left.active_take = Some(0);
        right.active_take = Some(2);

        assert_eq!(clip.resolved_audio_pool_index(left.active_take), Some(10));
        assert_eq!(clip.resolved_audio_pool_index(right.active_take), Some(12));
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

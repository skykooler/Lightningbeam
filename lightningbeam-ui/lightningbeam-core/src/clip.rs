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
    pub fn content_duration(&self, framerate: f64) -> f64 {
        self.content_duration_with(framerate, |_| None)
    }

    /// Like `content_duration`, but with a closure that resolves clip durations
    /// for audio/video/effect clip instances inside this movie clip.
    pub fn content_duration_with(&self, framerate: f64, clip_duration_fn: impl Fn(&Uuid) -> Option<f64>) -> f64 {
        let frame_duration = 1.0 / framerate;
        let mut last_time: Option<f64> = None;

        for layer_node in self.layers.iter() {
            // Check clip instances on ALL layer types (vector, audio, video, effect)
            let clip_instances: &[ClipInstance] = match &layer_node.data {
                AnyLayer::Vector(vl) => &vl.clip_instances,
                AnyLayer::Audio(al) => &al.clip_instances,
                AnyLayer::Video(vl) => &vl.clip_instances,
                AnyLayer::Effect(el) => &el.clip_instances,
                AnyLayer::Group(_) => &[],
                AnyLayer::Raster(_) => &[],
            };
            for ci in clip_instances {
                let end = if let Some(td) = ci.timeline_duration {
                    ci.timeline_start + td
                } else if let Some(te) = ci.trim_end {
                    ci.timeline_start + (te - ci.trim_start).max(0.0)
                } else if let Some(clip_dur) = clip_duration_fn(&ci.clip_id) {
                    ci.timeline_start + (clip_dur - ci.trim_start).max(0.0)
                } else {
                    continue;
                };
                last_time = Some(match last_time {
                    Some(t) => t.max(end),
                    None => end,
                });
            }

            // Also check vector layer keyframes
            if let AnyLayer::Vector(vector_layer) = &layer_node.data {
                if let Some(last_kf) = vector_layer.keyframes.last() {
                    last_time = Some(match last_time {
                        Some(t) => t.max(last_kf.time),
                        None => last_kf.time,
                    });
                }
            }
        }

        match last_time {
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
                    // Convert parent clip time to nested clip local time
                    // Apply timeline offset and playback speed, then add trim offset
                    let nested_clip_time = ((clip_time - clip_instance.timeline_start) * clip_instance.playback_speed) + clip_instance.trim_start;

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

    /// Embedded image data (for project portability)
    /// If None, the image will be loaded from path when needed
    #[serde(skip_serializing_if = "Option::is_none")]
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

    /// Duration in seconds
    /// For sampled audio, this can be set to trim the audio shorter than the source file
    pub duration: f64,

    /// Audio clip type (sampled or MIDI)
    pub clip_type: AudioClipType,

    /// Folder this clip belongs to (None = root of category)
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

impl AudioClip {
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
        duration: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            duration,
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

    /// When this instance starts on the timeline (in seconds, relative to parent layer)
    /// This is the external positioning - where the instance appears on the timeline
    /// Default: 0.0 (start at beginning of layer)
    pub timeline_start: f64,

    /// How long this instance appears on the timeline (in seconds)
    /// If timeline_duration > (trim_end - trim_start), the trimmed content will loop
    /// Default: None (use trimmed clip duration, no looping)
    pub timeline_duration: Option<f64>,

    /// Trim start: offset into the clip's internal content (in seconds)
    /// Allows trimming the beginning of the clip
    /// - For audio: offset into the audio file
    /// - For video: offset into the video file
    /// - For vector: offset into the animation timeline
    /// Default: 0.0 (start at beginning of clip)
    pub trim_start: f64,

    /// Trim end: offset into the clip's internal content (in seconds)
    /// Allows trimming the end of the clip
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

    /// How far (in seconds) the looped content extends before timeline_start.
    /// When set, loop iterations are drawn/played before the content start.
    /// Default: None (no pre-loop)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_before: Option<f64>,
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
            timeline_start: 0.0,
            timeline_duration: None,
            trim_start: 0.0,
            trim_end: None,
            playback_speed: 1.0,
            gain: 1.0,
            loop_before: None,
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
            timeline_start: 0.0,
            timeline_duration: None,
            trim_start: 0.0,
            trim_end: None,
            playback_speed: 1.0,
            gain: 1.0,
            loop_before: None,
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

    /// Set timeline position
    pub fn with_timeline_start(mut self, timeline_start: f64) -> Self {
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

    /// Set explicit timeline duration by setting trim_end
    ///
    /// For effect instances, this effectively sets the duration since
    /// effects have infinite internal duration (trim_start defaults to 0).
    pub fn with_timeline_duration(mut self, duration: f64) -> Self {
        self.trim_end = Some(self.trim_start + duration);
        self
    }

    /// Get the effective duration of this instance (accounting for trimming and looping)
    /// If timeline_duration is set, returns that (enabling content looping)
    /// Otherwise returns the trimmed content duration
    pub fn effective_duration(&self, clip_duration: f64) -> f64 {
        // If timeline_duration is explicitly set, use that (for looping)
        if let Some(timeline_dur) = self.timeline_duration {
            return timeline_dur;
        }

        // Otherwise, return the trimmed content duration
        let end = self.trim_end.unwrap_or(clip_duration);
        (end - self.trim_start).max(0.0)
    }

    /// Get the effective start position on the timeline, accounting for loop_before.
    /// This is the left edge of the clip's visual extent.
    pub fn effective_start(&self) -> f64 {
        self.timeline_start - self.loop_before.unwrap_or(0.0)
    }

    /// Get the total visual duration including both loop_before and effective_duration.
    pub fn total_duration(&self, clip_duration: f64) -> f64 {
        self.loop_before.unwrap_or(0.0) + self.effective_duration(clip_duration)
    }

    /// Remap timeline time to clip content time
    ///
    /// Takes a global timeline time and returns the corresponding time within this
    /// clip's content, accounting for:
    /// - Instance position (timeline_start)
    /// - Playback speed
    /// - Trimming (trim_start, trim_end)
    /// - Looping (if timeline_duration > content window)
    ///
    /// Returns None if the clip instance is not active at the given timeline time.
    pub fn remap_time(&self, timeline_time: f64, clip_duration: f64) -> Option<f64> {
        // Check if clip instance is active at this time
        let instance_end = self.timeline_start + self.effective_duration(clip_duration);
        if timeline_time < self.timeline_start || timeline_time >= instance_end {
            return None;
        }

        // Calculate relative time within the instance (0.0 = start of instance)
        let relative_time = timeline_time - self.timeline_start;

        // Account for playback speed
        let content_time = relative_time * self.playback_speed;

        // Get the content window size (the portion of clip we're sampling)
        let trim_end = self.trim_end.unwrap_or(clip_duration);
        let content_window = (trim_end - self.trim_start).max(0.0);

        // If content_window is zero, can't sample anything
        if content_window == 0.0 {
            return Some(self.trim_start);
        }

        // Apply looping if content exceeds the window
        let looped_time = if content_time > content_window {
            content_time % content_window
        } else {
            content_time
        };

        // Add trim_start offset to get final clip time
        Some(self.trim_start + looped_time)
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
        let clip = AudioClip::new_midi("Piano Melody", 1, 60.0);
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
        assert_eq!(instance.timeline_start, 0.0);
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
        assert_eq!(instance.effective_duration(10.0), 6.0);
    }

    #[test]
    fn test_clip_instance_no_end_trim() {
        let clip_id = Uuid::new_v4();
        let instance = ClipInstance::new(clip_id)
            .with_trimming(2.0, None);

        assert_eq!(instance.trim_start, 2.0);
        assert_eq!(instance.trim_end, None);
        assert_eq!(instance.effective_duration(10.0), 8.0);
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
}

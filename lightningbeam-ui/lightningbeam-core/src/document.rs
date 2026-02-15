//! Document structure for Lightningbeam
//!
//! The Document represents a complete animation project with settings
//! and a root graphics object containing the scene graph.

use crate::asset_folder::AssetFolderTree;
use crate::clip::{AudioClip, ClipInstance, ImageAsset, VideoClip, VectorClip};
use crate::effect::EffectDefinition;
use crate::layer::AnyLayer;
use crate::layout::LayoutNode;
use crate::shape::ShapeColor;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Root graphics object containing all layers in the scene
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphicsObject {
    /// Unique identifier
    pub id: Uuid,

    /// Name of this graphics object
    pub name: String,

    /// Child layers
    pub children: Vec<AnyLayer>,
}

impl GraphicsObject {
    /// Create a new graphics object
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            children: Vec::new(),
        }
    }

    /// Add a layer as a child
    pub fn add_child(&mut self, layer: AnyLayer) -> Uuid {
        let id = layer.id();
        self.children.push(layer);
        id
    }

    /// Get a child layer by ID
    pub fn get_child(&self, id: &Uuid) -> Option<&AnyLayer> {
        self.children.iter().find(|l| &l.id() == id)
    }

    /// Get a mutable child layer by ID
    pub fn get_child_mut(&mut self, id: &Uuid) -> Option<&mut AnyLayer> {
        self.children.iter_mut().find(|l| &l.id() == id)
    }

    /// Remove a child layer by ID
    pub fn remove_child(&mut self, id: &Uuid) -> Option<AnyLayer> {
        if let Some(index) = self.children.iter().position(|l| &l.id() == id) {
            Some(self.children.remove(index))
        } else {
            None
        }
    }
}

impl Default for GraphicsObject {
    fn default() -> Self {
        Self::new("Root")
    }
}

/// Asset category for folder tree access
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCategory {
    Vector,
    Video,
    Audio,
    Images,
    Effects,
}

/// Document settings and scene
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    /// Unique identifier for this document
    pub id: Uuid,

    /// Document name
    pub name: String,

    /// Background color
    pub background_color: ShapeColor,

    /// Canvas width in pixels
    pub width: f64,

    /// Canvas height in pixels
    pub height: f64,

    /// Framerate (frames per second)
    pub framerate: f64,

    /// Duration in seconds
    pub duration: f64,

    /// Root graphics object containing all layers
    pub root: GraphicsObject,

    /// Clip libraries - reusable clip definitions
    /// VectorClips can be instantiated multiple times with different transforms/timing
    pub vector_clips: HashMap<Uuid, VectorClip>,

    /// Video clip library - references to video files
    pub video_clips: HashMap<Uuid, VideoClip>,

    /// Audio clip library - sampled audio and MIDI clips
    pub audio_clips: HashMap<Uuid, AudioClip>,

    /// Image asset library - static images for fill textures
    pub image_assets: HashMap<Uuid, ImageAsset>,

    /// Instance groups for linked clip instances
    pub instance_groups: HashMap<Uuid, crate::instance_group::InstanceGroup>,

    /// Effect definitions (all effects are embedded in the document)
    #[serde(default)]
    pub effect_definitions: HashMap<Uuid, EffectDefinition>,

    /// Folder organization for vector clips
    #[serde(default)]
    pub vector_folders: AssetFolderTree,

    /// Folder organization for video clips
    #[serde(default)]
    pub video_folders: AssetFolderTree,

    /// Folder organization for audio clips
    #[serde(default)]
    pub audio_folders: AssetFolderTree,

    /// Folder organization for image assets
    #[serde(default)]
    pub image_folders: AssetFolderTree,

    /// Folder organization for effect definitions
    #[serde(default)]
    pub effect_folders: AssetFolderTree,

    /// Current UI layout state (serialized for save/load)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_layout: Option<LayoutNode>,

    /// Name of base layout this was derived from (for reference only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_layout_base: Option<String>,

    /// Current playback time in seconds
    #[serde(skip)]
    pub current_time: f64,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Untitled".to_string(),
            background_color: ShapeColor::rgb(255, 255, 255), // White background
            width: 1920.0,
            height: 1080.0,
            framerate: 60.0,
            duration: 10.0,
            root: GraphicsObject::default(),
            vector_clips: HashMap::new(),
            video_clips: HashMap::new(),
            audio_clips: HashMap::new(),
            image_assets: HashMap::new(),
            instance_groups: HashMap::new(),
            effect_definitions: HashMap::new(),
            vector_folders: AssetFolderTree::new(),
            video_folders: AssetFolderTree::new(),
            audio_folders: AssetFolderTree::new(),
            image_folders: AssetFolderTree::new(),
            effect_folders: AssetFolderTree::new(),
            ui_layout: None,
            ui_layout_base: None,
            current_time: 0.0,
        }
    }
}

impl Document {
    /// Create a new document with default settings
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Create a document with custom dimensions
    pub fn with_size(name: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            name: name.into(),
            width,
            height,
            ..Default::default()
        }
    }

    /// Set the background color
    pub fn with_background(mut self, color: ShapeColor) -> Self {
        self.background_color = color;
        self
    }

    /// Set the framerate
    pub fn with_framerate(mut self, framerate: f64) -> Self {
        self.framerate = framerate;
        self
    }

    /// Set the duration
    pub fn with_duration(mut self, duration: f64) -> Self {
        self.duration = duration;
        self
    }

    /// Get the aspect ratio
    pub fn aspect_ratio(&self) -> f64 {
        self.width / self.height
    }

    /// Get the folder tree for a specific asset category
    pub fn get_folder_tree(&self, category: AssetCategory) -> &AssetFolderTree {
        match category {
            AssetCategory::Vector => &self.vector_folders,
            AssetCategory::Video => &self.video_folders,
            AssetCategory::Audio => &self.audio_folders,
            AssetCategory::Images => &self.image_folders,
            AssetCategory::Effects => &self.effect_folders,
        }
    }

    /// Get a mutable reference to the folder tree for a specific asset category
    pub fn get_folder_tree_mut(&mut self, category: AssetCategory) -> &mut AssetFolderTree {
        match category {
            AssetCategory::Vector => &mut self.vector_folders,
            AssetCategory::Video => &mut self.video_folders,
            AssetCategory::Audio => &mut self.audio_folders,
            AssetCategory::Images => &mut self.image_folders,
            AssetCategory::Effects => &mut self.effect_folders,
        }
    }

    /// Calculate the actual timeline endpoint based on the last clip
    ///
    /// Returns the end time of the last clip instance across all layers,
    /// or the document's duration if no clips are found.
    pub fn calculate_timeline_endpoint(&self) -> f64 {
        let mut max_end_time: f64 = 0.0;

        // Helper function to calculate the end time of a clip instance
        let calculate_instance_end = |instance: &ClipInstance, clip_duration: f64| -> f64 {
            let effective_duration = if let Some(timeline_duration) = instance.timeline_duration {
                // Explicit timeline duration set (may include looping)
                timeline_duration
            } else {
                // Calculate from trim points
                let trim_end = instance.trim_end.unwrap_or(clip_duration);
                let trimmed_duration = trim_end - instance.trim_start;
                trimmed_duration / instance.playback_speed // Adjust for playback speed
            };
            instance.timeline_start + effective_duration
        };

        // Iterate through all layers to find the maximum end time
        for layer in &self.root.children {
            match layer {
                crate::layer::AnyLayer::Vector(vector_layer) => {
                    for instance in &vector_layer.clip_instances {
                        if let Some(clip) = self.vector_clips.get(&instance.clip_id) {
                            let end_time = calculate_instance_end(instance, clip.duration);
                            max_end_time = max_end_time.max(end_time);
                        }
                    }
                }
                crate::layer::AnyLayer::Audio(audio_layer) => {
                    for instance in &audio_layer.clip_instances {
                        if let Some(clip) = self.audio_clips.get(&instance.clip_id) {
                            let end_time = calculate_instance_end(instance, clip.duration);
                            max_end_time = max_end_time.max(end_time);
                        }
                    }
                }
                crate::layer::AnyLayer::Video(video_layer) => {
                    for instance in &video_layer.clip_instances {
                        if let Some(clip) = self.video_clips.get(&instance.clip_id) {
                            let end_time = calculate_instance_end(instance, clip.duration);
                            max_end_time = max_end_time.max(end_time);
                        }
                    }
                }
                crate::layer::AnyLayer::Effect(effect_layer) => {
                    for instance in &effect_layer.clip_instances {
                        if let Some(clip_duration) = self.get_clip_duration(&instance.clip_id) {
                            let end_time = calculate_instance_end(instance, clip_duration);
                            max_end_time = max_end_time.max(end_time);
                        }
                    }
                }
            }
        }

        // Return the maximum end time, or document duration if no clips found
        if max_end_time > 0.0 {
            max_end_time
        } else {
            self.duration
        }
    }

    /// Set the current playback time
    pub fn set_time(&mut self, time: f64) {
        self.current_time = time.max(0.0).min(self.duration);
    }

    /// Get visible layers from the root graphics object
    pub fn visible_layers(&self) -> impl Iterator<Item = &AnyLayer> {
        self.root
            .children
            .iter()
            .filter(|layer| layer.layer().visible)
    }

    /// Get a layer by ID
    pub fn get_layer(&self, id: &Uuid) -> Option<&AnyLayer> {
        self.root.get_child(id)
    }

    // === MUTATION METHODS (pub(crate) - only accessible to action module) ===

    /// Get mutable access to the root graphics object
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn root_mut(&mut self) -> &mut GraphicsObject {
        &mut self.root
    }

    /// Get mutable access to a layer by ID
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub fn get_layer_mut(&mut self, id: &Uuid) -> Option<&mut AnyLayer> {
        self.root.get_child_mut(id)
    }

    // === CLIP LIBRARY METHODS ===

    /// Add a vector clip to the library
    pub fn add_vector_clip(&mut self, clip: VectorClip) -> Uuid {
        let id = clip.id;
        self.vector_clips.insert(id, clip);
        id
    }

    /// Add a video clip to the library
    pub fn add_video_clip(&mut self, clip: VideoClip) -> Uuid {
        let id = clip.id;
        self.video_clips.insert(id, clip);
        id
    }

    /// Add an audio clip to the library
    pub fn add_audio_clip(&mut self, clip: AudioClip) -> Uuid {
        let id = clip.id;
        self.audio_clips.insert(id, clip);
        id
    }

    /// Add an instance group to the document
    pub fn add_instance_group(&mut self, group: crate::instance_group::InstanceGroup) -> Uuid {
        let id = group.id;
        self.instance_groups.insert(id, group);
        id
    }

    /// Remove an instance group from the document
    pub fn remove_instance_group(&mut self, group_id: &Uuid) {
        self.instance_groups.remove(group_id);
    }

    /// Find the group that contains a specific clip instance
    pub fn find_group_for_instance(&self, instance_id: &Uuid) -> Option<&crate::instance_group::InstanceGroup> {
        self.instance_groups.values()
            .find(|group| group.contains_instance(instance_id))
    }

    /// Get a vector clip by ID
    pub fn get_vector_clip(&self, id: &Uuid) -> Option<&VectorClip> {
        self.vector_clips.get(id)
    }

    /// Get a video clip by ID
    pub fn get_video_clip(&self, id: &Uuid) -> Option<&VideoClip> {
        self.video_clips.get(id)
    }

    /// Get an audio clip by ID
    pub fn get_audio_clip(&self, id: &Uuid) -> Option<&AudioClip> {
        self.audio_clips.get(id)
    }

    /// Get a mutable vector clip by ID
    pub fn get_vector_clip_mut(&mut self, id: &Uuid) -> Option<&mut VectorClip> {
        self.vector_clips.get_mut(id)
    }

    /// Get a mutable video clip by ID
    pub fn get_video_clip_mut(&mut self, id: &Uuid) -> Option<&mut VideoClip> {
        self.video_clips.get_mut(id)
    }

    /// Get a mutable audio clip by ID
    pub fn get_audio_clip_mut(&mut self, id: &Uuid) -> Option<&mut AudioClip> {
        self.audio_clips.get_mut(id)
    }

    /// Remove a vector clip from the library
    pub fn remove_vector_clip(&mut self, id: &Uuid) -> Option<VectorClip> {
        self.vector_clips.remove(id)
    }

    /// Remove a video clip from the library
    pub fn remove_video_clip(&mut self, id: &Uuid) -> Option<VideoClip> {
        self.video_clips.remove(id)
    }

    /// Remove an audio clip from the library
    pub fn remove_audio_clip(&mut self, id: &Uuid) -> Option<AudioClip> {
        self.audio_clips.remove(id)
    }

    // === IMAGE ASSET METHODS ===

    /// Add an image asset to the library
    pub fn add_image_asset(&mut self, asset: ImageAsset) -> Uuid {
        let id = asset.id;
        self.image_assets.insert(id, asset);
        id
    }

    /// Get an image asset by ID
    pub fn get_image_asset(&self, id: &Uuid) -> Option<&ImageAsset> {
        self.image_assets.get(id)
    }

    /// Get a mutable image asset by ID
    pub fn get_image_asset_mut(&mut self, id: &Uuid) -> Option<&mut ImageAsset> {
        self.image_assets.get_mut(id)
    }

    /// Remove an image asset from the library
    pub fn remove_image_asset(&mut self, id: &Uuid) -> Option<ImageAsset> {
        self.image_assets.remove(id)
    }

    // === EFFECT DEFINITION METHODS ===

    /// Add an effect definition to the document
    pub fn add_effect_definition(&mut self, definition: EffectDefinition) -> Uuid {
        let id = definition.id;
        self.effect_definitions.insert(id, definition);
        id
    }

    /// Get an effect definition by ID
    pub fn get_effect_definition(&self, id: &Uuid) -> Option<&EffectDefinition> {
        self.effect_definitions.get(id)
    }

    /// Get a mutable effect definition by ID
    pub fn get_effect_definition_mut(&mut self, id: &Uuid) -> Option<&mut EffectDefinition> {
        self.effect_definitions.get_mut(id)
    }

    /// Remove an effect definition from the document
    pub fn remove_effect_definition(&mut self, id: &Uuid) -> Option<EffectDefinition> {
        self.effect_definitions.remove(id)
    }

    /// Get all effect definitions
    pub fn effect_definitions(&self) -> impl Iterator<Item = &EffectDefinition> {
        self.effect_definitions.values()
    }

    // === CLIP OVERLAP DETECTION METHODS ===

    /// Get the duration of any clip type by ID
    ///
    /// Searches through all clip libraries to find the clip and return its duration.
    /// For effect definitions, returns `EFFECT_DURATION` (f64::MAX) since effects
    /// have infinite internal duration.
    pub fn get_clip_duration(&self, clip_id: &Uuid) -> Option<f64> {
        if let Some(clip) = self.vector_clips.get(clip_id) {
            Some(clip.duration)
        } else if let Some(clip) = self.video_clips.get(clip_id) {
            Some(clip.duration)
        } else if let Some(clip) = self.audio_clips.get(clip_id) {
            Some(clip.duration)
        } else if self.effect_definitions.contains_key(clip_id) {
            // Effects have infinite internal duration - their timeline length
            // is controlled by ClipInstance.trim_end
            Some(crate::effect::EFFECT_DURATION)
        } else {
            None
        }
    }

    /// Calculate the end time of a clip instance on the timeline
    pub fn get_clip_instance_end_time(&self, layer_id: &Uuid, instance_id: &Uuid) -> Option<f64> {
        let layer = self.get_layer(layer_id)?;

        // Find the clip instance
        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
        };

        let instance = instances.iter().find(|inst| &inst.id == instance_id)?;
        let clip_duration = self.get_clip_duration(&instance.clip_id)?;

        let trim_start = instance.trim_start;
        let trim_end = instance.trim_end.unwrap_or(clip_duration);
        let effective_duration = trim_end - trim_start;

        Some(instance.timeline_start + effective_duration)
    }

    /// Check if a time range overlaps with any existing clip on the layer
    ///
    /// Returns (overlaps, conflicting_instance_id)
    ///
    /// Only checks audio, video, and effect layers - vector/MIDI layers return false
    pub fn check_overlap_on_layer(
        &self,
        layer_id: &Uuid,
        start_time: f64,
        end_time: f64,
        exclude_instance_ids: &[Uuid],
    ) -> (bool, Option<Uuid>) {
        let Some(layer) = self.get_layer(layer_id) else {
            return (false, None);
        };

        // Check audio, video, and effect layers (effects cannot overlap on same layer)
        if !matches!(layer, AnyLayer::Audio(_) | AnyLayer::Video(_) | AnyLayer::Effect(_)) {
            return (false, None);
        }

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
        };

        for instance in instances {
            // Skip excluded instances
            if exclude_instance_ids.contains(&instance.id) {
                continue;
            }

            // Calculate instance end time
            let Some(clip_duration) = self.get_clip_duration(&instance.clip_id) else {
                continue;
            };

            let instance_start = instance.timeline_start;
            let trim_start = instance.trim_start;
            let trim_end = instance.trim_end.unwrap_or(clip_duration);
            let instance_end = instance_start + (trim_end - trim_start);

            // Check overlap: start_a < end_b AND start_b < end_a
            if start_time < instance_end && instance_start < end_time {
                return (true, Some(instance.id));
            }
        }

        (false, None)
    }

    /// Find the nearest valid position for a clip on a layer to avoid overlaps
    ///
    /// Returns adjusted timeline_start, or None if no valid position exists
    ///
    /// Strategy: Snaps to whichever side (left or right) is closest to the desired position
    pub fn find_nearest_valid_position(
        &self,
        layer_id: &Uuid,
        desired_start: f64,
        clip_duration: f64,
        exclude_instance_ids: &[Uuid],
    ) -> Option<f64> {
        let layer = self.get_layer(layer_id)?;

        // Clamp to timeline start (can't go before 0)
        let desired_start = desired_start.max(0.0);

        // Vector layers don't need overlap adjustment, but still respect timeline start
        if matches!(layer, AnyLayer::Vector(_)) {
            return Some(desired_start);
        }

        // Check if desired position is already valid
        let desired_end = desired_start + clip_duration;
        let (overlaps, _) = self.check_overlap_on_layer(layer_id, desired_start, desired_end, exclude_instance_ids);
        if !overlaps {
            return Some(desired_start);
        }

        // Collect all existing clip time ranges on this layer
        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
            AnyLayer::Vector(_) => return Some(desired_start), // Shouldn't reach here
        };

        let mut occupied_ranges: Vec<(f64, f64, Uuid)> = Vec::new();
        for instance in instances {
            if exclude_instance_ids.contains(&instance.id) {
                continue;
            }

            if let Some(clip_dur) = self.get_clip_duration(&instance.clip_id) {
                let inst_start = instance.timeline_start;
                let trim_start = instance.trim_start;
                let trim_end = instance.trim_end.unwrap_or(clip_dur);
                let inst_end = inst_start + (trim_end - trim_start);
                occupied_ranges.push((inst_start, inst_end, instance.id));
            }
        }

        // Sort by start time
        occupied_ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // Find the clip we're overlapping with and try both sides, pick nearest
        for (occupied_start, occupied_end, _) in &occupied_ranges {
            if desired_start < *occupied_end && *occupied_start < desired_end {
                let mut candidates: Vec<f64> = Vec::new();

                // Try snapping to the right (after this clip)
                let snap_right = *occupied_end;
                let snap_right_end = snap_right + clip_duration;
                let (overlaps_right, _) = self.check_overlap_on_layer(
                    layer_id,
                    snap_right,
                    snap_right_end,
                    exclude_instance_ids,
                );
                if !overlaps_right {
                    candidates.push(snap_right);
                }

                // Try snapping to the left (before this clip)
                let snap_left = occupied_start - clip_duration;
                if snap_left >= 0.0 {
                    let (overlaps_left, _) = self.check_overlap_on_layer(
                        layer_id,
                        snap_left,
                        *occupied_start,
                        exclude_instance_ids,
                    );
                    if !overlaps_left {
                        candidates.push(snap_left);
                    }
                }

                // Pick the candidate closest to desired_start
                if !candidates.is_empty() {
                    candidates.sort_by(|a, b| {
                        let dist_a = (a - desired_start).abs();
                        let dist_b = (b - desired_start).abs();
                        dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    return Some(candidates[0]);
                }
            }
        }

        // If no gap found, try placing at timeline start
        if occupied_ranges.is_empty() || occupied_ranges[0].0 >= clip_duration {
            return Some(0.0);
        }

        // No valid position found
        None
    }

    /// Clamp a group move offset so no clip in the group overlaps a non-group clip or
    /// goes before timeline start. All clips move by the same returned offset.
    pub fn clamp_group_move_offset(
        &self,
        layer_id: &Uuid,
        group: &[(Uuid, f64, f64)], // (instance_id, timeline_start, effective_duration)
        desired_offset: f64,
    ) -> f64 {
        let Some(layer) = self.get_layer(layer_id) else {
            return desired_offset;
        };
        if matches!(layer, AnyLayer::Vector(_)) {
            return desired_offset;
        }

        let group_ids: Vec<Uuid> = group.iter().map(|(id, _, _)| *id).collect();

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(a) => &a.clip_instances,
            AnyLayer::Video(v) => &v.clip_instances,
            AnyLayer::Effect(e) => &e.clip_instances,
            AnyLayer::Vector(v) => &v.clip_instances,
        };

        // Collect non-group clip ranges
        let mut non_group: Vec<(f64, f64)> = Vec::new();
        for inst in instances {
            if group_ids.contains(&inst.id) {
                continue;
            }
            if let Some(dur) = self.get_clip_duration(&inst.clip_id) {
                let end = inst.timeline_start + (inst.trim_end.unwrap_or(dur) - inst.trim_start);
                non_group.push((inst.timeline_start, end));
            }
        }

        let mut clamped = desired_offset;

        for &(_, start, duration) in group {
            let end = start + duration;

            // Can't go before timeline start
            clamped = clamped.max(-start);

            // Check against non-group clips
            for &(ns, ne) in &non_group {
                if clamped < 0.0 {
                    // Moving left: if non-group clip end is between our destination and current start
                    if ne <= start && ne > start + clamped {
                        clamped = clamped.max(ne - start);
                    }
                } else if clamped > 0.0 {
                    // Moving right: if non-group clip start is between our current end and destination
                    if ns >= end && ns < end + clamped {
                        clamped = clamped.min(ns - end);
                    }
                }
            }
        }

        clamped
    }

    /// Find the maximum amount we can extend a clip to the left without overlapping
    ///
    /// Returns the distance to the nearest clip to the left, or the distance to
    /// timeline start (0.0) if no clips exist to the left.
    pub fn find_max_trim_extend_left(
        &self,
        layer_id: &Uuid,
        instance_id: &Uuid,
        current_timeline_start: f64,
    ) -> f64 {
        let Some(layer) = self.get_layer(layer_id) else {
            return current_timeline_start; // No limit if layer not found
        };

        // Only check audio, video, and effect layers
        if matches!(layer, AnyLayer::Vector(_)) {
            return current_timeline_start; // No limit for vector layers
        };

        // Find the nearest clip to the left
        let mut nearest_end = 0.0; // Can extend to timeline start by default

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
        };

        for other in instances {
            if &other.id == instance_id {
                continue;
            }

            // Calculate other clip's end time
            if let Some(clip_duration) = self.get_clip_duration(&other.clip_id) {
                let trim_end = other.trim_end.unwrap_or(clip_duration);
                let other_end = other.timeline_start + (trim_end - other.trim_start);

                // If this clip is to the left and closer than current nearest
                if other_end <= current_timeline_start && other_end > nearest_end {
                    nearest_end = other_end;
                }
            }
        }

        current_timeline_start - nearest_end
    }

    /// Find the maximum amount we can extend a clip to the right without overlapping
    ///
    /// Returns the distance to the nearest clip to the right, or f64::MAX if no
    /// clips exist to the right.
    pub fn find_max_trim_extend_right(
        &self,
        layer_id: &Uuid,
        instance_id: &Uuid,
        current_timeline_start: f64,
        current_effective_duration: f64,
    ) -> f64 {
        let Some(layer) = self.get_layer(layer_id) else {
            return f64::MAX; // No limit if layer not found
        };

        // Only check audio, video, and effect layers
        if matches!(layer, AnyLayer::Vector(_)) {
            return f64::MAX; // No limit for vector layers
        }

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
        };

        let mut nearest_start = f64::MAX;
        let current_end = current_timeline_start + current_effective_duration;

        for other in instances {
            if &other.id == instance_id {
                continue;
            }

            // If this clip is to the right and closer than current nearest
            if other.timeline_start >= current_end && other.timeline_start < nearest_start {
                nearest_start = other.timeline_start;
            }
        }

        if nearest_start == f64::MAX {
            f64::MAX // No clip to the right, can extend freely
        } else {
            (nearest_start - current_end).max(0.0) // Gap between our end and next clip's start
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerTrait, VectorLayer};

    #[test]
    fn test_document_creation() {
        let doc = Document::new("Test Project");
        assert_eq!(doc.name, "Test Project");
        assert_eq!(doc.width, 1920.0);
        assert_eq!(doc.height, 1080.0);
        assert_eq!(doc.root.children.len(), 0);
    }

    #[test]
    fn test_graphics_object() {
        let mut root = GraphicsObject::new("Root");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = root.add_child(AnyLayer::Vector(vector_layer));

        assert_eq!(root.children.len(), 1);
        assert!(root.get_child(&layer_id).is_some());
    }

    #[test]
    fn test_document_with_layers() {
        let mut doc = Document::new("Test");

        let layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");

        // Hide layer2 to test visibility filtering
        layer2.layer.visible = false;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // Only visible layers should be returned
        assert_eq!(doc.visible_layers().count(), 1);

        // Update layer2 to be visible via root access
        let ids: Vec<_> = doc.root.children.iter().map(|n| n.id()).collect();
        if let Some(layer) = doc.root.get_child_mut(&ids[1]) {
            layer.set_visible(true);
        }

        assert_eq!(doc.visible_layers().count(), 2);
    }
}

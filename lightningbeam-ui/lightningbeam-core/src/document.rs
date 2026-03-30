//! Document structure for Lightningbeam
//!
//! The Document represents a complete animation project with settings
//! and a root graphics object containing the scene graph.

use crate::asset_folder::AssetFolderTree;
use crate::clip::{AudioClip, ClipInstance, ImageAsset, VideoClip, VectorClip};
use crate::effect::EffectDefinition;
use crate::layer::AnyLayer;
use crate::script::ScriptDefinition;
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

    /// Get a child layer by ID (searches direct children and recurses into groups)
    pub fn get_child(&self, id: &Uuid) -> Option<&AnyLayer> {
        for layer in &self.children {
            if &layer.id() == id {
                return Some(layer);
            }
            if let AnyLayer::Group(group) = layer {
                if let Some(found) = Self::find_in_group(&group.children, id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Get a mutable child layer by ID (searches direct children and recurses into groups)
    pub fn get_child_mut(&mut self, id: &Uuid) -> Option<&mut AnyLayer> {
        for layer in &mut self.children {
            if &layer.id() == id {
                return Some(layer);
            }
            if let AnyLayer::Group(group) = layer {
                if let Some(found) = Self::find_in_group_mut(&mut group.children, id) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn find_in_group<'a>(children: &'a [AnyLayer], id: &Uuid) -> Option<&'a AnyLayer> {
        for child in children {
            if &child.id() == id {
                return Some(child);
            }
            if let AnyLayer::Group(group) = child {
                if let Some(found) = Self::find_in_group(&group.children, id) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn find_in_group_mut<'a>(children: &'a mut [AnyLayer], id: &Uuid) -> Option<&'a mut AnyLayer> {
        for child in children {
            if &child.id() == id {
                return Some(child);
            }
            if let AnyLayer::Group(group) = child {
                if let Some(found) = Self::find_in_group_mut(&mut group.children, id) {
                    return Some(found);
                }
            }
        }
        None
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

/// Musical time signature
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u32,   // beats per measure (e.g., 4)
    pub denominator: u32, // beat unit (e.g., 4 = quarter note)
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self { numerator: 4, denominator: 4 }
    }
}

fn default_bpm() -> f64 { 120.0 }

/// How time is displayed in the timeline
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimelineMode {
    #[default]
    Seconds,
    Measures,
    Frames,
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

    /// Tempo in beats per minute
    #[serde(default = "default_bpm")]
    pub bpm: f64,

    /// Time signature
    #[serde(default)]
    pub time_signature: TimeSignature,

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

    /// BeamDSP script definitions (audio DSP scripts for node graph)
    #[serde(default)]
    pub script_definitions: HashMap<Uuid, ScriptDefinition>,

    /// Folder organization for script definitions
    #[serde(default)]
    pub script_folders: AssetFolderTree,

    /// How time is displayed in the timeline (saved with document)
    #[serde(default)]
    pub timeline_mode: TimelineMode,

    /// Current UI layout state (serialized for save/load)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_layout: Option<LayoutNode>,

    /// Name of base layout this was derived from (for reference only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_layout_base: Option<String>,

    /// Current playback time in seconds
    #[serde(skip)]
    pub current_time: f64,

    /// Reverse lookup: layer_id → clip_id for layers inside vector clips.
    /// Enables O(1) lookup in get_layer/get_layer_mut instead of scanning all clips.
    #[serde(skip)]
    pub layer_to_clip_map: HashMap<Uuid, Uuid>,
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
            bpm: 120.0,
            time_signature: TimeSignature::default(),
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
            script_definitions: HashMap::new(),
            script_folders: AssetFolderTree::new(),
            timeline_mode: TimelineMode::Seconds,
            ui_layout: None,
            ui_layout_base: None,
            current_time: 0.0,
            layer_to_clip_map: HashMap::new(),
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

    /// Rebuild the layer→clip reverse lookup map from all vector clips.
    /// Call after deserialization or bulk clip modifications.
    pub fn rebuild_layer_to_clip_map(&mut self) {
        self.layer_to_clip_map.clear();
        for (clip_id, clip) in &self.vector_clips {
            for node in &clip.layers.roots {
                self.layer_to_clip_map.insert(node.data.id(), *clip_id);
            }
        }
    }

    /// Register a layer as belonging to a clip (for O(1) lookup).
    pub fn register_layer_in_clip(&mut self, layer_id: Uuid, clip_id: Uuid) {
        self.layer_to_clip_map.insert(layer_id, clip_id);
    }

    /// Unregister a layer from the clip lookup map.
    pub fn unregister_layer_from_clip(&mut self, layer_id: &Uuid) {
        self.layer_to_clip_map.remove(layer_id);
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
                crate::layer::AnyLayer::Raster(_) => {
                    // Raster layers don't have clip instances
                }
                crate::layer::AnyLayer::Group(group) => {
                    // Recurse into group children to find their clip instance endpoints
                    fn process_group_children(
                        children: &[crate::layer::AnyLayer],
                        doc: &Document,
                        max_end: &mut f64,
                        calc_end: &dyn Fn(&ClipInstance, f64) -> f64,
                    ) {
                        for child in children {
                            match child {
                                crate::layer::AnyLayer::Vector(vl) => {
                                    for inst in &vl.clip_instances {
                                        if let Some(clip) = doc.vector_clips.get(&inst.clip_id) {
                                            *max_end = max_end.max(calc_end(inst, clip.duration));
                                        }
                                    }
                                }
                                crate::layer::AnyLayer::Audio(al) => {
                                    for inst in &al.clip_instances {
                                        if let Some(clip) = doc.audio_clips.get(&inst.clip_id) {
                                            *max_end = max_end.max(calc_end(inst, clip.duration));
                                        }
                                    }
                                }
                                crate::layer::AnyLayer::Video(vl) => {
                                    for inst in &vl.clip_instances {
                                        if let Some(clip) = doc.video_clips.get(&inst.clip_id) {
                                            *max_end = max_end.max(calc_end(inst, clip.duration));
                                        }
                                    }
                                }
                                crate::layer::AnyLayer::Effect(el) => {
                                    for inst in &el.clip_instances {
                                        if let Some(dur) = doc.get_clip_duration(&inst.clip_id) {
                                            *max_end = max_end.max(calc_end(inst, dur));
                                        }
                                    }
                                }
                                crate::layer::AnyLayer::Raster(_) => {
                                    // Raster layers don't have clip instances
                                }
                                crate::layer::AnyLayer::Group(g) => {
                                    process_group_children(&g.children, doc, max_end, calc_end);
                                }
                            }
                        }
                    }
                    process_group_children(&group.children, self, &mut max_end_time, &calculate_instance_end);
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

    /// Get visible layers for the current editing context
    pub fn context_visible_layers(&self, clip_id: Option<&Uuid>) -> Vec<&AnyLayer> {
        self.context_layers(clip_id)
            .into_iter()
            .filter(|layer| layer.layer().visible)
            .collect()
    }

    /// Get a layer by ID (searches root layers, then clip layers via O(1) map lookup)
    pub fn get_layer(&self, id: &Uuid) -> Option<&AnyLayer> {
        // First check root layers
        if let Some(layer) = self.root.get_child(id) {
            return Some(layer);
        }
        // O(1) lookup: check if this layer belongs to a clip
        if let Some(clip_id) = self.layer_to_clip_map.get(id) {
            if let Some(clip) = self.vector_clips.get(clip_id) {
                for node in &clip.layers.roots {
                    if &node.data.id() == id {
                        return Some(&node.data);
                    }
                }
            }
        }
        None
    }

    // === MUTATION METHODS (pub(crate) - only accessible to action module) ===

    /// Get mutable access to the root graphics object
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn root_mut(&mut self) -> &mut GraphicsObject {
        &mut self.root
    }

    /// Get mutable access to a layer by ID (searches root layers, then clip layers via O(1) map lookup)
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub fn get_layer_mut(&mut self, id: &Uuid) -> Option<&mut AnyLayer> {
        // First check root layers
        if self.root.get_child(id).is_some() {
            return self.root.get_child_mut(id);
        }
        // O(1) lookup: check if this layer belongs to a clip
        if let Some(clip_id) = self.layer_to_clip_map.get(id).copied() {
            if let Some(clip) = self.vector_clips.get_mut(&clip_id) {
                for node in &mut clip.layers.roots {
                    if &node.data.id() == id {
                        return Some(&mut node.data);
                    }
                }
            }
        }
        None
    }

    // === EDITING CONTEXT METHODS ===

    /// Get the layers for the current editing context.
    /// When `clip_id` is None, returns root layers. When Some, returns the clip's layers.
    pub fn context_layers(&self, clip_id: Option<&Uuid>) -> Vec<&AnyLayer> {
        match clip_id {
            None => self.root.children.iter().collect(),
            Some(id) => self.vector_clips.get(id)
                .map(|clip| clip.layers.root_data())
                .unwrap_or_default(),
        }
    }

    /// Get mutable layers for the current editing context.
    pub fn context_layers_mut(&mut self, clip_id: Option<&Uuid>) -> Vec<&mut AnyLayer> {
        match clip_id {
            None => self.root.children.iter_mut().collect(),
            Some(id) => self.vector_clips.get_mut(id)
                .map(|clip| clip.layers.root_data_mut())
                .unwrap_or_default(),
        }
    }

    /// Look up a layer by ID within an editing context.
    pub fn get_layer_in_context(&self, clip_id: Option<&Uuid>, layer_id: &Uuid) -> Option<&AnyLayer> {
        self.context_layers(clip_id).into_iter().find(|l| &l.id() == layer_id)
    }

    /// Look up a mutable layer by ID within an editing context.
    pub fn get_layer_in_context_mut(&mut self, clip_id: Option<&Uuid>, layer_id: &Uuid) -> Option<&mut AnyLayer> {
        self.context_layers_mut(clip_id).into_iter().find(|l| &l.id() == layer_id)
    }

    /// Get all layers across the entire document (root + inside all vector clips).
    pub fn all_layers(&self) -> Vec<&AnyLayer> {
        let mut layers: Vec<&AnyLayer> = Vec::new();
        fn collect_layers<'a>(list: &'a [AnyLayer], out: &mut Vec<&'a AnyLayer>) {
            for layer in list {
                out.push(layer);
                if let AnyLayer::Group(g) = layer {
                    collect_layers(&g.children, out);
                }
            }
        }
        collect_layers(&self.root.children, &mut layers);
        for clip in self.vector_clips.values() {
            layers.extend(clip.layers.root_data());
        }
        layers
    }

    /// Migrate old documents: compute beats/frames from seconds for any ClipInstance whose
    /// derived fields are still zero (i.e., documents saved before triple-representation).
    /// Call once after loading a document.
    pub fn sync_all_clip_positions(&mut self) {
        let bpm = self.bpm;
        let fps = self.framerate;

        fn sync_list(list: &mut [crate::layer::AnyLayer], bpm: f64, fps: f64) {
            for layer in list.iter_mut() {
                match layer {
                    crate::layer::AnyLayer::Vector(vl) => {
                        for ci in &mut vl.clip_instances {
                            if ci.timeline_start_beats == 0.0 { ci.sync_from_seconds(bpm, fps); }
                        }
                    }
                    crate::layer::AnyLayer::Audio(al) => {
                        for ci in &mut al.clip_instances {
                            if ci.timeline_start_beats == 0.0 { ci.sync_from_seconds(bpm, fps); }
                        }
                    }
                    crate::layer::AnyLayer::Video(vl) => {
                        for ci in &mut vl.clip_instances {
                            if ci.timeline_start_beats == 0.0 { ci.sync_from_seconds(bpm, fps); }
                        }
                    }
                    crate::layer::AnyLayer::Effect(el) => {
                        for ci in &mut el.clip_instances {
                            if ci.timeline_start_beats == 0.0 { ci.sync_from_seconds(bpm, fps); }
                        }
                    }
                    crate::layer::AnyLayer::Group(g) => {
                        sync_list(&mut g.children, bpm, fps);
                    }
                    crate::layer::AnyLayer::Raster(_) => {}
                }
            }
        }

        sync_list(&mut self.root.children, bpm, fps);
        for clip in self.vector_clips.values_mut() {
            for node in &mut clip.layers.roots {
                if let crate::layer::AnyLayer::Vector(vl) = &mut node.data {
                    for ci in &mut vl.clip_instances {
                        if ci.timeline_start_beats == 0.0 { ci.sync_from_seconds(bpm, fps); }
                    }
                }
            }
        }
    }

    // === CLIP LIBRARY METHODS ===

    /// Add a vector clip to the library
    pub fn add_vector_clip(&mut self, clip: VectorClip) -> Uuid {
        let id = clip.id;
        // Register all layers in the clip for O(1) reverse lookup
        for node in &clip.layers.roots {
            self.layer_to_clip_map.insert(node.data.id(), id);
        }
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

    /// Find the document audio clip (UUID + ref) that owns the given backend pool index.
    pub fn audio_clip_by_pool_index(&self, pool_index: usize) -> Option<(Uuid, &AudioClip)> {
        self.audio_clips.iter()
            .find(|(_, c)| c.audio_pool_index() == Some(pool_index))
            .map(|(&id, c)| (id, c))
    }

    /// Find the document audio clip (UUID + ref) that owns the given backend MIDI clip ID.
    pub fn audio_clip_by_midi_clip_id(&self, midi_clip_id: u32) -> Option<(Uuid, &AudioClip)> {
        self.audio_clips.iter()
            .find(|(_, c)| c.midi_clip_id() == Some(midi_clip_id))
            .map(|(&id, c)| (id, c))
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
        if let Some(clip) = self.vector_clips.remove(id) {
            // Unregister all layers from the reverse lookup map
            for node in &clip.layers.roots {
                self.layer_to_clip_map.remove(&node.data.id());
            }
            Some(clip)
        } else {
            None
        }
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

    // === SCRIPT DEFINITION METHODS ===

    pub fn add_script_definition(&mut self, definition: ScriptDefinition) -> Uuid {
        let id = definition.id;
        self.script_definitions.insert(id, definition);
        id
    }

    pub fn get_script_definition(&self, id: &Uuid) -> Option<&ScriptDefinition> {
        self.script_definitions.get(id)
    }

    pub fn get_script_definition_mut(&mut self, id: &Uuid) -> Option<&mut ScriptDefinition> {
        self.script_definitions.get_mut(id)
    }

    pub fn script_definitions(&self) -> impl Iterator<Item = &ScriptDefinition> {
        self.script_definitions.values()
    }

    // === CLIP OVERLAP DETECTION METHODS ===

    /// Get the duration of any clip type by ID
    ///
    /// Searches through all clip libraries to find the clip and return its duration.
    /// For effect definitions, returns `EFFECT_DURATION` (f64::MAX) since effects
    /// have infinite internal duration.
    pub fn get_clip_duration(&self, clip_id: &Uuid) -> Option<f64> {
        if let Some(clip) = self.vector_clips.get(clip_id) {
            if clip.is_group {
                Some(clip.duration)
            } else {
                Some(clip.content_duration_with(self.framerate, |id| {
                    // Resolve nested clip durations (audio, video, other vector clips)
                    if let Some(vc) = self.vector_clips.get(id) {
                        // Avoid deep recursion — use stored duration for nested vector clips
                        Some(vc.content_duration(self.framerate))
                    } else if let Some(ac) = self.audio_clips.get(id) {
                        Some(ac.duration)
                    } else if let Some(vc) = self.video_clips.get(id) {
                        Some(vc.duration)
                    } else if self.effect_definitions.contains_key(id) {
                        Some(crate::effect::EFFECT_DURATION)
                    } else {
                        None
                    }
                }))
            }
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
            AnyLayer::Group(_) => &[],
            AnyLayer::Raster(_) => &[],
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
            AnyLayer::Group(_) => &[],
            AnyLayer::Raster(_) => &[],
        };

        for instance in instances {
            // Skip excluded instances
            if exclude_instance_ids.contains(&instance.id) {
                continue;
            }

            // Calculate instance extent (accounting for loop_before)
            let Some(clip_duration) = self.get_clip_duration(&instance.clip_id) else {
                continue;
            };

            let instance_start = instance.effective_start();
            let instance_end = instance.timeline_start + instance.effective_duration(clip_duration);

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
        if matches!(layer, AnyLayer::Vector(_) | AnyLayer::Group(_)) {
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
            AnyLayer::Group(_) => return Some(desired_start), // Groups don't have own clips
            AnyLayer::Raster(_) => return Some(desired_start), // Raster layers don't have own clips
        };

        let mut occupied_ranges: Vec<(f64, f64, Uuid)> = Vec::new();
        for instance in instances {
            if exclude_instance_ids.contains(&instance.id) {
                continue;
            }

            if let Some(clip_dur) = self.get_clip_duration(&instance.clip_id) {
                let inst_start = instance.effective_start();
                let inst_end = instance.timeline_start + instance.effective_duration(clip_dur);
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
        if matches!(layer, AnyLayer::Vector(_) | AnyLayer::Group(_)) {
            return desired_offset;
        }

        let group_ids: Vec<Uuid> = group.iter().map(|(id, _, _)| *id).collect();

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(a) => &a.clip_instances,
            AnyLayer::Video(v) => &v.clip_instances,
            AnyLayer::Effect(e) => &e.clip_instances,
            AnyLayer::Vector(v) => &v.clip_instances,
            AnyLayer::Group(_) => &[],
            AnyLayer::Raster(_) => &[],
        };

        // Collect non-group clip ranges
        let mut non_group: Vec<(f64, f64)> = Vec::new();
        for inst in instances {
            if group_ids.contains(&inst.id) {
                continue;
            }
            if let Some(dur) = self.get_clip_duration(&inst.clip_id) {
                let start = inst.effective_start();
                let end = inst.timeline_start + inst.effective_duration(dur);
                non_group.push((start, end));
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
        if matches!(layer, AnyLayer::Vector(_) | AnyLayer::Group(_)) {
            return current_timeline_start; // No limit for vector/group layers
        };

        // Find the nearest clip to the left
        let mut nearest_end = 0.0; // Can extend to timeline start by default

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
            AnyLayer::Group(_) => &[],
            AnyLayer::Raster(_) => &[],
        };

        for other in instances {
            if &other.id == instance_id {
                continue;
            }

            // Calculate other clip's extent (accounting for loop_before)
            if let Some(clip_duration) = self.get_clip_duration(&other.clip_id) {
                let other_end = other.timeline_start + other.effective_duration(clip_duration);

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
        if matches!(layer, AnyLayer::Vector(_) | AnyLayer::Group(_)) {
            return f64::MAX; // No limit for vector/group layers
        }

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
            AnyLayer::Group(_) => &[],
            AnyLayer::Raster(_) => &[],
        };

        let mut nearest_start = f64::MAX;
        let current_end = current_timeline_start + current_effective_duration;

        for other in instances {
            if &other.id == instance_id {
                continue;
            }

            // Use effective_start to account for loop_before on the other clip
            let other_start = other.effective_start();
            if other_start >= current_end && other_start < nearest_start {
                nearest_start = other_start;
            }
        }

        if nearest_start == f64::MAX {
            f64::MAX // No clip to the right, can extend freely
        } else {
            (nearest_start - current_end).max(0.0) // Gap between our end and next clip's start
        }
    }
    /// Find the maximum amount we can extend loop_before to the left without overlapping.
    ///
    /// Returns the max additional loop_before distance (from the current effective start).
    pub fn find_max_loop_extend_left(
        &self,
        layer_id: &Uuid,
        instance_id: &Uuid,
        current_effective_start: f64,
    ) -> f64 {
        let Some(layer) = self.get_layer(layer_id) else {
            return current_effective_start;
        };

        if matches!(layer, AnyLayer::Vector(_) | AnyLayer::Group(_)) {
            return current_effective_start;
        }

        let instances: &[ClipInstance] = match layer {
            AnyLayer::Audio(audio) => &audio.clip_instances,
            AnyLayer::Video(video) => &video.clip_instances,
            AnyLayer::Effect(effect) => &effect.clip_instances,
            AnyLayer::Vector(vector) => &vector.clip_instances,
            AnyLayer::Group(_) => &[],
            AnyLayer::Raster(_) => &[],
        };

        let mut nearest_end = 0.0;

        for other in instances {
            if &other.id == instance_id {
                continue;
            }

            if let Some(clip_duration) = self.get_clip_duration(&other.clip_id) {
                let other_end = other.timeline_start + other.effective_duration(clip_duration);

                if other_end <= current_effective_start && other_end > nearest_end {
                    nearest_end = other_end;
                }
            }
        }

        current_effective_start - nearest_end
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

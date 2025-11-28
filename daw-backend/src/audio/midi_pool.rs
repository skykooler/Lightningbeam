use std::collections::HashMap;
use super::midi::{MidiClip, MidiClipId, MidiEvent};

/// Pool for storing MIDI clip content
/// Similar to AudioClipPool but for MIDI data
pub struct MidiClipPool {
    clips: HashMap<MidiClipId, MidiClip>,
    next_id: MidiClipId,
}

impl MidiClipPool {
    /// Create a new empty MIDI clip pool
    pub fn new() -> Self {
        Self {
            clips: HashMap::new(),
            next_id: 1, // Start at 1 so 0 can indicate "no clip"
        }
    }

    /// Add a new clip to the pool with the given events and duration
    /// Returns the ID of the newly created clip
    pub fn add_clip(&mut self, events: Vec<MidiEvent>, duration: f64, name: String) -> MidiClipId {
        let id = self.next_id;
        self.next_id += 1;

        let clip = MidiClip::new(id, events, duration, name);
        self.clips.insert(id, clip);
        id
    }

    /// Add an existing clip to the pool (used when loading projects)
    /// The clip's ID is preserved
    pub fn add_existing_clip(&mut self, clip: MidiClip) {
        // Update next_id to avoid collisions
        if clip.id >= self.next_id {
            self.next_id = clip.id + 1;
        }
        self.clips.insert(clip.id, clip);
    }

    /// Get a clip by ID
    pub fn get_clip(&self, id: MidiClipId) -> Option<&MidiClip> {
        self.clips.get(&id)
    }

    /// Get a mutable clip by ID
    pub fn get_clip_mut(&mut self, id: MidiClipId) -> Option<&mut MidiClip> {
        self.clips.get_mut(&id)
    }

    /// Remove a clip from the pool
    pub fn remove_clip(&mut self, id: MidiClipId) -> Option<MidiClip> {
        self.clips.remove(&id)
    }

    /// Duplicate a clip, returning the new clip's ID
    pub fn duplicate_clip(&mut self, id: MidiClipId) -> Option<MidiClipId> {
        let clip = self.clips.get(&id)?;
        let new_id = self.next_id;
        self.next_id += 1;

        let mut new_clip = clip.clone();
        new_clip.id = new_id;
        new_clip.name = format!("{} (copy)", clip.name);

        self.clips.insert(new_id, new_clip);
        Some(new_id)
    }

    /// Get all clip IDs in the pool
    pub fn clip_ids(&self) -> Vec<MidiClipId> {
        self.clips.keys().copied().collect()
    }

    /// Get the number of clips in the pool
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// Check if the pool is empty
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Clear all clips from the pool
    pub fn clear(&mut self) {
        self.clips.clear();
        self.next_id = 1;
    }

    /// Get an iterator over all clips
    pub fn iter(&self) -> impl Iterator<Item = (&MidiClipId, &MidiClip)> {
        self.clips.iter()
    }
}

impl Default for MidiClipPool {
    fn default() -> Self {
        Self::new()
    }
}

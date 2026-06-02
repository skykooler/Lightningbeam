use uuid::Uuid;
use serde::{Serialize, Deserialize};

/// A group of clip instances that should be manipulated together
///
/// Instance groups ensure that operations like moving or trimming
/// are applied to all member instances simultaneously. This is used
/// to keep video and audio clip instances synchronized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceGroup {
    /// Unique identifier for this group
    pub id: Uuid,

    /// Optional name for the group (e.g., "Video 1 + Audio")
    pub name: Option<String>,

    /// Instance IDs in this group (across potentially different layers)
    /// Format: Vec<(layer_id, clip_instance_id)>
    pub members: Vec<(Uuid, Uuid)>,
}

impl InstanceGroup {
    /// Create a new empty instance group
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            members: Vec::new(),
        }
    }

    /// Set the name for this group
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a member to this group
    pub fn add_member(&mut self, layer_id: Uuid, instance_id: Uuid) {
        self.members.push((layer_id, instance_id));
    }

    /// Check if this group contains a specific instance
    pub fn contains_instance(&self, instance_id: &Uuid) -> bool {
        self.members.iter().any(|(_, id)| id == instance_id)
    }

    /// Get all members of this group
    pub fn get_members(&self) -> &[(Uuid, Uuid)] {
        &self.members
    }
}

impl Default for InstanceGroup {
    fn default() -> Self {
        Self::new()
    }
}

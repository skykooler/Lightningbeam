/// BeamDSP script definitions for the asset library
///
/// Scripts are audio DSP programs written in the BeamDSP language.
/// They live in the asset library and can be referenced by Script nodes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A BeamDSP script definition stored in the document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptDefinition {
    pub id: Uuid,
    pub name: String,
    pub source: String,
    /// Folder this script belongs to (None = root)
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

impl ScriptDefinition {
    pub fn new(name: String, source: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            source,
            folder_id: None,
        }
    }

    pub fn with_id(id: Uuid, name: String, source: String) -> Self {
        Self {
            id,
            name,
            source,
            folder_id: None,
        }
    }
}

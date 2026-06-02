//! Debug test mode data types — input recording, panic capture & visual replay.
//!
//! All types are gated behind `#[cfg(debug_assertions)]` at the module level.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Serializable 2D point (avoids needing kurbo serde dependency)
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SerPoint {
    pub x: f64,
    pub y: f64,
}

impl From<vello::kurbo::Point> for SerPoint {
    fn from(p: vello::kurbo::Point) -> Self {
        Self { x: p.x, y: p.y }
    }
}

impl From<SerPoint> for vello::kurbo::Point {
    fn from(p: SerPoint) -> Self {
        vello::kurbo::Point::new(p.x, p.y)
    }
}

impl From<egui::Vec2> for SerPoint {
    fn from(v: egui::Vec2) -> Self {
        Self {
            x: v.x as f64,
            y: v.y as f64,
        }
    }
}

/// Serializable modifier keys
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SerModifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// All recordable event types — recorded in clip-local document coordinates
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestEventKind {
    MouseDown { pos: SerPoint },
    MouseUp { pos: SerPoint },
    MouseDrag { pos: SerPoint },
    MouseMove { pos: SerPoint },
    Scroll { delta_x: f32, delta_y: f32 },
    KeyDown { key: String, modifiers: SerModifiers },
    KeyUp { key: String, modifiers: SerModifiers },
    ToolChanged { tool: String },
    ActionExecuted { description: String },
}

/// A single timestamped event
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestEvent {
    pub index: usize,
    pub timestamp_ms: u64,
    pub kind: TestEventKind,
}

/// Initial state snapshot for deterministic replay
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanvasState {
    pub zoom: f32,
    pub pan_offset: (f32, f32),
    pub selected_tool: String,
    pub fill_color: [u8; 4],
    pub stroke_color: [u8; 4],
    pub stroke_width: f64,
    pub fill_enabled: bool,
    pub snap_enabled: bool,
    pub polygon_sides: u32,
}

/// A complete test case (saved as pretty-printed JSON)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestCase {
    pub name: String,
    pub description: String,
    pub recorded_at: String,
    pub initial_canvas: CanvasState,
    pub events: Vec<TestEvent>,
    pub ended_with_panic: bool,
    pub panic_message: Option<String>,
    pub panic_backtrace: Option<String>,
    /// Serialized geometry context at the time of the crash (e.g. DCEL + region path).
    /// Populated by set_pending_geometry before risky operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_context: Option<serde_json::Value>,
}

impl TestCase {
    /// Create a new empty test case with the given name and canvas state
    pub fn new(name: String, initial_canvas: CanvasState) -> Self {
        Self {
            name,
            description: String::new(),
            recorded_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            initial_canvas,
            events: Vec::new(),
            ended_with_panic: false,
            panic_message: None,
            panic_backtrace: None,
            geometry_context: None,
        }
    }

    /// Save to a JSON file
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    /// Load from a JSON file
    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

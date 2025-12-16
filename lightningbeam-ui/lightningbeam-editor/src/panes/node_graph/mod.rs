//! Node Graph Pane
//!
//! Audio/MIDI node graph editor for modular synthesis and effects processing

pub mod actions;
pub mod audio_backend;
pub mod backend;
pub mod graph_data;
pub mod node_types;

use backend::{BackendNodeId, GraphBackend};
use graph_data::{AllNodeTemplates, DataType, GraphState, NodeData, NodeTemplate, ValueType};
use super::NodePath;
use eframe::egui;
use egui_node_graph2::*;
use std::collections::HashMap;
use uuid::Uuid;

/// Node graph pane with egui_node_graph2 integration
pub struct NodeGraphPane {
    /// The graph editor state
    state: GraphEditorState<NodeData, DataType, ValueType, NodeTemplate, GraphState>,

    /// User state for the graph
    user_state: GraphState,

    /// Backend integration
    #[allow(dead_code)]
    backend: Option<Box<dyn GraphBackend>>,

    /// Maps frontend node IDs to backend node IDs
    #[allow(dead_code)]
    node_id_map: HashMap<NodeId, BackendNodeId>,

    /// Track ID this graph belongs to
    #[allow(dead_code)]
    track_id: Option<Uuid>,

    /// Pending action to execute
    #[allow(dead_code)]
    pending_action: Option<Box<dyn lightningbeam_core::action::Action>>,
}

impl NodeGraphPane {
    pub fn new() -> Self {
        let state = GraphEditorState::new(1.0);

        Self {
            state,
            user_state: GraphState::default(),
            backend: None,
            node_id_map: HashMap::new(),
            track_id: None,
            pending_action: None,
        }
    }

    pub fn with_track_id(
        track_id: Uuid,
        audio_controller: std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>,
    ) -> Self {
        // Get backend track ID (placeholder - would need actual mapping)
        let backend_track_id = 0;

        let backend = Box::new(audio_backend::AudioGraphBackend::new(
            backend_track_id,
            audio_controller,
        ));

        Self {
            state: GraphEditorState::new(1.0),
            user_state: GraphState::default(),
            backend: Some(backend),
            node_id_map: HashMap::new(),
            track_id: Some(track_id),
            pending_action: None,
        }
    }

    fn draw_dot_grid_background(
        ui: &mut egui::Ui,
        rect: egui::Rect,
        bg_color: egui::Color32,
        dot_color: egui::Color32,
        pan_zoom: &egui_node_graph2::PanZoom,
    ) {
        let painter = ui.painter();

        // Draw background
        painter.rect_filled(rect, 0.0, bg_color);

        // Draw grid dots with pan/zoom transform
        let grid_spacing = 20.0;
        let dot_radius = 1.0 * pan_zoom.zoom;

        // Get pan offset and zoom
        let pan = pan_zoom.pan;
        let zoom = pan_zoom.zoom;

        // Calculate zoom center (same as nodes - they zoom relative to viewport center)
        let half_size = rect.size() / 2.0;
        let zoom_center = rect.min.to_vec2() + half_size - pan;

        // Calculate grid bounds in graph space
        // Screen to graph: (screen_pos - zoom_center) / zoom
        let graph_min = egui::pos2(
            (rect.min.x - zoom_center.x) / zoom,
            (rect.min.y - zoom_center.y) / zoom,
        );
        let graph_max = egui::pos2(
            (rect.max.x - zoom_center.x) / zoom,
            (rect.max.y - zoom_center.y) / zoom,
        );

        let start_x = (graph_min.x / grid_spacing).floor() * grid_spacing;
        let start_y = (graph_min.y / grid_spacing).floor() * grid_spacing;

        let mut y = start_y;
        while y < graph_max.y {
            let mut x = start_x;
            while x < graph_max.x {
                // Transform to screen space: graph_pos * zoom + zoom_center
                let screen_pos = egui::pos2(
                    x * zoom + zoom_center.x,
                    y * zoom + zoom_center.y,
                );
                if rect.contains(screen_pos) {
                    painter.circle_filled(screen_pos, dot_radius, dot_color);
                }
                x += grid_spacing;
            }
            y += grid_spacing;
        }
    }
}

impl crate::panes::PaneRenderer for NodeGraphPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        // Get colors from theme
        let bg_style = shared.theme.style(".node-graph-background", ui.ctx());
        let grid_style = shared.theme.style(".node-graph-grid", ui.ctx());

        let bg_color = bg_style.background_color.unwrap_or(egui::Color32::from_gray(45));
        let grid_color = grid_style.background_color.unwrap_or(egui::Color32::from_gray(55));

        // Allocate the rect and render the graph editor within it
        ui.allocate_ui_at_rect(rect, |ui| {
            // Check for scroll input to override library's default zoom behavior
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let modifiers = ui.input(|i| i.modifiers);
            let has_scroll = scroll_delta != egui::Vec2::ZERO;
            let has_ctrl = modifiers.ctrl || modifiers.command;

            // Save current zoom to detect if library changed it
            let zoom_before = self.state.pan_zoom.zoom;
            let pan_before = self.state.pan_zoom.pan;

            // Draw dot grid background with pan/zoom
            let pan_zoom = &self.state.pan_zoom;
            Self::draw_dot_grid_background(ui, rect, bg_color, grid_color, pan_zoom);

            // Draw the graph editor (library will process scroll as zoom by default)
            let _graph_response = self.state.draw_graph_editor(
                ui,
                AllNodeTemplates,
                &mut self.user_state,
                Vec::default(),
            );

            // Override library's default scroll behavior:
            // - Library uses scroll for zoom
            // - We want: scroll = pan, ctrl+scroll = zoom
            if has_scroll && ui.rect_contains_pointer(rect) {
                if !has_ctrl {
                    // Scroll without ctrl: library zoomed, but we want pan instead
                    // Undo the zoom and apply pan
                    if self.state.pan_zoom.zoom != zoom_before {
                        // Library changed zoom - revert it
                        let undo_zoom = zoom_before / self.state.pan_zoom.zoom;
                        self.state.zoom(ui, undo_zoom);
                    }
                    // Apply pan
                    self.state.pan_zoom.pan = pan_before + scroll_delta;
                }
                // If ctrl is held, library already zoomed correctly, so do nothing
            }

            // Draw menu button in top-left corner
            let button_pos = rect.min + egui::vec2(8.0, 8.0);
            ui.allocate_ui_at_rect(
                egui::Rect::from_min_size(button_pos, egui::vec2(100.0, 24.0)),
                |ui| {
                    if ui.button("➕ Add Node").clicked() {
                        // Open node finder at button's top-left position
                        self.state.node_finder = Some(egui_node_graph2::NodeFinder::new_at(button_pos));
                    }
                },
            );
        });

        // TODO: Handle node responses and sync with backend
    }

    fn name(&self) -> &str {
        "Node Graph"
    }
}

impl Default for NodeGraphPane {
    fn default() -> Self {
        Self::new()
    }
}

//! Node Graph Pane
//!
//! Audio/MIDI node graph editor for modular synthesis and effects processing

pub mod actions;
pub mod audio_backend;
pub mod backend;
pub mod graph_data;
pub mod node_types;
pub mod palette;

use backend::{BackendNodeId, GraphBackend};
use graph_data::{AudioNode, AudioNodeViewer};
use node_types::NodeTypeRegistry;
use palette::NodePalette;
use super::NodePath;
use eframe::egui;
use egui_snarl::ui::{SnarlWidget, SnarlStyle, BackgroundPattern, Grid};
use egui_snarl::Snarl;
use std::collections::HashMap;
use uuid::Uuid;

/// Node graph pane with egui-snarl integration
pub struct NodeGraphPane {
    /// The graph structure
    snarl: Snarl<AudioNode>,

    /// Node viewer for rendering
    viewer: AudioNodeViewer,

    /// Node palette (left sidebar)
    palette: NodePalette,

    /// Node type registry
    node_registry: NodeTypeRegistry,

    /// Backend integration
    #[allow(dead_code)]
    backend: Option<Box<dyn GraphBackend>>,

    /// Maps frontend node IDs to backend node IDs
    #[allow(dead_code)]
    node_id_map: HashMap<egui_snarl::NodeId, BackendNodeId>,

    /// Track ID this graph belongs to
    #[allow(dead_code)]
    track_id: Option<Uuid>,

    /// Pending action to execute
    #[allow(dead_code)]
    pending_action: Option<Box<dyn lightningbeam_core::action::Action>>,

    /// Counter for offsetting clicked nodes
    click_node_offset: f32,
}

impl NodeGraphPane {
    pub fn new() -> Self {
        let mut snarl = Snarl::new();

        // Add a test node to verify rendering works
        snarl.insert_node(
            egui::pos2(300.0, 200.0),
            AudioNode::Oscillator {
                frequency: 440.0,
                waveform: "sine".to_string(),
            },
        );

        Self {
            snarl,
            viewer: AudioNodeViewer,
            palette: NodePalette::new(),
            node_registry: NodeTypeRegistry::new(),
            backend: None,
            node_id_map: HashMap::new(),
            track_id: None,
            pending_action: None,
            click_node_offset: 0.0,
        }
    }

    pub fn with_track_id(track_id: Uuid, audio_controller: std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>) -> Self {
        // Get backend track ID (placeholder - would need actual mapping)
        let backend_track_id = 0;

        let backend = Box::new(audio_backend::AudioGraphBackend::new(
            backend_track_id,
            audio_controller,
        ));

        Self {
            snarl: Snarl::new(),
            viewer: AudioNodeViewer,
            palette: NodePalette::new(),
            node_registry: NodeTypeRegistry::new(),
            backend: Some(backend),
            node_id_map: HashMap::new(),
            track_id: Some(track_id),
            pending_action: None,
            click_node_offset: 0.0,
        }
    }
}

impl crate::panes::PaneRenderer for NodeGraphPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        _shared: &mut crate::panes::SharedPaneState,
    ) {
        // Use a horizontal layout for palette + graph
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.horizontal(|ui| {
                // Track clicked node from palette
                let mut clicked_node: Option<String> = None;

                // Left panel: Node palette (fixed width)
                ui.allocate_ui_with_layout(
                    egui::vec2(200.0, rect.height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let palette_rect = ui.available_rect_before_wrap();
                        self.palette.render(ui, palette_rect, |node_type| {
                            clicked_node = Some(node_type.to_string());
                        });
                    },
                );

                // Right panel: Graph area (fill remaining space)
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let mut style = SnarlStyle::new();
                        style.bg_pattern = Some(BackgroundPattern::Grid(Grid::default()));

                        // Get the graph rect before showing the widget
                        let graph_rect = ui.available_rect_before_wrap();

                        let response = SnarlWidget::new()
                            .style(style)
                            .show(&mut self.snarl, &mut self.viewer, ui);

                        // Handle drop first - check for released payload
                        let mut handled_drop = false;
                        if let Some(payload) = response.dnd_release_payload::<String>() {
                            // Try using hover_pos from response, which should be in the right coordinate space
                            if let Some(pos) = response.hover_pos() {
                                println!("Drop detected! Node type: {} at hover_pos {:?}", payload, pos);
                                self.add_node_at_position(&payload, pos);
                                handled_drop = true;
                            }
                        }

                        // Add clicked node at center only if we didn't handle a drop
                        if !handled_drop {
                            if let Some(ref node_type) = clicked_node {
                                // Place at a fixed graph-space position (origin) with small offset to avoid stacking
                                // This ensures nodes appear at a predictable location regardless of pan/zoom
                                let pos = egui::pos2(self.click_node_offset, self.click_node_offset);
                                self.click_node_offset += 30.0;
                                if self.click_node_offset > 300.0 {
                                    self.click_node_offset = 0.0;
                                }
                                println!("Click detected! Adding {} at graph origin with offset {:?}", node_type, pos);
                                self.add_node_at_position(node_type, pos);
                            }
                        }
                    },
                );
            });
        });
    }

    fn name(&self) -> &str {
        "Node Graph"
    }
}

impl NodeGraphPane {
    /// Add a node at a specific position
    fn add_node_at_position(&mut self, node_type: &str, pos: egui::Pos2) {
        println!("add_node_at_position called with: {} at {:?}", node_type, pos);

        // Map node type string to AudioNode enum
        let node = match node_type {
            "Oscillator" => AudioNode::Oscillator {
                frequency: 440.0,
                waveform: "sine".to_string(),
            },
            "Noise" => AudioNode::Noise {
                color: "white".to_string(),
            },
            "Filter" => AudioNode::Filter {
                cutoff: 1000.0,
                resonance: 0.5,
            },
            "Gain" => AudioNode::Gain { gain: 1.0 },
            "ADSR" => AudioNode::Adsr {
                attack: 0.01,
                decay: 0.1,
                sustain: 0.7,
                release: 0.3,
            },
            "LFO" => AudioNode::Lfo {
                frequency: 1.0,
                waveform: "sine".to_string(),
            },
            "AudioOutput" => AudioNode::AudioOutput,
            "AudioInput" => AudioNode::AudioOutput, // Map to output for now
            "MidiInput" => AudioNode::MidiInput,
            _ => {
                eprintln!("Unknown node type: {}", node_type);
                return;
            }
        };

        let node_id = self.snarl.insert_node(pos, node);

        println!("Added node: {} (ID: {:?}) at position {:?}", node_type, node_id, pos);
    }
}

impl Default for NodeGraphPane {
    fn default() -> Self {
        Self::new()
    }
}

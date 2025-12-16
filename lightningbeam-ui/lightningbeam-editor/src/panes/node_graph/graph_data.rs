//! Graph Data Types for egui-snarl
//!
//! Node definitions and viewer implementation for audio/MIDI node graph

use super::backend::BackendNodeId;
use super::node_types::DataType as SignalType;
use eframe::egui;
use egui_snarl::ui::{PinInfo, SnarlStyle, SnarlViewer};
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

/// Audio/MIDI node types
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum AudioNode {
    /// Oscillator generator
    Oscillator {
        frequency: f32,
        waveform: String,
    },
    /// Noise generator
    Noise {
        color: String,
    },
    /// Audio filter
    Filter {
        cutoff: f32,
        resonance: f32,
    },
    /// Gain/volume control
    Gain {
        gain: f32,
    },
    /// ADSR envelope
    Adsr {
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
    },
    /// LFO modulator
    Lfo {
        frequency: f32,
        waveform: String,
    },
    /// Audio output
    AudioOutput,
    /// MIDI input
    MidiInput,
}

impl AudioNode {
    /// Get the display name for this node type
    pub fn type_name(&self) -> &'static str {
        match self {
            AudioNode::Oscillator { .. } => "Oscillator",
            AudioNode::Noise { .. } => "Noise",
            AudioNode::Filter { .. } => "Filter",
            AudioNode::Gain { .. } => "Gain",
            AudioNode::Adsr { .. } => "ADSR",
            AudioNode::Lfo { .. } => "LFO",
            AudioNode::AudioOutput => "Audio Output",
            AudioNode::MidiInput => "MIDI Input",
        }
    }

    /// Get the signal type for an output pin
    fn output_type(&self, _pin: usize) -> SignalType {
        match self {
            AudioNode::MidiInput => SignalType::Midi,
            AudioNode::Lfo { .. } => SignalType::CV,
            AudioNode::Adsr { .. } => SignalType::CV,
            _ => SignalType::Audio,
        }
    }

    /// Get the signal type for an input pin
    fn input_type(&self, pin: usize) -> SignalType {
        match self {
            AudioNode::Filter { .. } => {
                if pin == 0 {
                    SignalType::Audio
                } else {
                    SignalType::CV
                }
            }
            AudioNode::Gain { .. } => {
                if pin == 0 {
                    SignalType::Audio
                } else {
                    SignalType::CV
                }
            }
            _ => SignalType::Audio,
        }
    }
}

/// Viewer implementation for audio node graph
pub struct AudioNodeViewer;

impl SnarlViewer<AudioNode> for AudioNodeViewer {
    fn title(&mut self, node: &AudioNode) -> String {
        node.type_name().to_string()
    }

    fn inputs(&mut self, node: &AudioNode) -> usize {
        match node {
            AudioNode::Oscillator { .. } => 1, // FM input
            AudioNode::Noise { .. } => 0,
            AudioNode::Filter { .. } => 2, // Audio + cutoff CV
            AudioNode::Gain { .. } => 2,   // Audio + gain CV
            AudioNode::Adsr { .. } => 1,   // Gate/trigger
            AudioNode::Lfo { .. } => 0,
            AudioNode::AudioOutput => 1,
            AudioNode::MidiInput => 0,
        }
    }

    fn outputs(&mut self, node: &AudioNode) -> usize {
        match node {
            AudioNode::AudioOutput => 0,
            _ => 1,
        }
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<AudioNode>,
    ) -> PinInfo {
        let node = &snarl[pin.id.node];
        let signal_type = node.input_type(pin.id.input);

        ui.label(match pin.id.input {
            0 => match node {
                AudioNode::Oscillator { .. } => "FM",
                AudioNode::Filter { .. } => "In",
                AudioNode::Gain { .. } => "In",
                AudioNode::Adsr { .. } => "Gate",
                AudioNode::AudioOutput => "In",
                _ => "In",
            },
            1 => match node {
                AudioNode::Filter { .. } => "Cutoff",
                AudioNode::Gain { .. } => "Gain",
                _ => "In",
            },
            _ => "In",
        });

        PinInfo::square().with_fill(signal_type.color())
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<AudioNode>,
    ) -> PinInfo {
        let node = &snarl[pin.id.node];
        let signal_type = node.output_type(pin.id.output);

        ui.label("Out");

        PinInfo::square().with_fill(signal_type.color())
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<AudioNode>) {
        let from_node = &snarl[from.id.node];
        let to_node = &snarl[to.id.node];

        let from_type = from_node.output_type(from.id.output);
        let to_type = to_node.input_type(to.id.input);

        // Only allow connections between compatible signal types
        if from_type == to_type {
            // Disconnect existing connection to this input
            for remote_out in snarl.in_pin(to.id).remotes.iter().copied().collect::<Vec<_>>() {
                snarl.disconnect(remote_out, to.id);
            }
            // Create new connection
            snarl.connect(from.id, to.id);
        }
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<AudioNode>) -> bool {
        false // We use the palette instead
    }

    fn has_node_menu(&mut self, _node: &AudioNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<AudioNode>,
    ) {
        if ui.button("Remove").clicked() {
            snarl.remove_node(node);
            ui.close_menu();
        }
    }
}

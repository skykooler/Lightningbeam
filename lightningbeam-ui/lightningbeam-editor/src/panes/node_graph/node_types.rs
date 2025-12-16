//! Node Type Registry
//!
//! Defines metadata for all available node types

use eframe::egui;
use std::collections::HashMap;

/// Signal type for connections (matches daw_backend::SignalType)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataType {
    Audio,
    Midi,
    CV,
}

impl DataType {
    /// Get the color for this signal type
    pub fn color(&self) -> egui::Color32 {
        match self {
            DataType::Audio => egui::Color32::from_rgb(33, 150, 243), // Blue (#2196F3)
            DataType::Midi => egui::Color32::from_rgb(76, 175, 80),   // Green (#4CAF50)
            DataType::CV => egui::Color32::from_rgb(255, 152, 0),     // Orange (#FF9800)
        }
    }
}

/// Node category for organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeCategory {
    Inputs,
    Generators,
    Effects,
    Utilities,
    Outputs,
}

impl NodeCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            NodeCategory::Inputs => "Inputs",
            NodeCategory::Generators => "Generators",
            NodeCategory::Effects => "Effects",
            NodeCategory::Utilities => "Utilities",
            NodeCategory::Outputs => "Outputs",
        }
    }
}

/// Port information
#[derive(Debug, Clone)]
pub struct PortInfo {
    pub index: usize,
    pub name: String,
    pub signal_type: DataType,
    pub description: String,
}

/// Parameter units
#[derive(Debug, Clone, Copy)]
pub enum ParameterUnit {
    Hz,
    Percent,
    Decibels,
    Seconds,
    Milliseconds,
    Semitones,
    None,
}

impl ParameterUnit {
    pub fn suffix(&self) -> &'static str {
        match self {
            ParameterUnit::Hz => " Hz",
            ParameterUnit::Percent => "%",
            ParameterUnit::Decibels => " dB",
            ParameterUnit::Seconds => " s",
            ParameterUnit::Milliseconds => " ms",
            ParameterUnit::Semitones => " st",
            ParameterUnit::None => "",
        }
    }
}

/// Parameter information
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: String,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub unit: ParameterUnit,
    pub description: String,
}

/// Node type metadata
#[derive(Debug, Clone)]
pub struct NodeTypeInfo {
    pub id: String,
    pub display_name: String,
    pub category: NodeCategory,
    pub inputs: Vec<PortInfo>,
    pub outputs: Vec<PortInfo>,
    pub parameters: Vec<ParameterInfo>,
    pub description: String,
}

/// Registry of all available node types
pub struct NodeTypeRegistry {
    types: HashMap<String, NodeTypeInfo>,
}

impl NodeTypeRegistry {
    pub fn new() -> Self {
        let mut types = HashMap::new();

        // === INPUTS ===

        types.insert(
            "MidiInput".to_string(),
            NodeTypeInfo {
                id: "MidiInput".to_string(),
                display_name: "MIDI Input".to_string(),
                category: NodeCategory::Inputs,
                inputs: vec![],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "MIDI".to_string(),
                    signal_type: DataType::Midi,
                    description: "MIDI output from connected device".to_string(),
                }],
                parameters: vec![],
                description: "Receives MIDI from connected input devices".to_string(),
            },
        );

        types.insert(
            "AudioInput".to_string(),
            NodeTypeInfo {
                id: "AudioInput".to_string(),
                display_name: "Audio Input".to_string(),
                category: NodeCategory::Inputs,
                inputs: vec![],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Audio".to_string(),
                    signal_type: DataType::Audio,
                    description: "Audio from microphone/line input".to_string(),
                }],
                parameters: vec![],
                description: "Receives audio from connected input devices".to_string(),
            },
        );

        // === GENERATORS ===

        types.insert(
            "Oscillator".to_string(),
            NodeTypeInfo {
                id: "Oscillator".to_string(),
                display_name: "Oscillator".to_string(),
                category: NodeCategory::Generators,
                inputs: vec![
                    PortInfo {
                        index: 0,
                        name: "Freq".to_string(),
                        signal_type: DataType::CV,
                        description: "Frequency control (V/Oct)".to_string(),
                    },
                    PortInfo {
                        index: 1,
                        name: "Sync".to_string(),
                        signal_type: DataType::CV,
                        description: "Hard sync input".to_string(),
                    },
                ],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::Audio,
                    description: "Audio output".to_string(),
                }],
                parameters: vec![
                    ParameterInfo {
                        id: 0,
                        name: "Frequency".to_string(),
                        default: 440.0,
                        min: 20.0,
                        max: 20000.0,
                        unit: ParameterUnit::Hz,
                        description: "Base frequency".to_string(),
                    },
                    ParameterInfo {
                        id: 1,
                        name: "Waveform".to_string(),
                        default: 0.0,
                        min: 0.0,
                        max: 3.0,
                        unit: ParameterUnit::None,
                        description: "0=Sine, 1=Saw, 2=Square, 3=Triangle".to_string(),
                    },
                ],
                description: "Basic oscillator with multiple waveforms".to_string(),
            },
        );

        types.insert(
            "Noise".to_string(),
            NodeTypeInfo {
                id: "Noise".to_string(),
                display_name: "Noise".to_string(),
                category: NodeCategory::Generators,
                inputs: vec![],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::Audio,
                    description: "Noise output".to_string(),
                }],
                parameters: vec![ParameterInfo {
                    id: 0,
                    name: "Color".to_string(),
                    default: 0.0,
                    min: 0.0,
                    max: 2.0,
                    unit: ParameterUnit::None,
                    description: "0=White, 1=Pink, 2=Brown".to_string(),
                }],
                description: "Noise generator (white, pink, brown)".to_string(),
            },
        );

        // === EFFECTS ===

        types.insert(
            "Gain".to_string(),
            NodeTypeInfo {
                id: "Gain".to_string(),
                display_name: "Gain".to_string(),
                category: NodeCategory::Effects,
                inputs: vec![
                    PortInfo {
                        index: 0,
                        name: "In".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio input".to_string(),
                    },
                    PortInfo {
                        index: 1,
                        name: "Gain".to_string(),
                        signal_type: DataType::CV,
                        description: "Gain control CV".to_string(),
                    },
                ],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::Audio,
                    description: "Gained audio output".to_string(),
                }],
                parameters: vec![ParameterInfo {
                    id: 0,
                    name: "Gain".to_string(),
                    default: 0.0,
                    min: -60.0,
                    max: 12.0,
                    unit: ParameterUnit::Decibels,
                    description: "Gain amount in dB".to_string(),
                }],
                description: "Amplifies or attenuates audio signal".to_string(),
            },
        );

        types.insert(
            "Filter".to_string(),
            NodeTypeInfo {
                id: "Filter".to_string(),
                display_name: "Filter".to_string(),
                category: NodeCategory::Effects,
                inputs: vec![
                    PortInfo {
                        index: 0,
                        name: "In".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio input".to_string(),
                    },
                    PortInfo {
                        index: 1,
                        name: "Cutoff".to_string(),
                        signal_type: DataType::CV,
                        description: "Cutoff frequency CV".to_string(),
                    },
                ],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::Audio,
                    description: "Filtered audio output".to_string(),
                }],
                parameters: vec![
                    ParameterInfo {
                        id: 0,
                        name: "Cutoff".to_string(),
                        default: 1000.0,
                        min: 20.0,
                        max: 20000.0,
                        unit: ParameterUnit::Hz,
                        description: "Cutoff frequency".to_string(),
                    },
                    ParameterInfo {
                        id: 1,
                        name: "Resonance".to_string(),
                        default: 0.0,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Filter resonance".to_string(),
                    },
                    ParameterInfo {
                        id: 2,
                        name: "Type".to_string(),
                        default: 0.0,
                        min: 0.0,
                        max: 3.0,
                        unit: ParameterUnit::None,
                        description: "0=LPF, 1=HPF, 2=BPF, 3=Notch".to_string(),
                    },
                ],
                description: "Multi-mode filter (lowpass, highpass, bandpass, notch)".to_string(),
            },
        );

        types.insert(
            "Delay".to_string(),
            NodeTypeInfo {
                id: "Delay".to_string(),
                display_name: "Delay".to_string(),
                category: NodeCategory::Effects,
                inputs: vec![PortInfo {
                    index: 0,
                    name: "In".to_string(),
                    signal_type: DataType::Audio,
                    description: "Audio input".to_string(),
                }],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::Audio,
                    description: "Delayed audio output".to_string(),
                }],
                parameters: vec![
                    ParameterInfo {
                        id: 0,
                        name: "Time".to_string(),
                        default: 250.0,
                        min: 1.0,
                        max: 2000.0,
                        unit: ParameterUnit::Milliseconds,
                        description: "Delay time".to_string(),
                    },
                    ParameterInfo {
                        id: 1,
                        name: "Feedback".to_string(),
                        default: 0.3,
                        min: 0.0,
                        max: 0.95,
                        unit: ParameterUnit::None,
                        description: "Feedback amount".to_string(),
                    },
                    ParameterInfo {
                        id: 2,
                        name: "Mix".to_string(),
                        default: 0.5,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Dry/wet mix".to_string(),
                    },
                ],
                description: "Time-based delay effect".to_string(),
            },
        );

        // === UTILITIES ===

        types.insert(
            "ADSR".to_string(),
            NodeTypeInfo {
                id: "ADSR".to_string(),
                display_name: "ADSR".to_string(),
                category: NodeCategory::Utilities,
                inputs: vec![PortInfo {
                    index: 0,
                    name: "Gate".to_string(),
                    signal_type: DataType::CV,
                    description: "Gate input (triggers envelope)".to_string(),
                }],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::CV,
                    description: "Envelope CV output (0-1)".to_string(),
                }],
                parameters: vec![
                    ParameterInfo {
                        id: 0,
                        name: "Attack".to_string(),
                        default: 10.0,
                        min: 0.1,
                        max: 2000.0,
                        unit: ParameterUnit::Milliseconds,
                        description: "Attack time".to_string(),
                    },
                    ParameterInfo {
                        id: 1,
                        name: "Decay".to_string(),
                        default: 100.0,
                        min: 0.1,
                        max: 2000.0,
                        unit: ParameterUnit::Milliseconds,
                        description: "Decay time".to_string(),
                    },
                    ParameterInfo {
                        id: 2,
                        name: "Sustain".to_string(),
                        default: 0.7,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Sustain level".to_string(),
                    },
                    ParameterInfo {
                        id: 3,
                        name: "Release".to_string(),
                        default: 200.0,
                        min: 0.1,
                        max: 5000.0,
                        unit: ParameterUnit::Milliseconds,
                        description: "Release time".to_string(),
                    },
                ],
                description: "ADSR envelope generator".to_string(),
            },
        );

        types.insert(
            "LFO".to_string(),
            NodeTypeInfo {
                id: "LFO".to_string(),
                display_name: "LFO".to_string(),
                category: NodeCategory::Utilities,
                inputs: vec![],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::CV,
                    description: "LFO CV output".to_string(),
                }],
                parameters: vec![
                    ParameterInfo {
                        id: 0,
                        name: "Rate".to_string(),
                        default: 1.0,
                        min: 0.01,
                        max: 20.0,
                        unit: ParameterUnit::Hz,
                        description: "LFO rate".to_string(),
                    },
                    ParameterInfo {
                        id: 1,
                        name: "Waveform".to_string(),
                        default: 0.0,
                        min: 0.0,
                        max: 3.0,
                        unit: ParameterUnit::None,
                        description: "0=Sine, 1=Triangle, 2=Square, 3=Saw".to_string(),
                    },
                ],
                description: "Low-frequency oscillator for modulation".to_string(),
            },
        );

        types.insert(
            "Mixer".to_string(),
            NodeTypeInfo {
                id: "Mixer".to_string(),
                display_name: "Mixer".to_string(),
                category: NodeCategory::Utilities,
                inputs: vec![
                    PortInfo {
                        index: 0,
                        name: "In 1".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio input 1".to_string(),
                    },
                    PortInfo {
                        index: 1,
                        name: "In 2".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio input 2".to_string(),
                    },
                    PortInfo {
                        index: 2,
                        name: "In 3".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio input 3".to_string(),
                    },
                    PortInfo {
                        index: 3,
                        name: "In 4".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio input 4".to_string(),
                    },
                ],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::Audio,
                    description: "Mixed audio output".to_string(),
                }],
                parameters: vec![
                    ParameterInfo {
                        id: 0,
                        name: "Level 1".to_string(),
                        default: 1.0,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Input 1 level".to_string(),
                    },
                    ParameterInfo {
                        id: 1,
                        name: "Level 2".to_string(),
                        default: 1.0,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Input 2 level".to_string(),
                    },
                    ParameterInfo {
                        id: 2,
                        name: "Level 3".to_string(),
                        default: 1.0,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Input 3 level".to_string(),
                    },
                    ParameterInfo {
                        id: 3,
                        name: "Level 4".to_string(),
                        default: 1.0,
                        min: 0.0,
                        max: 1.0,
                        unit: ParameterUnit::None,
                        description: "Input 4 level".to_string(),
                    },
                ],
                description: "4-channel audio mixer".to_string(),
            },
        );

        types.insert(
            "Splitter".to_string(),
            NodeTypeInfo {
                id: "Splitter".to_string(),
                display_name: "Splitter".to_string(),
                category: NodeCategory::Utilities,
                inputs: vec![PortInfo {
                    index: 0,
                    name: "In".to_string(),
                    signal_type: DataType::Audio,
                    description: "Audio input".to_string(),
                }],
                outputs: vec![
                    PortInfo {
                        index: 0,
                        name: "Out 1".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio output 1".to_string(),
                    },
                    PortInfo {
                        index: 1,
                        name: "Out 2".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio output 2".to_string(),
                    },
                    PortInfo {
                        index: 2,
                        name: "Out 3".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio output 3".to_string(),
                    },
                    PortInfo {
                        index: 3,
                        name: "Out 4".to_string(),
                        signal_type: DataType::Audio,
                        description: "Audio output 4".to_string(),
                    },
                ],
                parameters: vec![],
                description: "Splits one audio signal into four outputs".to_string(),
            },
        );

        types.insert(
            "Constant".to_string(),
            NodeTypeInfo {
                id: "Constant".to_string(),
                display_name: "Constant".to_string(),
                category: NodeCategory::Utilities,
                inputs: vec![],
                outputs: vec![PortInfo {
                    index: 0,
                    name: "Out".to_string(),
                    signal_type: DataType::CV,
                    description: "Constant CV output".to_string(),
                }],
                parameters: vec![ParameterInfo {
                    id: 0,
                    name: "Value".to_string(),
                    default: 0.0,
                    min: -1.0,
                    max: 1.0,
                    unit: ParameterUnit::None,
                    description: "Constant value".to_string(),
                }],
                description: "Outputs a constant CV value".to_string(),
            },
        );

        // === OUTPUTS ===

        types.insert(
            "AudioOutput".to_string(),
            NodeTypeInfo {
                id: "AudioOutput".to_string(),
                display_name: "Audio Output".to_string(),
                category: NodeCategory::Outputs,
                inputs: vec![
                    PortInfo {
                        index: 0,
                        name: "Left".to_string(),
                        signal_type: DataType::Audio,
                        description: "Left channel input".to_string(),
                    },
                    PortInfo {
                        index: 1,
                        name: "Right".to_string(),
                        signal_type: DataType::Audio,
                        description: "Right channel input".to_string(),
                    },
                ],
                outputs: vec![],
                parameters: vec![],
                description: "Sends audio to the track output".to_string(),
            },
        );

        Self { types }
    }

    pub fn get(&self, node_type: &str) -> Option<&NodeTypeInfo> {
        self.types.get(node_type)
    }

    pub fn get_by_category(&self, category: NodeCategory) -> Vec<&NodeTypeInfo> {
        self.types
            .values()
            .filter(|info| info.category == category)
            .collect()
    }

    pub fn all_categories(&self) -> Vec<NodeCategory> {
        vec![
            NodeCategory::Inputs,
            NodeCategory::Generators,
            NodeCategory::Effects,
            NodeCategory::Utilities,
            NodeCategory::Outputs,
        ]
    }
}

impl Default for NodeTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

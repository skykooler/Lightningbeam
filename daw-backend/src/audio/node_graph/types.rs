use serde::{Deserialize, Serialize};

/// Three distinct signal types for graph edges
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalType {
    /// Audio-rate signals (-1.0 to 1.0 typically) - Blue in UI
    Audio,
    /// MIDI events (discrete messages) - Green in UI
    Midi,
    /// Control Voltage (modulation signals, typically 0.0 to 1.0) - Orange in UI
    CV,
}

/// Port definition for node inputs/outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePort {
    pub name: String,
    pub signal_type: SignalType,
    pub index: usize,
}

impl NodePort {
    pub fn new(name: impl Into<String>, signal_type: SignalType, index: usize) -> Self {
        Self {
            name: name.into(),
            signal_type,
            index,
        }
    }
}

/// Node category for UI organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeCategory {
    Input,
    Generator,
    Effect,
    Utility,
    Output,
}

/// User-facing parameter definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub id: u32,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: ParameterUnit,
}

impl Parameter {
    pub fn new(id: u32, name: impl Into<String>, min: f32, max: f32, default: f32, unit: ParameterUnit) -> Self {
        Self {
            id,
            name: name.into(),
            min,
            max,
            default,
            unit,
        }
    }
}

/// Units for parameter values
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParameterUnit {
    Generic,
    Frequency,  // Hz
    Decibels,   // dB
    Time,       // seconds
    Percent,    // 0-100
}

/// Errors that can occur during graph operations
#[derive(Debug, Clone)]
pub enum ConnectionError {
    TypeMismatch { expected: SignalType, got: SignalType },
    InvalidPort,
    WouldCreateCycle,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionError::TypeMismatch { expected, got } => {
                write!(f, "Signal type mismatch: expected {:?}, got {:?}", expected, got)
            }
            ConnectionError::InvalidPort => write!(f, "Invalid port index"),
            ConnectionError::WouldCreateCycle => write!(f, "Connection would create a cycle"),
        }
    }
}

impl std::error::Error for ConnectionError {}

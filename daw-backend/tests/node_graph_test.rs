use daw_backend::audio::node_graph::{
    nodes::{AudioOutputNode, GainNode, OscillatorNode},
    ConnectionError, InstrumentGraph, SignalType,
};

#[test]
fn test_basic_node_graph() {
    // Create a graph with sample rate 44100 and buffer size 512
    let mut graph = InstrumentGraph::new(44100, 512);

    // Create nodes
    let osc = Box::new(OscillatorNode::new("Oscillator"));
    let gain = Box::new(GainNode::new("Gain"));
    let output = Box::new(AudioOutputNode::new("Output"));

    // Add nodes to graph
    let osc_idx = graph.add_node(osc);
    let gain_idx = graph.add_node(gain);
    let output_idx = graph.add_node(output);

    // Connect: Oscillator -> Gain -> Output
    assert!(graph.connect(osc_idx, 0, gain_idx, 0).is_ok());
    assert!(graph.connect(gain_idx, 0, output_idx, 0).is_ok());

    // Set output node
    graph.set_output_node(Some(output_idx));

    // Set oscillator frequency to 440 Hz
    if let Some(node) = graph.get_graph_node_mut(osc_idx) {
        node.node.set_parameter(0, 440.0); // Frequency parameter
    }

    // Process a buffer
    let mut output_buffer = vec![0.0f32; 512];
    graph.process(&mut output_buffer, &[]);

    // Check that we got some audio output (oscillator should produce non-zero samples)
    let has_output = output_buffer.iter().any(|&s| s != 0.0);
    assert!(has_output, "Expected non-zero audio output from oscillator");

    // Check that output is within reasonable bounds
    let max_amplitude = output_buffer.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(max_amplitude <= 1.0, "Output amplitude too high: {}", max_amplitude);
}

#[test]
fn test_connection_type_validation() {

    let mut graph = InstrumentGraph::new(44100, 512);

    let osc = Box::new(OscillatorNode::new("Oscillator"));
    let output = Box::new(AudioOutputNode::new("Output"));

    let osc_idx = graph.add_node(osc);
    let output_idx = graph.add_node(output);

    // This should work (Audio -> Audio)
    let result = graph.connect(osc_idx, 0, output_idx, 0);
    assert!(result.is_ok());

    // Try to connect CV to Audio - should fail
    // Oscillator CV input (port 0 - wait, actually oscillator has CV input)
    // Let me create a more clear test:
    let osc2 = Box::new(OscillatorNode::new("Oscillator2"));
    let osc2_idx = graph.add_node(osc2);

    // Try to connect audio output to CV input
    // This would be caught if we had different signal types
    // For now, just verify the connection succeeds with matching types
    let result = graph.connect(osc_idx, 0, osc2_idx, 0);
    // This should actually fail because audio output can't connect to CV input
    assert!(result.is_err());

    match result {
        Err(ConnectionError::TypeMismatch { expected, got }) => {
            assert_eq!(expected, SignalType::CV);
            assert_eq!(got, SignalType::Audio);
        }
        _ => panic!("Expected TypeMismatch error"),
    }
}

#[test]
fn test_cycle_detection() {
    let mut graph = InstrumentGraph::new(44100, 512);

    let gain1 = Box::new(GainNode::new("Gain1"));
    let gain2 = Box::new(GainNode::new("Gain2"));
    let gain3 = Box::new(GainNode::new("Gain3"));

    let g1 = graph.add_node(gain1);
    let g2 = graph.add_node(gain2);
    let g3 = graph.add_node(gain3);

    // Create a chain: g1 -> g2 -> g3
    assert!(graph.connect(g1, 0, g2, 0).is_ok());
    assert!(graph.connect(g2, 0, g3, 0).is_ok());

    // Try to create a cycle: g3 -> g1
    let result = graph.connect(g3, 0, g1, 0);
    assert!(result.is_err());

    match result {
        Err(ConnectionError::WouldCreateCycle) => {
            // Expected!
        }
        _ => panic!("Expected WouldCreateCycle error"),
    }
}

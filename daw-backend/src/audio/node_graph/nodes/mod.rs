mod amp_sim;
pub mod bundled_models;
mod adsr;
mod subtrack_inputs;
mod arpeggiator;
mod audio_input;
mod audio_to_cv;
mod automation_input;
mod beat;
mod bit_crusher;
mod bpm_detector;
mod chorus;
mod compressor;
mod constant;
mod echo;
mod distortion;
mod envelope_follower;
mod eq;
mod filter;
mod flanger;
mod limiter;
mod fm_synth;
mod gain;
mod lfo;
mod math;
mod midi_input;
mod midi_to_cv;
mod mixer;
mod multi_sampler;
mod noise;
mod oscillator;
mod oscilloscope;
mod output;
mod pan;
mod phaser;
mod quantizer;
mod reverb;
mod ring_modulator;
mod sample_hold;
mod script_node;
mod sequencer;
mod simple_sampler;
mod slew_limiter;
mod splitter;
mod svf;
mod template_io;
mod vibrato;
mod vocoder;
mod voice_allocator;
mod wavetable_oscillator;

pub use amp_sim::AmpSimNode;
pub use adsr::ADSRNode;
pub use arpeggiator::ArpeggiatorNode;
pub use audio_input::AudioInputNode;
pub use audio_to_cv::AudioToCVNode;
pub use automation_input::{AutomationInputNode, AutomationKeyframe, InterpolationType};
pub use beat::BeatNode;
pub use bit_crusher::BitCrusherNode;
pub use bpm_detector::BpmDetectorNode;
pub use chorus::ChorusNode;
pub use compressor::CompressorNode;
pub use constant::ConstantNode;
pub use echo::EchoNode;
pub use distortion::DistortionNode;
pub use envelope_follower::EnvelopeFollowerNode;
pub use eq::EQNode;
pub use filter::FilterNode;
pub use flanger::FlangerNode;
pub use limiter::LimiterNode;
pub use fm_synth::FMSynthNode;
pub use gain::GainNode;
pub use lfo::LFONode;
pub use math::MathNode;
pub use midi_input::MidiInputNode;
pub use midi_to_cv::MidiToCVNode;
pub use mixer::MixerNode;
pub use multi_sampler::{MultiSamplerNode, LoopMode};
pub use noise::NoiseGeneratorNode;
pub use oscillator::OscillatorNode;
pub use oscilloscope::OscilloscopeNode;
pub use output::AudioOutputNode;
pub use pan::PanNode;
pub use phaser::PhaserNode;
pub use quantizer::QuantizerNode;
pub use reverb::ReverbNode;
pub use ring_modulator::RingModulatorNode;
pub use sample_hold::SampleHoldNode;
pub use script_node::ScriptNode;
pub use sequencer::SequencerNode;
pub use simple_sampler::SimpleSamplerNode;
pub use slew_limiter::SlewLimiterNode;
pub use splitter::SplitterNode;
pub use svf::SVFNode;
pub use template_io::{TemplateInputNode, TemplateOutputNode};
pub use vibrato::VibratoNode;
pub use vocoder::VocoderNode;
pub use voice_allocator::VoiceAllocatorNode;
pub use wavetable_oscillator::WavetableOscillatorNode;
pub use subtrack_inputs::SubtrackInputsNode;

/// Create a node instance by type name string.
///
/// Returns `None` for unknown type names. `sample_rate` and `buffer_size`
/// are only used by VoiceAllocator; other nodes ignore them.
pub fn create_node(node_type: &str, sample_rate: u32, buffer_size: usize) -> Option<Box<dyn super::AudioNode>> {
    Some(match node_type {
        "Oscillator" => Box::new(OscillatorNode::new("Oscillator")),
        "Gain" => Box::new(GainNode::new("Gain")),
        "Mixer" => Box::new(MixerNode::new("Mixer")),
        "Filter" => Box::new(FilterNode::new("Filter")),
        "SVF" => Box::new(SVFNode::new("SVF")),
        "ADSR" => Box::new(ADSRNode::new("ADSR")),
        "LFO" => Box::new(LFONode::new("LFO")),
        "NoiseGenerator" => Box::new(NoiseGeneratorNode::new("Noise")),
        "Splitter" => Box::new(SplitterNode::new("Splitter")),
        "Pan" => Box::new(PanNode::new("Pan")),
        "Quantizer" => Box::new(QuantizerNode::new("Quantizer")),
        "Echo" | "Delay" => Box::new(EchoNode::new("Echo")),
        "Distortion" => Box::new(DistortionNode::new("Distortion")),
        "Reverb" => Box::new(ReverbNode::new("Reverb")),
        "Chorus" => Box::new(ChorusNode::new("Chorus")),
        "Compressor" => Box::new(CompressorNode::new("Compressor")),
        "Constant" => Box::new(ConstantNode::new("Constant")),
        "BpmDetector" => Box::new(BpmDetectorNode::new("BPM Detector")),
        "Beat" => Box::new(BeatNode::new("Beat")),
        "Arpeggiator" => Box::new(ArpeggiatorNode::new("Arpeggiator")),
        "Sequencer" => Box::new(SequencerNode::new("Sequencer")),
        "Script" => Box::new(ScriptNode::new("Script")),
        "EnvelopeFollower" => Box::new(EnvelopeFollowerNode::new("Envelope Follower")),
        "Limiter" => Box::new(LimiterNode::new("Limiter")),
        "Math" => Box::new(MathNode::new("Math")),
        "EQ" => Box::new(EQNode::new("EQ")),
        "Flanger" => Box::new(FlangerNode::new("Flanger")),
        "FMSynth" => Box::new(FMSynthNode::new("FM Synth")),
        "Phaser" => Box::new(PhaserNode::new("Phaser")),
        "BitCrusher" => Box::new(BitCrusherNode::new("Bit Crusher")),
        "Vocoder" => Box::new(VocoderNode::new("Vocoder")),
        "RingModulator" => Box::new(RingModulatorNode::new("Ring Modulator")),
        "SampleHold" => Box::new(SampleHoldNode::new("Sample & Hold")),
        "WavetableOscillator" => Box::new(WavetableOscillatorNode::new("Wavetable")),
        "SimpleSampler" => Box::new(SimpleSamplerNode::new("Sampler")),
        "SlewLimiter" => Box::new(SlewLimiterNode::new("Slew Limiter")),
        "MultiSampler" => Box::new(MultiSamplerNode::new("Multi Sampler")),
        "MidiInput" => Box::new(MidiInputNode::new("MIDI Input")),
        "MidiToCV" => Box::new(MidiToCVNode::new("MIDI→CV")),
        "AudioToCV" => Box::new(AudioToCVNode::new("Audio→CV")),
        "AudioInput" => Box::new(AudioInputNode::new("Audio Input")),
        "AutomationInput" => Box::new(AutomationInputNode::new("Automation")),
        "Oscilloscope" => Box::new(OscilloscopeNode::new("Oscilloscope")),
        "TemplateInput" => Box::new(TemplateInputNode::new("Template Input")),
        "TemplateOutput" => Box::new(TemplateOutputNode::new("Template Output")),
        "VoiceAllocator" => Box::new(VoiceAllocatorNode::new("VoiceAllocator", sample_rate, buffer_size)),
        "Vibrato" => Box::new(VibratoNode::new("Vibrato")),
        "AmpSim" => Box::new(AmpSimNode::new("Amp Sim")),
        "AudioOutput" => Box::new(AudioOutputNode::new("Output")),
        "SubtrackInputs" => Box::new(SubtrackInputsNode::new("Subtrack Inputs", vec![])),
        _ => return None,
    })
}

// Node type definitions for the audio node graph editor
// Each node type defines its inputs, outputs, parameters, and HTML template

/**
 * Signal types for node ports
 * These match the backend SignalType enum
 */
export const SignalType = {
  AUDIO: 'audio',  // Blue circles
  MIDI: 'midi',    // Green squares
  CV: 'cv'         // Orange diamonds
};

/**
 * Node category for organization in the palette
 */
export const NodeCategory = {
  INPUT: 'input',
  GENERATOR: 'generator',
  EFFECT: 'effect',
  UTILITY: 'utility',
  OUTPUT: 'output'
};

/**
 * Get CSS class for a port based on its signal type
 */
export function getPortClass(signalType) {
  return `connector-${signalType}`;
}

/**
 * Node type catalog
 * Maps node type names to their definitions
 */
export const nodeTypes = {
  Oscillator: {
    name: 'Oscillator',
    category: NodeCategory.GENERATOR,
    description: 'Oscillator with multiple waveforms and CV modulation',
    inputs: [
      { name: 'V/Oct', type: SignalType.CV, index: 0 },
      { name: 'FM', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'Audio', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'frequency', label: 'Frequency', min: 20, max: 20000, default: 440, unit: 'Hz' },
      { id: 1, name: 'amplitude', label: 'Amplitude', min: 0, max: 1, default: 0.5, unit: '' },
      { id: 2, name: 'waveform', label: 'Waveform', min: 0, max: 3, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Oscillator</div>
        <div class="node-param">
          <label>Waveform: <span id="wave-${nodeId}">Sine</span></label>
          <input type="range"
                
                 data-node="${nodeId}"
                 data-param="2"
                 min="0"
                 max="3"
                 value="0"
                 step="1">
        </div>
        <div class="node-param">
          <label>Frequency: <span id="freq-${nodeId}">440</span> Hz</label>
          <input type="range"
                
                 data-node="${nodeId}"
                 data-param="0"
                 min="20"
                 max="20000"
                 value="440"
                 step="1">
        </div>
        <div class="node-param">
          <label>Amplitude: <span id="amp-${nodeId}">0.5</span></label>
          <input type="range"
                
                 data-node="${nodeId}"
                 data-param="1"
                 min="0"
                 max="1"
                 value="0.5"
                 step="0.01">
        </div>
      </div>
    `
  },

  Gain: {
    name: 'Gain',
    category: NodeCategory.UTILITY,
    description: 'VCA (voltage-controlled amplifier) - CV multiplies gain',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 },
      { name: 'CV', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'gain', label: 'Gain', min: 0, max: 2, default: 1, unit: 'x' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Gain</div>
        <div class="node-param">
          <label>Gain: <span id="gain-${nodeId}">1.0</span>x</label>
          <input type="range"
                
                 data-node="${nodeId}"
                 data-param="0"
                 min="0"
                 max="2"
                 value="1"
                 step="0.01">
        </div>
      </div>
    `
  },

  Mixer: {
    name: 'Mixer',
    category: NodeCategory.UTILITY,
    description: 'Mix up to 4 audio inputs with independent gain controls',
    inputs: [
      { name: 'Input 1', type: SignalType.AUDIO, index: 0 },
      { name: 'Input 2', type: SignalType.AUDIO, index: 1 },
      { name: 'Input 3', type: SignalType.AUDIO, index: 2 },
      { name: 'Input 4', type: SignalType.AUDIO, index: 3 }
    ],
    outputs: [
      { name: 'Mixed Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'gain1', label: 'Gain 1', min: 0, max: 2, default: 1, unit: 'x' },
      { id: 1, name: 'gain2', label: 'Gain 2', min: 0, max: 2, default: 1, unit: 'x' },
      { id: 2, name: 'gain3', label: 'Gain 3', min: 0, max: 2, default: 1, unit: 'x' },
      { id: 3, name: 'gain4', label: 'Gain 4', min: 0, max: 2, default: 1, unit: 'x' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Mixer</div>
        <div class="node-param">
          <label>Gain 1: <span id="g1-${nodeId}">1.0</span>x</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0" max="2" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Gain 2: <span id="g2-${nodeId}">1.0</span>x</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="2" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Gain 3: <span id="g3-${nodeId}">1.0</span>x</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="2" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Gain 4: <span id="g4-${nodeId}">1.0</span>x</label>
          <input type="range" data-node="${nodeId}" data-param="3" min="0" max="2" value="1" step="0.01">
        </div>
      </div>
    `
  },

  Filter: {
    name: 'Filter',
    category: NodeCategory.EFFECT,
    description: 'Biquad filter with lowpass/highpass modes',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 },
      { name: 'Cutoff CV', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'cutoff', label: 'Cutoff', min: 20, max: 20000, default: 1000, unit: 'Hz' },
      { id: 1, name: 'resonance', label: 'Resonance', min: 0.1, max: 10, default: 0.707, unit: 'Q' },
      { id: 2, name: 'type', label: 'Type', min: 0, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Filter</div>
        <div class="node-param">
          <label>Cutoff: <span id="cutoff-${nodeId}">1000</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="20" max="20000" value="1000" step="1">
        </div>
        <div class="node-param">
          <label>Resonance: <span id="res-${nodeId}">0.707</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.1" max="10" value="0.707" step="0.01">
        </div>
        <div class="node-param">
          <label>Type: <span id="ftype-${nodeId}">LP</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0" step="1">
        </div>
      </div>
    `
  },

  ADSR: {
    name: 'ADSR',
    category: NodeCategory.UTILITY,
    description: 'Attack-Decay-Sustain-Release envelope generator',
    inputs: [
      { name: 'Gate', type: SignalType.CV, index: 0 }
    ],
    outputs: [
      { name: 'Envelope', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'attack', label: 'Attack', min: 0.001, max: 5, default: 0.01, unit: 's' },
      { id: 1, name: 'decay', label: 'Decay', min: 0.001, max: 5, default: 0.1, unit: 's' },
      { id: 2, name: 'sustain', label: 'Sustain', min: 0, max: 1, default: 0.7, unit: '' },
      { id: 3, name: 'release', label: 'Release', min: 0.001, max: 5, default: 0.2, unit: 's' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">ADSR</div>
        <div class="node-param">
          <label>A: <span id="a-${nodeId}">0.01</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.001" max="5" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>D: <span id="d-${nodeId}">0.1</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.001" max="5" value="0.1" step="0.001">
        </div>
        <div class="node-param">
          <label>S: <span id="s-${nodeId}">0.7</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0.7" step="0.01">
        </div>
        <div class="node-param">
          <label>R: <span id="r-${nodeId}">0.2</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="3" min="0.001" max="5" value="0.2" step="0.001">
        </div>
      </div>
    `
  },

  MidiInput: {
    name: 'MidiInput',
    category: NodeCategory.INPUT,
    description: 'MIDI input - receives MIDI events from track',
    inputs: [],
    outputs: [
      { name: 'MIDI Out', type: SignalType.MIDI, index: 0 }
    ],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">MIDI Input</div>
        <div class="node-info">Receives MIDI from track</div>
      </div>
    `
  },

  MidiToCV: {
    name: 'MidiToCV',
    category: NodeCategory.UTILITY,
    description: 'Convert MIDI notes to CV signals',
    inputs: [
      { name: 'MIDI In', type: SignalType.MIDI, index: 0 }
    ],
    outputs: [
      { name: 'V/Oct', type: SignalType.CV, index: 0 },
      { name: 'Gate', type: SignalType.CV, index: 1 },
      { name: 'Velocity', type: SignalType.CV, index: 2 }
    ],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">MIDI→CV</div>
        <div class="node-info">Converts MIDI to CV signals</div>
      </div>
    `
  },

  AudioToCV: {
    name: 'AudioToCV',
    category: NodeCategory.UTILITY,
    description: 'Envelope follower - converts audio amplitude to CV',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'attack', label: 'Attack', min: 0.001, max: 1.0, default: 0.01, unit: 's' },
      { id: 1, name: 'release', label: 'Release', min: 0.001, max: 1.0, default: 0.1, unit: 's' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Audio→CV</div>
        <div class="node-param">
          <label>Attack: <span id="att-${nodeId}">0.01</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.001" max="1.0" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>Release: <span id="rel-${nodeId}">0.1</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.001" max="1.0" value="0.1" step="0.001">
        </div>
      </div>
    `
  },

  Oscilloscope: {
    name: 'Oscilloscope',
    category: NodeCategory.UTILITY,
    description: 'Visual audio signal monitor (pass-through)',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 },
      { name: 'V/oct', type: SignalType.CV, index: 1 },
      { name: 'CV In', type: SignalType.CV, index: 2 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'time_scale', label: 'Time Scale', min: 10, max: 1000, default: 100, unit: 'ms' },
      { id: 1, name: 'trigger_mode', label: 'Trigger', min: 0, max: 3, default: 0, unit: '' },
      { id: 2, name: 'trigger_level', label: 'Trigger Level', min: -1, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Oscilloscope</div>
        <canvas id="oscilloscope-canvas-${nodeId}" width="200" height="80" style="width: 200px; height: 80px; background: #1a1a1a; border: 1px solid #444; border-radius: 2px; display: block; margin: 4px 0;"></canvas>
        <div class="node-param">
          <label>Time: <span id="time_scale-${nodeId}">100</span>ms</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="10" max="1000" value="100" step="10">
        </div>
        <div class="node-param">
          <label>Trigger: <span id="trigger_mode-${nodeId}">Free</span></label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="1" min="0" max="3" value="0" step="1">
        </div>
      </div>
    `
  },

  VoiceAllocator: {
    name: 'VoiceAllocator',
    category: NodeCategory.UTILITY,
    description: 'Polyphonic voice allocator - creates N instances of internal graph',
    inputs: [
      { name: 'MIDI In', type: SignalType.MIDI, index: 0 }
    ],
    outputs: [
      { name: 'Mixed Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'voices', label: 'Voices', min: 1, max: 16, default: 8, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="voice-allocator-header">
          <div class="node-title">Voice Allocator</div>
          <div class="node-param">
            <label>Voices: <span id="voices-${nodeId}">8</span></label>
            <input type="range" data-node="${nodeId}" data-param="0" min="1" max="16" value="8" step="1">
          </div>
          <div class="node-info" style="margin-top: 4px; font-size: 10px;">Double-click to edit</div>
        </div>
        <div class="voice-allocator-contents" id="voice-allocator-contents-${nodeId}" style="display: none;">
          <div class="node-info" style="font-size: 10px; color: #aaa; margin-bottom: 8px;">Build a synth voice template:</div>
          <div class="node-info" style="font-size: 9px; color: #c77dff;">Purple nodes are Template Input/Output (non-deletable)</div>
          <div class="node-info" style="font-size: 9px; color: #888;">Connect MIDI from Template Input → MidiToCV</div>
          <div class="node-info" style="font-size: 9px; color: #888;">Add synth nodes: Oscillator, ADSR, Gain, etc.</div>
          <div class="node-info" style="font-size: 9px; color: #888;">Connect final audio → Template Output</div>
          <div class="node-info" style="font-size: 9px; color: #666; margin-top: 8px;">Drag nodes from palette • Container auto-resizes</div>
        </div>
      </div>
    `
  },

  AudioOutput: {
    name: 'AudioOutput',
    category: NodeCategory.OUTPUT,
    description: 'Final audio output',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Audio Output</div>
        <div class="node-info">Final output to speakers</div>
      </div>
    `
  },

  TemplateInput: {
    name: 'TemplateInput',
    category: NodeCategory.INPUT,
    description: 'VoiceAllocator template input - receives MIDI for one voice',
    inputs: [],
    outputs: [
      { name: 'MIDI Out', type: SignalType.MIDI, index: 0 }
    ],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Template Input</div>
        <div class="node-info" style="font-size: 9px;">MIDI for one voice</div>
      </div>
    `
  },

  TemplateOutput: {
    name: 'TemplateOutput',
    category: NodeCategory.OUTPUT,
    description: 'VoiceAllocator template output - sends audio to voice mixer',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Template Output</div>
        <div class="node-info" style="font-size: 9px;">Audio to mixer</div>
      </div>
    `
  },

  LFO: {
    name: 'LFO',
    category: NodeCategory.UTILITY,
    description: 'Low frequency oscillator for modulation',
    inputs: [],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'frequency', label: 'Frequency', min: 0.01, max: 20, default: 1.0, unit: 'Hz' },
      { id: 1, name: 'amplitude', label: 'Amplitude', min: 0, max: 1, default: 1.0, unit: '' },
      { id: 2, name: 'waveform', label: 'Waveform', min: 0, max: 4, default: 0, unit: '' },
      { id: 3, name: 'phase', label: 'Phase', min: 0, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">LFO</div>
        <div class="node-param">
          <label>Wave: <span id="lfowave-${nodeId}">Sine</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="4" value="0" step="1">
        </div>
        <div class="node-param">
          <label>Freq: <span id="lfofreq-${nodeId}">1.0</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.01" max="20" value="1.0" step="0.01">
        </div>
        <div class="node-param">
          <label>Depth: <span id="lfoamp-${nodeId}">1.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="1" value="1.0" step="0.01">
        </div>
      </div>
    `
  },

  NoiseGenerator: {
    name: 'NoiseGenerator',
    category: NodeCategory.GENERATOR,
    description: 'White and pink noise generator',
    inputs: [],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'amplitude', label: 'Amplitude', min: 0, max: 1, default: 0.5, unit: '' },
      { id: 1, name: 'color', label: 'Color', min: 0, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Noise</div>
        <div class="node-param">
          <label>Color: <span id="noisecolor-${nodeId}">White</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="1" value="0" step="1">
        </div>
        <div class="node-param">
          <label>Level: <span id="noiselevel-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0" max="1" value="0.5" step="0.01">
        </div>
      </div>
    `
  },

  Splitter: {
    name: 'Splitter',
    category: NodeCategory.UTILITY,
    description: 'Split audio signal to multiple outputs for parallel routing',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Out 1', type: SignalType.AUDIO, index: 0 },
      { name: 'Out 2', type: SignalType.AUDIO, index: 1 },
      { name: 'Out 3', type: SignalType.AUDIO, index: 2 },
      { name: 'Out 4', type: SignalType.AUDIO, index: 3 }
    ],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Splitter</div>
        <div class="node-info" style="font-size: 10px;">1→4 split</div>
      </div>
    `
  },

  Pan: {
    name: 'Pan',
    category: NodeCategory.UTILITY,
    description: 'Stereo panning with CV modulation',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 },
      { name: 'Pan CV', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'pan', label: 'Pan', min: -1, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Pan</div>
        <div class="node-param">
          <label>Position: <span id="panpos-${nodeId}">0.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="-1" max="1" value="0" step="0.01">
        </div>
      </div>
    `
  },

  Delay: {
    name: 'Delay',
    category: NodeCategory.EFFECT,
    description: 'Stereo delay with feedback',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'delay_time', label: 'Delay Time', min: 0.001, max: 2.0, default: 0.5, unit: 's' },
      { id: 1, name: 'feedback', label: 'Feedback', min: 0, max: 0.95, default: 0.5, unit: '' },
      { id: 2, name: 'wet_dry', label: 'Wet/Dry', min: 0, max: 1, default: 0.5, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Delay</div>
        <div class="node-param">
          <label>Time: <span id="delaytime-${nodeId}">0.5</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.001" max="2" value="0.5" step="0.001">
        </div>
        <div class="node-param">
          <label>Feedback: <span id="feedback-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="0.95" value="0.5" step="0.01">
        </div>
        <div class="node-param">
          <label>Wet/Dry: <span id="wetdry-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0.5" step="0.01">
        </div>
      </div>
    `
  },

  Reverb: {
    name: 'Reverb',
    category: NodeCategory.EFFECT,
    description: 'Schroeder reverb with room size and damping',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'room_size', label: 'Room Size', min: 0, max: 1, default: 0.5, unit: '' },
      { id: 1, name: 'damping', label: 'Damping', min: 0, max: 1, default: 0.5, unit: '' },
      { id: 2, name: 'wet_dry', label: 'Wet/Dry', min: 0, max: 1, default: 0.3, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Reverb</div>
        <div class="node-param">
          <label>Room Size: <span id="roomsize-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0" max="1" value="0.5" step="0.01">
        </div>
        <div class="node-param">
          <label>Damping: <span id="damping-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="1" value="0.5" step="0.01">
        </div>
        <div class="node-param">
          <label>Wet/Dry: <span id="wetdry-${nodeId}">0.3</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0.3" step="0.01">
        </div>
      </div>
    `
  },

  Chorus: {
    name: 'Chorus',
    category: NodeCategory.EFFECT,
    description: 'Chorus effect with modulated delay',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'rate', label: 'Rate', min: 0.1, max: 5.0, default: 1.0, unit: 'Hz' },
      { id: 1, name: 'depth', label: 'Depth', min: 0, max: 1, default: 0.5, unit: '' },
      { id: 2, name: 'wet_dry', label: 'Wet/Dry', min: 0, max: 1, default: 0.5, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Chorus</div>
        <div class="node-param">
          <label>Rate: <span id="chorusrate-${nodeId}">1.0</span>Hz</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.1" max="5" value="1.0" step="0.1">
        </div>
        <div class="node-param">
          <label>Depth: <span id="chorusdepth-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="1" value="0.5" step="0.01">
        </div>
        <div class="node-param">
          <label>Wet/Dry: <span id="choruswetdry-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0.5" step="0.01">
        </div>
      </div>
    `
  },

  Flanger: {
    name: 'Flanger',
    category: NodeCategory.EFFECT,
    description: 'Flanger effect with feedback',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'rate', label: 'Rate', min: 0.1, max: 10.0, default: 0.5, unit: 'Hz' },
      { id: 1, name: 'depth', label: 'Depth', min: 0, max: 1, default: 0.7, unit: '' },
      { id: 2, name: 'feedback', label: 'Feedback', min: -0.95, max: 0.95, default: 0.5, unit: '' },
      { id: 3, name: 'wet_dry', label: 'Wet/Dry', min: 0, max: 1, default: 0.5, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Flanger</div>
        <div class="node-param">
          <label>Rate: <span id="flangerrate-${nodeId}">0.5</span>Hz</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.1" max="10" value="0.5" step="0.1">
        </div>
        <div class="node-param">
          <label>Depth: <span id="flangerdepth-${nodeId}">0.7</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="1" value="0.7" step="0.01">
        </div>
        <div class="node-param">
          <label>Feedback: <span id="flangerfeedback-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="-0.95" max="0.95" value="0.5" step="0.01">
        </div>
        <div class="node-param">
          <label>Wet/Dry: <span id="flangerwetdry-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="3" min="0" max="1" value="0.5" step="0.01">
        </div>
      </div>
    `
  },

  FMSynth: {
    name: 'FM Synth',
    category: NodeCategory.GENERATOR,
    description: '4-operator FM synthesizer',
    inputs: [
      { name: 'V/Oct', type: SignalType.CV, index: 0 },
      { name: 'Gate', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'algorithm', label: 'Algorithm', min: 0, max: 3, default: 0, unit: '' },
      { id: 1, name: 'op1_ratio', label: 'Op1 Ratio', min: 0.25, max: 16, default: 1.0, unit: '' },
      { id: 2, name: 'op1_level', label: 'Op1 Level', min: 0, max: 1, default: 1.0, unit: '' },
      { id: 3, name: 'op2_ratio', label: 'Op2 Ratio', min: 0.25, max: 16, default: 2.0, unit: '' },
      { id: 4, name: 'op2_level', label: 'Op2 Level', min: 0, max: 1, default: 0.8, unit: '' },
      { id: 5, name: 'op3_ratio', label: 'Op3 Ratio', min: 0.25, max: 16, default: 3.0, unit: '' },
      { id: 6, name: 'op3_level', label: 'Op3 Level', min: 0, max: 1, default: 0.6, unit: '' },
      { id: 7, name: 'op4_ratio', label: 'Op4 Ratio', min: 0.25, max: 16, default: 4.0, unit: '' },
      { id: 8, name: 'op4_level', label: 'Op4 Level', min: 0, max: 1, default: 0.4, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">FM Synth</div>
        <div class="node-param">
          <label>Algorithm: <span id="fmalgo-${nodeId}">0</span></label>
          <select data-node="${nodeId}" data-param="0" style="width: 100%; padding: 2px;">
            <option value="0">Stack (1→2→3→4)</option>
            <option value="1">Parallel</option>
            <option value="2">Bell (1→2, 3→4)</option>
            <option value="3">Dual (1→2, 3→4)</option>
          </select>
        </div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">Operator 1</div>
        <div class="node-param">
          <label>Ratio: <span id="op1ratio-${nodeId}">1.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.25" max="16" value="1.0" step="0.25">
        </div>
        <div class="node-param">
          <label>Level: <span id="op1level-${nodeId}">1.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="1.0" step="0.01">
        </div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">Operator 2</div>
        <div class="node-param">
          <label>Ratio: <span id="op2ratio-${nodeId}">2.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="3" min="0.25" max="16" value="2.0" step="0.25">
        </div>
        <div class="node-param">
          <label>Level: <span id="op2level-${nodeId}">0.8</span></label>
          <input type="range" data-node="${nodeId}" data-param="4" min="0" max="1" value="0.8" step="0.01">
        </div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">Operator 3</div>
        <div class="node-param">
          <label>Ratio: <span id="op3ratio-${nodeId}">3.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="5" min="0.25" max="16" value="3.0" step="0.25">
        </div>
        <div class="node-param">
          <label>Level: <span id="op3level-${nodeId}">0.6</span></label>
          <input type="range" data-node="${nodeId}" data-param="6" min="0" max="1" value="0.6" step="0.01">
        </div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">Operator 4</div>
        <div class="node-param">
          <label>Ratio: <span id="op4ratio-${nodeId}">4.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="7" min="0.25" max="16" value="4.0" step="0.25">
        </div>
        <div class="node-param">
          <label>Level: <span id="op4level-${nodeId}">0.4</span></label>
          <input type="range" data-node="${nodeId}" data-param="8" min="0" max="1" value="0.4" step="0.01">
        </div>
      </div>
    `
  },

  WavetableOscillator: {
    name: 'Wavetable',
    category: NodeCategory.GENERATOR,
    description: 'Wavetable oscillator with preset waveforms',
    inputs: [
      { name: 'V/Oct', type: SignalType.CV, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'wavetable', label: 'Wavetable', min: 0, max: 7, default: 0, unit: '' },
      { id: 1, name: 'fine_tune', label: 'Fine Tune', min: -1, max: 1, default: 0, unit: '' },
      { id: 2, name: 'position', label: 'Position', min: 0, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Wavetable</div>
        <div class="node-param">
          <label>Waveform: <span id="wavetable-${nodeId}">Sine</span></label>
          <select data-node="${nodeId}" data-param="0" style="width: 100%; padding: 2px;">
            <option value="0">Sine</option>
            <option value="1">Saw</option>
            <option value="2">Square</option>
            <option value="3">Triangle</option>
            <option value="4">PWM</option>
            <option value="5">Harmonic</option>
            <option value="6">Inharmonic</option>
            <option value="7">Digital</option>
          </select>
        </div>
        <div class="node-param">
          <label>Fine: <span id="finetune-${nodeId}">0.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="-1" max="1" value="0" step="0.01">
        </div>
        <div class="node-param">
          <label>Position: <span id="position-${nodeId}">0.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0" step="0.01">
        </div>
      </div>
    `
  },

  SimpleSampler: {
    name: 'Sampler',
    category: NodeCategory.GENERATOR,
    description: 'Simple sample playback with pitch shifting',
    inputs: [
      { name: 'V/Oct', type: SignalType.CV, index: 0 },
      { name: 'Gate', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'gain', label: 'Gain', min: 0, max: 2, default: 1.0, unit: '' },
      { id: 1, name: 'loop', label: 'Loop', min: 0, max: 1, default: 0, unit: '' },
      { id: 2, name: 'pitch_shift', label: 'Pitch Shift', min: -12, max: 12, default: 0, unit: 'semi' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Sampler</div>
        <div class="node-param">
          <label>Gain: <span id="gain-${nodeId}">1.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0" max="2" value="1.0" step="0.01">
        </div>
        <div class="node-param">
          <label>Loop: <span id="loop-${nodeId}">Off</span></label>
          <input type="checkbox" class="node-checkbox" data-node="${nodeId}" data-param="1">
        </div>
        <div class="node-param">
          <label>Pitch: <span id="pitch-${nodeId}">0</span> semi</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="-12" max="12" value="0" step="1">
        </div>
        <div class="node-param" style="margin-top: 4px;">
          <button class="load-sample-btn" data-node="${nodeId}" style="width: 100%; padding: 4px; font-size: 10px;">Load Sample</button>
        </div>
        <div id="sample-info-${nodeId}" style="font-size: 9px; color: #888; margin-top: 2px; text-align: center;">No sample loaded</div>
      </div>
    `
  },

  MultiSampler: {
    name: 'Multi Sampler',
    category: NodeCategory.GENERATOR,
    description: 'Multi-sample instrument with velocity layers and key zones',
    inputs: [
      { name: 'MIDI In', type: SignalType.MIDI, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'gain', label: 'Gain', min: 0, max: 2, default: 1.0, unit: '' },
      { id: 1, name: 'attack', label: 'Attack', min: 0.001, max: 1, default: 0.01, unit: 's' },
      { id: 2, name: 'release', label: 'Release', min: 0.01, max: 5, default: 0.1, unit: 's' },
      { id: 3, name: 'transpose', label: 'Transpose', min: -24, max: 24, default: 0, unit: 'semi' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Multi Sampler</div>
        <div class="node-param">
          <label>Gain: <span id="gain-${nodeId}">1.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0" max="2" value="1.0" step="0.01">
        </div>
        <div class="node-param">
          <label>Attack: <span id="attack-${nodeId}">0.01</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.001" max="1" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>Release: <span id="release-${nodeId}">0.10</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0.01" max="5" value="0.1" step="0.01">
        </div>
        <div class="node-param">
          <label>Transpose: <span id="transpose-${nodeId}">0</span> semi</label>
          <input type="range" data-node="${nodeId}" data-param="3" min="-24" max="24" value="0" step="1">
        </div>
        <div class="node-param" style="margin-top: 4px;">
          <button class="add-layer-btn" data-node="${nodeId}" style="width: 100%; padding: 4px; font-size: 10px;">Add Sample Layer</button>
        </div>
        <div id="sample-layers-container-${nodeId}" class="sample-layers-container">
          <table id="sample-layers-table-${nodeId}" class="sample-layers-table">
            <thead>
              <tr>
                <th>File</th>
                <th>Range</th>
                <th>Root</th>
                <th>Vel</th>
                <th></th>
              </tr>
            </thead>
            <tbody id="sample-layers-list-${nodeId}">
              <tr><td colspan="5" class="sample-layers-empty">No layers loaded</td></tr>
            </tbody>
          </table>
        </div>
      </div>
    `
  },

  Compressor: {
    name: 'Compressor',
    category: NodeCategory.EFFECT,
    description: 'Dynamic range compressor with soft-knee',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'threshold', label: 'Threshold', min: -60, max: 0, default: -20, unit: 'dB' },
      { id: 1, name: 'ratio', label: 'Ratio', min: 1, max: 20, default: 4, unit: ':1' },
      { id: 2, name: 'attack', label: 'Attack', min: 0.1, max: 100, default: 5, unit: 'ms' },
      { id: 3, name: 'release', label: 'Release', min: 10, max: 1000, default: 100, unit: 'ms' },
      { id: 4, name: 'makeup_gain', label: 'Makeup Gain', min: 0, max: 20, default: 0, unit: 'dB' },
      { id: 5, name: 'knee', label: 'Knee', min: 0, max: 12, default: 6, unit: 'dB' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Compressor</div>
        <div class="node-param">
          <label>Threshold: <span id="threshold-${nodeId}">-20</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="-60" max="0" value="-20" step="0.1">
        </div>
        <div class="node-param">
          <label>Ratio: <span id="ratio-${nodeId}">4.0</span>:1</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="1" max="20" value="4" step="0.1">
        </div>
        <div class="node-param">
          <label>Attack: <span id="attack-${nodeId}">5</span> ms</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0.1" max="100" value="5" step="0.1">
        </div>
        <div class="node-param">
          <label>Release: <span id="release-${nodeId}">100</span> ms</label>
          <input type="range" data-node="${nodeId}" data-param="3" min="10" max="1000" value="100" step="1">
        </div>
        <div class="node-param">
          <label>Makeup: <span id="makeup-${nodeId}">0</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="4" min="0" max="20" value="0" step="0.1">
        </div>
        <div class="node-param">
          <label>Knee: <span id="knee-${nodeId}">6</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="5" min="0" max="12" value="6" step="0.1">
        </div>
      </div>
    `
  },

  Limiter: {
    name: 'Limiter',
    category: NodeCategory.EFFECT,
    description: 'Peak limiter with ceiling control',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'threshold', label: 'Threshold', min: -60, max: 0, default: -10, unit: 'dB' },
      { id: 1, name: 'release', label: 'Release', min: 10, max: 1000, default: 50, unit: 'ms' },
      { id: 2, name: 'ceiling', label: 'Ceiling', min: -20, max: 0, default: 0, unit: 'dB' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Limiter</div>
        <div class="node-param">
          <label>Threshold: <span id="limthreshold-${nodeId}">-10</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="-60" max="0" value="-10" step="0.1">
        </div>
        <div class="node-param">
          <label>Release: <span id="limrelease-${nodeId}">50</span> ms</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="10" max="1000" value="50" step="1">
        </div>
        <div class="node-param">
          <label>Ceiling: <span id="ceiling-${nodeId}">0</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="-20" max="0" value="0" step="0.1">
        </div>
      </div>
    `
  },

  Distortion: {
    name: 'Distortion',
    category: NodeCategory.EFFECT,
    description: 'Waveshaping distortion with multiple algorithms',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'drive', label: 'Drive', min: 0.01, max: 20, default: 1, unit: '' },
      { id: 1, name: 'type', label: 'Type', min: 0, max: 3, default: 0, unit: '' },
      { id: 2, name: 'tone', label: 'Tone', min: 0, max: 1, default: 0.7, unit: '' },
      { id: 3, name: 'mix', label: 'Mix', min: 0, max: 1, default: 1, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Distortion</div>
        <div class="node-param">
          <label>Type: <span id="disttype-${nodeId}">Soft Clip</span></label>
          <select data-node="${nodeId}" data-param="1" style="width: 100%; padding: 2px;">
            <option value="0">Soft Clip</option>
            <option value="1">Hard Clip</option>
            <option value="2">Tanh</option>
            <option value="3">Asymmetric</option>
          </select>
        </div>
        <div class="node-param">
          <label>Drive: <span id="drive-${nodeId}">1.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.01" max="20" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Tone: <span id="tone-${nodeId}">0.70</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0" max="1" value="0.7" step="0.01">
        </div>
        <div class="node-param">
          <label>Mix: <span id="mix-${nodeId}">1.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="3" min="0" max="1" value="1" step="0.01">
        </div>
      </div>
    `
  },

  Constant: {
    name: 'Constant',
    category: NodeCategory.UTILITY,
    description: 'Constant CV source - outputs a fixed voltage',
    inputs: [],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'value', label: 'Value', min: -10, max: 10, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Constant</div>
        <div class="node-param">
          <label>Value:</label>
          <input type="number" class="node-number-input" data-node="${nodeId}" data-param="0" min="-10" max="10" value="0" step="0.01" style="width: 60px; padding: 2px;">
          <input type="range" data-node="${nodeId}" data-param="0" min="-10" max="10" value="0" step="0.01">
        </div>
      </div>
    `
  },

  Math: {
    name: 'Math',
    category: NodeCategory.UTILITY,
    description: 'Mathematical and logical operations on CV signals',
    inputs: [
      { name: 'CV In A', type: SignalType.CV, index: 0 },
      { name: 'CV In B', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'operation', label: 'Operation', min: 0, max: 13, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Math</div>
        <div class="node-param">
          <label>Op: <span id="operation-${nodeId}">Add</span></label>
          <select class="node-select" data-node="${nodeId}" data-param="0" style="width: 100%; padding: 2px;">
            <option value="0">Add</option>
            <option value="1">Subtract</option>
            <option value="2">Multiply</option>
            <option value="3">Divide</option>
            <option value="4">Min</option>
            <option value="5">Max</option>
            <option value="6">Average</option>
            <option value="7">Invert</option>
            <option value="8">Abs</option>
            <option value="9">Clamp</option>
            <option value="10">Wrap</option>
            <option value="11">Greater</option>
            <option value="12">Less</option>
            <option value="13">Equal</option>
          </select>
        </div>
      </div>
    `
  },

  Quantizer: {
    name: 'Quantizer',
    category: NodeCategory.UTILITY,
    description: 'Quantize CV to musical scales',
    inputs: [
      { name: 'CV In', type: SignalType.CV, index: 0 }
    ],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 },
      { name: 'Gate Out', type: SignalType.CV, index: 1 }
    ],
    parameters: [
      { id: 0, name: 'scale', label: 'Scale', min: 0, max: 10, default: 0, unit: '' },
      { id: 1, name: 'root', label: 'Root', min: 0, max: 11, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Quantizer</div>
        <div class="node-param">
          <label>Scale: <span id="scale-${nodeId}">Chromatic</span></label>
          <select class="node-select" data-node="${nodeId}" data-param="0" style="width: 100%; padding: 2px;">
            <option value="0">Chromatic</option>
            <option value="1">Major</option>
            <option value="2">Minor</option>
            <option value="3">Pent. Major</option>
            <option value="4">Pent. Minor</option>
            <option value="5">Dorian</option>
            <option value="6">Phrygian</option>
            <option value="7">Lydian</option>
            <option value="8">Mixolydian</option>
            <option value="9">Whole Tone</option>
            <option value="10">Octaves</option>
          </select>
        </div>
        <div class="node-param">
          <label>Root: <span id="root-${nodeId}">C</span></label>
          <select class="node-select" data-node="${nodeId}" data-param="1" style="width: 100%; padding: 2px;">
            <option value="0">C</option>
            <option value="1">C#</option>
            <option value="2">D</option>
            <option value="3">D#</option>
            <option value="4">E</option>
            <option value="5">F</option>
            <option value="6">F#</option>
            <option value="7">G</option>
            <option value="8">G#</option>
            <option value="9">A</option>
            <option value="10">A#</option>
            <option value="11">B</option>
          </select>
        </div>
      </div>
    `
  },

  SlewLimiter: {
    name: 'SlewLimiter',
    category: NodeCategory.UTILITY,
    description: 'Limit rate of change for portamento/glide effects',
    inputs: [
      { name: 'CV In', type: SignalType.CV, index: 0 }
    ],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'rise_time', label: 'Rise Time', min: 0, max: 5, default: 0.01, unit: 's' },
      { id: 1, name: 'fall_time', label: 'Fall Time', min: 0, max: 5, default: 0.01, unit: 's' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Slew Limiter</div>
        <div class="node-param">
          <label>Rise: <span id="slewrise-${nodeId}">0.01</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0" max="5" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>Fall: <span id="slewfall-${nodeId}">0.01</span>s</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0" max="5" value="0.01" step="0.001">
        </div>
      </div>
    `
  },

  EQ: {
    name: 'EQ',
    category: NodeCategory.EFFECT,
    description: '3-band parametric EQ',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'low_freq', label: 'Low Freq', min: 20, max: 500, default: 100, unit: 'Hz' },
      { id: 1, name: 'low_gain', label: 'Low Gain', min: -24, max: 24, default: 0, unit: 'dB' },
      { id: 2, name: 'mid_freq', label: 'Mid Freq', min: 200, max: 5000, default: 1000, unit: 'Hz' },
      { id: 3, name: 'mid_gain', label: 'Mid Gain', min: -24, max: 24, default: 0, unit: 'dB' },
      { id: 4, name: 'mid_q', label: 'Mid Q', min: 0.1, max: 10, default: 0.707, unit: '' },
      { id: 5, name: 'high_freq', label: 'High Freq', min: 2000, max: 20000, default: 8000, unit: 'Hz' },
      { id: 6, name: 'high_gain', label: 'High Gain', min: -24, max: 24, default: 0, unit: 'dB' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">EQ</div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">Low Band</div>
        <div class="node-param">
          <label>Freq: <span id="lowfreq-${nodeId}">100</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="20" max="500" value="100" step="1">
        </div>
        <div class="node-param">
          <label>Gain: <span id="lowgain-${nodeId}">0</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="-24" max="24" value="0" step="0.1">
        </div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">Mid Band</div>
        <div class="node-param">
          <label>Freq: <span id="midfreq-${nodeId}">1000</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="200" max="5000" value="1000" step="10">
        </div>
        <div class="node-param">
          <label>Gain: <span id="midgain-${nodeId}">0</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="3" min="-24" max="24" value="0" step="0.1">
        </div>
        <div class="node-param">
          <label>Q: <span id="midq-${nodeId}">0.71</span></label>
          <input type="range" data-node="${nodeId}" data-param="4" min="0.1" max="10" value="0.707" step="0.01">
        </div>
        <div style="font-size: 10px; margin-top: 4px; font-weight: bold;">High Band</div>
        <div class="node-param">
          <label>Freq: <span id="highfreq-${nodeId}">8000</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="5" min="2000" max="20000" value="8000" step="100">
        </div>
        <div class="node-param">
          <label>Gain: <span id="highgain-${nodeId}">0</span> dB</label>
          <input type="range" data-node="${nodeId}" data-param="6" min="-24" max="24" value="0" step="0.1">
        </div>
      </div>
    `
  },

  SampleHold: {
    name: 'Sample & Hold',
    category: NodeCategory.UTILITY,
    description: 'Samples CV input when gate signal goes high',
    inputs: [
      { name: 'CV In', type: SignalType.CV, index: 0 },
      { name: 'Gate In', type: SignalType.CV, index: 1 }
    ],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Sample & Hold</div>
        <div style="padding: 8px; font-size: 11px; color: #888;">
          Samples CV input<br>on gate rising edge
        </div>
      </div>
    `
  },

  EnvelopeFollower: {
    name: 'Envelope Follower',
    category: NodeCategory.UTILITY,
    description: 'Extracts amplitude envelope from audio signal',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'CV Out', type: SignalType.CV, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'attack', label: 'Attack', min: 0.001, max: 1.0, default: 0.01, unit: 's' },
      { id: 1, name: 'release', label: 'Release', min: 0.001, max: 1.0, default: 0.1, unit: 's' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Envelope Follower</div>
        <div class="node-param">
          <label>Attack: <span id="attack-${nodeId}">0.01</span> s</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.001" max="1.0" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>Release: <span id="release-${nodeId}">0.1</span> s</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.001" max="1.0" value="0.1" step="0.001">
        </div>
      </div>
    `
  },

  RingModulator: {
    name: 'Ring Modulator',
    category: NodeCategory.EFFECT,
    description: 'Multiplies carrier and modulator for metallic timbres',
    inputs: [
      { name: 'Carrier', type: SignalType.AUDIO, index: 0 },
      { name: 'Modulator', type: SignalType.AUDIO, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'mix', label: 'Mix', min: 0.0, max: 1.0, default: 1.0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Ring Modulator</div>
        <div class="node-param">
          <label>Mix: <span id="mix-${nodeId}">1.00</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.0" max="1.0" value="1.0" step="0.01">
        </div>
      </div>
    `
  },

  Phaser: {
    name: 'Phaser',
    category: NodeCategory.EFFECT,
    description: 'Sweeping all-pass filters for phase shifting effect',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'rate', label: 'Rate', min: 0.1, max: 10.0, default: 0.5, unit: 'Hz' },
      { id: 1, name: 'depth', label: 'Depth', min: 0.0, max: 1.0, default: 0.7, unit: '' },
      { id: 2, name: 'stages', label: 'Stages', min: 2, max: 8, default: 6, unit: '' },
      { id: 3, name: 'feedback', label: 'Feedback', min: -0.95, max: 0.95, default: 0.5, unit: '' },
      { id: 4, name: 'wetdry', label: 'Wet/Dry', min: 0.0, max: 1.0, default: 0.5, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Phaser</div>
        <div class="node-param">
          <label>Rate: <span id="rate-${nodeId}">0.5</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="0.1" max="10.0" value="0.5" step="0.1">
        </div>
        <div class="node-param">
          <label>Depth: <span id="depth-${nodeId}">0.7</span></label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.0" max="1.0" value="0.7" step="0.01">
        </div>
        <div class="node-param">
          <label>Stages: <span id="stages-${nodeId}">6</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="2" max="8" value="6" step="2">
        </div>
        <div class="node-param">
          <label>Feedback: <span id="feedback-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="3" min="-0.95" max="0.95" value="0.5" step="0.01">
        </div>
        <div class="node-param">
          <label>Wet/Dry: <span id="wetdry-${nodeId}">0.5</span></label>
          <input type="range" data-node="${nodeId}" data-param="4" min="0.0" max="1.0" value="0.5" step="0.01">
        </div>
      </div>
    `
  },

  BitCrusher: {
    name: 'Bit Crusher',
    category: NodeCategory.EFFECT,
    description: 'Lo-fi effect via bit depth and sample rate reduction',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'bitdepth', label: 'Bit Depth', min: 1, max: 16, default: 8, unit: 'bits' },
      { id: 1, name: 'samplerate', label: 'Sample Rate', min: 100, max: 48000, default: 8000, unit: 'Hz' },
      { id: 2, name: 'mix', label: 'Mix', min: 0.0, max: 1.0, default: 1.0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Bit Crusher</div>
        <div class="node-param">
          <label>Bit Depth: <span id="bitdepth-${nodeId}">8</span> bits</label>
          <input type="range" data-node="${nodeId}" data-param="0" min="1" max="16" value="8" step="1">
        </div>
        <div class="node-param">
          <label>Sample Rate: <span id="samplerate-${nodeId}">8000</span> Hz</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="100" max="48000" value="8000" step="100">
        </div>
        <div class="node-param">
          <label>Mix: <span id="mix-${nodeId}">1.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0.0" max="1.0" value="1.0" step="0.01">
        </div>
      </div>
    `
  },

  Vocoder: {
    name: 'Vocoder',
    category: NodeCategory.EFFECT,
    description: 'Multi-band vocoder - modulator controls carrier spectrum',
    inputs: [
      { name: 'Modulator', type: SignalType.AUDIO, index: 0 },
      { name: 'Carrier', type: SignalType.AUDIO, index: 1 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'bands', label: 'Bands', min: 8, max: 32, default: 16, unit: '' },
      { id: 1, name: 'attack', label: 'Attack', min: 0.001, max: 0.1, default: 0.01, unit: 's' },
      { id: 2, name: 'release', label: 'Release', min: 0.001, max: 1.0, default: 0.05, unit: 's' },
      { id: 3, name: 'mix', label: 'Mix', min: 0.0, max: 1.0, default: 1.0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Vocoder</div>
        <div class="node-param">
          <label>Bands: <span id="bands-${nodeId}">16</span></label>
          <input type="range" data-node="${nodeId}" data-param="0" min="8" max="32" value="16" step="1">
        </div>
        <div class="node-param">
          <label>Attack: <span id="attack-${nodeId}">0.01</span> s</label>
          <input type="range" data-node="${nodeId}" data-param="1" min="0.001" max="0.1" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>Release: <span id="release-${nodeId}">0.05</span> s</label>
          <input type="range" data-node="${nodeId}" data-param="2" min="0.001" max="1.0" value="0.05" step="0.001">
        </div>
        <div class="node-param">
          <label>Mix: <span id="mix-${nodeId}">1.0</span></label>
          <input type="range" data-node="${nodeId}" data-param="3" min="0.0" max="1.0" value="1.0" step="0.01">
        </div>
      </div>
    `
  }
};

/**
 * Get all node types in a specific category
 */
export function getNodesByCategory(category) {
  return Object.entries(nodeTypes)
    .filter(([_, def]) => def.category === category)
    .map(([type, def]) => ({ type, ...def }));
}

/**
 * Get all categories that have nodes
 */
export function getCategories() {
  const categories = new Set();
  Object.values(nodeTypes).forEach(def => categories.add(def.category));
  return Array.from(categories);
}

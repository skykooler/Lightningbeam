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
                 class="node-slider"
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
                 class="node-slider"
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
                 class="node-slider"
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
                 class="node-slider"
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
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="0" max="2" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Gain 2: <span id="g2-${nodeId}">1.0</span>x</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="1" min="0" max="2" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Gain 3: <span id="g3-${nodeId}">1.0</span>x</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="2" min="0" max="2" value="1" step="0.01">
        </div>
        <div class="node-param">
          <label>Gain 4: <span id="g4-${nodeId}">1.0</span>x</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="3" min="0" max="2" value="1" step="0.01">
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
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="20" max="20000" value="1000" step="1">
        </div>
        <div class="node-param">
          <label>Resonance: <span id="res-${nodeId}">0.707</span></label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="1" min="0.1" max="10" value="0.707" step="0.01">
        </div>
        <div class="node-param">
          <label>Type: <span id="ftype-${nodeId}">LP</span></label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="2" min="0" max="1" value="0" step="1">
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
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="0.001" max="5" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>D: <span id="d-${nodeId}">0.1</span>s</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="1" min="0.001" max="5" value="0.1" step="0.001">
        </div>
        <div class="node-param">
          <label>S: <span id="s-${nodeId}">0.7</span></label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="2" min="0" max="1" value="0.7" step="0.01">
        </div>
        <div class="node-param">
          <label>R: <span id="r-${nodeId}">0.2</span>s</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="3" min="0.001" max="5" value="0.2" step="0.001">
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
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="0.001" max="1.0" value="0.01" step="0.001">
        </div>
        <div class="node-param">
          <label>Release: <span id="rel-${nodeId}">0.1</span>s</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="1" min="0.001" max="1.0" value="0.1" step="0.001">
        </div>
      </div>
    `
  },

  Oscilloscope: {
    name: 'Oscilloscope',
    category: NodeCategory.UTILITY,
    description: 'Visual audio signal monitor (pass-through)',
    inputs: [
      { name: 'Audio In', type: SignalType.AUDIO, index: 0 }
    ],
    outputs: [
      { name: 'Audio Out', type: SignalType.AUDIO, index: 0 }
    ],
    parameters: [
      { id: 0, name: 'time_scale', label: 'Time Scale', min: 10, max: 1000, default: 100, unit: 'ms' },
      { id: 1, name: 'trigger_mode', label: 'Trigger', min: 0, max: 2, default: 0, unit: '' },
      { id: 2, name: 'trigger_level', label: 'Trigger Level', min: -1, max: 1, default: 0, unit: '' }
    ],
    getHTML: (nodeId) => `
      <div class="node-content">
        <div class="node-title">Oscilloscope</div>
        <div class="node-param">
          <label>Time: <span id="time-${nodeId}">100</span>ms</label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="10" max="1000" value="100" step="10">
        </div>
        <div class="node-param">
          <label>Trigger: <span id="trig-${nodeId}">Free</span></label>
          <input type="range" class="node-slider" data-node="${nodeId}" data-param="1" min="0" max="2" value="0" step="1">
        </div>
        <div class="node-info" style="margin-top: 4px; font-size: 10px;">Pass-through monitor</div>
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
            <input type="range" class="node-slider" data-node="${nodeId}" data-param="0" min="1" max="16" value="8" step="1">
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

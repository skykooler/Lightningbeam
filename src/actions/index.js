// Actions module - extracted from main.js
// This module contains all the undo/redo-able actions for the application

// Imports for dependencies
import { context, pointerList } from '../state.js';
import { Shape } from '../models/shapes.js';
import { Bezier } from '../bezier.js';
import {
  Keyframe,
  AnimationCurve,
  AnimationData,
  Frame
} from '../models/animation.js';
import { GraphicsObject } from '../models/graphics-object.js';
import { VectorLayer, AudioTrack, VideoLayer } from '../models/layer.js';
import {
  arraysAreEqual,
  lerp,
  lerpColor,
  generateWaveform,
  signedAngleBetweenVectors,
  rotateAroundPointIncremental,
  getRotatedBoundingBox,
  growBoundingBox
} from '../utils.js';

// UUID generation function (keeping local version for now)
function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

/**
 * Initialize a timeline curve for an AutomationInput node
 * Creates the curve with a default keyframe at time 0
 * @param {number} trackId - Track ID
 * @param {number} nodeId - Backend node ID
 */
async function initializeAutomationCurve(trackId, nodeId) {
  try {
    // Find the audio/MIDI track
    const track = context.activeObject.audioTracks?.find(t => t.audioTrackId === trackId);
    if (!track) {
      console.error(`Track ${trackId} not found`);
      return;
    }

    // Create curve parameter name: "automation.{nodeId}"
    const curveName = `automation.${nodeId}`;

    // Check if curve already exists
    if (track.animationData.curves[curveName]) {
      console.log(`Curve ${curveName} already exists`);
      return;
    }

    // Create the curve with a default keyframe at time 0, value 0
    const curve = track.animationData.getOrCreateCurve(curveName);
    curve.addKeyframe({
      time: 0,
      value: 0,
      interpolation: 'linear',
      easeIn: { x: 0.42, y: 0 },
      easeOut: { x: 0.58, y: 1 },
      idx: `${Date.now()}-${Math.random()}`
    });

    console.log(`Initialized automation curve: ${curveName}`);

    // Redraw timeline if it's open
    if (context.timeline?.requestRedraw) {
      context.timeline.requestRedraw();
    }
  } catch (err) {
    console.error('Failed to initialize automation curve:', err);
  }
}

/**
 * Update automation node name based on its connection
 * If the source node is an AutomationInput, generate a friendly name from the target
 * @param {number} trackId - Track ID
 * @param {number} fromNode - Source node ID
 * @param {number} toNode - Target node ID
 * @param {string} toPortClass - Target port name (frontend)
 */
async function updateAutomationName(trackId, fromNode, toNode, toPortClass) {
  try {
    // Get the full graph state to find node types and port information
    const graphStateJson = await invoke('graph_get_state', { trackId });
    const graphState = JSON.parse(graphStateJson);

    // Find the source node
    const sourceNode = graphState.nodes.find(n => n.id === fromNode);
    if (!sourceNode || sourceNode.node_type !== 'AutomationInput') {
      return; // Not an AutomationInput, nothing to do
    }

    // Find the target node
    const targetNode = graphState.nodes.find(n => n.id === toNode);
    if (!targetNode) {
      return;
    }

    // Find the connection from this AutomationInput to the target node
    const connection = graphState.connections.find(c =>
      c.from_node === fromNode && c.to_node === toNode
    );

    if (!connection) {
      return;
    }

    // Use the backend port name from the connection
    // This will be something like "cutoff", "frequency", etc.
    const portName = connection.to_port;

    // Generate a friendly name: "{TargetType} {PortName}"
    // e.g., "Filter cutoff" or "Oscillator frequency"
    const name = `${targetNode.node_type} ${portName}`;

    // Set the automation name in the backend
    await invoke('automation_set_name', {
      trackId: trackId,
      nodeId: fromNode,
      name
    });

    // Update the node UI display if the node editor is open
    if (context.nodeEditor) {
      const nameElement = document.getElementById(`automation-name-${fromNode}`);
      if (nameElement) {
        nameElement.textContent = name;
      }
    }

    // Invalidate the timeline cache for this automation node
    if (context.timelineWidget) {
      const cacheKey = `${trackId}:${fromNode}`;
      context.timelineWidget.automationNameCache.delete(cacheKey);

      // Trigger a redraw to fetch and display the new name
      if (context.timelineWidget.requestRedraw) {
        context.timelineWidget.requestRedraw();
      }
    }

    console.log(`Auto-named automation node ${fromNode}: "${name}"`);
  } catch (err) {
    console.error('Failed to update automation name:', err);
  }
}

// Dependencies that will be injected
let undoStack = null;
let redoStack = null;
let updateMenu = null;
let updateLayers = null;
let updateUI = null;
let updateVideoFrames = null;
let updateInfopanel = null;
let invoke = null;
let config = null;

/**
 * Initialize the actions module with required dependencies
 * @param {Object} deps - Dependencies object
 * @param {Array} deps.undoStack - Reference to the undo stack
 * @param {Array} deps.redoStack - Reference to the redo stack
 * @param {Function} deps.updateMenu - Function to update the menu
 * @param {Function} deps.updateLayers - Function to update layers UI
 * @param {Function} deps.updateUI - Function to update main UI
 * @param {Function} deps.updateInfopanel - Function to update info panel
 * @param {Function} deps.invoke - Tauri invoke function
 * @param {Object} deps.config - Application config object
 */
// Export the auto-naming function for use in main.js
export { updateAutomationName };

export function initializeActions(deps) {
  undoStack = deps.undoStack;
  redoStack = deps.redoStack;
  updateMenu = deps.updateMenu;
  updateLayers = deps.updateLayers;
  updateUI = deps.updateUI;
  updateVideoFrames = deps.updateVideoFrames;
  updateInfopanel = deps.updateInfopanel;
  invoke = deps.invoke;
  config = deps.config;
}

export const actions = {
  addShape: {
    create: (parent, shape, ctx) => {
      // parent should be a GraphicsObject
      if (!parent.activeLayer) return;
      if (shape.curves.length == 0) return;
      redoStack.length = 0; // Clear redo stack
      let serializableCurves = [];
      for (let curve of shape.curves) {
        serializableCurves.push({ points: curve.points, color: curve.color });
      }
      let c = {
        ...context,
        ...ctx,
      };
      let action = {
        parent: parent.idx,
        layer: parent.activeLayer.idx,
        curves: serializableCurves,
        startx: shape.startx,
        starty: shape.starty,
        context: {
          fillShape: c.fillShape,
          strokeShape: c.strokeShape,
          fillStyle: c.fillStyle,
          sendToBack: c.sendToBack,
          lineWidth: c.lineWidth,
        },
        uuid: uuidv4(),
        time: parent.currentTime, // Use currentTime instead of currentFrame
      };
      undoStack.push({ name: "addShape", action: action });
      actions.addShape.execute(action);
      updateMenu();
      updateLayers();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      let curvesList = action.curves;
      let cxt = {
        ...context,
        ...action.context,
      };
      let shape = new Shape(action.startx, action.starty, cxt, layer, action.uuid);
      for (let curve of curvesList) {
        shape.addCurve(
          new Bezier(
            curve.points[0].x,
            curve.points[0].y,
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
            curve.points[3].x,
            curve.points[3].y,
          ).setColor(curve.color),
        );
      }
      let shapes = shape.update();
      for (let newShape of shapes) {
        // Add shape to layer's shapes array
        layer.shapes.push(newShape);

        // Determine zOrder based on sendToBack
        let zOrder;
        if (cxt.sendToBack) {
          // Insert at back (zOrder 0), shift all other shapes up
          zOrder = 0;
          // Increment zOrder for all existing shapes
          for (let existingShape of layer.shapes) {
            if (existingShape !== newShape) {
              let existingZOrderCurve = layer.animationData.curves[`shape.${existingShape.shapeId}.zOrder`];
              if (existingZOrderCurve) {
                // Find keyframe at this time and increment it
                for (let kf of existingZOrderCurve.keyframes) {
                  if (kf.time === action.time) {
                    kf.value += 1;
                  }
                }
              }
            }
          }
        } else {
          // Insert at front (max zOrder + 1)
          zOrder = layer.shapes.length - 1;
        }

        // Add keyframes to AnimationData for this shape
        // Use shapeId (not idx) so that multiple versions share curves
        let existsKeyframe = new Keyframe(action.time, 1, "hold");
        layer.animationData.addKeyframe(`shape.${newShape.shapeId}.exists`, existsKeyframe);

        let zOrderKeyframe = new Keyframe(action.time, zOrder, "hold");
        layer.animationData.addKeyframe(`shape.${newShape.shapeId}.zOrder`, zOrderKeyframe);

        let shapeIndexKeyframe = new Keyframe(action.time, 0, "linear");
        layer.animationData.addKeyframe(`shape.${newShape.shapeId}.shapeIndex`, shapeIndexKeyframe);
      }
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      let shape = pointerList[action.uuid];

      // Remove shape from layer's shapes array
      let shapeIndex = layer.shapes.indexOf(shape);
      if (shapeIndex !== -1) {
        layer.shapes.splice(shapeIndex, 1);
      }

      // Remove keyframes from AnimationData (use shapeId not idx)
      delete layer.animationData.curves[`shape.${shape.shapeId}.exists`];
      delete layer.animationData.curves[`shape.${shape.shapeId}.zOrder`];
      delete layer.animationData.curves[`shape.${shape.shapeId}.shapeIndex`];

      delete pointerList[action.uuid];
    },
  },
  editShape: {
    create: (shape, newCurves) => {
      redoStack.length = 0; // Clear redo stack
      let serializableNewCurves = [];
      for (let curve of newCurves) {
        serializableNewCurves.push({
          points: curve.points,
          color: curve.color,
        });
      }
      let serializableOldCurves = [];
      for (let curve of shape.curves) {
        serializableOldCurves.push({ points: curve.points });
      }
      let action = {
        shape: shape.idx,
        oldCurves: serializableOldCurves,
        newCurves: serializableNewCurves,
      };
      undoStack.push({ name: "editShape", action: action });
      actions.editShape.execute(action);
    },
    execute: (action) => {
      let shape = pointerList[action.shape];
      let curvesList = action.newCurves;
      shape.curves = [];
      for (let curve of curvesList) {
        shape.addCurve(
          new Bezier(
            curve.points[0].x,
            curve.points[0].y,
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
            curve.points[3].x,
            curve.points[3].y,
          ).setColor(curve.color),
        );
      }
      shape.update();
      updateUI();
    },
    rollback: (action) => {
      let shape = pointerList[action.shape];
      let curvesList = action.oldCurves;
      shape.curves = [];
      for (let curve of curvesList) {
        shape.addCurve(
          new Bezier(
            curve.points[0].x,
            curve.points[0].y,
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
            curve.points[3].x,
            curve.points[3].y,
          ).setColor(curve.color),
        );
      }
      shape.update();
    },
  },
  colorShape: {
    create: (shape, color) => {
      redoStack.length = 0; // Clear redo stack
      let action = {
        shape: shape.idx,
        oldColor: shape.fillStyle,
        newColor: color,
      };
      undoStack.push({ name: "colorShape", action: action });
      actions.colorShape.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let shape = pointerList[action.shape];
      shape.fillStyle = action.newColor;
    },
    rollback: (action) => {
      let shape = pointerList[action.shape];
      shape.fillStyle = action.oldColor;
    },
  },
  addImageObject: {
    create: (x, y, imgsrc, ix, parent) => {
      redoStack.length = 0; // Clear redo stack
      let action = {
        shapeUuid: uuidv4(),
        objectUuid: uuidv4(),
        x: x,
        y: y,
        src: imgsrc,
        ix: ix,
        parent: parent.idx,
      };
      undoStack.push({ name: "addImageObject", action: action });
      actions.addImageObject.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      let imageObject = new GraphicsObject(action.objectUuid);
      function loadImage(src) {
        return new Promise((resolve, reject) => {
          let img = new Image();
          img.onload = () => resolve(img); // Resolve the promise with the image once loaded
          img.onerror = (err) => reject(err); // Reject the promise if there's an error loading the image
          img.src = src; // Start loading the image
        });
      }
      let img = await loadImage(action.src);
      // img.onload = function() {
      let ct = {
        ...context,
        fillImage: img,
        strokeShape: false,
      };
      let imageShape = new Shape(0, 0, ct, imageObject.activeLayer, action.shapeUuid);
      imageShape.addLine(img.width, 0);
      imageShape.addLine(img.width, img.height);
      imageShape.addLine(0, img.height);
      imageShape.addLine(0, 0);
      imageShape.update();
      imageShape.fillImage = img;
      imageShape.filled = true;

      // Add shape to layer using new AnimationData-aware method
      const time = imageObject.currentTime || 0;
      imageObject.activeLayer.addShape(imageShape, time);
      let parent = pointerList[action.parent];
      parent.addObject(
        imageObject,
        action.x - img.width / 2 + 20 * action.ix,
        action.y - img.height / 2 + 20 * action.ix,
      );
      updateUI();
      // }
      // img.src = action.src
    },
    rollback: (action) => {
      let shape = pointerList[action.shapeUuid];
      let object = pointerList[action.objectUuid];
      let parent = pointerList[action.parent];
      object.getFrame(0).removeShape(shape);
      delete pointerList[action.shapeUuid];
      parent.removeChild(object);
      delete pointerList[action.objectUuid];
      let selectIndex = context.selection.indexOf(object);
      if (selectIndex >= 0) {
        context.selection.splice(selectIndex, 1);
      }
    },
  },
  addAudio: {
    create: (filePath, object, audioname) => {
      redoStack.length = 0;
      let action = {
        filePath: filePath,
        audioname: audioname,
        trackuuid: uuidv4(),
        object: object.idx,
      };
      undoStack.push({ name: "addAudio", action: action });
      actions.addAudio.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      // Create new AudioTrack with DAW backend
      let newAudioTrack = new AudioTrack(action.trackuuid, action.audioname);
      let object = pointerList[action.object];

      // Add placeholder clip immediately so user sees feedback
      newAudioTrack.clips.push({
        clipId: 0,
        poolIndex: 0,
        name: 'Loading...',
        startTime: 0,
        duration: 10,
        offset: 0,
        loading: true
      });

      // Add track to object immediately
      object.audioTracks.push(newAudioTrack);

      // Update UI to show placeholder
      updateLayers();
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw();
      }

      // Load audio asynchronously and update clip
      try {
        // Initialize track in backend and load audio file
        await newAudioTrack.initializeTrack();
        const metadata = await newAudioTrack.loadAudioFile(action.filePath);

        // Use actual duration from the audio file metadata
        const duration = metadata.duration;

        // Replace placeholder clip with real clip
        newAudioTrack.clips[0] = {
          clipId: 0,
          poolIndex: metadata.pool_index,
          name: action.audioname,
          startTime: 0,
          duration: duration,
          offset: 0,
          loading: false,
          waveform: metadata.waveform  // Store waveform data for rendering
        };

        // Add clip to backend (call backend directly to avoid duplicate push)
        const { invoke } = window.__TAURI__.core
        await invoke('audio_add_clip', {
          trackId: newAudioTrack.audioTrackId,
          poolIndex: metadata.pool_index,
          startTime: 0,
          duration: duration,
          offset: 0
        });

        // Update UI with real clip data
        updateLayers();
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }

        // Make this the active track
        if (context.activeObject) {
          context.activeObject.activeLayer = newAudioTrack;
          updateLayers(); // Refresh to show active state
          // Reload node editor to show the new track's graph
          if (context.reloadNodeEditor) {
            await context.reloadNodeEditor();
          }
        }

        // Prompt user to set BPM if detected
        if (metadata.detected_bpm && context.timelineWidget) {
          const currentBpm = context.timelineWidget.timelineState.bpm;
          const detectedBpm = metadata.detected_bpm;
          const shouldSetBpm = confirm(
            `Detected BPM: ${detectedBpm}\n\n` +
            `Current project BPM: ${currentBpm}\n\n` +
            `Would you like to set the project BPM to ${detectedBpm}?`
          );

          if (shouldSetBpm) {
            context.timelineWidget.timelineState.bpm = detectedBpm;
            context.timelineWidget.requestRedraw(); // Redraw to show updated BPM
            console.log(`Project BPM set to ${detectedBpm}`);
            // Notify all registered listeners of BPM change
            if (context.notifyBpmChange) {
              context.notifyBpmChange(detectedBpm);
            }
          }
        }
      } catch (error) {
        console.error('Failed to load audio:', error);
        // Update clip to show error
        newAudioTrack.clips[0].name = 'Error loading';
        newAudioTrack.clips[0].loading = false;
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }
      }
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      let track = pointerList[action.trackuuid];
      object.audioTracks.splice(object.audioTracks.indexOf(track), 1);
      updateLayers();
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw();
      }
    },
  },
  addVideo: {
    create: (filePath, object, videoname) => {
      redoStack.length = 0;
      let action = {
        filePath: filePath,
        videoname: videoname,
        layeruuid: uuidv4(),
        object: object.idx,
      };
      undoStack.push({ name: "addVideo", action: action });
      actions.addVideo.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      // Create new VideoLayer
      let newVideoLayer = new VideoLayer(action.layeruuid, action.videoname);
      let object = pointerList[action.object];

      // Add layer to object
      object.layers.push(newVideoLayer);

      // Update UI
      updateLayers();
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw();
      }

      // Load video asynchronously
      try {
        const metadata = await invoke('video_load_file', {
          path: action.filePath
        });

        // Add clip to video layer
        await newVideoLayer.addClip(
          metadata.pool_index,
          0, // startTime
          metadata.duration,
          0, // offset
          action.videoname,
          metadata.duration // sourceDuration
        );

        // If video has audio, create linked AudioTrack
        if (metadata.has_audio && metadata.audio_pool_index !== null) {
          const audioTrackUuid = uuidv4();
          const audioTrackName = `${action.videoname} (Audio)`;
          const newAudioTrack = new AudioTrack(audioTrackUuid, audioTrackName);

          // Initialize track in backend
          await newAudioTrack.initializeTrack();

          // Add audio clip using the extracted audio
          const audioClipId = newAudioTrack.clips.length;
          await invoke('audio_add_clip', {
            trackId: newAudioTrack.audioTrackId,
            poolIndex: metadata.audio_pool_index,
            startTime: 0,
            duration: metadata.audio_duration,
            offset: 0
          });

          const audioClip = {
            clipId: audioClipId,
            poolIndex: metadata.audio_pool_index,
            name: audioTrackName,
            startTime: 0,
            duration: metadata.audio_duration,
            offset: 0,
            waveform: metadata.audio_waveform,
            sourceDuration: metadata.audio_duration
          };
          newAudioTrack.clips.push(audioClip);

          // Link the clips to each other
          const videoClip = newVideoLayer.clips[0];  // The video clip we just added
          if (videoClip) {
            videoClip.linkedAudioClip = audioClip;
            audioClip.linkedVideoClip = videoClip;
          }

          // Also keep track-level references for convenience
          newVideoLayer.linkedAudioTrack = newAudioTrack;
          newAudioTrack.linkedVideoLayer = newVideoLayer;

          // Add audio track to object
          object.audioTracks.push(newAudioTrack);

          // Store reference for rollback
          action.audioTrackUuid = audioTrackUuid;

          console.log(`Video audio extracted: ${metadata.audio_duration}s, ${metadata.audio_sample_rate}Hz, ${metadata.audio_channels}ch`);
        }

        // Update UI with real clip data
        updateLayers();
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }

        // Make this the active layer
        if (context.activeObject) {
          context.activeObject.activeLayer = newVideoLayer;
          updateLayers();
        }

        // Fetch first frame
        if (updateVideoFrames) {
          await updateVideoFrames(context.activeObject.currentTime || 0);
        }

        // Trigger redraw to show the first frame
        updateUI();

        console.log(`Video loaded: ${action.videoname}, ${metadata.width}x${metadata.height}, ${metadata.duration}s`);
      } catch (error) {
        console.error('Failed to load video:', error);
      }
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      let layer = pointerList[action.layeruuid];
      object.layers.splice(object.layers.indexOf(layer), 1);

      // Remove linked audio track if it was created
      if (action.audioTrackUuid) {
        let audioTrack = pointerList[action.audioTrackUuid];
        if (audioTrack) {
          const index = object.audioTracks.indexOf(audioTrack);
          if (index !== -1) {
            object.audioTracks.splice(index, 1);
          }
        }
      }

      updateLayers();
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw();
      }
    },
  },
  addMIDI: {
    create: (filePath, object, midiname) => {
      redoStack.length = 0;
      let action = {
        filePath: filePath,
        midiname: midiname,
        trackuuid: uuidv4(),
        object: object.idx,
      };
      undoStack.push({ name: "addMIDI", action: action });
      actions.addMIDI.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      // Create new AudioTrack with type='midi' for MIDI files
      let newMIDITrack = new AudioTrack(action.trackuuid, action.midiname, 'midi');
      let object = pointerList[action.object];

      // Note: MIDI tracks now use node-based instruments via instrument_graph
      const { invoke } = window.__TAURI__.core;

      // Add placeholder clip immediately so user sees feedback
      newMIDITrack.clips.push({
        clipId: 0,
        name: 'Loading...',
        startTime: 0,
        duration: 10,
        loading: true
      });

      // Add track to object immediately
      object.audioTracks.push(newMIDITrack);

      // Update UI to show placeholder
      updateLayers();
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw();
      }

      // Load MIDI file asynchronously and update clip
      try {
        // Initialize track in backend
        await newMIDITrack.initializeTrack();

        // Load MIDI file into the track
        const metadata = await invoke('audio_load_midi_file', {
          trackId: newMIDITrack.audioTrackId,
          path: action.filePath,
          startTime: 0
        });

        // Replace placeholder clip with real clip including note data
        newMIDITrack.clips[0] = {
          clipId: 0,
          name: action.midiname,
          startTime: 0,
          duration: metadata.duration,
          notes: metadata.notes,  // Store MIDI notes for visualization
          loading: false
        };

        // Update UI with real clip data
        updateLayers();
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }
      } catch (error) {
        console.error('Failed to load MIDI file:', error);
        // Update clip to show error
        newMIDITrack.clips[0].name = 'Error loading';
        newMIDITrack.clips[0].loading = false;
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }
      }
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      let track = pointerList[action.trackuuid];
      object.audioTracks.splice(object.audioTracks.indexOf(track), 1);
      updateLayers();
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw();
      }
    },
  },
  duplicateObject: {
    create: (items) => {
      redoStack.length = 0;
      function deepCopyWithIdxMapping(obj, dictionary = {}) {
        if (Array.isArray(obj)) {
          return obj.map(item => deepCopyWithIdxMapping(item, dictionary));
        }
        if (obj === null || typeof obj !== 'object') {
          return obj;
        }

        const newObj = {};
        for (const key in obj) {
          let value = obj[key];

          if (key === 'idx' && !(value in dictionary)) {
            dictionary[value] = uuidv4();
          }

          newObj[key] = value in dictionary ? dictionary[value] : value;
          if (typeof newObj[key] === 'object' && newObj[key] !== null) {
            newObj[key] = deepCopyWithIdxMapping(newObj[key], dictionary);
          }
        }

        return newObj;
      }
      let action = {
        items: deepCopyWithIdxMapping(items),
        object: context.activeObject.idx,
        layer: context.activeObject.activeLayer.idx,
        time: context.activeObject.currentTime || 0,
        uuid: uuidv4(),
      };
      undoStack.push({ name: "duplicateObject", action: action });
      actions.duplicateObject.execute(action);
      updateMenu();
    },
    execute: (action) => {
      const object = pointerList[action.object];
      const layer = pointerList[action.layer];
      const time = action.time;

      for (let item of action.items) {
        if (item.type == "shape") {
          const shape = Shape.fromJSON(item);
          layer.addShape(shape, time);
        } else if (item.type == "GraphicsObject") {
          const newObj = GraphicsObject.fromJSON(item);
          object.addObject(newObj);
        }
      }
      updateUI();
    },
    rollback: (action) => {
      const object = pointerList[action.object];
      const layer = pointerList[action.layer];

      for (let item of action.items) {
        if (item.type == "shape") {
          layer.removeShape(pointerList[item.idx]);
        } else if (item.type == "GraphicsObject") {
          object.removeChild(pointerList[item.idx]);
        }
      }
      updateUI();
    },
  },
  deleteObjects: {
    create: (objects, shapes) => {
      redoStack.length = 0;
      const layer = context.activeObject.activeLayer;
      const time = context.activeObject.currentTime || 0;

      let serializableObjects = [];
      let oldObjectExists = {};
      for (let object of objects) {
        serializableObjects.push(object.idx);
        // Store old exists value for rollback
        const existsValue = layer.animationData.interpolate(`object.${object.idx}.exists`, time);
        oldObjectExists[object.idx] = existsValue !== null ? existsValue : 1;
      }

      let serializableShapes = [];
      for (let shape of shapes) {
        serializableShapes.push(shape.idx);
      }

      let action = {
        objects: serializableObjects,
        shapes: serializableShapes,
        layer: layer.idx,
        time: time,
        oldObjectExists: oldObjectExists,
      };
      undoStack.push({ name: "deleteObjects", action: action });
      actions.deleteObjects.execute(action);
      updateMenu();
    },
    execute: (action) => {
      const layer = pointerList[action.layer];
      const time = action.time;

      // For objects: set exists to 0 at this time
      for (let objectIdx of action.objects) {
        const existsCurve = layer.animationData.getCurve(`object.${objectIdx}.exists`);
        const kf = existsCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = 0;
        } else {
          layer.animationData.addKeyframe(`object.${objectIdx}.exists`, new Keyframe(time, 0, "hold"));
        }
      }

      // For shapes: remove them (leaves holes that can be filled on undo)
      for (let shapeIdx of action.shapes) {
        layer.removeShape(pointerList[shapeIdx]);
      }
      updateUI();
    },
    rollback: (action) => {
      const layer = pointerList[action.layer];
      const time = action.time;

      // Restore old exists values for objects
      for (let objectIdx of action.objects) {
        const oldExists = action.oldObjectExists[objectIdx];
        const existsCurve = layer.animationData.getCurve(`object.${objectIdx}.exists`);
        const kf = existsCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = oldExists;
        } else {
          layer.animationData.addKeyframe(`object.${objectIdx}.exists`, new Keyframe(time, oldExists, "hold"));
        }
      }

      // For shapes: restore them with their original shapeIndex (fills the holes)
      for (let shapeIdx of action.shapes) {
        const shape = pointerList[shapeIdx];
        if (shape) {
          layer.addShape(shape, time);
        }
      }
      updateUI();
    },
  },
  addLayer: {
    create: () => {
      redoStack.length = 0;
      let action = {
        object: context.activeObject.idx,
        uuid: uuidv4(),
      };
      undoStack.push({ name: "addLayer", action: action });
      actions.addLayer.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let object = pointerList[action.object];
      let layer = new VectorLayer(action.uuid);
      layer.name = `VectorLayer ${object.layers.length + 1}`;
      object.layers.push(layer);
      object.currentLayer = object.layers.indexOf(layer);
      updateLayers();
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      let layer = pointerList[action.uuid];
      object.layers.splice(object.layers.indexOf(layer), 1);
      object.currentLayer = Math.min(
        object.currentLayer,
        object.layers.length - 1,
      );
      updateLayers();
    },
  },
  deleteLayer: {
    create: (layer) => {
      redoStack.length = 0;
      // Don't allow deleting the only layer
      if (context.activeObject.layers.length == 1) return;
      if (!(layer instanceof VectorLayer)) {
        layer = context.activeObject.activeLayer;
      }
      let action = {
        object: context.activeObject.idx,
        layer: layer.idx,
        index: context.activeObject.layers.indexOf(layer),
      };
      undoStack.push({ name: "deleteLayer", action: action });
      actions.deleteLayer.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let object = pointerList[action.object];
      let layer = pointerList[action.layer];
      let changelayer = false;
      if (object.activeLayer == layer) {
        changelayer = true;
      }
      object.layers.splice(object.layers.indexOf(layer), 1);
      if (changelayer) {
        object.currentLayer = 0;
      }
      updateUI();
      updateLayers();
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      let layer = pointerList[action.layer];
      object.layers.splice(action.index, 0, layer);
      updateUI();
      updateLayers();
    },
  },
  changeLayerName: {
    create: (layer, newName) => {
      redoStack.length = 0;
      let action = {
        layer: layer.idx,
        newName: newName,
        oldName: layer.name,
      };
      undoStack.push({ name: "changeLayerName", action: action });
      actions.changeLayerName.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      layer.name = action.newName;
      updateLayers();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      layer.name = action.oldName;
      updateLayers();
    },
  },
  importObject: {
    create: (object) => {
      redoStack.length = 0;
      let action = {
        object: object,
        activeObject: context.activeObject.idx,
      };
      undoStack.push({ name: "importObject", action: action });
      actions.importObject.execute(action);
      updateMenu();
    },
    execute: (action) => {
      const activeObject = pointerList[action.activeObject];
      switch (action.object.type) {
        case "GraphicsObject":
          let object = GraphicsObject.fromJSON(action.object);
          activeObject.addObject(object);
          break;
        case "VectorLayer":
          let layer = VectorLayer.fromJSON(action.object);
          activeObject.addLayer(layer);
      }
      updateUI();
      updateLayers();
    },
    rollback: (action) => {
      const activeObject = pointerList[action.activeObject];
      switch (action.object.type) {
        case "GraphicsObject":
          let object = pointerList[action.object.idx];
          activeObject.removeChild(object);
          break;
        case "VectorLayer":
          let layer = pointerList[action.object.idx];
          activeObject.removeLayer(layer);
      }
      updateUI();
      updateLayers();
    },
  },
  transformObjects: {
    initialize: (
      frame,
      _selection,
      direction,
      mouse,
      transform = undefined,
    ) => {
      let bbox = undefined;
      const selection = {};
      for (let item of _selection) {
        if (bbox == undefined) {
          bbox = getRotatedBoundingBox(item);
        } else {
          growBoundingBox(bbox, getRotatedBoundingBox(item));
        }
        selection[item.idx] = {
          x: item.x,
          y: item.y,
          scale_x: item.scale_x,
          scale_y: item.scale_y,
          rotation: item.rotation,
        };
      }
      let action = {
        type: "transformObjects",
        oldState: structuredClone(frame.keys),
        frame: frame.idx,
        transform: {
          initial: {
            x: { min: bbox.x.min, max: bbox.x.max },
            y: { min: bbox.y.min, max: bbox.y.max },
            rotation: 0,
            mouse: { x: mouse.x, y: mouse.y },
            selection: selection,
          },
          current: {
            x: { min: bbox.x.min, max: bbox.x.max },
            y: { min: bbox.y.min, max: bbox.y.max },
            scale_x: 1,
            scale_y: 1,
            rotation: 0,
            mouse: { x: mouse.x, y: mouse.y },
            selection: structuredClone(selection),
          },
        },
        selection: selection,
        direction: direction,
      };
      if (transform) {
        action.transform = transform;
      }
      return action;
    },
    update: (action, mouse) => {
      const initial = action.transform.initial;
      const current = action.transform.current;
      if (action.direction.indexOf("n") != -1) {
        current.y.min = mouse.y;
      } else if (action.direction.indexOf("s") != -1) {
        current.y.max = mouse.y;
      }
      if (action.direction.indexOf("w") != -1) {
        current.x.min = mouse.x;
      } else if (action.direction.indexOf("e") != -1) {
        current.x.max = mouse.x;
      }
      if (context.dragDirection == "r") {
        const pivot = {
          x: (initial.x.min + initial.x.max) / 2,
          y: (initial.y.min + initial.y.max) / 2,
        };
        current.rotation = signedAngleBetweenVectors(
          pivot,
          initial.mouse,
          mouse,
        );
        const { dx, dy } = rotateAroundPointIncremental(
          current.x.min,
          current.y.min,
          pivot,
          current.rotation,
        );
      }

      // Calculate the scaling factor based on the difference between current and initial values
      action.transform.current.scale_x =
        (current.x.max - current.x.min) / (initial.x.max - initial.x.min);
      action.transform.current.scale_y =
        (current.y.max - current.y.min) / (initial.y.max - initial.y.min);
      return action;
    },
    render: (action, ctx) => {
      const initial = action.transform.initial;
      const current = action.transform.current;
      ctx.save();
      ctx.translate(
        (current.x.max + current.x.min) / 2,
        (current.y.max - current.y.min) / 2,
      );
      ctx.rotate(current.rotation);
      ctx.translate(
        -(current.x.max + current.x.min) / 2,
        -(current.y.max - current.y.min) / 2,
      );
      const cxt = {
        ctx: ctx,
        selection: [],
        shapeselection: [],
      };
      for (let obj in action.selection) {
        const object = pointerList[obj];
        const transform = ctx.getTransform()
        ctx.translate(object.x, object.y)
        ctx.scale(object.scale_x, object.scale_y)
        ctx.rotate(object.rotation)
        object.draw(ctx)
        ctx.setTransform(transform)
      }
      ctx.strokeStyle = "#00ffff";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.rect(
        current.x.min,
        current.y.min,
        current.x.max - current.x.min,
        current.y.max - current.y.min,
      );
      ctx.stroke();
      ctx.fillStyle = "#000000";
      const rectRadius = 5;
      const xdiff = current.x.max - current.x.min;
      const ydiff = current.y.max - current.y.min;
      for (let i of [
        [0, 0],
        [0.5, 0],
        [1, 0],
        [1, 0.5],
        [1, 1],
        [0.5, 1],
        [0, 1],
        [0, 0.5],
      ]) {
        ctx.beginPath();
        ctx.rect(
          current.x.min + xdiff * i[0] - rectRadius,
          current.y.min + ydiff * i[1] - rectRadius,
          rectRadius * 2,
          rectRadius * 2,
        );
        ctx.fill();
      }
      ctx.restore();
    },
    finalize: (action) => {
      undoStack.push({ name: "transformObjects", action: action });
      actions.transformObjects.execute(action);
      context.activeAction = undefined;
      updateMenu();
    },
    execute: (action) => {
      const frame = pointerList[action.frame];
      const initial = action.transform.initial;
      const current = action.transform.current;
      const delta_x = current.x.min - initial.x.min;
      const delta_y = current.y.min - initial.y.min;
      const delta_rot = current.rotation - initial.rotation;
      // frame.keys = structuredClone(action.newState)
      for (let idx in action.selection) {
        const item = frame.keys[idx];
        const xoffset = action.selection[idx].x - initial.x.min;
        const yoffset = action.selection[idx].y - initial.y.min;
        item.x = initial.x.min + delta_x + xoffset * current.scale_x;
        item.y = initial.y.min + delta_y + yoffset * current.scale_y;
        item.scale_x = action.selection[idx].scale_x * current.scale_x;
        item.scale_y = action.selection[idx].scale_y * current.scale_y;
        item.rotation = action.selection[idx].rotation + delta_rot;
      }
      updateUI();
    },
    rollback: (action) => {
      let frame = pointerList[action.frame];
      frame.keys = structuredClone(action.oldState);
      updateUI();
    },
  },
  moveObjects: {
    initialize: (objects, layer, time) => {
      let oldPositions = {};
      let hadKeyframes = {};
      for (let obj of objects) {
        const xCurve = layer.animationData.getCurve(`child.${obj.idx}.x`);
        const yCurve = layer.animationData.getCurve(`child.${obj.idx}.y`);
        const xKf = xCurve?.getKeyframeAtTime(time);
        const yKf = yCurve?.getKeyframeAtTime(time);

        const x = layer.animationData.interpolate(`child.${obj.idx}.x`, time);
        const y = layer.animationData.interpolate(`child.${obj.idx}.y`, time);
        oldPositions[obj.idx] = { x, y };
        hadKeyframes[obj.idx] = { x: !!xKf, y: !!yKf };
      }
      let action = {
        type: "moveObjects",
        objects: objects.map(o => o.idx),
        layer: layer.idx,
        time: time,
        oldPositions: oldPositions,
        hadKeyframes: hadKeyframes,
      };
      return action;
    },
    finalize: (action) => {
      const layer = pointerList[action.layer];
      let newPositions = {};
      for (let objIdx of action.objects) {
        const obj = pointerList[objIdx];
        newPositions[objIdx] = { x: obj.x, y: obj.y };
      }
      action.newPositions = newPositions;
      undoStack.push({ name: "moveObjects", action: action });
      actions.moveObjects.execute(action);
      context.activeAction = undefined;
      updateMenu();
    },
    render: (action, ctx) => {},
    create: (objects, layer, time, oldPositions, newPositions) => {
      redoStack.length = 0;

      // Track which keyframes existed before the move
      let hadKeyframes = {};
      for (let obj of objects) {
        const xCurve = layer.animationData.getCurve(`child.${obj.idx}.x`);
        const yCurve = layer.animationData.getCurve(`child.${obj.idx}.y`);
        const xKf = xCurve?.getKeyframeAtTime(time);
        const yKf = yCurve?.getKeyframeAtTime(time);
        hadKeyframes[obj.idx] = { x: !!xKf, y: !!yKf };
      }

      let action = {
        objects: objects.map(o => o.idx),
        layer: layer.idx,
        time: time,
        oldPositions: oldPositions,
        newPositions: newPositions,
        hadKeyframes: hadKeyframes,
      };
      undoStack.push({ name: "moveObjects", action: action });
      actions.moveObjects.execute(action);
      updateMenu();
    },
    execute: (action) => {
      const layer = pointerList[action.layer];
      const time = action.time;

      for (let objIdx of action.objects) {
        const obj = pointerList[objIdx];
        const newPos = action.newPositions[objIdx];

        // Update object properties
        obj.x = newPos.x;
        obj.y = newPos.y;

        // Add/update keyframes in AnimationData
        const xCurve = layer.animationData.getCurve(`child.${objIdx}.x`);
        const kf = xCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = newPos.x;
        } else {
          layer.animationData.addKeyframe(`child.${objIdx}.x`, new Keyframe(time, newPos.x, "linear"));
        }

        const yCurve = layer.animationData.getCurve(`child.${objIdx}.y`);
        const kfy = yCurve?.getKeyframeAtTime(time);
        if (kfy) {
          kfy.value = newPos.y;
        } else {
          layer.animationData.addKeyframe(`child.${objIdx}.y`, new Keyframe(time, newPos.y, "linear"));
        }
      }
      updateUI();
    },
    rollback: (action) => {
      const layer = pointerList[action.layer];
      const time = action.time;

      for (let objIdx of action.objects) {
        const obj = pointerList[objIdx];
        const oldPos = action.oldPositions[objIdx];
        const hadKfs = action.hadKeyframes?.[objIdx] || { x: false, y: false };

        // Restore object properties
        obj.x = oldPos.x;
        obj.y = oldPos.y;

        // Restore or remove keyframes in AnimationData
        const xCurve = layer.animationData.getCurve(`child.${objIdx}.x`);
        if (hadKfs.x) {
          // Had a keyframe before - restore its value
          const kf = xCurve?.getKeyframeAtTime(time);
          if (kf) {
            kf.value = oldPos.x;
          } else {
            layer.animationData.addKeyframe(`child.${objIdx}.x`, new Keyframe(time, oldPos.x, "linear"));
          }
        } else {
          // No keyframe before - remove the entire curve
          if (xCurve) {
            layer.animationData.removeCurve(`child.${objIdx}.x`);
          }
        }

        const yCurve = layer.animationData.getCurve(`child.${objIdx}.y`);
        if (hadKfs.y) {
          // Had a keyframe before - restore its value
          const kfy = yCurve?.getKeyframeAtTime(time);
          if (kfy) {
            kfy.value = oldPos.y;
          } else {
            layer.animationData.addKeyframe(`child.${objIdx}.y`, new Keyframe(time, oldPos.y, "linear"));
          }
        } else {
          // No keyframe before - remove the entire curve
          if (yCurve) {
            layer.animationData.removeCurve(`child.${objIdx}.y`);
          }
        }
      }

      updateUI();
    },
  },
  editFrame: {
    // DEPRECATED: Kept for backwards compatibility
    initialize: (frame) => {
      console.warn("editFrame is deprecated, use moveObjects instead");
      return null;
    },
    finalize: (action, frame) => {},
    render: (action, ctx) => {},
    create: (frame) => {},
    execute: (action) => {},
    rollback: (action) => {},
  },
  addFrame: {
    create: () => {
      redoStack.length = 0;
      let frames = [];
      for (
        let i = context.activeObject.activeLayer.frames.length;
        i <= context.activeObject.currentFrameNum;
        i++
      ) {
        frames.push(uuidv4());
      }
      let action = {
        frames: frames,
        layer: context.activeObject.activeLayer.idx,
      };
      undoStack.push({ name: "addFrame", action: action });
      actions.addFrame.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      for (let frame of action.frames) {
        layer.frames.push(new Frame("normal", frame));
      }
      updateLayers();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      for (let _frame of action.frames) {
        layer.frames.pop();
      }
      updateLayers();
    },
  },
  addKeyframe: {
    create: () => {
      let frameNum = context.activeObject.currentFrameNum;
      let layer = context.activeObject.activeLayer;
      let formerType;
      let addedFrames = {};
      if (frameNum >= layer.frames.length) {
        formerType = "none";
        // for (let i = layer.frames.length; i <= frameNum; i++) {
        //   addedFrames[i] = uuidv4();
        // }
      } else if (!layer.frames[frameNum]) {
        formerType = undefined
      } else if (layer.frames[frameNum].frameType != "keyframe") {
        formerType = layer.frames[frameNum].frameType;
      } else {
        return; // Already a keyframe, nothing to do
      }
      redoStack.length = 0;
      let action = {
        frameNum: frameNum,
        object: context.activeObject.idx,
        layer: layer.idx,
        formerType: formerType,
        addedFrames: addedFrames,
        uuid: uuidv4(),
      };
      undoStack.push({ name: "addKeyframe", action: action });
      actions.addKeyframe.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let object = pointerList[action.object];
      let layer = pointerList[action.layer];
      layer.addOrChangeFrame(
        action.frameNum,
        "keyframe",
        action.uuid,
        action.addedFrames,
      );
      updateLayers();
      updateUI();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      if (action.formerType == "none") {
        for (let i in action.addedFrames) {
          layer.frames.pop();
        }
      } else {
        let layer = pointerList[action.layer];
        if (action.formerType) {
          layer.frames[action.frameNum].frameType = action.formerType;
        } else {
          layer.frames[action.frameNum = undefined]
        }
      }
      updateLayers();
      updateUI();
    },
  },
  deleteFrame: {
    create: (frame, layer) => {
      redoStack.length = 0;
      let action = {
        frame: frame.idx,
        layer: layer.idx,
        replacementUuid: uuidv4(),
      };
      undoStack.push({ name: "deleteFrame", action: action });
      actions.deleteFrame.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      layer.deleteFrame(
        action.frame,
        undefined,
        action.replacementUuid ? action.replacementUuid : uuidv4(),
      );
      updateLayers();
      updateUI();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      let frame = pointerList[action.frame];
      layer.addFrame(action.frameNum, frame, {});
      updateLayers();
      updateUI();
    },
  },
  moveFrames: {
    create: (offset) => {
      redoStack.length = 0;
      const selectedFrames = structuredClone(context.selectedFrames);
      for (let frame of selectedFrames) {
        frame.replacementUuid = uuidv4();
        frame.layer = context.activeObject.layers.length - frame.layer - 1;
      }
      // const fillFrames = []
      // for (let i=0; i<context.activeObject.layers.length;i++) {
      //   const fillLayer = []
      //   for (let j=0; j<Math.abs(offset.frames); j++) {
      //     fillLayer.push(uuidv4())
      //   }
      //   fillFrames.push(fillLayer)
      // }
      let action = {
        selectedFrames: selectedFrames,
        offset: offset,
        object: context.activeObject.idx,
        // fillFrames: fillFrames
      };
      undoStack.push({ name: "moveFrames", action: action });
      actions.moveFrames.execute(action);
      updateMenu();
    },
    execute: (action) => {
      const object = pointerList[action.object];
      const frameBuffer = [];
      for (let frameObj of action.selectedFrames) {
        let layer = object.layers[frameObj.layer];
        let frame = layer.frames[frameObj.frameNum];
        if (frameObj) {
          frameBuffer.push({
            frame: frame,
            frameNum: frameObj.frameNum,
            layer: frameObj.layer,
          });
          layer.deleteFrame(frame.idx, undefined, frameObj.replacementUuid);
        }
      }
      for (let frameObj of frameBuffer) {
        // TODO: figure out object tracking when moving frames between layers
        const layer_idx = frameObj.layer// + action.offset.layers;
        let layer = object.layers[layer_idx];
        let frame = frameObj.frame;
        layer.addFrame(frameObj.frameNum + action.offset.frames, frame, []); //fillFrames[layer_idx])
      }
      updateLayers();
      updateUI();
    },
    rollback: (action) => {
      const object = pointerList[action.object];
      const frameBuffer = [];
      for (let frameObj of action.selectedFrames) {
        let layer = object.layers[frameObj.layer];
        let frame = layer.frames[frameObj.frameNum + action.offset.frames];
        if (frameObj) {
          frameBuffer.push({
            frame: frame,
            frameNum: frameObj.frameNum,
            layer: frameObj.layer,
          });
          layer.deleteFrame(frame.idx, "none")
        }
      }
      for (let frameObj of frameBuffer) {
        let layer = object.layers[frameObj.layer];
        let frame = frameObj.frame;
        if (frameObj) {
          layer.addFrame(frameObj.frameNum, frame, [])
        }
      }
    },
  },
  addMotionTween: {
    create: () => {
      redoStack.length = 0;
      let frameNum = context.activeObject.currentFrameNum;
      let layer = context.activeObject.activeLayer;

      const frameInfo = layer.getFrameValue(frameNum)
      let lastKeyframeBefore, firstKeyframeAfter
      if (frameInfo.valueAtN) {
        lastKeyframeBefore = frameNum
      } else if (frameInfo.prev) {
        lastKeyframeBefore = frameInfo.prevIndex
      } else {
        return
      }
      firstKeyframeAfter = frameInfo.nextIndex

      let action = {
        frameNum: frameNum,
        layer: layer.idx,
        lastBefore: lastKeyframeBefore,
        firstAfter: firstKeyframeAfter,
      };
      undoStack.push({ name: "addMotionTween", action: action });
      actions.addMotionTween.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      let frames = layer.frames;
      if (action.lastBefore != undefined) {
        console.log("adding motion")
        frames[action.lastBefore].keyTypes.add("motion")
      }
      updateLayers();
      updateUI();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      let frames = layer.frames;
      if (action.lastBefore != undefined) {
        frames[action.lastBefore].keyTypes.delete("motion")
      }
      updateLayers();
      updateUI();
    },
  },
  addShapeTween: {
    create: () => {
      redoStack.length = 0;
      let frameNum = context.activeObject.currentFrameNum;
      let layer = context.activeObject.activeLayer;

      const frameInfo = layer.getFrameValue(frameNum)
      let lastKeyframeBefore, firstKeyframeAfter
      if (frameInfo.valueAtN) {
        lastKeyframeBefore = frameNum
      } else if (frameInfo.prev) {
        lastKeyframeBefore = frameInfo.prevIndex
      } else {
        return
      }
      firstKeyframeAfter = frameInfo.nextIndex


      let action = {
        frameNum: frameNum,
        layer: layer.idx,
        lastBefore: lastKeyframeBefore,
        firstAfter: firstKeyframeAfter,
      };
      console.log(action)
      undoStack.push({ name: "addShapeTween", action: action });
      actions.addShapeTween.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      let frames = layer.frames;
      if (action.lastBefore != undefined) {
        frames[action.lastBefore].keyTypes.add("shape")
      }
      updateLayers();
      updateUI();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      let frames = layer.frames;
      if (action.lastBefore != undefined) {
        frames[action.lastBefore].keyTypes.delete("shape")
      }
      updateLayers();
      updateUI();
    },
  },
  group: {
    create: () => {
      redoStack.length = 0;
      let serializableShapes = [];
      let serializableObjects = [];
      let bbox;
      const currentTime = context.activeObject?.currentTime || 0;
      const layer = context.activeObject.activeLayer;

      // For shapes - use AnimationData system
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx);
        if (bbox == undefined) {
          bbox = shape.bbox();
        } else {
          growBoundingBox(bbox, shape.bbox());
        }
      }

      // For objects - check if they exist at current time
      for (let object of context.selection) {
        const existsValue = layer.animationData.interpolate(`object.${object.idx}.exists`, currentTime);
        if (existsValue > 0) {
          serializableObjects.push(object.idx);
          // TODO: rotated bbox
          if (bbox == undefined) {
            bbox = object.bbox();
          } else {
            growBoundingBox(bbox, object.bbox());
          }
        }
      }

      // If nothing was selected, don't create a group
      if (!bbox) {
        return;
      }

      context.shapeselection = [];
      context.selection = [];
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        groupUuid: uuidv4(),
        parent: context.activeObject.idx,
        layer: layer.idx,
        currentTime: currentTime,
        position: {
          x: (bbox.x.min + bbox.x.max) / 2,
          y: (bbox.y.min + bbox.y.max) / 2,
        },
      };
      undoStack.push({ name: "group", action: action });
      actions.group.execute(action);
      updateMenu();
      updateLayers();
    },
    execute: (action) => {
      let group = new GraphicsObject(action.groupUuid);
      let parent = pointerList[action.parent];
      let layer = pointerList[action.layer] || parent.activeLayer;
      const currentTime = action.currentTime || 0;

      // Move shapes from parent layer to group's first layer
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        shape.translate(-action.position.x, -action.position.y);

        // Remove shape from parent layer's shapes array
        let shapeIndex = layer.shapes.indexOf(shape);
        if (shapeIndex !== -1) {
          layer.shapes.splice(shapeIndex, 1);
        }

        // Remove animation curves for this shape from parent layer
        layer.animationData.removeCurve(`shape.${shape.shapeId}.exists`);
        layer.animationData.removeCurve(`shape.${shape.shapeId}.zOrder`);
        layer.animationData.removeCurve(`shape.${shape.shapeId}.shapeIndex`);

        // Add shape to group's first layer
        let groupLayer = group.activeLayer;
        shape.parent = groupLayer;
        groupLayer.shapes.push(shape);

        // Add animation curves for this shape in group's layer
        let existsCurve = new AnimationCurve(`shape.${shape.shapeId}.exists`);
        existsCurve.addKeyframe(new Keyframe(0, 1, 'linear'));
        groupLayer.animationData.setCurve(`shape.${shape.shapeId}.exists`, existsCurve);

        let zOrderCurve = new AnimationCurve(`shape.${shape.shapeId}.zOrder`);
        zOrderCurve.addKeyframe(new Keyframe(0, groupLayer.shapes.length - 1, 'linear'));
        groupLayer.animationData.setCurve(`shape.${shape.shapeId}.zOrder`, zOrderCurve);

        let shapeIndexCurve = new AnimationCurve(`shape.${shape.shapeId}.shapeIndex`);
        shapeIndexCurve.addKeyframe(new Keyframe(0, 0, 'linear'));
        groupLayer.animationData.setCurve(`shape.${shape.shapeId}.shapeIndex`, shapeIndexCurve);
      }

      // Move objects (children) to the group
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];

        // Get object position from AnimationData if available
        const objX = layer.animationData.interpolate(`object.${objectIdx}.x`, currentTime);
        const objY = layer.animationData.interpolate(`object.${objectIdx}.y`, currentTime);

        if (objX !== null && objY !== null) {
          group.addObject(
            object,
            objX - action.position.x,
            objY - action.position.y,
            currentTime
          );
        } else {
          group.addObject(object, 0, 0, currentTime);
        }
        parent.removeChild(object);
      }

      // Add group to parent using time-based API
      parent.addObject(group, action.position.x, action.position.y, currentTime);
      context.selection = [group];
      context.activeCurve = undefined;
      context.activeVertex = undefined;
      updateUI();
      updateInfopanel();
    },
    rollback: (action) => {
      let group = pointerList[action.groupUuid];
      let parent = pointerList[action.parent];
      const layer = pointerList[action.layer] || parent.activeLayer;
      const currentTime = action.currentTime || 0;

      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        shape.translate(action.position.x, action.position.y);
        layer.addShape(shape, currentTime);
        group.activeLayer.removeShape(shape);
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        parent.addObject(object, object.x, object.y, currentTime);
        group.removeChild(object);
      }
      parent.removeChild(group);
      updateUI();
      updateInfopanel();
    },
  },
  sendToBack: {
    create: () => {
      redoStack.length = 0;
      const currentTime = context.activeObject.currentTime || 0;
      const layer = context.activeObject.activeLayer;

      let serializableShapes = [];
      let oldZOrders = {};

      // Store current zOrder for each shape
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx);
        const zOrder = layer.animationData.interpolate(`shape.${shape.shapeId}.zOrder`, currentTime);
        oldZOrders[shape.idx] = zOrder !== null ? zOrder : 0;
      }

      let serializableObjects = [];
      let formerIndices = {};
      for (let object of context.selection) {
        serializableObjects.push(object.idx);
        formerIndices[object.idx] = layer.children.indexOf(object);
      }

      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        layer: layer.idx,
        time: currentTime,
        oldZOrders: oldZOrders,
        formerIndices: formerIndices,
      };
      undoStack.push({ name: "sendToBack", action: action });
      actions.sendToBack.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      const time = action.time;

      // For shapes: set zOrder to 0, increment all others
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];

        // Increment zOrder for all other shapes at this time
        for (let otherShape of layer.shapes) {
          if (otherShape.shapeId !== shape.shapeId) {
            const zOrderCurve = layer.animationData.getCurve(`shape.${otherShape.shapeId}.zOrder`);
            if (zOrderCurve) {
              const kf = zOrderCurve.getKeyframeAtTime(time);
              if (kf) {
                kf.value += 1;
              } else {
                // Add keyframe at current time with incremented value
                const currentZOrder = layer.animationData.interpolate(`shape.${otherShape.shapeId}.zOrder`, time) || 0;
                layer.animationData.addKeyframe(`shape.${otherShape.shapeId}.zOrder`, new Keyframe(time, currentZOrder + 1, "hold"));
              }
            }
          }
        }

        // Set this shape's zOrder to 0
        const zOrderCurve = layer.animationData.getCurve(`shape.${shape.shapeId}.zOrder`);
        const kf = zOrderCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = 0;
        } else {
          layer.animationData.addKeyframe(`shape.${shape.shapeId}.zOrder`, new Keyframe(time, 0, "hold"));
        }
      }

      // For objects: move to front of children array
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        layer.children.splice(layer.children.indexOf(object), 1);
        layer.children.unshift(object);
      }
      updateUI();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      const time = action.time;

      // Restore old zOrder values for shapes
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        const oldZOrder = action.oldZOrders[shapeIdx];

        const zOrderCurve = layer.animationData.getCurve(`shape.${shape.shapeId}.zOrder`);
        const kf = zOrderCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = oldZOrder;
        } else {
          layer.animationData.addKeyframe(`shape.${shape.shapeId}.zOrder`, new Keyframe(time, oldZOrder, "hold"));
        }
      }

      // Restore old positions for objects
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        layer.children.splice(layer.children.indexOf(object), 1);
        layer.children.splice(action.formerIndices[objectIdx], 0, object);
      }
      updateUI();
    },
  },
  bringToFront: {
    create: () => {
      redoStack.length = 0;
      const currentTime = context.activeObject.currentTime || 0;
      const layer = context.activeObject.activeLayer;

      let serializableShapes = [];
      let oldZOrders = {};

      // Store current zOrder for each shape
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx);
        const zOrder = layer.animationData.interpolate(`shape.${shape.shapeId}.zOrder`, currentTime);
        oldZOrders[shape.idx] = zOrder !== null ? zOrder : 0;
      }

      let serializableObjects = [];
      let formerIndices = {};
      for (let object of context.selection) {
        serializableObjects.push(object.idx);
        formerIndices[object.idx] = layer.children.indexOf(object);
      }

      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        layer: layer.idx,
        time: currentTime,
        oldZOrders: oldZOrders,
        formerIndices: formerIndices,
      };
      undoStack.push({ name: "bringToFront", action: action });
      actions.bringToFront.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let layer = pointerList[action.layer];
      const time = action.time;

      // Find max zOrder at this time
      let maxZOrder = -1;
      for (let shape of layer.shapes) {
        const zOrder = layer.animationData.interpolate(`shape.${shape.shapeId}.zOrder`, time);
        if (zOrder !== null && zOrder > maxZOrder) {
          maxZOrder = zOrder;
        }
      }

      // For shapes: set zOrder to max+1, max+2, etc.
      let newZOrder = maxZOrder + 1;
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];

        const zOrderCurve = layer.animationData.getCurve(`shape.${shape.shapeId}.zOrder`);
        const kf = zOrderCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = newZOrder;
        } else {
          layer.animationData.addKeyframe(`shape.${shape.shapeId}.zOrder`, new Keyframe(time, newZOrder, "hold"));
        }
        newZOrder++;
      }

      // For objects: move to end of children array
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        layer.children.splice(layer.children.indexOf(object), 1);
        object.parentLayer = layer;
        layer.children.push(object);
      }
      updateUI();
    },
    rollback: (action) => {
      let layer = pointerList[action.layer];
      const time = action.time;

      // Restore old zOrder values for shapes
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        const oldZOrder = action.oldZOrders[shapeIdx];

        const zOrderCurve = layer.animationData.getCurve(`shape.${shape.shapeId}.zOrder`);
        const kf = zOrderCurve?.getKeyframeAtTime(time);
        if (kf) {
          kf.value = oldZOrder;
        } else {
          layer.animationData.addKeyframe(`shape.${shape.shapeId}.zOrder`, new Keyframe(time, oldZOrder, "hold"));
        }
      }

      // Restore old positions for objects
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        layer.children.splice(layer.children.indexOf(object), 1);
        layer.children.splice(action.formerIndices[objectIdx], 0, object);
      }
      updateUI();
    },
  },
  setName: {
    create: (object, name) => {
      redoStack.length = 0;
      let action = {
        object: object.idx,
        newName: name,
        oldName: object.name,
      };
      undoStack.push({ name: "setName", action: action });
      actions.setName.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let object = pointerList[action.object];
      object.name = action.newName;
      updateInfopanel();
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      object.name = action.oldName;
      updateInfopanel();
    },
  },
  selectAll: {
    create: () => {
      redoStack.length = 0;
      let selection = [];
      let shapeselection = [];
      const currentTime = context.activeObject.currentTime || 0;
      const layer = context.activeObject.activeLayer;
      for (let child of layer.children) {
        let idx = child.idx;
        const existsValue = layer.animationData.interpolate(`object.${idx}.exists`, currentTime);
        if (existsValue > 0) {
          selection.push(child.idx);
        }
      }
      // Use getVisibleShapes instead of currentFrame.shapes
      if (layer) {
        for (let shape of layer.getVisibleShapes(currentTime)) {
          shapeselection.push(shape.idx);
        }
      }
      let action = {
        selection: selection,
        shapeselection: shapeselection,
      };
      undoStack.push({ name: "selectAll", action: action });
      actions.selectAll.execute(action);
      updateMenu();
    },
    execute: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.selection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.shapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
    rollback: (action) => {
      context.selection = [];
      context.shapeselection = [];
      updateUI();
      updateMenu();
    },
  },
  selectNone: {
    create: () => {
      redoStack.length = 0;
      let selection = [];
      let shapeselection = [];
      for (let item of context.selection) {
        selection.push(item.idx);
      }
      for (let shape of context.shapeselection) {
        shapeselection.push(shape.idx);
      }
      let action = {
        selection: selection,
        shapeselection: shapeselection,
      };
      undoStack.push({ name: "selectNone", action: action });
      actions.selectNone.execute(action);
      updateMenu();
    },
    execute: (action) => {
      context.selection = [];
      context.shapeselection = [];
      updateUI();
      updateMenu();
    },
    rollback: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.selection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.shapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
  },
  select: {
    create: () => {
      redoStack.length = 0;
      if (
        arraysAreEqual(context.oldselection, context.selection) &&
        arraysAreEqual(context.oldshapeselection, context.shapeselection)
      )
        return;
      let oldselection = [];
      let oldshapeselection = [];
      for (let item of context.oldselection) {
        oldselection.push(item.idx);
      }
      for (let shape of context.oldshapeselection) {
        oldshapeselection.push(shape.idx);
      }
      let selection = [];
      let shapeselection = [];
      for (let item of context.selection) {
        selection.push(item.idx);
      }
      for (let shape of context.shapeselection) {
        shapeselection.push(shape.idx);
      }
      let action = {
        selection: selection,
        shapeselection: shapeselection,
        oldselection: oldselection,
        oldshapeselection: oldshapeselection,
      };
      undoStack.push({ name: "select", action: action });
      actions.select.execute(action);
      updateMenu();
    },
    execute: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.selection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.shapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
    rollback: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.oldselection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.oldshapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
  },
  // Node graph actions
  graphAddNode: {
    create: (trackId, nodeType, position, nodeId, backendId) => {
      redoStack.length = 0;
      let action = {
        trackId: trackId,
        nodeType: nodeType,
        position: position,
        nodeId: nodeId, // Frontend node ID from Drawflow
        backendId: backendId
      };
      undoStack.push({ name: "graphAddNode", action: action });
      actions.graphAddNode.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      // Re-add node via Tauri and reload frontend
      const result = await invoke('graph_add_node', {
        trackId: action.trackId,
        nodeType: action.nodeType,
        posX: action.position.x,
        posY: action.position.y
      });

      // If this is an AutomationInput node, create a timeline curve for it
      if (action.nodeType === 'AutomationInput') {
        await initializeAutomationCurve(action.trackId, result);
      }

      // Reload the entire graph to show the restored node
      if (context.reloadNodeEditor) {
        await context.reloadNodeEditor();
      }
    },
    rollback: async (action) => {
      // Remove node from backend
      await invoke('graph_remove_node', {
        trackId: action.trackId,
        nodeId: action.backendId
      });
      // Remove from frontend
      if (context.nodeEditor) {
        context.nodeEditor.removeNodeId(`node-${action.nodeId}`);
      }
    },
  },
  graphRemoveNode: {
    create: (trackId, nodeId, backendId, nodeData) => {
      redoStack.length = 0;
      let action = {
        trackId: trackId,
        nodeId: nodeId,
        backendId: backendId,
        nodeData: nodeData, // Store full node data for restoration
      };
      undoStack.push({ name: "graphRemoveNode", action: action });
      actions.graphRemoveNode.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      await invoke('graph_remove_node', {
        trackId: action.trackId,
        nodeId: action.backendId
      });
      if (context.nodeEditor) {
        context.nodeEditor.removeNodeId(`node-${action.nodeId}`);
      }
    },
    rollback: async (action) => {
      // Re-add node to backend
      const result = await invoke('graph_add_node', {
        trackId: action.trackId,
        nodeType: action.nodeData.nodeType,
        posX: action.nodeData.position.x,
        posY: action.nodeData.position.y
      });

      // Store new backend ID
      const newBackendId = result.node_id || result;

      // Re-add to frontend via reloadGraph
      if (context.reloadNodeEditor) {
        await context.reloadNodeEditor();
      }
    },
  },
  graphAddConnection: {
    create: (trackId, fromNode, fromPort, toNode, toPort, frontendFromId, frontendToId, fromPortClass, toPortClass) => {
      redoStack.length = 0;
      let action = {
        trackId: trackId,
        fromNode: fromNode,
        fromPort: fromPort,
        toNode: toNode,
        toPort: toPort,
        frontendFromId: frontendFromId,
        frontendToId: frontendToId,
        fromPortClass: fromPortClass,
        toPortClass: toPortClass
      };
      undoStack.push({ name: "graphAddConnection", action: action });
      actions.graphAddConnection.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      // Suppress action recording during undo/redo
      if (context.nodeEditorState) {
        context.nodeEditorState.suppressActionRecording = true;
      }

      try {
        await invoke('graph_connect', {
          trackId: action.trackId,
          fromNode: action.fromNode,
          fromPort: action.fromPort,
          toNode: action.toNode,
          toPort: action.toPort
        });
        // Add connection in frontend only if it doesn't exist
        if (context.nodeEditor) {
          const inputNode = context.nodeEditor.getNodeFromId(action.frontendToId);
          const inputConnections = inputNode?.inputs[action.toPortClass]?.connections;
          const alreadyConnected = inputConnections?.some(conn =>
            conn.node === action.frontendFromId && conn.input === action.fromPortClass
          );

          if (!alreadyConnected) {
            context.nodeEditor.addConnection(
              action.frontendFromId,
              action.frontendToId,
              action.fromPortClass,
              action.toPortClass
            );
          }
        }

        // Auto-name AutomationInput nodes when connected
        await updateAutomationName(action.trackId, action.fromNode, action.toNode, action.toPortClass);
      } finally {
        if (context.nodeEditorState) {
          context.nodeEditorState.suppressActionRecording = false;
        }
      }
    },
    rollback: async (action) => {
      // Suppress action recording during undo/redo
      if (context.nodeEditorState) {
        context.nodeEditorState.suppressActionRecording = true;
      }

      try {
        await invoke('graph_disconnect', {
          trackId: action.trackId,
          fromNode: action.fromNode,
          fromPort: action.fromPort,
          toNode: action.toNode,
          toPort: action.toPort
        });
        // Remove from frontend
        if (context.nodeEditor) {
          context.nodeEditor.removeSingleConnection(
            action.frontendFromId,
            action.frontendToId,
            action.fromPortClass,
            action.toPortClass
          );
        }
      } finally {
        if (context.nodeEditorState) {
          context.nodeEditorState.suppressActionRecording = false;
        }
      }
    },
  },
  graphRemoveConnection: {
    create: (trackId, fromNode, fromPort, toNode, toPort, frontendFromId, frontendToId, fromPortClass, toPortClass) => {
      redoStack.length = 0;
      let action = {
        trackId: trackId,
        fromNode: fromNode,
        fromPort: fromPort,
        toNode: toNode,
        toPort: toPort,
        frontendFromId: frontendFromId,
        frontendToId: frontendToId,
        fromPortClass: fromPortClass,
        toPortClass: toPortClass
      };
      undoStack.push({ name: "graphRemoveConnection", action: action });
      actions.graphRemoveConnection.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      // Suppress action recording during undo/redo
      if (context.nodeEditorState) {
        context.nodeEditorState.suppressActionRecording = true;
      }

      try {
        await invoke('graph_disconnect', {
          trackId: action.trackId,
          fromNode: action.fromNode,
          fromPort: action.fromPort,
          toNode: action.toNode,
          toPort: action.toPort
        });
        if (context.nodeEditor) {
          context.nodeEditor.removeSingleConnection(
            action.frontendFromId,
            action.frontendToId,
            action.fromPortClass,
            action.toPortClass
          );
        }
      } finally {
        if (context.nodeEditorState) {
          context.nodeEditorState.suppressActionRecording = false;
        }
      }
    },
    rollback: async (action) => {
      // Suppress action recording during undo/redo
      if (context.nodeEditorState) {
        context.nodeEditorState.suppressActionRecording = true;
      }

      try {
        await invoke('graph_connect', {
          trackId: action.trackId,
          fromNode: action.fromNode,
          fromPort: action.fromPort,
          toNode: action.toNode,
          toPort: action.toPort
        });
        // Re-add connection in frontend
        if (context.nodeEditor) {
          context.nodeEditor.addConnection(
            action.frontendFromId,
            action.frontendToId,
            action.fromPortClass,
            action.toPortClass
          );
        }
      } finally {
        if (context.nodeEditorState) {
          context.nodeEditorState.suppressActionRecording = false;
        }
      }
    },
  },
  graphSetParameter: {
    initialize: (trackId, nodeId, paramId, frontendNodeId, currentValue) => {
      return {
        trackId: trackId,
        nodeId: nodeId,
        paramId: paramId,
        frontendNodeId: frontendNodeId,
        oldValue: currentValue,
      };
    },
    finalize: (action, newValue) => {
      action.newValue = newValue;
      // Only record if value actually changed
      if (action.oldValue !== action.newValue) {
        undoStack.push({ name: "graphSetParameter", action: action });
        updateMenu();
      }
    },
    create: (trackId, nodeId, paramId, frontendNodeId, newValue, oldValue) => {
      redoStack.length = 0;
      let action = {
        trackId: trackId,
        nodeId: nodeId,
        paramId: paramId,
        frontendNodeId: frontendNodeId,
        newValue: newValue,
        oldValue: oldValue,
      };
      undoStack.push({ name: "graphSetParameter", action: action });
      actions.graphSetParameter.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      await invoke('graph_set_parameter', {
        trackId: action.trackId,
        nodeId: action.nodeId,
        paramId: action.paramId,
        value: action.newValue
      });
      // Update frontend slider if it exists
      const slider = document.querySelector(`#node-${action.frontendNodeId} input[data-param="${action.paramId}"]`);
      if (slider) {
        slider.value = action.newValue;
        // Trigger display update
        slider.dispatchEvent(new Event('input'));
      }
    },
    rollback: async (action) => {
      await invoke('graph_set_parameter', {
        trackId: action.trackId,
        nodeId: action.nodeId,
        paramId: action.paramId,
        value: action.oldValue
      });
      // Update frontend slider
      const slider = document.querySelector(`#node-${action.frontendNodeId} input[data-param="${action.paramId}"]`);
      if (slider) {
        slider.value = action.oldValue;
        slider.dispatchEvent(new Event('input'));
      }
    },
  },
  graphMoveNode: {
    create: (trackId, nodeId, oldPosition, newPosition) => {
      redoStack.length = 0;
      let action = {
        trackId: trackId,
        nodeId: nodeId,
        oldPosition: oldPosition,
        newPosition: newPosition,
      };
      undoStack.push({ name: "graphMoveNode", action: action });
      // Don't call execute - movement already happened in UI
      updateMenu();
    },
    execute: (action) => {
      // Move node in frontend
      if (context.nodeEditor) {
        const node = context.nodeEditor.getNodeFromId(action.nodeId);
        if (node) {
          context.nodeEditor.drawflow.drawflow[context.nodeEditor.module].data[action.nodeId].pos_x = action.newPosition.x;
          context.nodeEditor.drawflow.drawflow[context.nodeEditor.module].data[action.nodeId].pos_y = action.newPosition.y;
          // Update visual position
          const nodeElement = document.getElementById(`node-${action.nodeId}`);
          if (nodeElement) {
            nodeElement.style.left = action.newPosition.x + 'px';
            nodeElement.style.top = action.newPosition.y + 'px';
          }
          context.nodeEditor.updateConnectionNodes(`node-${action.nodeId}`);
        }
      }
    },
    rollback: (action) => {
      // Move node back to old position
      if (context.nodeEditor) {
        const node = context.nodeEditor.getNodeFromId(action.nodeId);
        if (node) {
          context.nodeEditor.drawflow.drawflow[context.nodeEditor.module].data[action.nodeId].pos_x = action.oldPosition.x;
          context.nodeEditor.drawflow.drawflow[context.nodeEditor.module].data[action.nodeId].pos_y = action.oldPosition.y;
          const nodeElement = document.getElementById(`node-${action.nodeId}`);
          if (nodeElement) {
            nodeElement.style.left = action.oldPosition.x + 'px';
            nodeElement.style.top = action.oldPosition.y + 'px';
          }
          context.nodeEditor.updateConnectionNodes(`node-${action.nodeId}`);
        }
      }
    },
  },
};

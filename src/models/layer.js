// Layer models: VectorLayer, AudioTrack, and VideoLayer classes

import { context, config, pointerList } from '../state.js';
import { Frame, AnimationData, Keyframe, tempFrame } from './animation.js';
import { Widget } from '../widgets.js';
import { Bezier } from '../bezier.js';
import {
  lerp,
  lerpColor,
  getKeyframesSurrounding,
  growBoundingBox,
  floodFillRegion,
  getShapeAtPoint,
  generateWaveform
} from '../utils.js';

// External libraries (globals)
const Tone = window.Tone;

// Tauri API
const { invoke, Channel } = window.__TAURI__.core;

// Helper function for UUID generation
function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

// Forward declarations for circular dependencies
// These will be set by main.js after all modules are loaded
let GraphicsObject = null;
let Shape = null;
let TempShape = null;
let updateUI = null;
let updateMenu = null;
let updateLayers = null;
let vectorDist = null;
let minSegmentSize = null;
let debugQuadtree = null;
let debugCurves = null;
let debugPoints = null;
let debugPaintbucket = null;
let d3 = null;
let actions = null;

// Initialize function to be called from main.js
export function initializeLayerDependencies(deps) {
  GraphicsObject = deps.GraphicsObject;
  Shape = deps.Shape;
  TempShape = deps.TempShape;
  updateUI = deps.updateUI;
  updateMenu = deps.updateMenu;
  updateLayers = deps.updateLayers;
  vectorDist = deps.vectorDist;
  minSegmentSize = deps.minSegmentSize;
  debugQuadtree = deps.debugQuadtree;
  debugCurves = deps.debugCurves;
  debugPoints = deps.debugPoints;
  debugPaintbucket = deps.debugPaintbucket;
  d3 = deps.d3;
  actions = deps.actions;
}

class VectorLayer extends Widget {
  constructor(uuid, parentObject = null) {
    super(0,0)
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    this.name = "VectorLayer";
    // LEGACY: Keep frames array for backwards compatibility during migration
    this.frames = [new Frame("keyframe", this.idx + "-F1")];
    this.animationData = new AnimationData(this);
    this.parentObject = parentObject; // Reference to parent GraphicsObject (for nested objects)
    // this.frameNum = 0;
    this.visible = true;
    this.audible = true;
    pointerList[this.idx] = this;
    this.children = []
    this.shapes = []
  }
  static fromJSON(json, parentObject = null) {
    const layer = new VectorLayer(json.idx, parentObject);
    for (let i in json.children) {
      const child = json.children[i];
      const childObject = GraphicsObject.fromJSON(child);
      childObject.parentLayer = layer;
      layer.children.push(childObject);
    }
    layer.name = json.name;

    // Load animation data if present (new system)
    if (json.animationData) {
      layer.animationData = AnimationData.fromJSON(json.animationData, layer);
    }

    // Load shapes if present
    if (json.shapes) {
      layer.shapes = json.shapes.map(shape => Shape.fromJSON(shape, layer));
    }

    // Load frames if present (old system - for backwards compatibility)
    if (json.frames) {
      layer.frames = [];
      for (let i in json.frames) {
        const frame = json.frames[i];
        if (!frame) {
          layer.frames.push(undefined)
          continue;
        }
        if (frame.frameType=="keyframe") {
          layer.frames.push(Frame.fromJSON(frame));
        } else {
          if (layer.frames[layer.frames.length-1]) {
            if (frame.frameType == "motion") {
              layer.frames[layer.frames.length-1].keyTypes.add("motion")
            } else if (frame.frameType == "shape") {
              layer.frames[layer.frames.length-1].keyTypes.add("shape")
            }
          }
          layer.frames.push(undefined)
        }
      }
    }

    layer.visible = json.visible;
    layer.audible = json.audible;

    return layer;
  }
  toJSON(randomizeUuid = false) {
    const json = {};
    json.type = "VectorLayer";
    if (randomizeUuid) {
      json.idx = uuidv4();
      json.name = this.name + " copy";
    } else {
      json.idx = this.idx;
      json.name = this.name;
    }
    json.children = [];
    let idMap = {}
    for (let child of this.children) {
      let childJson = child.toJSON(randomizeUuid)
      idMap[child.idx] = childJson.idx
      json.children.push(childJson);
    }

    // Serialize animation data (new system)
    json.animationData = this.animationData.toJSON();

    // If randomizing UUIDs, update the curve parameter keys to use new child IDs
    if (randomizeUuid && json.animationData.curves) {
      const newCurves = {};
      for (let paramKey in json.animationData.curves) {
        // paramKey format: "childId.property"
        const parts = paramKey.split('.');
        if (parts.length >= 2) {
          const oldChildId = parts[0];
          const property = parts.slice(1).join('.');
          if (oldChildId in idMap) {
            const newParamKey = `${idMap[oldChildId]}.${property}`;
            newCurves[newParamKey] = json.animationData.curves[paramKey];
            newCurves[newParamKey].parameter = newParamKey;
          } else {
            newCurves[paramKey] = json.animationData.curves[paramKey];
          }
        } else {
          newCurves[paramKey] = json.animationData.curves[paramKey];
        }
      }
      json.animationData.curves = newCurves;
    }

    // Serialize shapes
    json.shapes = this.shapes.map(shape => shape.toJSON(randomizeUuid));

    // Serialize frames (old system - for backwards compatibility)
    if (this.frames) {
      json.frames = [];
      for (let frame of this.frames) {
        if (frame) {
          let frameJson = frame.toJSON(randomizeUuid)
          for (let key in frameJson.keys) {
            if (key in idMap) {
              frameJson.keys[idMap[key]] = frameJson.keys[key]
            }
          }
          json.frames.push(frameJson);
        } else {
          json.frames.push(undefined)
        }
      }
    }

    json.visible = this.visible;
    json.audible = this.audible;
    return json;
  }
  // Get all animated property values for all children at a given time
  getAnimatedState(time) {
    const state = {
      shapes: [...this.shapes],  // Base shapes from layer
      childStates: {}            // Animated states for each child GraphicsObject
    };

    // For each child, get its animated properties at this time
    for (let child of this.children) {
      const childState = {};

      // Animatable properties for GraphicsObjects
      const properties = ['x', 'y', 'rotation', 'scale_x', 'scale_y', 'exists', 'shapeIndex'];

      for (let prop of properties) {
        const paramKey = `${child.idx}.${prop}`;
        const value = this.animationData.interpolate(paramKey, time);

        if (value !== null) {
          childState[prop] = value;
        }
      }

      if (Object.keys(childState).length > 0) {
        state.childStates[child.idx] = childState;
      }
    }

    return state;
  }

  // Helper method to add a keyframe for a child's property
  addKeyframeForChild(childId, property, time, value, interpolation = "linear") {
    const paramKey = `${childId}.${property}`;
    const keyframe = new Keyframe(time, value, interpolation);
    this.animationData.addKeyframe(paramKey, keyframe);
    return keyframe;
  }

  // Helper method to remove a keyframe
  removeKeyframeForChild(childId, property, keyframe) {
    const paramKey = `${childId}.${property}`;
    this.animationData.removeKeyframe(paramKey, keyframe);
  }

  // Helper method to get all keyframes for a child's property
  getKeyframesForChild(childId, property) {
    const paramKey = `${childId}.${property}`;
    const curve = this.animationData.getCurve(paramKey);
    return curve ? curve.keyframes : [];
  }

  /**
   * Add a shape to this layer at the given time
   * Creates AnimationData keyframes for exists, zOrder, and shapeIndex
   */
  addShape(shape, time, sendToBack = false) {
    // Add to shapes array
    this.shapes.push(shape);

    // Determine zOrder
    let zOrder;
    if (sendToBack) {
      zOrder = 0;
      // Increment zOrder for all existing shapes at this time
      for (let existingShape of this.shapes) {
        if (existingShape !== shape) {
          let existingZOrderCurve = this.animationData.curves[`shape.${existingShape.shapeId}.zOrder`];
          if (existingZOrderCurve) {
            for (let kf of existingZOrderCurve.keyframes) {
              if (kf.time === time) {
                kf.value += 1;
              }
            }
          }
        }
      }
    } else {
      zOrder = this.shapes.length - 1;
    }

    // Add AnimationData keyframes
    this.animationData.addKeyframe(`shape.${shape.shapeId}.exists`, new Keyframe(time, 1, "hold"));
    this.animationData.addKeyframe(`shape.${shape.shapeId}.zOrder`, new Keyframe(time, zOrder, "hold"));
    this.animationData.addKeyframe(`shape.${shape.shapeId}.shapeIndex`, new Keyframe(time, shape.shapeIndex, "linear"));
  }

  /**
   * Remove a specific shape instance from this layer
   * Leaves a "hole" in shapeIndex values so the shape can be restored later
   */
  removeShape(shape) {
    const shapeIndex = this.shapes.indexOf(shape);
    if (shapeIndex < 0) return;

    const shapeId = shape.shapeId;
    const removedShapeIndex = shape.shapeIndex;

    // Remove from array
    this.shapes.splice(shapeIndex, 1);

    // Get shapeIndex curve
    const shapeIndexCurve = this.animationData.getCurve(`shape.${shapeId}.shapeIndex`);
    if (shapeIndexCurve) {
      // Remove keyframes that point to this shapeIndex
      const keyframesToRemove = shapeIndexCurve.keyframes.filter(kf => kf.value === removedShapeIndex);
      for (let kf of keyframesToRemove) {
        shapeIndexCurve.removeKeyframe(kf);
      }
      // Note: We intentionally leave a "hole" at this shapeIndex value
      // so the shape can be restored with the same index if undeleted
    }
  }

  getFrame(num) {
    if (this.frames[num]) {
      if (this.frames[num].frameType == "keyframe") {
        return this.frames[num];
      } else if (this.frames[num].frameType == "motion") {
        let frameKeys = {};
        let prevFrame = this.frames[num].prev;
        let nextFrame = this.frames[num].next;
        const t =
          (num - this.frames[num].prevIndex) /
          (this.frames[num].nextIndex - this.frames[num].prevIndex);
        for (let key in prevFrame?.keys) {
          frameKeys[key] = {};
          let prevKeyDict = prevFrame.keys[key];
          let nextKeyDict = nextFrame.keys[key];
          for (let prop in prevKeyDict) {
            frameKeys[key][prop] =
              (1 - t) * prevKeyDict[prop] + t * nextKeyDict[prop];
          }
        }
        let frame = new Frame("motion", "temp");
        frame.keys = frameKeys;
        return frame;
      } else if (this.frames[num].frameType == "shape") {
        let prevFrame = this.frames[num].prev;
        let nextFrame = this.frames[num].next;
        const t =
          (num - this.frames[num].prevIndex) /
          (this.frames[num].nextIndex - this.frames[num].prevIndex);
        let shapes = [];
        for (let shape1 of prevFrame?.shapes) {
          if (shape1.curves.length == 0) continue;
          let shape2 = undefined;
          for (let i of nextFrame.shapes) {
            if (shape1.shapeId == i.shapeId) {
              shape2 = i;
            }
          }
          if (shape2 != undefined) {
            let path1 = [
              {
                type: "M",
                x: shape1.curves[0].points[0].x,
                y: shape1.curves[0].points[0].y,
              },
            ];
            for (let curve of shape1.curves) {
              path1.push({
                type: "C",
                x1: curve.points[1].x,
                y1: curve.points[1].y,
                x2: curve.points[2].x,
                y2: curve.points[2].y,
                x: curve.points[3].x,
                y: curve.points[3].y,
              });
            }
            let path2 = [];
            if (shape2.curves.length > 0) {
              path2.push({
                type: "M",
                x: shape2.curves[0].points[0].x,
                y: shape2.curves[0].points[0].y,
              });
              for (let curve of shape2.curves) {
                path2.push({
                  type: "C",
                  x1: curve.points[1].x,
                  y1: curve.points[1].y,
                  x2: curve.points[2].x,
                  y2: curve.points[2].y,
                  x: curve.points[3].x,
                  y: curve.points[3].y,
                });
              }
            }
            const interpolator = d3.interpolatePathCommands(path1, path2);
            let current = interpolator(t);
            let curves = [];
            let start = current.shift();
            let { x, y } = start;
            for (let curve of current) {
              curves.push(
                new Bezier(
                  x,
                  y,
                  curve.x1,
                  curve.y1,
                  curve.x2,
                  curve.y2,
                  curve.x,
                  curve.y,
                ),
              );
              x = curve.x;
              y = curve.y;
            }
            let lineWidth = lerp(shape1.lineWidth, shape2.lineWidth, t);
            let strokeStyle = lerpColor(
              shape1.strokeStyle,
              shape2.strokeStyle,
              t,
            );
            let fillStyle;
            if (!shape1.fillImage) {
              fillStyle = lerpColor(shape1.fillStyle, shape2.fillStyle, t);
            }
            shapes.push(
              new TempShape(
                start.x,
                start.y,
                curves,
                shape1.lineWidth,
                shape1.stroked,
                shape1.filled,
                strokeStyle,
                fillStyle,
              ),
            );
          }
        }
        let frame = new Frame("shape", "temp");
        frame.shapes = shapes;
        return frame;
      } else {
        for (let i = Math.min(num, this.frames.length - 1); i >= 0; i--) {
          if (this.frames[i]?.frameType == "keyframe") {
            let tempFrame = this.frames[i].copy("tempFrame");
            tempFrame.frameType = "normal";
            return tempFrame;
          }
        }
      }
    } else {
      for (let i = Math.min(num, this.frames.length - 1); i >= 0; i--) {
        // if (this.frames[i].frameType == "keyframe") {
        //   let tempFrame = this.frames[i].copy("tempFrame")
        //   tempFrame.frameType = "normal"
        return tempFrame;
        // }
      }
    }
  }
  getLatestFrame(num) {
    for (let i = num; i >= 0; i--) {
      if (this.frames[i]?.exists) {
        return this.getFrame(i);
      }
    }
  }
  copy(idx) {
    let newLayer = new VectorLayer(idx.slice(0, 8) + this.idx.slice(8));
    let idxMapping = {};
    for (let child of this.children) {
      let newChild = child.copy(idx);
      idxMapping[child.idx] = newChild.idx;
      newLayer.children.push(newChild);
    }
    newLayer.frames = [];
    for (let frame of this.frames) {
      let newFrame = frame.copy(idx);
      newFrame.keys = {};
      for (let key in frame.keys) {
        newFrame.keys[idxMapping[key]] = structuredClone(frame.keys[key]);
      }
      newLayer.frames.push(newFrame);
    }
    return newLayer;
  }
  addFrame(num, frame, addedFrames) {
    // let updateDest = undefined;
    // if (!this.frames[num]) {
    //   for (const [index, idx] of Object.entries(addedFrames)) {
    //     if (!this.frames[index]) {
    //       this.frames[index] = new Frame("normal", idx);
    //     }
    //   }
    // } else {
    //   if (this.frames[num].frameType == "motion") {
    //     updateDest = "motion";
    //   } else if (this.frames[num].frameType == "shape") {
    //     updateDest = "shape";
    //   }
    // }
    this.frames[num] = frame;
    // if (updateDest) {
    //   this.updateFrameNextAndPrev(num - 1, updateDest);
    //   this.updateFrameNextAndPrev(num + 1, updateDest);
    // }
  }
  addOrChangeFrame(num, frameType, uuid, addedFrames) {
    let latestFrame = this.getLatestFrame(num);
    let newKeyframe = new Frame(frameType, uuid);
    for (let key in latestFrame.keys) {
      newKeyframe.keys[key] = structuredClone(latestFrame.keys[key]);
    }
    for (let shape of latestFrame.shapes) {
      newKeyframe.shapes.push(shape.copy(uuid));
    }
    this.addFrame(num, newKeyframe, addedFrames);
  }
  deleteFrame(uuid, destinationType, replacementUuid) {
    let frame = pointerList[uuid];
    let i = this.frames.indexOf(frame);
    if (i != -1) {
      if (destinationType == undefined) {
        // Determine destination type from surrounding frames
        const prevFrame = this.frames[i - 1];
        const nextFrame = this.frames[i + 1];
        const prevType = prevFrame ? prevFrame.frameType : null;
        const nextType = nextFrame ? nextFrame.frameType : null;
        if (prevType === "motion" || nextType === "motion") {
          destinationType = "motion";
        } else if (prevType === "shape" || nextType === "shape") {
          destinationType = "shape";
        } else if (prevType !== null && nextType !== null) {
          destinationType = "normal";
        } else {
          destinationType = "none";
        }
      }
      if (destinationType == "none") {
        delete this.frames[i];
      } else {
        this.frames[i] = this.frames[i].copy(replacementUuid);
        this.frames[i].frameType = destinationType;
        this.updateFrameNextAndPrev(i, destinationType);
      }
    }
  }
  updateFrameNextAndPrev(num, frameType, lastBefore, firstAfter) {
    if (!this.frames[num] || this.frames[num].frameType == "keyframe") return;
    if (lastBefore == undefined || firstAfter == undefined) {
      let { lastKeyframeBefore, firstKeyframeAfter } = getKeyframesSurrounding(
        this.frames,
        num,
      );
      lastBefore = lastKeyframeBefore;
      firstAfter = firstKeyframeAfter;
    }
    for (let i = lastBefore + 1; i < firstAfter; i++) {
      this.frames[i].frameType = frameType;
      this.frames[i].prev = this.frames[lastBefore];
      this.frames[i].next = this.frames[firstAfter];
      this.frames[i].prevIndex = lastBefore;
      this.frames[i].nextIndex = firstAfter;
    }
  }
  toggleVisibility() {
    this.visible = !this.visible;
    updateUI();
    updateMenu();
    updateLayers();
  }
  getFrameValue(n) {
    const valueAtN = this.frames[n];
    if (valueAtN !== undefined) {
        return { valueAtN, prev: null, next: null, prevIndex: null, nextIndex: null };
    }
    let prev = n - 1;
    let next = n + 1;

    while (prev >= 0 && this.frames[prev] === undefined) {
        prev--;
    }
    while (next < this.frames.length && this.frames[next] === undefined) {
        next++;
    }

    return {
        valueAtN: undefined,
        prev: prev >= 0 ? this.frames[prev] : null,
        next: next < this.frames.length ? this.frames[next] : null,
        prevIndex: prev >= 0 ? prev : null,
        nextIndex: next < this.frames.length ? next : null
    };
  }

  // Get all shapes that exist at the given time
  getVisibleShapes(time) {
    const visibleShapes = [];

    // Calculate tolerance based on framerate (half a frame)
    const halfFrameDuration = 0.5 / config.framerate;

    // Group shapes by shapeId
    const shapesByShapeId = new Map();
    for (let shape of this.shapes) {
      if (shape instanceof TempShape) continue;
      if (!shapesByShapeId.has(shape.shapeId)) {
        shapesByShapeId.set(shape.shapeId, []);
      }
      shapesByShapeId.get(shape.shapeId).push(shape);
    }

    // For each logical shape (shapeId), determine which version to return for EDITING
    for (let [shapeId, shapes] of shapesByShapeId) {
      // Check if this logical shape exists at current time
      let existsValue = this.animationData.interpolate(`shape.${shapeId}.exists`, time);
      if (existsValue === null || existsValue <= 0) continue;

      // Get shapeIndex curve
      const shapeIndexCurve = this.animationData.getCurve(`shape.${shapeId}.shapeIndex`);

      if (!shapeIndexCurve || !shapeIndexCurve.keyframes || shapeIndexCurve.keyframes.length === 0) {
        // No shapeIndex curve, return shape with index 0
        const shape = shapes.find(s => s.shapeIndex === 0);
        if (shape) {
          visibleShapes.push(shape);
        }
        continue;
      }

      // Find bracketing keyframes
      const { prev: prevKf, next: nextKf } = shapeIndexCurve.getBracketingKeyframes(time);

      // Get interpolated shapeIndex value
      let shapeIndexValue = shapeIndexCurve.interpolate(time);
      if (shapeIndexValue === null) shapeIndexValue = 0;

      // Check if we're at a keyframe (within half a frame)
      const atPrevKeyframe = prevKf && Math.abs(shapeIndexValue - prevKf.value) < halfFrameDuration;
      const atNextKeyframe = nextKf && Math.abs(shapeIndexValue - nextKf.value) < halfFrameDuration;

      if (atPrevKeyframe) {
        // At previous keyframe - return that version for editing
        const shape = shapes.find(s => s.shapeIndex === prevKf.value);
        if (shape) visibleShapes.push(shape);
      } else if (atNextKeyframe) {
        // At next keyframe - return that version for editing
        const shape = shapes.find(s => s.shapeIndex === nextKf.value);
        if (shape) visibleShapes.push(shape);
      } else if (prevKf && prevKf.interpolation === 'hold') {
        // Between keyframes but using "hold" interpolation - no morphing
        // Return the previous keyframe's shape since that's what's shown
        const shape = shapes.find(s => s.shapeIndex === prevKf.value);
        if (shape) visibleShapes.push(shape);
      }
      // Otherwise: between keyframes with morphing, return nothing (can't edit a morph)
    }

    return visibleShapes;
  }

  draw(ctx) {
    // super.draw(ctx)
    if (!this.visible) return;

    let cxt = {...context}
    cxt.ctx = ctx

    // Draw shapes using AnimationData curves for exists, zOrder, and shape tweening
    let currentTime = context.activeObject?.currentTime || 0;

    // Group shapes by shapeId for tweening support
    const shapesByShapeId = new Map();
    for (let shape of this.shapes) {
      if (shape instanceof TempShape) continue;
      if (!shapesByShapeId.has(shape.shapeId)) {
        shapesByShapeId.set(shape.shapeId, []);
      }
      shapesByShapeId.get(shape.shapeId).push(shape);
    }

    // Process each logical shape (shapeId)
    let visibleShapes = [];
    for (let [shapeId, shapes] of shapesByShapeId) {
      // Check if this logical shape exists at current time
      let existsValue = this.animationData.interpolate(`shape.${shapeId}.exists`, currentTime);
      if (existsValue === null || existsValue <= 0) continue;

      // Get z-order
      let zOrder = this.animationData.interpolate(`shape.${shapeId}.zOrder`, currentTime);

      // Get shapeIndex curve and surrounding keyframes
      const shapeIndexCurve = this.animationData.getCurve(`shape.${shapeId}.shapeIndex`);
      if (!shapeIndexCurve || !shapeIndexCurve.keyframes || shapeIndexCurve.keyframes.length === 0) {
        // No shapeIndex curve, just show shape with index 0
        const shape = shapes.find(s => s.shapeIndex === 0);
        if (shape) {
          visibleShapes.push({ shape, zOrder: zOrder || 0, selected: context.shapeselection.includes(shape) });
        }
        continue;
      }

      // Find surrounding keyframes
      const { prev: prevKf, next: nextKf } = getKeyframesSurrounding(shapeIndexCurve.keyframes, currentTime);

      // Get interpolated value
      let shapeIndexValue = shapeIndexCurve.interpolate(currentTime);
      if (shapeIndexValue === null) shapeIndexValue = 0;

      // Sort shape versions by shapeIndex
      shapes.sort((a, b) => a.shapeIndex - b.shapeIndex);

      // Determine whether to morph based on whether interpolated value equals a keyframe value
      // Check if we're at either the previous or next keyframe value (no morphing needed)
      const atPrevKeyframe = prevKf && Math.abs(shapeIndexValue - prevKf.value) < 0.001;
      const atNextKeyframe = nextKf && Math.abs(shapeIndexValue - nextKf.value) < 0.001;

      if (atPrevKeyframe || atNextKeyframe) {
        // No morphing - display the shape at the keyframe value
        const targetValue = atNextKeyframe ? nextKf.value : prevKf.value;
        const shape = shapes.find(s => s.shapeIndex === targetValue);
        if (shape) {
          visibleShapes.push({ shape, zOrder: zOrder || 0, selected: context.shapeselection.includes(shape) });
        }
      } else if (prevKf && nextKf && prevKf.value !== nextKf.value) {
        // Morph between shapes specified by surrounding keyframes
        const shape1 = shapes.find(s => s.shapeIndex === prevKf.value);
        const shape2 = shapes.find(s => s.shapeIndex === nextKf.value);

        if (shape1 && shape2) {
          // Calculate t based on time position between keyframes
          const t = (currentTime - prevKf.time) / (nextKf.time - prevKf.time);
          const morphedShape = shape1.lerpShape(shape2, t);
          visibleShapes.push({ shape: morphedShape, zOrder: zOrder || 0, selected: context.shapeselection.includes(shape1) || context.shapeselection.includes(shape2) });
        } else if (shape1) {
          visibleShapes.push({ shape: shape1, zOrder: zOrder || 0, selected: context.shapeselection.includes(shape1) });
        } else if (shape2) {
          visibleShapes.push({ shape: shape2, zOrder: zOrder || 0, selected: context.shapeselection.includes(shape2) });
        }
      } else if (nextKf) {
        // Only next keyframe exists, show that shape
        const shape = shapes.find(s => s.shapeIndex === nextKf.value);
        if (shape) {
          visibleShapes.push({ shape, zOrder: zOrder || 0, selected: context.shapeselection.includes(shape) });
        }
      }
    }

    // Sort by zOrder (lowest first = back, highest last = front)
    visibleShapes.sort((a, b) => a.zOrder - b.zOrder);

    // Draw sorted shapes
    for (let { shape, selected } of visibleShapes) {
      cxt.selected = selected;
      shape.draw(cxt);
    }

    // Draw children (GraphicsObjects) using AnimationData curves
    for (let child of this.children) {
      // Check if child exists at current time using AnimationData
      // null means no exists curve (defaults to visible)
      const existsValue = this.animationData.interpolate(`object.${child.idx}.exists`, currentTime);
      if (existsValue !== null && existsValue <= 0) continue;

      // Get child properties from AnimationData curves
      const childX = this.animationData.interpolate(`object.${child.idx}.x`, currentTime);
      const childY = this.animationData.interpolate(`object.${child.idx}.y`, currentTime);
      const childRotation = this.animationData.interpolate(`object.${child.idx}.rotation`, currentTime);
      const childScaleX = this.animationData.interpolate(`object.${child.idx}.scale_x`, currentTime);
      const childScaleY = this.animationData.interpolate(`object.${child.idx}.scale_y`, currentTime);

      // Apply properties if they exist in AnimationData
      if (childX !== null) child.x = childX;
      if (childY !== null) child.y = childY;
      if (childRotation !== null) child.rotation = childRotation;
      if (childScaleX !== null) child.scale_x = childScaleX;
      if (childScaleY !== null) child.scale_y = childScaleY;

      // Draw the child if not in objectStack
      if (!context.objectStack.includes(child)) {
        const transform = ctx.getTransform();
        ctx.translate(child.x, child.y);
        ctx.scale(child.scale_x, child.scale_y);
        ctx.rotate(child.rotation);
        child.draw(ctx);

        // Draw selection outline if selected
        if (context.selection.includes(child)) {
          ctx.lineWidth = 1;
          ctx.strokeStyle = "#00ffff";
          ctx.beginPath();
          let bbox = child.bbox();
          ctx.rect(bbox.x.min - child.x, bbox.y.min - child.y, bbox.x.max - bbox.x.min, bbox.y.max - bbox.y.min);
          ctx.stroke();
        }
        ctx.setTransform(transform);
      }
    }
    // Draw activeShape regardless of whether frame exists
    if (this.activeShape) {
      console.log("Layer.draw: Drawing activeShape", this.activeShape);
      this.activeShape.draw(cxt)
      console.log("Layer.draw: Drew activeShape");
    }
  }
  bbox() {
    let bbox = super.bbox();
    let currentTime = context.activeObject?.currentTime || 0;

    // Get visible shapes at current time using AnimationData
    const visibleShapes = this.getVisibleShapes(currentTime);

    if (visibleShapes.length > 0 && bbox === undefined) {
      bbox = structuredClone(visibleShapes[0].boundingBox);
    }
    for (let shape of visibleShapes) {
      growBoundingBox(bbox, shape.boundingBox);
    }
    return bbox;
  }
  mousedown(x, y) {
    console.log("Layer.mousedown called - this:", this.name, "activeLayer:", context.activeLayer?.name, "context.mode:", context.mode);
    const mouse = {x: x, y: y}
    if (this==context.activeLayer) {
      console.log("This IS the active layer");
      switch(context.mode) {
        case "rectangle":
        case "ellipse":
        case "draw":
          console.log("Creating shape for context.mode:", context.mode);
          this.clicked = true
          this.activeShape = new Shape(x, y, context, this, uuidv4())
          this.lastMouse = mouse;
          console.log("Shape created:", this.activeShape);
          break;
        case "select":
        case "transform":
          break;
        case "paint_bucket":
          debugCurves = [];
          debugPoints = [];
          let epsilon = context.fillGaps;
          let regionPoints;

          // First, see if there's an existing shape to change the color of
          let currentTime = context.activeObject?.currentTime || 0;
          let visibleShapes = this.getVisibleShapes(currentTime);
          let pointShape = getShapeAtPoint(mouse, visibleShapes);

          if (pointShape) {
            actions.colorShape.create(pointShape, context.fillStyle);
            break;
          }

          // We didn't find an existing region to paintbucket, see if we can make one
          try {
            regionPoints = floodFillRegion(
              mouse,
              epsilon,
              config.fileWidth,
              config.fileHeight,
              context,
              debugPoints,
              debugPaintbucket,
              visibleShapes,
            );
          } catch (e) {
            updateUI();
            throw e;
          }
          if (regionPoints.length > 0 && regionPoints.length < 10) {
            // probably a very small area, rerun with minimum epsilon
            regionPoints = floodFillRegion(
              mouse,
              1,
              config.fileWidth,
              config.fileHeight,
              context,
              debugPoints,
              false,
              visibleShapes,
            );
          }
          let points = [];
          for (let point of regionPoints) {
            points.push([point.x, point.y]);
          }
          let cxt = {
            ...context,
            fillShape: true,
            strokeShape: false,
            sendToBack: true,
          };
          let shape = new Shape(regionPoints[0].x, regionPoints[0].y, cxt, this);
          shape.fromPoints(points, 1);
          actions.addShape.create(context.activeObject, shape, cxt);
          break;
      }
    }
  }
  mousemove(x, y) {
    const mouse = {x: x, y: y}
    if (this==context.activeLayer) {
      switch (context.mode) {
        case "draw":
          if (this.activeShape) {
            if (vectorDist(mouse, context.lastMouse) > minSegmentSize) {
              this.activeShape.addLine(x, y);
              this.lastMouse = mouse;
            }
          }
          break;
        case "rectangle":
          if (this.activeShape) {
            this.activeShape.clear();
            this.activeShape.addLine(x, this.activeShape.starty);
            this.activeShape.addLine(x, y);
            this.activeShape.addLine(this.activeShape.startx, y);
            this.activeShape.addLine(
              this.activeShape.startx,
              this.activeShape.starty,
            );
            this.activeShape.update();
          }
          break;
        case "ellipse":
          if (this.activeShape) {
            let midX = (mouse.x + this.activeShape.startx) / 2;
            let midY = (mouse.y + this.activeShape.starty) / 2;
            let xDiff = (mouse.x - this.activeShape.startx) / 2;
            let yDiff = (mouse.y - this.activeShape.starty) / 2;
            let ellipseConst = 0.552284749831; // (4/3)*tan(pi/(2n)) where n=4
            this.activeShape.clear();
            this.activeShape.addCurve(
              new Bezier(
                midX,
                this.activeShape.starty,
                midX + ellipseConst * xDiff,
                this.activeShape.starty,
                mouse.x,
                midY - ellipseConst * yDiff,
                mouse.x,
                midY,
              ),
            );
            this.activeShape.addCurve(
              new Bezier(
                mouse.x,
                midY,
                mouse.x,
                midY + ellipseConst * yDiff,
                midX + ellipseConst * xDiff,
                mouse.y,
                midX,
                mouse.y,
              ),
            );
            this.activeShape.addCurve(
              new Bezier(
                midX,
                mouse.y,
                midX - ellipseConst * xDiff,
                mouse.y,
                this.activeShape.startx,
                midY + ellipseConst * yDiff,
                this.activeShape.startx,
                midY,
              ),
            );
            this.activeShape.addCurve(
              new Bezier(
                this.activeShape.startx,
                midY,
                this.activeShape.startx,
                midY - ellipseConst * yDiff,
                midX - ellipseConst * xDiff,
                this.activeShape.starty,
                midX,
                this.activeShape.starty,
              ),
            );
          }
          break;
      }
    }
  }
  mouseup(x, y) {
    console.log("Layer.mouseup called - context.mode:", context.mode, "activeShape:", this.activeShape);
    this.clicked = false
    if (this==context.activeLayer) {
      switch (context.mode) {
        case "draw":
          if (this.activeShape) {
            this.activeShape.addLine(x, y);
            this.activeShape.simplify(context.simplifyMode);
          }
        case "rectangle":
        case "ellipse":
          if (this.activeShape) {
            console.log("Adding shape via actions.addShape.create");
            actions.addShape.create(context.activeObject, this.activeShape);
            console.log("Shape added, clearing activeShape");
            this.activeShape = undefined;
          }
          break;
      }
    }
  }
}

class AudioTrack {
  constructor(uuid, name, type = 'audio') {
    // ID and name
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    this.name = name || (type === 'midi' ? "MIDI" : "Audio");
    this.type = type; // 'audio' or 'midi'
    this.audible = true;
    this.visible = true;  // For consistency with Layer (audio tracks are always "visible" in timeline)

    // AnimationData for automation curves (like Layer)
    this.animationData = new AnimationData(this);

    // Read-only empty arrays for layer compatibility (audio tracks don't have shapes/children)
    Object.defineProperty(this, 'shapes', {
      value: Object.freeze([]),
      writable: false,
      enumerable: true,
      configurable: false
    });
    Object.defineProperty(this, 'children', {
      value: Object.freeze([]),
      writable: false,
      enumerable: true,
      configurable: false
    });

    // Reference to DAW backend track
    this.audioTrackId = null;

    // Audio clips (for audio tracks) or MIDI clips (for MIDI tracks)
    this.clips = []; // { clipId, poolIndex, name, startTime, duration, offset } or MIDI clip data

    // Timeline display settings (for track hierarchy)
    this.collapsed = false
    this.curvesMode = 'segment'  // 'segment' | 'keyframe' | 'curve'
    this.curvesHeight = 150  // Height in pixels when curves are in curve view

    pointerList[this.idx] = this;
  }

  // Sync automation to backend using generic parameter setter
  async syncAutomation(time) {
    if (this.audioTrackId === null) return;

    // Get all automation parameters and sync them
    const params = ['volume', 'mute', 'solo', 'pan'];
    for (const param of params) {
      const value = this.animationData.interpolate(`track.${param}`, time);
      if (value !== null) {
        await invoke('audio_set_track_parameter', {
          trackId: this.audioTrackId,
          parameter: param,
          value
        });
      }
    }
  }

  // Get all automation parameter names
  getAutomationParameters() {
    return [
      'track.volume',
      'track.pan',
      'track.mute',
      'track.solo',
      ...this.clips.flatMap(clip => [
        `clip.${clip.clipId}.gain`,
        `clip.${clip.clipId}.pan`
      ])
    ];
  }

  // Initialize the audio track in the DAW backend
  async initializeTrack() {
    if (this.audioTrackId !== null) {
      console.warn('Track already initialized');
      return;
    }

    try {
      const params = {
        name: this.name,
        trackType: this.type
      };

      // Add instrument parameter for MIDI tracks
      if (this.type === 'midi' && this.instrument) {
        params.instrument = this.instrument;
      }

      const trackId = await invoke('audio_create_track', params);
      this.audioTrackId = trackId;
      console.log(`${this.type === 'midi' ? 'MIDI' : 'Audio'} track created:`, this.name, 'with ID:', trackId);
    } catch (error) {
      console.error(`Failed to create ${this.type} track:`, error);
      throw error;
    }
  }

  // Load an audio file and add it to the pool
  // Returns metadata including: pool_index, duration, sample_rate, channels, waveform
  async loadAudioFile(path) {
    try {
      const metadata = await invoke('audio_load_file', {
        path: path
      });
      console.log('Audio file loaded:', path, 'metadata:', metadata);
      return metadata;
    } catch (error) {
      console.error('Failed to load audio file:', error);
      throw error;
    }
  }

  // Add a clip to this track
  async addClip(poolIndex, startTime, duration, offset = 0.0, name = '', waveform = null) {
    if (this.audioTrackId === null) {
      throw new Error('Track not initialized. Call initializeTrack() first.');
    }

    try {
      await invoke('audio_add_clip', {
        trackId: this.audioTrackId,
        poolIndex,
        startTime,
        duration,
        offset
      });

      // Store clip metadata locally
      // Note: clipId will be assigned by backend, we'll get it via ClipAdded event
      this.clips.push({
        clipId: this.clips.length, // Temporary ID
        poolIndex,
        name: name || `Clip ${this.clips.length + 1}`,
        startTime,
        duration,
        offset,
        waveform  // Store waveform data for rendering
      });

      console.log('Clip added to track', this.audioTrackId);
    } catch (error) {
      console.error('Failed to add clip:', error);
      throw error;
    }
  }

  static fromJSON(json) {
    const audioTrack = new AudioTrack(json.idx, json.name, json.trackType || 'audio');

    // Load AnimationData if present
    if (json.animationData) {
      audioTrack.animationData = AnimationData.fromJSON(json.animationData, audioTrack);
    }

    // Load clips if present
    if (json.clips) {
      audioTrack.clips = json.clips.map(clip => {
        const clipData = {
          clipId: clip.clipId,
          name: clip.name,
          startTime: clip.startTime,
          duration: clip.duration,
          offset: clip.offset || 0,  // Default to 0 if not present
        };

        // Restore audio-specific fields
        if (clip.poolIndex !== undefined) {
          clipData.poolIndex = clip.poolIndex;
        }

        // Restore MIDI-specific fields
        if (clip.notes) {
          clipData.notes = clip.notes;
        }

        return clipData;
      });
    }

    audioTrack.audible = json.audible;
    return audioTrack;
  }

  toJSON(randomizeUuid = false) {
    const json = {
      type: "AudioTrack",
      idx: randomizeUuid ? uuidv4() : this.idx,
      name: randomizeUuid ? this.name + " copy" : this.name,
      trackType: this.type, // 'audio' or 'midi'
      audible: this.audible,

      // AnimationData (includes automation curves)
      animationData: this.animationData.toJSON(),

      // Clips
      clips: this.clips.map(clip => {
        const clipData = {
          clipId: clip.clipId,
          name: clip.name,
          startTime: clip.startTime,
          duration: clip.duration,
        };

        // Add audio-specific fields
        if (clip.poolIndex !== undefined) {
          clipData.poolIndex = clip.poolIndex;
          clipData.offset = clip.offset;
        }

        // Add MIDI-specific fields
        if (clip.notes) {
          clipData.notes = clip.notes;
        }

        return clipData;
      })
    };

    return json;
  }

  copy(idx) {
    // Serialize and deserialize with randomized UUID
    const json = this.toJSON(true);
    json.idx = idx.slice(0, 8) + this.idx.slice(8);
    return AudioTrack.fromJSON(json);
  }
}

class VideoLayer extends Widget {
  constructor(uuid, name) {
    super(0, 0);
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    this.name = name || "Video";
    this.type = 'video';
    this.visible = true;
    this.audible = true;
    this.animationData = new AnimationData(this);

    // Empty arrays for layer compatibility
    Object.defineProperty(this, 'shapes', {
      value: Object.freeze([]),
      writable: false,
      enumerable: true,
      configurable: false
    });
    Object.defineProperty(this, 'children', {
      value: Object.freeze([]),
      writable: false,
      enumerable: true,
      configurable: false
    });

    // Video clips on this layer
    // { clipId, poolIndex, name, startTime, duration, offset, width, height }
    this.clips = [];

    // Associated audio track (if video has audio)
    this.linkedAudioTrack = null;  // Reference to AudioTrack

    // Performance settings
    this.useJpegCompression = false;  // JPEG compression adds more overhead than it saves (default: false)
    this.prefetchCount = 3;  // Number of frames to prefetch ahead of playhead

    // Timeline display
    this.collapsed = false;
    this.curvesMode = 'segment';
    this.curvesHeight = 150;

    pointerList[this.idx] = this;
  }

  async addClip(poolIndex, startTime, duration, offset = 0.0, name = '', sourceDuration = null, metadata = null) {
    const poolInfo = await invoke('video_get_pool_info', { poolIndex });
    // poolInfo is [width, height, fps] tuple from Rust
    const [width, height, fps] = poolInfo;

    const clip = {
      clipId: this.clips.length,
      poolIndex,
      name: name || `Video ${this.clips.length + 1}`,
      startTime,
      duration,
      offset,
      width,
      height,
      sourceDuration: sourceDuration || duration,  // Store original file duration
      httpUrl: metadata?.http_url || null,
      isBrowserCompatible: metadata?.is_browser_compatible || false,
      transcoding: metadata?.transcoding || false,
      videoElement: null,  // Will hold HTML5 video element if using browser playback
      useBrowserVideo: false,  // Switch to true when video element is ready
      isPlaying: false,  // Track if video element is actively playing
    };

    this.clips.push(clip);

    console.log(`Video clip added: ${name}, ${width}x${height}, duration: ${duration}s, browser-compatible: ${clip.isBrowserCompatible}, http_url: ${clip.httpUrl}`);

    // If HTTP URL is available, create video element immediately
    if (clip.httpUrl) {
      await this._createVideoElement(clip);
      clip.useBrowserVideo = true;
    }
    // If transcoding is in progress, start polling
    else if (clip.transcoding) {
      console.log(`[Video] Starting transcode polling for ${clip.name}`);
      this._pollTranscodeStatus(clip);
    }
  }

  async _createVideoElement(clip) {
    // Create hidden video element for hardware-accelerated decoding
    const video = document.createElement('video');
    // Hide video element using opacity (browsers may skip decoding if off-screen)
    video.style.position = 'fixed';
    video.style.bottom = '0';
    video.style.right = '0';
    video.style.width = '1px';
    video.style.height = '1px';
    video.style.opacity = '0.01';  // Nearly invisible but not 0 (some browsers optimize opacity:0)
    video.style.pointerEvents = 'none';
    video.style.zIndex = '-1';
    video.preload = 'auto';
    video.muted = true;  // Mute video element (audio plays separately)
    video.playsInline = true;
    video.autoplay = false;
    video.crossOrigin = 'anonymous';  // Required for canvas drawing - prevent CORS taint

    // Add event listeners for debugging
    video.addEventListener('loadedmetadata', () => {
      console.log(`[Video] Loaded metadata for ${clip.name}: ${video.videoWidth}x${video.videoHeight}, duration: ${video.duration}s`);
    });

    video.addEventListener('loadeddata', () => {
      console.log(`[Video] Loaded data for ${clip.name}, readyState: ${video.readyState}`);
    });

    video.addEventListener('canplay', () => {
      console.log(`[Video] Can play ${clip.name}, duration: ${video.duration}s`);
      // Mark video as ready for seeking once we can play AND have valid duration
      if (video.duration > 0 && !isNaN(video.duration) && video.duration !== Infinity) {
        clip.videoReady = true;
        console.log(`[Video] Video is ready for seeking`);
      }
    });

    // When seek completes, trigger UI redraw to show the new frame
    video.addEventListener('seeked', () => {
      if (updateUI) {
        updateUI();
      }
    });

    video.addEventListener('error', (e) => {
      const error = video.error;
      const errorMessages = {
        1: 'MEDIA_ERR_ABORTED - Fetching aborted',
        2: 'MEDIA_ERR_NETWORK - Network error',
        3: 'MEDIA_ERR_DECODE - Decoding error',
        4: 'MEDIA_ERR_SRC_NOT_SUPPORTED - Format not supported or file not accessible'
      };
      const errorMsg = errorMessages[error?.code] || 'Unknown error';
      console.error(`[Video] Error loading ${clip.name}: ${errorMsg}`, error?.message);
    });

    // Use HTTP URL from local server (supports range requests for seeking)
    video.src = clip.httpUrl;

    // Try to load the video
    video.load();

    document.body.appendChild(video);
    clip.videoElement = video;

    console.log(`[Video] Created video element for clip ${clip.name}: ${clip.httpUrl}`);
  }

  async _pollTranscodeStatus(clip) {
    // Poll transcode status every 2 seconds
    const pollInterval = setInterval(async () => {
      try {
        const status = await invoke('video_get_transcode_status', { poolIndex: clip.poolIndex });

        if (status && status[2]) {  // [path, progress, completed, httpUrl]
          // Transcode complete!
          clearInterval(pollInterval);
          const [outputPath, progress, completed, httpUrl] = status;

          clip.transcodedPath = outputPath;
          clip.httpUrl = httpUrl;
          clip.transcoding = false;
          clip.useBrowserVideo = true;

          console.log(`[Video] Transcode complete for ${clip.name}, switching to browser playback: ${httpUrl}`);

          // Create video element for browser playback
          await this._createVideoElement(clip);
        }
      } catch (error) {
        console.error('Failed to poll transcode status:', error);
        clearInterval(pollInterval);
      }
    }, 2000);
  }

  // Pre-fetch frames for current time (call before draw)
  async updateFrame(currentTime) {
    // Prevent concurrent calls - if already updating, skip
    if (this.updateInProgress) {
      return;
    }
    this.updateInProgress = true;

    try {
      for (let clip of this.clips) {
        // Check if clip is active at current time
        if (currentTime < clip.startTime ||
            currentTime >= clip.startTime + clip.duration) {
          clip.currentFrame = null;

          // Pause video element if we left its time range
          if (clip.videoElement && clip.isPlaying) {
            clip.videoElement.pause();
            clip.isPlaying = false;
          }

          continue;
        }

        // If using browser video element
        if (clip.useBrowserVideo && clip.videoElement) {
          const videoTime = clip.offset + (currentTime - clip.startTime);

          // Don't do anything until video is fully ready
          if (!clip.videoReady) {
            if (!clip._notReadyWarned) {
              console.warn(`[Video updateFrame] Video not ready yet (duration=${clip.videoElement.duration})`);
              clip._notReadyWarned = true;
            }
            continue;
          }

          // During playback: let video play naturally
          if (context.playing) {
            // Check if we just entered this clip (need to start playing)
            if (!clip.isPlaying) {
              // Start playing one frame ahead to compensate for canvas drawing lag
              const frameDuration = 1 / (clip.fps || 30); // Use clip's actual framerate
              const maxVideoTime = clip.sourceDuration - frameDuration; // Don't seek past end
              const startTime = Math.min(videoTime + frameDuration, maxVideoTime);
              console.log(`[Video updateFrame] Starting playback at ${startTime.toFixed(3)}s (compensated by ${frameDuration.toFixed(3)}s for ${clip.fps}fps)`);
              clip.videoElement.currentTime = startTime;
              clip.videoElement.play().catch(e => console.error('Failed to play video:', e));
              clip.isPlaying = true;
            }
            // Otherwise, let it play naturally - don't seek!
          }
          // When scrubbing (not playing): seek to exact position and pause
          else {
            if (clip.isPlaying) {
              clip.videoElement.pause();
              clip.isPlaying = false;
            }

            // Only seek if the time is actually different
            if (!clip.videoElement.seeking) {
              const timeDiff = Math.abs(clip.videoElement.currentTime - videoTime);
              if (timeDiff > 0.016) { // ~1 frame tolerance at 60fps
                clip.videoElement.currentTime = videoTime;
              }
            }
          }

          continue;  // Skip frame fetching
        }

        // Use frame batching for frame-based playback

        // Initialize frame cache if needed
        if (!clip.frameCache) {
          clip.frameCache = new Map();
        }

        // Check if current frame is already cached
        if (clip.frameCache.has(currentVideoTimestamp)) {
          clip.currentFrame = clip.frameCache.get(currentVideoTimestamp);
          clip.lastFetchedTimestamp = currentVideoTimestamp;
          continue;
        }

        // Skip if already fetching
        if (clip.fetchInProgress) {
          continue;
        }

        clip.fetchInProgress = true;

        try {
          // Calculate timestamps to prefetch (current + next N frames)
          const frameDuration = 1 / 30; // Assume 30fps for now, could get from clip metadata
          const timestamps = [];
          for (let i = 0; i < this.prefetchCount; i++) {
            const ts = currentVideoTimestamp + (i * frameDuration);
            // Don't exceed clip duration
            if (ts <= clip.offset + clip.sourceDuration) {
              timestamps.push(ts);
            }
          }

          if (timestamps.length === 0) {
            continue;
          }

          const t_start = performance.now();

          // Request batch of frames using IPC Channel
          const batchDataPromise = new Promise((resolve, reject) => {
            const channel = new Channel();

            channel.onmessage = (data) => {
              resolve(data);
            };

            invoke('video_get_frames_batch', {
              poolIndex: clip.poolIndex,
              timestamps: timestamps,
              useJpeg: this.useJpegCompression,
              channel: channel
            }).catch(reject);
          });

          let batchData = await batchDataPromise;
          const t_after_ipc = performance.now();

          // Ensure data is Uint8Array
          if (!(batchData instanceof Uint8Array)) {
            batchData = new Uint8Array(batchData);
          }

          // Unpack the batch format: [frame_count: u32][frame1_size: u32][frame1_data...][frame2_size: u32][frame2_data...]...
          const view = new DataView(batchData.buffer, batchData.byteOffset, batchData.byteLength);
          let offset = 0;

          // Read frame count
          const frameCount = view.getUint32(offset, true); // little-endian
          offset += 4;

          if (frameCount !== timestamps.length) {
            console.warn(`Expected ${timestamps.length} frames, got ${frameCount}`);
          }

          const t_before_conversion = performance.now();

          // Process each frame
          for (let i = 0; i < frameCount; i++) {
            // Read frame size
            const frameSize = view.getUint32(offset, true);
            offset += 4;

            // Extract frame data
            const frameData = new Uint8Array(batchData.buffer, batchData.byteOffset + offset, frameSize);
            offset += frameSize;

            let imageData;

            if (this.useJpegCompression) {
              // Decode JPEG using createImageBitmap
              const blob = new Blob([frameData], { type: 'image/jpeg' });
              const imageBitmap = await createImageBitmap(blob);

              // Create temporary canvas to extract ImageData
              const tempCanvas = document.createElement('canvas');
              tempCanvas.width = clip.width;
              tempCanvas.height = clip.height;
              const tempCtx = tempCanvas.getContext('2d');
              tempCtx.drawImage(imageBitmap, 0, 0);
              imageData = tempCtx.getImageData(0, 0, clip.width, clip.height);

              imageBitmap.close();
            } else {
              // Raw RGBA data
              const expectedSize = clip.width * clip.height * 4;

              if (frameData.length !== expectedSize) {
                console.error(`Invalid frame ${i} data size: got ${frameData.length}, expected ${expectedSize}`);
                continue;
              }

              imageData = new ImageData(
                new Uint8ClampedArray(frameData),
                clip.width,
                clip.height
              );
            }

            // Create canvas for this frame
            const frameCanvas = document.createElement('canvas');
            frameCanvas.width = clip.width;
            frameCanvas.height = clip.height;
            const frameCtx = frameCanvas.getContext('2d');
            frameCtx.putImageData(imageData, 0, 0);

            // Cache the frame
            clip.frameCache.set(timestamps[i], frameCanvas);

            // Set as current frame if it's the first one
            if (i === 0) {
              clip.currentFrame = frameCanvas;
              clip.lastFetchedTimestamp = timestamps[i];
            }
          }

          const t_after_conversion = performance.now();

          // Limit cache size to avoid memory issues
          const maxCacheSize = this.prefetchCount * 2;
          if (clip.frameCache.size > maxCacheSize) {
            // Remove oldest entries (simple LRU by keeping only recent timestamps)
            const sortedKeys = Array.from(clip.frameCache.keys()).sort((a, b) => a - b);
            const toRemove = sortedKeys.slice(0, sortedKeys.length - maxCacheSize);
            for (let key of toRemove) {
              clip.frameCache.delete(key);
            }
          }

          // Log timing breakdown
          const total_time = t_after_conversion - t_start;
          const ipc_time = t_after_ipc - t_start;
          const conversion_time = t_after_conversion - t_before_conversion;
          const compression_mode = this.useJpegCompression ? 'JPEG' : 'RAW';
          const avg_per_frame = total_time / frameCount;

          console.log(`[JS Video Batch ${compression_mode}] Fetched ${frameCount} frames | Total: ${total_time.toFixed(1)}ms | IPC: ${ipc_time.toFixed(1)}ms (${(ipc_time/total_time*100).toFixed(0)}%) | Convert: ${conversion_time.toFixed(1)}ms | Avg/frame: ${avg_per_frame.toFixed(1)}ms | Size: ${(batchData.length/1024/1024).toFixed(2)}MB`);
        } catch (error) {
          console.error('Failed to get video frames batch:', error);
          clip.currentFrame = null;
        } finally {
          clip.fetchInProgress = false;
        }
      }
    } finally {
      this.updateInProgress = false;
    }
  }

  // Draw cached frames (synchronous)
  draw(cxt, currentTime) {
    if (!this.visible) {
      return;
    }

    const ctx = cxt.ctx || cxt;

    // Use currentTime from context if not provided
    if (currentTime === undefined) {
      currentTime = cxt.activeObject?.currentTime || 0;
    }

    for (let clip of this.clips) {
      // Check if clip is active at current time
      if (currentTime < clip.startTime ||
          currentTime >= clip.startTime + clip.duration) {
        continue;
      }

      // Debug: log what path we're taking
      if (!clip._drawPathLogged) {
        console.log(`[Video Draw] useBrowserVideo=${clip.useBrowserVideo}, videoElement=${!!clip.videoElement}, currentFrame=${!!clip.currentFrame}`);
        clip._drawPathLogged = true;
      }

      // Prefer browser video element if available
      if (clip.useBrowserVideo && clip.videoElement) {
        // Debug: log readyState issues
        if (clip.videoElement.readyState < 2) {
          if (!clip._readyStateWarned) {
            console.warn(`[Video] Video not ready: readyState=${clip.videoElement.readyState}, src=${clip.videoElement.src}`);
            clip._readyStateWarned = true;
          }
        }

        // Draw if video is ready (shows last frame while seeking, updates when seek completes)
        if (clip.videoElement.readyState >= 2) {
          try {
            // Calculate expected video time
            const expectedVideoTime = clip.offset + (currentTime - clip.startTime);
            const actualVideoTime = clip.videoElement.currentTime;
            const timeDiff = Math.abs(expectedVideoTime - actualVideoTime);

            // Debug: log if time is significantly different
            if (timeDiff > 0.1 && (!clip._lastTimeDiffWarning || Date.now() - clip._lastTimeDiffWarning > 1000)) {
              console.warn(`[Video Draw] Time mismatch: expected ${expectedVideoTime.toFixed(2)}s, actual ${actualVideoTime.toFixed(2)}s, diff=${timeDiff.toFixed(2)}s`);
              clip._lastTimeDiffWarning = Date.now();
            }

            // Debug: log successful draw periodically
            if (!clip._lastDrawLog || Date.now() - clip._lastDrawLog > 1000) {
              console.log(`[Video Draw] Drawing at currentTime=${actualVideoTime.toFixed(2)}s (expected ${expectedVideoTime.toFixed(2)}s)`);
              clip._lastDrawLog = Date.now();
            }

            // Scale to fit canvas while maintaining aspect ratio
            const canvasWidth = config.fileWidth;
            const canvasHeight = config.fileHeight;
            const scale = Math.min(
              canvasWidth / clip.videoElement.videoWidth,
              canvasHeight / clip.videoElement.videoHeight
            );
            const scaledWidth = clip.videoElement.videoWidth * scale;
            const scaledHeight = clip.videoElement.videoHeight * scale;
            const x = (canvasWidth - scaledWidth) / 2;
            const y = (canvasHeight - scaledHeight) / 2;

            // Debug: draw a test rectangle to verify canvas is working
            if (!clip._canvasTestDone) {
              ctx.save();
              ctx.fillStyle = 'red';
              ctx.fillRect(10, 10, 100, 100);
              ctx.restore();
              console.log(`[Video Draw] Drew test rectangle at (10, 10, 100, 100)`);
              console.log(`[Video Draw] Canvas dimensions: ${canvasWidth}x${canvasHeight}`);
              console.log(`[Video Draw] Scaled video dimensions: ${scaledWidth}x${scaledHeight} at (${x}, ${y})`);
              clip._canvasTestDone = true;
            }

            // Debug: Check if video element has dimensions
            if (!clip._videoDimensionsLogged) {
              console.log(`[Video Draw] Video element dimensions: videoWidth=${clip.videoElement.videoWidth}, videoHeight=${clip.videoElement.videoHeight}, naturalWidth=${clip.videoElement.videoWidth}, naturalHeight=${clip.videoElement.videoHeight}`);
              console.log(`[Video Draw] Video element state: paused=${clip.videoElement.paused}, ended=${clip.videoElement.ended}, seeking=${clip.videoElement.seeking}, readyState=${clip.videoElement.readyState}`);
              clip._videoDimensionsLogged = true;
            }

            ctx.drawImage(clip.videoElement, x, y, scaledWidth, scaledHeight);

            // Debug: Sample a pixel to see if video is actually drawing
            if (!clip._pixelTestDone) {
              const imageData = ctx.getImageData(canvasWidth / 2, canvasHeight / 2, 1, 1);
              const pixel = imageData.data;
              console.log(`[Video Draw] Center pixel after drawImage: R=${pixel[0]}, G=${pixel[1]}, B=${pixel[2]}, A=${pixel[3]}`);
              clip._pixelTestDone = true;
            }
          } catch (error) {
            console.error('Failed to draw video element:', error);
          }
        }
      }
      // Fall back to cached frame if available
      else if (clip.currentFrame) {
        try {
          // Scale to fit canvas while maintaining aspect ratio
          const canvasWidth = config.fileWidth;
          const canvasHeight = config.fileHeight;
          const scale = Math.min(
            canvasWidth / clip.width,
            canvasHeight / clip.height
          );
          const scaledWidth = clip.width * scale;
          const scaledHeight = clip.height * scale;
          const x = (canvasWidth - scaledWidth) / 2;
          const y = (canvasHeight - scaledHeight) / 2;

          ctx.drawImage(clip.currentFrame, x, y, scaledWidth, scaledHeight);
        } catch (error) {
          console.error('Failed to draw video frame:', error);
        }
      } else {
        // Draw placeholder if frame not loaded yet
        ctx.save();
        ctx.fillStyle = '#333333';
        ctx.fillRect(0, 0, config.fileWidth, config.fileHeight);
        ctx.fillStyle = '#ffffff';
        ctx.font = '24px sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        const msg = clip.transcoding ? 'Transcoding...' : 'Loading...';
        ctx.fillText(msg, config.fileWidth / 2, config.fileHeight / 2);
        ctx.restore();
      }
    }
  }

  static fromJSON(json) {
    const videoLayer = new VideoLayer(json.idx, json.name);

    if (json.animationData) {
      videoLayer.animationData = AnimationData.fromJSON(json.animationData, videoLayer);
    }

    if (json.clips) {
      videoLayer.clips = json.clips;
    }

    if (json.linkedAudioTrack) {
      // Will be resolved after all objects are loaded
      videoLayer.linkedAudioTrack = json.linkedAudioTrack;
    }

    videoLayer.visible = json.visible;
    videoLayer.audible = json.audible;

    // Restore compression setting (default to true if not specified for backward compatibility)
    if (json.useJpegCompression !== undefined) {
      videoLayer.useJpegCompression = json.useJpegCompression;
    }

    return videoLayer;
  }

  toJSON(randomizeUuid = false) {
    return {
      type: "VideoLayer",
      idx: randomizeUuid ? uuidv4() : this.idx,
      name: randomizeUuid ? this.name + " copy" : this.name,
      visible: this.visible,
      audible: this.audible,
      animationData: this.animationData.toJSON(),
      clips: this.clips,
      linkedAudioTrack: this.linkedAudioTrack?.idx,
      useJpegCompression: this.useJpegCompression
    };
  }

  copy(idx) {
    const json = this.toJSON(true);
    json.idx = idx.slice(0, 8) + this.idx.slice(8);
    return VideoLayer.fromJSON(json);
  }

  // Compatibility methods for layer interface
  bbox() {
    return {
      x: { min: 0, max: config.fileWidth },
      y: { min: 0, max: config.fileHeight }
    };
  }
}

export { VectorLayer, AudioTrack, VideoLayer };

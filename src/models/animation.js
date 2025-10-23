// Animation system models: Frame, Keyframe, AnimationCurve, AnimationData

import { context, config, pointerList, startProps } from '../state.js';

// Helper function for UUID generation
function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

class Frame {
  constructor(frameType = "normal", uuid = undefined) {
    this.keys = {};
    this.shapes = [];
    this.frameType = frameType;
    this.keyTypes = new Set()
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    pointerList[this.idx] = this;
  }
  get exists() {
    return true;
  }
  saveState() {
    startProps[this.idx] = structuredClone(this.keys);
  }
  copy(idx) {
    let newFrame = new Frame(
      this.frameType,
      idx.slice(0, 8) + this.idx.slice(8),
    );
    newFrame.keys = structuredClone(this.keys);
    newFrame.shapes = [];
    for (let shape of this.shapes) {
      newFrame.shapes.push(shape.copy(idx));
    }
    return newFrame;
  }
  static fromJSON(json, Shape = null) {
    if (!json) {
      return undefined
    }
    // Shape parameter passed in to avoid circular dependency
    // Will be provided by the calling code that has access to both modules
    const frame = new Frame(json.frameType, json.idx);
    frame.keyTypes = new Set(json.keyTypes)
    frame.keys = json.keys;
    if (Shape) {
      for (let i in json.shapes) {
        const shape = json.shapes[i];
        frame.shapes.push(Shape.fromJSON(shape));
      }
    }

    return frame;
  }
  toJSON(randomizeUuid = false) {
    const json = {};
    json.type = "Frame";
    json.frameType = this.frameType;
    json.keyTypes = Array.from(this.keyTypes)
    if (randomizeUuid) {
      json.idx = uuidv4();
    } else {
      json.idx = this.idx;
    }
    json.keys = structuredClone(this.keys);
    json.shapes = [];
    for (let shape of this.shapes) {
      json.shapes.push(shape.toJSON(randomizeUuid));
    }
    return json;
  }
  addShape(shape, sendToBack) {
    if (sendToBack) {
      this.shapes.unshift(shape);
    } else {
      this.shapes.push(shape);
    }
  }
  removeShape(shape) {
    let shapeIndex = this.shapes.indexOf(shape);
    if (shapeIndex >= 0) {
      this.shapes.splice(shapeIndex, 1);
    }
  }
}

class TempFrame {
  constructor() {}
  get exists() {
    return false;
  }
  get idx() {
    return "tempFrame";
  }
  get keys() {
    return {};
  }
  get shapes() {
    return [];
  }
  get frameType() {
    return "temp";
  }
  copy() {
    return this;
  }
  addShape() {}
  removeShape() {}
}

const tempFrame = new TempFrame();

// Animation system classes
class Keyframe {
  constructor(time, value, interpolation = "linear", uuid = undefined) {
    this.time = time;
    this.value = value;
    this.interpolation = interpolation; // 'linear', 'bezier', 'step', 'hold'
    // For bezier interpolation
    this.easeIn = { x: 0.42, y: 0 };  // Default ease-in control point
    this.easeOut = { x: 0.58, y: 1 }; // Default ease-out control point
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
  }

  static fromJSON(json) {
    const keyframe = new Keyframe(json.time, json.value, json.interpolation, json.idx);
    if (json.easeIn) keyframe.easeIn = json.easeIn;
    if (json.easeOut) keyframe.easeOut = json.easeOut;
    return keyframe;
  }

  toJSON() {
    return {
      idx: this.idx,
      time: this.time,
      value: this.value,
      interpolation: this.interpolation,
      easeIn: this.easeIn,
      easeOut: this.easeOut
    };
  }
}

class AnimationCurve {
  constructor(parameter, uuid = undefined, parentAnimationData = null) {
    this.parameter = parameter; // e.g., "x", "y", "rotation", "scale_x", "exists"
    this.keyframes = []; // Always kept sorted by time
    this.parentAnimationData = parentAnimationData; // Reference to parent AnimationData for duration updates
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
  }

  addKeyframe(keyframe) {
    // Time resolution based on framerate - half a frame's duration
    // This can be exposed via UI later
    const framerate = context.config?.framerate || 24;
    const timeResolution = (1 / framerate) / 2;

    // Check if there's already a keyframe within the time resolution
    const existingKeyframe = this.getKeyframeAtTime(keyframe.time, timeResolution);

    if (existingKeyframe) {
      // Update the existing keyframe's value instead of adding a new one
      existingKeyframe.value = keyframe.value;
      existingKeyframe.interpolation = keyframe.interpolation;
      if (keyframe.easeIn) existingKeyframe.easeIn = keyframe.easeIn;
      if (keyframe.easeOut) existingKeyframe.easeOut = keyframe.easeOut;
    } else {
      // Add new keyframe
      this.keyframes.push(keyframe);
      // Keep sorted by time
      this.keyframes.sort((a, b) => a.time - b.time);
    }

    // Update animation duration after adding keyframe
    if (this.parentAnimationData) {
      this.parentAnimationData.updateDuration();
    }
  }

  removeKeyframe(keyframe) {
    const index = this.keyframes.indexOf(keyframe);
    if (index >= 0) {
      this.keyframes.splice(index, 1);

      // Update animation duration after removing keyframe
      if (this.parentAnimationData) {
        this.parentAnimationData.updateDuration();
      }
    }
  }

  getKeyframeAtTime(time, timeResolution = 0) {
    if (this.keyframes.length === 0) return null;

    // If no tolerance, use exact match with binary search
    if (timeResolution === 0) {
      let left = 0;
      let right = this.keyframes.length - 1;

      while (left <= right) {
        const mid = Math.floor((left + right) / 2);
        if (this.keyframes[mid].time === time) {
          return this.keyframes[mid];
        } else if (this.keyframes[mid].time < time) {
          left = mid + 1;
        } else {
          right = mid - 1;
        }
      }
      return null;
    }

    // With tolerance, find the closest keyframe within timeResolution
    let left = 0;
    let right = this.keyframes.length - 1;
    let closest = null;
    let closestDist = Infinity;

    // Binary search to find the insertion point
    while (left <= right) {
      const mid = Math.floor((left + right) / 2);
      const dist = Math.abs(this.keyframes[mid].time - time);

      if (dist < closestDist) {
        closestDist = dist;
        closest = this.keyframes[mid];
      }

      if (this.keyframes[mid].time < time) {
        left = mid + 1;
      } else {
        right = mid - 1;
      }
    }

    // Also check adjacent keyframes for closest match
    if (left < this.keyframes.length) {
      const dist = Math.abs(this.keyframes[left].time - time);
      if (dist < closestDist) {
        closestDist = dist;
        closest = this.keyframes[left];
      }
    }
    if (right >= 0) {
      const dist = Math.abs(this.keyframes[right].time - time);
      if (dist < closestDist) {
        closestDist = dist;
        closest = this.keyframes[right];
      }
    }

    return closestDist < timeResolution ? closest : null;
  }

  // Find the two keyframes that bracket the given time
  getBracketingKeyframes(time) {
    if (this.keyframes.length === 0) return { prev: null, next: null };
    if (this.keyframes.length === 1) return { prev: this.keyframes[0], next: this.keyframes[0] };

    // Binary search to find the last keyframe at or before time
    let left = 0;
    let right = this.keyframes.length - 1;
    let prevIndex = -1;

    while (left <= right) {
      const mid = Math.floor((left + right) / 2);
      if (this.keyframes[mid].time <= time) {
        prevIndex = mid;  // This could be our answer
        left = mid + 1;   // But check if there's a better one to the right
      } else {
        right = mid - 1;  // Time is too large, search left
      }
    }

    // If time is before all keyframes
    if (prevIndex === -1) {
      return { prev: this.keyframes[0], next: this.keyframes[0], t: 0 };
    }

    // If time is after all keyframes
    if (prevIndex === this.keyframes.length - 1) {
      return { prev: this.keyframes[prevIndex], next: this.keyframes[prevIndex], t: 1 };
    }

    const prev = this.keyframes[prevIndex];
    const next = this.keyframes[prevIndex + 1];
    const t = (time - prev.time) / (next.time - prev.time);

    return { prev, next, t };
  }

  interpolate(time) {
    if (this.keyframes.length === 0) {
      return null;
    }

    const { prev, next, t } = this.getBracketingKeyframes(time);

    if (!prev || !next) {
      return null;
    }
    if (prev === next) {
      return prev.value;
    }

    // Handle different interpolation types
    switch (prev.interpolation) {
      case "step":
      case "hold":
        return prev.value;

      case "linear":
        // Simple linear interpolation
        if (typeof prev.value === "number" && typeof next.value === "number") {
          return prev.value + (next.value - prev.value) * t;
        }
        return prev.value;

      case "bezier":
        // Cubic bezier interpolation using control points
        if (typeof prev.value === "number" && typeof next.value === "number") {
          // Use ease-in/ease-out control points
          const easedT = this.cubicBezierEase(t, prev.easeOut, next.easeIn);
          return prev.value + (next.value - prev.value) * easedT;
        }
        return prev.value;

      case "zero":
        // Return 0 for the entire interval (used for inactive segments)
        return 0;

      default:
        return prev.value;
    }
  }

  // Cubic bezier easing function
  cubicBezierEase(t, easeOut, easeIn) {
    // Simplified cubic bezier for 0,0 -> easeOut -> easeIn -> 1,1
    const u = 1 - t;
    return 3 * u * u * t * easeOut.y +
           3 * u * t * t * easeIn.y +
           t * t * t;
  }

  // Display color for this curve in timeline (based on parameter type) - Phase 4
  get displayColor() {
    // Auto-determined from parameter name
    if (this.parameter.endsWith('.x')) return '#7a00b3'  // purple
    if (this.parameter.endsWith('.y')) return '#ff00ff'  // magenta
    if (this.parameter.endsWith('.rotation')) return '#5555ff'  // blue
    if (this.parameter.endsWith('.scale_x')) return '#ffaa00'  // orange
    if (this.parameter.endsWith('.scale_y')) return '#ffff55'  // yellow
    if (this.parameter.endsWith('.exists')) return '#55ff55'  // green
    if (this.parameter.endsWith('.zOrder')) return '#55ffff'  // cyan
    if (this.parameter.endsWith('.frameNumber')) return '#ff5555'  // red
    return '#ffffff'  // default white
  }

  static fromJSON(json) {
    const curve = new AnimationCurve(json.parameter, json.idx);
    for (let kfJson of json.keyframes || []) {
      curve.keyframes.push(Keyframe.fromJSON(kfJson));
    }
    return curve;
  }

  toJSON() {
    return {
      idx: this.idx,
      parameter: this.parameter,
      keyframes: this.keyframes.map(kf => kf.toJSON())
    };
  }
}

class AnimationData {
  constructor(parentLayer = null, uuid = undefined) {
    this.curves = {}; // parameter name -> AnimationCurve
    this.duration = 0; // Duration in seconds (max time of all keyframes)
    this.parentLayer = parentLayer; // Reference to parent Layer for updating segment keyframes
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
  }

  getCurve(parameter) {
    return this.curves[parameter];
  }

  getOrCreateCurve(parameter) {
    if (!this.curves[parameter]) {
      this.curves[parameter] = new AnimationCurve(parameter, undefined, this);
    }
    return this.curves[parameter];
  }

  addKeyframe(parameter, keyframe) {
    const curve = this.getOrCreateCurve(parameter);
    curve.addKeyframe(keyframe);
  }

  removeKeyframe(parameter, keyframe) {
    const curve = this.curves[parameter];
    if (curve) {
      curve.removeKeyframe(keyframe);
    }
  }

  removeCurve(parameter) {
    delete this.curves[parameter];
  }

  setCurve(parameter, curve) {
    // Set parent reference for duration tracking
    curve.parentAnimationData = this;
    this.curves[parameter] = curve;
    // Update duration after adding curve with keyframes
    this.updateDuration();
  }

  interpolate(parameter, time) {
    const curve = this.curves[parameter];
    if (!curve) return null;
    return curve.interpolate(time);
  }

  // Get all animated values at a given time
  getValuesAtTime(time) {
    const values = {};
    for (let parameter in this.curves) {
      values[parameter] = this.curves[parameter].interpolate(time);
    }
    return values;
  }

  /**
   * Update the duration based on all keyframes
   * Called automatically when keyframes are added/removed
   */
  updateDuration() {
    // Calculate max time from all keyframes in all curves
    let maxTime = 0;
    for (let parameter in this.curves) {
      const curve = this.curves[parameter];
      if (curve.keyframes && curve.keyframes.length > 0) {
        const lastKeyframe = curve.keyframes[curve.keyframes.length - 1];
        maxTime = Math.max(maxTime, lastKeyframe.time);
      }
    }

    // Update this AnimationData's duration
    this.duration = maxTime;

    // If this layer belongs to a nested group, update the segment keyframes in the parent
    if (this.parentLayer && this.parentLayer.parentObject) {
      this.updateParentSegmentKeyframes();
    }
  }

  /**
   * Update segment keyframes in parent layer when this layer's duration changes
   * This ensures that nested group segments automatically resize when internal animation is added
   */
  updateParentSegmentKeyframes() {
    const parentObject = this.parentLayer.parentObject;

    // Get the layer that contains this nested object (parentObject.parentLayer)
    if (!parentObject.parentLayer || !parentObject.parentLayer.animationData) {
      return;
    }

    const parentLayer = parentObject.parentLayer;

    // Get the frameNumber curve for this nested object using the correct naming convention
    const curveName = `child.${parentObject.idx}.frameNumber`;
    const frameNumberCurve = parentLayer.animationData.getCurve(curveName);

    if (!frameNumberCurve || frameNumberCurve.keyframes.length < 2) {
      return;
    }

    // Update the last keyframe to match the new duration
    const lastKeyframe = frameNumberCurve.keyframes[frameNumberCurve.keyframes.length - 1];
    const newFrameValue = Math.ceil(this.duration * config.framerate) + 1; // +1 because frameNumber is 1-indexed
    const newTime = this.duration;

    // Only update if the time or value actually changed
    if (lastKeyframe.value !== newFrameValue || lastKeyframe.time !== newTime) {
      lastKeyframe.value = newFrameValue;
      lastKeyframe.time = newTime;

      // Re-sort keyframes in case the time change affects order
      frameNumberCurve.keyframes.sort((a, b) => a.time - b.time);

      // Don't recursively call updateDuration to avoid infinite loop
    }
  }

  static fromJSON(json, parentLayer = null) {
    const animData = new AnimationData(parentLayer, json.idx);
    for (let param in json.curves || {}) {
      const curve = AnimationCurve.fromJSON(json.curves[param]);
      curve.parentAnimationData = animData; // Restore parent reference
      animData.curves[param] = curve;
    }
    // Recalculate duration after loading all curves
    animData.updateDuration();
    return animData;
  }

  toJSON() {
    const curves = {};
    for (let param in this.curves) {
      curves[param] = this.curves[param].toJSON();
    }
    return {
      idx: this.idx,
      curves: curves
    };
  }
}

export { Frame, TempFrame, tempFrame, Keyframe, AnimationCurve, AnimationData };

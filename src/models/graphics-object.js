// GraphicsObject model: Main container for layers and animation

import { context, config, pointerList, startProps } from '../state.js';
import { VectorLayer, AudioTrack, VideoLayer } from './layer.js';
import { TempShape } from './shapes.js';
import { AnimationCurve, Keyframe } from './animation.js';
import { Widget } from '../widgets.js';

// Helper function for UUID generation
function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

// Forward declarations for dependencies that will be injected
let growBoundingBox = null;
let getRotatedBoundingBox = null;
let multiplyMatrices = null;
let uuidToColor = null;

// Initialize function to be called from main.js
export function initializeGraphicsObjectDependencies(deps) {
  growBoundingBox = deps.growBoundingBox;
  getRotatedBoundingBox = deps.getRotatedBoundingBox;
  multiplyMatrices = deps.multiplyMatrices;
  uuidToColor = deps.uuidToColor;
}

class GraphicsObject extends Widget {
  constructor(uuid, initialChildType = 'layer') {
    super(0, 0)
    this.rotation = 0; // in radians
    this.scale_x = 1;
    this.scale_y = 1;
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    pointerList[this.idx] = this;
    this.name = this.idx;

    this.currentFrameNum = 0; // LEGACY: kept for backwards compatibility
    this._currentTime = 0; // Internal storage for currentTime
    this.currentLayer = 0;

    // Make currentTime a getter/setter property
    Object.defineProperty(this, 'currentTime', {
      get: function() {
        return this._currentTime;
      },
      set: function(value) {
        this._currentTime = value;
      },
      enumerable: true,
      configurable: true
    });
    this._activeAudioTrack = null; // Reference to active audio track (if any)

    // Initialize children and audioTracks based on initialChildType
    this.children = [];
    this.audioTracks = [];

    if (initialChildType === 'layer') {
      this.children = [new VectorLayer(uuid + "-L1", this)];
      this.currentLayer = 0;  // Set first layer as active
    } else if (initialChildType === 'video') {
      this.children = [new VideoLayer(uuid + "-V1", "Video 1")];
      this.currentLayer = 0;  // Set first video layer as active
    } else if (initialChildType === 'midi') {
      const midiTrack = new AudioTrack(uuid + "-M1", "MIDI 1", 'midi');
      this.audioTracks.push(midiTrack);
      this._activeAudioTrack = midiTrack;  // Set MIDI track as active (the object, not index)
      // Initialize the MIDI track in the audio backend
      midiTrack.initializeTrack().catch(err => {
        console.error('Failed to initialize MIDI track:', err);
      });
    } else if (initialChildType === 'audio') {
      const audioTrack = new AudioTrack(uuid + "-A1", "Audio 1", 'audio');
      this.audioTracks.push(audioTrack);
      this._activeAudioTrack = audioTrack;  // Set audio track as active (the object, not index)
      audioTrack.initializeTrack().catch(err => {
        console.error('Failed to initialize audio track:', err);
      });
    }
    // If initialChildType is 'none' or anything else, leave both arrays empty

    this.shapes = [];

    // Parent reference for nested objects (set when added to a layer)
    this.parentLayer = null

    // Timeline display settings (Phase 3)
    this.showSegment = true  // Show segment bar in timeline
    this.curvesMode = 'keyframe'  // 'segment' | 'keyframe' | 'curve'
    this.curvesHeight = 150  // Height in pixels when curves are in curve view

    this._globalEvents.add("mousedown")
    this._globalEvents.add("mousemove")
    this._globalEvents.add("mouseup")
  }
  static fromJSON(json) {
    const graphicsObject = new GraphicsObject(json.idx);
    graphicsObject.x = json.x;
    graphicsObject.y = json.y;
    graphicsObject.rotation = json.rotation;
    graphicsObject.scale_x = json.scale_x;
    graphicsObject.scale_y = json.scale_y;
    graphicsObject.name = json.name;
    graphicsObject.currentFrameNum = json.currentFrameNum;
    graphicsObject.currentLayer = json.currentLayer;
    graphicsObject.children = [];
    if (json.parent in pointerList) {
      graphicsObject.parent = pointerList[json.parent]
    }
    for (let layer of json.layers) {
      if (layer.type === 'VideoLayer') {
        graphicsObject.layers.push(VideoLayer.fromJSON(layer));
      } else {
        // Default to VectorLayer
        graphicsObject.layers.push(VectorLayer.fromJSON(layer, graphicsObject));
      }
    }
    // Handle audioTracks (may not exist in older files)
    if (json.audioTracks) {
      for (let audioTrack of json.audioTracks) {
        graphicsObject.audioTracks.push(AudioTrack.fromJSON(audioTrack));
      }
    }
    return graphicsObject;
  }
  toJSON(randomizeUuid = false) {
    const json = {};
    json.type = "GraphicsObject";
    json.x = this.x;
    json.y = this.y;
    json.rotation = this.rotation;
    json.scale_x = this.scale_x;
    json.scale_y = this.scale_y;
    if (randomizeUuid) {
      json.idx = uuidv4();
      json.name = this.name + " copy";
    } else {
      json.idx = this.idx;
      json.name = this.name;
    }
    json.currentFrameNum = this.currentFrameNum;
    json.currentLayer = this.currentLayer;
    json.layers = [];
    json.parent = this.parent?.idx
    for (let layer of this.layers) {
      json.layers.push(layer.toJSON(randomizeUuid));
    }
    json.audioTracks = [];
    for (let audioTrack of this.audioTracks) {
      json.audioTracks.push(audioTrack.toJSON(randomizeUuid));
    }
    return json;
  }
  get activeLayer() {
    // If an audio track is active, return it instead of a visual layer
    if (this._activeAudioTrack !== null) {
      return this._activeAudioTrack;
    }
    return this.layers[this.currentLayer];
  }
  set activeLayer(layer) {
    // Allow setting activeLayer to an AudioTrack or a regular Layer
    if (layer instanceof AudioTrack) {
      this._activeAudioTrack = layer;
    } else {
      // It's a regular layer - find its index and set currentLayer
      this._activeAudioTrack = null;
      const layerIndex = this.children.indexOf(layer);
      if (layerIndex !== -1) {
        this.currentLayer = layerIndex;
      }
    }
  }
  // get children() {
  //   return this.activeLayer.children;
  // }
  get layers() {
    return this.children
  }

  /**
   * Get the total duration of this GraphicsObject's animation
   * Returns the maximum duration across all layers
   */
  get duration() {
    let maxDuration = 0;

    // Check visual layers
    for (let layer of this.layers) {
      // Check animation data duration
      if (layer.animationData && layer.animationData.duration > maxDuration) {
        maxDuration = layer.animationData.duration;
      }

      // Check video layer clips (VideoLayer has clips like AudioTrack)
      if (layer.type === 'video' && layer.clips) {
        for (let clip of layer.clips) {
          const clipEnd = clip.startTime + clip.duration;
          if (clipEnd > maxDuration) {
            maxDuration = clipEnd;
          }
        }
      }
    }

    // Check audio tracks
    for (let audioTrack of this.audioTracks) {
      for (let clip of audioTrack.clips) {
        const clipEnd = clip.startTime + clip.duration;
        if (clipEnd > maxDuration) {
          maxDuration = clipEnd;
        }
      }
    }

    return maxDuration;
  }
  get allLayers() {
    return [...this.audioTracks, ...this.layers];
  }
  get maxFrame() {
    return (
      Math.max(
        ...this.layers.map((layer) => {
          return (
            layer.frames.findLastIndex((frame) => frame !== undefined) || -1
          );
        }),
      ) + 1
    );
  }
  get segmentColor() {
    return uuidToColor(this.idx);
  }
  /**
   * Set the current playback time in seconds
   */
  setTime(time) {
    time = Math.max(0, time);
    this.currentTime = time;

    // Update legacy currentFrameNum for any remaining code that needs it
    this.currentFrameNum = Math.floor(time * config.framerate);

    // Update layer frameNum for legacy code
    for (let layer of this.layers) {
      layer.frameNum = this.currentFrameNum;
    }
  }

  advanceFrame() {
    const frameDuration = 1 / config.framerate;
    this.setTime(this.currentTime + frameDuration);
  }

  decrementFrame() {
    const frameDuration = 1 / config.framerate;
    this.setTime(Math.max(0, this.currentTime - frameDuration));
  }
  bbox() {
    let bbox;

    // NEW: Include shapes from AnimationData system
    let currentTime = this.currentTime || 0;
    for (let layer of this.layers) {
      for (let shape of layer.shapes) {
        // Check if shape exists at current time
        let existsValue = layer.animationData.interpolate(`shape.${shape.shapeId}.exists`, currentTime);
        if (existsValue !== null && existsValue > 0) {
          if (!bbox) {
            bbox = structuredClone(shape.boundingBox);
          } else {
            growBoundingBox(bbox, shape.boundingBox);
          }
        }
      }
    }

    // Include children
    if (this.children.length > 0) {
      if (!bbox) {
        bbox = structuredClone(this.children[0].bbox());
      }
      for (let child of this.children) {
        growBoundingBox(bbox, child.bbox());
      }
    }

    if (bbox == undefined) {
      bbox = { x: { min: 0, max: 0 }, y: { min: 0, max: 0 } };
    }
    bbox.x.max *= this.scale_x;
    bbox.y.max *= this.scale_y;
    bbox.x.min += this.x;
    bbox.x.max += this.x;
    bbox.y.min += this.y;
    bbox.y.max += this.y;
    return bbox;
  }

  draw(context, calculateTransform=false) {
    let ctx = context.ctx;
    ctx.save();
    if (calculateTransform) {
      this.transformCanvas(ctx)
    } else {
      ctx.translate(this.x, this.y);
      ctx.rotate(this.rotation);
      ctx.scale(this.scale_x, this.scale_y);
    }
    // if (this.currentFrameNum>=this.maxFrame) {
    //   this.currentFrameNum = 0;
    // }
    if (
      context.activeAction &&
      context.activeAction.selection &&
      this.idx in context.activeAction.selection
    )
      return;

    for (let layer of this.layers) {
      if (context.activeObject == this && !layer.visible) continue;

      // Handle VideoLayer differently - call its draw method
      if (layer.type === 'video') {
        layer.draw(context);
        continue;
      }

      // Draw activeShape (shape being drawn in progress) for active layer only
      if (layer === context.activeLayer && layer.activeShape) {
        let cxt = {...context};
        layer.activeShape.draw(cxt);
      }

      // NEW: Use AnimationData system to draw shapes with shape tweening/morphing
      let currentTime = this.currentTime || 0;

      // Group shapes by shapeId (multiple Shape objects can share a shapeId for tweening)
      const shapesByShapeId = new Map();
      for (let shape of layer.shapes) {
        if (shape instanceof TempShape) continue;
        if (!shapesByShapeId.has(shape.shapeId)) {
          shapesByShapeId.set(shape.shapeId, []);
        }
        shapesByShapeId.get(shape.shapeId).push(shape);
      }

      // Process each logical shape (shapeId) and determine what to draw
      let visibleShapes = [];
      for (let [shapeId, shapes] of shapesByShapeId) {
        // Check if this logical shape exists at current time
        const existsCurveKey = `shape.${shapeId}.exists`;
        let existsValue = layer.animationData.interpolate(existsCurveKey, currentTime);
        
        if (existsValue === null || existsValue <= 0) {
          console.log(`[Widget.draw] Skipping shape ${shapeId} - not visible`);
          continue;
        }

        // Get z-order
        let zOrder = layer.animationData.interpolate(`shape.${shapeId}.zOrder`, currentTime);

        // Get shapeIndex curve and surrounding keyframes
        const shapeIndexCurve = layer.animationData.getCurve(`shape.${shapeId}.shapeIndex`);
        if (!shapeIndexCurve || !shapeIndexCurve.keyframes || shapeIndexCurve.keyframes.length === 0) {
          // No shapeIndex curve, just show shape with index 0
          const shape = shapes.find(s => s.shapeIndex === 0);
          if (shape) {
            visibleShapes.push({
              shape,
              zOrder: zOrder || 0,
              selected: context.shapeselection.includes(shape)
            });
          }
          continue;
        }

        // Find surrounding keyframes using AnimationCurve's built-in method
        const { prev: prevKf, next: nextKf, t: interpolationT } = shapeIndexCurve.getBracketingKeyframes(currentTime);

        // Get interpolated value
        let shapeIndexValue = shapeIndexCurve.interpolate(currentTime);
        if (shapeIndexValue === null) shapeIndexValue = 0;

        // Sort shape versions by shapeIndex
        shapes.sort((a, b) => a.shapeIndex - b.shapeIndex);

        // Determine whether to morph based on whether interpolated value equals a keyframe value
        const atPrevKeyframe = prevKf && Math.abs(shapeIndexValue - prevKf.value) < 0.001;
        const atNextKeyframe = nextKf && Math.abs(shapeIndexValue - nextKf.value) < 0.001;

        if (atPrevKeyframe || atNextKeyframe) {
          // No morphing - display the shape at the keyframe value
          const targetValue = atNextKeyframe ? nextKf.value : prevKf.value;
          const shape = shapes.find(s => s.shapeIndex === targetValue);
          if (shape) {
            visibleShapes.push({
              shape,
              zOrder: zOrder || 0,
              selected: context.shapeselection.includes(shape)
            });
          }
        } else if (prevKf && nextKf && prevKf.value !== nextKf.value) {
          // Morph between shapes specified by surrounding keyframes
          const shape1 = shapes.find(s => s.shapeIndex === prevKf.value);
          const shape2 = shapes.find(s => s.shapeIndex === nextKf.value);

          if (shape1 && shape2) {
            // Use the interpolated shapeIndexValue to calculate blend factor
            // This respects the bezier easing curve
            const t = (shapeIndexValue - prevKf.value) / (nextKf.value - prevKf.value);
            console.log(`[Widget.draw] Morphing from shape ${prevKf.value} to ${nextKf.value}, shapeIndexValue=${shapeIndexValue}, t=${t}`);
            const morphedShape = shape1.lerpShape(shape2, t);
            visibleShapes.push({
              shape: morphedShape,
              zOrder: zOrder || 0,
              selected: context.shapeselection.includes(shape1) || context.shapeselection.includes(shape2)
            });
          } else if (shape1) {
            visibleShapes.push({
              shape: shape1,
              zOrder: zOrder || 0,
              selected: context.shapeselection.includes(shape1)
            });
          } else if (shape2) {
            visibleShapes.push({
              shape: shape2,
              zOrder: zOrder || 0,
              selected: context.shapeselection.includes(shape2)
            });
          }
        } else if (nextKf) {
          // Only next keyframe exists, show that shape
          const shape = shapes.find(s => s.shapeIndex === nextKf.value);
          if (shape) {
            visibleShapes.push({
              shape,
              zOrder: zOrder || 0,
              selected: context.shapeselection.includes(shape)
            });
          }
        }
      }

      // Sort by zOrder
      visibleShapes.sort((a, b) => a.zOrder - b.zOrder);

      // Draw sorted shapes
      for (let { shape, selected } of visibleShapes) {
        let cxt = {...context}
        if (selected) {
          cxt.selected = true
        }
        shape.draw(cxt);
      }

      // Draw child objects using AnimationData curves
      for (let child of layer.children) {
        if (child == context.activeObject) continue;
        let idx = child.idx;

        // Use AnimationData to get child's transform
        let childX = layer.animationData.interpolate(`child.${idx}.x`, currentTime);
        let childY = layer.animationData.interpolate(`child.${idx}.y`, currentTime);
        let childRotation = layer.animationData.interpolate(`child.${idx}.rotation`, currentTime);
        let childScaleX = layer.animationData.interpolate(`child.${idx}.scale_x`, currentTime);
        let childScaleY = layer.animationData.interpolate(`child.${idx}.scale_y`, currentTime);
        let childFrameNumber = layer.animationData.interpolate(`child.${idx}.frameNumber`, currentTime);

        if (childX !== null && childY !== null) {
          child.x = childX;
          child.y = childY;
          child.rotation = childRotation || 0;
          child.scale_x = childScaleX || 1;
          child.scale_y = childScaleY || 1;

          // Set child's currentTime based on its frameNumber
          // frameNumber 1 = time 0, frameNumber 2 = time 1/framerate, etc.
          if (childFrameNumber !== null) {
            child.currentTime = (childFrameNumber - 1) / config.framerate;
          }

          ctx.save();
          child.draw(context);
          ctx.restore();
        }
      }
    }
    if (this == context.activeObject) {
      // Draw selection rectangles for selected items
      if (context.mode == "select") {
        for (let item of context.selection) {
          if (!item) continue;
          ctx.save();
          ctx.strokeStyle = "#00ffff";
          ctx.lineWidth = 1;
          ctx.beginPath();
          let bbox = getRotatedBoundingBox(item);
          ctx.rect(
            bbox.x.min,
            bbox.y.min,
            bbox.x.max - bbox.x.min,
            bbox.y.max - bbox.y.min,
          );
          ctx.stroke();
          ctx.restore();
        }
        // Draw drag selection rectangle
        if (context.selectionRect) {
          ctx.save();
          ctx.strokeStyle = "#00ffff";
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.rect(
            context.selectionRect.x1,
            context.selectionRect.y1,
            context.selectionRect.x2 - context.selectionRect.x1,
            context.selectionRect.y2 - context.selectionRect.y1,
          );
          ctx.stroke();
          ctx.restore();
        }
      } else if (context.mode == "transform") {
        let bbox = undefined;
        for (let item of context.selection) {
          if (bbox == undefined) {
            bbox = getRotatedBoundingBox(item);
          } else {
            growBoundingBox(bbox, getRotatedBoundingBox(item));
          }
        }
        if (bbox != undefined) {
          ctx.save();
          ctx.strokeStyle = "#00ffff";
          ctx.lineWidth = 1;
          ctx.beginPath();
          let xdiff = bbox.x.max - bbox.x.min;
          let ydiff = bbox.y.max - bbox.y.min;
          ctx.rect(bbox.x.min, bbox.y.min, xdiff, ydiff);
          ctx.stroke();
          ctx.fillStyle = "#000000";
          let rectRadius = 5;
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
              bbox.x.min + xdiff * i[0] - rectRadius,
              bbox.y.min + ydiff * i[1] - rectRadius,
              rectRadius * 2,
              rectRadius * 2,
            );
            ctx.fill();
          }
          ctx.restore();
        }
      }

      if (context.activeCurve) {
        ctx.strokeStyle = "magenta";
        ctx.beginPath();
        ctx.moveTo(
          context.activeCurve.current.points[0].x,
          context.activeCurve.current.points[0].y,
        );
        ctx.bezierCurveTo(
          context.activeCurve.current.points[1].x,
          context.activeCurve.current.points[1].y,
          context.activeCurve.current.points[2].x,
          context.activeCurve.current.points[2].y,
          context.activeCurve.current.points[3].x,
          context.activeCurve.current.points[3].y,
        );
        ctx.stroke();
      }
      if (context.activeVertex) {
        ctx.save();
        ctx.strokeStyle = "#00ffff";
        let curves = {
          ...context.activeVertex.current.startCurves,
          ...context.activeVertex.current.endCurves,
        };
        // I don't understand why I can't use a for...of loop here
        for (let idx in curves) {
          let curve = curves[idx];
          ctx.beginPath();
          ctx.moveTo(curve.points[0].x, curve.points[0].y);
          ctx.bezierCurveTo(
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
            curve.points[3].x,
            curve.points[3].y,
          );
          ctx.stroke();
        }
        ctx.fillStyle = "#000000aa";
        ctx.beginPath();
        let vertexSize = 15 / context.zoomLevel;
        ctx.rect(
          context.activeVertex.current.point.x - vertexSize / 2,
          context.activeVertex.current.point.y - vertexSize / 2,
          vertexSize,
          vertexSize,
        );
        ctx.fill();
        ctx.restore();
      }
    }
    ctx.restore();
  }
  /*
  draw(ctx) {
    super.draw(ctx)
    if (this==context.activeObject) {
      if (context.mode == "select") {
        for (let item of context.selection) {
          if (!item) continue;
          // Check if this is a child object and if it exists at current time
          if (item.idx) {
            const existsValue = this.activeLayer.animationData.interpolate(
              `object.${item.idx}.exists`,
              this.currentTime
            );
            if (existsValue === null || existsValue <= 0) continue;
          }
          ctx.save();
          ctx.strokeStyle = "#00ffff";
          ctx.lineWidth = 1;
          ctx.beginPath();
          let bbox = getRotatedBoundingBox(item);
          ctx.rect(
            bbox.x.min,
            bbox.y.min,
            bbox.x.max - bbox.x.min,
            bbox.y.max - bbox.y.min,
          );
          ctx.stroke();
          ctx.restore();
        }
        if (context.selectionRect) {
          ctx.save();
          ctx.strokeStyle = "#00ffff";
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.rect(
            context.selectionRect.x1,
            context.selectionRect.y1,
            context.selectionRect.x2 - context.selectionRect.x1,
            context.selectionRect.y2 - context.selectionRect.y1,
          );
          ctx.stroke();
          ctx.restore();
        }
      } else if (context.mode == "transform") {
        let bbox = undefined;
        for (let item of context.selection) {
          if (bbox == undefined) {
            bbox = getRotatedBoundingBox(item);
          } else {
            growBoundingBox(bbox, getRotatedBoundingBox(item));
          }
        }
        if (bbox != undefined) {
          ctx.save();
          ctx.strokeStyle = "#00ffff";
          ctx.lineWidth = 1;
          ctx.beginPath();
          let xdiff = bbox.x.max - bbox.x.min;
          let ydiff = bbox.y.max - bbox.y.min;
          ctx.rect(bbox.x.min, bbox.y.min, xdiff, ydiff);
          ctx.stroke();
          ctx.fillStyle = "#000000";
          let rectRadius = 5;
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
              bbox.x.min + xdiff * i[0] - rectRadius,
              bbox.y.min + ydiff * i[1] - rectRadius,
              rectRadius * 2,
              rectRadius * 2,
            );
            ctx.fill();
          }

          ctx.restore();
        }
      }
    }
  }
  */
  transformCanvas(ctx) {
    if (this.parent) {
      this.parent.transformCanvas(ctx)
    }
    ctx.translate(this.x, this.y);
    ctx.scale(this.scale_x, this.scale_y);
    ctx.rotate(this.rotation);
  }
  transformMouse(mouse) {
    // Apply the transformation matrix to the mouse position
    let matrix = this.generateTransformMatrix();
    let { x, y } = mouse;
  
    return {
      x: matrix[0][0] * x + matrix[0][1] * y + matrix[0][2],
      y: matrix[1][0] * x + matrix[1][1] * y + matrix[1][2]
    };
  }
  generateTransformMatrix() {
    // Start with the parent's transform matrix if it exists
    let parentMatrix = this.parent ? this.parent.generateTransformMatrix() : [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
  
    // Calculate the rotation matrix components
    const cos = Math.cos(this.rotation);
    const sin = Math.sin(this.rotation);
  
    // Scaling matrix
    const scaleMatrix = [
      [1/this.scale_x, 0, 0],
      [0, 1/this.scale_y, 0],
      [0, 0, 1]
    ];
  
    // Rotation matrix (inverse rotation for transforming back)
    const rotationMatrix = [
      [cos, -sin, 0],
      [sin, cos, 0],
      [0, 0, 1]
    ];
  
    // Translation matrix (inverse translation to adjust for object's position)
    const translationMatrix = [
      [1, 0, -this.x],
      [0, 1, -this.y],
      [0, 0, 1]
    ];
  
    // Multiply translation * rotation * scaling to get the current object's final transformation matrix
    let tempMatrix = multiplyMatrices(translationMatrix, rotationMatrix);
    let objectMatrix = multiplyMatrices(tempMatrix, scaleMatrix);
  
    // Now combine with the parent's matrix (parent * object)
    let finalMatrix = multiplyMatrices(parentMatrix, objectMatrix);
  
    return finalMatrix;
  }
  handleMouseEvent(eventType, x, y) {
    for (let i in this.layers) {
      if (i==this.currentLayer) {
        this.layers[i]._globalEvents.add("mousedown")
        this.layers[i]._globalEvents.add("mousemove")
        this.layers[i]._globalEvents.add("mouseup")
      } else {
        this.layers[i]._globalEvents.delete("mousedown")
        this.layers[i]._globalEvents.delete("mousemove")
        this.layers[i]._globalEvents.delete("mouseup")
      }
    }
    super.handleMouseEvent(eventType, x, y)
  }
  addObject(object, x = 0, y = 0, time = undefined, layer=undefined) {
    if (time == undefined) {
      time = this.currentTime || 0;
    }
    if (layer==undefined) {
      layer = this.activeLayer
    }

    layer.children.push(object)
    object.parent = this;
    object.parentLayer = layer;
    object.x = x;
    object.y = y;
    let idx = object.idx;

    // Add animation curves for the object's position/transform in the layer
    let xCurve = new AnimationCurve(`child.${idx}.x`);
    xCurve.addKeyframe(new Keyframe(time, x, 'linear'));
    layer.animationData.setCurve(`child.${idx}.x`, xCurve);

    let yCurve = new AnimationCurve(`child.${idx}.y`);
    yCurve.addKeyframe(new Keyframe(time, y, 'linear'));
    layer.animationData.setCurve(`child.${idx}.y`, yCurve);

    let rotationCurve = new AnimationCurve(`child.${idx}.rotation`);
    rotationCurve.addKeyframe(new Keyframe(time, 0, 'linear'));
    layer.animationData.setCurve(`child.${idx}.rotation`, rotationCurve);

    let scaleXCurve = new AnimationCurve(`child.${idx}.scale_x`);
    scaleXCurve.addKeyframe(new Keyframe(time, 1, 'linear'));
    layer.animationData.setCurve(`child.${idx}.scale_x`, scaleXCurve);

    let scaleYCurve = new AnimationCurve(`child.${idx}.scale_y`);
    scaleYCurve.addKeyframe(new Keyframe(time, 1, 'linear'));
    layer.animationData.setCurve(`child.${idx}.scale_y`, scaleYCurve);

    // Add exists curve (object visibility)
    let existsCurve = new AnimationCurve(`object.${idx}.exists`);
    existsCurve.addKeyframe(new Keyframe(time, 1, 'hold'));
    layer.animationData.setCurve(`object.${idx}.exists`, existsCurve);

    // Initialize frameNumber curve with two keyframes defining the segment
    // The segment length is based on the object's internal animation duration
    let frameNumberCurve = new AnimationCurve(`child.${idx}.frameNumber`);

    // Get the object's animation duration (max time across all its layers)
    const objectDuration = object.duration || 0;
    const framerate = config.framerate;

    // Calculate the last frame number (frameNumber 1 = time 0, so add 1)
    const lastFrameNumber = Math.max(1, Math.ceil(objectDuration * framerate) + 1);

    // Calculate the end time for the segment (minimum 1 frame duration)
    const segmentDuration = Math.max(objectDuration, 1 / framerate);
    const endTime = time + segmentDuration;

    // Start keyframe: frameNumber 1 at the current time, linear interpolation
    frameNumberCurve.addKeyframe(new Keyframe(time, 1, 'linear'));

    // End keyframe: last frame at end time, zero interpolation (inactive after this)
    frameNumberCurve.addKeyframe(new Keyframe(endTime, lastFrameNumber, 'zero'));

    layer.animationData.setCurve(`child.${idx}.frameNumber`, frameNumberCurve);
  }
  removeChild(childObject) {
    let idx = childObject.idx;
    for (let layer of this.layers) {
      layer.children = layer.children.filter(child => child.idx !== idx);
      for (let frame of layer.frames) {
        if (frame) {
          delete frame[idx];
        }
      }
    }
    // this.children.splice(this.children.indexOf(childObject), 1);
  }

  /**
   * Update this object's frameNumber curve in its parent layer based on child content
   * This is called when shapes/children are added/modified within this object
   */
  updateFrameNumberCurve() {
    // Find parent layer that contains this object
    if (!this.parent || !this.parent.animationData) return;

    const parentLayer = this.parent;
    const frameNumberKey = `child.${this.idx}.frameNumber`;

    // Collect all keyframe times from this object's content
    let allKeyframeTimes = new Set();

    // Check all layers in this object
    for (let layer of this.layers) {
      if (!layer.animationData) continue;

      // Get keyframes from all shape curves
      for (let shape of layer.shapes) {
        const existsKey = `shape.${shape.shapeId}.exists`;
        const existsCurve = layer.animationData.curves[existsKey];
        if (existsCurve && existsCurve.keyframes) {
          for (let kf of existsCurve.keyframes) {
            allKeyframeTimes.add(kf.time);
          }
        }
      }

      // Get keyframes from all child object curves
      for (let child of layer.children) {
        const childFrameNumberKey = `child.${child.idx}.frameNumber`;
        const childFrameNumberCurve = layer.animationData.curves[childFrameNumberKey];
        if (childFrameNumberCurve && childFrameNumberCurve.keyframes) {
          for (let kf of childFrameNumberCurve.keyframes) {
            allKeyframeTimes.add(kf.time);
          }
        }
      }
    }

    if (allKeyframeTimes.size === 0) return;

    // Sort times
    const times = Array.from(allKeyframeTimes).sort((a, b) => a - b);
    const firstTime = times[0];
    const lastTime = times[times.length - 1];

    // Calculate frame numbers (1-based)
    const framerate = this.framerate || 24;
    const firstFrame = Math.floor(firstTime * framerate) + 1;
    const lastFrame = Math.floor(lastTime * framerate) + 1;

    // Update or create frameNumber curve in parent layer
    let frameNumberCurve = parentLayer.animationData.curves[frameNumberKey];
    if (!frameNumberCurve) {
      frameNumberCurve = new AnimationCurve(frameNumberKey);
      parentLayer.animationData.setCurve(frameNumberKey, frameNumberCurve);
    }

    // Clear existing keyframes and add new ones
    frameNumberCurve.keyframes = [];
    frameNumberCurve.addKeyframe(new Keyframe(firstTime, firstFrame, 'hold'));
    frameNumberCurve.addKeyframe(new Keyframe(lastTime, lastFrame, 'hold'));
  }

  addLayer(layer) {
    this.children.push(layer);
  }
  removeLayer(layer) {
    this.children.splice(this.children.indexOf(layer), 1);
  }
  saveState() {
    startProps[this.idx] = {
      x: this.x,
      y: this.y,
      rotation: this.rotation,
      scale_x: this.scale_x,
      scale_y: this.scale_y,
    };
  }
  copy(idx) {
    let newGO = new GraphicsObject(idx.slice(0, 8) + this.idx.slice(8));
    newGO.x = this.x;
    newGO.y = this.y;
    newGO.rotation = this.rotation;
    newGO.scale_x = this.scale_x;
    newGO.scale_y = this.scale_y;
    newGO.parent = this.parent;
    pointerList[this.idx] = this;

    newGO.layers = [];
    for (let layer of this.layers) {
      newGO.layers.push(layer.copy(idx));
    }
    for (let audioTrack of this.audioTracks) {
      newGO.audioTracks.push(audioTrack.copy(idx));
    }

    return newGO;
  }
}

export { GraphicsObject };

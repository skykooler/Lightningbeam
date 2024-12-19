const { invoke } = window.__TAURI__.core;
import * as fitCurve from '/fit-curve.js';
import { Bezier } from "/bezier.js";
import { Quadtree } from './quadtree.js';
import { createNewFileDialog, showNewFileDialog, closeDialog } from './newfile.js';
import { titleCase, getMousePositionFraction, getKeyframesSurrounding, invertPixels, lerpColor, lerp, camelToWords, generateWaveform, floodFillRegion, getShapeAtPoint, hslToRgb, drawCheckerboardBackground, hexToHsl, hsvToRgb, hexToHsv, rgbToHex, clamp } from './utils.js';
const { writeTextFile: writeTextFile, readTextFile: readTextFile, writeFile: writeFile, readFile: readFile }=  window.__TAURI__.fs;
const {
  open: openFileDialog,
  save: saveFileDialog,
  message: messageDialog,
  confirm: confirmDialog,
} = window.__TAURI__.dialog;
const { documentDir, join, basename } = window.__TAURI__.path;
const { Menu, MenuItem, Submenu } = window.__TAURI__.menu ;
const { getCurrentWindow } = window.__TAURI__.window;
const { getVersion } = window.__TAURI__.app;
const { warn, debug, trace, info, error } = window.__TAURI__.log;

function forwardConsole(fnName, logger) {
  const original = console[fnName];
  console[fnName] = (message) => {
    original(message);
    logger(message);
  };
}

// forwardConsole('log', trace);
forwardConsole('debug', debug);
forwardConsole('info', info);
forwardConsole('warn', warn);
forwardConsole('error', error);

// Debug flags
const debugQuadtree = false
const debugPaintbucket = false

const macOS = navigator.userAgent.includes('Macintosh')

let simplifyPolyline = simplify

let greetInputEl;
let greetMsgEl;
let rootPane;

let canvases = [];

let debugCurves = [];
let debugPoints = [];

let mode = "select"

let minSegmentSize = 5;
let maxSmoothAngle = 0.6;

let undoStack = [];
let redoStack = [];
let lastSaveIndex = 0;

let layoutElements = []

let minFileVersion = "1.2"
let maxFileVersion = "2.0"

let filePath = undefined
let fileExportPath = undefined
let fileWidth = 1500
let fileHeight = 1000
let fileFps = 12


let playing = false

let clipboard = []

let tools = {
  select: {
    icon: "/assets/select.svg",
    properties: {
      "selectedObjects": {
        type: "text",
        label: "Selected Object",
        enabled: () => context.selection.length==1,
        value: {
          get: () => {
            if (context.selection.length==1) {
              return context.selection[0].name
            } else if (context.selection.length==0) {
              return ""
            } else {
              return "<multiple>"
            }
          },
          set: (val) => {
            if (context.selection.length==1) {
              actions.setName.create(context.selection[0], val)
            }
          }
        }
      },
      "goToFrame": {
        type: "number",
        label: "Go To Frame",
        enabled: () => context.selection.length==1,
        value: {
          get: () => {
            if (context.selection.length != 1) return undefined
            const selectedObject = context.selection[0]
            return context.activeObject.currentFrame.keys[selectedObject.idx].goToFrame
          },
          set: (val) => {
            if (context.selection.length != 1) return undefined
            const selectedObject = context.selection[0]
            context.activeObject.currentFrame.keys[selectedObject.idx].goToFrame = val
            selectedObject.setFrameNum(val-1)
            updateUI()
          }
        }
      },
      "play": {
        type: "boolean",
        label: "Play",
        enabled: () => context.selection.length==1
      }
    }

  },
  transform: {
    icon: "/assets/transform.svg",
    properties: {}

  },
  draw: {
    icon: "/assets/draw.svg",
    properties: {
      "lineWidth": {
        type: "number",
        label: "Line Width"
      },
      "simplifyMode": {
        type: "enum",
        options: ["corners", "smooth"], // "auto"],
        label: "Line Mode"
      },
      "fillShape": {
        type: "boolean",
        label: "Fill Shape"
      }
    }
  },
  rectangle: {
    icon: "/assets/rectangle.svg",
    properties: {
      "lineWidth": {
        type: "number",
        label: "Line Width"
      },
      "fillShape": {
        type: "boolean",
        label: "Fill Shape"
      }
    }
  },
  ellipse: {
    icon: "assets/ellipse.svg",
    properties: {
      "lineWidth": {
        type: "number",
        label: "Line Width"
      },
      "fillShape": {
        type: "boolean",
        label: "Fill Shape"
      }
    }
  },
  paint_bucket: {
    icon: "/assets/paint_bucket.svg",
    properties: {
      "fillGaps": {
        type: "number",
        label: "Fill Gaps",
        min: 1,
      }
    }
  }
}

let mouseEvent;

let context = {
  mouseDown: false,
  mousePos: {x: 0, y: 0},
  swatches: [
    "#000000",
    "#FFFFFF",
    "#FF0000",
    "#FFFF00",
    "#00FF00",
    "#00FFFF",
    "#0000FF",
    "#FF00FF",
  ],
  lineWidth: 5,
  simplifyMode: "smooth",
  fillShape: false,
  strokeShape: true,
  fillGaps: 5,
  dragging: false,
  selectionRect: undefined,
  selection: [],
  shapeselection: [],
  dragDirection: undefined,
  zoomLevel: 1,
}

let config = {
  shortcuts: {
    playAnimation: " ",
    // undo: "<ctrl>+z"
    undo: "<mod>z",
    redo: "<mod>Z",
    new: "<mod>n",
    save: "<mod>s",
    saveAs: "<mod>S",
    open: "<mod>o",
    import: "<mod>i",
    quit: "<mod>q",
    copy: "<mod>c",
    paste: "<mod>v",
    delete: "Backspace",
    selectAll: "<mod>a",
    group: "<mod>g",
    zoomIn: "<mod>+",
    zoomOut: "<mod>-",
  }
}

// Pointers to all objects
let pointerList = {}
// Keeping track of initial values of variables when we edit them continuously
let startProps = {}

let actions = {
  addShape: {
    create: (parent, shape, ctx) => {
      if (shape.curves.length==0) return;
      redoStack.length = 0; // Clear redo stack
      let serializableCurves = []
      for (let curve of shape.curves) {
        serializableCurves.push({ points: curve.points, color: curve.color })
      }
      let c = {
        ...context,
        ...ctx
      }
      let action = {
        parent: parent.idx,
        curves: serializableCurves,
        startx: shape.startx,
        starty: shape.starty,
        context: {
          fillShape: c.fillShape,
          strokeShape: c.strokeShape,
          fillStyle: c.fillStyle,
          sendToBack: c.sendToBack
        },
        uuid: uuidv4()
      }
      undoStack.push({name: "addShape", action: action})
      actions.addShape.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let object = pointerList[action.parent]
      let curvesList = action.curves
      let cxt = {
        ...context,
        ...action.context
      }
      let shape = new Shape(action.startx, action.starty, cxt, action.uuid)
      for (let curve of curvesList) {
        shape.addCurve(new Bezier(
          curve.points[0].x, curve.points[0].y,
          curve.points[1].x, curve.points[1].y,
          curve.points[2].x, curve.points[2].y,
          curve.points[3].x, curve.points[3].y
        ).setColor(curve.color))
      }
      let shapes = shape.update()
      for (let newShape of shapes) {
        object.addShape(newShape, cxt.sendToBack)
      }
    },
    rollback: (action) => {
      let object = pointerList[action.parent]
      let shape = pointerList[action.uuid]
      object.removeShape(shape)
      delete pointerList[action.uuid]
    }
  },
  editShape: {
    create: (shape, newCurves) => {
      redoStack.length = 0; // Clear redo stack
      let serializableNewCurves = []
      for (let curve of newCurves) {
        serializableNewCurves.push({ points: curve.points, color: curve.color })
      }
      let serializableOldCurves = []
      for (let curve of shape.curves) {
        serializableOldCurves.push({ points: curve.points })
      }
      let action = {
        shape: shape.idx,
        oldCurves: serializableOldCurves,
        newCurves: serializableNewCurves
      }
      undoStack.push({name: "editShape", action: action})
      actions.editShape.execute(action)

    },
    execute: (action) => {
      let shape = pointerList[action.shape]
      let curvesList = action.newCurves
      shape.curves = []
      for (let curve of curvesList) {
        shape.addCurve(new Bezier(
          curve.points[0].x, curve.points[0].y,
          curve.points[1].x, curve.points[1].y,
          curve.points[2].x, curve.points[2].y,
          curve.points[3].x, curve.points[3].y
        ).setColor(curve.color))
      }
      shape.update()
    },
    rollback: (action) => {
      let shape = pointerList[action.shape]
      let curvesList = action.oldCurves
      shape.curves = []
      for (let curve of curvesList) {
        shape.addCurve(new Bezier(
          curve.points[0].x, curve.points[0].y,
          curve.points[1].x, curve.points[1].y,
          curve.points[2].x, curve.points[2].y,
          curve.points[3].x, curve.points[3].y
        ).setColor(curve.color))
      }
      shape.update()
    }
  },
  colorShape: {
    create: (shape, color) => {
      redoStack.length = 0; // Clear redo stack
      let action = {
        shape: shape.idx,
        oldColor: shape.fillStyle,
        newColor: color
      }
      undoStack.push({name: "colorShape", action: action})
      actions.colorShape.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let shape = pointerList[action.shape]
      shape.fillStyle = action.newColor
    },
    rollback: (action) => {
      let shape = pointerList[action.shape]
      shape.fillStyle = action.oldColor
    }
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
        parent: parent.idx

      }
      undoStack.push({name: "addImageObject", action: action})
      actions.addImageObject.execute(action)
      updateMenu()
    },
    execute: async (action) => {
      let imageObject = new GraphicsObject(action.objectUuid)
      // let img = pointerList[action.img] 
      let img = new Image();
      function loadImage(src) {
        return new Promise((resolve, reject) => {
          let img = new Image();
          img.onload = () => resolve(img);  // Resolve the promise with the image once loaded
          img.onerror = (err) => reject(err);  // Reject the promise if there's an error loading the image
          img.src = src;  // Start loading the image
        });
      }
      img = await loadImage(action.src)
      console.log(img.crossOrigin)
      // img.onload = function() {
      let ct = {
        ...context,
        fillImage: img,
        strokeShape: false,
      }
      let imageShape = new Shape(0, 0, ct, action.shapeUuid)
      imageShape.addLine(img.width, 0)
      imageShape.addLine(img.width, img.height)
      imageShape.addLine(0, img.height)
      imageShape.addLine(0, 0)
      imageShape.update()
      imageShape.fillImage = img
      imageShape.filled = true
      imageObject.addShape(imageShape)
      let parent = pointerList[action.parent]
      parent.addObject(
        imageObject,
        action.x-img.width/2 + (20*action.ix),
        action.y-img.height/2 + (20*action.ix)
      )
      updateUI();
      // }
      // img.src = action.src
    },
    rollback: (action) => {
      let shape = pointerList[action.shapeUuid]
      let object = pointerList[action.objectUuid]
      let parent = pointerList[action.parent]
      object.removeShape(shape)
      delete pointerList[action.shapeUuid]
      parent.removeChild(object)
      delete pointerList[action.objectUuid]
      let selectIndex = context.selection.indexOf(object)
      if (selectIndex >= 0) {
        context.selection.splice(selectIndex, 1)
      }
    }
  },
  addAudio: {
    create: (audiosrc, object, audioname) => {
      redoStack.length = 0
      let action = {
        audiosrc:audiosrc,
        audioname: audioname,
        uuid: uuidv4(),
        layeruuid: uuidv4(),
        frameNum: object.currentFrameNum,
        object: object.idx
      }
      undoStack.push({name: 'addAudio', action: action})
      actions.addAudio.execute(action)
      updateMenu()
    },
    execute: async (action) => {
      const player = new Tone.Player().toDestination();
      await player.load(action.audiosrc)
      // player.autostart = true;
      let newAudioLayer = new AudioLayer(action.layeruuid, action.audioname)
      let object = pointerList[action.object]
      const img = new Image();
      img.className = "audioWaveform"
      let soundObj = {
        player: player,
        start: action.frameNum,
        img: img
      }
      pointerList[action.uuid] = soundObj
      newAudioLayer.sounds[action.uuid] = soundObj
      // TODO: change start time
      newAudioLayer.track.add(0,action.uuid)
      object.audioLayers.push(newAudioLayer)
      // TODO: compute image height better
      generateWaveform(img, player.buffer, 50, 25, fileFps)
      updateLayers()
    },
    rollback: (action) => {
      let object = pointerList[action.object]
      let layer = pointerList[action.layeruuid]
      object.audioLayers.splice(object.audioLayers.indexOf(layer),1)
      updateLayers()
    }
  },
  duplicateObject: {
    create: (object) => {
      redoStack.length = 0
      let action = {
        object: object.idx,
        uuid: uuidv4()
      }
      undoStack.push({name: 'duplicateObject', action: action})
      actions.duplicateObject.execute(action)
      updateMenu()
    },
    execute: (action) => {
      // your code here
      let object = pointerList[action.object]
      let newObj = object.copy(action.uuid)
      // newObj.idx = action.uuid
      context.activeObject.addObject(newObj)
      updateUI()
    },
    rollback: (action) => {
      let object = pointerList[action.uuid]
      context.activeObject.removeChild(object)
      updateUI()
    }
  },
  deleteObjects: {
    create: (objects, shapes) => {
      redoStack.length = 0
      let serializableObjects = []
      let serializableShapes = []
      for (let object of objects) {
        serializableObjects.push(object.idx)
      }
      for (let shape of shapes) {
        serializableShapes.push(shape.idx)
      }
      let action = {
        objects: serializableObjects,
        shapes: serializableShapes,
        frame: context.activeObject.currentFrame.idx,
        oldState: structuredClone(context.activeObject.currentFrame.keys)
      }
      undoStack.push({name: 'deleteObjects', action: action})
      actions.deleteObjects.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let frame = pointerList[action.frame]
      for (let object of action.objects) {
        delete frame.keys[object]
      }
      for (let shape of action.shapes) {
        frame.shapes.splice(frame.shapes.indexOf(pointerList[shape]),1)
      }
      updateUI()
    },
    rollback: (action) => {
      let frame = pointerList[action.frame]
      for (let object of action.objects) {
        frame.keys[object] = action.oldState[object]
      }
      for (let shape of action.shapes) {
        frame.shapes.push(pointerList[shape])
      }
      updateUI()
    }
  },
  addLayer: {
    create: () => {
      redoStack.length = 0
      let action = {
        object: context.activeObject.idx,
        uuid: uuidv4()
      }
      undoStack.push({name: 'addLayer', action: action})
      actions.addLayer.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let object = pointerList[action.object]
      let layer = new Layer(action.uuid)
      layer.name = `Layer ${object.layers.length + 1}`
      object.layers.push(layer)
      object.currentLayer = object.layers.indexOf(layer)
      updateLayers()
    },
    rollback: (action) => {
      let object = pointerList[action.object]
      let layer = pointerList[action.uuid]
      object.layers.splice(object.layers.indexOf(layer),1)
      updateLayers()
    }
  },
  deleteLayer: {
    create: (layer) => {
      redoStack.length = 0
      // Don't allow deleting the only layer
      if (context.activeObject.layers.length==1) return;
      if (!(layer instanceof Layer)) {
        layer = context.activeObject.activeLayer
      }
      let action = {
        object: context.activeObject.idx,
        layer: layer.idx,
        index: context.activeObject.layers.indexOf(layer)
      }
      undoStack.push({name: 'deleteLayer', action: action})
      actions.deleteLayer.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let object = pointerList[action.object]
      let layer = pointerList[action.layer]
      let changelayer = false
      if (object.activeLayer == layer) {
        changelayer = true
      }
      object.layers.splice(object.layers.indexOf(layer),1)
      if (changelayer) {
        object.currentLayer = 0
      }
      updateUI()
      updateLayers()
    },
    rollback: (action) => {
      let object = pointerList[action.object]
      let layer = pointerList[action.layer]
      object.layers.splice(action.index,0,layer)
      updateUI( )
      updateLayers()
    }
  },
  changeLayerName: {
    create: (layer, newName) => {
      redoStack.length = 0
      let action = {
        layer: layer.idx,
        newName: newName,
        oldName: layer.name
      }
      undoStack.push({name: 'changeLayerName', action: action})
      actions.changeLayerName.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let layer = pointerList[action.layer]
      layer.name = action.newName
      updateLayers()
    },
    rollback: (action) => {
      let layer = pointerList[action.layer]
      layer.name = action.oldName
      updateLayers()
    }
  },
  editFrame: {
    create: (frame) => {
      redoStack.length = 0; // Clear redo stack
      let action = {    
        newState: structuredClone(frame.keys),
        oldState: startProps[frame.idx],
        frame: frame.idx
      }
      undoStack.push({name: "editFrame", action: action})
      actions.editFrame.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let frame = pointerList[action.frame]
      frame.keys = structuredClone(action.newState)
      console.log(structuredClone(frame.keys))
      console.log(frame.keys)
      updateUI()
    },
    rollback: (action) => {
      let frame = pointerList[action.frame]
      frame.keys = structuredClone(action.oldState)
      updateUI()
    }
  },
  addFrame: {
    create: () => {
      redoStack.length = 0
      let frames = []
      for (let i=context.activeObject.activeLayer.frames.length; i<=context.activeObject.currentFrameNum; i++) {
        frames.push(uuidv4())
      }
      let action = {
        frames: frames,
        layer: context.activeObject.activeLayer.idx
      }
      undoStack.push({name: 'addFrame', action: action})
      actions.addFrame.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let layer = pointerList[action.layer]
      for (let frame of action.frames) {
        layer.frames.push(new Frame("normal", frame))
      }
      updateLayers()
    },
    rollback: (action) => {
      let layer = pointerList[action.layer]
      for (let _frame of action.frames) {
        layer.frames.pop()
      }
      updateLayers()
    }
  },
  addKeyframe: {
    create: () => {
      let frameNum = context.activeObject.currentFrameNum
      let layer = context.activeObject.activeLayer
      let formerType;
      let addedFrames = {};
      if (frameNum >= layer.frames.length) {
        formerType = "none"
        for (let i=layer.frames.length; i<=frameNum; i++) {
          addedFrames[i] = uuidv4()
        }
      } else if (layer.frames[frameNum].frameType != "keyframe") {
        formerType = layer.frames[frameNum].frameType
      } else {
        return // Already a keyframe, nothing to do
      }
      redoStack.length = 0
      let action = {
        frameNum: frameNum,
        object: context.activeObject.idx,
        layer: layer.idx,
        formerType: formerType,
        addedFrames: addedFrames,
        uuid: uuidv4()
      }
      undoStack.push({name: 'addKeyframe', action: action})
      actions.addKeyframe.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let object = pointerList[action.object]
      let layer = pointerList[action.layer]
      console.log(pointerList)
      console.log(action.object)
      console.log(object)
      let latestFrame = object.getFrame(Math.max(action.frameNum-1, 0))
      let newKeyframe = new Frame("keyframe", action.uuid)
      for (let key in latestFrame.keys) {
        newKeyframe.keys[key] = structuredClone(latestFrame.keys[key])
      }
      for (let shape of latestFrame.shapes) {
        newKeyframe.shapes.push(shape.copy(action.uuid))
      }
      if (action.frameNum >= layer.frames.length) {
        for (const [index, idx] of Object.entries(action.addedFrames)) {
          layer.frames[index] = new Frame("normal", idx)
        }
      }
      //   layer.frames.push(newKeyframe)
      // } else if (layer.frames[action.frameNum].frameType != "keyframe") {
        layer.frames[action.frameNum] = newKeyframe
      // }
      updateLayers()
    },
    rollback: (action) => {
      let layer = pointerList[action.layer]
      if (action.formerType == "none") {
        for (let i in action.addedFrames) {
          layer.frames.pop()
        }
      } else {
        let layer = pointerList[action.layer]
        layer.frames[action.frameNum].frameType = action.formerType
      }
      updateLayers()
    }
  },
  addMotionTween: {
    create: () => {
      redoStack.length = 0
      let frameNum = context.activeObject.currentFrameNum
      let layer = context.activeObject.activeLayer
      let frames = layer.frames
      let {lastKeyframeBefore, firstKeyframeAfter} = getKeyframesSurrounding(frames, frameNum)
      
      let action = {
        frameNum: frameNum,
        layer: layer.idx,
        lastBefore: lastKeyframeBefore,
        firstAfter: firstKeyframeAfter,
      }
      undoStack.push({name: 'addMotionTween', action: action})
      actions.addMotionTween.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let layer = pointerList[action.layer]
      let frames = layer.frames
      if ((action.lastBefore != undefined) && (action.firstAfter != undefined)) {
        for (let i=action.lastBefore + 1; i<action.firstAfter; i++) {
          frames[i].frameType = "motion"
          frames[i].prev = frames[action.lastBefore]
          frames[i].next = frames[action.firstAfter]
          frames[i].prevIndex = action.lastBefore
          frames[i].nextIndex = action.firstAfter
        }
      }
      updateLayers()
      updateUI()
    },
    rollback: (action) => {
      let layer = pointerList[action.layer]
      let frames = layer.frames
      for (let i=action.lastBefore + 1; i<action.firstAfter; i++) {
        frames[i].frameType = "normal"
      }
      updateLayers()
      updateUI()
    }
  },
  addShapeTween: {
    create: () => {
      redoStack.length = 0
      let frameNum = context.activeObject.currentFrameNum
      let layer = context.activeObject.activeLayer
      let frames = layer.frames
      let {lastKeyframeBefore, firstKeyframeAfter} = getKeyframesSurrounding(frames, frameNum)
      
      let action = {
        frameNum: frameNum,
        layer: layer.idx,
        lastBefore: lastKeyframeBefore,
        firstAfter: firstKeyframeAfter,
      }
      undoStack.push({name: 'addShapeTween', action: action})
      actions.addShapeTween.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let layer = pointerList[action.layer]
      let frames = layer.frames
      if ((action.lastBefore != undefined) && (action.firstAfter != undefined)) {
        for (let i=action.lastBefore + 1; i<action.firstAfter; i++) {
          frames[i].frameType = "shape"
          frames[i].prev = frames[action.lastBefore]
          frames[i].next = frames[action.firstAfter]
          frames[i].prevIndex = action.lastBefore
          frames[i].nextIndex = action.firstAfter
        }
      }
      updateLayers()
      updateUI()
    },
    rollback: (action) => {
      let layer = pointerList[action.layer]
      let frames = layer.frames
      for (let i=action.lastBefore + 1; i<action.firstAfter; i++) {
        frames[i].frameType = "normal"
      }
      updateLayers()
      updateUI()
    }
  },
  group: {
    create: () => {
      redoStack.length = 0
      let serializableShapes = []
      let serializableObjects = []
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx)
      }
      for (let object of context.selection) {
        serializableObjects.push(object.idx)
      }
      context.shapeselection = []
      context.selection = []
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        groupUuid: uuidv4(),
        parent: context.activeObject.idx
      }
      undoStack.push({name: 'group', action: action})
      actions.group.execute(action)
      updateMenu()
    },
    execute: (action) => {
      // your code here
      let group = new GraphicsObject(action.groupUuid)
      let parent = pointerList[action.parent]
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx]
        group.addShape(shape)
        parent.removeShape(shape)
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx]
        group.addObject(object, object.x, object.y)
        parent.removeChild(object)
      }
      parent.addObject(group)
      if (context.activeObject==parent && context.selection.length==0 && context.shapeselection.length==0) {
        context.selection.push(group)
      }
      updateUI()
      updateInfopanel()
    },
    rollback: (action) => {
      let group = pointerList[action.groupUuid]
      let parent = pointerList[action.parent]
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx]
        parent.addShape(shape)
        group.removeShape(shape)
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx]
        parent.addObject(object, object.x, object.y)
        group.removeChild(object)
      }
      parent.removeChild(group)
      updateUI()
      updateInfopanel()
    }
  },
  sendToBack: {
    create: () => {
      redoStack.length = 0
      let serializableShapes = []
      let serializableObjects = []
      let formerIndices = {}
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx)
        formerIndices[shape.idx] = context.activeObject.currentFrame.shapes.indexOf(shape)
      }
      for (let object of context.selection) {
        serializableObjects.push(object.idx)
        formerIndices[object.idx] = context.activeObject.activeLayer.children.indexOf(object)
      }
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        layer: context.activeObject.activeLayer.idx,
        frame: context.activeObject.currentFrame.idx,
        formerIndices: formerIndices
      }
      undoStack.push({name: 'sendToBack', action: action})
      actions.sendToBack.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let frame = pointerList[action.frame]
      let layer = pointerList[action.layer]
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx]
        frame.shapes.splice(frame.shapes.indexOf(shape),1)
        frame.shapes.unshift(shape)
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx]
        layer.children.splice(layer.children.indexOf(object),1)
        layer.children.unshift(object)
      }
      updateUI()
    },
    rollback: (action) => {
      let frame = pointerList[action.frame]
      let layer = pointerList[action.layer]
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx]
        frame.shapes.splice(frame.shapes.indexOf(shape),1)
        frame.shapes.splice(action.formerIndices[shapeIdx], 0, shape)
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx]
        layer.children.splice(layer.children.indexOf(object),1)
        layer.children.splice(action.formerIndices[objectIdx], 0, object  )
      }
      updateUI()
    }
  },
  bringToFront: {
    create: () => {
      redoStack.length = 0
      let serializableShapes = []
      let serializableObjects = []
      let formerIndices = {}
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx)
        formerIndices[shape.idx] = context.activeObject.currentFrame.shapes.indexOf(shape)
      }
      for (let object of context.selection) {
        serializableObjects.push(object.idx)
        formerIndices[object.idx] = context.activeObject.activeLayer.children.indexOf(object)
      }
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        layer: context.activeObject.activeLayer.idx,
        frame: context.activeObject.currentFrame.idx,
        formerIndices: formerIndices
      }
      undoStack.push({name: 'bringToFront', action: action})
      actions.bringToFront.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let frame = pointerList[action.frame]
      let layer = pointerList[action.layer]
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx]
        frame.shapes.splice(frame.shapes.indexOf(shape),1)
        frame.shapes.push(shape)
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx]
        layer.children.splice(layer.children.indexOf(object),1)
        layer.children.push(object)
      }
      updateUI()
    },
    rollback: (action) => {
      let frame = pointerList[action.frame]
      let layer = pointerList[action.layer]
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx]
        frame.shapes.splice(frame.shapes.indexOf(shape),1)
        frame.shapes.splice(action.formerIndices[shapeIdx], 0, shape)
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx]
        layer.children.splice(layer.children.indexOf(object),1)
        layer.children.splice(action.formerIndices[objectIdx], 0, object  )
      }
      updateUI()
    }
  },
  setName: {
    create: (object, name) => {
      redoStack.length = 0
      let action = {
        object: object.idx,
        newName: name,
        oldName: object.name
      }
      undoStack.push({name: 'setName', action: action})
      actions.setName.execute(action)
      updateMenu()
    },
    execute: (action) => {
      let object = pointerList[action.object]
      object.name = action.newName
      updateInfopanel()
    },
    rollback: (action) => {
      let object = pointerList[action.object]
      object.name = action.oldName
      updateInfopanel()
    }
  },
}

function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, c =>
    (+c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> +c / 4).toString(16)
  );
}
function vectorDist(a, b) {
  return Math.sqrt((a.x-b.x)*(a.x-b.x) + (a.y-b.y)*(a.y-b.y))
}

function getMousePos(canvas, evt) {
  var rect = canvas.getBoundingClientRect();
  return {
    x: (evt.clientX - rect.left) / context.zoomLevel,
    y: (evt.clientY - rect.top) / context.zoomLevel
  };
}

function getProperty(context, path) {
  let pointer = context;
  let pathComponents = path.split('.')
  for (let component of pathComponents) {
    pointer = pointer[component]
  }
  return pointer
}

function setProperty(context, path, value) {
  let pointer = context;
  let pathComponents = path.split('.')
  let finalComponent = pathComponents.pop()
  for (let component of pathComponents) {
    pointer = pointer[component]
  }
  pointer[finalComponent] = value
}

function selectCurve(context, mouse) {
  let mouseTolerance = 15;
  let closestDist = mouseTolerance;
  let closestCurve = undefined
  let closestShape = undefined
  for (let shape of context.activeObject.currentFrame.shapes) {
    if (shape instanceof TempShape) continue;
    if (mouse.x > shape.boundingBox.x.min - mouseTolerance &&
        mouse.x < shape.boundingBox.x.max + mouseTolerance &&
        mouse.y > shape.boundingBox.y.min - mouseTolerance &&
        mouse.y < shape.boundingBox.y.max + mouseTolerance) {
      for (let curve of shape.curves) {
        let dist = vectorDist(mouse, curve.project(mouse))
        if (dist <= closestDist ) {
          closestDist = dist
          closestCurve = curve
          closestShape = shape
        }
      }
      }
    }
      if (closestCurve) {
        return {curve:closestCurve, shape:closestShape}
      } else {
        return undefined
  }
}
function selectVertex(context, mouse) {
  let mouseTolerance = 15;
  let closestDist = mouseTolerance;
  let closestVertex = undefined
  let closestShape = undefined
  for (let shape of context.activeObject.currentFrame.shapes) {
    if (shape instanceof TempShape) continue;
    if (mouse.x > shape.boundingBox.x.min - mouseTolerance &&
        mouse.x < shape.boundingBox.x.max + mouseTolerance &&
        mouse.y > shape.boundingBox.y.min - mouseTolerance &&
        mouse.y < shape.boundingBox.y.max + mouseTolerance) {
      for (let vertex of shape.vertices) {
        let dist = vectorDist(mouse, vertex.point)
        if (dist <= closestDist ) {
          closestDist = dist
          closestVertex = vertex
          closestShape = shape
        }
      }
      }
    }
      if (closestVertex) {
        return {vertex:closestVertex, shape:closestShape}
      } else {
        return undefined
  }
}

function moldCurve(curve, mouse, oldmouse) {
  let diff = {x: mouse.x - oldmouse.x, y: mouse.y - oldmouse.y}
  let p = curve.project(mouse)
  let min_influence = 0.1
  const CP1 = {
    x: curve.points[1].x + diff.x*(1-p.t)*2,
    y: curve.points[1].y + diff.y*(1-p.t)*2
  }
  const CP2 = {
    x: curve.points[2].x + diff.x*(p.t)*2,
    y: curve.points[2].y + diff.y*(p.t)*2
  }
  return new Bezier(curve.points[0], CP1, CP2, curve.points[3])
  // return curve
}


function deriveControlPoints(S, A, E, e1, e2, t) {
  // Deriving the control points is effectively "doing what
  // we talk about in the section", in code:

  const v1 = {
      x: A.x - (A.x - e1.x)/(1-t),
      y: A.y - (A.y - e1.y)/(1-t)
  };
  const v2 = {
      x: A.x - (A.x - e2.x)/t,
      y: A.y - (A.y - e2.y)/t
  };

  const C1 = {
      x: S.x + (v1.x - S.x) / t,
      y: S.y + (v1.y - S.y) / t
  };
  const C2 = {
      x: E.x + (v2.x - E.x) / (1-t),
      y: E.y + (v2.y - E.y) / (1-t)
  };

  return {v1, v2, C1, C2};
}


function growBoundingBox(bboxa, bboxb) {
  bboxa.x.min = Math.min(bboxa.x.min, bboxb.x.min)
  bboxa.y.min = Math.min(bboxa.y.min, bboxb.y.min)
  bboxa.x.max = Math.max(bboxa.x.max, bboxb.x.max)
  bboxa.y.max = Math.max(bboxa.y.max, bboxb.y.max)
}

function regionToBbox(region) {
  return {
    x: {min: Math.min(region.x1, region.x2), max: Math.max(region.x1, region.x2)},
    y: {min: Math.min(region.y1, region.y2), max: Math.max(region.y1, region.y2)}
  }
}

function hitTest(candidate, object) {
  let bbox = object.bbox()
  if (candidate.x.min) {
    // We're checking a bounding box
    if (candidate.x.min < bbox.x.max && candidate.x.max > bbox.x.min &&
      candidate.y.min < bbox.y.max && candidate.y.max > bbox.y.min) {
        return true;
    } else {
      return false;
    }
  } else {
    // We're checking a point
    if (candidate.x > bbox.x.min &&
      candidate.x < bbox.x.max &&
      candidate.y > bbox.y.min &&
      candidate.y < bbox.y.max) {
        return true;
    } else {
      return false
    }
  }

}

function undo() {
  let action = undoStack.pop()
  if (action) {
    actions[action.name].rollback(action.action)
    redoStack.push(action)
    updateUI()
    updateMenu()
  } else {
    console.log("No actions to undo")
    updateMenu()
  }
}

function redo() {
  let action = redoStack.pop()
  if (action) {
    actions[action.name].execute(action.action)
    undoStack.push(action)
    updateUI()
    updateMenu()
  } else {
    console.log("No actions to redo")
    updateMenu()
  }
}


class Frame {
  constructor(frameType="normal", uuid=undefined) {
    this.keys = {}
    this.shapes = []
    this.frameType = frameType
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    pointerList[this.idx] = this
  }
  saveState() {
    startProps[this.idx] = structuredClone(this.keys)
  }
  copy(idx) {
    let newFrame = new Frame(this.frameType, idx.slice(0,8)+this.idx.slice(8))
    newFrame.keys = structuredClone(this.keys)
    newFrame.shapes = []
    for (let shape of this.shapes) {
      newFrame.shapes.push(shape.copy(idx))
    }
    return newFrame
  }
}

class Layer {
  constructor(uuid) {         
    this.children = []
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    this.name = "Layer"
    this.frames = [new Frame("keyframe", this.idx+"-F1")]
    this.visible = true
    pointerList[this.idx] = this
  }
  getFrame(num) {
    if (this.frames[num]) {
      if (this.frames[num].frameType == "keyframe") {
        return this.frames[num]
      } else if (this.frames[num].frameType == "motion") {
        let frameKeys = {}
        let prevFrame = this.frames[num].prev
        let nextFrame = this.frames[num].next
        const t = (num - this.frames[num].prevIndex) / (this.frames[num].nextIndex - this.frames[num].prevIndex);
        for (let key in prevFrame.keys) {
          frameKeys[key] = {}
          let prevKeyDict = prevFrame.keys[key]
          let nextKeyDict = nextFrame.keys[key]
          for (let prop in prevKeyDict) {
            frameKeys[key][prop] = (1 - t) * prevKeyDict[prop] + t * nextKeyDict[prop];
          }

        }
        let frame = new Frame("motion", "temp")
        frame.keys = frameKeys
        return frame
      } else if (this.frames[num].frameType == "shape") {
        let prevFrame = this.frames[num].prev
        let nextFrame = this.frames[num].next
        const t = (num - this.frames[num].prevIndex) / (this.frames[num].nextIndex - this.frames[num].prevIndex);
        let shapes = []
        for (let shape1 of prevFrame.shapes) {
          if (shape1.curves.length == 0) continue;
          let shape2 = undefined
          for (let i of nextFrame.shapes) {
            if (shape1.shapeId == i.shapeId) {
              shape2 = i
            }
          }
          if (shape2 != undefined) {
            let path1 = [{type: "M", x:shape1.curves[0].points[0].x, y:shape1.curves[0].points[0].y}]
            for (let curve of shape1.curves) {
              path1.push({type:"C", x1:curve.points[1].x, y1:curve.points[1].y, 
                x2: curve.points[2].x, y2: curve.points[2].y,
                x: curve.points[3].x, y:curve.points[3].y
              })
            }
            let path2 = []
            if (shape2.curves.length > 0) {
              path2.push({type: "M", x:shape2.curves[0].points[0].x, y:shape2.curves[0].points[0].y})
              for (let curve of shape2.curves) {
                path2.push({type:"C", x1:curve.points[1].x, y1:curve.points[1].y, 
                  x2: curve.points[2].x, y2: curve.points[2].y,
                  x: curve.points[3].x, y:curve.points[3].y
                })
              }
            }
            const interpolator = d3.interpolatePathCommands(path1, path2)
            let current = interpolator(t)
            let curves = []
            let start = current.shift()
            let {x, y} = start
            for (let curve of current) {
              curves.push(new Bezier(x, y, curve.x1, curve.y1, curve.x2, curve.y2, curve.x, curve.y))
              x = curve.x
              y = curve.y
            }
            let lineWidth = lerp(shape1.lineWidth, shape2.lineWidth, t)
            let strokeStyle = lerpColor(shape1.strokeStyle, shape2.strokeStyle, t)
            let fillStyle;
            if (!shape1.fillImage) {
              fillStyle = lerpColor(shape1.fillStyle, shape2.fillStyle, t)
            }
            shapes.push(new TempShape(
              start.x, start.y, curves, shape1.lineWidth,
              shape1.stroked, shape1.filled, strokeStyle, fillStyle
            ))
          }
        }
        let frame = new Frame("shape", "temp")
        frame.shapes = shapes
        return frame
      } else {
        for (let i=Math.min(num, this.frames.length-1); i>=0; i--) {
          if (this.frames[i].frameType == "keyframe") {
            return this.frames[i]
          }
        }
      }
    } else {
      for (let i=Math.min(num, this.frames.length-1); i>=0; i--) {
        if (this.frames[i].frameType == "keyframe") {
          return this.frames[i]
        }
      }
    }
  }
  copy(idx) {
    let newLayer = new Layer(idx.slice(0,8)+this.idx.slice(8))
    let idxMapping = {}
    for (let child of this.children) {
      let newChild = child.copy(idx)
      idxMapping[child.idx] = newChild.idx
      newLayer.children.push(newChild)
    }
    newLayer.frames = []
    for (let frame of this.frames) {
      let newFrame = frame.copy(idx)
      newFrame.keys = {}
      for (let key in frame.keys) {
        newFrame.keys[idxMapping[key]] = structuredClone(frame.keys[key])
      }
      newLayer.frames.push(newFrame)
    }
    return newLayer
  }
  toggleVisibility() {
    this.visible = !this.visible
    updateUI()
    updateMenu()
    updateLayers()
  }
}

class AudioLayer {
  constructor(uuid, name) {
    this.sounds = {}
      this.track = new Tone.Part(((time, sound) => {
        console.log(this.sounds[sound])
        this.sounds[sound].player.start(time)
      }))
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    if (!name) {
      this.name = "Audio"
    } else {
      this.name = name
    }
  }
  copy(idx) {
    let newAudioLayer = new AudioLayer(idx.slice(0,8)+this.idx.slice(8), this.name)
    for (let soundIdx in this.sounds) {
      let sound = this.sounds[soundIdx]
      let newPlayer = new Tone.Player(sound.buffer()).toDestination()
      let idx = this.idx.slice(0,8)+soundIdx.slice(8)
      let soundObj = {
        player: newPlayer,
        start: sound.start
      }
      pointerList[idx] = soundObj
      newAudioLayer.sounds[idx] = soundObj
    }
  }
}

class BaseShape {
  constructor(startx, starty) {
    this.startx = startx
    this.starty = starty
    this.curves = []
    this.regions = [];
    this.boundingBox = {
      x: {min: startx, max: starty},
      y: {min: starty, max: starty}
    }
  }
  recalculateBoundingBox() {
    for (let curve of this.curves) {
      growBoundingBox(this.boundingBox, curve.bbox())
    }
  }
  draw(context) {
    let ctx = context.ctx;
    ctx.lineWidth = this.lineWidth
    ctx.lineCap = "round"
    // for (let region of this.regions) {
    //   // if (region.filled) continue;
    //   if ((region.fillStyle || region.fillImage) && region.filled) {
    //     // ctx.fillStyle = region.fill
    //     if (region.fillImage) {
    //       let pat = ctx.createPattern(region.fillImage, "no-repeat")
    //       ctx.fillStyle = pat
    //     } else {
    //       ctx.fillStyle = region.fillStyle
    //     }
    //     ctx.beginPath()
    //     for (let curve of region.curves) {
    //       ctx.lineTo(curve.points[0].x, curve.points[0].y)
    //       ctx.bezierCurveTo(curve.points[1].x, curve.points[1].y,
    //                         curve.points[2].x, curve.points[2].y,
    //                         curve.points[3].x, curve.points[3].y)
    //     }
    //     ctx.fill()
    //   }
    // }
    if (this.filled) {
      ctx.beginPath()
      if (this.fillImage) {
        let pat = ctx.createPattern(this.fillImage, "no-repeat")
        ctx.fillStyle = pat
      } else {
        ctx.fillStyle = this.fillStyle
      }
      if (context.debugColor) {
        ctx.fillStyle = context.debugColor
      }
      if (this.curves.length > 0) {
        ctx.moveTo(this.curves[0].points[0].x, this.curves[0].points[0].y)
        for (let curve of this.curves) {
          ctx.bezierCurveTo(curve.points[1].x, curve.points[1].y,
                            curve.points[2].x, curve.points[2].y,
                            curve.points[3].x, curve.points[3].y)
        }
      }
      ctx.fill()
    }
    if (this.stroked && !context.debugColor) {
      for (let curve of this.curves) {
        ctx.strokeStyle = curve.color
        ctx.beginPath()
        ctx.moveTo(curve.points[0].x, curve.points[0].y)
        ctx.bezierCurveTo(curve.points[1].x, curve.points[1].y,
                          curve.points[2].x, curve.points[2].y,
                          curve.points[3].x, curve.points[3].y)
        ctx.stroke()

        // // Debug, show curve control points
        // ctx.beginPath()
        // ctx.arc(curve.points[1].x,curve.points[1].y, 5, 0, 2*Math.PI)
        // ctx.arc(curve.points[2].x,curve.points[2].y, 5, 0, 2*Math.PI)
        // ctx.arc(curve.points[3].x,curve.points[3].y, 5, 0, 2*Math.PI)
        // ctx.fill()
      }
    }
    // Debug, show quadtree
    if (debugQuadtree && this.quadtree && !context.debugColor) {
      this.quadtree.draw(ctx)
    }

  }
}

class TempShape extends BaseShape {
  constructor(startx, starty, curves, lineWidth, stroked, filled, strokeStyle, fillStyle) {
    super(startx, starty)
    this.curves = curves
    this.lineWidth = lineWidth
    this.stroked = stroked
    this.filled = filled
    this.strokeStyle = strokeStyle
    this.fillStyle = fillStyle
    this.inProgress = false
    this.recalculateBoundingBox()
  }
}

class Shape extends BaseShape {
  constructor(startx, starty, context, uuid=undefined, shapeId=undefined) {
    super(startx, starty)
    this.vertices = [];
    this.triangles = [];
    this.fillStyle = context.fillStyle;
    this.fillImage = context.fillImage;
    this.strokeStyle = context.strokeStyle;
    this.lineWidth = context.lineWidth
    this.filled = context.fillShape;
    this.stroked = context.strokeShape;
    this.quadtree = new Quadtree({x: {min: 0, max: 500}, y: {min: 0, max: 500}}, 4)
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    if (!shapeId) {
      this.shapeId = uuidv4()
    } else {
      this.shapeId = shapeId
    }
    pointerList[this.idx] = this
    this.regionIdx = 0;
    this.inProgress = true
  }
  addCurve(curve) {
    if (curve.color == undefined) {
      curve.color = context.strokeStyle
    }
    this.curves.push(curve)
    this.quadtree.insert(curve, this.curves.length - 1)
    growBoundingBox(this.boundingBox, curve.bbox())
  }
  addLine(x, y) {
    let lastpoint;
    if (this.curves.length) {
      lastpoint = this.curves[this.curves.length - 1].points[3]
    } else {
      lastpoint = {x: this.startx, y: this.starty}
    }
    let midpoint = {x: (x + lastpoint.x) / 2, y: (y + lastpoint.y) / 2}
    let curve = new Bezier(lastpoint.x, lastpoint.y,
                           midpoint.x, midpoint.y,
                           midpoint.x, midpoint.y,
                           x, y)
    curve.color = context.strokeStyle
    this.quadtree.insert(curve, this.curves.length - 1)
    this.curves.push(curve)
  }
  bbox() {
    return this.boundingBox
  }
  clear() {
    this.curves = []
    this.quadtree.clear()
  }
  copy(idx) {
    let newShape = new Shape(this.startx, this.starty, {}, idx.slice(0,8)+this.idx.slice(8), this.shapeId)
    newShape.startx = this.startx;
    newShape.starty = this.starty;
    for (let curve of this.curves) {
      let newCurve = new Bezier(
        curve.points[0].x, curve.points[0].y,
        curve.points[1].x, curve.points[1].y,
        curve.points[2].x, curve.points[2].y,
        curve.points[3].x, curve.points[3].y,
      )
      newCurve.color = curve.color
      newShape.addCurve(newCurve)
    }
    // TODO
    // for (let vertex of this.vertices) {

    // }
    newShape.updateVertices()
    newShape.fillStyle = this.fillStyle;
    newShape.fillImage = this.fillImage;
    newShape.strokeStyle = this.strokeStyle;
    newShape.lineWidth = this.lineWidth
    newShape.filled = this.filled;
    newShape.stroked = this.stroked;

    return newShape
  }
  fromPoints(points, error=30) {
    console.log(error)
    this.curves = []
    let curves = fitCurve.fitCurve(points, error)
    for (let curve of curves) {
      let bezier = new Bezier(curve[0][0], curve[0][1],
                              curve[1][0],curve[1][1],
                              curve[2][0], curve[2][1],
                              curve[3][0], curve[3][1])
      this.curves.push(bezier)
      this.quadtree.insert(bezier, this.curves.length - 1)
    }
    return this
  }
  simplify(mode="corners") {
    this.quadtree.clear()
    this.inProgress = false
    // Mode can be corners, smooth or auto
    if (mode=="corners") {
      let points = [{x: this.startx, y: this.starty}]
      for (let curve of this.curves) {
        points.push(curve.points[3])
      }
      // points = points.concat(this.curves)
      let newpoints = simplifyPolyline(points, 10, false)
      this.curves = []
      let lastpoint = newpoints.shift()
      let midpoint
      for (let point of newpoints) {
        midpoint = {x: (lastpoint.x+point.x)/2, y: (lastpoint.y+point.y)/2}
        let bezier = new Bezier(lastpoint.x, lastpoint.y,
                                midpoint.x, midpoint.y,
                                midpoint.x,midpoint.y,
                                point.x,point.y)
        this.curves.push(bezier)
        this.quadtree.insert(bezier, this.curves.length - 1)
        lastpoint = point
      }
    } else if (mode=="smooth") {
      let error = 30;
      let points = [[this.startx, this.starty]]
      for (let curve of this.curves) {
        points.push([curve.points[3].x, curve.points[3].y])
      }
      this.fromPoints(points, error)
    }
    let epsilon = 0.01
    let newCurves = []
    let intersectMap = {}
    for (let i=0; i<this.curves.length-1; i++) {
      // for (let j=i+1; j<this.curves.length; j++) {
      for (let j of this.quadtree.query(this.curves[i].bbox())) {
        if (i >= j) continue;
        let intersects = this.curves[i].intersects(this.curves[j])
        if (intersects.length) {
          intersectMap[i] ||= []
          intersectMap[j] ||= []
          for(let intersect of intersects) {
            let [t1, t2] = intersect.split("/")
            intersectMap[i].push(parseFloat(t1))
            intersectMap[j].push(parseFloat(t2))
          }
        }
      }
    }
    for (let lst in intersectMap) {
      for (let i=1; i<intersectMap[lst].length; i++) {
        if (Math.abs(intersectMap[lst][i] - intersectMap[lst][i-1]) < epsilon) {
          intersectMap[lst].splice(i,1)
          i--
        }
      }
    }
    for (let i=this.curves.length-1; i>=0; i--) {
      if (i in intersectMap) {
        intersectMap[i].sort().reverse()
        let remainingFraction = 1
        let remainingCurve = this.curves[i]
        for (let t of intersectMap[i]) {
          let split = remainingCurve.split(t / remainingFraction)
          remainingFraction = t
          newCurves.push(split.right)
          remainingCurve = split.left
        }
        newCurves.push(remainingCurve)

      } else {
        newCurves.push(this.curves[i])
      }
    }
    for (let curve of newCurves) {
      curve.color = context.strokeStyle
    }
    newCurves.reverse()
    this.curves = newCurves
  }
  update() {
    this.recalculateBoundingBox()
    this.updateVertices()
    if (this.curves.length) {
      this.startx = this.curves[0].points[0].x
      this.starty = this.curves[0].points[0].y
    }
    return [this]
  }
  getClockwiseCurves(point, otherPoints) {
    // Returns array of {x, y, idx, angle}

    let points = []
    for (let point of otherPoints) {
      points.push({...this.vertices[point].point, idx: point})
    }
    // Add an angle property to each point using tan(angle) = y/x
    const angles = points.map(({ x, y, idx }) => {
      return { x, y, idx, angle: Math.atan2(y - point.y, x - point.x) * 180 / Math.PI };
    });
    // Sort your points by angle
    const pointsSorted = angles.sort((a, b) => a.angle - b.angle);
    return pointsSorted
  }
  updateVertices() {
    this.vertices = []
    let utils = Bezier.getUtils()
    let epsilon = 1.5 // big epsilon whoa
    let tooClose;
    let i = 0;


    let region = {idx: `${this.idx}-r${this.regionIdx++}`, curves: [], fillStyle: context.fillStyle, filled: context.fillShape  }
    pointerList[region.idx] = region
    this.regions = [region]
    for (let curve of this.curves) {
      this.regions[0].curves.push(curve)
    }
    if (this.regions[0].curves.length) {
      if (utils.dist(
        this.regions[0].curves[0].points[0],
        this.regions[0].curves[this.regions[0].curves.length - 1].points[3]
      ) < epsilon) {
        this.regions[0].filled = true
      }
    }

    // Generate vertices
    for (let curve of this.curves) {
      for (let index of [0, 3]) {
        tooClose = false
        for (let vertex of this.vertices) {
          if (utils.dist(curve.points[index], vertex.point) < epsilon){
            tooClose = true;
            vertex[["startCurves",,,"endCurves"][index]][i] = curve
            break
          }
        }
        if (!tooClose) {
          if (index==0) {
            this.vertices.push({
              point:curve.points[index],
              startCurves: {[i]:curve},
              endCurves: {}
            })
          } else {
            this.vertices.push({
              point:curve.points[index],
              startCurves: {},
              endCurves: {[i]:curve}
            })
          }
        }
      }
      i++;
    }

    let shapes = [this]
    this.vertices.forEach((vertex, i) => {
      for (let i=0; i<Math.min(10,this.regions.length); i++) {
        let region = this.regions[i]
        let regionVertexCurves = []
        let vertexCurves = {...vertex.startCurves, ...vertex.endCurves}
        if (Object.keys(vertexCurves).length==1) {
          // endpoint
          continue;
        } else if (Object.keys(vertexCurves).length==2) {
          // path vertex, don't need to do anything
          continue;
        } else if (Object.keys(vertexCurves).length==3) {
          // T junction. Region doesn't change but might need to update curves?
          // Skip for now.
          continue;
        } else if (Object.keys(vertexCurves).length==4) {
          // Intersection, split region in 2
          for (let i in vertexCurves) {
            let curve = vertexCurves[i]
            if (region.curves.includes(curve)) {
              regionVertexCurves.push(curve)
            }
          }
          let start = region.curves.indexOf(regionVertexCurves[1])
          let end = region.curves.indexOf(regionVertexCurves[3])
          if (end > start) {
            let newRegion = {
              idx: `${this.idx}-r${this.regionIdx++}`, // TODO: generate this deterministically so that undo/redo works
              curves: region.curves.splice(start, end - start),
              fillStyle: region.fillStyle,
              filled: true
            }
            pointerList[newRegion.idx] = newRegion
            this.regions.push(newRegion)  
          }
        } else {
          // not sure how to handle vertices with more than 4 curves
          console.log(`Unexpected vertex with ${Object.keys(vertexCurves).length} curves!`)
        }
      }
    })
  }
  
}

class GraphicsObject {
  constructor(uuid) {
    this.x = 0;
    this.y = 0;
    this.rotation = 0; // in radians
    this.scale_x = 1;
    this.scale_y = 1;
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    pointerList[this.idx] = this
    this.name = this.idx

    this.currentFrameNum = 0;
    this.currentLayer = 0;
    this.layers = [new Layer(uuid+"-L1")]
    this.audioLayers = []
    // this.children = []

    this.shapes = []
  }
  get activeLayer() {
    return this.layers[this.currentLayer]
  }
  get children() {
    return this.activeLayer.children
  }
  get currentFrame() {
    return this.getFrame(this.currentFrameNum)
  }
  get maxFrame() {
    return Math.max(this.layers.map((layer)=>{return layer.frames.length}))
  }
  advanceFrame() {
    this.setFrameNum(this.currentFrameNum + 1)
  }
  decrementFrame() {
    this.setFrameNum(this.currentFrameNum - 1)
  }
  getFrame(num) {
    return this.activeLayer.getFrame(num)
  }
  setFrameNum(num) {
    // this.currentFrameNum = Math.max(0, Math.min(this.maxFrame, num))
    this.currentFrameNum = Math.max(0, num)
    if (this.currentFrame.frameType=="keyframe") {
      for (let child of this.children) {
        if (this.currentFrame.keys[child.idx].goToFrame != undefined) {
          // Frames are 1-indexed
          child.setFrameNum(this.currentFrame.keys[child.idx].goToFrame - 1)
        }
      }
    }
  }
  bbox() {
    let bbox;
    for (let layer of this.layers) {
      let frame = layer.getFrame(this.currentFrameNum)
      if (frame.shapes.length > 0 && bbox == undefined) {
        bbox = structuredClone(frame.shapes[0].boundingBox)
      }
      for (let shape of frame.shapes) {
        growBoundingBox(bbox, shape.boundingBox)
      }
    }
    if (this.children.length > 0) {
      if (!bbox) {
        bbox = structuredClone(this.children[0].bbox())
      }
      for (let child of this.children) {
        growBoundingBox(bbox, child.bbox())
      }
    }
    if (bbox == undefined) {
      bbox = {x:{min:0, max:0}, y:{min:0,max:0}}
    }
    bbox.x.max *= this.scale_x
    bbox.y.max *= this.scale_y
    bbox.x.min += this.x
    bbox.x.max += this.x
    bbox.y.min += this.y
    bbox.y.max += this.y
    return bbox
  }
  draw(context) {
    let ctx = context.ctx;
    ctx.translate(this.x, this.y)
    ctx.rotate(this.rotation)
    ctx.scale(this.scale_x, this.scale_y)
    // if (this.currentFrameNum>=this.maxFrame) {
    //   this.currentFrameNum = 0;
    // }
    for (let layer of this.layers) {
      if (context.activeObject==this && !layer.visible) continue;
      let frame = layer.getFrame(this.currentFrameNum)
      for (let shape of frame.shapes) {
        if (context.shapeselection.indexOf(shape) >= 0) {
          invertPixels(ctx, fileWidth, fileHeight)
        }
        shape.draw(context)
        if (context.shapeselection.indexOf(shape) >= 0) {
          invertPixels(ctx, fileWidth, fileHeight)
        }
      }
      for (let child of layer.children) {
        if (child==context.activeObject) continue;
        let idx = child.idx
        if (idx in frame.keys) {
          child.x = frame.keys[idx].x;
          child.y = frame.keys[idx].y;
          child.rotation = frame.keys[idx].rotation;
          child.scale_x = frame.keys[idx].scale_x;
          child.scale_y = frame.keys[idx].scale_y;
          ctx.save()
          child.draw(context)
          if (true) {

          }
          ctx.restore()
        }
      }
    }
    if (this == context.activeObject) {
      if (context.activeCurve) {
        ctx.strokeStyle = "magenta"
        ctx.beginPath()
        ctx.moveTo(context.activeCurve.current.points[0].x, context.activeCurve.current.points[0].y)
        ctx.bezierCurveTo(context.activeCurve.current.points[1].x, context.activeCurve.current.points[1].y,
                          context.activeCurve.current.points[2].x, context.activeCurve.current.points[2].y,
                          context.activeCurve.current.points[3].x, context.activeCurve.current.points[3].y
        )
        ctx.stroke()
      }
      if (context.activeVertex) {
        ctx.save()
        ctx.strokeStyle = "#00ffff"
        let curves = {...context.activeVertex.current.startCurves,
          ...context.activeVertex.current.endCurves
        }
        // I don't understand why I can't use a for...of loop here
        for (let idx in curves) {
          let curve = curves[idx]
          ctx.beginPath()
          ctx.moveTo(curve.points[0].x, curve.points[0].y)
          ctx.bezierCurveTo(
            curve.points[1].x,curve.points[1].y,
            curve.points[2].x,curve.points[2].y,
            curve.points[3].x,curve.points[3].y
          )
          ctx.stroke()
        }
        ctx.fillStyle = "black"
        ctx.beginPath()
        let vertexSize = 15
        ctx.rect(context.activeVertex.current.point.x - vertexSize/2,
          context.activeVertex.current.point.y - vertexSize/2, vertexSize, vertexSize
        )
        ctx.fill()
        ctx.restore()
      }
      if (mode == "select") {
        for (let item of context.selection) {
          if (item.idx in this.currentFrame.keys) {
            ctx.save()
            ctx.strokeStyle = "#00ffff"
            ctx.lineWidth = 1;
            ctx.beginPath()
            let bbox = item.bbox()
            ctx.rect(bbox.x.min, bbox.y.min, bbox.x.max - bbox.x.min, bbox.y.max - bbox.y.min)
            ctx.stroke()
            ctx.restore()
          }
        }
        if (context.selectionRect) {
          ctx.save()
          ctx.strokeStyle = "#00ffff"
          ctx.lineWidth = 1;
          ctx.beginPath()
          ctx.rect(
            context.selectionRect.x1, context.selectionRect.y1,
            context.selectionRect.x2 - context.selectionRect.x1,
            context.selectionRect.y2 - context.selectionRect.y1
          )
          ctx.stroke()
          ctx.restore()
        }
      } else if (mode == "transform") {
        let bbox = undefined;
        for (let item of context.selection) {
          if (bbox==undefined) {
            bbox = structuredClone(item.bbox())
          } else {
            growBoundingBox(bbox, item.bbox())
          }
        }
        if (bbox != undefined) {
          // ctx.save()
          // ctx.strokeStyle = "#00ffff"
          // ctx.lineWidth = 1;
          // ctx.beginPath()
          // let xdiff = bbox.x.max - bbox.x.min
          // let ydiff =  bbox.y.max - bbox.y.min
          // ctx.rect(
          //   bbox.x.min, bbox.y.min,
          //   xdiff,
          //  ydiff
          // )
          // ctx.stroke()
          // ctx.fillStyle = "#000000"
          // let rectRadius = 5
          // for (let i of [[0,0],[0.5,0],[1,0],[1,0.5],[1,1],[0.5,1],[0,1],[0,0.5]]) {
          //   ctx.beginPath()
          //   ctx.rect(
          //     bbox.x.min + xdiff * i[0] - rectRadius,
          //     bbox.y.min + ydiff * i[1] - rectRadius,
          //     rectRadius*2, rectRadius*2
          //   )
          //   ctx.fill()
          // }

          // ctx.restore()
        }
      }
    }
  }
  transformMouse(mouse) {
    if (this.parent) {
      mouse = this.parent.transformMouse(mouse)
    }
    mouse.x -= this.x
    mouse.y -= this.y
    return mouse
  }
  addShape(shape, sendToBack=false) {
    if (sendToBack) {
      this.currentFrame.shapes.unshift(shape)
    } else {
      this.currentFrame.shapes.push(shape)
    }
  }
  addObject(object, x=0, y=0) {
    this.children.push(object)
    object.parent = this
    let idx = object.idx
    this.currentFrame.keys[idx] = {
      x: x,
      y: y,
      rotation: 0,
      scale_x: 1, 
      scale_y: 1,
      goToFrame: 1,
    }
  }
  removeShape(shape) {
    for (let layer of this.layers) {
      for (let frame of layer.frames) {
        let shapeIndex = frame.shapes.indexOf(shape)
        if (shapeIndex >= 0) {
          frame.shapes.splice(shapeIndex, 1)
        }
      }
    }
  }
  removeChild(childObject) {
    let idx = childObject.idx
    for (let layer of this.layers) {
      for (let frame of layer.frames) {
        delete frame[idx]
      }
    }
    this.children.splice(this.children.indexOf(childObject), 1)
  }
  saveState() {
    startProps[this.idx] = {
      x: this.x,
      y: this.y,
      rotation: this.rotation,
      scale_x: this.scale_x,
      scale_y: this.scale_y
    }
  }
  copy(idx) {
    let newGO = new GraphicsObject(idx.slice(0,8)+this.idx.slice(8))
    console.log(pointerList)
    console.log(newGO.idx)
    newGO.x = this.x;
    newGO.y = this.y;
    newGO.rotation = this.rotation;
    newGO.scale_x = this.scale_x;
    newGO.scale_y = this.scale_y;
    newGO.parent = this.parent;
    pointerList[this.idx] = this

    newGO.layers = []
    for (let layer of this.layers) {
      newGO.layers.push(layer.copy(idx))
    }
    for (let audioLayer of this.audioLayers) {
      newGO.audioLayers.push(audioLayer.copy(idx))
    }

    return newGO;
  }
}

let root = new GraphicsObject("root");
Object.defineProperty(
  context, 
  'activeObject', 
  {
      get: function() { 
          return this.objectStack.at(-1)
      }
  }
);
context.objectStack = [root]

async function greet() {
  // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
  greetMsgEl.textContent = await invoke("greet", { name: greetInputEl.value });

}

window.addEventListener("DOMContentLoaded", () => {
  rootPane = document.querySelector("#root")
  rootPane.appendChild(createPane(panes.toolbar))
  rootPane.addEventListener("mousemove", (e) => {
    mouseEvent = e;
  })
  let [_toolbar, panel] = splitPane(rootPane, 10, true, createPane(panes.timeline))
  let [stageAndTimeline, _infopanel] = splitPane(panel, 70, false, createPane(panes.infopanel))
  let [_timeline, _stage] = splitPane(stageAndTimeline, 30, false, createPane(panes.stage))
});

window.addEventListener("resize", () => {
  updateAll()
})

window.addEventListener("click", function(event) {
  const popupMenu = document.getElementById("popupMenu");

  // If the menu exists and the click is outside the menu and any button with the class 'paneButton', remove the menu
  if (popupMenu && !popupMenu.contains(event.target) && !event.target.classList.contains("paneButton")) {
      popupMenu.remove();  // Remove the menu from the DOM
  }
})

window.addEventListener("keydown", (e) => {
  // let shortcuts = {}
  // for (let shortcut of config.shortcuts) {
    // shortcut = shortcut.split("+")
    // TODO
  // }
  if (e.target.contentEditable == "plaintext-only") {
    return;
  }
  console.log(e)
  let mod = macOS ? e.metaKey : e.ctrlKey;
  let key = (mod ? "<mod>" : "") + e.key
  switch(key) {
    case config.shortcuts.playAnimation:
      console.log("Spacebar pressed")
      playPause()
      break;
    case config.shortcuts.new:
      newFile()
      break;
    case config.shortcuts.save:
      save()
      break;
    case config.shortcuts.saveAs:
      saveAs()
      break;
    case config.shortcuts.open:
      open()
      break;
    case config.shortcuts.import:
      importFile()
      break;
    case config.shortcuts.quit:
      quit()
      break;
    case config.shortcuts.undo:
      undo()
      break;
    case config.shortcuts.redo:
      redo()
      break;
    case config.shortcuts.copy:
      copy()
      break;
    case config.shortcuts.paste:
      paste()
      break;
    case config.shortcuts.delete:
      delete_action()
      break;
    case config.shortcuts.selectAll:
      selectAll()
      e.preventDefault()
      break;
    case config.shortcuts.group:
      actions.group.create()
      break;
    case config.shortcuts.zoomIn:
      zoomIn()
      break;
    case config.shortcuts.zoomOut:
      zoomOut()
      break;
    // TODO: put these in shortcuts
    case "<mod>ArrowRight":
      advanceFrame()
      e.preventDefault()
      break;
    case "ArrowRight":
      if (context.selection) {
        context.activeObject.currentFrame.saveState()
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].x += 1
        }
        actions.editFrame.create(context.activeObject.currentFrame)
        updateUI()
      }
      e.preventDefault()
      break;
    case "<mod>ArrowLeft":
      decrementFrame()
      break;
    case "ArrowLeft":
      if (context.selection) {
        context.activeObject.currentFrame.saveState()
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].x -= 1
        }
        actions.editFrame.create(context.activeObject.currentFrame)
        updateUI()
      }
      e.preventDefault()
      break;
    case "ArrowUp":
     if (context.selection) {
        context.activeObject.currentFrame.saveState()
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].y -= 1
        }
        actions.editFrame.create(context.activeObject.currentFrame)
        updateUI()
      }
      e.preventDefault()
      break;
    case "ArrowDown":
      if (context.selection) {
        context.activeObject.currentFrame.saveState()
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].y += 1
        }
        actions.editFrame.create(context.activeObject.currentFrame)
        updateUI()
      }
      e.preventDefault()
      break;
    default:
      break
  }
})

function playPause() {
  playing = !playing
  if (playing) {
    for (let audioLayer of context.activeObject.audioLayers) {
      console.log(1)
      for (let i in audioLayer.sounds) {
        let sound = audioLayer.sounds[i]
        sound.player.start(0,context.activeObject.currentFrameNum / fileFps)
      }
    }
    advanceFrame()
  } else {
    for (let audioLayer of context.activeObject.audioLayers) {
      for (let i in audioLayer.sounds) {
        let sound = audioLayer.sounds[i]
        sound.player.stop()
      }
    }

  }
}

function advanceFrame() {
  context.activeObject.advanceFrame()
  updateLayers()
  updateMenu()
  updateUI()
  if (playing) {
    if (context.activeObject.currentFrameNum < context.activeObject.maxFrame - 1) {
      setTimeout(advanceFrame, 1000/fileFps)
    } else {
      playing = false
      for (let audioLayer of context.activeObject.audioLayers) {
        for (let i in audioLayer.sounds) {
          let sound = audioLayer.sounds[i]
          sound.player.stop()
        }
      }
    }
  }
}

function decrementFrame() {
  context.activeObject.decrementFrame()
  updateLayers()
  updateMenu()
  updateUI()
}

function _newFile(width, height, fps) {
  root = new GraphicsObject("root");
  context.objectStack = [root]
  fileWidth = width
  fileHeight = height
  fileFps = fps
  undoStack = []
  redoStack = []
  for (let stage of document.querySelectorAll(".stage")) {
    stage.width = width
    stage.height = height
    stage.style.width = `${width}px`
    stage.style.height = `${height}px`
  }
  updateUI()
  updateLayers()
  updateMenu()
}

async function newFile() {
  if (await confirmDialog("Create a new file? Unsaved work will be lost.", {title: "New file", kind: "warning"})) {
    showNewFileDialog()
  }
}

async function _save(path) {
  try {
    const fileData = {
      version: "1.3",
      width: fileWidth,
      height: fileHeight,
      fps: fileFps,
      actions: undoStack
    }
    const contents = JSON.stringify(fileData);
    await writeTextFile(path, contents)
    filePath = path
    lastSaveIndex = undoStack.length - 1;
    console.log(`${path} saved successfully!`);
  } catch (error) {
    console.error("Error saving text file:", error);
  }
}

async function save() {
  if (filePath) {
    _save(filePath)
  } else {
    saveAs()
  }
}

async function saveAs() {
  const filename = filePath ? await basename(filePath) : "untitled.beam"
  const path = await saveFileDialog({
    filters: [
      {
        name: 'Lightningbeam files (.beam)',
        extensions: ['beam'],
      },
    ],
    defaultPath: await join(await documentDir(), filename)
  });
  if (path != undefined) _save(path);
}

async function open() {
  const path = await openFileDialog({
    multiple: false,
    directory: false,
    filters: [
      {
        name: 'Lightningbeam files (.beam)',
        extensions: ['beam'],
      },
    ],
    defaultPath: await documentDir(),
  });
  if (path) {
    try {
      const contents = await readTextFile(path)
      let file = JSON.parse(contents)
      if (file.version == undefined) {
        await messageDialog("Could not read file version!", { title: "Load error", kind: 'error' })
        return
      }
      if (file.version >= minFileVersion) {
        if (file.version < maxFileVersion) {
          _newFile(file.width, file.height, file.fps)
          if (file.actions == undefined) {
            await messageDialog("File has no content!", {title: "Parse error", kind: 'error'})
            return
          }
          for (let action of file.actions) {
            if (!(action.name in actions)) {
              await messageDialog(`Invalid action ${action.name}. File may be corrupt.`, { title: "Error", kind: 'error'})
              return
            }
            console.log(action.name)
            await actions[action.name].execute(action.action)
            undoStack.push(action)
          }
          lastSaveIndex = undoStack.length - 1;
          filePath = path
          updateUI()
        } else {
          await messageDialog(`File ${path} was created in a newer version of Lightningbeam and cannot be opened in this version.`, { title: 'File version mismatch', kind: 'error' });
        }
      } else {
        await messageDialog(`File ${path} is too old to be opened in this version of Lightningbeam.`, { title: 'File version mismatch', kind: 'error' });
      }
    } catch (e) {
      console.log(e )
      if (e instanceof SyntaxError) {
        await messageDialog(`Could not parse ${path}, ${e.message}`, { title: 'Error', kind: 'error' })
      } else if (e.startsWith("failed to read file as text")) {
        await messageDialog(`Could not parse ${path}, is it actually a Lightningbeam file?`, { title: 'Error', kind: 'error' })
      }
    }
  }
}

function revert() {
  for (let _=0; undoStack.length > lastSaveIndex+1; _++) {
    undo()
  }
}

async function importFile() {
  const path = await openFileDialog({
    multiple: false,
    directory: false,
    filters: [
      {
        name: 'Image files',
        extensions: ['png', 'gif', 'avif', 'jpg', 'jpeg'],
      },
      {
        name: 'Audio files',
        extensions: ['mp3'],
      },
    ],
    defaultPath: await documentDir(),
    title: "Import File"
  });
  const imageMimeTypes = [
    "image/jpeg",   // JPEG
    "image/png",    // PNG
    "image/gif",    // GIF
    "image/webp",   // WebP
    // "image/svg+xml",// SVG
    "image/bmp",    // BMP
    // "image/tiff",   // TIFF
    // "image/x-icon", // ICO
    // "image/heif",   // HEIF
    // "image/avif"    // AVIF
  ];
  const audioMimeTypes = [
    "audio/mpeg",      // MP3
    // "audio/wav",       // WAV
    // "audio/ogg",       // OGG
    // "audio/webm",      // WebM
    // "audio/aac",       // AAC
    // "audio/flac",      // FLAC
    // "audio/midi",      // MIDI
    // "audio/x-wav",     // X-WAV (older WAV files)
    // "audio/opus"       // Opus
  ];
  if (path) {
    const filename = await basename(path)
    const {dataURL, mimeType} = await convertToDataURL(path, imageMimeTypes.concat(audioMimeTypes));
    if (imageMimeTypes.indexOf(mimeType) != -1) {
      actions.addImageObject.create(50, 50, dataURL, 0, context.activeObject)
    } else {
      actions.addAudio.create(dataURL, context.activeObject, filename)
    }
  }
  
}

async function quit() {
  if (undoStack.length) {
    if (await confirmDialog("Are you sure you want to quit?", {title: 'Really quit?', kind: "warning"})) {
      getCurrentWindow().close()
    }
  } else {
    getCurrentWindow().close()
  }
}

function copy() {
  clipboard = []
  for (let object of context.selection) {
    clipboard.push(object)
  }
  for (let shape of context.shapeselection) {
    clipboard.push(shape)
  }
  console.log(clipboard)
}

function paste() {
  for (let item of clipboard) {
    if (item instanceof GraphicsObject) {
      console.log(item)
      // context.activeObject.addObject(item.copy())
      actions.duplicateObject.create(item)
    }
  }
  updateUI()
}

function delete_action() {
  if (context.selection.length || context.shapeselection.length) {
    actions.deleteObjects.create(context.selection, context.shapeselection)
    context.selection = []
  }
  updateUI()
}

function selectAll() {
  context.selection = []
  context.shapeselection = []
  for (let child of context.activeObject.activeLayer.children) {
    let idx = child.idx
    if (idx in context.activeObject.currentFrame.keys) {
      context.selection.push(child)
    }
  }
  for (let shape of context.activeObject.currentFrame.shapes) {
    context.shapeselection.push(shape)
  }
  updateUI()
}

function addFrame() {
  if (context.activeObject.currentFrameNum >= context.activeObject.activeLayer.frames.length) {
    actions.addFrame.create()
  }
}

function addKeyframe() {
  actions.addKeyframe.create()
}
async function about () {
  messageDialog(`Lightningbeam version ${await getVersion()}\nDeveloped by Skyler Lehmkuhl`,
    {title: 'About', kind: "info"}
  )
}

async function render() {
  document.querySelector("body").style.cursor = "wait"
  const path = await saveFileDialog({
    filters: [
      {
        name: 'APNG files (.png)',
        extensions: ['png'],
      },
    ],
    defaultPath: await join(await documentDir(), "untitled.png")
  });
  if (path != undefined) {
    
    // SVG balks on images
    // let ctx = new C2S(fileWidth, fileHeight)
    // context.ctx = ctx
    // root.draw(context)
    // let serializedSVG = ctx.getSerializedSvg()
    // await writeTextFile(path, serializedSVG)
    // fileExportPath = path
    // console.log("wrote SVG")


    const frames = [];
    const canvas = document.createElement('canvas');
    canvas.width = fileWidth;  // Set desired width
    canvas.height = fileHeight; // Set desired height
    let exportContext = {
      ...context,
      ctx: canvas.getContext('2d'),
      selectionRect: undefined,
      selection: [],
      shapeselection: []
    }

  
    for (let i = 0; i < root.maxFrame; i++) {
      
      root.currentFrameNum = i
      exportContext.ctx.fillStyle = "white"
      exportContext.ctx.rect(0,0,fileWidth, fileHeight)
      exportContext.ctx.fill()
      await root.draw(exportContext)

      // Convert the canvas content to a PNG image (this is the "frame" we add to the APNG)
      const imageData = exportContext.ctx.getImageData(0, 0, canvas.width, canvas.height);

    // Step 2: Create a frame buffer (Uint8Array) from the image data
      const frameBuffer = new Uint8Array(imageData.data.buffer);
      
      frames.push(frameBuffer); // Add the frame buffer to the frames array
    }
  
    // Step 3: Use UPNG.js to create the animated PNG
    const apng = UPNG.encode(frames, canvas.width, canvas.height, 0, parseInt(100/fileFps));
  
    // Step 4: Save the APNG file (in Tauri, use writeFile or in the browser, download it)
    const apngBlob = new Blob([apng], { type: 'image/png' });
  
    // If you're using Tauri:
    await writeFile(
      path, // The destination file path for saving
      new Uint8Array(await apngBlob.arrayBuffer())
    );
  }
  document.querySelector("body").style.cursor = "default"
}

function updateScrollPosition(zoomFactor) {
  if (context.mousePos) {
    for (let scroller of document.querySelectorAll(".scroll")) {
      let stage = scroller.querySelector('.stage')
      let scrollLeft = scroller.scrollLeft;
      let scrollTop = scroller.scrollTop;

      // Get the mouse position relative to the scroller (taking into account the scroller's position)
      let scrollerMouseX = context.mousePos.x + scrollLeft;
      let scrollerMouseY = context.mousePos.y + scrollTop;

      let newScrollLeft = scrollerMouseX - (context.mousePos.x * zoomFactor);
      let newScrollTop = scrollerMouseY - (context.mousePos.y * zoomFactor);

      // Apply the scroll position adjustments
      console.log(context.mousePos, scrollerMouseX, scrollerMouseY, newScrollLeft, newScrollTop)
      scroller.scrollLeft = -newScrollLeft;
      scroller.scrollTop = -newScrollTop;
    }
  }
}

function zoomIn() {
  let zoomFactor = 2
  if (context.zoomLevel < 8) {
    context.zoomLevel *= zoomFactor
    updateScrollPosition(zoomFactor)
    updateUI()
    updateMenu()
  }
}
function zoomOut() {
  let zoomFactor = 0.5
  if (context.zoomLevel > 1/8) {
    context.zoomLevel *= zoomFactor
    updateScrollPosition(zoomFactor)
    updateUI()
    updateMenu()
  }
}
function resetZoom() {
  context.zoomLevel = 1;
  updateUI()
  updateMenu()
}

function stage() {
  let stage = document.createElement("canvas")
  let scroller = document.createElement("div")
  let stageWrapper = document.createElement("div")
  stage.className = "stage"
  stage.width = 1500
  stage.height = 1000
  scroller.className = "scroll"
  stageWrapper.className = "stageWrapper"
  let selectionRect = document.createElement("div")
  selectionRect.className = "selectionRect"
  for (let i of ["nw", "ne", "se", "sw"]) {
    let cornerRotateRect = document.createElement("div")
    cornerRotateRect.classList.add("cornerRotateRect")
    cornerRotateRect.classList.add(i)
    cornerRotateRect.addEventListener('mouseup', (e) => {
      const newEvent = new MouseEvent(e.type, e);
      stage.dispatchEvent(newEvent)
    })
    cornerRotateRect.addEventListener('mousemove', (e) => {
      const newEvent = new MouseEvent(e.type, e);
      stage.dispatchEvent(newEvent)
    })
    selectionRect.appendChild(cornerRotateRect)
  }
  for (let i of ["nw", "n", "ne", "e", "se", "s", "sw", "w"]) {
    let cornerRect = document.createElement("div")
    cornerRect.classList.add("cornerRect")
    cornerRect.classList.add(i)
    cornerRect.addEventListener('mousedown', (e) => {
      let bbox = undefined;
      let selection = {}
      for (let item of context.selection) {
        if (bbox==undefined) {
          bbox = structuredClone(item.bbox())
        } else {
          growBoundingBox(bbox, item.bbox())
        }
        selection[item.idx] = {x: item.x, y: item.y, scale_x: item.scale_x, scale_y: item.scale_y}
      }
      if (bbox != undefined) {
        context.dragDirection = i
        context.activeTransform = {
          initial: {
            x: {min: bbox.x.min, max: bbox.x.max},
            y: {min: bbox.y.min, max: bbox.y.max},
            selection: selection
          },
          current: {
            x: {min: bbox.x.min, max: bbox.x.max},
            y: {min: bbox.y.min, max: bbox.y.max},
            selection: structuredClone(selection)
          }
        }
        context.activeObject.currentFrame.saveState()
      }
    })
    cornerRect.addEventListener('mouseup', (e) => {
      const newEvent = new MouseEvent(e.type, e);
      stage.dispatchEvent(newEvent)
    })
    cornerRect.addEventListener('mousemove', (e) => {
      const newEvent = new MouseEvent(e.type, e);
      stage.dispatchEvent(newEvent)
    })
    selectionRect.appendChild(cornerRect)
  }

  stage.addEventListener("drop", (e) => {
    e.preventDefault()
    let mouse = getMousePos(stage, e)
    const imageTypes = ['image/png', 'image/gif', 'image/avif', 'image/jpeg',
       'image/webp', //'image/svg+xml' // Disabling SVG until we can export them nicely
    ];
    const audioTypes = ['audio/mpeg'] // TODO: figure out what other audio formats Tone.js accepts
    if (e.dataTransfer.items) {
      let i = 0
      for (let item of e.dataTransfer.items) {
        if (item.kind == "file") {
          let file = item.getAsFile()
          if (imageTypes.includes(file.type)) {
            let img = new Image();
            let reader = new FileReader();
            
            // Read the file as a data URL
            reader.readAsDataURL(file);
            reader.ix = i
            
            reader.onload = function(event) {
              let imgsrc = event.target.result;  // This is the data URL
              actions.addImageObject.create(
                mouse.x, mouse.y, imgsrc, reader.ix, context.activeObject);
            };
  
            reader.onerror = function(error) {
              console.error("Error reading file as data URL", error);
            };
          } else if (audioTypes.includes(file.type)) {
            let reader = new FileReader();
            
            // Read the file as a data URL
            reader.readAsDataURL(file);
            reader.onload = function(event) {
              let audiosrc = event.target.result;
              actions.addAudio.create(audiosrc, context.activeObject, file.name)
            }
          }
          i++;
        }
      }
    } else {
    }
  })
  stage.addEventListener("dragover", (e) => {
    e.preventDefault()
  })
  canvases.push(stage)
  stageWrapper.appendChild(stage)
  stageWrapper.appendChild(selectionRect)
  scroller.appendChild(stageWrapper)
  stage.addEventListener("mousedown", (e) => {
    let mouse = getMousePos(stage, e)
    mouse = context.activeObject.transformMouse(mouse)
    switch (mode) {
      case "rectangle":
      case "ellipse":
      case "draw":
        context.mouseDown = true
        context.activeShape = new Shape(mouse.x, mouse.y, context, uuidv4())
        context.lastMouse = mouse
        break;
      case "select":
        if (context.activeObject.currentFrame.frameType != "keyframe") break;
        let selection = selectVertex(context, mouse)
        if (selection) {
          context.dragging = true
          context.activeCurve = undefined
          context.activeVertex = {
            current: {
              point: {x: selection.vertex.point.x, y: selection.vertex.point.y},
              startCurves: structuredClone(selection.vertex.startCurves),
              endCurves: structuredClone(selection.vertex.endCurves),
            },
            initial: selection.vertex,
            shape: selection.shape,
            startmouse: {x: mouse.x, y: mouse.y}
          }
          console.log("gonna move this")
        } else {
          selection = selectCurve(context, mouse)
          if (selection) {
            context.dragging = true
            context.activeVertex = undefined
            context.activeCurve = {
              initial: selection.curve,
              current: new Bezier(selection.curve.points).setColor(selection.curve.color),
              shape: selection.shape,
              startmouse: {x: mouse.x, y: mouse.y}
            }
            console.log("gonna move this")
          } else {
            let selected = false
            let child;
            if (context.selection.length) {
              for (child of context.selection) {
                if (hitTest(mouse, child)) {
                  context.dragging = true
                  context.lastMouse = mouse
                  context.activeObject.currentFrame.saveState()
                  break
                }
              }
            }
            if (!context.dragging) {
              // Have to iterate in reverse order to grab the frontmost object when two overlap
              for (let i=context.activeObject.children.length-1; i>=0; i--) {
                child = context.activeObject.children[i]
                if (!(child.idx in context.activeObject.currentFrame.keys)) continue;
                // let bbox = child.bbox()
                if (hitTest(mouse, child)) {
                    if (context.selection.indexOf(child) != -1) {
                      // dragging = true
                    }
                    child.saveState()
                    if (e.shiftKey) {
                      context.selection.push(child)
                    } else {
                      context.selection = [child]
                    }
                    context.dragging = true
                    selected = true
                    context.activeObject.currentFrame.saveState()
                    break
                }
              }
              if (!selected) {
                context.selection = []
                context.shapeselection = []
                context.selectionRect = {x1: mouse.x, x2: mouse.x, y1: mouse.y, y2:mouse.y}
              }
            }
          }
        }
        break;
      case "paint_bucket":
        let line = {p1: mouse, p2: {x: mouse.x + 3000, y: mouse.y}}
        debugCurves = []
        debugPoints = []
        let epsilon = context.fillGaps;
        let min_x = Infinity;
        let curveB = undefined
        let point = undefined
        let regionPoints
        
        // First, see if there's an existing shape to change the color of
        const startTime = performance.now()
        let pointShape = getShapeAtPoint(mouse, context.activeObject.currentFrame.shapes)
        const endTime = performance.now()

        console.log(pointShape)
        console.log(`getShapeAtPoint took ${endTime - startTime} milliseconds.`)

        if (pointShape) {
          actions.colorShape.create(pointShape, context.fillStyle);
          break;
        }
        
        // We didn't find an existing region to paintbucket, see if we can make one
        try {
          regionPoints = floodFillRegion(mouse,epsilon,fileWidth,fileHeight,context, debugPoints, debugPaintbucket)
        } catch (e) {
          updateUI()
          throw e;
          
        }
        console.log(regionPoints.length)
        if (regionPoints.length>0 && regionPoints.length < 10) {
          // probably a very small area, rerun with minimum epsilon
          regionPoints = floodFillRegion(mouse,1,fileWidth,fileHeight,context, debugPoints)
        }
        let points = []
        for (let point of regionPoints) {
          points.push([point.x, point.y])
        }
        let cxt = {
          ...context,
          fillShape: true,
          strokeShape: false,
          sendToBack: true
        }
        let shape = new Shape(regionPoints[0].x, regionPoints[0].y, cxt)
        shape.fromPoints(points, 1)
        actions.addShape.create(context.activeObject, shape, cxt)
        break
        // Loop labels in JS!
        shapeLoop:
        // Iterate in reverse so we paintbucket the frontmost shape
        for (let i=context.activeObject.currentFrame.shapes.length-1; i>=0; i--) {
          let shape = context.activeObject.currentFrame.shapes[i]
          // for (let region of shape.regions) {
            let intersect_count = 0;
            for (let curve of shape.curves) {
              intersect_count += curve.intersects(line).length
            }
            if (intersect_count%2==1) {
              actions.colorShape.create(shape, context.fillStyle)
              break shapeLoop;
            }
          // }
        }
        break;
      default:
        break;
    }
    context.lastMouse = mouse
    updateUI()
    updateInfopanel()
  })
  stage.addEventListener("mouseup", (e) => {
    context.mouseDown = false
    context.dragging = false
    context.dragDirection = undefined
    context.selectionRect = undefined
    let mouse = getMousePos(stage, e)
    mouse = context.activeObject.transformMouse(mouse)
    switch (mode) {
      case "draw":
        if (context.activeShape) {
          context.activeShape.addLine(mouse.x, mouse.y)
          context.activeShape.simplify(context.simplifyMode)
          actions.addShape.create(context.activeObject, context.activeShape)
          context.activeShape = undefined
        }
        break;
      case "rectangle":
      case "ellipse":
        actions.addShape.create(context.activeObject, context.activeShape)
        context.activeShape = undefined
        break;
      case "select":
        if (context.activeVertex) {
          let newCurves = []
          for (let i in context.activeVertex.shape.curves) {
            if (i in context.activeVertex.current.startCurves) {
              newCurves.push(context.activeVertex.current.startCurves[i])
            } else if (i in context.activeVertex.current.endCurves) {
              newCurves.push(context.activeVertex.current.endCurves[i])
            } else {
              newCurves.push(context.activeVertex.shape.curves[i])
            }
          }
          actions.editShape.create(context.activeVertex.shape, newCurves)
        } else if (context.activeCurve) {
          let newCurves = []
          for (let curve of context.activeCurve.shape.curves) {
            if (curve == context.activeCurve.initial) {
              newCurves.push(context.activeCurve.current)
            } else {
              newCurves.push(curve)
            }
          }
          actions.editShape.create(context.activeCurve.shape, newCurves)
        } else if (context.selection.length) {
          actions.editFrame.create(context.activeObject.currentFrame)
        } 
        break;
      case "transform":
        actions.editFrame.create(context.activeObject.currentFrame)
        break;
      default:
        break;
    }
    context.lastMouse = mouse
    context.activeCurve = undefined
    updateUI()
    updateMenu()
    updateInfopanel()
  })
  stage.addEventListener("mousemove", (e) => {
    let mouse = getMousePos(stage, e)
    mouse = context.activeObject.transformMouse(mouse)
    context.mousePos = mouse
    switch (mode) {
      case "draw":
        context.activeCurve = undefined
        if (context.activeShape) {
          if (vectorDist(mouse, context.lastMouse) > minSegmentSize) {
            context.activeShape.addLine(mouse.x, mouse.y)
            context.lastMouse = mouse
          }
        }
        break;
      case "rectangle":
        context.activeCurve = undefined
        if (context.activeShape) {
          context.activeShape.clear()
          context.activeShape.addLine(mouse.x, context.activeShape.starty)
          context.activeShape.addLine(mouse.x, mouse.y)
          context.activeShape.addLine(context.activeShape.startx, mouse.y)
          context.activeShape.addLine(context.activeShape.startx, context.activeShape.starty)
          context.activeShape.update()
        }
        break;
      case "ellipse":
        context.activeCurve = undefined
        if (context.activeShape) {
          let midX = (mouse.x + context.activeShape.startx) / 2
          let midY = (mouse.y + context.activeShape.starty) / 2
          let xDiff = (mouse.x - context.activeShape.startx) / 2
          let yDiff = (mouse.y - context.activeShape.starty) / 2
          let ellipseConst = 0.552284749831 // (4/3)*tan(pi/(2n)) where n=4
          context.activeShape.clear()
          context.activeShape.addCurve(new Bezier(
            midX, context.activeShape.starty,
            midX + ellipseConst * xDiff, context.activeShape.starty,
            mouse.x, midY - ellipseConst * yDiff,
            mouse.x, midY
          ))
          context.activeShape.addCurve(new Bezier(
            mouse.x, midY,
            mouse.x, midY + ellipseConst * yDiff,
            midX + ellipseConst * xDiff, mouse.y,
            midX, mouse.y
          ))
          context.activeShape.addCurve(new Bezier(
            midX, mouse.y,
            midX - ellipseConst * xDiff, mouse.y,
            context.activeShape.startx, midY + ellipseConst * yDiff,
            context.activeShape.startx, midY
          ))
          context.activeShape.addCurve(new Bezier(
            context.activeShape.startx, midY,
            context.activeShape.startx, midY - ellipseConst * yDiff,
            midX - ellipseConst * xDiff, context.activeShape.starty,
            midX, context.activeShape.starty
          ))
        }
        break;
      case "select":
        if (context.dragging) {
          if (context.activeVertex) {
            let vert = context.activeVertex
            let mouseDelta = {x: mouse.x - vert.startmouse.x, y: mouse.y - vert.startmouse.y}
            vert.current.point.x = vert.initial.point.x + mouseDelta.x
            vert.current.point.y = vert.initial.point.y + mouseDelta.y
            for (let i in vert.current.startCurves) {
              let curve = vert.current.startCurves[i]
              let oldCurve = vert.initial.startCurves[i]
              curve.points[0] = vert.current.point
              curve.points[1] = {
                x: oldCurve.points[1].x + mouseDelta.x,
                y: oldCurve.points[1].y + mouseDelta.y
              }
            }
            for (let i in vert.current.endCurves) {
              let curve = vert.current.endCurves[i]
              let oldCurve = vert.initial.endCurves[i]
              curve.points[3] = {x:vert.current.point.x, y:vert.current.point.y}
              curve.points[2] = {
                x: oldCurve.points[2].x + mouseDelta.x,
                y: oldCurve.points[2].y + mouseDelta.y
              }
            }
          } else if (context.activeCurve) {
            context.activeCurve.current.points = moldCurve(
              context.activeCurve.initial, mouse, context.activeCurve.startmouse
            ).points 
          } else {
            for (let child of context.selection) {
              context.activeObject.currentFrame.keys[child.idx].x += (mouse.x - context.lastMouse.x)
              context.activeObject.currentFrame.keys[child.idx] .y += (mouse.y - context.lastMouse.y)
            }
          }
        } else if (context.selectionRect) {
          context.selectionRect.x2 = mouse.x
          context.selectionRect.y2 = mouse.y
          context.selection = []
          context.shapeselection = []
          for (let child of context.activeObject.children) {
            if (hitTest(regionToBbox(context.selectionRect), child)) {
              context.selection.push(child)
            }
          }
          for (let shape of context.activeObject.currentFrame.shapes) {
            if (hitTest(regionToBbox(context.selectionRect), shape)) {
              context.shapeselection.push(shape)
            }
          }
        } else {
          let selection = selectVertex(context, mouse)
          if (selection) {
            context.activeCurve = undefined
            context.activeVertex = {
              current: selection.vertex,
              initial: {
                point: {x: selection.vertex.point.x, y: selection.vertex.point.y}, 
                startCurves: structuredClone(selection.vertex.startCurves),
                endCurves: structuredClone(selection.vertex.endCurves),
              },
              shape: selection.shape,
              startmouse: {x: mouse.x, y: mouse.y}
            }
          } else {
            context.activeVertex = undefined
            selection = selectCurve(context, mouse)
            if (selection) {
              context.activeCurve = {
                current: selection.curve, 
                initial: new Bezier(selection.curve.points).setColor(selection.curve.color),
                shape: selection.shape,
                startmouse: mouse
              }
            } else {
              context.activeCurve = undefined
            }
          }
        }
        context.lastMouse = mouse
        break;
      case "transform":
        if (context.dragDirection) {
          let initial = context.activeTransform.initial
          let current = context.activeTransform.current
          let initialSelection = context.activeTransform.initial.selection
          if (context.dragDirection.indexOf('n') != -1) {
            current.y.min = mouse.y
          } else if (context.dragDirection.indexOf('s') != -1) {
            current.y.max = mouse.y
          }
          if (context.dragDirection.indexOf('w') != -1) {
            current.x.min = mouse.x
          } else if (context.dragDirection.indexOf('e') != -1) {
            current.x.max = mouse.x
          }
            // Calculate the translation difference between current and initial values
          const delta_x = current.x.min - initial.x.min;
          const delta_y = current.y.min - initial.y.min;

          // Calculate the scaling factor based on the difference between current and initial values
          const scale_x_ratio = (current.x.max - current.x.min) / (initial.x.max - initial.x.min);
          const scale_y_ratio = (current.y.max - current.y.min) / (initial.y.max - initial.y.min);

          for (let idx in initialSelection) {
            let item = context.activeObject.currentFrame.keys[idx]
            let xoffset = initialSelection[idx].x - initial.x.min
            let yoffset = initialSelection[idx].y - initial.y.min
            item.x = initial.x.min + delta_x + xoffset * scale_x_ratio
            item.y = initial.y.min + delta_y + yoffset * scale_y_ratio    
            item.scale_x = initialSelection[idx].scale_x * scale_x_ratio
            item.scale_y = initialSelection[idx].scale_y * scale_y_ratio
          }
        }
        break;
      default:
        break;
    }
    updateUI()
  })
  stage.addEventListener("dblclick", (e) => {
    context.mouseDown = false
    context.dragging = false
    context.dragDirection = undefined
    context.selectionRect = undefined
    let mouse = getMousePos(stage, e)
    modeswitcher:
    switch(mode) {
      case "select":
        for (let i=context.activeObject.children.length-1; i>=0; i--) {
          let child = context.activeObject.children[i]
          if (!(child.idx in context.activeObject.currentFrame.keys)) continue;
          if (hitTest(mouse, child)) {
            context.objectStack.push(child)
            context.selection = [];
            context.shapeselection = [];
            updateUI()
            updateLayers()
            updateMenu()
            updateInfopanel()
            break modeswitcher;
          }
        }
        // we didn't click on a child, go up a level
        if (context.activeObject.parent) {
          context.selection = [context.activeObject]
          context.shapeselection = []
          context.objectStack.pop()
          updateUI()
          updateLayers()
          updateMenu()
          updateInfopanel()
        }
        break
      default:
        break;
    }
  })
  return scroller
}

function toolbar() {
  let tools_scroller = document.createElement("div")
  tools_scroller.className = "toolbar"
  for (let tool in tools) {
    let toolbtn = document.createElement("button")
    toolbtn.className = "toolbtn"
    let icon = document.createElement("img")
    icon.className = "icon"
    icon.src = tools[tool].icon
    toolbtn.appendChild(icon)
    tools_scroller.appendChild(toolbtn)
    toolbtn.addEventListener("click", () => {
      mode = tool
      updateInfopanel()
      updateUI()
      console.log(tool)
    })
  }
  let tools_break = document.createElement("div")
  tools_break.className = "horiz_break"
  tools_scroller.appendChild(tools_break)
  let fillColor = document.createElement("div")
  let strokeColor = document.createElement("div")
  fillColor.className = "color-field"
  strokeColor.className = "color-field"
  fillColor.setColor = (color) => {
    fillColor.style.setProperty('--color', color)
    fillColor.color = color
    context.fillStyle = color
  }
  strokeColor.setColor = (color) => {
    strokeColor.style.setProperty('--color', color)
    strokeColor.color = color
    context.strokeStyle = color
  }
  fillColor.setColor("#ff0000");
  strokeColor.setColor("#000000");
  fillColor.style.setProperty('--label-text', `"Fill color:"`)
  strokeColor.style.setProperty('--label-text', `"Stroke color:"`)
  fillColor.type="color"
  fillColor.value = "#ff0000"
  strokeColor.value = "#000000"
  let evtListener;
  let padding = 10
  let gradwidth = 25
  let ccwidth = 300
  let mainSize = ccwidth - (3*padding + gradwidth)
  let colorClickHandler = (e) => {
    let colorCvs = document.querySelector("#color-cvs")
    if (colorCvs==null) {
      console.log('creating new one')
      colorCvs = document.createElement("canvas")
      colorCvs.id = "color-cvs"
      document.body.appendChild(colorCvs)
      colorCvs.width = ccwidth
      colorCvs.height = 500
      colorCvs.style.width = "300px"
      colorCvs.style.height = "500px"
      colorCvs.style.position = "absolute"
      colorCvs.style.left = '500px'
      colorCvs.style.top = '500px'
      colorCvs.style.boxShadow = "0 2px 2px rgba(0,0,0,0.2)"
      colorCvs.style.cursor = "crosshair"
      colorCvs.currentColor = "#00ffba88"
      colorCvs.draw = function() {
        const darkMode = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
        let ctx = colorCvs.getContext('2d')
        ctx.lineWidth = 2;
        if (darkMode) {
          ctx.fillStyle = "#333"
        } else {
          ctx.fillStyle = "#ccc" //TODO
        }
        ctx.fillRect(0,0,colorCvs.width, colorCvs.height)

        // draw current color
        drawCheckerboardBackground(ctx, padding, padding, colorCvs.width - 2*padding, 50, 10)
        ctx.fillStyle = colorCvs.currentColor
        ctx.fillRect(padding,padding,colorCvs.width-2*padding, 50)

        // Draw main gradient
        let mainGradient = ctx.createImageData(mainSize, mainSize)
        let data = mainGradient.data
        let {h, s, v} = hexToHsv(colorCvs.currentColor)
        for (let i=0; i<data.length; i+=4) {
          let x = ((i/4)%mainSize) / mainSize
          let y = Math.floor((i/4) / mainSize) / mainSize;
          let hue = h
          let rgb = hsvToRgb(hue, x, 1-y)
          data[i+0] = rgb.r;
          data[i+1] = rgb.g;
          data[i+2] = rgb.b;
          data[i+3] = 255;
        }
        ctx.putImageData(mainGradient, padding, 2*padding + 50)
        // draw pointer
        ctx.beginPath()
        ctx.arc(s*mainSize + padding, (1-v)*mainSize + 2*padding + 50, 3, 0, 2*Math.PI)
        ctx.strokeStyle = "white"
        ctx.stroke()


        let hueGradient = ctx.createImageData(mainSize, gradwidth)
        data = hueGradient.data
        for (let i=0; i<data.length; i+=4) {
          let x = ((i/4)%mainSize) / mainSize
          let y = Math.floor((i/4) / gradwidth)
          let rgb = hslToRgb(x, 1, 0.5)
          data[i+0] = rgb.r;
          data[i+1] = rgb.g;
          data[i+2] = rgb.b;
          data[i+3] = 255;
        }
        ctx.putImageData(hueGradient, padding, 3*padding + 50 + mainSize)
        // draw pointer
        ctx.beginPath()
        ctx.rect(h * mainSize + padding - 2, 3*padding + 50 + mainSize, 4, gradwidth)
        ctx.strokeStyle = "white"
        ctx.stroke()

        drawCheckerboardBackground(ctx, colorCvs.width - (padding + gradwidth), 2*padding+50, gradwidth, mainSize, 10)
        const gradient = ctx.createLinearGradient(0, 2*padding+50, 0, 2*padding+50+mainSize); // Vertical gradient
        gradient.addColorStop(0, `${colorCvs.currentColor.slice(0,7)}ff`); // Full color at the top
        gradient.addColorStop(1, `${colorCvs.currentColor.slice(0,7)}00`)
        ctx.fillStyle = gradient
        ctx.fillRect(colorCvs.width - (padding + gradwidth), 2*padding+50, gradwidth, mainSize)
        let alpha = parseInt(colorCvs.currentColor.slice(7,9) || 'ff', 16) / 255;
        // draw pointer
        ctx.beginPath()
        ctx.rect(colorCvs.width - (padding + gradwidth), 2*padding+50 + (1-alpha) * mainSize - 2, gradwidth, 4)
        ctx.strokeStyle = "white"
        ctx.stroke()
      }
      colorCvs.addEventListener('mousedown', (e) => {
        colorCvs.clickedMainGradient = false
        colorCvs.clickedHueGradient = false
        colorCvs.clickedAlphaGradient = false
        let mouse = getMousePos(colorCvs, e)
        let {h, s, v} = hexToHsv(colorCvs.currentColor)
        if (mouse.x > padding && mouse.x < padding + mainSize &&
          mouse.y > 2*padding + 50 && mouse.y < 2*padding + 50 + mainSize) {
          // we clicked in the main gradient
          let x = (mouse.x - padding) / mainSize
          let y = (mouse.y - (2*padding + 50)) / mainSize
          let rgb = hsvToRgb(h, x, 1-y)
          let alpha = colorCvs.currentColor.slice(7,9) || 'ff'
          colorCvs.currentColor = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha
          colorCvs.clickedMainGradient = true
        } else if (mouse.x > padding && mouse.x < padding + mainSize &&
          mouse.y > 3*padding + 50+ mainSize && mouse.y < 3*padding + 50 + mainSize + gradwidth
        ) {
          // we clicked in the hue gradient
          let x = (mouse.x - padding) / mainSize
          let rgb = hsvToRgb(x, s, v)
          let alpha = colorCvs.currentColor.slice(7,9) || 'ff'
          colorCvs.currentColor = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha
          colorCvs.clickedHueGradient = true
        } else if (mouse.x > colorCvs.width - (padding + gradwidth) && mouse.x < colorCvs.width - padding &&
          mouse.y > 2 * padding + 50 && mouse.y < 2 * padding + 50 + mainSize
        ) {
          // we clicked in the alpha gradient
          let y = 1 - ((mouse.y - (2 * padding + 50)) / mainSize)
          let alpha = Math.round(y*255).toString(16)
          colorCvs.currentColor = `${colorCvs.currentColor.slice(0,7)}${alpha}`
          colorCvs.clickedAlphaGradient = true
        }
        colorCvs.colorEl.setColor(colorCvs.currentColor);
        colorCvs.draw()
      })
      
      window.addEventListener('mouseup', (e) => {
        let mouse = getMousePos(colorCvs, e)
        colorCvs.clickedMainGradient = false
        colorCvs.clickedHueGradient = false
        colorCvs.clickedAlphaGradient = false
        if (e.target != colorCvs) {
          colorCvs.style.display = 'none'
          window.removeEventListener('mousemove', evtListener)
        }
      })
    } else {
      colorCvs.style.display = "block"
    }
    evtListener = window.addEventListener('mousemove', (e) => {
      let mouse = getMousePos(colorCvs, e)
      let {h, s, v} = hexToHsv(colorCvs.currentColor)
      if (colorCvs.clickedMainGradient) {
        let x = clamp((mouse.x - padding) / mainSize)
        let y = clamp((mouse.y - (2*padding + 50)) / mainSize)
        let rgb = hsvToRgb(h, x, 1-y)
        let alpha = colorCvs.currentColor.slice(7,9) || 'ff'
        colorCvs.currentColor = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha
        colorCvs.draw()
      } else if (colorCvs.clickedHueGradient) {
        let x = clamp((mouse.x - padding) / mainSize)
        let rgb = hsvToRgb(x, s, v)
        let alpha = colorCvs.currentColor.slice(7,9) || 'ff'
        colorCvs.currentColor = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha
        colorCvs.draw()
      } else if (colorCvs.clickedAlphaGradient) {
        let y = clamp(1 - ((mouse.y - (2 * padding + 50)) / mainSize))
        let alpha = Math.round(y * 255).toString(16).padStart(2, '0');
        colorCvs.currentColor = `${colorCvs.currentColor.slice(0,7)}${alpha}`
        colorCvs.draw()
      }
      console.log(colorCvs.currentColor)
      colorCvs.colorEl.setColor(colorCvs.currentColor);
    })
    // Get mouse coordinates relative to the viewport
    const mouseX = e.clientX + window.scrollX;
    const mouseY = e.clientY + window.scrollY;

    const divWidth = colorCvs.offsetWidth;
    const divHeight = colorCvs.offsetHeight;
    const windowWidth = window.innerWidth;
    const windowHeight = window.innerHeight;

    // Default position to the mouse cursor
    let left = mouseX;
    let top = mouseY;

    // If the window is narrower than twice the width, center horizontally
    if (windowWidth < divWidth * 2) {
        left = (windowWidth - divWidth) / 2;
    } else {
        // If it would overflow on the right side, position it to the left of the cursor
        if (left + divWidth > windowWidth) {
            left = mouseX - divWidth;
        }
    }

    // If the window is shorter than twice the height, center vertically
    if (windowHeight < divHeight * 2) {
        top = (windowHeight - divHeight) / 2;
    } else {
        // If it would overflow at the bottom, position it above the cursor
        if (top + divHeight > windowHeight) {
            top = mouseY - divHeight;
        }
    }

    colorCvs.style.left = `${left}px`;
    colorCvs.style.top = `${top}px`;
    colorCvs.colorEl = e.target
    colorCvs.currentColor = e.target.color
    colorCvs.draw()
    e.preventDefault()
  }
  fillColor.addEventListener('click', colorClickHandler)
  strokeColor.addEventListener('click', colorClickHandler)
  // Fill and stroke colors use the same set of swatches
  fillColor.addEventListener("change", e => {
    context.swatches.unshift(fillColor.value)
    if (context.swatches.length>12) context.swatches.pop();
  })
  strokeColor.addEventListener("change", e => {
    context.swatches.unshift(strokeColor.value)
    if (context.swatches.length>12) context.swatches.pop();
  })
  tools_scroller.appendChild(fillColor)
  tools_scroller.appendChild(strokeColor)
  return tools_scroller
}

function timeline() {
  let container = document.createElement("div")
  let layerspanel = document.createElement("div")
  let framescontainer = document.createElement("div")
  container.classList.add("horizontal-grid")
  container.classList.add("layers-container")
  layerspanel.className = "layers"
  framescontainer.className = "frames-container"
  container.appendChild(layerspanel)
  container.appendChild(framescontainer)
  layoutElements.push(container)
  container.setAttribute("lb-percent", 20)

  return container
}

function infopanel() {
  let panel = document.createElement("div")
  panel.className = "infopanel"
  updateInfopanel()
  return panel
}


createNewFileDialog(_newFile);
showNewFileDialog()

function createPaneMenu(div) {
  const menuItems = ["Item 1", "Item 2", "Item 3"]; // The items for the menu

  // Get the menu container (create a new div for the menu)
  const popupMenu = document.createElement("div");
  popupMenu.id = "popupMenu";  // Set the ID to ensure we can target it later

  // Create a <ul> element to hold the list items
  const ul = document.createElement("ul");

  // Loop through the menuItems array and create a <li> for each item
  for (let pane in panes) {
      const li = document.createElement("li");
      // Create the <img> element for the icon
      const img = document.createElement("img");
      img.src = `assets/${panes[pane].name}.svg`;  // Use the appropriate SVG as the source
      // img.style.width = "20px";  // Set the icon size
      // img.style.height = "20px";  // Set the icon size
      // img.style.marginRight = "10px";  // Add space between the icon and text

      // Append the image to the <li> element
      li.appendChild(img);

      // Set the text of the item
      li.appendChild(document.createTextNode(titleCase(panes[pane].name)));
      li.addEventListener("click", () => {
        createPane(panes[pane], div)
        updateUI()
        updateLayers()
        updateAll()
        popupMenu.remove()
      })
      ul.appendChild(li); // Append the <li> to the <ul>
  }

  popupMenu.appendChild(ul); // Append the <ul> to the popupMenu div
  document.body.appendChild(popupMenu); // Append the menu to the body
  return popupMenu; // Return the created menu element
}

function createPane(paneType=undefined, div=undefined) {
  if (!div) {
    div = document.createElement("div")
  } else {
    div.textContent = ''
  }
  let header = document.createElement("div")
  if (!paneType) {
    paneType = panes.stage // TODO: change based on type
  }
  let content = paneType.func()
  header.className = "header"

  let button = document.createElement("button")
  header.appendChild(button)
  let icon = document.createElement("img")
  icon.className="icon"
  icon.src = `/assets/${paneType.name}.svg`
  button.appendChild(icon)
  button.addEventListener("click", () => {
    let popupMenu = document.getElementById("popupMenu");

    // If the menu is already in the DOM, remove it
    if (popupMenu) {
        popupMenu.remove(); // Remove the menu from the DOM
    } else {
        // Create and append the new menu to the DOM
        popupMenu = createPaneMenu(div);

        // Position the menu below the button
        const buttonRect = event.target.getBoundingClientRect();
        popupMenu.style.left = `${buttonRect.left}px`;
        popupMenu.style.top = `${buttonRect.bottom + window.scrollY}px`;
    }

    // Prevent the click event from propagating to the window click listener
    event.stopPropagation();
  })

  div.className = "vertical-grid"
  header.style.height = "calc( 2 * var(--lineheight))"
  content.style.height = "calc( 100% - 2 * var(--lineheight) )"
  div.appendChild(header)
  div.appendChild(content)
  return div
}

function splitPane(div, percent, horiz, newPane=undefined) {
  let content = div.firstElementChild
  let div1 = document.createElement("div")
  let div2 = document.createElement("div")

  div1.className = "panecontainer"
  div2.className = "panecontainer"

  div1.appendChild(content)
  if (newPane) {
    div2.appendChild(newPane)
  } else {
    div2.appendChild(createPane())
  }
  div.appendChild(div1)
  div.appendChild(div2)

  if (horiz) {
    div.className = "horizontal-grid"
  } else {
    div.className = "vertical-grid"
  }
  div.setAttribute("lb-percent", percent) // TODO: better attribute name
  div.addEventListener('mousedown', function(event) {
    // Check if the clicked element is the parent itself and not a child element
    if (event.target === event.currentTarget) {
      event.currentTarget.setAttribute("dragging", true)
      event.currentTarget.style.userSelect = 'none';
      rootPane.style.userSelect = "none";
    } else {
      event.currentTarget.setAttribute("dragging", false)
    }
  });
  div.addEventListener('mousemove', function(event) {
    // Check if the clicked element is the parent itself and not a child element
    if (event.currentTarget.getAttribute("dragging")=="true") {
      const frac = getMousePositionFraction(event, event.currentTarget)
      div.setAttribute("lb-percent", frac*100)
      updateAll()
    }
  });
  div.addEventListener('mouseup', (event) => {
    console.log("mouseup")
    event.currentTarget.setAttribute("dragging", false)
    // event.currentTarget.style.userSelect = 'auto';
  })
  updateAll()
  updateUI()
  updateLayers()
  return [div1, div2]
}

function updateAll() {
  updateLayout(rootPane)
  for (let element of layoutElements) {
    updateLayout(element)
  }
}

function updateLayout(element) {
  let rect = element.getBoundingClientRect()
  let percent = element.getAttribute("lb-percent")
  percent ||= 50
  let children = element.children
  if (children.length != 2) return;
  if (element.classList.contains("horizontal-grid")) {
    children[0].style.width = `${rect.width * percent / 100}px`
    children[1].style.width = `${rect.width * (100 - percent) / 100}px`
    children[0].style.height = `${rect.height}px`
    children[1].style.height = `${rect.height}px`
  } else if (element.classList.contains("vertical-grid")) {
    children[0].style.height = `${rect.height * percent / 100}px`
    children[1].style.height = `${rect.height * (100 - percent) / 100}px`
    children[0].style.width = `${rect.width}px`
    children[1].style.width = `${rect.width}px`
  }
  if (children[0].getAttribute("lb-percent")) {
    updateLayout(children[0])
  }
  if (children[1].getAttribute("lb-percent")) {
    updateLayout(children[1])
  }
}

function updateUI() {
  for (let canvas of canvases) {
    canvas.width = fileWidth * context.zoomLevel
    canvas.height = fileHeight * context.zoomLevel
    canvas.style.width = `${fileWidth * context.zoomLevel}px`
    canvas.style.height = `${fileHeight * context.zoomLevel}px`
    let ctx = canvas.getContext("2d")
    ctx.resetTransform();
    ctx.scale(context.zoomLevel, context.zoomLevel)
    ctx.beginPath()
    ctx.fillStyle = "white"
    ctx.fillRect(0,0,fileWidth,fileHeight)

    context.ctx = ctx;
    root.draw(context)
    if (context.activeObject != root) {
      ctx.fillStyle = "rgba(255,255,255,0.5)"
      ctx.fillRect(0,0,fileWidth,fileHeight)
      context.activeObject.draw(context)
    }
    if (context.activeShape) {
      context.activeShape.draw(context)
    }

    // Debug rendering
    if (debugQuadtree) {

      ctx.fillStyle = "rgba(255,255,255,0.5)"
      ctx.fillRect(0,0,fileWidth,fileHeight)
      const ep = 2.5
      const bbox = {
        x: { min: context.mousePos.x - ep, max: context.mousePos.x + ep },
        y: { min: context.mousePos.y - ep, max: context.mousePos.y + ep }
      };
      debugCurves = []
      for (let shape of context.activeObject.currentFrame.shapes) {
        for (let i of shape.quadtree.query(bbox)) {
          debugCurves.push(shape.curves[i])
        }
      }
    }
    // let i=4;
    for (let curve of debugCurves) {
      ctx.beginPath()
      // ctx.strokeStyle = `#ff${i}${i}${i}${i}`
      // i = (i+3)%10
      ctx.strokeStyle = '#'+(Math.random()*0xFFFFFF<<0).toString(16);
      ctx.moveTo(curve.points[0].x, curve.points[0].y)
      ctx.bezierCurveTo(
        curve.points[1].x, curve.points[1].y,
        curve.points[2].x, curve.points[2].y,
        curve.points[3].x, curve.points[3].y
      )
      ctx.stroke()
      ctx.beginPath()
      let bbox = curve.bbox()
      ctx.rect(bbox.x.min,bbox.y.min,bbox.x.max-bbox.x.min,bbox.y.max-bbox.y.min)
      ctx.stroke()
    }
    let i=0;
    for (let point of debugPoints) {
      ctx.beginPath()
      let j = i.toString(16).padStart(2, '0');
      ctx.fillStyle = `#${j}ff${j}`
      i+=1
      i %= 255
      ctx.arc(point.x, point.y, 3, 0, 2*Math.PI)
      ctx.fill()
    }

  }
  for (let selectionRect of document.querySelectorAll(".selectionRect")) {
    selectionRect.style.display = "none"
  }
  if (mode == "transform") {
    if (context.selection.length > 0) {
      for (let selectionRect of document.querySelectorAll(".selectionRect")) {
        let bbox = undefined;
        for (let item of context.selection) {
          if (bbox==undefined) {
            bbox = structuredClone(item.bbox())
          } else {
            growBoundingBox(bbox, item.bbox())
          }
        }
        if (bbox != undefined) {
          selectionRect.style.display = "block"
          selectionRect.style.left = `${bbox.x.min}px`
          selectionRect.style.top = `${bbox.y.min}px`
          selectionRect.style.width = `${bbox.x.max - bbox.x.min}px`
          selectionRect.style.height = `${bbox.y.max - bbox.y.min}px`
        }
      }
    }
  }
}

function updateLayers() {
  for (let container of document.querySelectorAll(".layers-container")) {
    let layerspanel = container.querySelectorAll(".layers")[0]
    let framescontainer = container.querySelectorAll(".frames-container")[0]
    layerspanel.textContent = ""
    framescontainer.textContent = ""
    for (let layer of context.activeObject.layers) {
      let layerHeader = document.createElement("div")
      layerHeader.className = "layer-header"
      if (context.activeObject.activeLayer == layer) {
        layerHeader.classList.add("active")
      }
      layerspanel.appendChild(layerHeader)
      let layerName = document.createElement("div")
      layerName.className = "layer-name"
      layerName.contentEditable = "plaintext-only"
      layerName.addEventListener("click", (e) => {
        e.stopPropagation()
      })
      layerName.addEventListener("blur", (e) => {
        actions.changeLayerName.create(layer, layerName.innerText)
      })
      layerName.innerText = layer.name
      layerHeader.appendChild(layerName)
      // Visibility icon element
      let visibilityIcon = document.createElement("img")
      visibilityIcon.className = "visibility-icon"
      visibilityIcon.src = layer.visible ? "assets/eye-fill.svg" : "assets/eye-slash.svg"

      // Toggle visibility on click
      visibilityIcon.addEventListener("click", (e) => {
        e.stopPropagation() // Prevent click from bubbling to the layerHeader click listener
        layer.visible = !layer.visible
        // visibilityIcon.src = layer.visible ? "assets/eye-fill.svg" : "assets/eye-slash.svg"
        updateUI()
        updateMenu()
        updateLayers()
      })

      layerHeader.appendChild(visibilityIcon)
      layerHeader.addEventListener("click", (e) => {
        context.activeObject.currentLayer = context.activeObject.layers.indexOf(layer)
        updateLayers()
        updateUI()
      })
      let layerTrack = document.createElement("div")
      layerTrack.className = "layer-track"
      if (!layer.visible) {
        layerTrack.classList.add("invisible")
      }
      framescontainer.appendChild(layerTrack)
      layerTrack.addEventListener("click", (e) => {
        let mouse = getMousePos(layerTrack, e)
        let frameNum = parseInt(mouse.x/25)
        context.activeObject.setFrameNum(frameNum)
        updateLayers()
        updateMenu()
        updateUI()
        updateInfopanel()
      })
      let highlightedFrame = false
      layer.frames.forEach((frame, i) => {
        let frameEl = document.createElement("div")
        frameEl.className = "frame"
        frameEl.setAttribute("frameNum", i)
        if (i == context.activeObject.currentFrameNum) {
          frameEl.classList.add("active")
          highlightedFrame = true
        }

        frameEl.classList.add(frame.frameType)
        layerTrack.appendChild(frameEl)
      })
      if (!highlightedFrame) {
        let highlightObj = document.createElement("div")
        let frameCount = layer.frames.length
        highlightObj.className = "frame-highlight"
        highlightObj.style.left = `${(context.activeObject.currentFrameNum - frameCount) * 25}px`;
        layerTrack.appendChild(highlightObj)
      }
    }
    for (let audioLayer of context.activeObject.audioLayers) {
      let layerHeader = document.createElement("div")
      layerHeader.className = "layer-header"
      layerHeader.classList.add("audio")
      layerspanel.appendChild(layerHeader)
      let layerTrack = document.createElement("div")
      layerTrack.className = "layer-track"
      layerTrack.classList.add("audio")
      framescontainer.appendChild(layerTrack)
      console.log(audioLayer)
      for (let i in audioLayer.sounds) {
        let sound = audioLayer.sounds[i]
        layerTrack.appendChild(sound.img)
      }
      let layerName = document.createElement("div")
      layerName.className = "layer-name"
      layerName.contentEditable = "plaintext-only"
      layerName.addEventListener("click", (e) => {
        e.stopPropagation()
      })
      layerName.addEventListener("blur", (e) => {
        actions.changeLayerName.create(audioLayer, layerName.innerText)
      })
      layerName.innerText = audioLayer.name
      layerHeader.appendChild(layerName)
    }
  }
}

function updateInfopanel() {
  for (let panel of document.querySelectorAll('.infopanel')) {
    panel.innerText = ""
    let input;
    let label;
    let span;
    let breadcrumbs = document.createElement("div")
    const bctitle = document.createElement('span');
    bctitle.style.cursor = "default"
    bctitle.textContent = "Active object: ";
    breadcrumbs.appendChild(bctitle);
    let crumbs = []
    for (let object of context.objectStack) {
      crumbs.push({name: object.name, object: object})
    }
    crumbs.forEach((crumb, index) => {
      const crumbText = document.createElement('span');
      crumbText.textContent = crumb.name;
      breadcrumbs.appendChild(crumbText);

      if (index < crumbs.length - 1) {
        const separator = document.createElement('span');
        separator.textContent = ' > ';
        separator.style.cursor = "default"
        crumbText.style.cursor = "pointer"
        breadcrumbs.appendChild(separator);
      } else {
        crumbText.style.cursor = "default"
      }
    });

    breadcrumbs.addEventListener('click', function(event) {
      const span = event.target;

      // Only handle clicks on the breadcrumb text segments (not the separators)
      if (span.tagName === 'SPAN' && span.textContent !== ' > ') {
        const clickedText = span.textContent;

        // Find the crumb associated with the clicked text
        const crumb = crumbs.find(c => c.name === clickedText);
        if (crumb) {
          const index = context.objectStack.indexOf(crumb.object);
          if (index !== -1) {
            // Keep only the objects up to the clicked one and add the clicked one as the last item
            context.objectStack = context.objectStack.slice(0, index + 1);
            updateUI()
            updateLayers()
            updateMenu()
            updateInfopanel()
          }
        }
      }
    });
    panel.appendChild(breadcrumbs)
    for (let property in tools[mode].properties) {
      let prop = tools[mode].properties[property]
      label = document.createElement("label")
      label.className = "infopanel-field"
      span = document.createElement("span")
      span.className = "infopanel-label"
      span.innerText = prop.label
      switch (prop.type) {
        case "number":
          input = document.createElement("input")
          input.className = "infopanel-input"
          input.type = "number"
          input.disabled = prop.enabled==undefined ? false : !prop.enabled()
          if (prop.value) {
            input.value = prop.value.get()
          } else {
            input.value = getProperty(context, property)
          }
          if (prop.min) {
            input.min = prop.min
          }
          if (prop.max) {
            input.max = prop.max
          }
          break;
        case "enum":
          input = document.createElement("select")
          input.className = "infopanel-input"
          input.disabled = prop.enabled==undefined ? false : !prop.enabled()
          let optionEl;
          for (let option of prop.options) {
            optionEl = document.createElement("option")
            optionEl.value = option
            optionEl.innerText = option
            input.appendChild(optionEl)
          }
          if (prop.value) {
            input.value = prop.value.get()
          } else {
            input.value = getProperty(context, property)
          }
          break;
        case "boolean":
          input = document.createElement("input")
          input.className = "infopanel-input"
          input.type = "checkbox"
          input.disabled = prop.enabled==undefined ? false : !prop.enabled()
          if (prop.value) {
            input.checked = prop.value.get()
          } else {
            input.checked = getProperty(context, property)
          }
          break;
        case "text":
          input = document.createElement("input")
          input.className = "infopanel-input"
          input.disabled = prop.enabled==undefined ? false : !prop.enabled()
          if (prop.value) {
            input.value = prop.value.get()
          } else {
            input.value = getProperty(context, property)
          }
          break;
      }
      input.addEventListener("input", (e) => {
        switch (prop.type) {
          case "number":
            if (!isNaN(e.target.value) && e.target.value > 0) {
              if (prop.value) {
                prop.value.set(parseInt(e.target.value))
              } else {
                setProperty(context, property, parseInt(e.target.value))
              }
            }
            break;
          case "enum":
            if (prop.options.indexOf(e.target.value) >= 0) {
              setProperty(context, property, e.target.value)
            }
            break;
          case "boolean":
            if (prop.value) {
              prop.value.set(e.target.value)
            } else {
              setProperty(context, property, e.target.checked)
            }
            break;
          case "text":
            if (prop.value) {
              prop.value.set(e.target.value)
            } else {
              setProperty(context, property, parseInt(e.target.value))
            }
            break;
        }

      })
      label.appendChild(span)
      label.appendChild(input)
      panel.appendChild(label)
    }
  }
}

async function updateMenu() {
  let activeFrame;
  let activeKeyframe;
  let newFrameMenuItem;
  let newKeyframeMenuItem;
  let deleteFrameMenuItem;
  
  activeKeyframe = false
  if (context.activeObject.activeLayer.frames[context.activeObject.currentFrameNum]) {
    activeFrame = true
    if (context.activeObject.activeLayer.frames[context.activeObject.currentFrameNum].frameType=="keyframe") {
      activeKeyframe = true
    }
  } else {
    activeFrame = false
  }
  const appSubmenu = await Submenu.new({
    text: 'Lightningbeam',
    items: [
      {
        text: 'About Lightningbeam',
        enabled: true,
        action:about
      },
      {
        text: 'Settings',
        enabled: false,
        action: () => {}
      },
      {
        text: 'Quit Lightningbeam',
        enabled: true,
        action: quit,
      },
    ]
  })
  const fileSubmenu = await Submenu.new({
    text: 'File',
    items: [
      {
        text: 'New file...',
        enabled: true,
        action: newFile,
      },
      {
        text: 'Save',
        enabled: true,
        action: save,
      },
      {
        text: 'Save As...',
        enabled: true,
        action: saveAs,
      },
      {
        text: 'Open File...',
        enabled: true,
        action: open,
      },
      {
        text: 'Revert',
        enabled: undoStack.length > lastSaveIndex,
        action: revert,
      },
      {
        text: 'Import...',
        enabled: true,
        action: importFile,
      },
      {
        text: "Export...",
        enabled: true,
        action: render
      },
      {
        text: 'Quit',
        enabled: true,
        action: quit,
      },
    ]
  })

  const editSubmenu = await Submenu.new({
    text: "Edit",
    items: [
      {
        text: "Undo " + ((undoStack.length>0) ? camelToWords(undoStack[undoStack.length-1].name) : ""),
        enabled: undoStack.length > 0,
        action: undo
      },
      {
        text: "Redo " + ((redoStack.length>0) ? camelToWords(redoStack[redoStack.length-1].name) : ""),
        enabled: redoStack.length > 0,
        action: redo
      },
      {
        text: "Cut",
        enabled: false,
        action: () => {}
      },
      {
        text: "Copy",
        enabled: (context.selection.length > 0 || context.shapeselection.length > 0),
        action: copy
      },
      {
        text: "Paste",
        enabled: true,
        action: paste
      },
      {
        text: "Delete",
        enabled: (context.selection.length > 0 || context.shapeselection.length > 0),
        action: delete_action
      },
      {
        text: "Select All",
        enabled: true,
        action: selectAll
      },
    ]
  });

  const modifySubmenu = await Submenu.new({
    text: "Modify",
    items: [
      {
        text: "Group",
        enabled: context.selection.length != 0 || context.shapeselection.length != 0,
        action: actions.group.create
      },
      {
        text: "Send to back",
        enabled: context.selection.length != 0 || context.shapeselection.length != 0,
        action: actions.sendToBack.create
      },
      {
        text: "Bring to front",
        enabled: context.selection.length != 0 || context.shapeselection.length != 0,
        action: actions.bringToFront.create
      },
    ]
  })

  const layerSubmenu = await Submenu.new({
    text: "Layer",
    items: [
      {
        text: "Add Layer",
        enabled: true,
        action: actions.addLayer.create
      },
      {
        text: "Delete Layer",
        enabled: context.activeObject.layers.length > 1,
        action: actions.deleteLayer.create
      },
      {
        text: context.activeObject.activeLayer.visible ? "Hide Layer" : "Show Layer",
        enabled: true,
        action: () => {context.activeObject.activeLayer.toggleVisibility()}
      }
    ]
  })

  newFrameMenuItem = {
    text: "New Frame",
    enabled: !activeFrame,
    action: addFrame
  }
  newKeyframeMenuItem = {
    text: "New Keyframe",
    enabled: !activeKeyframe,
    action: addKeyframe
  }
  deleteFrameMenuItem = {
    text: "Delete Frame",
    enabled: activeFrame,
    action: () => {}
  }

  const timelineSubmenu = await Submenu.new({
    text: "Timeline",
    items: [
      newFrameMenuItem,
      newKeyframeMenuItem,
      deleteFrameMenuItem,
      {
        text: "Add Motion Tween",
        enabled: activeFrame && (!activeKeyframe),
        action: actions.addMotionTween.create
      },
      {
        text: "Add Shape Tween",
        enabled: activeFrame && (!activeKeyframe),
        action: actions.addShapeTween.create
      },
      {
        text: "Return to start",
        enabled: false,
        action: () => {}
      },
      {
        text: "Play",
        enabled: !playing,
        action: playPause
      },
    ]
  });
  const viewSubmenu = await Submenu.new({
    text: "View",
    items: [
      {
        text: "Zoom In",
        enabled: true,
        action: zoomIn
      },
      {
        text: "Zoom Out",
        enabled: true,
        action: zoomOut
      },
      {
        text: "Actual Size",
        enabled: context.zoomLevel != 1,
        action: resetZoom
      },
    ]
  }); 
  const helpSubmenu = await Submenu.new({
    text: "Help",
    items: [
      {
        text: "About...",
        enabled: true,
        action: about
      }
    ]
  });

  let items = [fileSubmenu, editSubmenu, modifySubmenu, layerSubmenu, timelineSubmenu, viewSubmenu, helpSubmenu]
  if (macOS) {
    items.unshift(appSubmenu)
  }
  const menu = await Menu.new({
    items: items,
  })
  await (macOS ? menu.setAsAppMenu() : menu.setAsWindowMenu())
}
updateMenu()

const panes = {
  stage: {
    name: "stage",
    func: stage
  },
  toolbar: {
    name: "toolbar",
    func: toolbar
  },
  timeline: {
    name: "timeline",
    func: timeline
  },
  infopanel: {
    name: "infopanel",
    func: infopanel
  },
}

function _arrayBufferToBase64( buffer ) {
  var binary = '';
  var bytes = new Uint8Array( buffer );
  var len = bytes.byteLength;
  for (var i = 0; i < len; i++) {
      binary += String.fromCharCode( bytes[ i ] );
  }
  return window.btoa( binary );
}

async function convertToDataURL(filePath, allowedMimeTypes) {
  try {
    // Read the image file as a binary file (buffer)
    const binaryData = await readFile(filePath);
    const mimeType = getMimeType(filePath);
    if (!mimeType) {
      throw new Error('Unsupported file type');
    }
    if (allowedMimeTypes.indexOf(mimeType)==-1) {
      throw new Error(`Unsupported MIME type ${mimeType}`)
    }

    const base64Data = _arrayBufferToBase64(binaryData)
    const dataURL = `data:${mimeType};base64,${base64Data}`;

    return {dataURL, mimeType};
  } catch (error) {
    console.log(error)
    console.error('Error reading the file:', error);
    return null;
  }
}

// Determine the MIME type based on the file extension
function getMimeType(filePath) {
  const ext = filePath.split('.').pop().toLowerCase();
  switch (ext) {
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'png':
      return 'image/png';
    case 'gif':
      return 'image/gif';
    case 'bmp':
      return 'image/bmp';
    case 'webp':
      return 'image/webp';
    case 'mp3':
      return 'audio/mpeg'
    default:
      return null; // Unsupported file type
  }
}

function startToneOnUserInteraction() {
  // Function to handle the first interaction (click or key press)
  const startTone = () => {
    Tone.start().then(() => {
      console.log("Tone.js started!");
    }).catch(err => {
      console.error("Error starting Tone.js:", err);
    });

    // Remove the event listeners to prevent them from firing again
    document.removeEventListener("click", startTone);
    document.removeEventListener("keydown", startTone);
  };

  // Add event listeners for mouse click and key press
  document.addEventListener("click", startTone);
  document.addEventListener("keydown", startTone);
}
startToneOnUserInteraction()
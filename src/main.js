const { invoke } = window.__TAURI__.core;
import * as fitCurve from "/fit-curve.js";
import { Bezier } from "/bezier.js";
import { Quadtree } from "./quadtree.js";
import {
  createNewFileDialog,
  showNewFileDialog,
  closeDialog,
} from "./newfile.js";
import {
  titleCase,
  getMousePositionFraction,
  getKeyframesSurrounding,
  lerpColor,
  lerp,
  camelToWords,
  generateWaveform,
  floodFillRegion,
  getShapeAtPoint,
  hslToRgb,
  drawCheckerboardBackground,
  hexToHsl,
  hsvToRgb,
  hexToHsv,
  rgbToHex,
  clamp,
  drawBorderedRect,
  drawCenteredText,
  drawHorizontallyCenteredText,
  deepMerge,
  getPointNearBox,
  arraysAreEqual,
  drawRegularPolygon,
  getFileExtension,
  createModal,
  deeploop,
  signedAngleBetweenVectors,
  rotateAroundPoint,
  getRotatedBoundingBox,
  rotateAroundPointIncremental,
  rgbToHsv,
  multiplyMatrices,
  growBoundingBox,
  createMissingTexturePattern,
} from "./utils.js";
import {
  backgroundColor,
  darkMode,
  foregroundColor,
  frameWidth,
  gutterHeight,
  highlight,
  iconSize,
  triangleSize,
  labelColor,
  layerHeight,
  layerWidth,
  scrubberColor,
  shade,
  shadow,
} from "./styles.js";
import { Icon } from "./icon.js";
import { AlphaSelectionBar, ColorSelectorWidget, ColorWidget, HueSelectionBar, SaturationValueSelectionGradient, TimelineWindow, Widget } from "./widgets.js";
const {
  writeTextFile: writeTextFile,
  readTextFile: readTextFile,
  writeFile: writeFile,
  readFile: readFile,
} = window.__TAURI__.fs;
const {
  open: openFileDialog,
  save: saveFileDialog,
  message: messageDialog,
  confirm: confirmDialog,
} = window.__TAURI__.dialog;
const { documentDir, join, basename, appLocalDataDir } = window.__TAURI__.path;
const { Menu, MenuItem, PredefinedMenuItem, Submenu } = window.__TAURI__.menu;
const { getCurrentWindow } = window.__TAURI__.window;
const { getVersion } = window.__TAURI__.app;

import init, { CoreInterface } from './pkg/lightningbeam_core.js';

window.onerror = (message, source, lineno, colno, error) => {
  invoke("error", { msg: `${message} at ${source}:${lineno}:${colno}\n${error?.stack || ''}` });
};

window.addEventListener('unhandledrejection', (event) => {
  invoke("error", { msg: `Unhandled Promise Rejection: ${event.reason?.stack || event.reason}` });
});

function forwardConsole(fnName, dest) {
  const original = console[fnName];
  console[fnName] = (...args) => {
    const error = new Error();
    const stackLines = error.stack.split("\n");

    let message = args.join(" ");  // Join all arguments into a single string
    const location = stackLines.length>1 ? stackLines[1].match(/([a-zA-Z0-9_-]+\.js:\d+)/) : stackLines.toString();

    if (fnName === "error") {
      // Send the full stack trace for errors
      invoke(dest, { msg: `${message}\nStack trace:\n${stackLines.slice(1).join("\n")}` });
    } else {
      // For other log levels, just extract the file and line number
      invoke(dest, { msg: `${location ? location[0] : 'unknown'}: ${message}` });
    }

    original(location ? location[0] : 'unknown', ...args);  // Pass all arguments to the original console method
  };
}

forwardConsole('trace', "trace");
forwardConsole('log', "trace");
forwardConsole('debug', "debug");
forwardConsole('info', "info");
forwardConsole('warn', "warn");
forwardConsole('error', "error");

console.log("*** Starting Lightningbeam ***")

// Debug flags
const debugQuadtree = false;
const debugPaintbucket = false;

const macOS = navigator.userAgent.includes("Macintosh");

let simplifyPolyline = simplify;

let greetInputEl;
let greetMsgEl;
let rootPane;

let canvases = [];

let debugCurves = [];
let debugPoints = [];

let mode = "select";

let minSegmentSize = 5;
let maxSmoothAngle = 0.6;

let undoStack = [];
let redoStack = [];
let lastSaveIndex = 0;

let layoutElements = [];

// Version changes:
// 1.4: addShape uses frame as a reference instead of object
// 1.6: object coordinates are created relative to their location

let minFileVersion = "1.3";
let maxFileVersion = "2.0";

let filePath = undefined;
let fileExportPath = undefined;

let state = "normal";

let playing = false;
let lastFrameTime;

let uiDirty = false;
let layersDirty = false;
let menuDirty = false;
let outlinerDirty = false;
let infopanelDirty = false;
let lastErrorMessage = null;  // To keep track of the last error
let repeatCount = 0;

let clipboard = [];

const CONFIG_FILE_PATH = "config.json";
const defaultConfig = {};

let tools = {
  select: {
    icon: "/assets/select.svg",
    properties: {
      selectedObjects: {
        type: "text",
        label: "Selected Object",
        enabled: () => context.selection.length == 1,
        value: {
          get: () => {
            if (context.selection.length == 1) {
              return context.selection[0].name;
            } else if (context.selection.length == 0) {
              return "";
            } else {
              return "<multiple>";
            }
          },
          set: (val) => {
            if (context.selection.length == 1) {
              actions.setName.create(context.selection[0], val);
            }
          },
        },
      },
      goToFrame: {
        type: "number",
        label: "Go To Frame",
        enabled: () => context.selection.length == 1,
        value: {
          get: () => {
            if (context.selection.length != 1) return undefined;
            const selectedObject = context.selection[0];
            if (! (selectedObject.idx in context.activeObject.currentFrame.keys)) return undefined;
            return context.activeObject.currentFrame.keys[selectedObject.idx]
              .goToFrame;
          },
          set: (val) => {
            if (context.selection.length != 1) return undefined;
            const selectedObject = context.selection[0];
            context.activeObject.currentFrame.keys[
              selectedObject.idx
            ].goToFrame = val;
            selectedObject.setFrameNum(val - 1);
            updateUI();
          },
        },
      },
      playFromFrame: {
        type: "boolean",
        label: "Play From Frame",
        enabled: () => context.selection.length == 1,
        value: {
          get: () => {
            if (context.selection.length != 1) return undefined;
            const selectedObject = context.selection[0];
            if (!(selectedObject.idx in context.activeObject.currentFrame.keys)) return;
            return context.activeObject.currentFrame.keys[selectedObject.idx]
              .playFromFrame;
          },
          set: (val) => {
            if (context.selection.length != 1) return undefined;
            const selectedObject = context.selection[0];
            context.activeObject.currentFrame.keys[
              selectedObject.idx
            ].playFromFrame = val;
            selectedObject.playing = true;
            updateUI();
          },
        },
      },
    },
  },
  transform: {
    icon: "/assets/transform.svg",
    properties: {},
  },
  draw: {
    icon: "/assets/draw.svg",
    properties: {
      lineWidth: {
        type: "number",
        label: "Line Width",
      },
      simplifyMode: {
        type: "enum",
        options: ["corners", "smooth", "verbatim"], // "auto"],
        label: "Line Mode",
      },
      fillShape: {
        type: "boolean",
        label: "Fill Shape",
      },
    },
  },
  rectangle: {
    icon: "/assets/rectangle.svg",
    properties: {
      lineWidth: {
        type: "number",
        label: "Line Width",
      },
      fillShape: {
        type: "boolean",
        label: "Fill Shape",
      },
    },
  },
  ellipse: {
    icon: "assets/ellipse.svg",
    properties: {
      lineWidth: {
        type: "number",
        label: "Line Width",
      },
      fillShape: {
        type: "boolean",
        label: "Fill Shape",
      },
    },
  },
  paint_bucket: {
    icon: "/assets/paint_bucket.svg",
    properties: {
      fillGaps: {
        type: "number",
        label: "Fill Gaps",
        min: 1,
      },
    },
  },
  eyedropper: {
    icon: "/assets/eyedropper.svg",
    properties: {
      dropperColor: {
        type: "enum",
        options: ["Fill color", "Stroke color"],
        label: "Color"
      }
    }
  }
};

let mouseEvent;

let context = {
  mouseDown: false,
  mousePos: { x: 0, y: 0 },
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
  dropperColor: "Fill color",
  dragging: false,
  selectionRect: undefined,
  selection: [],
  shapeselection: [],
  oldselection: [],
  oldshapeselection: [],
  selectedFrames: [],
  dragDirection: undefined,
  zoomLevel: 1,
};

let config = {
  shortcuts: {
    playAnimation: " ",
    undo: "<mod>z",
    redo: "<mod>Z",
    new: "<mod>n",
    newWindow: "<mod>N",
    save: "<mod>s",
    saveAs: "<mod>S",
    open: "<mod>o",
    import: "<mod>i",
    export: "<mod>e",
    quit: "<mod>q",
    copy: "<mod>c",
    paste: "<mod>v",
    delete: "Backspace",
    selectAll: "<mod>a",
    group: "<mod>g",
    addLayer: "<mod>l",
    addKeyframe: "F6",
    addBlankKeyframe: "F7",
    zoomIn: "<mod>+",
    zoomOut: "<mod>-",
    resetZoom: "<mod>0",
  },
  fileWidth: 800,
  fileHeight: 600,
  framerate: 24,
  recentFiles: [],
  scrollSpeed: 1,
  debug: false,
  reopenLastSession: false
};

function getShortcut(shortcut) {
  if (!(shortcut in config.shortcuts)) return undefined;

  let shortcutValue = config.shortcuts[shortcut].replace("<mod>", "CmdOrCtrl+");
  const key = shortcutValue.slice(-1);

  // If the last character is uppercase, prepend "Shift+" to it
  return key === key.toUpperCase() && key !== key.toLowerCase()
    ? shortcutValue.replace(key, `Shift+${key}`)
    : shortcutValue.replace("++", "+Shift+="); // Hardcode uppercase from = to +
}

// Load the configuration from the file system
async function loadConfig() {
  try {
    // const configPath = await join(await appLocalDataDir(), CONFIG_FILE_PATH);
    // const configData = await readTextFile(configPath);
    const configData = localStorage.getItem("lightningbeamConfig") || "{}";
    config = deepMerge({ ...config }, JSON.parse(configData));
    updateUI();
  } catch (error) {
    console.log("Error loading config, returning default config:", error);
  }
}

// Save the configuration to a file
async function saveConfig() {
  try {
    // const configPath = await join(await appLocalDataDir(), CONFIG_FILE_PATH);
    // await writeTextFile(configPath, JSON.stringify(config, null, 2));
    localStorage.setItem(
      "lightningbeamConfig",
      JSON.stringify(config, null, 2),
    );
  } catch (error) {
    console.error("Error saving config:", error);
  }
}

async function addRecentFile(filePath) {
  config.recentFiles = [filePath, ...config.recentFiles.filter(file => file !== filePath)].slice(0, 10);
  await saveConfig(config);
}

// Pointers to all objects
let pointerList = {};
// Keeping track of initial values of variables when we edit them continuously
let startProps = {};

let actions = {
  addShape: {
    create: (parent, shape, ctx) => {
      if (!parent.currentFrame?.exists) return;
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
        frame: parent.currentFrame.idx,
      };
      undoStack.push({ name: "addShape", action: action });
      actions.addShape.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let object = pointerList[action.parent];
      let frame = action.frame
        ? pointerList[action.frame]
        : object.currentFrame;
      let curvesList = action.curves;
      let cxt = {
        ...context,
        ...action.context,
      };
      let shape = new Shape(action.startx, action.starty, cxt, action.uuid);
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
        frame.addShape(newShape, cxt.sendToBack, frame);
      }
    },
    rollback: (action) => {
      let object = pointerList[action.parent];
      let frame = action.frame
        ? pointerList[action.frame]
        : object.currentFrame;
      let shape = pointerList[action.uuid];
      frame.removeShape(shape);
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
      let imageShape = new Shape(0, 0, ct, action.shapeUuid);
      imageShape.addLine(img.width, 0);
      imageShape.addLine(img.width, img.height);
      imageShape.addLine(0, img.height);
      imageShape.addLine(0, 0);
      imageShape.update();
      imageShape.fillImage = img;
      imageShape.filled = true;
      imageObject.currentFrame.addShape(imageShape);
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
    create: (audiosrc, object, audioname) => {
      redoStack.length = 0;
      let action = {
        audiosrc: audiosrc,
        audioname: audioname,
        uuid: uuidv4(),
        layeruuid: uuidv4(),
        frameNum: object.currentFrameNum,
        object: object.idx,
      };
      undoStack.push({ name: "addAudio", action: action });
      actions.addAudio.execute(action);
      updateMenu();
    },
    execute: async (action) => {
      const player = new Tone.Player().toDestination();
      await player.load(action.audiosrc);
      // player.autostart = true;
      let newAudioLayer = new AudioLayer(action.layeruuid, action.audioname);
      let object = pointerList[action.object];
      const img = new Image();
      img.className = "audioWaveform";
      let soundObj = {
        player: player,
        start: action.frameNum,
        img: img,
        src: action.audiosrc,
        uuid: action.uuid
      };
      pointerList[action.uuid] = soundObj;
      newAudioLayer.sounds[action.uuid] = soundObj;
      // TODO: change start time
      newAudioLayer.track.add(0, action.uuid);
      object.audioLayers.push(newAudioLayer);
      // TODO: compute image height better
      generateWaveform(img, player.buffer, 50, 25, config.framerate);
      updateLayers();
    },
    rollback: (action) => {
      let object = pointerList[action.object];
      let layer = pointerList[action.layeruuid];
      object.audioLayers.splice(object.audioLayers.indexOf(layer), 1);
      updateLayers();
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
        frame: context.activeObject.currentFrame.idx,
        uuid: uuidv4(),
      };
      undoStack.push({ name: "duplicateObject", action: action });
      actions.duplicateObject.execute(action);
      updateMenu();
    },
    execute: (action) => {
      const object = pointerList[action.object];
      const frame = pointerList[action.frame];
      for (let item of action.items) {
        if (item.type == "shape") {
          frame.addShape(Shape.fromJSON(item));
        } else if (item.type == "GraphicsObject") {
          const newObj = GraphicsObject.fromJSON(item);
          object.addObject(newObj);
        }
      }
      // newObj.idx = action.uuid
      // context.activeObject.addObject(newObj);
      updateUI();
    },
    rollback: (action) => {
      const object = pointerList[action.object];
      const frame = pointerList[action.frame];
      for (let item of action.items) {
        if (item.type == "shape") {
          frame.removeShape(pointerList[item.idx]);
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
      let serializableObjects = [];
      let serializableShapes = [];
      for (let object of objects) {
        serializableObjects.push(object.idx);
      }
      for (let shape of shapes) {
        serializableShapes.push(shape.idx);
      }
      let action = {
        objects: serializableObjects,
        shapes: serializableShapes,
        frame: context.activeObject.currentFrame.idx,
        oldState: structuredClone(context.activeObject.currentFrame.keys),
      };
      undoStack.push({ name: "deleteObjects", action: action });
      actions.deleteObjects.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let frame = pointerList[action.frame];
      for (let object of action.objects) {
        delete frame.keys[object];
      }
      for (let shape of action.shapes) {
        frame.shapes.splice(frame.shapes.indexOf(pointerList[shape]), 1);
      }
      updateUI();
    },
    rollback: (action) => {
      let frame = pointerList[action.frame];
      for (let object of action.objects) {
        frame.keys[object] = action.oldState[object];
      }
      for (let shape of action.shapes) {
        frame.shapes.push(pointerList[shape]);
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
      let layer = new Layer(action.uuid);
      layer.name = `Layer ${object.layers.length + 1}`;
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
      if (!(layer instanceof Layer)) {
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
        case "Layer":
          let layer = Layer.fromJSON(action.object);
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
        case "Layer":
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
  editFrame: {
    initialize: (frame) => {
      let action = {
        type: "editFrame",
        oldState: structuredClone(frame.keys),
        frame: frame.idx,
      };
      return action;
    },
    finalize: (action, frame) => {
      action.newState = structuredClone(frame.keys);
      undoStack.push({ name: "editFrame", action: action });
      actions.editFrame.execute(action);
      context.activeAction = undefined;
      updateMenu();
    },
    render: (action, ctx) => {},
    create: (frame) => {
      redoStack.length = 0; // Clear redo stack
      if (!(frame.idx in startProps)) return;
      let action = {
        newState: structuredClone(frame.keys),
        oldState: startProps[frame.idx],
        frame: frame.idx,
      };
      undoStack.push({ name: "editFrame", action: action });
      actions.editFrame.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let frame = pointerList[action.frame];
      if (frame == undefined) return;
      frame.keys = structuredClone(action.newState);
      updateUI();
    },
    rollback: (action) => {
      let frame = pointerList[action.frame];
      frame.keys = structuredClone(action.oldState);
      updateUI();
    },
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
      const frame = context.activeObject.currentFrame
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx);
        if (bbox == undefined) {
          bbox = shape.bbox();
        } else {
          growBoundingBox(bbox, shape.bbox());
        }
      }
      for (let object of context.selection) {
        if (object.idx in frame.keys) {
          serializableObjects.push(object.idx);
          // TODO: rotated bbox
          if (bbox == undefined) {
            bbox = object.bbox();
          } else {
            growBoundingBox(bbox, object.bbox());
          }
        }
      }
      context.shapeselection = [];
      context.selection = [];
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        groupUuid: uuidv4(),
        parent: context.activeObject.idx,
        frame: frame.idx,
        layer: context.activeObject.activeLayer.idx,
        position: {
          x: (bbox.x.min + bbox.x.max) / 2,
          y: (bbox.y.min + bbox.y.max) / 2,
        },
      };
      undoStack.push({ name: "group", action: action });
      actions.group.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let group = new GraphicsObject(action.groupUuid);
      let parent = pointerList[action.parent];
      let frame = action.frame
        ? pointerList[action.frame]
        : parent.currentFrame;
      let layer;
      if (action.layer) {
        layer = pointerList[action.layer]
      } else {
        for (let _layer of parent.layers) {
          for (let _frame of _layer.frames) {
            if (_frame && (_frame.idx == frame.idx)) {
              layer = _layer
            }
          }
        }
        if (layer==undefined) {
          layer = parent.activeLayer
        }
      }
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        shape.translate(-action.position.x, -action.position.y);
        group.currentFrame.addShape(shape);
        frame.removeShape(shape);
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        group.addObject(
          object,
          frame.keys[objectIdx].x - action.position.x,
          frame.keys[objectIdx].y - action.position.y,
        );
        parent.removeChild(object);
      }
      parent.addObject(group, action.position.x, action.position.y, frame);
      context.selection = [group];
      updateUI();
      updateInfopanel();
    },
    rollback: (action) => {
      let group = pointerList[action.groupUuid];
      let parent = pointerList[action.parent];
      let frame = action.frame
        ? pointerList[action.frame]
        : parent.currentFrame;
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        frame.addShape(shape);
        shape.translate(action.position.x, action.position.y);
        group.getFrame(0).removeShape(shape);
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        parent.addObject(object, object.x, object.y);
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
      let serializableShapes = [];
      let serializableObjects = [];
      let formerIndices = {};
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx);
        formerIndices[shape.idx] =
          context.activeObject.currentFrame.shapes.indexOf(shape);
      }
      for (let object of context.selection) {
        serializableObjects.push(object.idx);
        formerIndices[object.idx] =
          context.activeObject.activeLayer.children.indexOf(object);
      }
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        layer: context.activeObject.activeLayer.idx,
        frame: context.activeObject.currentFrame.idx,
        formerIndices: formerIndices,
      };
      undoStack.push({ name: "sendToBack", action: action });
      actions.sendToBack.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let frame = pointerList[action.frame];
      let layer = pointerList[action.layer];
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        frame.shapes.splice(frame.shapes.indexOf(shape), 1);
        frame.shapes.unshift(shape);
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        layer.children.splice(layer.children.indexOf(object), 1);
        layer.children.unshift(object);
      }
      updateUI();
    },
    rollback: (action) => {
      let frame = pointerList[action.frame];
      let layer = pointerList[action.layer];
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        frame.shapes.splice(frame.shapes.indexOf(shape), 1);
        frame.shapes.splice(action.formerIndices[shapeIdx], 0, shape);
      }
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
      let serializableShapes = [];
      let serializableObjects = [];
      let formerIndices = {};
      for (let shape of context.shapeselection) {
        serializableShapes.push(shape.idx);
        formerIndices[shape.idx] =
          context.activeObject.currentFrame.shapes.indexOf(shape);
      }
      for (let object of context.selection) {
        serializableObjects.push(object.idx);
        formerIndices[object.idx] =
          context.activeObject.activeLayer.children.indexOf(object);
      }
      let action = {
        shapes: serializableShapes,
        objects: serializableObjects,
        layer: context.activeObject.activeLayer.idx,
        frame: context.activeObject.currentFrame.idx,
        formerIndices: formerIndices,
      };
      undoStack.push({ name: "bringToFront", action: action });
      actions.bringToFront.execute(action);
      updateMenu();
    },
    execute: (action) => {
      let frame = pointerList[action.frame];
      let layer = pointerList[action.layer];
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        frame.shapes.splice(frame.shapes.indexOf(shape), 1);
        frame.shapes.push(shape);
      }
      for (let objectIdx of action.objects) {
        let object = pointerList[objectIdx];
        layer.children.splice(layer.children.indexOf(object), 1);
        layer.children.push(object);
      }
      updateUI();
    },
    rollback: (action) => {
      let frame = pointerList[action.frame];
      let layer = pointerList[action.layer];
      for (let shapeIdx of action.shapes) {
        let shape = pointerList[shapeIdx];
        frame.shapes.splice(frame.shapes.indexOf(shape), 1);
        frame.shapes.splice(action.formerIndices[shapeIdx], 0, shape);
      }
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
      for (let child of context.activeObject.activeLayer.children) {
        let idx = child.idx;
        if (idx in context.activeObject.currentFrame.keys) {
          selection.push(child.idx);
        }
      }
      for (let shape of context.activeObject.currentFrame.shapes) {
        shapeselection.push(shape.idx);
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
};

function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}
function vectorDist(a, b) {
  return Math.sqrt((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y));
}

function getMousePos(canvas, evt, skipOffsets = false, skipZoom = false) {
  var rect = canvas.getBoundingClientRect();
  let offsetX = canvas.offsetX || 0;
  let offsetY = canvas.offsetY || 0;
  let zoomLevel = canvas.zoomLevel || 1;
  if (skipOffsets) {
    offsetX = 0;
    offsetY = 0;
  }
  return {
    x: (evt.clientX + offsetX - rect.left) / (skipZoom ? 1 : zoomLevel),
    y: (evt.clientY + offsetY - rect.top) / (skipZoom ? 1 : zoomLevel),
  };
}

function getProperty(context, path) {
  let pointer = context;
  let pathComponents = path.split(".");
  for (let component of pathComponents) {
    pointer = pointer[component];
  }
  return pointer;
}

function setProperty(context, path, value) {
  let pointer = context;
  let pathComponents = path.split(".");
  let finalComponent = pathComponents.pop();
  for (let component of pathComponents) {
    pointer = pointer[component];
  }
  pointer[finalComponent] = value;
}

function selectCurve(context, mouse) {
  let mouseTolerance = 15;
  let closestDist = mouseTolerance;
  let closestCurve = undefined;
  let closestShape = undefined;
  for (let shape of context.activeObject.currentFrame.shapes) {
    if (shape instanceof TempShape) continue;
    if (
      mouse.x > shape.boundingBox.x.min - mouseTolerance &&
      mouse.x < shape.boundingBox.x.max + mouseTolerance &&
      mouse.y > shape.boundingBox.y.min - mouseTolerance &&
      mouse.y < shape.boundingBox.y.max + mouseTolerance
    ) {
      for (let curve of shape.curves) {
        let dist = vectorDist(mouse, curve.project(mouse));
        if (dist <= closestDist) {
          closestDist = dist;
          closestCurve = curve;
          closestShape = shape;
        }
      }
    }
  }
  if (closestCurve) {
    return { curve: closestCurve, shape: closestShape };
  } else {
    return undefined;
  }
}
function selectVertex(context, mouse) {
  let mouseTolerance = 15;
  let closestDist = mouseTolerance;
  let closestVertex = undefined;
  let closestShape = undefined;
  for (let shape of context.activeObject.currentFrame.shapes) {
    if (shape instanceof TempShape) continue;
    if (
      mouse.x > shape.boundingBox.x.min - mouseTolerance &&
      mouse.x < shape.boundingBox.x.max + mouseTolerance &&
      mouse.y > shape.boundingBox.y.min - mouseTolerance &&
      mouse.y < shape.boundingBox.y.max + mouseTolerance
    ) {
      for (let vertex of shape.vertices) {
        let dist = vectorDist(mouse, vertex.point);
        if (dist <= closestDist) {
          closestDist = dist;
          closestVertex = vertex;
          closestShape = shape;
        }
      }
    }
  }
  if (closestVertex) {
    return { vertex: closestVertex, shape: closestShape };
  } else {
    return undefined;
  }
}

function moldCurve(curve, mouse, oldMouse, epsilon = 0.01) {
  // Step 1: Find the closest point on the curve to the old mouse position
  const projection = curve.project(oldMouse);
  let t = projection.t;
  const P1 = curve.points[1];
  const P2 = curve.points[2];
  // Make copies of the control points to avoid editing the original curve
  const newP1 = { ...P1 };
  const newP2 = { ...P2 };

  // Step 2: Create new Bezier curves with the control points slightly offset
  const offsetP1 = { x: P1.x + epsilon, y: P1.y + epsilon };
  const offsetP2 = { x: P2.x + epsilon, y: P2.y + epsilon };
  const offsetCurveP1 = new Bezier(
    curve.points[0],
    offsetP1,
    curve.points[2],
    curve.points[3],
  );
  const offsetCurveP2 = new Bezier(
    curve.points[0],
    curve.points[1],
    offsetP2,
    curve.points[3],
  );

  // Step 3: See where the same point lands on the offset curves
  const offset1 = offsetCurveP1.compute(t);
  const offset2 = offsetCurveP2.compute(t);

  // Step 4: Calculate derivatives with respect to control points
  const derivativeP1 = {
    x: (offset1.x - projection.x) / epsilon,
    y: (offset1.y - projection.y) / epsilon,
  };
  const derivativeP2 = {
    x: (offset2.x - projection.x) / epsilon,
    y: (offset2.y - projection.y) / epsilon,
  };

  // Step 5: Use the derivatives to move the projected point to the mouse
  const deltaX = mouse.x - projection.x;
  const deltaY = mouse.y - projection.y;

  newP1.x = newP1.x + (deltaX / derivativeP1.x) * (1 - t * t);
  newP1.y = newP1.y + (deltaY / derivativeP1.y) * (1 - t * t);
  newP2.x = newP2.x + (deltaX / derivativeP2.x) * t * t;
  newP2.y = newP2.y + (deltaY / derivativeP2.y) * t * t;

  // Return the updated Bezier curve
  return new Bezier(curve.points[0], newP1, newP2, curve.points[3]);
}

function deriveControlPoints(S, A, E, e1, e2, t) {
  // Deriving the control points is effectively "doing what
  // we talk about in the section", in code:

  const v1 = {
    x: A.x - (A.x - e1.x) / (1 - t),
    y: A.y - (A.y - e1.y) / (1 - t),
  };
  const v2 = {
    x: A.x - (A.x - e2.x) / t,
    y: A.y - (A.y - e2.y) / t,
  };

  const C1 = {
    x: S.x + (v1.x - S.x) / t,
    y: S.y + (v1.y - S.y) / t,
  };
  const C2 = {
    x: E.x + (v2.x - E.x) / (1 - t),
    y: E.y + (v2.y - E.y) / (1 - t),
  };

  return { v1, v2, C1, C2 };
}

function regionToBbox(region) {
  return {
    x: {
      min: Math.min(region.x1, region.x2),
      max: Math.max(region.x1, region.x2),
    },
    y: {
      min: Math.min(region.y1, region.y2),
      max: Math.max(region.y1, region.y2),
    },
  };
}

function hitTest(candidate, object) {
  let bbox = object.bbox();
  if (candidate.x.min) {
    // We're checking a bounding box
    if (
      candidate.x.min < bbox.x.max &&
      candidate.x.max > bbox.x.min &&
      candidate.y.min < bbox.y.max &&
      candidate.y.max > bbox.y.min
    ) {
      return true;
    } else {
      return false;
    }
  } else {
    // We're checking a point
    if (
      candidate.x > bbox.x.min &&
      candidate.x < bbox.x.max &&
      candidate.y > bbox.y.min &&
      candidate.y < bbox.y.max
    ) {
      return true;
    } else {
      return false;
    }
  }
}

function undo() {
  let action = undoStack.pop();
  if (action) {
    actions[action.name].rollback(action.action);
    redoStack.push(action);
    updateUI();
    updateMenu();
  } else {
    console.log("No actions to undo");
    updateMenu();
  }
}

function redo() {
  let action = redoStack.pop();
  if (action) {
    actions[action.name].execute(action.action);
    undoStack.push(action);
    updateUI();
    updateMenu();
  } else {
    console.log("No actions to redo");
    updateMenu();
  }
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
  static fromJSON(json) {
    if (!json) {
      return undefined
    }
    const frame = new Frame(json.frameType, json.idx);
    frame.keyTypes = new Set(json.keyTypes)
    frame.keys = json.keys;
    for (let i in json.shapes) {
      const shape = json.shapes[i];
      frame.shapes.push(Shape.fromJSON(shape));
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

class Layer extends Widget {
  constructor(uuid) {
    super(0,0)
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    this.name = "Layer";
    this.frames = [new Frame("keyframe", this.idx + "-F1")];
    this.frameNum = 0;
    this.visible = true;
    this.audible = true;
    pointerList[this.idx] = this;
  }
  static fromJSON(json) {
    const layer = new Layer(json.idx);
    for (let i in json.children) {
      const child = json.children[i];
      layer.children.push(GraphicsObject.fromJSON(child));
    }
    layer.name = json.name;
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
    // for (let frame in layer.frames) {
    //   if (layer.frames[frame]) {
    //     if (["motion", "shape"].indexOf(layer.frames[frame].frameType) != -1) {
    //       layer.updateFrameNextAndPrev(frame, layer.frames[frame].frameType);
    //     }
    //   }
    // }
    layer.visible = json.visible;
    layer.audible = json.audible;

    return layer;
  }
  toJSON(randomizeUuid = false) {
    const json = {};
    json.type = "Layer";
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
      // TODO: we may not need the randomizeUuid parameter anymore
      let childJson = child.toJSON(randomizeUuid)
      idMap[child.idx] = childJson.idx
      json.children.push(childJson);
    }
    json.frames = [];
    for (let frame of this.frames) {
      if (frame) {
        let frameJson = frame.toJSON(randomizeUuid)
        for (let key in frameJson.keys) {
          if (key in idMap) {
            frameJson.keys[idMap[key]] = frameJson.keys[key]
            // delete frameJson.keys[key]
          }
        }
        json.frames.push(frameJson);
      } else {
        json.frames.push(undefined)
      }
    }
    json.visible = this.visible;
    json.audible = this.audible;
    return json;
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
    let newLayer = new Layer(idx.slice(0, 8) + this.idx.slice(8));
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

  draw(ctx) {
    // super.draw(ctx)
    if (!this.visible) return;
    let frameInfo = this.getFrameValue(this.frameNum);
    let frame = frameInfo.valueAtN !== undefined ? frameInfo.valueAtN : frameInfo.prev;
    const keyframe = frameInfo.valueAtN ? true : false

    // let frame = this.getFrame(this.currentFrameNum);
    let cxt = {...context}
    cxt.ctx = ctx
    let t = null;
    if (frameInfo.prev && frameInfo.next) {
      t = (this.frameNum - frameInfo.prevIndex) / (frameInfo.nextIndex - frameInfo.prevIndex);
    }

    if (frame) {
      // Update shapes and children
      for (let shape of frame.shapes) {
        // If prev.frameType is "shape", look for a matching shape in next
        if (frameInfo.prev && frameInfo.prev.keyTypes.has("shape")) {
          const shape2 = frameInfo.next.shapes.find(s => s.shapeId === shape.shapeId);
          
          if (shape2) {
            // If matching shape is found, interpolate and draw
            shape.lerpShape(shape2, t).draw(cxt);
            continue; // Skip to next shape
          }
        }

        // Otherwise, just draw the shape as usual
        cxt.selected = context.shapeselection.includes(shape);
        shape.draw(cxt);
      }

      for (let child of this.children) {
        if (child.idx in frame.keys) {
          for (let key in frame.keys[child.idx]) {
            // If both prev and next exist and prev.frameType is "motion", interpolate child keys
            if (frameInfo.prev && frameInfo.next && frameInfo.prev.keyTypes.has("motion")) {
              // Interpolate between prev and next for this key
              child[key] = lerp(frameInfo.prev.keys[child.idx][key], frameInfo.next.keys[child.idx][key], t);
            } else {
              // Otherwise, use the value from the current frame
              child[key] = frame.keys[child.idx][key];
            }
          }
          if (!context.objectStack.includes(child)) {
            if (keyframe) {
              if (child.goToFrame != undefined) {
                child.setFrameNum(child.goToFrame - 1)
                if (child.playFromFrame) {
                  child.playing = true
                } else {
                  child.playing = false
                }
                child.playing = true
              }
            }
            if (child.playing) {
              let lastFrame = 0;
              for (let i = this.frameNum; i >= 0; i--) {
                if (
                  this.frames[i] &&
                  this.frames[i].keys[child.idx] &&
                  this.frames[i].keys[child.idx].playFromFrame
                ) {
                  lastFrame = i;
                  break;
                }
              }
              child.setFrameNum(this.frameNum - lastFrame);
            }
          }
          const transform = ctx.getTransform()
          ctx.translate(child.x, child.y)
          ctx.scale(child.scale_x, child.scale_y)
          ctx.rotate(child.rotation)
          child.draw(ctx)
          if (context.selection.includes(child)) {
            ctx.lineWidth = 1;
            ctx.strokeStyle = "#00ffff";
            ctx.beginPath();
            let bbox = child.bbox()
            ctx.rect(bbox.x.min-child.x, bbox.y.min-child.y, bbox.x.max-bbox.x.min, bbox.y.max-bbox.y.min)
            ctx.stroke()
          }
          ctx.setTransform(transform)
        }
      }
      if (this.activeShape) {
        this.activeShape.draw(cxt)
      }
      if (context.activeCurve) {
        if (frame.shapes.indexOf(context.activeCurve.shape) != -1) {
          cxt.selected = true

        }
      }
    }
  }
  bbox() {
    let bbox = super.bbox()
    const frameInfo = this.getFrameValue(this.frameNum);
    // TODO: if there are multiple layers we should only be counting valid frames
    const frame = frameInfo.valueAtN ? frameInfo.valueAtN : frameInfo.prev
    if (!frame) return bbox;
    if (frame.shapes.length > 0 && bbox == undefined) {
      bbox = structuredClone(frame.shapes[0].boundingBox);
    }
    for (let shape of frame.shapes) {
      growBoundingBox(bbox, shape.boundingBox);
    }
    return bbox
  }
  mousedown(x, y) {
    const mouse = {x: x, y: y}
    if (this==context.activeLayer) {
      switch(mode) {
        case "rectangle":
        case "ellipse":
        case "draw":
          this.clicked = true
          this.activeShape = new Shape(x, y, context, uuidv4())
          this.lastMouse = mouse;
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
          // TODO: get this from self
          let pointShape = getShapeAtPoint(
            mouse,
            context.activeObject.currentFrame.shapes,
          );

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
          let shape = new Shape(regionPoints[0].x, regionPoints[0].y, cxt);
          shape.fromPoints(points, 1);
          actions.addShape.create(context.activeObject, shape, cxt);
          break;
      }
    }
  }
  mousemove(x, y) {
    const mouse = {x: x, y: y}
    if (this==context.activeLayer) {
      switch (mode) {
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
    this.clicked = false
    if (this==context.activeLayer) {
      switch (mode) {
        case "draw":
          if (this.activeShape) {
            this.activeShape.addLine(x, y);
            this.activeShape.simplify(context.simplifyMode);
          }
        case "rectangle":
        case "ellipse":
          if (this.activeShape) {
            actions.addShape.create(context.activeObject, this.activeShape);
            this.activeShape = undefined;
          }
          break;
      }
    }
  }
}

class AudioLayer {
  constructor(uuid, name) {
    this.sounds = {};
    this.track = new Tone.Part((time, sound) => {
      console.log(this.sounds[sound]);
      this.sounds[sound].player.start(time);
    });
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    if (!name) {
      this.name = "Audio";
    } else {
      this.name = name;
    }
    this.audible = true;
  }
  static fromJSON(json) {
    const audioLayer = new AudioLayer(json.idx, json.name);
    // TODO: load audiolayer from json
    audioLayer.sounds = {};
    for (let id in json.sounds) {
      const jsonSound = json.sounds[id]
      const img = new Image();
      img.className = "audioWaveform";
      const player = new Tone.Player().toDestination();
      player.load(jsonSound.src)
      .then(() => {
        generateWaveform(img, player.buffer, 50, 25, config.framerate);
      })
      .catch(error => {
        // Handle any errors that occur during the load or waveform generation
        console.error(error);
      });

      let soundObj = {
        player: player,
        start: jsonSound.start,
        img: img,
        src: jsonSound.src,
        uuid: jsonSound.uuid
      };
      pointerList[jsonSound.uuid] = soundObj;
      audioLayer.sounds[jsonSound.uuid] = soundObj;
      // TODO: change start time
      audioLayer.track.add(0, jsonSound.uuid);
    }
    audioLayer.audible = json.audible;
    return audioLayer;
  }
  toJSON(randomizeUuid = false) {
    console.log(this.sounds)
    const json = {};
    json.type = "AudioLayer";
    // TODO: build json from audiolayer
    json.sounds = {};
    for (let id in this.sounds) {
      const sound = this.sounds[id]
      json.sounds[id] = {
        start: sound.start,
        src: sound.src,
        uuid: sound.uuid
      }
    }
    json.audible = this.audible;
    if (randomizeUuid) {
      json.idx = uuidv4();
      json.name = this.name + " copy";
    } else {
      json.idx = this.idx;
      json.name = this.name;
    }
    return json;
  }
  copy(idx) {
    let newAudioLayer = new AudioLayer(
      idx.slice(0, 8) + this.idx.slice(8),
      this.name,
    );
    for (let soundIdx in this.sounds) {
      let sound = this.sounds[soundIdx];
      let newPlayer = new Tone.Player(sound.buffer()).toDestination();
      let idx = this.idx.slice(0, 8) + soundIdx.slice(8);
      let soundObj = {
        player: newPlayer,
        start: sound.start,
      };
      pointerList[idx] = soundObj;
      newAudioLayer.sounds[idx] = soundObj;
    }
  }
}

class BaseShape {
  constructor(startx, starty) {
    this.startx = startx;
    this.starty = starty;
    this.curves = [];
    this.regions = [];
    this.boundingBox = {
      x: { min: startx, max: starty },
      y: { min: starty, max: starty },
    };
  }
  recalculateBoundingBox() {
    this.boundingBox = undefined;
    for (let curve of this.curves) {
      if (!this.boundingBox) {
        this.boundingBox = curve.bbox();
      }
      growBoundingBox(this.boundingBox, curve.bbox());
    }
  }
  draw(context) {
    let ctx = context.ctx;
    ctx.lineWidth = this.lineWidth;
    ctx.lineCap = "round";

    // Create a repeating pattern for indicating selected shapes
    if (!this.patternCanvas) {
      this.patternCanvas = document.createElement('canvas');
      this.patternCanvas.width = 2;
      this.patternCanvas.height = 2;
      let patternCtx = this.patternCanvas.getContext('2d');
      // Draw the pattern:
      // black,       transparent,
      // transparent, white
      patternCtx.fillStyle = 'black';
      patternCtx.fillRect(0, 0, 1, 1);
      patternCtx.clearRect(1, 0, 1, 1);
      patternCtx.clearRect(0, 1, 1, 1);
      patternCtx.fillStyle = 'white';
      patternCtx.fillRect(1, 1, 1, 1);
    }
    let pattern = ctx.createPattern(this.patternCanvas, 'repeat'); // repeat the pattern across the canvas

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
      ctx.beginPath();
      if (this.fillImage && this.fillImage instanceof Element) {
        let pat;
        if (this.fillImage instanceof Element ||
          Object.keys(this.fillImage).length !== 0) {
          pat = ctx.createPattern(this.fillImage, "no-repeat");
        } else {
          pat = createMissingTexturePattern(ctx)
        }
        ctx.fillStyle = pat;
      } else {
        ctx.fillStyle = this.fillStyle;
      }
      if (context.debugColor) {
        ctx.fillStyle = context.debugColor;
      }
      if (this.curves.length > 0) {
        ctx.moveTo(this.curves[0].points[0].x, this.curves[0].points[0].y);
        for (let curve of this.curves) {
          ctx.bezierCurveTo(
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
            curve.points[3].x,
            curve.points[3].y,
          );
        }
      }
      ctx.fill();
      if (context.selected) {
        ctx.fillStyle = pattern
        ctx.fill()
      }
    }
    function drawCurve(curve, selected) {
      ctx.strokeStyle = curve.color;
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
      if (selected) {
        ctx.strokeStyle = pattern
        ctx.stroke()
      }
    }
    if (this.stroked && !context.debugColor) {
      for (let curve of this.curves) {
        drawCurve(curve, context.selected)

        // // Debug, show curve control points
        // ctx.beginPath()
        // ctx.arc(curve.points[1].x,curve.points[1].y, 5, 0, 2*Math.PI)
        // ctx.arc(curve.points[2].x,curve.points[2].y, 5, 0, 2*Math.PI)
        // ctx.arc(curve.points[3].x,curve.points[3].y, 5, 0, 2*Math.PI)
        // ctx.fill()
      }
    }
    if (context.activeCurve && this==context.activeCurve.shape) {
      drawCurve(context.activeCurve.current, true)
    }
    if (context.activeVertex && this==context.activeVertex.shape) {
      const curves = {
        ...context.activeVertex.current.startCurves,
        ...context.activeVertex.current.endCurves
      }
      for (let i in curves) {
        let curve = curves[i]
        drawCurve(curve, true)
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
    }
    // Debug, show quadtree
    if (debugQuadtree && this.quadtree && !context.debugColor) {
      this.quadtree.draw(ctx);
    }
  }
  lerpShape(shape2, t) {
    if (this.curves.length == 0) return this;
    let path1 = [
      {
        type: "M",
        x: this.curves[0].points[0].x,
        y: this.curves[0].points[0].y,
      },
    ];
    for (let curve of this.curves) {
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
    let bezier;
    for (let curve of current) {
      bezier = new Bezier(
        x,
        y,
        curve.x1,
        curve.y1,
        curve.x2,
        curve.y2,
        curve.x,
        curve.y,
      )
      bezier.color = lerpColor(this.strokeStyle, shape2.strokeStyle)
      curves.push(bezier);
      x = curve.x;
      y = curve.y;
    }
    let lineWidth = lerp(this.lineWidth, shape2.lineWidth, t);
    let strokeStyle = lerpColor(
      this.strokeStyle,
      shape2.strokeStyle,
      t,
    );
    let fillStyle;
    if (!this.fillImage) {
      fillStyle = lerpColor(this.fillStyle, shape2.fillStyle, t);
    }
    return new TempShape(
      start.x,
      start.y,
      curves,
      lineWidth,
      this.stroked,
      this.filled,
      strokeStyle,
      fillStyle,
    )
  }
}

class TempShape extends BaseShape {
  constructor(
    startx,
    starty,
    curves,
    lineWidth,
    stroked,
    filled,
    strokeStyle,
    fillStyle,
  ) {
    super(startx, starty);
    this.curves = curves;
    this.lineWidth = lineWidth;
    this.stroked = stroked;
    this.filled = filled;
    this.strokeStyle = strokeStyle;
    this.fillStyle = fillStyle;
    this.inProgress = false;
    this.recalculateBoundingBox();
  }
}

class Shape extends BaseShape {
  constructor(startx, starty, context, uuid = undefined, shapeId = undefined) {
    super(startx, starty);
    this.vertices = [];
    this.triangles = [];
    this.fillStyle = context.fillStyle;
    this.fillImage = context.fillImage;
    this.strokeStyle = context.strokeStyle;
    this.lineWidth = context.lineWidth;
    this.filled = context.fillShape;
    this.stroked = context.strokeShape;
    this.quadtree = new Quadtree(
      { x: { min: 0, max: 500 }, y: { min: 0, max: 500 } },
      4,
    );
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    if (!shapeId) {
      this.shapeId = uuidv4();
    } else {
      this.shapeId = shapeId;
    }
    pointerList[this.idx] = this;
    this.regionIdx = 0;
    this.inProgress = true;
  }
  static fromJSON(json) {
    let fillImage = undefined;
    if (json.fillImage && Object.keys(json.fillImage).length !== 0) {
      let img = new Image();
      img.src = json.fillImage.src
      fillImage = img
    } else {
      fillImage = {}
    }
    const shape = new Shape(
      json.startx,
      json.starty,
      {
        fillStyle: json.fillStyle,
        fillImage: fillImage,
        strokeStyle: json.strokeStyle,
        lineWidth: json.lineWidth,
        fillShape: json.filled,
        strokeShape: json.stroked,
      },
      json.idx,
      json.shapeId,
    );
    for (let curve of json.curves) {
      shape.addCurve(Bezier.fromJSON(curve));
    }
    for (let region of json.regions) {
      const curves = [];
      for (let curve of region.curves) {
        curves.push(Bezier.fromJSON(curve));
      }
      shape.regions.push({
        idx: region.idx,
        curves: curves,
        fillStyle: region.fillStyle,
        filled: region.filled,
      });
    }
    return shape;
  }
  toJSON(randomizeUuid = false) {
    const json = {};
    json.type = "Shape";
    json.startx = this.startx;
    json.starty = this.starty;
    json.fillStyle = this.fillStyle;
    if (this.fillImage instanceof Element) {
      json.fillImage = {
        src: this.fillImage.src
      }
    }
    json.strokeStyle = this.fillStyle;
    json.lineWidth = this.lineWidth;
    json.filled = this.filled;
    json.stroked = this.stroked;
    if (randomizeUuid) {
      json.idx = uuidv4();
    } else {
      json.idx = this.idx;
    }
    json.shapeId = this.shapeId;
    json.curves = [];
    for (let curve of this.curves) {
      json.curves.push(curve.toJSON(randomizeUuid));
    }
    json.regions = [];
    for (let region of this.regions) {
      const curves = [];
      for (let curve of region.curves) {
        curves.push(curve.toJSON(randomizeUuid));
      }
      json.regions.push({
        idx: region.idx,
        curves: curves,
        fillStyle: region.fillStyle,
        filled: region.filled,
      });
    }
    return json;
  }
  addCurve(curve) {
    if (curve.color == undefined) {
      curve.color = context.strokeStyle;
    }
    this.curves.push(curve);
    this.quadtree.insert(curve, this.curves.length - 1);
    growBoundingBox(this.boundingBox, curve.bbox());
  }
  addLine(x, y) {
    let lastpoint;
    if (this.curves.length) {
      lastpoint = this.curves[this.curves.length - 1].points[3];
    } else {
      lastpoint = { x: this.startx, y: this.starty };
    }
    let midpoint = { x: (x + lastpoint.x) / 2, y: (y + lastpoint.y) / 2 };
    let curve = new Bezier(
      lastpoint.x,
      lastpoint.y,
      midpoint.x,
      midpoint.y,
      midpoint.x,
      midpoint.y,
      x,
      y,
    );
    curve.color = context.strokeStyle;
    this.quadtree.insert(curve, this.curves.length - 1);
    this.curves.push(curve);
  }
  bbox() {
    return this.boundingBox;
  }
  clear() {
    this.curves = [];
    this.quadtree.clear();
  }
  copy(idx) {
    let newShape = new Shape(
      this.startx,
      this.starty,
      {},
      idx.slice(0, 8) + this.idx.slice(8),
      this.shapeId,
    );
    newShape.startx = this.startx;
    newShape.starty = this.starty;
    for (let curve of this.curves) {
      let newCurve = new Bezier(
        curve.points[0].x,
        curve.points[0].y,
        curve.points[1].x,
        curve.points[1].y,
        curve.points[2].x,
        curve.points[2].y,
        curve.points[3].x,
        curve.points[3].y,
      );
      newCurve.color = curve.color;
      newShape.addCurve(newCurve);
    }
    // TODO
    // for (let vertex of this.vertices) {

    // }
    newShape.updateVertices();
    newShape.fillStyle = this.fillStyle;
    if (this.fillImage instanceof Element) {
      newShape.fillImage = this.fillImage.cloneNode(true)
    } else {
      newShape.fillImage = this.fillImage;
    }
    newShape.strokeStyle = this.strokeStyle;
    newShape.lineWidth = this.lineWidth;
    newShape.filled = this.filled;
    newShape.stroked = this.stroked;

    return newShape;
  }
  fromPoints(points, error = 30) {
    console.log(error);
    this.curves = [];
    let curves = fitCurve.fitCurve(points, error);
    for (let curve of curves) {
      let bezier = new Bezier(
        curve[0][0],
        curve[0][1],
        curve[1][0],
        curve[1][1],
        curve[2][0],
        curve[2][1],
        curve[3][0],
        curve[3][1],
      );
      this.curves.push(bezier);
      this.quadtree.insert(bezier, this.curves.length - 1);
    }
    return this;
  }
  simplify(mode = "corners") {
    this.quadtree.clear();
    this.inProgress = false;
    // Mode can be corners, smooth or auto
    if (mode == "corners") {
      let points = [{ x: this.startx, y: this.starty }];
      for (let curve of this.curves) {
        points.push(curve.points[3]);
      }
      // points = points.concat(this.curves)
      let newpoints = simplifyPolyline(points, 10, false);
      this.curves = [];
      let lastpoint = newpoints.shift();
      let midpoint;
      for (let point of newpoints) {
        midpoint = {
          x: (lastpoint.x + point.x) / 2,
          y: (lastpoint.y + point.y) / 2,
        };
        let bezier = new Bezier(
          lastpoint.x,
          lastpoint.y,
          midpoint.x,
          midpoint.y,
          midpoint.x,
          midpoint.y,
          point.x,
          point.y,
        );
        this.curves.push(bezier);
        this.quadtree.insert(bezier, this.curves.length - 1);
        lastpoint = point;
      }
    } else if (mode == "smooth") {
      let error = 30;
      let points = [[this.startx, this.starty]];
      for (let curve of this.curves) {
        points.push([curve.points[3].x, curve.points[3].y]);
      }
      this.fromPoints(points, error);
    } else if (mode == "verbatim") {
      // Just keep existing shape
    }
    let epsilon = 0.01;
    let newCurves = [];
    let intersectMap = {};
    for (let i = 0; i < this.curves.length - 1; i++) {
      // for (let j=i+1; j<this.curves.length; j++) {
      for (let j of this.quadtree.query(this.curves[i].bbox())) {
        if (i >= j) continue;
        let intersects = this.curves[i].intersects(this.curves[j]);
        if (intersects.length) {
          intersectMap[i] ||= [];
          intersectMap[j] ||= [];
          for (let intersect of intersects) {
            let [t1, t2] = intersect.split("/");
            intersectMap[i].push(parseFloat(t1));
            intersectMap[j].push(parseFloat(t2));
          }
        }
      }
    }
    for (let lst in intersectMap) {
      for (let i = 1; i < intersectMap[lst].length; i++) {
        if (
          Math.abs(intersectMap[lst][i] - intersectMap[lst][i - 1]) < epsilon
        ) {
          intersectMap[lst].splice(i, 1);
          i--;
        }
      }
    }
    for (let i = this.curves.length - 1; i >= 0; i--) {
      if (i in intersectMap) {
        intersectMap[i].sort().reverse();
        let remainingFraction = 1;
        let remainingCurve = this.curves[i];
        for (let t of intersectMap[i]) {
          let split = remainingCurve.split(t / remainingFraction);
          remainingFraction = t;
          newCurves.push(split.right);
          remainingCurve = split.left;
        }
        newCurves.push(remainingCurve);
      } else {
        newCurves.push(this.curves[i]);
      }
    }
    for (let curve of newCurves) {
      curve.color = context.strokeStyle;
    }
    newCurves.reverse();
    this.curves = newCurves;
  }
  update() {
    this.recalculateBoundingBox();
    this.updateVertices();
    if (this.curves.length) {
      this.startx = this.curves[0].points[0].x;
      this.starty = this.curves[0].points[0].y;
    }
    return [this];
  }
  getClockwiseCurves(point, otherPoints) {
    // Returns array of {x, y, idx, angle}

    let points = [];
    for (let point of otherPoints) {
      points.push({ ...this.vertices[point].point, idx: point });
    }
    // Add an angle property to each point using tan(angle) = y/x
    const angles = points.map(({ x, y, idx }) => {
      return {
        x,
        y,
        idx,
        angle: (Math.atan2(y - point.y, x - point.x) * 180) / Math.PI,
      };
    });
    // Sort your points by angle
    const pointsSorted = angles.sort((a, b) => a.angle - b.angle);
    return pointsSorted;
  }
  translate(x, y) {
    this.quadtree.clear()
    let j=0;
    for (let curve of this.curves) {
      for (let i in curve.points) {
        const point = curve.points[i];
        curve.points[i] = { x: point.x + x, y: point.y + y };
      }
      this.quadtree.insert(curve, j)
      j++;
    }
    this.update();
  }
  updateVertices() {
    this.vertices = [];
    let utils = Bezier.getUtils();
    let epsilon = 1.5; // big epsilon whoa
    let tooClose;
    let i = 0;

    let region = {
      idx: `${this.idx}-r${this.regionIdx++}`,
      curves: [],
      fillStyle: context.fillStyle,
      filled: context.fillShape,
    };
    pointerList[region.idx] = region;
    this.regions = [region];
    for (let curve of this.curves) {
      this.regions[0].curves.push(curve);
    }
    if (this.regions[0].curves.length) {
      if (
        utils.dist(
          this.regions[0].curves[0].points[0],
          this.regions[0].curves[this.regions[0].curves.length - 1].points[3],
        ) < epsilon
      ) {
        this.regions[0].filled = true;
      }
    }

    // Generate vertices
    for (let curve of this.curves) {
      for (let index of [0, 3]) {
        tooClose = false;
        for (let vertex of this.vertices) {
          if (utils.dist(curve.points[index], vertex.point) < epsilon) {
            tooClose = true;
            vertex[["startCurves", , , "endCurves"][index]][i] = curve;
            break;
          }
        }
        if (!tooClose) {
          if (index == 0) {
            this.vertices.push({
              point: curve.points[index],
              startCurves: { [i]: curve },
              endCurves: {},
            });
          } else {
            this.vertices.push({
              point: curve.points[index],
              startCurves: {},
              endCurves: { [i]: curve },
            });
          }
        }
      }
      i++;
    }

    let shapes = [this];
    this.vertices.forEach((vertex, i) => {
      for (let i = 0; i < Math.min(10, this.regions.length); i++) {
        let region = this.regions[i];
        let regionVertexCurves = [];
        let vertexCurves = { ...vertex.startCurves, ...vertex.endCurves };
        if (Object.keys(vertexCurves).length == 1) {
          // endpoint
          continue;
        } else if (Object.keys(vertexCurves).length == 2) {
          // path vertex, don't need to do anything
          continue;
        } else if (Object.keys(vertexCurves).length == 3) {
          // T junction. Region doesn't change but might need to update curves?
          // Skip for now.
          continue;
        } else if (Object.keys(vertexCurves).length == 4) {
          // Intersection, split region in 2
          for (let i in vertexCurves) {
            let curve = vertexCurves[i];
            if (region.curves.includes(curve)) {
              regionVertexCurves.push(curve);
            }
          }
          let start = region.curves.indexOf(regionVertexCurves[1]);
          let end = region.curves.indexOf(regionVertexCurves[3]);
          if (end > start) {
            let newRegion = {
              idx: `${this.idx}-r${this.regionIdx++}`, // TODO: generate this deterministically so that undo/redo works
              curves: region.curves.splice(start, end - start),
              fillStyle: region.fillStyle,
              filled: true,
            };
            pointerList[newRegion.idx] = newRegion;
            this.regions.push(newRegion);
          }
        } else {
          // not sure how to handle vertices with more than 4 curves
          console.log(
            `Unexpected vertex with ${Object.keys(vertexCurves).length} curves!`,
          );
        }
      }
    });
  }
}

class GraphicsObject extends Widget {
  constructor(uuid) {
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

    this.currentFrameNum = 0;
    this.currentLayer = 0;
    this.children = [new Layer(uuid + "-L1")];
    // this.layers = [new Layer(uuid + "-L1")];
    this.audioLayers = [];
    // this.children = []

    this.shapes = [];

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
      graphicsObject.layers.push(Layer.fromJSON(layer));
    }
    for (let audioLayer of json.audioLayers) {
      graphicsObject.audioLayers.push(AudioLayer.fromJSON(audioLayer));
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
    json.audioLayers = [];
    for (let audioLayer of this.audioLayers) {
      json.audioLayers.push(audioLayer.toJSON(randomizeUuid));
    }
    return json;
  }
  get activeLayer() {
    return this.layers[this.currentLayer];
  }
  // get children() {
  //   return this.activeLayer.children;
  // }
  get layers() {
    return this.children
  }
  get allLayers() {
    return [...this.audioLayers, ...this.layers];
  }
  get currentFrame() {
    return this.getFrame(this.currentFrameNum);
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
  advanceFrame() {
    this.setFrameNum(this.currentFrameNum + 1);
  }
  decrementFrame() {
    this.setFrameNum(this.currentFrameNum - 1);
  }
  getFrame(num) {
    return this.activeLayer?.getFrame(num);
  }
  setFrameNum(num) {
    num = Math.max(0, num);
    for (let layer of this.layers) {
      this.currentFrameNum = num;
      layer.frameNum = num
      let frame = layer.getFrame(num);
      for (let child of this.children) {
        let idx = child.idx;
        if (idx in frame.keys) {
          child.x = frame.keys[idx].x;
          child.y = frame.keys[idx].y;
          child.rotation = frame.keys[idx].rotation;
          child.scale_x = frame.keys[idx].scale_x;
          child.scale_y = frame.keys[idx].scale_y;
          child.playFromFrame = frame.keys[idx].playFromFrame;
          if (
            frame.frameType == "keyframe" &&
            frame.keys[idx].goToFrame != undefined
          ) {
            // Frames are 1-indexed
            child.setFrameNum(frame.keys[idx].goToFrame - 1);
            if (child.playFromFrame) {
              child.playing = true;
            } else {
              child.playing = false;
            }
          } else if (child.playing) {
            let lastFrame = 0;
            for (let i = num; i >= 0; i--) {
              if (
                layer.frames[i].frameType == "keyframe" &&
                layer.frames[i].keys[idx].playFromFrame
              ) {
                lastFrame = i;
                break;
              }
            }
            child.setFrameNum(num - lastFrame);
          }
        }
      }
    }
  }
  bbox() {
    let bbox;
    // for (let layer of this.layers) {
    //   let frame = layer.getFrame(this.currentFrameNum);
    //   if (frame.shapes.length > 0 && bbox == undefined) {
    //     bbox = structuredClone(frame.shapes[0].boundingBox);
    //   }
    //   for (let shape of frame.shapes) {
    //     growBoundingBox(bbox, shape.boundingBox);
    //   }
    // }
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
  /*
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
      let frame = layer.getFrame(this.currentFrameNum);
      for (let shape of frame.shapes) {
        let cxt = {...context}
        if (context.shapeselection.indexOf(shape) >= 0) {
          cxt.selected = true
        }
        shape.draw(cxt);
      }
      for (let child of layer.children) {
        if (child == context.activeObject) continue;
        let idx = child.idx;
        if (idx in frame.keys) {
          child.x = frame.keys[idx].x;
          child.y = frame.keys[idx].y;
          child.rotation = frame.keys[idx].rotation;
          child.scale_x = frame.keys[idx].scale_x;
          child.scale_y = frame.keys[idx].scale_y;
          ctx.save();
          child.draw(context);
          if (true) {
          }
          ctx.restore();
        }
      }
    }
    if (this == context.activeObject) {
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
  */
  draw(ctx) {
    super.draw(ctx)
    if (this==context.activeObject) {
      if (mode == "select") {
        for (let item of context.selection) {
          if (!item) continue;
          if (item.idx in this.currentFrame.keys) {
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
      } else if (mode == "transform") {
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
  addObject(object, x = 0, y = 0, frame = undefined, layer=undefined) {
    if (frame == undefined) {
      frame = this.currentFrame;
    }
    if (layer==undefined) {
      layer = this.activeLayer
    }
    // this.children.push(object);
    layer.children.push(object)
    object.parent = this;
    object.x = x;
    object.y = y;
    let idx = object.idx;
    frame.keys[idx] = {
      x: x,
      y: y,
      rotation: 0,
      scale_x: 1,
      scale_y: 1,
      goToFrame: 1,
    };
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
    for (let audioLayer of this.audioLayers) {
      newGO.audioLayers.push(audioLayer.copy(idx));
    }

    return newGO;
  }
}

let root = new GraphicsObject("root");
Object.defineProperty(context, "activeObject", {
  get: function () {
    return this.objectStack.at(-1);
  },
});
Object.defineProperty(context, "activeLayer", {
  get: function () {
    return this.objectStack.at(-1).activeLayer
  }
})
context.objectStack = [root];

async function greet() {
  // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
  greetMsgEl.textContent = await invoke("greet", { name: greetInputEl.value });
}

window.addEventListener("DOMContentLoaded", () => {
  rootPane = document.querySelector("#root");
  rootPane.appendChild(createPane(panes.toolbar));
  rootPane.addEventListener("pointermove", (e) => {
    mouseEvent = e;
  });
  let [_toolbar, panel] = splitPane(
    rootPane,
    10,
    true,
    createPane(panes.timeline),
  );
  let [stageAndTimeline, _infopanel] = splitPane(
    panel,
    70,
    false,
    createPane(panes.infopanel),
  );
  let [_timeline, _stage] = splitPane(
    stageAndTimeline,
    30,
    false,
    createPane(panes.stage),
  );
});

window.addEventListener("resize", () => {
  updateAll();
});

window.addEventListener("click", function (event) {
  const popupMenu = document.getElementById("popupMenu");

  // If the menu exists and the click is outside the menu and any button with the class 'paneButton', remove the menu
  if (
    popupMenu &&
    !popupMenu.contains(event.target) &&
    !event.target.classList.contains("paneButton")
  ) {
    popupMenu.remove(); // Remove the menu from the DOM
  }
});

window.addEventListener("contextmenu", async (e) => {
  e.preventDefault()
  // const menu = await Menu.new({
  //   items: [
  //   ],
  // });
  // menu.popup({ x: event.clientX, y: event.clientY });
})

window.addEventListener("keydown", (e) => {
  // let shortcuts = {}
  // for (let shortcut of config.shortcuts) {
  // shortcut = shortcut.split("+")
  // TODO
  // }
  if (
    e.target.tagName === "INPUT" ||
    e.target.tagName === "TEXTAREA" ||
    e.target.isContentEditable
  ) {
    return; // Do nothing if the event target is an input field, textarea, or contenteditable element
  }
  // console.log(e);
  let mod = macOS ? e.metaKey : e.ctrlKey;
  let key = (mod ? "<mod>" : "") + e.key;
  switch (key) {
    case config.shortcuts.playAnimation:
      console.log("Spacebar pressed");
      playPause();
      break;
    case config.shortcuts.selectAll:
      e.preventDefault();
      break;
    // TODO: put these in shortcuts
    case "<mod>ArrowRight":
      advanceFrame();
      e.preventDefault();
      break;
    case "ArrowRight":
      if (context.selection) {
        context.activeObject.currentFrame.saveState();
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].x += 1;
        }
        actions.editFrame.create(context.activeObject.currentFrame);
        updateUI();
      }
      e.preventDefault();
      break;
    case "<mod>ArrowLeft":
      decrementFrame();
      break;
    case "ArrowLeft":
      if (context.selection) {
        context.activeObject.currentFrame.saveState();
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].x -= 1;
        }
        actions.editFrame.create(context.activeObject.currentFrame);
        updateUI();
      }
      e.preventDefault();
      break;
    case "ArrowUp":
      if (context.selection) {
        context.activeObject.currentFrame.saveState();
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].y -= 1;
        }
        actions.editFrame.create(context.activeObject.currentFrame);
        updateUI();
      }
      e.preventDefault();
      break;
    case "ArrowDown":
      if (context.selection) {
        context.activeObject.currentFrame.saveState();
        for (let item of context.selection) {
          context.activeObject.currentFrame.keys[item.idx].y += 1;
        }
        actions.editFrame.create(context.activeObject.currentFrame);
        updateUI();
      }
      e.preventDefault();
      break;
    default:
      break;
  }
});

function playPause() {
  playing = !playing;
  if (playing) {
    if (context.activeObject.currentFrameNum >= context.activeObject.maxFrame - 1) {
      context.activeObject.setFrameNum(0);
    }
    for (let audioLayer of context.activeObject.audioLayers) {
      if (audioLayer.audible) {
        for (let i in audioLayer.sounds) {
          let sound = audioLayer.sounds[i];
          sound.player.start(
            0,
            context.activeObject.currentFrameNum / config.framerate,
          );
        }
      }
    }
    lastFrameTime = performance.now();
    advanceFrame();
  } else {
    for (let audioLayer of context.activeObject.audioLayers) {
      for (let i in audioLayer.sounds) {
        let sound = audioLayer.sounds[i];
        sound.player.stop();
      }
    }
  }
}

function advanceFrame() {
  context.activeObject.advanceFrame();
  updateLayers();
  updateUI();
  if (playing) {
    if (
      context.activeObject.currentFrameNum <
      context.activeObject.maxFrame - 1
    ) {
      const now = performance.now();
      const elapsedTime = now - lastFrameTime;

      // Calculate the time remaining for the next frame
      const targetTimePerFrame = 1000 / config.framerate;
      const timeToWait = Math.max(0, targetTimePerFrame - elapsedTime);
      lastFrameTime = now + timeToWait;
      setTimeout(advanceFrame, timeToWait);
    } else {
      playing = false;
      for (let audioLayer of context.activeObject.audioLayers) {
        for (let i in audioLayer.sounds) {
          let sound = audioLayer.sounds[i];
          sound.player.stop();
        }
      }
    }
  } else {
    updateMenu();
  }
}

function decrementFrame() {
  context.activeObject.decrementFrame();
  updateLayers();
  updateMenu();
  updateUI();
}

function newWindow(path) {
  invoke("create_window", {app: window.__TAURI__.app, path: path})
}

function _newFile(width, height, fps) {
  root = new GraphicsObject("root");
  context.objectStack = [root];
  context.selection = [];
  context.shapeselection = [];
  config.fileWidth = width;
  config.fileHeight = height;
  config.framerate = fps;
  filePath = undefined;
  saveConfig();
  undoStack = [];
  redoStack = [];
  updateUI();
  updateLayers();
  updateMenu();
}

async function newFile() {
  if (
    await confirmDialog("Create a new file? Unsaved work will be lost.", {
      title: "New file",
      kind: "warning",
    })
  ) {
    showNewFileDialog(config);
  }
}

async function _save(path) {
  try {
    function replacer(key, value) {
      if (key === "parent") {
        return undefined; // Avoid circular references
      }
      return value;
    }
    // for (let action of undoStack) {
    //   console.log(action.name);
    // }
    const fileData = {
      version: "1.7.7",
      width: config.fileWidth,
      height: config.fileHeight,
      fps: config.framerate,
      actions: undoStack,
      json: root.toJSON(),
    };
    if (config.debug) {
      // Pretty print file structure when debugging
      const contents = JSON.stringify(fileData, null, 2);
      await writeTextFile(path, contents);
    } else {
      const contents = JSON.stringify(fileData);
      await writeTextFile(path, contents);
    }
    filePath = path;
    addRecentFile(path);
    lastSaveIndex = undoStack.length;
    updateMenu();
    console.log(`${path} saved successfully!`);
  } catch (error) {
    console.error("Error saving text file:", error);
  }
}

async function save() {
  if (filePath) {
    _save(filePath);
  } else {
    saveAs();
  }
}

async function saveAs() {
  const filename = filePath ? await basename(filePath) : "untitled.beam";
  const path = await saveFileDialog({
    filters: [
      {
        name: "Lightningbeam files (.beam)",
        extensions: ["beam"],
      },
    ],
    defaultPath: await join(await documentDir(), filename),
  });
  if (path != undefined) _save(path);
}

async function _open(path, returnJson = false) {
  document.body.style.cursor = "wait"
  closeDialog();
  try {
    const contents = await readTextFile(path);
    let file = JSON.parse(contents);
    if (file.version == undefined) {
      await messageDialog("Could not read file version!", {
        title: "Load error",
        kind: "error",
      });
      document.body.style.cursor = "default"
      return;
    }
    if (file.version >= minFileVersion) {
      if (file.version < maxFileVersion) {
        if (returnJson) {
          if (file.json == undefined) {
            await messageDialog(
              "Could not import from this file. Re-save it with a current version of Lightningbeam.",
            );
          }
          document.body.style.cursor = "default"
          return file.json;
        } else {
          _newFile(file.width, file.height, file.fps);
          if (file.actions == undefined) {
            await messageDialog("File has no content!", {
              title: "Parse error",
              kind: "error",
            });
            document.body.style.cursor = "default"
            return;
          }

          const objectOffsets = {};
          const frameIDs = []
          if (file.version < "1.7.5") {
            for (let action of file.actions) {
              if (!(action.name in actions)) {
                await messageDialog(
                  `Invalid action ${action.name}. File may be corrupt.`,
                  { title: "Error", kind: "error" },
                );
                document.body.style.cursor = "default"
                return;
              }

              console.log(action.name);
              // Data fixes
              if (file.version <= "1.5") {
                // Fix coordinates of objects
                if (action.name == "group") {
                  let bbox;
                  for (let i of action.action.shapes) {
                    const shape = pointerList[i];
                    if (bbox == undefined) {
                      bbox = shape.bbox();
                    } else {
                      growBoundingBox(bbox, shape.bbox());
                    }
                  }
                  for (let i of action.action.objects) {
                    const object = pointerList[i]; // TODO: rotated bbox
                    if (bbox == undefined) {
                      bbox = object.bbox();
                    } else {
                      growBoundingBox(bbox, object.bbox());
                    }
                  }
                  const position = {
                    x: (bbox.x.min + bbox.x.max) / 2,
                    y: (bbox.y.min + bbox.y.max) / 2,
                  };
                  action.action.position = position;
                  objectOffsets[action.action.groupUuid] = position;
                  for (let shape of action.action.shapes) {
                    objectOffsets[shape] = position
                  }
                } else if (action.name == "editFrame") {
                  for (let key in action.action.newState) {
                    if (key in objectOffsets) {
                      action.action.newState[key].x += objectOffsets[key].x;
                      action.action.newState[key].y += objectOffsets[key].y;
                    }
                  }
                  for (let key in action.action.oldState) {
                    if (key in objectOffsets) {
                      action.action.oldState[key].x += objectOffsets[key].x;
                      action.action.oldState[key].y += objectOffsets[key].y;
                    }
                  }
                } else if (action.name == "addKeyframe") {
                  for (let id in objectOffsets) {
                    objectOffsets[action.action.uuid.slice(0,8) + id.slice(8)] = objectOffsets[id]
                  }
                } else if (action.name == "editShape") {
                  if (action.action.shape in objectOffsets) {
                    console.log("editing shape")
                    for (let curve of action.action.newCurves) {
                      for (let point of curve.points) {
                        point.x -= objectOffsets[action.action.shape].x
                        point.y -= objectOffsets[action.action.shape].y
                      }
                    }
                    for (let curve of action.action.oldCurves) {
                      for (let point of curve.points) {
                        point.x -= objectOffsets[action.action.shape].x
                        point.y -= objectOffsets[action.action.shape].y
                      }
                    }
                  }
                }
              }
              if (file.version <= "1.6") {
                // Fix copy-paste
                if (action.name == "duplicateObject") {
                  const obj = pointerList[action.action.object];
                  const objJson = obj.toJSON(true);
                  objJson.idx =
                    action.action.uuid.slice(0, 8) +
                    action.action.object.slice(8);
                  action.action.items = [objJson];
                  action.action.object = "root";
                  action.action.frame = root.currentFrame.idx;
                }
              }

              await actions[action.name].execute(action.action);
              undoStack.push(action);
            }
          } else {
            if (file.version < "1.7.7") {
              function setParentReferences(obj, parentIdx = null) {
                if (obj.type === "GraphicsObject") {
                  obj.parent = parentIdx; // Set the parent property
                }
              
                Object.values(obj).forEach(child => {
                  if (typeof child === 'object' && child !== null) setParentReferences(child, obj.type === "GraphicsObject" ? obj.idx : parentIdx);
                })
              }
              setParentReferences(file.json)
              console.log(file.json)
            }
            if (file.version < "1.7.6") {
              function restoreLineColors(obj) {
                // Step 1: Create colorMapping dictionary
                const colorMapping = (obj.actions || []).reduce((map, action) => {
                    if (action.name === "addShape" && action.action.curves.length > 0) {
                        map[action.action.uuid] = action.action.curves[0].color;
                    }
                    return map;
                }, {});
            
                // Step 2: Recursive pass to add colors from colorMapping back to curves
                function recurse(item) {
                    if (item?.curves && item.idx && colorMapping[item.idx]) {
                        item.curves.forEach(curve => {
                            if (Array.isArray(curve)) curve.push(colorMapping[item.idx]);
                        });
                    }
                    Object.values(item).forEach(value => {
                        if (typeof value === 'object' && value !== null) recurse(value);
                    });
                }
            
                recurse(obj);
              }
            
              restoreLineColors(file)

              function restoreAudio(obj) {
                const audioSrcMapping = (obj.actions || []).reduce((map, action) => {
                    if (action.name === "addAudio") {
                        map[action.action.layeruuid] = action.action;
                    }
                    return map;
                }, {});
            
                function recurse(item) {
                    if (item.type=="AudioLayer" && audioSrcMapping[item.idx]) {
                        const action = audioSrcMapping[item.idx]
                        item.sounds[action.uuid] = {
                          start: action.frameNum,
                          src: action.audiosrc,
                          uuid: action.uuid
                        }
                    }
                    Object.values(item).forEach(value => {
                        if (typeof value === 'object' && value !== null) recurse(value);
                    });
                }
            
                recurse(obj);
              }
            
              restoreAudio(file)
            }
            // disabled for now
            // for (let action of file.actions) {
            //   undoStack.push(action)
            // }
            root = GraphicsObject.fromJSON(file.json)
            context.objectStack = [root]
          }

          lastSaveIndex = undoStack.length;
          filePath = path;
          // Tauri thinks it is setting the title here, but it isn't getting updated
          await getCurrentWindow().setTitle(await basename(filePath));
          addRecentFile(path);
          updateUI();
        }
      } else {
        await messageDialog(
          `File ${path} was created in a newer version of Lightningbeam and cannot be opened in this version.`,
          { title: "File version mismatch", kind: "error" },
        );
      }
    } else {
      await messageDialog(
        `File ${path} is too old to be opened in this version of Lightningbeam.`,
        { title: "File version mismatch", kind: "error" },
      );
    }
  } catch (e) {
    console.log(e);
    if (e instanceof SyntaxError) {
      await messageDialog(`Could not parse ${path}, ${e.message}`, {
        title: "Error",
        kind: "error",
      });
    } else if (
      e instanceof String &&
      e.startsWith("failed to read file as text")
    ) {
      await messageDialog(
        `Could not parse ${path}, is it actually a Lightningbeam file?`,
        { title: "Error", kind: "error" },
      );
    } else {
      console.error(e);
      await messageDialog(
        `Error replaying file: ${e}`,
        { title: "Error", kind: "error" },
      );
    }
  }
  document.body.style.cursor = "default"
}

async function open() {
  const path = await openFileDialog({
    multiple: false,
    directory: false,
    filters: [
      {
        name: "Lightningbeam files (.beam)",
        extensions: ["beam"],
      },
    ],
    defaultPath: await documentDir(),
  });
  console.log(path);
  if (path) {
    document.body.style.cursor = "wait"
    setTimeout(()=>_open(path),10);
  }
}

function revert() {
  for (let _ = 0; undoStack.length > lastSaveIndex; _++) {
    undo();
  }
}

async function importFile() {
  const path = await openFileDialog({
    multiple: false,
    directory: false,
    filters: [
      {
        name: "Image files",
        extensions: ["png", "gif", "avif", "jpg", "jpeg"],
      },
      {
        name: "Audio files",
        extensions: ["mp3"],
      },
      {
        name: "Lightningbeam files",
        extensions: ["beam"],
      },
    ],
    defaultPath: await documentDir(),
    title: "Import File",
  });
  const imageMimeTypes = [
    "image/jpeg", // JPEG
    "image/png", // PNG
    "image/gif", // GIF
    "image/webp", // WebP
    // "image/svg+xml",// SVG
    "image/bmp", // BMP
    // "image/tiff",   // TIFF
    // "image/x-icon", // ICO
    // "image/heif",   // HEIF
    // "image/avif"    // AVIF
  ];
  const audioMimeTypes = [
    "audio/mpeg", // MP3
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
    const filename = await basename(path);
    if (getFileExtension(filename) == "beam") {
      function reassignIdxs(json) {
        if (json.idx in pointerList) {
          json.idx = uuidv4();
        }
        deeploop(json, (key, item) => {
          if (item.idx in pointerList) {
            item.idx = uuidv4();
          }
        });
      }
      function assignUUIDs(obj, existing) {
        const uuidCache = {}; // Cache to store UUIDs for existing values

        function replaceUuids(obj) {
          for (const [key, value] of Object.entries(obj)) {
            if (typeof value === "object" && value !== null) {
              replaceUuids(value);
            } else if (value in existing && key != "name") {
              if (!uuidCache[value]) {
                uuidCache[value] = uuidv4();
              }
              obj[key] = uuidCache[value];
            }
          }
        }

        function replaceReferences(obj) {
          for (const [key, value] of Object.entries(obj)) {
            if (key in existing) {
              obj[uuidCache[key]] = obj[key];
              delete obj[key]
            }
            if (typeof value === "object" && value !== null) {
              replaceReferences(value);
            } else if (value in uuidCache) {
              obj[key] = value
            }
          }
        }

        // Start the recursion with the provided object
        replaceUuids(obj);
        replaceReferences(obj)

        return obj; // Return the updated object
      }

      const json = await _open(path, true);
      if (json == undefined) return;
      assignUUIDs(json, pointerList);
      createModal(outliner, json, (object) => {
        actions.importObject.create(object);
      });
      updateOutliner();
    } else {
      const { dataURL, mimeType } = await convertToDataURL(
        path,
        imageMimeTypes.concat(audioMimeTypes),
      );
      if (imageMimeTypes.indexOf(mimeType) != -1) {
        actions.addImageObject.create(50, 50, dataURL, 0, context.activeObject);
      } else {
        actions.addAudio.create(dataURL, context.activeObject, filename);
      }
    }
  }
}

async function quit() {
  if (undoStack.length > lastSaveIndex) {
    if (
      await confirmDialog("Are you sure you want to quit?", {
        title: "Really quit?",
        kind: "warning",
      })
    ) {
      getCurrentWindow().close();
    }
  } else {
    getCurrentWindow().close();
  }
}

function copy() {
  clipboard = [];
  for (let object of context.selection) {
    clipboard.push(object.toJSON(true));
  }
  for (let shape of context.shapeselection) {
    clipboard.push(shape.toJSON(true));
  }
}

function paste() {
  // for (let item of clipboard) {
  //   if (item instanceof GraphicsObject) {
  //     console.log(item);
  //     // context.activeObject.addObject(item.copy())
  //     actions.duplicateObject.create(item);
  //   }
  // }
  actions.duplicateObject.create(clipboard);
  updateUI();
}

function delete_action() {
  if (context.selection.length || context.shapeselection.length) {
    actions.deleteObjects.create(context.selection, context.shapeselection);
    context.selection = [];
  }
  updateUI();
}

function addFrame() {
  if (
    context.activeObject.currentFrameNum >=
    context.activeObject.activeLayer.frames.length
  ) {
    actions.addFrame.create();
  }
}

function addKeyframe() {
  actions.addKeyframe.create();
}
function deleteFrame() {
  let frame = context.activeObject.currentFrame;
  let layer = context.activeObject.activeLayer;
  if (frame) {
    actions.deleteFrame.create(frame, layer);
  }
}
async function about() {
  messageDialog(
    `Lightningbeam version ${await getVersion()}\nDeveloped by Skyler Lehmkuhl`,
    { title: "About", kind: "info" },
  );
}

// Export stuff that's all crammed in here and needs refactored
function createProgressModal() {
  // Check if the modal already exists
  const existingModal = document.getElementById('progressModal');
  if (existingModal) {
    existingModal.style.display = 'flex';
    return; // If the modal already exists, do nothing
  }

  // Create modal container with a unique ID
  const modal = document.createElement('div');
  modal.id = 'progressModal';  // Give the modal a unique ID
  modal.style.position = 'fixed';
  modal.style.top = '0';
  modal.style.left = '0';
  modal.style.width = '100%';
  modal.style.height = '100%';
  modal.style.backgroundColor = 'rgba(0, 0, 0, 0.5)';
  modal.style.display = 'flex';
  modal.style.justifyContent = 'center';
  modal.style.alignItems = 'center';
  modal.style.zIndex = '9999';
  
  // Create inner modal box
  const modalContent = document.createElement('div');
  modalContent.style.backgroundColor = backgroundColor;
  modalContent.style.padding = '20px';
  modalContent.style.borderRadius = '8px';
  modalContent.style.textAlign = 'center';
  modalContent.style.minWidth = '300px';
  
  // Create progress bar
  const progressBar = document.createElement('progress');
  progressBar.id = 'progressBar';
  progressBar.value = 0;
  progressBar.max = 100;
  progressBar.style.width = '100%';

  // Create text to show the current frame info
  const progressText = document.createElement('p');
  progressText.id = 'progressText';
  progressText.innerText = 'Initializing...';

  // Append elements to modalContent
  modalContent.appendChild(progressBar);
  modalContent.appendChild(progressText);
  
  // Append modalContent to modal
  modal.appendChild(modalContent);

  // Append modal to body
  document.body.appendChild(modal);
}


async function setupVideoExport(ext, path, canvas, exportContext) {
  createProgressModal();
  
  await LibAVWebCodecs.load();
  console.log("Codecs loaded");

  let target;
  let muxer;
  let videoEncoder;
  let videoConfig;
  let audioEncoder;
  let audioConfig;
  const frameTimeMicroseconds = parseInt(1_000_000 / config.framerate)
  const oldContext = context;
  context = exportContext;

  const oldRootFrame = root.currentFrameNum
  const bitrate = 1e6
  
  // Choose muxer and encoder configuration based on file extension
  if (ext === "mp4") {
    target = new Mp4Muxer.ArrayBufferTarget();
    // TODO: add video options dialog for width, height, bitrate
    muxer = new Mp4Muxer.Muxer({
      target: target,
      video: {
        codec: 'avc',
        width: config.fileWidth,
        height: config.fileHeight,
        frameRate: config.framerate,
      },
      fastStart: 'in-memory',
      firstTimestampBehavior: 'offset',
    });

    videoConfig = {
      codec: 'avc1.42001f',
      width: config.fileWidth,
      height: config.fileHeight,
      bitrate: bitrate,
    };

    // Todo: add configuration for mono/stereo
    audioConfig = {
      codec: 'mp4a.40.2', // AAC codec
      sampleRate: 44100,
      numberOfChannels: 2, // Mono
      bitrate: 64000,
    };
  } else if (ext === "webm") {
    target = new WebMMuxer.ArrayBufferTarget();
    muxer = new WebMMuxer.Muxer({
      target: target,
      video: {
        codec: 'V_VP9',
        width: config.fileWidth,
        height: config.fileHeight,
        frameRate: config.framerate,
      },
      firstTimestampBehavior: 'offset',
    });

    videoConfig = {
      codec: 'vp09.00.10.08',
      width: config.fileWidth,
      height: config.fileHeight,
      bitrate: bitrate,
      bitrateMode: "constant",
    };

    audioConfig = {
      codec: 'opus',  // Use Opus codec for WebM
      sampleRate: 48000,
      numberOfChannels: 2,
      bitrate: 64000,
    }
  }

  // Initialize the video encoder
  videoEncoder = new VideoEncoder({
    output: (chunk, meta) => muxer.addVideoChunk(chunk, meta, undefined, undefined, frameTimeMicroseconds),
    error: (e) => console.error(e),
  });

  videoEncoder.configure(videoConfig);

  // audioEncoder = new AudioEncoder({
  //   output: (chunk, meta) => muxer.addAudioChunk(chunk, meta),
  //   error: (e) => console.error(e),
  // });

  // audioEncoder.configure(audioConfig)

  async function finishEncoding() {
    const progressText = document.getElementById('progressText');
    progressText.innerText = 'Finalizing...';
    const progressBar = document.getElementById('progressBar');
    progressBar.value = 100;
    await videoEncoder.flush();
    muxer.finalize();
    await writeFile(path, new Uint8Array(target.buffer));
    const modal = document.getElementById('progressModal');
    modal.style.display = 'none';
    document.querySelector("body").style.cursor = "default";
  }

  const processFrame = async (currentFrame) => {
    if (currentFrame < root.maxFrame) {
      // Update progress bar
      const progressText = document.getElementById('progressText');
      progressText.innerText = `Rendering frame ${currentFrame + 1} of ${root.maxFrame}`;
      const progressBar = document.getElementById('progressBar');
      const progress = Math.round(((currentFrame + 1) / root.maxFrame) * 100);
      progressBar.value = progress;

      root.setFrameNum(currentFrame);
      exportContext.ctx.fillStyle = "white";
      exportContext.ctx.rect(0, 0, config.fileWidth, config.fileHeight);
      exportContext.ctx.fill();
      root.draw(exportContext.ctx);
      const frame = new VideoFrame(
        await LibAVWebCodecs.createImageBitmap(canvas),
        { timestamp: currentFrame * frameTimeMicroseconds }
      );

      // Encode frame
      const keyFrame = currentFrame % 60 === 0; // Every 60th frame is a key frame
      videoEncoder.encode(frame, { keyFrame });
      frame.close();

      currentFrame++;
      setTimeout(() => processFrame(currentFrame), 4);
    } else {
      // Once all frames are processed, reset context and export
      context = oldContext;
      root.setFrameNum(oldRootFrame);
      finishEncoding();
    }
  };

  processFrame(0);
}

async function render() {
  document.querySelector("body").style.cursor = "wait";
  const path = await saveFileDialog({
    filters: [
      {
        name: "WebM files (.webm)",
        extensions: ["webm"],
      },
      {
        name: "MP4 files (.mp4)",
        extensions: ["mp4"],
      },
      {
        name: "APNG files (.png)",
        extensions: ["png"],
      },
      {
        name: "Packed HTML player (.html)",
        extensions: ["html"],
      },
    ],
    defaultPath: await join(await documentDir(), "untitled.webm"),
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

    const ext = path.split(".").pop().toLowerCase();

    const canvas = document.createElement("canvas");
    canvas.width = config.fileWidth; // Set desired width
    canvas.height = config.fileHeight; // Set desired height
    let exportContext = {
      ...context,
      ctx: canvas.getContext("2d"),
      selectionRect: undefined,
      selection: [],
      shapeselection: [],
    };


    switch (ext) {
      case "mp4":
      case "webm":
        await setupVideoExport(ext, path, canvas, exportContext);
        break;
      case "html":
        fetch("/player.html")
          .then((response) => {
            if (!response.ok) {
              throw new Error("Network response was not ok");
            }
            return response.text(); // Read the response body as a string
          })
          .then((data) => {
            // TODO: strip out the stuff tauri injects
            let json = JSON.stringify({
              fileWidth: config.fileWidth,
              fileHeight: config.fileHeight,
              root: root.toJSON(),
            });
            data = data.replace('"${file}"', json);
            console.log(data); // The content of the file as a string
          })
          .catch((error) => {
            // TODO: alert
            console.error(
              "There was a problem with the fetch operation:",
              error,
            );
          });

        break;
      case "png":
        const frames = [];
        canvas = document.createElement("canvas");
        canvas.width = config.fileWidth; // Set desired width
        canvas.height = config.fileHeight; // Set desired height
        

        for (let i = 0; i < root.maxFrame; i++) {
          root.currentFrameNum = i;
          exportContext.ctx.fillStyle = "white";
          exportContext.ctx.rect(0, 0, config.fileWidth, config.fileHeight);
          exportContext.ctx.fill();
          root.draw(exportContext);

          // Convert the canvas content to a PNG image (this is the "frame" we add to the APNG)
          const imageData = exportContext.ctx.getImageData(
            0,
            0,
            canvas.width,
            canvas.height,
          );

          // Step 2: Create a frame buffer (Uint8Array) from the image data
          const frameBuffer = new Uint8Array(imageData.data.buffer);

          frames.push(frameBuffer); // Add the frame buffer to the frames array
        }

        // Step 3: Use UPNG.js to create the animated PNG
        const apng = UPNG.encode(
          frames,
          canvas.width,
          canvas.height,
          0,
          parseInt(100 / config.framerate),
        );

        // Step 4: Save the APNG file (in Tauri, use writeFile or in the browser, download it)
        const apngBlob = new Blob([apng], { type: "image/png" });

        // If you're using Tauri:
        await writeFile(
          path, // The destination file path for saving
          new Uint8Array(await apngBlob.arrayBuffer()),
        );
        break;
    }
  }
  document.querySelector("body").style.cursor = "default";
}

function updateScrollPosition(zoomFactor) {
  if (context.mousePos) {
    for (let canvas of canvases) {
      canvas.offsetX =
        (canvas.offsetX + context.mousePos.x) * zoomFactor - context.mousePos.x;
      canvas.offsetY =
        (canvas.offsetY + context.mousePos.y) * zoomFactor - context.mousePos.y;
      canvas.zoomLevel = context.zoomLevel
    }
  }
}

function zoomIn() {
  let zoomFactor = 2;
  if (context.zoomLevel < 8) {
    context.zoomLevel *= zoomFactor;
    updateScrollPosition(zoomFactor);
    updateUI();
    updateMenu();
  }
}
function zoomOut() {
  let zoomFactor = 0.5;
  if (context.zoomLevel > 1 / 8) {
    context.zoomLevel *= zoomFactor;
    updateScrollPosition(zoomFactor);
    updateUI();
    updateMenu();
  }
}
function resetZoom() {
  context.zoomLevel = 1;
  recenter()
}

function recenter() {
  for (let canvas of canvases) {
    canvas.offsetX = canvas.offsetY = 0;
  }
  updateUI();
  updateMenu();
}

function stage() {
  let stage = document.createElement("canvas");
  // let scroller = document.createElement("div")
  // let stageWrapper = document.createElement("div")
  stage.className = "stage";
  // stage.width = config.fileWidth
  // stage.height = config.fileHeight
  stage.offsetX = 0;
  stage.offsetY = 0;
  stage.zoomLevel = context.zoomLevel

  let lastResizeTime = 0;
  const throttleIntervalMs = 20;

  function updateStageCanvasSize() {
    const canvasStyles = window.getComputedStyle(stage);

    stage.width = parseInt(canvasStyles.width);
    stage.height = parseInt(canvasStyles.height);
    updateUI();
    renderAll();
  }
  const resizeObserver = new ResizeObserver(() => {
    const currentTime = Date.now();

    if (currentTime - lastResizeTime > throttleIntervalMs) {
      lastResizeTime = currentTime;
      updateStageCanvasSize();
    }
  });
  resizeObserver.observe(stage);
  updateStageCanvasSize();

  stage.addEventListener("wheel", (event) => {
    event.preventDefault();
    const deltaX = event.deltaX * config.scrollSpeed;
    const deltaY = event.deltaY * config.scrollSpeed;

    stage.offsetX += deltaX;
    stage.offsetY += deltaY;
    const currentTime = Date.now();
    if (currentTime - lastResizeTime > throttleIntervalMs) {
      lastResizeTime = currentTime;
      updateUI();
    }
  });
  // scroller.className = "scroll"
  // stageWrapper.className = "stageWrapper"
  // let selectionRect = document.createElement("div")
  // selectionRect.className = "selectionRect"
  // for (let i of ["nw", "ne", "se", "sw"]) {
  //   let cornerRotateRect = document.createElement("div")
  //   cornerRotateRect.classList.add("cornerRotateRect")
  //   cornerRotateRect.classList.add(i)
  //   cornerRotateRect.addEventListener('mouseup', (e) => {
  //     const newEvent = new MouseEvent(e.type, e);
  //     stage.dispatchEvent(newEvent)
  //   })
  //   cornerRotateRect.addEventListener('mousemove', (e) => {
  //     const newEvent = new MouseEvent(e.type, e);
  //     stage.dispatchEvent(newEvent)
  //   })
  //   selectionRect.appendChild(cornerRotateRect)
  // }
  // for (let i of ["nw", "n", "ne", "e", "se", "s", "sw", "w"]) {
  //   let cornerRect = document.createElement("div")
  //   cornerRect.classList.add("cornerRect")
  //   cornerRect.classList.add(i)
  //   cornerRect.addEventListener('mousedown', (e) => {
  //     let bbox = undefined;
  //     let selection = {}
  //     for (let item of context.selection) {
  //       if (bbox==undefined) {
  //         bbox = structuredClone(item.bbox())
  //       } else {
  //         growBoundingBox(bbox, item.bbox())
  //       }
  //       selection[item.idx] = {x: item.x, y: item.y, scale_x: item.scale_x, scale_y: item.scale_y}
  //     }
  //     if (bbox != undefined) {
  //       context.dragDirection = i
  //       context.activeTransform = {
  //         initial: {
  //           x: {min: bbox.x.min, max: bbox.x.max},
  //           y: {min: bbox.y.min, max: bbox.y.max},
  //           selection: selection
  //         },
  //         current: {
  //           x: {min: bbox.x.min, max: bbox.x.max},
  //           y: {min: bbox.y.min, max: bbox.y.max},
  //           selection: structuredClone(selection)
  //         }
  //       }
  //       context.activeObject.currentFrame.saveState()
  //     }
  //   })
  //   cornerRect.addEventListener('mouseup', (e) => {
  //     const newEvent = new MouseEvent(e.type, e);
  //     stage.dispatchEvent(newEvent)
  //   })
  //   cornerRect.addEventListener('mousemove', (e) => {
  //     const newEvent = new MouseEvent(e.type, e);
  //     stage.dispatchEvent(newEvent)
  //   })
  //   selectionRect.appendChild(cornerRect)
  // }

  stage.addEventListener("drop", (e) => {
    e.preventDefault();
    let mouse = getMousePos(stage, e);
    const imageTypes = [
      "image/png",
      "image/gif",
      "image/avif",
      "image/jpeg",
      "image/webp", //'image/svg+xml' // Disabling SVG until we can export them nicely
    ];
    const audioTypes = ["audio/mpeg"]; // TODO: figure out what other audio formats Tone.js accepts
    if (e.dataTransfer.items) {
      let i = 0;
      for (let item of e.dataTransfer.items) {
        if (item.kind == "file") {
          let file = item.getAsFile();
          if (imageTypes.includes(file.type)) {
            let img = new Image();
            let reader = new FileReader();

            // Read the file as a data URL
            reader.readAsDataURL(file);
            reader.ix = i;

            reader.onload = function (event) {
              let imgsrc = event.target.result; // This is the data URL
              actions.addImageObject.create(
                mouse.x,
                mouse.y,
                imgsrc,
                reader.ix,
                context.activeObject,
              );
            };

            reader.onerror = function (error) {
              console.error("Error reading file as data URL", error);
            };
          } else if (audioTypes.includes(file.type)) {
            let reader = new FileReader();

            // Read the file as a data URL
            reader.readAsDataURL(file);
            reader.onload = function (event) {
              let audiosrc = event.target.result;
              actions.addAudio.create(
                audiosrc,
                context.activeObject,
                file.name,
              );
            };
          }
          i++;
        }
      }
    } else {
    }
  });
  stage.addEventListener("dragover", (e) => {
    e.preventDefault();
  });
  canvases.push(stage);
  // stageWrapper.appendChild(stage)
  // stageWrapper.appendChild(selectionRect)
  // scroller.appendChild(stageWrapper)
  stage.addEventListener("pointerdown", (e) => {
    let mouse = getMousePos(stage, e);
    root.handleMouseEvent("mousedown", mouse.x, mouse.y)
    mouse = context.activeObject.transformMouse(mouse);
    let selection;
    if (!context.activeObject.currentFrame?.exists) return;
    switch (mode) {
      case "rectangle":
      case "ellipse":
      case "draw":
        // context.mouseDown = true;
        // context.activeShape = new Shape(mouse.x, mouse.y, context, uuidv4());
        // context.lastMouse = mouse;
        break;
      case "select":
        if (context.activeObject.currentFrame.frameType != "keyframe") break;
        selection = selectVertex(context, mouse);
        if (selection) {
          context.dragging = true;
          context.activeCurve = undefined;
          context.activeVertex = {
            current: {
              point: {
                x: selection.vertex.point.x,
                y: selection.vertex.point.y,
              },
              startCurves: structuredClone(selection.vertex.startCurves),
              endCurves: structuredClone(selection.vertex.endCurves),
            },
            initial: selection.vertex,
            shape: selection.shape,
            startmouse: { x: mouse.x, y: mouse.y },
          };
        } else {
          selection = selectCurve(context, mouse);
          if (selection) {
            context.dragging = true;
            context.activeVertex = undefined;
            context.activeCurve = {
              initial: selection.curve,
              current: new Bezier(selection.curve.points).setColor(
                selection.curve.color,
              ),
              shape: selection.shape,
              startmouse: { x: mouse.x, y: mouse.y },
            };
          } else {
            let selected = false;
            let child;
            if (context.selection.length) {
              for (child of context.selection) {
                if (hitTest(mouse, child)) {
                  context.dragging = true;
                  context.lastMouse = mouse;
                  context.activeAction = actions.editFrame.initialize(
                    context.activeObject.currentFrame,
                  );
                  break;
                }
              }
            }
            if (!context.dragging) {
              // Have to iterate in reverse order to grab the frontmost object when two overlap
              for (
                let i = context.activeObject.activeLayer.children.length - 1;
                i >= 0;
                i--
              ) {
                child = context.activeObject.activeLayer.children[i];
                if (!(child.idx in context.activeObject.currentFrame.keys))
                  continue;
                // let bbox = child.bbox()
                if (hitTest(mouse, child)) {
                  if (context.selection.indexOf(child) != -1) {
                    // dragging = true
                  }
                  child.saveState();
                  if (e.shiftKey) {
                    context.selection.push(child);
                  } else {
                    context.selection = [child];
                  }
                  context.dragging = true;
                  selected = true;
                  context.activeAction = actions.editFrame.initialize(
                    context.activeObject.currentFrame,
                  );
                  break;
                }
              }
              if (!selected) {
                context.oldselection = context.selection;
                context.oldshapeselection = context.shapeselection;
                context.selection = [];
                context.shapeselection = [];
                if (
                  context.oldselection.length ||
                  context.oldshapeselection.length
                ) {
                  actions.select.create();
                }
                context.oldselection = context.selection;
                context.oldshapeselection = context.selection;
                context.selectionRect = {
                  x1: mouse.x,
                  x2: mouse.x,
                  y1: mouse.y,
                  y2: mouse.y,
                };
              }
            }
          }
        }
        break;
      case "transform":
        let bbox = undefined;
        selection = {};
        for (let item of context.selection) {
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
        let transformPoint = getPointNearBox(bbox, mouse, 10);
        if (transformPoint) {
          context.dragDirection = transformPoint;
          context.activeTransform = {
            initial: {
              x: { min: bbox.x.min, max: bbox.x.max },
              y: { min: bbox.y.min, max: bbox.y.max },
              rotation: 0,
              selection: selection,
            },
            current: {
              x: { min: bbox.x.min, max: bbox.x.max },
              y: { min: bbox.y.min, max: bbox.y.max },
              rotation: 0,
              selection: structuredClone(selection),
            },
          };
          context.activeAction = actions.transformObjects.initialize(
            context.activeObject.currentFrame,
            context.selection,
            transformPoint,
            mouse,
          );
        } else {
          transformPoint = getPointNearBox(bbox, mouse, 30, false);
          if (transformPoint) {
            stage.style.cursor = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24' fill='currentColor' class='bi bi-arrow-counterclockwise' viewBox='0 0 16 16'%3E%3Cpath fill-rule='evenodd' d='M8 3a5 5 0 1 1-4.546 2.914.5.5 0 0 0-.908-.417A6 6 0 1 0 8 2z'/%3E%3Cpath d='M8 4.466V.534a.25.25 0 0 0-.41-.192L5.23 2.308a.25.25 0 0 0 0 .384l2.36 1.966A.25.25 0 0 0 8 4.466'/%3E%3C/svg%3E") 12 12, auto`;
            context.dragDirection = "r";
            context.activeTransform = {
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
                rotation: 0,
                mouse: { x: mouse.x, y: mouse.y },
                selection: structuredClone(selection),
              },
            };
            context.activeAction = actions.transformObjects.initialize(
              context.activeObject.currentFrame,
              context.selection,
              "r",
              mouse,
            );
          } else {
            stage.style.cursor = "default";
          }
        }
        break;
      case "paint_bucket":
        let line = { p1: mouse, p2: { x: mouse.x + 3000, y: mouse.y } };
        // debugCurves = [];
        // debugPoints = [];
        // let epsilon = context.fillGaps;
        // let min_x = Infinity;
        // let curveB = undefined;
        // let point = undefined;
        // let regionPoints;

        // // First, see if there's an existing shape to change the color of
        // const startTime = performance.now();
        // let pointShape = getShapeAtPoint(
        //   mouse,
        //   context.activeObject.currentFrame.shapes,
        // );
        // const endTime = performance.now();

        // console.log(
        //   `getShapeAtPoint took ${endTime - startTime} milliseconds.`,
        // );

        // if (pointShape) {
        //   actions.colorShape.create(pointShape, context.fillStyle);
        //   break;
        // }

        // // We didn't find an existing region to paintbucket, see if we can make one
        // const offset = context.activeObject.transformMouse({x:0, y:0})
        // try {
        //   regionPoints = floodFillRegion(
        //     mouse,
        //     epsilon,
        //     offset,
        //     config.fileWidth,
        //     config.fileHeight,
        //     context,
        //     debugPoints,
        //     debugPaintbucket,
        //   );
        // } catch (e) {
        //   updateUI();
        //   throw e;
        // }
        // if (regionPoints.length > 0 && regionPoints.length < 10) {
        //   // probably a very small area, rerun with minimum epsilon
        //   regionPoints = floodFillRegion(
        //     mouse,
        //     1,
        //     offset,
        //     config.fileWidth,
        //     config.fileHeight,
        //     context,
        //     debugPoints,
        //   );
        // }
        // let points = [];
        // for (let point of regionPoints) {
        //   points.push([point.x, point.y]);
        // }
        // let cxt = {
        //   ...context,
        //   fillShape: true,
        //   strokeShape: false,
        //   sendToBack: true,
        // };
        // let shape = new Shape(regionPoints[0].x, regionPoints[0].y, cxt);
        // shape.fromPoints(points, 1);
        // actions.addShape.create(context.activeObject, shape, cxt);
        break;
        // Loop labels in JS!
        // Iterate in reverse so we paintbucket the frontmost shape
        shapeLoop: for (
          let i = context.activeObject.currentFrame.shapes.length - 1;
          i >= 0;
          i--
        ) {
          let shape = context.activeObject.currentFrame.shapes[i];
          // for (let region of shape.regions) {
          let intersect_count = 0;
          for (let curve of shape.curves) {
            intersect_count += curve.intersects(line).length;
          }
          if (intersect_count % 2 == 1) {
            actions.colorShape.create(shape, context.fillStyle);
            break shapeLoop;
          }
          // }
        }
        break;
      case "eyedropper":
        const ctx = stage.getContext("2d")
        const imageData = ctx.getImageData(mouse.x, mouse.y, 1, 1); // Get pixel at (x, y)
        const data = imageData.data; // The pixel data is in the `data` array
        const hsv = rgbToHsv(...data)
        if (context.dropperColor == "Fill color") {
          for (let el of document.querySelectorAll(".color-field.fill")) {
            el.setColor(hsv, 'ff')
          }
        } else {
          for (let el of document.querySelectorAll(".color-field.stroke")) {
            el.setColor(hsv, 'ff')
          }
        }
        break;
      default:
        break;
    }
    context.lastMouse = mouse;
    updateUI();
    updateInfopanel();
  });
  stage.mouseup = (e) => {
    context.mouseDown = false;
    context.dragging = false;
    context.dragDirection = undefined;
    context.selectionRect = undefined;
    if (!context.activeObject.currentFrame?.exists) return;
    let mouse = getMousePos(stage, e);
    root.handleMouseEvent("mouseup", mouse.x, mouse.y)
    mouse = context.activeObject.transformMouse(mouse);
    switch (mode) {
      case "draw":
        // if (context.activeShape) {
        //   context.activeShape.addLine(mouse.x, mouse.y);
        //   context.activeShape.simplify(context.simplifyMode);
        //   actions.addShape.create(context.activeObject, context.activeShape);
        //   context.activeShape = undefined;
        // }
        break;
      case "rectangle":
      case "ellipse":
        // actions.addShape.create(context.activeObject, context.activeShape);
        // context.activeShape = undefined;
        break;
      case "select":
        if (context.activeAction) {
          actions[context.activeAction.type].finalize(
            context.activeAction,
            context.activeObject.currentFrame,
          );
        } else if (context.activeVertex) {
          let newCurves = [];
          for (let i in context.activeVertex.shape.curves) {
            if (i in context.activeVertex.current.startCurves) {
              newCurves.push(context.activeVertex.current.startCurves[i]);
            } else if (i in context.activeVertex.current.endCurves) {
              newCurves.push(context.activeVertex.current.endCurves[i]);
            } else {
              newCurves.push(context.activeVertex.shape.curves[i]);
            }
          }
          actions.editShape.create(context.activeVertex.shape, newCurves);
        } else if (context.activeCurve) {
          let newCurves = [];
          for (let curve of context.activeCurve.shape.curves) {
            if (curve == context.activeCurve.initial) {
              newCurves.push(context.activeCurve.current);
            } else {
              newCurves.push(curve);
            }
          }
          actions.editShape.create(context.activeCurve.shape, newCurves);
        } else if (context.selection.length) {
          actions.select.create();
          // actions.editFrame.create(context.activeObject.currentFrame)
        } else if (context.shapeselection.length) {
          actions.select.create();
        }
        break;
      case "transform":
        if (context.activeAction) {
          actions[context.activeAction.type].finalize(
            context.activeAction,
            context.activeObject.currentFrame,
          );
        }
        // actions.editFrame.create(context.activeObject.currentFrame)
        break;
      default:
        break;
    }
    context.lastMouse = mouse;
    context.activeCurve = undefined;
    updateUI();
    updateMenu();
    updateInfopanel();
  };
  stage.addEventListener("pointerup", stage.mouseup);
  stage.addEventListener("pointermove", (e) => {
    let mouse = getMousePos(stage, e);
    root.handleMouseEvent("mousemove", mouse.x, mouse.y)
    mouse = context.activeObject.transformMouse(mouse);
    context.mousePos = mouse;
    // if mouse is released, even if it happened outside the stage
    if (
      e.buttons == 0 &&
      (context.mouseDown ||
        context.dragging ||
        context.dragDirection ||
        context.selectionRect)
    ) {
      stage.mouseup(e);
      return;
    }
    if (!context.activeObject.currentFrame?.exists) return;
    switch (mode) {
      case "draw":
        stage.style.cursor = "default";
        context.activeCurve = undefined;
        if (context.activeShape) {
          if (vectorDist(mouse, context.lastMouse) > minSegmentSize) {
            context.activeShape.addLine(mouse.x, mouse.y);
            context.lastMouse = mouse;
          }
        }
        break;
      case "rectangle":
        stage.style.cursor = "default";
        context.activeCurve = undefined;
      //   if (context.activeShape) {
      //     context.activeShape.clear();
      //     context.activeShape.addLine(mouse.x, context.activeShape.starty);
      //     context.activeShape.addLine(mouse.x, mouse.y);
      //     context.activeShape.addLine(context.activeShape.startx, mouse.y);
      //     context.activeShape.addLine(
      //       context.activeShape.startx,
      //       context.activeShape.starty,
      //     );
      //     context.activeShape.update();
      //   }
      //   break;
      case "ellipse":
        stage.style.cursor = "default";
        context.activeCurve = undefined;
      //   if (context.activeShape) {
      //     let midX = (mouse.x + context.activeShape.startx) / 2;
      //     let midY = (mouse.y + context.activeShape.starty) / 2;
      //     let xDiff = (mouse.x - context.activeShape.startx) / 2;
      //     let yDiff = (mouse.y - context.activeShape.starty) / 2;
      //     let ellipseConst = 0.552284749831; // (4/3)*tan(pi/(2n)) where n=4
      //     context.activeShape.clear();
      //     context.activeShape.addCurve(
      //       new Bezier(
      //         midX,
      //         context.activeShape.starty,
      //         midX + ellipseConst * xDiff,
      //         context.activeShape.starty,
      //         mouse.x,
      //         midY - ellipseConst * yDiff,
      //         mouse.x,
      //         midY,
      //       ),
      //     );
      //     context.activeShape.addCurve(
      //       new Bezier(
      //         mouse.x,
      //         midY,
      //         mouse.x,
      //         midY + ellipseConst * yDiff,
      //         midX + ellipseConst * xDiff,
      //         mouse.y,
      //         midX,
      //         mouse.y,
      //       ),
      //     );
      //     context.activeShape.addCurve(
      //       new Bezier(
      //         midX,
      //         mouse.y,
      //         midX - ellipseConst * xDiff,
      //         mouse.y,
      //         context.activeShape.startx,
      //         midY + ellipseConst * yDiff,
      //         context.activeShape.startx,
      //         midY,
      //       ),
      //     );
      //     context.activeShape.addCurve(
      //       new Bezier(
      //         context.activeShape.startx,
      //         midY,
      //         context.activeShape.startx,
      //         midY - ellipseConst * yDiff,
      //         midX - ellipseConst * xDiff,
      //         context.activeShape.starty,
      //         midX,
      //         context.activeShape.starty,
      //       ),
      //     );
      //   }
      //   break;
      case "select":
        stage.style.cursor = "default";
        if (context.dragging) {
          if (context.activeVertex) {
            let vert = context.activeVertex;
            let mouseDelta = {
              x: mouse.x - vert.startmouse.x,
              y: mouse.y - vert.startmouse.y,
            };
            vert.current.point.x = vert.initial.point.x + mouseDelta.x;
            vert.current.point.y = vert.initial.point.y + mouseDelta.y;
            for (let i in vert.current.startCurves) {
              let curve = vert.current.startCurves[i];
              let oldCurve = vert.initial.startCurves[i];
              curve.points[0] = vert.current.point;
              curve.points[1] = {
                x: oldCurve.points[1].x + mouseDelta.x,
                y: oldCurve.points[1].y + mouseDelta.y,
              };
            }
            for (let i in vert.current.endCurves) {
              let curve = vert.current.endCurves[i];
              let oldCurve = vert.initial.endCurves[i];
              curve.points[3] = {
                x: vert.current.point.x,
                y: vert.current.point.y,
              };
              curve.points[2] = {
                x: oldCurve.points[2].x + mouseDelta.x,
                y: oldCurve.points[2].y + mouseDelta.y,
              };
            }
          } else if (context.activeCurve) {
            context.activeCurve.current.points = moldCurve(
              context.activeCurve.initial,
              mouse,
              context.activeCurve.startmouse,
            ).points;
          } else {
            for (let child of context.selection) {
              if (!context.activeObject.currentFrame) continue;
              if (!context.activeObject.currentFrame.keys) continue;
              if (!(child.idx in context.activeObject.currentFrame.keys)) continue;
              context.activeObject.currentFrame.keys[child.idx].x +=
                mouse.x - context.lastMouse.x;
              context.activeObject.currentFrame.keys[child.idx].y +=
                mouse.y - context.lastMouse.y;
            }
          }
        } else if (context.selectionRect) {
          context.selectionRect.x2 = mouse.x;
          context.selectionRect.y2 = mouse.y;
          context.selection = [];
          context.shapeselection = [];
          for (let child of context.activeObject.activeLayer.children) {
            if (hitTest(regionToBbox(context.selectionRect), child)) {
              context.selection.push(child);
            }
          }
          for (let shape of context.activeObject.currentFrame.shapes) {
            if (hitTest(regionToBbox(context.selectionRect), shape)) {
              context.shapeselection.push(shape);
            }
          }
        } else {
          let selection = selectVertex(context, mouse);
          if (selection) {
            context.activeCurve = undefined;
            context.activeVertex = {
              current: selection.vertex,
              initial: {
                point: {
                  x: selection.vertex.point.x,
                  y: selection.vertex.point.y,
                },
                startCurves: structuredClone(selection.vertex.startCurves),
                endCurves: structuredClone(selection.vertex.endCurves),
              },
              shape: selection.shape,
              startmouse: { x: mouse.x, y: mouse.y },
            };
          } else {
            context.activeVertex = undefined;
            selection = selectCurve(context, mouse);
            if (selection) {
              context.activeCurve = {
                current: selection.curve,
                initial: new Bezier(selection.curve.points).setColor(
                  selection.curve.color,
                ),
                shape: selection.shape,
                startmouse: mouse,
              };
            } else {
              context.activeCurve = undefined;
            }
          }
        }
        context.lastMouse = mouse;
        break;
      case "transform":
        // stage.style.cursor = "nw-resize"
        let bbox = undefined;
        for (let item of context.selection) {
          if (bbox == undefined) {
            bbox = getRotatedBoundingBox(item);
          } else {
            growBoundingBox(bbox, getRotatedBoundingBox(item));
          }
        }
        if (bbox == undefined) break;
        let point = getPointNearBox(bbox, mouse, 10);
        if (point) {
          stage.style.cursor = `${point}-resize`;
        } else {
          point = getPointNearBox(bbox, mouse, 30, false);
          if (point) {
            stage.style.cursor = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24' fill='currentColor' class='bi bi-arrow-counterclockwise' viewBox='0 0 16 16'%3E%3Cpath fill-rule='evenodd' d='M8 3a5 5 0 1 1-4.546 2.914.5.5 0 0 0-.908-.417A6 6 0 1 0 8 2z'/%3E%3Cpath d='M8 4.466V.534a.25.25 0 0 0-.41-.192L5.23 2.308a.25.25 0 0 0 0 .384l2.36 1.966A.25.25 0 0 0 8 4.466'/%3E%3C/svg%3E") 12 12, auto`;
          } else {
            stage.style.cursor = "default";
          }
        }
        // if (context.dragDirection) {
        //   let initial = context.activeTransform.initial
        //   let current = context.activeTransform.current
        //   let initialSelection = context.activeTransform.initial.selection
        //   if (context.dragDirection.indexOf('n') != -1) {
        //     current.y.min = mouse.y
        //   } else if (context.dragDirection.indexOf('s') != -1) {
        //     current.y.max = mouse.y
        //   }
        //   if (context.dragDirection.indexOf('w') != -1) {
        //     current.x.min = mouse.x
        //   } else if (context.dragDirection.indexOf('e') != -1) {
        //     current.x.max = mouse.x
        //   }
        //     // Calculate the translation difference between current and initial values
        //   let delta_x = current.x.min - initial.x.min;
        //   let delta_y = current.y.min - initial.y.min;

        //   if (context.dragDirection == 'r') {
        //     let pivot = {
        //       x: (initial.x.min+initial.x.max)/2,
        //       y: (initial.y.min+initial.y.max)/2,
        //     }
        //     current.rotation = signedAngleBetweenVectors(pivot, initial.mouse, mouse)
        //     const {dx, dy} = rotateAroundPointIncremental(current.x.min, current.y.min, pivot, current.rotation)
        //     // delta_x -= dx
        //     // delta_y -= dy
        //     // console.log(dx, dy)
        //   }

        //   // This is probably unnecessary since initial rotation is 0
        //   const delta_rot = current.rotation - initial.rotation

        //   // Calculate the scaling factor based on the difference between current and initial values
        //   const scale_x_ratio = (current.x.max - current.x.min) / (initial.x.max - initial.x.min);
        //   const scale_y_ratio = (current.y.max - current.y.min) / (initial.y.max - initial.y.min);

        //   for (let idx in initialSelection) {
        //     let item = context.activeObject.currentFrame.keys[idx]
        //     let xoffset = initialSelection[idx].x - initial.x.min
        //     let yoffset = initialSelection[idx].y - initial.y.min
        //     item.x = initial.x.min + delta_x + xoffset * scale_x_ratio
        //     item.y = initial.y.min + delta_y + yoffset * scale_y_ratio
        //     item.scale_x = initialSelection[idx].scale_x * scale_x_ratio
        //     item.scale_y = initialSelection[idx].scale_y * scale_y_ratio
        //     item.rotation = initialSelection[idx].rotation + delta_rot
        //   }
        // }
        if (context.activeAction) {
          actions[context.activeAction.type].update(
            context.activeAction,
            mouse,
          );
        }
        break;
      default:
        break;
    }
    updateUI();
  });
  stage.addEventListener("dblclick", (e) => {
    context.mouseDown = false;
    context.dragging = false;
    context.dragDirection = undefined;
    context.selectionRect = undefined;
    let mouse = getMousePos(stage, e);
    mouse = context.activeObject.transformMouse(mouse);
    modeswitcher: switch (mode) {
      case "select":
        for (let i = context.activeObject.activeLayer.children.length - 1; i >= 0; i--) {
          let child = context.activeObject.activeLayer.children[i];
          if (!(child.idx in context.activeObject.currentFrame.keys)) continue;
          if (hitTest(mouse, child)) {
            context.objectStack.push(child);
            context.selection = [];
            context.shapeselection = [];
            updateUI();
            updateLayers();
            updateMenu();
            updateInfopanel();
            break modeswitcher;
          }
        }
        // we didn't click on a child, go up a level
        if (context.activeObject.parent) {
          context.selection = [context.activeObject];
          context.activeObject.setFrameNum(0);
          context.shapeselection = [];
          context.objectStack.pop();
          updateUI();
          updateLayers();
          updateMenu();
          updateInfopanel();
        }
        break;
      default:
        break;
    }
  });
  return stage;
}

function toolbar() {
  let tools_scroller = document.createElement("div");
  tools_scroller.className = "toolbar";
  for (let tool in tools) {
    let toolbtn = document.createElement("button");
    toolbtn.className = "toolbtn";
    let icon = document.createElement("img");
    icon.className = "icon";
    icon.src = tools[tool].icon;
    toolbtn.appendChild(icon);
    tools_scroller.appendChild(toolbtn);
    toolbtn.addEventListener("click", () => {
      mode = tool;
      updateInfopanel();
      updateUI();
      console.log(`Switched tool to ${tool}`);
    });
  }
  let tools_break = document.createElement("div");
  tools_break.className = "horiz_break";
  tools_scroller.appendChild(tools_break);
  let fillColor = document.createElement("div");
  let strokeColor = document.createElement("div");
  fillColor.className = "color-field";
  strokeColor.className = "color-field";
  fillColor.classList.add("fill")
  strokeColor.classList.add("stroke")
  fillColor.setColor = (hsv, alpha) => {
    const rgb = hsvToRgb(...hsv)
    const color = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha
    fillColor.style.setProperty("--color", color);
    fillColor.color = color;
    fillColor.hsv = hsv
    fillColor.alpha = alpha
    context.fillStyle = color;
  };
  strokeColor.setColor = (hsv, alpha) => {
    const rgb = hsvToRgb(...hsv)
    const color = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha
    strokeColor.style.setProperty("--color", color);
    strokeColor.color = color;
    strokeColor.hsv = hsv
    strokeColor.alpha = alpha
    context.strokeStyle = color;
  };
  fillColor.setColor([0, 1, 1], 'ff');
  strokeColor.setColor([0,0,0], 'ff');
  fillColor.style.setProperty("--label-text", `"Fill color:"`);
  strokeColor.style.setProperty("--label-text", `"Stroke color:"`);
  fillColor.type = "color";
  fillColor.value = "#ff0000";
  strokeColor.value = "#000000";
  let evtListener;
  let padding = 10;
  let gradwidth = 25;
  let ccwidth = 300;
  let mainSize = ccwidth - (3 * padding + gradwidth);
  let colorClickHandler = (e) => {
    let colorCvs = document.querySelector("#color-cvs");
    if (colorCvs == null) {
      console.log("creating new one");
      colorCvs = document.createElement("canvas");
      colorCvs.id = "color-cvs";
      document.body.appendChild(colorCvs);
      colorCvs.width = ccwidth;
      colorCvs.height = 500;
      colorCvs.style.width = "300px";
      colorCvs.style.height = "500px";
      colorCvs.style.position = "absolute";
      colorCvs.style.left = "500px";
      colorCvs.style.top = "500px";
      colorCvs.style.boxShadow = "0 2px 2px rgba(0,0,0,0.2)";
      colorCvs.style.cursor = "crosshair";
      colorCvs.currentColor = "#00ffba88";
      colorCvs.currentHSV = [0,0,0]
      colorCvs.currentAlpha = 1

      colorCvs.colorSelectorWidget = new ColorSelectorWidget(0, 0, colorCvs)

      colorCvs.draw = function () {
        let ctx = colorCvs.getContext("2d");
        colorCvs.colorSelectorWidget.draw(ctx)

      };
      colorCvs.addEventListener("pointerdown", (e) => {
        colorCvs.clickedMainGradient = false;
        colorCvs.clickedHueGradient = false;
        colorCvs.clickedAlphaGradient = false;
        let mouse = getMousePos(colorCvs, e);
        colorCvs.colorSelectorWidget.handleMouseEvent("mousedown", mouse.x, mouse.y)
        colorCvs.colorEl.setColor(colorCvs.currentHSV, colorCvs.currentAlpha);
        colorCvs.draw();
      });

      window.addEventListener("pointerup", (e) => {
        let mouse = getMousePos(colorCvs, e);
        colorCvs.clickedMainGradient = false;
        colorCvs.clickedHueGradient = false;
        colorCvs.clickedAlphaGradient = false;

        colorCvs.colorSelectorWidget.handleMouseEvent("mouseup", mouse.x, mouse.y)
        if (e.target != colorCvs) {
          colorCvs.style.display = "none";
          window.removeEventListener("pointermove", evtListener);
        }
      });
    } else {
      colorCvs.style.display = "block";
    }
    evtListener = window.addEventListener("pointermove", (e) => {
      let mouse = getMousePos(colorCvs, e);
      colorCvs.colorSelectorWidget.handleMouseEvent("mousemove", mouse.x, mouse.y)
      colorCvs.draw()
      colorCvs.colorEl.setColor(colorCvs.currentHSV, colorCvs.currentAlpha);
    });
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
    colorCvs.colorEl = e.target;
    colorCvs.currentColor = e.target.color;
    colorCvs.currentHSV = e.target.hsv;
    colorCvs.currentAlpha = e.target.alpha
    colorCvs.draw();
    e.preventDefault();
  };
  fillColor.addEventListener("click", colorClickHandler);
  strokeColor.addEventListener("click", colorClickHandler);
  // Fill and stroke colors use the same set of swatches
  fillColor.addEventListener("change", (e) => {
    context.swatches.unshift(fillColor.value);
    if (context.swatches.length > 12) context.swatches.pop();
  });
  strokeColor.addEventListener("change", (e) => {
    context.swatches.unshift(strokeColor.value);
    if (context.swatches.length > 12) context.swatches.pop();
  });
  tools_scroller.appendChild(fillColor);
  tools_scroller.appendChild(strokeColor);
  return tools_scroller;
}

function timeline() {
  let timeline_cvs = document.createElement("canvas");
  timeline_cvs.className = "timeline";

  // Start building widget hierarchy
  timeline_cvs.timelinewindow = new TimelineWindow(0, 0, context)

  // Load icons for show/hide layer
  timeline_cvs.icons = {};
  timeline_cvs.icons.volume_up_fill = new Icon("assets/volume-up-fill.svg");
  timeline_cvs.icons.volume_mute = new Icon("assets/volume-mute.svg");
  timeline_cvs.icons.eye_fill = new Icon("assets/eye-fill.svg");
  timeline_cvs.icons.eye_slash = new Icon("assets/eye-slash.svg");

  // Variable to store the last time updateTimelineCanvasSize was called
  let lastResizeTime = 0;
  const throttleIntervalMs = 20;

  function updateTimelineCanvasSize() {
    const canvasStyles = window.getComputedStyle(timeline_cvs);

    timeline_cvs.width = parseInt(canvasStyles.width);
    timeline_cvs.height = parseInt(canvasStyles.height);
    updateLayers();
    renderAll();
  }

  // Set up ResizeObserver to watch for changes in the canvas size
  const resizeObserver = new ResizeObserver(() => {
    const currentTime = Date.now();

    // Only call updateTimelineCanvasSize if enough time has passed since the last call
    // This prevents error messages about a ResizeObserver loop
    if (currentTime - lastResizeTime > throttleIntervalMs) {
      lastResizeTime = currentTime;
      updateTimelineCanvasSize();
    }
  });
  resizeObserver.observe(timeline_cvs);

  timeline_cvs.frameDragOffset = {
    frames: 0,
    layers: 0,
  };

  timeline_cvs.addEventListener("dragstart", (event) => {
    event.preventDefault();
  });
  timeline_cvs.addEventListener("wheel", (event) => {
    event.preventDefault();
    const deltaX = event.deltaX * config.scrollSpeed;
    const deltaY = event.deltaY * config.scrollSpeed;

    let maxScroll =
      context.activeObject.layers.length * layerHeight +
      context.activeObject.audioLayers.length * layerHeight +
      gutterHeight -
      timeline_cvs.height;

    timeline_cvs.offsetX = Math.max(0, timeline_cvs.offsetX + deltaX);
    timeline_cvs.offsetY = Math.max(
      0,
      Math.min(maxScroll, timeline_cvs.offsetY + deltaY),
    );
    timeline_cvs.timelinewindow.offsetX = -timeline_cvs.offsetX
    timeline_cvs.timelinewindow.offsetY = -timeline_cvs.offsetY

    const currentTime = Date.now();
    if (currentTime - lastResizeTime > throttleIntervalMs) {
      lastResizeTime = currentTime;
      updateLayers();
    }
  });
  timeline_cvs.addEventListener("pointerdown", (e) => {
    let mouse = getMousePos(timeline_cvs, e, true, true);
    mouse.y += timeline_cvs.offsetY;
    if (mouse.x > layerWidth) {
      mouse.x -= layerWidth;
      mouse.x += timeline_cvs.offsetX;
      mouse.y -= gutterHeight;
      timeline_cvs.clicked_frame = Math.floor(mouse.x / frameWidth);
      context.activeObject.setFrameNum(timeline_cvs.clicked_frame);
      const layerIdx = Math.floor(mouse.y / layerHeight);
      if (layerIdx < context.activeObject.layers.length && layerIdx >= 0) {
        const layer =
          context.activeObject.layers[
            context.activeObject.layers.length - layerIdx - 1
          ];

        const frame = layer.getFrame(timeline_cvs.clicked_frame);
        if (frame.exists) {
          console.log(frame.keys)
          if (!e.shiftKey) {
            // Check if the clicked frame is already in the selection
            const existingIndex = context.selectedFrames.findIndex(
              (selected) =>
                selected.frameNum === timeline_cvs.clicked_frame &&
                selected.layer === layerIdx,
            );

            if (existingIndex !== -1) {
              if (!e.ctrlKey) {
                // Do nothing
              } else {
                // Remove the clicked frame from the selection
                context.selectedFrames.splice(existingIndex, 1);
              }
            } else {
              if (!e.ctrlKey) {
                context.selectedFrames = []; // Reset selection
              }
              // Add the clicked frame to the selection
              context.selectedFrames.push({
                layer: layerIdx,
                frameNum: timeline_cvs.clicked_frame,
              });
            }
          } else {
            const currentSelection =
              context.selectedFrames[context.selectedFrames.length - 1];

            const startFrame = Math.min(
              currentSelection.frameNum,
              timeline_cvs.clicked_frame,
            );
            const endFrame = Math.max(
              currentSelection.frameNum,
              timeline_cvs.clicked_frame,
            );

            const startLayer = Math.min(currentSelection.layer, layerIdx);
            const endLayer = Math.max(currentSelection.layer, layerIdx);

            for (let l = startLayer; l <= endLayer; l++) {
              const layerToAdd =
                context.activeObject.layers[
                  context.activeObject.layers.length - l - 1
                ];

              for (let f = startFrame; f <= endFrame; f++) {
                const frameToAdd = layerToAdd.getFrame(f);

                if (
                  frameToAdd.exists &&
                  !context.selectedFrames.some(
                    (selected) =>
                      selected.frameNum === f && selected.layer === l,
                  )
                ) {
                  context.selectedFrames.push({
                    layer: l,
                    frameNum: f,
                  });
                }
              }
            }
          }
          timeline_cvs.draggingFrames = true;
          timeline_cvs.dragFrameStart = {
            frame: timeline_cvs.clicked_frame,
            layer: layerIdx,
          };
          timeline_cvs.frameDragOffset = {
            frames: 0,
            layers: 0,
          };
        } else {
          context.selectedFrames = [];
        }
      } else {
        context.selectedFrames = [];
      }
      updateUI();
    } else {
      mouse.y -= gutterHeight;
      let l = Math.floor(mouse.y / layerHeight);
      if (l < context.activeObject.allLayers.length) {
        let i = context.activeObject.allLayers.length - (l + 1);
        mouse.y -= l * layerHeight;
        if (
          mouse.x > layerWidth - iconSize - 5 &&
          mouse.x < layerWidth - 5 &&
          mouse.y > 0.5 * (layerHeight - iconSize) &&
          mouse.y < 0.5 * (layerHeight + iconSize)
        ) {
          context.activeObject.allLayers[i].visible =
            !context.activeObject.allLayers[i].visible;
          updateUI();
          updateMenu();
        } else if (
          mouse.x > layerWidth - iconSize * 2 - 10 &&
          mouse.x < layerWidth - iconSize - 5 &&
          mouse.y > 0.5 * (layerHeight - iconSize) &&
          mouse.y < 0.5 * (layerHeight + iconSize)
        ) {
          context.activeObject.allLayers[i].audible =
            !context.activeObject.allLayers[i].audible;
          updateUI();
          updateMenu();
        } else {
          context.activeObject.currentLayer = i - context.activeObject.audioLayers.length;
        }
      }
    }
    updateLayers();
  });
  timeline_cvs.addEventListener("pointerup", (e) => {
    let mouse = getMousePos(timeline_cvs, e, true, true);
    mouse.y += timeline_cvs.offsetY;
    if (mouse.x > layerWidth || timeline_cvs.draggingFrames) {
      mouse.x += timeline_cvs.offsetX - layerWidth;
      if (timeline_cvs.draggingFrames) {
        if (
          timeline_cvs.frameDragOffset.frames != 0 ||
          timeline_cvs.frameDragOffset.layers != 0
        ) {
          actions.moveFrames.create(timeline_cvs.frameDragOffset);
          context.selectedFrames = [];
        }
      }
      timeline_cvs.draggingFrames = false;

      updateLayers();
      updateMenu();
    }
  });
  timeline_cvs.addEventListener("pointermove", (e) => {
    let mouse = getMousePos(timeline_cvs, e, true, true);
    mouse.y += timeline_cvs.offsetY;
    if (mouse.x > layerWidth || timeline_cvs.draggingFrames) {
      mouse.x += timeline_cvs.offsetX - layerWidth;
      if (timeline_cvs.draggingFrames) {
        const minFrameNum = -Math.min(
          ...context.selectedFrames.map((selection) => selection.frameNum),
        );
        const minLayer = -Math.min(
          ...context.selectedFrames.map((selection) => selection.layer),
        );
        const maxLayer =
          context.activeObject.layers.length -
          1 -
          Math.max(
            ...context.selectedFrames.map((selection) => selection.layer),
          );
        timeline_cvs.frameDragOffset = {
          frames: Math.max(
            Math.floor(mouse.x / frameWidth) -
              timeline_cvs.dragFrameStart.frame,
            minFrameNum,
          ),
          layers: Math.min(
            Math.max(
              Math.floor(mouse.y / layerHeight) -
                timeline_cvs.dragFrameStart.layer,
              minLayer,
            ),
            maxLayer,
          ),
        };
        updateLayers();
      }
    }
  });

  timeline_cvs.offsetX = 0;
  timeline_cvs.offsetY = 0;
  updateTimelineCanvasSize();
  return timeline_cvs;
}

function infopanel() {
  let panel = document.createElement("div");
  panel.className = "infopanel";
  updateInfopanel();
  return panel;
}

function outliner(object = undefined) {
  let outliner = document.createElement("canvas");
  outliner.className = "outliner";
  if (object == undefined) {
    outliner.object = root;
  } else {
    outliner.object = object;
  }
  outliner.style.cursor = "pointer";

  let lastResizeTime = 0;
  const throttleIntervalMs = 20;

  function updateTimelineCanvasSize() {
    const canvasStyles = window.getComputedStyle(outliner);

    outliner.width = parseInt(canvasStyles.width);
    outliner.height = parseInt(canvasStyles.height);
    updateOutliner();
    renderAll();
  }

  // Set up ResizeObserver to watch for changes in the canvas size
  const resizeObserver = new ResizeObserver(() => {
    const currentTime = Date.now();

    // Only call updateTimelineCanvasSize if enough time has passed since the last call
    // This prevents error messages about a ResizeObserver loop
    if (currentTime - lastResizeTime > throttleIntervalMs) {
      lastResizeTime = currentTime;
      updateTimelineCanvasSize();
    }
  });
  resizeObserver.observe(outliner);

  outliner.collapsed = {};
  outliner.offsetX = 0;
  outliner.offsetY = 0;

  outliner.addEventListener("click", function (e) {
    const mouse = getMousePos(outliner, e);
    const mouseY = mouse.y; // Get the Y position of the click
    const mouseX = mouse.x; // Get the X position (not used here, but can be used to check clicked area)

    // Iterate again to check which object was clicked
    let currentY = 20; // Starting y position
    const stack = [{ object: outliner.object, indent: 0 }];

    while (stack.length > 0) {
      const { object, indent } = stack.pop();

      // Check if the click was on this object
      if (mouseY >= currentY - 20 && mouseY <= currentY) {
        if (mouseX >= 0 && mouseX <= indent + 2 * triangleSize) {
          // Toggle the collapsed state of the object
          outliner.collapsed[object.idx] = !outliner.collapsed[object.idx];
        } else {
          outliner.active = object;
          // Only do selection when this is pointing at the actual file
          if (outliner.object==root) {
            context.objectStack = []
            let parent = object;
            while (true) {
              if (parent.parent) {
                parent = parent.parent
                context.objectStack.unshift(parent)
              } else {
                break
              }
            }
            if (context.objectStack.length==0) {
              context.objectStack.push(root)
            }
            context.oldselection = context.selection
            context.oldshapeselection = context.shapeselection
            context.selection = [object]
            context.shapeselection = []
            actions.select.create()
          }
        }
        updateOutliner(); // Re-render the outliner
        return;
      }

      // Update the Y position for the next object
      currentY += 20;

      // If the object is collapsed, skip it
      if (outliner.collapsed[object.idx]) {
        continue;
      }

      // If the object has layers, add them to the stack
      if (object.layers) {
        for (let i = object.layers.length - 1; i >= 0; i--) {
          const layer = object.layers[i];
          stack.push({ object: layer, indent: indent + 20 });
        }
      } else if (object.children) {
        for (let i = object.children.length - 1; i >= 0; i--) {
          const child = object.children[i];
          stack.push({ object: child, indent: indent + 40 });
        }
      }
    }
  });

  outliner.addEventListener("wheel", (event) => {
    event.preventDefault();
    const deltaY = event.deltaY * config.scrollSpeed;

    outliner.offsetY = Math.max(0, outliner.offsetY + deltaY);

    const currentTime = Date.now();
    if (currentTime - lastResizeTime > throttleIntervalMs) {
      lastResizeTime = currentTime;
      updateOutliner();
    }
  });

  return outliner;
}

async function startup() {
  await loadConfig();
  createNewFileDialog(_newFile, _open, config);
  if (!window.openedFiles?.length) {
    if (config.reopenLastSession && config.recentFiles?.length) {
      document.body.style.cursor = "wait"
      setTimeout(()=>_open(config.recentFiles[0]), 10)
    } else {
      showNewFileDialog(config);
    }
  }
}

startup();

function createPaneMenu(div) {
  const menuItems = ["Item 1", "Item 2", "Item 3"]; // The items for the menu

  // Get the menu container (create a new div for the menu)
  const popupMenu = document.createElement("div");
  popupMenu.id = "popupMenu"; // Set the ID to ensure we can target it later

  // Create a <ul> element to hold the list items
  const ul = document.createElement("ul");

  // Loop through the menuItems array and create a <li> for each item
  for (let pane in panes) {
    const li = document.createElement("li");
    // Create the <img> element for the icon
    const img = document.createElement("img");
    img.src = `assets/${panes[pane].name}.svg`; // Use the appropriate SVG as the source
    // img.style.width = "20px";  // Set the icon size
    // img.style.height = "20px";  // Set the icon size
    // img.style.marginRight = "10px";  // Add space between the icon and text

    // Append the image to the <li> element
    li.appendChild(img);

    // Set the text of the item
    li.appendChild(document.createTextNode(titleCase(panes[pane].name)));
    li.addEventListener("click", () => {
      createPane(panes[pane], div);
      updateUI();
      updateLayers();
      updateAll();
      popupMenu.remove();
    });
    ul.appendChild(li); // Append the <li> to the <ul>
  }

  popupMenu.appendChild(ul); // Append the <ul> to the popupMenu div
  document.body.appendChild(popupMenu); // Append the menu to the body
  return popupMenu; // Return the created menu element
}

function createPane(paneType = undefined, div = undefined) {
  if (!div) {
    div = document.createElement("div");
  } else {
    div.textContent = "";
  }
  let header = document.createElement("div");
  if (!paneType) {
    paneType = panes.stage; // TODO: change based on type
  }
  let content = paneType.func();
  header.className = "header";

  let button = document.createElement("button");
  header.appendChild(button);
  let icon = document.createElement("img");
  icon.className = "icon";
  icon.src = `/assets/${paneType.name}.svg`;
  button.appendChild(icon);
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
  });

  div.className = "vertical-grid";
  header.style.height = "calc( 2 * var(--lineheight))";
  content.style.height = "calc( 100% - 2 * var(--lineheight) )";
  div.appendChild(header);
  div.appendChild(content);
  return div;
}

function splitPane(div, percent, horiz, newPane = undefined) {
  let content = div.firstElementChild;
  let div1 = document.createElement("div");
  let div2 = document.createElement("div");

  div1.className = "panecontainer";
  div2.className = "panecontainer";

  div1.appendChild(content);
  if (newPane) {
    div2.appendChild(newPane);
  } else {
    div2.appendChild(createPane());
  }
  div.appendChild(div1);
  div.appendChild(div2);

  if (horiz) {
    div.className = "horizontal-grid";
  } else {
    div.className = "vertical-grid";
  }
  div.setAttribute("lb-percent", percent); // TODO: better attribute name
  div.addEventListener("pointerdown", function (event) {
    // Check if the clicked element is the parent itself and not a child element
    if (event.target === event.currentTarget) {
      if (event.button === 0) {
        // Left click
        event.currentTarget.setAttribute("dragging", true);
        event.currentTarget.style.userSelect = "none";
        rootPane.style.userSelect = "none";
      }
    } else {
      event.currentTarget.setAttribute("dragging", false);
    }
  });
  div.addEventListener("contextmenu", async function (event) {
    if (event.target === event.currentTarget) {
      event.preventDefault(); // Prevent the default context menu from appearing
      event.stopPropagation();

      function createSplit(direction) {
        let splitIndicator = document.createElement("div");
        splitIndicator.className = "splitIndicator";
        splitIndicator.style.flexDirection =
          direction == "vertical" ? "column" : "row";
        document.body.appendChild(splitIndicator);
        splitIndicator.addEventListener("pointermove", (e) => {
          const { clientX: mouseX, clientY: mouseY } = e;
          const rect = splitIndicator.getBoundingClientRect();

          // Create child elements and divider if not already present
          let firstHalf = splitIndicator.querySelector(".first-half");
          let secondHalf = splitIndicator.querySelector(".second-half");
          let divider = splitIndicator.querySelector(".divider");

          if (!firstHalf || !secondHalf || !divider) {
            firstHalf = document.createElement("div");
            secondHalf = document.createElement("div");
            divider = document.createElement("div");
            firstHalf.classList.add("first-half");
            secondHalf.classList.add("second-half");
            divider.classList.add("divider");
            splitIndicator.innerHTML = ""; // Clear previous children
            splitIndicator.append(firstHalf, divider, secondHalf);
          }

          const isVertical = direction === "vertical";

          // Calculate dimensions for halves
          const [first, second] = isVertical
            ? [mouseY - rect.top, rect.bottom - mouseY]
            : [mouseX - rect.left, rect.right - mouseX];

          const firstSize = `${first}px`;
          const secondSize = `${second}px`;

          splitIndicator.percent = isVertical
            ? ((mouseY - rect.top) / (rect.bottom - rect.top)) * 100
            : ((mouseX - rect.left) / (rect.right - rect.left)) * 100;

          // Apply styles for first and second halves
          firstHalf.style[isVertical ? "height" : "width"] = firstSize;
          secondHalf.style[isVertical ? "height" : "width"] = secondSize;
          firstHalf.style[isVertical ? "width" : "height"] = "100%";
          secondHalf.style[isVertical ? "width" : "height"] = "100%";

          // Apply divider styles
          divider.style.backgroundColor = "#000";
          if (isVertical) {
            divider.style.height = "2px";
            divider.style.width = "100%";
            divider.style.left = `${mouseX - rect.left}px`;
          } else {
            divider.style.width = "2px";
            divider.style.height = "100%";
            divider.style.top = `${mouseY - rect.top}px`;
          }
        });
        splitIndicator.addEventListener("click", (e) => {
          if (splitIndicator.percent) {
            splitPane(
              splitIndicator.targetElement,
              splitIndicator.percent,
              direction == "horizontal",
              createPane(panes.timeline),
            );
            document.body.removeChild(splitIndicator);
            document.removeEventListener("pointermove", splitListener);
            setTimeout(updateUI, 20);
          }
        });

        const splitListener = document.addEventListener("pointermove", (e) => {
          const mouseX = e.clientX;
          const mouseY = e.clientY;

          // Get all elements under the mouse pointer
          const elementsUnderMouse = document.querySelectorAll(":hover");

          let targetElement = null;
          for (let element of elementsUnderMouse) {
            if (
              element.matches(
                ".horizontal-grid > .panecontainer, .vertical-grid > .panecontainer",
              )
            ) {
              targetElement = element;
            }
          }
          if (targetElement) {
            const rect = targetElement.getBoundingClientRect();
            splitIndicator.style.left = `${rect.left}px`;
            splitIndicator.style.top = `${rect.top}px`;
            splitIndicator.style.width = `${rect.width}px`;
            splitIndicator.style.height = `${rect.height}px`;

            splitIndicator.targetElement = targetElement;
          }
        });
      }
      // TODO: use icon menu items
      // See https://github.com/tauri-apps/tauri/blob/dev/packages/api/src/menu/iconMenuItem.ts
      const menu = await Menu.new({
        items: [
          { id: "ctx_option0", text: "Area options", enabled: false },
          {
            id: "ctx_option1",
            text: "Vertical Split",
            action: () => createSplit("vertical"),
          },
          {
            id: "ctx_option2",
            text: "Horizontal Split",
            action: () => createSplit("horizontal"),
          },
          new PredefinedMenuItem("Separator"),
          { id: "ctx_option3", text: horiz ? "Join Left" : "Join Up" },
          { id: "ctx_option4", text: horiz ? "Join Right" : "Join Down" },
        ],
      });
      menu.popup({ x: event.clientX, y: event.clientY });
    }

    console.log("Right-click on the element");
    // Your custom logic here
  });
  div.addEventListener("pointermove", function (event) {
    // Check if the clicked element is the parent itself and not a child element
    if (event.currentTarget.getAttribute("dragging") == "true") {
      const frac = getMousePositionFraction(event, event.currentTarget);
      div.setAttribute("lb-percent", frac * 100);
      updateAll();
    }
  });
  div.addEventListener("pointerup", (event) => {
    event.currentTarget.setAttribute("dragging", false);
    // event.currentTarget.style.userSelect = 'auto';
  });
  updateAll();
  updateUI();
  updateLayers();
  return [div1, div2];
}

function updateAll() {
  updateLayout(rootPane);
  for (let element of layoutElements) {
    updateLayout(element);
  }
}

function updateLayout(element) {
  let rect = element.getBoundingClientRect();
  let percent = element.getAttribute("lb-percent");
  percent ||= 50;
  let children = element.children;
  if (children.length != 2) return;
  if (element.classList.contains("horizontal-grid")) {
    children[0].style.width = `${(rect.width * percent) / 100}px`;
    children[1].style.width = `${(rect.width * (100 - percent)) / 100}px`;
    children[0].style.height = `${rect.height}px`;
    children[1].style.height = `${rect.height}px`;
  } else if (element.classList.contains("vertical-grid")) {
    children[0].style.height = `${(rect.height * percent) / 100}px`;
    children[1].style.height = `${(rect.height * (100 - percent)) / 100}px`;
    children[0].style.width = `${rect.width}px`;
    children[1].style.width = `${rect.width}px`;
  }
  if (children[0].getAttribute("lb-percent")) {
    updateLayout(children[0]);
  }
  if (children[1].getAttribute("lb-percent")) {
    updateLayout(children[1]);
  }
}

function updateUI() {
  uiDirty = true;
}

function renderUI() {
  for (let canvas of canvases) {
    let ctx = canvas.getContext("2d");
    ctx.resetTransform();
    ctx.beginPath();
    ctx.fillStyle = backgroundColor;
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    ctx.translate(-canvas.offsetX, -canvas.offsetY);
    ctx.scale(context.zoomLevel, context.zoomLevel);

    ctx.fillStyle = "white";
    ctx.fillRect(0, 0, config.fileWidth, config.fileHeight);

    context.ctx = ctx;
    // root.draw(context);
    root.draw(ctx)
    if (context.activeObject != root) {
      ctx.fillStyle = "rgba(255,255,255,0.5)";
      ctx.fillRect(0, 0, config.fileWidth, config.fileHeight);
      const transform = ctx.getTransform()
      context.activeObject.transformCanvas(ctx)
      context.activeObject.draw(ctx);
      ctx.setTransform(transform)
    }
    if (context.activeShape) {
      context.activeShape.draw(context);
    }

    ctx.save()
    context.activeObject.transformCanvas(ctx)
    // Debug rendering
    if (debugQuadtree) {
      ctx.fillStyle = "rgba(255,255,255,0.5)";
      ctx.fillRect(0, 0, config.fileWidth, config.fileHeight);
      const ep = 2.5;
      const bbox = {
        x: { min: context.mousePos.x - ep, max: context.mousePos.x + ep },
        y: { min: context.mousePos.y - ep, max: context.mousePos.y + ep },
      };
      debugCurves = [];
      for (let shape of context.activeObject.currentFrame.shapes) {
        for (let i of shape.quadtree.query(bbox)) {
          debugCurves.push(shape.curves[i]);
        }
      }

    }
    // let i=4;
    for (let curve of debugCurves) {
      ctx.beginPath();
      // ctx.strokeStyle = `#ff${i}${i}${i}${i}`
      // i = (i+3)%10
      ctx.strokeStyle = "#" + ((Math.random() * 0xffffff) << 0).toString(16);
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
      ctx.beginPath();
      let bbox = curve.bbox();
      ctx.rect(
        bbox.x.min,
        bbox.y.min,
        bbox.x.max - bbox.x.min,
        bbox.y.max - bbox.y.min,
      );
      ctx.stroke();
    }
    let i = 0;
    for (let point of debugPoints) {
      ctx.beginPath();
      let j = i.toString(16).padStart(2, "0");
      ctx.fillStyle = `#${j}ff${j}`;
      i += 1;
      i %= 255;
      ctx.arc(point.x, point.y, 3, 0, 2 * Math.PI);
      ctx.fill();
    }
    ctx.restore()
    if (context.activeAction) {
      actions[context.activeAction.type].render(context.activeAction, ctx);
    }
  }
  for (let selectionRect of document.querySelectorAll(".selectionRect")) {
    selectionRect.style.display = "none";
  }
  if (mode == "transform") {
    if (context.selection.length > 0) {
      for (let selectionRect of document.querySelectorAll(".selectionRect")) {
        let bbox = undefined;
        for (let item of context.selection) {
          if (bbox == undefined) {
            bbox = structuredClone(item.bbox());
          } else {
            growBoundingBox(bbox, item.bbox());
          }
        }
        if (bbox != undefined) {
          selectionRect.style.display = "block";
          selectionRect.style.left = `${bbox.x.min}px`;
          selectionRect.style.top = `${bbox.y.min}px`;
          selectionRect.style.width = `${bbox.x.max - bbox.x.min}px`;
          selectionRect.style.height = `${bbox.y.max - bbox.y.min}px`;
        }
      }
    }
  }
}

function updateLayers() {
  layersDirty = true;
}

function renderLayers() {
  for (let canvas of document.querySelectorAll(".timeline")) {
    const width = canvas.width;
    const height = canvas.height;
    const ctx = canvas.getContext("2d");
    const offsetX = canvas.offsetX;
    const offsetY = canvas.offsetY;
    const frameCount = (width + offsetX - layerWidth) / frameWidth;
    ctx.fillStyle = backgroundColor;
    ctx.fillRect(0, 0, width, height);
    ctx.lineWidth = 1;


    ctx.save()
    ctx.translate(layerWidth, gutterHeight)
    canvas.timelinewindow.width = width - layerWidth
    canvas.timelinewindow.height = height - gutterHeight
    canvas.timelinewindow.draw(ctx)
    ctx.restore()

    // Draw timeline top
    ctx.save();
    ctx.save();
    ctx.beginPath();
    ctx.rect(layerWidth, 0, width - layerWidth, height);
    ctx.clip();
    ctx.translate(layerWidth - offsetX, 0);
    ctx.fillStyle = labelColor;
    for (
      let j = Math.floor(offsetX / (5 * frameWidth)) * 5;
      j < frameCount + 1;
      j += 5
    ) {
      drawCenteredText(
        ctx,
        j.toString(),
        (j - 0.5) * frameWidth,
        gutterHeight / 2,
        gutterHeight,
      );
    }
    ctx.restore();
    ctx.translate(0, gutterHeight);
    ctx.strokeStyle = shadow;
    ctx.beginPath();
    ctx.moveTo(layerWidth, 0);
    ctx.lineTo(layerWidth, height);
    ctx.stroke();

    ctx.save();
    ctx.rect(0, 0, width, height);
    ctx.clip();
    ctx.translate(0, -offsetY);
    // Draw layer headers
    let i = 0;
    for (let k = context.activeObject.allLayers.length - 1; k >= 0; k--) {
      let layer = context.activeObject.allLayers[k];
      if (context.activeObject.activeLayer == layer) {
        ctx.fillStyle = darkMode ? "#444" : "#ccc";
      } else {
        ctx.fillStyle = darkMode ? "#222" : "#aaa";
      }
      drawBorderedRect(
        ctx,
        0,
        i * layerHeight,
        layerWidth,
        layerHeight,
        highlight,
        shadow,
      );
      ctx.fillStyle = darkMode ? "white" : "black";
      drawHorizontallyCenteredText(
        ctx,
        layer.name,
        5,
        (i + 0.5) * layerHeight,
        layerHeight * 0.4,
      );
      ctx.save();
      const visibilityIcon = layer.visible
        ? canvas.icons.eye_fill
        : canvas.icons.eye_slash;
      visibilityIcon.render(
        ctx,
        layerWidth - iconSize - 5,
        (i + 0.5) * layerHeight - iconSize * 0.5,
        iconSize,
        iconSize,
        labelColor,
      );
      const audibilityIcon = layer.audible
        ? canvas.icons.volume_up_fill
        : canvas.icons.volume_mute;
      audibilityIcon.render(
        ctx,
        layerWidth - iconSize * 2 - 10,
        (i + 0.5) * layerHeight - iconSize * 0.5,
        iconSize,
        iconSize,
        labelColor,
      );
      ctx.restore();

      // ctx.save();
      // ctx.beginPath();
      // ctx.rect(layerWidth, i * layerHeight, width, layerHeight);
      // ctx.clip();
      // ctx.translate(layerWidth - offsetX, i * layerHeight);
      // // Draw empty frames
      // for (let j = Math.floor(offsetX / frameWidth); j < frameCount; j++) {
      //   ctx.fillStyle = (j + 1) % 5 == 0 ? shade : backgroundColor;
      //   drawBorderedRect(
      //     ctx,
      //     j * frameWidth,
      //     0,
      //     frameWidth,
      //     layerHeight,
      //     shadow,
      //     highlight,
      //     shadow,
      //     shadow,
      //   );
      // }
      // // Draw existing frames
      // if (layer instanceof Layer) {
      //   for (let j=0; j<layer.frames.length; j++) {
      //     const frameInfo = layer.getFrameValue(j)
      //     if (frameInfo.valueAtN) {
      //       ctx.fillStyle = foregroundColor;
      //       drawBorderedRect(
      //         ctx,
      //         j * frameWidth,
      //         0,
      //         frameWidth,
      //         layerHeight,
      //         highlight,
      //         shadow,
      //         shadow,
      //         shadow,
      //       );
      //       ctx.fillStyle = "#111";
      //       ctx.beginPath();
      //       ctx.arc(
      //         (j + 0.5) * frameWidth,
      //         layerHeight * 0.75,
      //         frameWidth * 0.25,
      //         0,
      //         2 * Math.PI,
      //       );
      //       ctx.fill();
      //       if (frameInfo.valueAtN.keyTypes.has("motion")) {
      //         ctx.strokeStyle = "#7a00b3";
      //         ctx.lineWidth = 2;
      //         ctx.beginPath()
      //         ctx.moveTo(j*frameWidth, layerHeight*0.25)
      //         ctx.lineTo((j+1)*frameWidth, layerHeight*0.25)
      //         ctx.stroke()
      //       }
      //       if (frameInfo.valueAtN.keyTypes.has("shape")) {
      //         ctx.strokeStyle = "#9bff9b";
      //         ctx.lineWidth = 2;
      //         ctx.beginPath()
      //         ctx.moveTo(j*frameWidth, layerHeight*0.35)
      //         ctx.lineTo((j+1)*frameWidth, layerHeight*0.35)
      //         ctx.stroke()
      //       }
      //     } else if (frameInfo.prev && frameInfo.next) {
      //       ctx.fillStyle = foregroundColor;
      //       drawBorderedRect(
      //         ctx,
      //         j * frameWidth,
      //         0,
      //         frameWidth,
      //         layerHeight,
      //         highlight,
      //         shadow,
      //         backgroundColor,
      //         backgroundColor,
      //       );
      //       if (frameInfo.prev.keyTypes.has("motion")) {
      //         ctx.strokeStyle = "#7a00b3";
      //         ctx.lineWidth = 2;
      //         ctx.beginPath()
      //         ctx.moveTo(j*frameWidth, layerHeight*0.25)
      //         ctx.lineTo((j+1)*frameWidth, layerHeight*0.25)
      //         ctx.stroke()
      //       }
      //       if (frameInfo.prev.keyTypes.has("shape")) {
      //         ctx.strokeStyle = "#9bff9b";
      //         ctx.lineWidth = 2;
      //         ctx.beginPath()
      //         ctx.moveTo(j*frameWidth, layerHeight*0.35)
      //         ctx.lineTo((j+1)*frameWidth, layerHeight*0.35)
      //         ctx.stroke()
      //       }
      //     }
      //   }
      //   // layer.frames.forEach((frame, j) => {
      //   //   if (!frame) return;
      //   //   switch (frame.frameType) {
      //   //     case "keyframe":
      //   //       ctx.fillStyle = foregroundColor;
      //   //       drawBorderedRect(
      //   //         ctx,
      //   //         j * frameWidth,
      //   //         0,
      //   //         frameWidth,
      //   //         layerHeight,
      //   //         highlight,
      //   //         shadow,
      //   //         shadow,
      //   //         shadow,
      //   //       );
      //   //       ctx.fillStyle = "#111";
      //   //       ctx.beginPath();
      //   //       ctx.arc(
      //   //         (j + 0.5) * frameWidth,
      //   //         layerHeight * 0.75,
      //   //         frameWidth * 0.25,
      //   //         0,
      //   //         2 * Math.PI,
      //   //       );
      //   //       ctx.fill();
      //   //       break;
      //   //     case "normal":
      //   //       ctx.fillStyle = foregroundColor;
      //   //       drawBorderedRect(
      //   //         ctx,
      //   //         j * frameWidth,
      //   //         0,
      //   //         frameWidth,
      //   //         layerHeight,
      //   //         highlight,
      //   //         shadow,
      //   //         backgroundColor,
      //   //         backgroundColor,
      //   //       );
      //   //       break;
      //   //     case "motion":
      //   //       ctx.fillStyle = "#7a00b3";
      //   //       ctx.fillRect(j * frameWidth, 0, frameWidth, layerHeight);
      //   //       break;
      //   //     case "shape":
      //   //       ctx.fillStyle = "#9bff9b";
      //   //       ctx.fillRect(j * frameWidth, 0, frameWidth, layerHeight);
      //   //       break;
      //   //   }
      //   // });
      // } else if (layer instanceof AudioLayer) {
      //   // TODO: split waveform into chunks
      //   for (let i in layer.sounds) {
      //     let sound = layer.sounds[i];
      //     // layerTrack.appendChild(sound.img)
      //     ctx.drawImage(sound.img, 0, 0);
      //   }
      // }
      // // if (context.activeObject.currentFrameNum)
      // ctx.restore();
      i++;
    }
    ctx.restore();

    // Draw highlighted frame
    ctx.save();
    ctx.translate(layerWidth - offsetX, -offsetY);
    ctx.translate(
      canvas.frameDragOffset.frames * frameWidth,
      canvas.frameDragOffset.layers * layerHeight,
    );
    ctx.globalCompositeOperation = "difference";
    for (let frame of context.selectedFrames) {
      ctx.fillStyle = "grey";
      ctx.fillRect(
        frame.frameNum * frameWidth,
        frame.layer * layerHeight,
        frameWidth,
        layerHeight,
      );
    }
    ctx.globalCompositeOperation = "source-over";
    ctx.restore();

    // Draw scrubber bar
    ctx.save();
    ctx.beginPath();
    ctx.rect(layerWidth, -gutterHeight, width, height);
    ctx.clip();
    ctx.translate(layerWidth - offsetX, 0);
    let frameNum = context.activeObject.currentFrameNum;
    ctx.strokeStyle = scrubberColor;
    ctx.beginPath();
    ctx.moveTo((frameNum + 0.5) * frameWidth, 0);
    ctx.lineTo((frameNum + 0.5) * frameWidth, height);
    ctx.stroke();
    ctx.beginPath();
    ctx.fillStyle = scrubberColor;
    ctx.fillRect(
      frameNum * frameWidth,
      -gutterHeight,
      frameWidth,
      gutterHeight,
    );
    ctx.fillStyle = "white";
    drawCenteredText(
      ctx,
      (frameNum + 1).toString(),
      (frameNum + 0.5) * frameWidth,
      -gutterHeight / 2,
      gutterHeight,
    );
    ctx.restore();
    ctx.restore();
  }
  return;
  for (let container of document.querySelectorAll(".layers-container")) {
    let layerspanel = container.querySelectorAll(".layers")[0];
    let framescontainer = container.querySelectorAll(".frames-container")[0];
    layerspanel.textContent = "";
    framescontainer.textContent = "";
    for (let layer of context.activeObject.layers) {
      let layerHeader = document.createElement("div");
      layerHeader.className = "layer-header";
      if (context.activeObject.activeLayer == layer) {
        layerHeader.classList.add("active");
      }
      layerspanel.appendChild(layerHeader);
      let layerName = document.createElement("div");
      layerName.className = "layer-name";
      layerName.contentEditable = "plaintext-only";
      layerName.addEventListener("click", (e) => {
        e.stopPropagation();
      });
      layerName.addEventListener("blur", (e) => {
        actions.changeLayerName.create(layer, layerName.innerText);
      });
      layerName.innerText = layer.name;
      layerHeader.appendChild(layerName);
      // Visibility icon element
      let visibilityIcon = document.createElement("img");
      visibilityIcon.className = "visibility-icon";
      visibilityIcon.src = layer.visible
        ? "assets/eye-fill.svg"
        : "assets/eye-slash.svg";

      // Toggle visibility on click
      visibilityIcon.addEventListener("click", (e) => {
        e.stopPropagation(); // Prevent click from bubbling to the layerHeader click listener
        layer.visible = !layer.visible;
        // visibilityIcon.src = layer.visible ? "assets/eye-fill.svg" : "assets/eye-slash.svg"
        updateUI();
        updateMenu();
        updateLayers();
      });

      layerHeader.appendChild(visibilityIcon);
      layerHeader.addEventListener("click", (e) => {
        context.activeObject.currentLayer =
          context.activeObject.layers.indexOf(layer);
        updateLayers();
        updateUI();
      });
      let layerTrack = document.createElement("div");
      layerTrack.className = "layer-track";
      if (!layer.visible) {
        layerTrack.classList.add("invisible");
      }
      framescontainer.appendChild(layerTrack);
      layerTrack.addEventListener("click", (e) => {
        let mouse = getMousePos(layerTrack, e);
        let frameNum = parseInt(mouse.x / 25);
        context.activeObject.setFrameNum(frameNum);
        updateLayers();
        updateMenu();
        updateUI();
        updateInfopanel();
      });
      let highlightedFrame = false;
      layer.frames.forEach((frame, i) => {
        let frameEl = document.createElement("div");
        frameEl.className = "frame";
        frameEl.setAttribute("frameNum", i);
        if (i == context.activeObject.currentFrameNum) {
          frameEl.classList.add("active");
          highlightedFrame = true;
        }

        frameEl.classList.add(frame.frameType);
        layerTrack.appendChild(frameEl);
      });
      if (!highlightedFrame) {
        let highlightObj = document.createElement("div");
        let frameCount = layer.frames.length;
        highlightObj.className = "frame-highlight";
        highlightObj.style.left = `${(context.activeObject.currentFrameNum - frameCount) * 25}px`;
        layerTrack.appendChild(highlightObj);
      }
    }
    for (let audioLayer of context.activeObject.audioLayers) {
      let layerHeader = document.createElement("div");
      layerHeader.className = "layer-header";
      layerHeader.classList.add("audio");
      layerspanel.appendChild(layerHeader);
      let layerTrack = document.createElement("div");
      layerTrack.className = "layer-track";
      layerTrack.classList.add("audio");
      framescontainer.appendChild(layerTrack);
      for (let i in audioLayer.sounds) {
        let sound = audioLayer.sounds[i];
        layerTrack.appendChild(sound.img);
      }
      let layerName = document.createElement("div");
      layerName.className = "layer-name";
      layerName.contentEditable = "plaintext-only";
      layerName.addEventListener("click", (e) => {
        e.stopPropagation();
      });
      layerName.addEventListener("blur", (e) => {
        actions.changeLayerName.create(audioLayer, layerName.innerText);
      });
      layerName.innerText = audioLayer.name;
      layerHeader.appendChild(layerName);
    }
  }
}

function updateInfopanel() {
  infopanelDirty = true;
}

function renderInfopanel() {
  for (let panel of document.querySelectorAll(".infopanel")) {
    panel.innerText = "";
    let input;
    let label;
    let span;
    let breadcrumbs = document.createElement("div");
    const bctitle = document.createElement("span");
    bctitle.style.cursor = "default";
    bctitle.textContent = "Context: ";
    breadcrumbs.appendChild(bctitle);
    let crumbs = [];
    for (let object of context.objectStack) {
      crumbs.push({ name: object.name, object: object });
    }
    crumbs.forEach((crumb, index) => {
      const crumbText = document.createElement("span");
      crumbText.textContent = crumb.name;
      breadcrumbs.appendChild(crumbText);

      if (index < crumbs.length - 1) {
        const separator = document.createElement("span");
        separator.textContent = " > ";
        separator.style.cursor = "default";
        crumbText.style.cursor = "pointer";
        breadcrumbs.appendChild(separator);
      } else {
        crumbText.style.cursor = "default";
      }
    });

    breadcrumbs.addEventListener("click", function (event) {
      const span = event.target;

      // Only handle clicks on the breadcrumb text segments (not the separators)
      if (span.tagName === "SPAN" && span.textContent !== " > ") {
        const clickedText = span.textContent;

        // Find the crumb associated with the clicked text
        const crumb = crumbs.find((c) => c.name === clickedText);
        if (crumb) {
          const index = context.objectStack.indexOf(crumb.object);
          if (index !== -1) {
            // Keep only the objects up to the clicked one and add the clicked one as the last item
            context.objectStack = context.objectStack.slice(0, index + 1);
            updateUI();
            updateLayers();
            updateMenu();
            updateInfopanel();
          }
        }
      }
    });
    panel.appendChild(breadcrumbs);
    for (let property in tools[mode].properties) {
      let prop = tools[mode].properties[property];
      label = document.createElement("label");
      label.className = "infopanel-field";
      span = document.createElement("span");
      span.className = "infopanel-label";
      span.innerText = prop.label;
      switch (prop.type) {
        case "number":
          input = document.createElement("input");
          input.className = "infopanel-input";
          input.type = "number";
          input.disabled = prop.enabled == undefined ? false : !prop.enabled();
          if (prop.value) {
            input.value = prop.value.get();
          } else {
            input.value = getProperty(context, property);
          }
          if (prop.min) {
            input.min = prop.min;
          }
          if (prop.max) {
            input.max = prop.max;
          }
          break;
        case "enum":
          input = document.createElement("select");
          input.className = "infopanel-input";
          input.disabled = prop.enabled == undefined ? false : !prop.enabled();
          let optionEl;
          for (let option of prop.options) {
            optionEl = document.createElement("option");
            optionEl.value = option;
            optionEl.innerText = option;
            input.appendChild(optionEl);
          }
          if (prop.value) {
            input.value = prop.value.get();
          } else {
            input.value = getProperty(context, property);
          }
          break;
        case "boolean":
          input = document.createElement("input");
          input.className = "infopanel-input";
          input.type = "checkbox";
          input.disabled = prop.enabled == undefined ? false : !prop.enabled();
          if (prop.value) {
            input.checked = prop.value.get();
          } else {
            input.checked = getProperty(context, property);
          }
          break;
        case "text":
          input = document.createElement("input");
          input.className = "infopanel-input";
          input.disabled = prop.enabled == undefined ? false : !prop.enabled();
          if (prop.value) {
            input.value = prop.value.get();
          } else {
            input.value = getProperty(context, property);
          }
          break;
      }
      input.addEventListener("input", (e) => {
        switch (prop.type) {
          case "number":
            if (!isNaN(e.target.value) && e.target.value > 0) {
              if (prop.value) {
                prop.value.set(parseInt(e.target.value));
              } else {
                setProperty(context, property, parseInt(e.target.value));
              }
            }
            break;
          case "enum":
            if (prop.options.indexOf(e.target.value) >= 0) {
              setProperty(context, property, e.target.value);
            }
            break;
          case "boolean":
            if (prop.value) {
              prop.value.set(e.target.value);
            } else {
              setProperty(context, property, e.target.checked);
            }
            break;
          case "text":
            // Do nothing because this event fires for every character typed
            break;
        }
      });
      input.addEventListener("blur", (e) => {
        switch (prop.type) {
          case "text":
            if (prop.value) {
              prop.value.set(e.target.value);
            } else {
              setProperty(context, property, parseInt(e.target.value));
            }
            break;
        }
      });

      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.target.blur();
        }
      });
      label.appendChild(span);
      label.appendChild(input);
      panel.appendChild(label);
    }
  }
}

function updateOutliner() {
  outlinerDirty = true;
}

function renderOutliner() {
  const padding = 20; // pixels
  for (let outliner of document.querySelectorAll(".outliner")) {
    const x = 0;
    let y = padding;
    const ctx = outliner.getContext("2d");
    ctx.fillStyle = "white";
    ctx.fillRect(0, 0, outliner.width, outliner.height);
    const stack = [{ object: outliner.object, indent: 0 }];

    ctx.save();
    ctx.translate(0, -outliner.offsetY);

    // Iterate as long as there are items in the stack
    while (stack.length > 0) {
      const { object, indent } = stack.pop();

      // Determine if the object is collapsed and draw the corresponding triangle
      const triangleX = x + indent + triangleSize; // X position for the triangle
      const triangleY = y - padding / 2; // Y position for the triangle (centered vertically)

      if (outliner.active === object) {
        ctx.fillStyle = "red";
        ctx.fillRect(0, y - padding, outliner.width, padding);
      }

      if (outliner.collapsed[object.idx]) {
        drawRegularPolygon(ctx, triangleX, triangleY, triangleSize, 3, "black");
      } else {
        drawRegularPolygon(
          ctx,
          triangleX,
          triangleY,
          triangleSize,
          3,
          "black",
          Math.PI / 2,
        );
      }

      // Draw the current object (GraphicsObject or Layer)
      const label = `(${object.constructor.name}) ${object.name}`;
      ctx.fillStyle = "black";
      // ctx.fillText(label, x + indent + 2*triangleSize, y);
      drawHorizontallyCenteredText(
        ctx,
        label,
        x + indent + 2 * triangleSize,
        y - padding / 2,
        padding * 0.75,
      );

      // Update the Y position for the next line
      y += padding; // Space between lines (adjust as necessary)

      if (outliner.collapsed[object.idx]) {
        continue;
      }

      // If the object has layers, add them to the stack
      if (object.layers) {
        for (let i = object.layers.length - 1; i >= 0; i--) {
          const layer = object.layers[i];
          stack.push({ object: layer, indent: indent + padding });
        }
      } else if (object.children) {
        for (let i = object.children.length - 1; i >= 0; i--) {
          const child = object.children[i];
          stack.push({ object: child, indent: indent + 2 * padding });
        }
      }
    }
    ctx.restore();
  }
}

function updateMenu() {
  menuDirty = true;
}

async function renderMenu() {
  let activeFrame;
  let activeKeyframe;
  let newFrameMenuItem;
  let newKeyframeMenuItem;
  let newBlankKeyframeMenuItem;
  let duplicateKeyframeMenuItem;
  let deleteFrameMenuItem;

  // Move this
  updateOutliner();

  let recentFilesList = [];
  config.recentFiles.forEach((file) => {
    recentFilesList.push({
      text: file,
      enabled: true,
      action: () => {
        document.body.style.cursor = "wait"
        setTimeout(()=>_open(file),10);
      },
    });
  });

  const frameInfo = context.activeObject.activeLayer.getFrameValue(
    context.activeObject.currentFrameNum
  )
  if (frameInfo.valueAtN) {
    activeFrame = true;
    activeKeyframe = true;
  } else if (frameInfo.prev && frameInfo.next) {
    activeFrame = true;
    activeKeyframe = false;
  } else {
    activeFrame = false;
    activeKeyframe = false;
  }
  const appSubmenu = await Submenu.new({
    text: "Lightningbeam",
    items: [
      {
        text: "About Lightningbeam",
        enabled: true,
        action: about,
      },
      {
        text: "Settings",
        enabled: false,
        action: () => {},
      },
      {
        text: "Close Window",
        enabled: true,
        action: quit,
      },
      {
        text: "Quit Lightningbeam",
        enabled: true,
        action: quit,
      },
    ],
  });
  const fileSubmenu = await Submenu.new({
    text: "File",
    items: [
      {
        text: "New file...",
        enabled: true,
        action: newFile,
        accelerator: getShortcut("new"),
      },
      {
        text: "New Window",
        enabled: true,
        action: newWindow,
        accelerator: getShortcut("newWindow"),
      },
      {
        text: "Save",
        enabled: true,
        action: save,
        accelerator: getShortcut("save"),
      },
      {
        text: "Save As...",
        enabled: true,
        action: saveAs,
        accelerator: getShortcut("saveAs"),
      },
      await Submenu.new({
        text: "Open Recent",
        items: recentFilesList,
      }),
      {
        text: "Open File...",
        enabled: true,
        action: open,
        accelerator: getShortcut("open"),
      },
      {
        text: "Revert",
        enabled: undoStack.length > lastSaveIndex,
        action: revert,
      },
      {
        text: "Import...",
        enabled: true,
        action: importFile,
        accelerator: getShortcut("import"),
      },
      {
        text: "Export...",
        enabled: true,
        action: render,
        accelerator: getShortcut("export"),
      },
      {
        text: "Quit",
        enabled: true,
        action: quit,
        accelerator: getShortcut("quit"),
      },
    ],
  });

  const editSubmenu = await Submenu.new({
    text: "Edit",
    items: [
      {
        text:
          "Undo " +
          (undoStack.length > 0
            ? camelToWords(undoStack[undoStack.length - 1].name)
            : ""),
        enabled: undoStack.length > 0,
        action: undo,
        accelerator: getShortcut("undo"),
      },
      {
        text:
          "Redo " +
          (redoStack.length > 0
            ? camelToWords(redoStack[redoStack.length - 1].name)
            : ""),
        enabled: redoStack.length > 0,
        action: redo,
        accelerator: getShortcut("redo"),
      },
      {
        text: "Cut",
        enabled: false,
        action: () => {},
      },
      {
        text: "Copy",
        enabled:
          context.selection.length > 0 || context.shapeselection.length > 0,
        action: copy,
        accelerator: getShortcut("copy"),
      },
      {
        text: "Paste",
        enabled: true,
        action: paste,
        accelerator: getShortcut("paste"),
      },
      {
        text: "Delete",
        enabled:
          context.selection.length > 0 || context.shapeselection.length > 0,
        action: delete_action,
        accelerator: getShortcut("delete"),
      },
      {
        text: "Select All",
        enabled: true,
        action: actions.selectAll.create,
        accelerator: getShortcut("selectAll"),
      },
      {
        text: "Select None",
        enabled: true,
        action: actions.selectNone.create,
        accelerator: getShortcut("selectNone"),
      },
    ],
  });

  const modifySubmenu = await Submenu.new({
    text: "Modify",
    items: [
      {
        text: "Group",
        enabled:
          context.selection.length != 0 || context.shapeselection.length != 0,
        action: actions.group.create,
        accelerator: getShortcut("group"),
      },
      {
        text: "Send to back",
        enabled:
          context.selection.length != 0 || context.shapeselection.length != 0,
        action: actions.sendToBack.create,
      },
      {
        text: "Bring to front",
        enabled:
          context.selection.length != 0 || context.shapeselection.length != 0,
        action: actions.bringToFront.create,
      },
    ],
  });

  const layerSubmenu = await Submenu.new({
    text: "Layer",
    items: [
      {
        text: "Add Layer",
        enabled: true,
        action: actions.addLayer.create,
        accelerator: getShortcut("addLayer"),
      },
      {
        text: "Delete Layer",
        enabled: context.activeObject.layers.length > 1,
        action: actions.deleteLayer.create,
      },
      {
        text: context.activeObject.activeLayer.visible
          ? "Hide Layer"
          : "Show Layer",
        enabled: true,
        action: () => {
          context.activeObject.activeLayer.toggleVisibility();
        },
      },
    ],
  });

  newFrameMenuItem = {
    text: "New Frame",
    enabled: !activeFrame,
    action: addFrame,
  };
  newKeyframeMenuItem = {
    text: "New Keyframe",
    enabled: !activeKeyframe,
    accelerator: getShortcut("addKeyframe"),
    action: addKeyframe,
  };
  newBlankKeyframeMenuItem = {
    text: "New Blank Keyframe",
    // enabled: !activeKeyframe,
    enabled: false,
    accelerator: getShortcut("addBlankKeyframe"),
    action: addKeyframe,
  };
  duplicateKeyframeMenuItem = {
    text: "Duplicate Keyframe",
    enabled: activeKeyframe,
    action: () => {
      context.activeObject.setFrameNum(context.activeObject.currentFrameNum+1)
      addKeyframe()
    },
  };
  deleteFrameMenuItem = {
    text: "Delete Frame",
    enabled: activeFrame,
    action: deleteFrame,
  };

  const timelineSubmenu = await Submenu.new({
    text: "Timeline",
    items: [
      // newFrameMenuItem,
      newKeyframeMenuItem,
      newBlankKeyframeMenuItem,
      deleteFrameMenuItem,
      duplicateKeyframeMenuItem,
      {
        text: "Add Motion Tween",
        enabled: activeFrame,
        action: actions.addMotionTween.create,
      },
      {
        text: "Add Shape Tween",
        enabled: activeFrame,
        action: actions.addShapeTween.create,
      },
      {
        text: "Return to start",
        enabled: false,
        action: () => {},
      },
      {
        text: "Play",
        enabled: !playing,
        action: playPause,
        accelerator: getShortcut("playAnimation"),
      },
    ],
  });
  const viewSubmenu = await Submenu.new({
    text: "View",
    items: [
      {
        text: "Zoom In",
        enabled: true,
        action: zoomIn,
        accelerator: getShortcut("zoomIn"),
      },
      {
        text: "Zoom Out",
        enabled: true,
        action: zoomOut,
        accelerator: getShortcut("zoomOut"),
      },
      {
        text: "Actual Size",
        enabled: context.zoomLevel != 1,
        action: resetZoom,
        accelerator: getShortcut("resetZoom"),
      },
      {
        text: "Recenter View",
        enabled: true,
        action: recenter,
        // accelerator: getShortcut("recenter"),
      },
    ],
  });
  const helpSubmenu = await Submenu.new({
    text: "Help",
    items: [
      {
        text: "About...",
        enabled: true,
        action: about,
      },
    ],
  });

  let items = [
    fileSubmenu,
    editSubmenu,
    modifySubmenu,
    layerSubmenu,
    timelineSubmenu,
    viewSubmenu,
    helpSubmenu,
  ];
  if (macOS) {
    items.unshift(appSubmenu);
  }
  const menu = await Menu.new({
    items: items,
  });
  await (macOS ? menu.setAsAppMenu() : menu.setAsWindowMenu());
}
updateMenu();

const panes = {
  stage: {
    name: "stage",
    func: stage,
  },
  toolbar: {
    name: "toolbar",
    func: toolbar,
  },
  timeline: {
    name: "timeline",
    func: timeline,
  },
  infopanel: {
    name: "infopanel",
    func: infopanel,
  },
  outlineer: {
    name: "outliner",
    func: outliner,
  },
};

function _arrayBufferToBase64(buffer) {
  var binary = "";
  var bytes = new Uint8Array(buffer);
  var len = bytes.byteLength;
  for (var i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return window.btoa(binary);
}

async function convertToDataURL(filePath, allowedMimeTypes) {
  try {
    // Read the image file as a binary file (buffer)
    const binaryData = await readFile(filePath);
    const mimeType = getMimeType(filePath);
    if (!mimeType) {
      throw new Error("Unsupported file type");
    }
    if (allowedMimeTypes.indexOf(mimeType) == -1) {
      throw new Error(`Unsupported MIME type ${mimeType}`);
    }

    const base64Data = _arrayBufferToBase64(binaryData);
    const dataURL = `data:${mimeType};base64,${base64Data}`;

    return { dataURL, mimeType };
  } catch (error) {
    console.log(error);
    console.error("Error reading the file:", error);
    return null;
  }
}

// Determine the MIME type based on the file extension
function getMimeType(filePath) {
  const ext = filePath.split(".").pop().toLowerCase();
  switch (ext) {
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "png":
      return "image/png";
    case "gif":
      return "image/gif";
    case "bmp":
      return "image/bmp";
    case "webp":
      return "image/webp";
    case "mp3":
      return "audio/mpeg";
    default:
      return null; // Unsupported file type
  }
}

function startToneOnUserInteraction() {
  // Function to handle the first interaction (click or key press)
  const startTone = () => {
    Tone.start()
      .then(() => {
        console.log("Tone.js started!");
      })
      .catch((err) => {
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
startToneOnUserInteraction();

function renderAll() {
  try {
    if (uiDirty) {
      renderUI();
      uiDirty = false;
    }
    if (layersDirty) {
      renderLayers();
      layersDirty = false;
    }
    if (outlinerDirty) {
      renderOutliner();
      outlinerDirty = false;
    }
    if (menuDirty) {
      renderMenu();
      menuDirty = false;
    }
    if (infopanelDirty) {
      renderInfopanel();
      infopanelDirty = false;
    }
  } catch (error) {
    const errorMessage = error.message || error.toString();  // Use error message or string representation of the error

    if (errorMessage !== lastErrorMessage) {
      // A new error, log it and reset repeat count
      console.error(error);
      lastErrorMessage = errorMessage;
      repeatCount = 1;
    } else if (repeatCount === 1) {
      // The error repeats for the second time, log "[Repeats]"
      console.warn("[Repeats]");
      repeatCount = 2;
    }
  } finally {
    requestAnimationFrame(renderAll);
  }
}

renderAll();

if (window.openedFiles?.length>0) {
  document.body.style.cursor = "wait"
  setTimeout(()=>_open(window.openedFiles[0]),10)
  for (let i=1; i<window.openedFiles.length; i++) {
    newWindow(window.openedFiles[i])
  }
}

async function testAudio() {
  console.log("Starting rust")
  await init();
  console.log("Rust started")
  const coreInterface = new CoreInterface(100, 100)
  coreInterface.init()
  coreInterface.play(0.0)
  console.log(coreInterface)

  let audioStarted = false;
  const startCoreInterfaceAudio = () => {
    if (!audioStarted) {
      try {
        coreInterface.resume_audio();
        audioStarted = true;
        console.log("Started CoreInterface Audio!")
      } catch (err) {
        console.error("Audio resume failed:", err);
      }
    }

    // Remove the event listeners to prevent them from firing again
    document.removeEventListener("click", startCoreInterfaceAudio);
    document.removeEventListener("keydown", startCoreInterfaceAudio);
  };

  // Add event listeners for mouse click and key press
  document.addEventListener("click", startCoreInterfaceAudio);
  document.addEventListener("keydown", startCoreInterfaceAudio);
}
testAudio()
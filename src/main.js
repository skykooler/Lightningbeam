const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
import * as fitCurve from "/fit-curve.js";
import { Bezier } from "/bezier.js";
import { Quadtree } from "./quadtree.js";
import {
  createNewFileDialog,
  showNewFileDialog,
  closeDialog,
} from "./newfile.js";
import {
  createStartScreen,
  updateStartScreen,
  showStartScreen,
  hideStartScreen,
} from "./startscreen.js";
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
  distanceToLineSegment,
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
import { AlphaSelectionBar, ColorSelectorWidget, ColorWidget, HueSelectionBar, SaturationValueSelectionGradient, TimelineWindow, TimelineWindowV2, VirtualPiano, PianoRollEditor, Widget } from "./widgets.js";
import { nodeTypes, SignalType, getPortClass, NodeCategory, getCategories, getNodesByCategory } from "./nodeTypes.js";

// State management
import {
  context,
  config,
  pointerList,
  startProps,
  getShortcut,
  loadConfig,
  saveConfig,
  addRecentFile
} from "./state.js";

// Data models
import {
  Frame,
  TempFrame,
  tempFrame,
  Keyframe,
  AnimationCurve,
  AnimationData
} from "./models/animation.js";
import {
  VectorLayer,
  AudioTrack,
  VideoLayer,
  initializeLayerDependencies
} from "./models/layer.js";
import {
  BaseShape,
  TempShape,
  Shape,
  initializeShapeDependencies
} from "./models/shapes.js";
import {
  GraphicsObject,
  initializeGraphicsObjectDependencies
} from "./models/graphics-object.js";
import { createRoot } from "./models/root.js";
import { actions, initializeActions, updateAutomationName } from "./actions/index.js";

// Layout system
import { defaultLayouts, getLayout, getLayoutNames } from "./layouts.js";
import { buildLayout, loadLayoutByKeyOrName, saveCustomLayout, serializeLayout } from "./layoutmanager.js";

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
const { PhysicalPosition, LogicalPosition } = window.__TAURI__.dpi;
const { getCurrentWindow } = window.__TAURI__.window;
const { getVersion } = window.__TAURI__.app;

// Supported file extensions
const imageExtensions = ["png", "gif", "avif", "jpg", "jpeg"];
const audioExtensions = ["mp3", "wav", "aiff", "ogg", "flac"];
const videoExtensions = ["mp4", "mov", "avi", "mkv", "webm", "m4v"];
const midiExtensions = ["mid", "midi"];
const beamExtensions = ["beam"];

// import init, { CoreInterface } from './pkg/lightningbeam_core.js';

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

// context.mode is now in context.context.mode (defined in state.js)

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
let maxFileVersion = "2.1";

let filePath = undefined;
let fileExportPath = undefined;

let state = "normal";

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

// Note: context, config, pointerList, startProps, getShortcut, loadConfig, saveConfig, addRecentFile
// are now imported from state.js

// Note: actions object is now imported from ./actions/index.js
// The actions will be initialized later with dependencies via initializeActions()

// Expose context and actions for UI testing
window.context = context;
window.actions = actions;
window.addKeyframeAtPlayhead = addKeyframeAtPlayhead;
window.updateVideoFrames = null; // Will be set after function is defined

// IPC Benchmark function - run from console: testIPCBenchmark()
window.testIPCBenchmark = async function() {
  const { invoke, Channel } = window.__TAURI__.core;

  // Test sizes: 1KB, 10KB, 50KB, 100KB, 500KB, 1MB, 2MB, 5MB
  const testSizes = [
    1024,           // 1 KB
    10 * 1024,      // 10 KB
    50 * 1024,      // 50 KB
    100 * 1024,     // 100 KB
    500 * 1024,     // 500 KB
    1024 * 1024,    // 1 MB
    2 * 1024 * 1024,  // 2 MB
    5 * 1024 * 1024   // 5 MB
  ];

  console.log('\n=== IPC Benchmark Starting ===\n');
  console.log('Size (KB)\tJS Total (ms)\tJS IPC (ms)\tJS Recv (ms)\tThroughput (MB/s)');
  console.log('─'.repeat(80));

  for (const sizeBytes of testSizes) {
    const t_start = performance.now();

    let receivedData = null;
    const dataPromise = new Promise((resolve, reject) => {
      const channel = new Channel();

      channel.onmessage = (data) => {
        const t_recv_start = performance.now();
        receivedData = data;
        const t_recv_end = performance.now();
        resolve(t_recv_end - t_recv_start);
      };

      invoke('video_ipc_benchmark', {
        sizeBytes: sizeBytes,
        channel: channel
      }).catch(reject);
    });

    const recv_time = await dataPromise;
    const t_after_ipc = performance.now();

    const total_time = t_after_ipc - t_start;
    const ipc_time = total_time - recv_time;
    const size_kb = sizeBytes / 1024;
    const size_mb = sizeBytes / (1024 * 1024);
    const throughput = size_mb / (total_time / 1000);

    console.log(`${size_kb.toFixed(0)}\t\t${total_time.toFixed(2)}\t\t${ipc_time.toFixed(2)}\t\t${recv_time.toFixed(2)}\t\t${throughput.toFixed(2)}`);

    // Small delay between tests
    await new Promise(resolve => setTimeout(resolve, 100));
  }

  console.log('\n=== IPC Benchmark Complete ===\n');
  console.log('Run again with: testIPCBenchmark()');
};

function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

/**
 * Generate a consistent pastel color from a UUID string
 * Uses hash of UUID to ensure same UUID always produces same color
 */
function uuidToColor(uuid) {
  // Simple hash function
  let hash = 0;
  for (let i = 0; i < uuid.length; i++) {
    hash = uuid.charCodeAt(i) + ((hash << 5) - hash);
    hash = hash & hash; // Convert to 32-bit integer
  }

  // Generate HSL color with fixed saturation and lightness for pastel appearance
  const hue = Math.abs(hash % 360);
  const saturation = 65; // Medium saturation for pleasant pastels
  const lightness = 70;  // Light enough to be pastel but readable

  return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
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

  // Get visible shapes from Layer using AnimationData
  let currentTime = context.activeObject?.currentTime || 0;
  let layer = context.activeObject?.activeLayer;
  if (!layer) return undefined;

  // AudioTracks don't have shapes, so return early
  if (!layer.getVisibleShapes) return undefined;

  for (let shape of layer.getVisibleShapes(currentTime)) {
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

  // Get visible shapes from Layer using AnimationData
  let currentTime = context.activeObject?.currentTime || 0;
  let layer = context.activeObject?.activeLayer;
  if (!layer) return undefined;

  // AudioTracks don't have shapes, so return early
  if (!layer.getVisibleShapes) return undefined;

  for (let shape of layer.getVisibleShapes(currentTime)) {
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

// ============================================================================
// Animation system classes (Frame, TempFrame, Keyframe, AnimationCurve, AnimationData)
// have been moved to src/models/animation.js and are imported at the top of this file
// ============================================================================

// ============================================================================
// Layer system classes (VectorLayer, AudioTrack, VideoLayer)
// have been moved to src/models/layer.js and are imported at the top of this file
// ============================================================================

// ============================================================================
// Shape classes (BaseShape, TempShape, Shape)
// have been moved to src/models/shapes.js and are imported at the top of this file
// ============================================================================

// ============================================================================
// GraphicsObject class
// has been moved to src/models/graphics-object.js and is imported at the top of this file
// ============================================================================

// Initialize layer and shape dependencies now that all classes are loaded
initializeLayerDependencies({
  GraphicsObject,
  Shape,
  TempShape,
  updateUI,
  updateMenu,
  updateLayers,
  vectorDist,
  minSegmentSize,
  debugQuadtree,
  debugCurves,
  debugPoints,
  debugPaintbucket,
  d3: window.d3,
  actions,
});

initializeShapeDependencies({
  growBoundingBox,
  lerp,
  lerpColor,
  uuidToColor,
  simplifyPolyline,
  fitCurve,
  createMissingTexturePattern,
  debugQuadtree,
  d3: window.d3,
});

initializeGraphicsObjectDependencies({
  growBoundingBox,
  getRotatedBoundingBox,
  multiplyMatrices,
  uuidToColor,
});

// ============ ROOT OBJECT INITIALIZATION ============
// Extracted to: models/root.js
let _rootInternal = createRoot();
console.log('[INIT] Setting root.frameRate to config.framerate:', config.framerate);
_rootInternal.frameRate = config.framerate;
console.log('[INIT] root.frameRate is now:', _rootInternal.frameRate);

// Make root a global variable with getter/setter to catch reassignments
let __root = new Proxy(_rootInternal, {
  get(target, prop) {
    return Reflect.get(target, prop);
  },
  set(target, prop, value) {
    return Reflect.set(target, prop, value);
  }
});

Object.defineProperty(globalThis, 'root', {
  get() {
    return __root;
  },
  set(newRoot) {
    __root = newRoot;
  },
  configurable: true,
  enumerable: true
});

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

  // Initialize audio system on startup
  (async () => {
    try {
      console.log('Initializing audio system...');
      const result = await invoke('audio_init', { bufferSize: config.audioBufferSize });
      console.log('Audio system initialized:', result);
    } catch (error) {
      if (error === 'Audio already initialized') {
        console.log('Audio system already initialized');
      } else {
        console.error('Failed to initialize audio system:', error);
      }
    }
  })();
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
  // menu.popup({ x: e.clientX, y: e.clientY });
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
      e.preventDefault(); // Prevent spacebar from clicking focused buttons
      break;
    case config.shortcuts.selectAll:
      e.preventDefault();
      break;
    // TODO: put these in shortcuts
    case "<mod>ArrowRight":
      advance();
      e.preventDefault();
      break;
    case "ArrowRight":
      if (context.selection.length) {
        const layer = context.activeObject.activeLayer;
        const time = context.activeObject.currentTime || 0;
        let oldPositions = {};
        let newPositions = {};

        for (let item of context.selection) {
          const oldX = layer.animationData.interpolate(`object.${item.idx}.x`, time) || item.x;
          const oldY = layer.animationData.interpolate(`object.${item.idx}.y`, time) || item.y;
          oldPositions[item.idx] = { x: oldX, y: oldY };
          item.x = oldX + 1;
          newPositions[item.idx] = { x: item.x, y: item.y };
        }

        actions.moveObjects.create(context.selection, layer, time, oldPositions, newPositions);
      }
      e.preventDefault();
      break;
    case "<mod>ArrowLeft":
      rewind();
      break;
    case "ArrowLeft":
      if (context.selection.length) {
        const layer = context.activeObject.activeLayer;
        const time = context.activeObject.currentTime || 0;
        let oldPositions = {};
        let newPositions = {};

        for (let item of context.selection) {
          const oldX = layer.animationData.interpolate(`object.${item.idx}.x`, time) || item.x;
          const oldY = layer.animationData.interpolate(`object.${item.idx}.y`, time) || item.y;
          oldPositions[item.idx] = { x: oldX, y: oldY };
          item.x = oldX - 1;
          newPositions[item.idx] = { x: item.x, y: item.y };
        }

        actions.moveObjects.create(context.selection, layer, time, oldPositions, newPositions);
      }
      e.preventDefault();
      break;
    case "ArrowUp":
      if (context.selection.length) {
        const layer = context.activeObject.activeLayer;
        const time = context.activeObject.currentTime || 0;
        let oldPositions = {};
        let newPositions = {};

        for (let item of context.selection) {
          const oldX = layer.animationData.interpolate(`object.${item.idx}.x`, time) || item.x;
          const oldY = layer.animationData.interpolate(`object.${item.idx}.y`, time) || item.y;
          oldPositions[item.idx] = { x: oldX, y: oldY };
          item.y = oldY - 1;
          newPositions[item.idx] = { x: item.x, y: item.y };
        }

        actions.moveObjects.create(context.selection, layer, time, oldPositions, newPositions);
      }
      e.preventDefault();
      break;
    case "ArrowDown":
      if (context.selection.length) {
        const layer = context.activeObject.activeLayer;
        const time = context.activeObject.currentTime || 0;
        let oldPositions = {};
        let newPositions = {};

        for (let item of context.selection) {
          const oldX = layer.animationData.interpolate(`object.${item.idx}.x`, time) || item.x;
          const oldY = layer.animationData.interpolate(`object.${item.idx}.y`, time) || item.y;
          oldPositions[item.idx] = { x: oldX, y: oldY };
          item.y = oldY + 1;
          newPositions[item.idx] = { x: item.x, y: item.y };
        }

        actions.moveObjects.create(context.selection, layer, time, oldPositions, newPositions);
      }
      e.preventDefault();
      break;
    default:
      break;
  }
});

async function playPause() {
  context.playing = !context.playing;
  if (context.playing) {
    // Reset to start if we're at the end
    const duration = context.activeObject.duration;
    if (duration > 0 && context.activeObject.currentTime >= duration) {
      context.activeObject.currentTime = 0;
    }

    // Sync playhead position with DAW backend before starting
    try {
      await invoke('audio_seek', { seconds: context.activeObject.currentTime });
    } catch (error) {
      console.error('Failed to seek audio:', error);
    }

    // Start DAW backend audio playback
    try {
      await invoke('audio_play');
    } catch (error) {
      console.error('Failed to start audio playback:', error);
    }

    // Re-enable auto-scroll when playback starts
    if (context.pianoRollEditor) {
      context.pianoRollEditor.autoScrollEnabled = true;
    }

    playbackLoop();
  } else {
    // Stop recording if active
    if (context.isRecording) {
      console.log('playPause - stopping recording for clip:', context.recordingClipId);
      try {
        await invoke('audio_stop_recording');
        context.isRecording = false;
        context.recordingTrackId = null;
        context.recordingClipId = null;
        console.log('Recording stopped by play/pause button');

        // Update record button appearance if it exists
        if (context.recordButton) {
          context.recordButton.className = "playback-btn playback-btn-record";
          context.recordButton.title = "Record";
        }
      } catch (error) {
        console.error('Failed to stop recording:', error);
      }
    }

    // Stop DAW backend audio playback
    try {
      await invoke('audio_stop');
    } catch (error) {
      console.error('Failed to stop audio playback:', error);
    }
  }

  // Update play/pause button appearance if it exists
  if (context.playPauseButton) {
    context.playPauseButton.className = context.playing ? "playback-btn playback-btn-pause" : "playback-btn playback-btn-play";
    context.playPauseButton.title = context.playing ? "Pause" : "Play";
  }
}

// Playback animation loop - redraws UI while playing
// Note: Time is synchronized from DAW via PlaybackPosition events
function playbackLoop() {
  // Redraw stage and timeline
  updateUI();
  if (context.timelineWidget?.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }

  if (context.playing) {
    const duration = context.activeObject.duration;

    // Check if we've reached the end (but allow infinite playback when recording)
    if (context.isRecording || (duration > 0 && context.activeObject.currentTime < duration)) {
      // Continue playing
      requestAnimationFrame(playbackLoop);
    } else {
      // Animation finished
      context.playing = false;

      // Stop DAW backend audio playback
      invoke('audio_stop').catch(error => {
        console.error('Failed to stop audio playback:', error);
      });

      // Update play/pause button appearance
      if (context.playPauseButton) {
        context.playPauseButton.className = "playback-btn playback-btn-play";
        context.playPauseButton.title = "Play";
      }

      for (let audioTrack of context.activeObject.audioTracks) {
        for (let i in audioTrack.sounds) {
          let sound = audioTrack.sounds[i];
          sound.player.stop();
        }
      }
    }
  }
}

// Update video frames for all VideoLayers in the scene
async function updateVideoFrames(currentTime) {
  // Recursively find all VideoLayers in the scene
  function findVideoLayers(obj) {
    const videoLayers = [];
    if (obj.layers) {
      for (let layer of obj.layers) {
        if (layer.type === 'video') {
          videoLayers.push(layer);
        }
      }
    }
    // Recursively check children (GraphicsObjects can contain other GraphicsObjects)
    if (obj.children) {
      for (let child of obj.children) {
        videoLayers.push(...findVideoLayers(child));
      }
    }
    return videoLayers;
  }

  const videoLayers = findVideoLayers(context.activeObject);

  // Update all video layers in parallel
  await Promise.all(videoLayers.map(layer => layer.updateFrame(currentTime)));

  // Note: No updateUI() call here - renderUI() will draw after awaiting this function
}

// Expose updateVideoFrames globally
window.updateVideoFrames = updateVideoFrames;

// Single-step forward by one frame/second
function advance() {
  if (context.timelineWidget?.timelineState?.timeFormat === "frames") {
    context.activeObject.currentTime += 1 / context.activeObject.frameRate;
  } else {
    context.activeObject.currentTime += 1;
  }

  // Sync timeline playhead position
  if (context.timelineWidget?.timelineState) {
    context.timelineWidget.timelineState.currentTime = context.activeObject.currentTime;
  }

  // Sync DAW backend
  invoke('audio_seek', { seconds: context.activeObject.currentTime });

  // Update video frames
  updateVideoFrames(context.activeObject.currentTime);

  updateLayers();
  updateMenu();
  updateUI();
  if (context.timelineWidget?.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }
}

// Calculate which MIDI notes are currently playing at a given time (efficient binary search)
function getPlayingNotesAtTime(time) {
  const playingNotes = [];

  // Check all MIDI tracks
  for (const track of context.activeObject.audioTracks) {
    if (track.type !== 'midi') continue;

    // Check all clips in the track
    for (const clip of track.clips) {
      if (!clip.notes || clip.notes.length === 0) continue;

      // Check if current time is within the clip's range
      const clipLocalTime = time - clip.startTime;
      if (clipLocalTime < 0 || clipLocalTime > clip.duration) {
        continue;
      }

      // Binary search to find the first note that might be playing
      // Notes are sorted by start_time
      let left = 0;
      let right = clip.notes.length - 1;
      let firstCandidate = clip.notes.length;

      while (left <= right) {
        const mid = Math.floor((left + right) / 2);
        const note = clip.notes[mid];
        const noteEndTime = note.start_time + note.duration;

        if (noteEndTime <= clipLocalTime) {
          // This note ends before current time, search right
          left = mid + 1;
        } else {
          // This note might be playing or starts after current time
          firstCandidate = mid;
          right = mid - 1;
        }
      }

      // Check notes from firstCandidate onwards until we find one that starts after current time
      for (let i = firstCandidate; i < clip.notes.length; i++) {
        const note = clip.notes[i];

        // If note starts after current time, we're done with this clip
        if (note.start_time > clipLocalTime) {
          break;
        }

        // Check if note is currently playing
        const noteEndTime = note.start_time + note.duration;
        if (note.start_time <= clipLocalTime && clipLocalTime < noteEndTime) {
          playingNotes.push(note.note);
        }
      }
    }
  }

  return playingNotes;
}

// Handle audio events pushed from Rust via Tauri event system
async function handleAudioEvent(event) {
  switch (event.type) {
    case 'PlaybackPosition':
      // Sync frontend time with DAW time
      if (context.playing) {
        // Quantize time to framerate for animation playback
        const framerate = context.activeObject.frameRate;
        const frameDuration = 1 / framerate;
        const quantizedTime = Math.floor(event.time / frameDuration) * frameDuration;

        context.activeObject.currentTime = quantizedTime;
        if (context.timelineWidget?.timelineState) {
          context.timelineWidget.timelineState.currentTime = quantizedTime;
        }

        // Update video frames
        updateVideoFrames(quantizedTime);

        // Update time display
        if (context.updateTimeDisplay) {
          context.updateTimeDisplay();
        }

        // Update piano widget with currently playing notes
        if (context.pianoWidget && context.pianoRedraw) {
          const playingNotes = getPlayingNotesAtTime(quantizedTime);
          context.pianoWidget.setPlayingNotes(playingNotes);
          context.pianoRedraw();
        }

        // Update piano roll editor to show playhead
        if (context.pianoRollRedraw) {
          context.pianoRollRedraw();
        }
      }
      break;

    case 'RecordingStarted':
      console.log('[FRONTEND] RecordingStarted - track:', event.track_id, 'clip:', event.clip_id);
      context.recordingClipId = event.clip_id;

      // Create the clip object in the audio track
      const recordingTrack = context.activeObject.audioTracks.find(t => t.audioTrackId === event.track_id);
      if (recordingTrack) {
        const startTime = context.activeObject.currentTime || 0;
        console.log('[FRONTEND] Creating clip object for clip', event.clip_id, 'on track', event.track_id, 'at time', startTime);
        recordingTrack.clips.push({
          clipId: event.clip_id,
          name: recordingTrack.name,
          poolIndex: null,  // Will be set when recording stops
          startTime: startTime,
          duration: 0,  // Will grow as recording progresses
          offset: 0,
          loading: true,
          waveform: []
        });

        updateLayers();
        if (context.timelineWidget?.requestRedraw) {
          context.timelineWidget.requestRedraw();
        }
      } else {
        console.error('[FRONTEND] Could not find audio track', event.track_id, 'for RecordingStarted event');
      }
      break;

    case 'RecordingProgress':
      // Update clip duration in UI
      console.log('Recording progress - clip:', event.clip_id, 'duration:', event.duration);
      updateRecordingClipDuration(event.clip_id, event.duration);
      break;

    case 'GraphNodeAdded':
      console.log('[FRONTEND] GraphNodeAdded event - track:', event.track_id, 'node_id:', event.node_id, 'node_type:', event.node_type);
      // Resolve the pending promise with the correct backend ID
      if (window.pendingNodeUpdate) {
        const { drawflowNodeId, nodeType, resolve } = window.pendingNodeUpdate;
        if (nodeType === event.node_type && resolve) {
          console.log('[FRONTEND] Resolving promise for node', drawflowNodeId, 'with backend ID:', event.node_id);
          resolve(event.node_id);
          window.pendingNodeUpdate = null;
        }
      }
      break;

    case 'RecordingStopped':
      console.log('[FRONTEND] RecordingStopped event - clip:', event.clip_id, 'pool_index:', event.pool_index, 'waveform peaks:', event.waveform?.length);
      console.log('[FRONTEND] Current recording state - isRecording:', context.isRecording, 'recordingClipId:', context.recordingClipId);
      await finalizeRecording(event.clip_id, event.pool_index, event.waveform);

      // Always clear recording state when we receive RecordingStopped
      console.log('[FRONTEND] Clearing recording state after RecordingStopped event');
      context.isRecording = false;
      context.recordingTrackId = null;
      context.recordingClipId = null;

      // Update record button appearance
      if (context.recordButton) {
        context.recordButton.className = "playback-btn playback-btn-record";
        context.recordButton.title = "Record";
      }
      break;

    case 'RecordingError':
      console.error('Recording error:', event.message);
      alert('Recording error: ' + event.message);
      context.isRecording = false;
      context.recordingTrackId = null;
      context.recordingClipId = null;
      break;

    case 'MidiRecordingProgress':
      // Update MIDI clip during recording with current duration and notes
      const progressMidiTrack = context.activeObject.audioTracks.find(t => t.audioTrackId === event.track_id);
      if (progressMidiTrack) {
        const progressClip = progressMidiTrack.clips.find(c => c.clipId === event.clip_id);
        if (progressClip) {
          console.log('[MIDI_PROGRESS] Updating clip', event.clip_id, '- duration:', event.duration, 'notes:', event.notes.length, 'loading:', progressClip.loading);
          progressClip.duration = event.duration;
          progressClip.loading = false; // Make sure clip is not in loading state
          // Convert backend note format to frontend format
          progressClip.notes = event.notes.map(([start_time, note, velocity, duration]) => ({
            note: note,
            start_time: start_time,
            duration: duration,
            velocity: velocity
          }));
          console.log('[MIDI_PROGRESS] Clip now has', progressClip.notes.length, 'notes');

          // Request redraw to show updated clip
          updateLayers();
          if (context.timelineWidget) {
            context.timelineWidget.requestRedraw();
          }
        } else {
          console.log('[MIDI_PROGRESS] Could not find clip', event.clip_id);
        }
      }
      break;

    case 'MidiRecordingStopped':
      console.log('[FRONTEND] ========== MidiRecordingStopped EVENT ==========');
      console.log('[FRONTEND] Event details - track:', event.track_id, 'clip:', event.clip_id, 'notes:', event.note_count);

      // Find the track and update the clip
      const midiTrack = context.activeObject.audioTracks.find(t => t.audioTrackId === event.track_id);
      console.log('[FRONTEND] Found MIDI track:', midiTrack ? midiTrack.name : 'NOT FOUND');

      if (midiTrack) {
        console.log('[FRONTEND] Track has', midiTrack.clips.length, 'clips:', midiTrack.clips.map(c => `{id:${c.clipId}, name:"${c.name}", loading:${c.loading}}`));

        // Find the clip we created when recording started
        let existingClip = midiTrack.clips.find(c => c.clipId === event.clip_id);
        console.log('[FRONTEND] Found existing clip:', existingClip ? `id:${existingClip.clipId}, name:"${existingClip.name}", loading:${existingClip.loading}` : 'NOT FOUND');

        if (existingClip) {
          // Fetch the clip data from the backend
          try {
            console.log('[FRONTEND] Fetching MIDI clip data from backend...');
            const clipData = await invoke('audio_get_midi_clip_data', {
              trackId: event.track_id,
              clipId: event.clip_id
            });
            console.log('[FRONTEND] Received clip data:', clipData);

            // Update the clip with the recorded notes
            console.log('[FRONTEND] Updating clip - before:', { loading: existingClip.loading, name: existingClip.name, duration: existingClip.duration, noteCount: existingClip.notes?.length });
            existingClip.loading = false;
            existingClip.name = `MIDI Clip (${event.note_count} notes)`;
            existingClip.duration = clipData.duration;
            existingClip.notes = clipData.notes;
            console.log('[FRONTEND] Updating clip - after:', { loading: existingClip.loading, name: existingClip.name, duration: existingClip.duration, noteCount: existingClip.notes?.length });
          } catch (error) {
            console.error('[FRONTEND] Failed to fetch MIDI clip data:', error);
            existingClip.loading = false;
            existingClip.name = `MIDI Clip (failed)`;
          }
        } else {
          console.error('[FRONTEND] Could not find clip', event.clip_id, 'on track', event.track_id);
        }

        // Request redraw to show the clip with recorded notes
        updateLayers();
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }
      }

      // Clear recording state
      console.log('[FRONTEND] Clearing MIDI recording state');
      context.isRecording = false;
      context.recordingTrackId = null;
      context.recordingClipId = null;

      // Update record button appearance
      if (context.recordButton) {
        context.recordButton.className = "playback-btn playback-btn-record";
        context.recordButton.title = "Record";
      }

      console.log('[FRONTEND] MIDI recording complete - recorded', event.note_count, 'notes');
      break;

    case 'GraphPresetLoaded':
      // Preset loaded - layers are already populated during graph reload
      console.log('GraphPresetLoaded event received for track:', event.track_id);
      break;

    case 'NoteOn':
      // MIDI note started - update virtual piano visual feedback
      if (context.pianoWidget) {
        context.pianoWidget.pressedKeys.add(event.note);
        if (context.pianoRedraw) {
          context.pianoRedraw();
        }
      }
      // Update MIDI activity timestamp
      context.lastMidiInputTime = Date.now();
      console.log('[NoteOn] Set lastMidiInputTime to:', context.lastMidiInputTime);

      // Start animation loop to keep redrawing the MIDI indicator
      if (!context.midiIndicatorAnimating) {
        context.midiIndicatorAnimating = true;
        const animateMidiIndicator = () => {
          if (context.timelineWidget && context.timelineWidget.requestRedraw) {
            context.timelineWidget.requestRedraw();
          }

          // Keep animating for 1 second after last MIDI input
          const elapsed = Date.now() - context.lastMidiInputTime;
          if (elapsed < 1000) {
            requestAnimationFrame(animateMidiIndicator);
          } else {
            context.midiIndicatorAnimating = false;
          }
        };
        requestAnimationFrame(animateMidiIndicator);
      }
      break;

    case 'NoteOff':
      // MIDI note stopped - update virtual piano visual feedback
      if (context.pianoWidget) {
        context.pianoWidget.pressedKeys.delete(event.note);
        if (context.pianoRedraw) {
          context.pianoRedraw();
        }
      }
      break;
  }
}

// Set up Tauri event listener for audio events
listen('audio-event', (tauriEvent) => {
  handleAudioEvent(tauriEvent.payload);
});

function updateRecordingClipDuration(clipId, duration) {
  // Find the clip in the active object's audio tracks and update its duration
  for (const audioTrack of context.activeObject.audioTracks) {
    const clip = audioTrack.clips.find(c => c.clipId === clipId);
    if (clip) {
      clip.duration = duration;
      updateLayers();
      if (context.timelineWidget?.requestRedraw) {
        context.timelineWidget.requestRedraw();
      }
      return;
    }
  }
}

async function finalizeRecording(clipId, poolIndex, waveform) {
  console.log('Finalizing recording - clipId:', clipId, 'poolIndex:', poolIndex, 'waveform length:', waveform?.length);

  // Find the clip and update it with the pool index and waveform
  for (const audioTrack of context.activeObject.audioTracks) {
    const clip = audioTrack.clips.find(c => c.clipId === clipId);
    if (clip) {
      console.log('Found clip to finalize:', clip);
      clip.poolIndex = poolIndex;
      clip.loading = false;
      clip.waveform = waveform;
      console.log('Clip after update:', clip);
      console.log('Waveform sample:', waveform?.slice(0, 5));

      updateLayers();
      if (context.timelineWidget?.requestRedraw) {
        context.timelineWidget.requestRedraw();
      }
      return;
    }
  }
  console.error('Could not find clip to finalize:', clipId);
}

// Single-step backward by one frame/second
function rewind() {
  if (context.timelineWidget?.timelineState?.timeFormat === "frames") {
    context.activeObject.currentTime -= 1 / context.activeObject.frameRate;
  } else {
    context.activeObject.currentTime -= 1;
  }

  // Sync timeline playhead position
  if (context.timelineWidget?.timelineState) {
    context.timelineWidget.timelineState.currentTime = context.activeObject.currentTime;
  }

  // Sync DAW backend
  invoke('audio_seek', { seconds: context.activeObject.currentTime });

  updateLayers();
  updateMenu();
  updateUI();
  if (context.timelineWidget?.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }
}

async function goToStart() {
  context.activeObject.currentTime = 0;

  // Sync timeline playhead position
  if (context.timelineWidget?.timelineState) {
    context.timelineWidget.timelineState.currentTime = 0;
  }

  // Sync with DAW backend
  try {
    await invoke('audio_seek', { seconds: 0 });
  } catch (error) {
    console.error('Failed to seek audio:', error);
  }

  updateLayers();
  updateUI();
  if (context.timelineWidget?.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }
}

async function goToEnd() {
  const duration = context.activeObject.duration;
  context.activeObject.currentTime = duration;

  // Sync timeline playhead position
  if (context.timelineWidget?.timelineState) {
    context.timelineWidget.timelineState.currentTime = duration;
  }

  // Sync with DAW backend
  try {
    await invoke('audio_seek', { seconds: duration });
  } catch (error) {
    console.error('Failed to seek audio:', error);
  }

  updateLayers();
  updateUI();
  if (context.timelineWidget?.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }
}

async function toggleRecording() {
  const { invoke } = window.__TAURI__.core;

  if (context.isRecording) {
    // Stop recording
    console.log('[FRONTEND] toggleRecording - stopping recording for clip:', context.recordingClipId);
    try {
      // Check if we're recording MIDI or audio
      const track = context.activeObject.audioTracks.find(t => t.audioTrackId === context.recordingTrackId);
      const isMidiRecording = track && track.type === 'midi';

      console.log('[FRONTEND] Stopping recording - isMIDI:', isMidiRecording, 'track type:', track?.type, 'track ID:', context.recordingTrackId);

      if (isMidiRecording) {
        console.log('[FRONTEND] Calling audio_stop_midi_recording...');
        await invoke('audio_stop_midi_recording');
        console.log('[FRONTEND] audio_stop_midi_recording returned successfully');
      } else {
        console.log('[FRONTEND] Calling audio_stop_recording...');
        await invoke('audio_stop_recording');
        console.log('[FRONTEND] audio_stop_recording returned successfully');
      }

      console.log('[FRONTEND] Clearing recording state in toggleRecording');
      context.isRecording = false;
      context.recordingTrackId = null;
      context.recordingClipId = null;
    } catch (error) {
      console.error('[FRONTEND] Failed to stop recording:', error);
    }
  } else {
    // Start recording - check if activeLayer is a track
    const audioTrack = context.activeObject.activeLayer;
    if (!audioTrack || !(audioTrack instanceof AudioTrack)) {
      alert('Please select a track to record to');
      return;
    }

    if (audioTrack.audioTrackId === null) {
      alert('Track not properly initialized');
      return;
    }

    // Start recording at current playhead position
    const startTime = context.activeObject.currentTime || 0;

    // Check if this is a MIDI track or audio track
    if (audioTrack.type === 'midi') {
      // MIDI recording
      console.log('[FRONTEND] Starting MIDI recording on track', audioTrack.audioTrackId, 'at time', startTime);
      try {
        // First, create a MIDI clip at the current playhead position
        const clipDuration = 4.0; // Default clip duration of 4 seconds (can be extended by recording)
        const clipId = await invoke('audio_create_midi_clip', {
          trackId: audioTrack.audioTrackId,
          startTime: startTime,
          duration: clipDuration
        });

        console.log('[FRONTEND] Created MIDI clip with ID:', clipId);

        // Add clip to track immediately (similar to MIDI import)
        audioTrack.clips.push({
          clipId: clipId,
          name: 'Recording...',
          startTime: startTime,
          duration: clipDuration,
          notes: [],
          loading: true
        });

        // Update UI to show the recording clip
        updateLayers();
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw();
        }

        // Now start MIDI recording
        await invoke('audio_start_midi_recording', {
          trackId: audioTrack.audioTrackId,
          clipId: clipId,
          startTime: startTime
        });

        context.isRecording = true;
        context.recordingTrackId = audioTrack.audioTrackId;
        context.recordingClipId = clipId;
        console.log('[FRONTEND] MIDI recording started successfully');

        // Start playback so the timeline moves (if not already playing)
        if (!context.playing) {
          await playPause();
        }
      } catch (error) {
        console.error('[FRONTEND] Failed to start MIDI recording:', error);
        alert('Failed to start MIDI recording: ' + error);
      }
    } else {
      // Audio recording
      console.log('[FRONTEND] Starting audio recording on track', audioTrack.audioTrackId, 'at time', startTime);
      try {
        await invoke('audio_start_recording', {
          trackId: audioTrack.audioTrackId,
          startTime: startTime
        });
        context.isRecording = true;
        context.recordingTrackId = audioTrack.audioTrackId;
        console.log('[FRONTEND] Audio recording started successfully, waiting for RecordingStarted event');

        // Start playback so the timeline moves (if not already playing)
        if (!context.playing) {
          await playPause();
        }
      } catch (error) {
        console.error('[FRONTEND] Failed to start audio recording:', error);
        alert('Failed to start audio recording: ' + error);
      }
    }
  }
}

function newWindow(path) {
  invoke("create_window", {app: window.__TAURI__.app, path: path})
}

async function _newFile(width, height, fps, layoutKey) {
  console.log('[_newFile] REPLACING ROOT - Creating new file with fps:', fps, 'layout:', layoutKey);
  console.trace('[_newFile] Stack trace for root replacement:');

  const oldRoot = root;
  console.log('[_newFile] Old root:', oldRoot, 'frameRate:', oldRoot?.frameRate);

  // Reset audio engine to clear any previous session data
  try {
    await invoke('audio_reset');
  } catch (error) {
    console.warn('Failed to reset audio engine:', error);
  }

  // Determine initial child type based on layout
  const initialChildType = layoutKey === 'audioDaw' ? 'midi' : 'layer';
  root = new GraphicsObject("root", initialChildType);

  // Switch to the selected layout if provided
  if (layoutKey) {
    config.currentLayout = layoutKey;
    config.defaultLayout = layoutKey;
    console.log('[_newFile] Switching to layout:', layoutKey);
    switchLayout(layoutKey);

    // Set default time format to measures for music mode
    if (layoutKey === 'audioDaw' && context.timelineWidget?.timelineState) {
      context.timelineWidget.timelineState.timeFormat = 'measures';
      // Show metronome button for audio projects
      if (context.metronomeGroup) {
        context.metronomeGroup.style.display = '';
      }
    }
  }

  // Define frameRate as a non-configurable property with a backing variable
  let _frameRate = fps;
  Object.defineProperty(root, 'frameRate', {
    get() {
      return _frameRate;
    },
    set(value) {
      console.log('[frameRate setter] Setting frameRate to:', value, 'from:', _frameRate);
      console.trace('[frameRate setter] Stack trace:');
      _frameRate = value;
    },
    enumerable: true,
    configurable: false
  });

  console.log('[_newFile] Immediately after setting frameRate:', root.frameRate);
  console.log('[_newFile] Checking if property exists:', 'frameRate' in root);
  console.log('[_newFile] Property descriptor:', Object.getOwnPropertyDescriptor(root, 'frameRate'));

  console.log('[_newFile] New root:', root, 'frameRate:', root.frameRate);
  console.log('[_newFile] After setting, root.frameRate:', root.frameRate);
  console.log('[_newFile] root object:', root);
  console.log('[_newFile] Before objectStack - root.frameRate:', root.frameRate);
  context.objectStack = [root];
  console.log('[_newFile] After objectStack - root.frameRate:', root.frameRate);
  context.selection = [];
  context.shapeselection = [];
  config.fileWidth = width;
  config.fileHeight = height;
  config.framerate = fps;
  filePath = undefined;
  console.log('[_newFile] Before saveConfig - root.frameRate:', root.frameRate);
  saveConfig();
  console.log('[_newFile] After saveConfig - root.frameRate:', root.frameRate);
  undoStack.length = 0;  // Clear without breaking reference
  redoStack.length = 0;  // Clear without breaking reference
  console.log('[_newFile] Before updateUI - root.frameRate:', root.frameRate);

  // Ensure there's an active layer - set to first layer if none is active
  if (!context.activeObject.activeLayer && context.activeObject.layers.length > 0) {
    context.activeObject.activeLayer = context.activeObject.layers[0];
  }

  updateUI();
  console.log('[_newFile] After updateUI - root.frameRate:', root.frameRate);
  updateLayers();
  console.log('[_newFile] After updateLayers - root.frameRate:', root.frameRate);
  updateMenu();
  console.log('[_newFile] After updateMenu - root.frameRate:', root.frameRate);
  console.log('[_newFile] At end of _newFile, root.frameRate:', root.frameRate);
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

    // Serialize audio pool (files < 10MB embedded, larger files saved as relative paths)
    let audioPool = [];
    try {
      audioPool = await invoke('audio_serialize_pool', { projectPath: path });
    } catch (error) {
      console.warn('Failed to serialize audio pool:', error);
      // Continue saving without audio pool - user may not have audio initialized
    }

    // Serialize track graphs (node graphs for each track)
    const trackGraphs = {};
    for (const track of root.audioTracks) {
      if (track.audioTrackId !== null) {
        try {
          const graphJson = await invoke('audio_serialize_track_graph', {
            trackId: track.audioTrackId,
            projectPath: path
          });
          trackGraphs[track.idx] = graphJson;
        } catch (error) {
          console.warn(`Failed to serialize graph for track ${track.name}:`, error);
        }
      }
    }

    // Serialize current layout structure (panes, splits, sizes)
    const serializedLayout = serializeLayout(rootPane);

    const fileData = {
      version: "2.0.0",
      width: config.fileWidth,
      height: config.fileHeight,
      fps: config.framerate,
      layoutState: serializedLayout, // Save current layout structure
      actions: undoStack,
      json: root.toJSON(),
      // Audio pool at the end for human readability
      audioPool: audioPool,
      // Track graphs for instruments/effects
      trackGraphs: trackGraphs,
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

/**
 * Handle missing audio files by prompting the user to locate them
 * @param {number[]} missingIndices - Array of pool indices that failed to load
 * @param {Object[]} audioPool - The audio pool entries from the project file
 * @param {string} projectPath - Path to the project file
 */
async function handleMissingAudioFiles(missingIndices, audioPool, projectPath) {
  const { open } = window.__TAURI__.dialog;

  for (const poolIndex of missingIndices) {
    const entry = audioPool[poolIndex];
    if (!entry) continue;

    const message = `Cannot find audio file:\n${entry.name}\n\nExpected location: ${entry.relativePath || 'embedded'}\n\nWould you like to locate this file?`;

    const result = await window.__TAURI__.dialog.confirm(message, {
      title: 'Missing Audio File',
      kind: 'warning',
      okLabel: 'Locate File',
      cancelLabel: 'Skip'
    });

    if (result) {
      // Let user browse for the file
      const selected = await open({
        title: `Locate ${entry.name}`,
        multiple: false,
        filters: [{
          name: 'Audio Files',
          extensions: audioExtensions
        }]
      });

      if (selected) {
        try {
          await invoke('audio_resolve_missing_file', {
            poolIndex: poolIndex,
            newPath: selected
          });
          console.log(`Successfully loaded ${entry.name} from ${selected}`);
        } catch (error) {
          console.error(`Failed to load ${entry.name}:`, error);
          await messageDialog(
            `Failed to load file: ${error}`,
            { title: "Load Error", kind: "error" }
          );
        }
      }
    }
  }
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
          await _newFile(file.width, file.height, file.fps);
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
                    if (item.type=="AudioTrack" && audioSrcMapping[item.idx]) {
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

            // Restore frameRate property with getter/setter (same pattern as in _newFile)
            // This is needed because GraphicsObject.fromJSON creates a new object without frameRate
            let _frameRate = config.framerate;  // frameRate was set from file.fps in _newFile call above
            Object.defineProperty(root, 'frameRate', {
              get() {
                return _frameRate;
              },
              set(value) {
                console.log('[frameRate setter] Setting frameRate to:', value, 'from:', _frameRate);
                console.trace('[frameRate setter] Stack trace:');
                _frameRate = value;
              },
              enumerable: true,
              configurable: false
            });
            console.log('[openFile] After restoring frameRate property, root.frameRate:', root.frameRate);

            context.objectStack = [root]
          }

          // Reset audio engine to clear any previous session data
          try {
            await invoke('audio_reset');
          } catch (error) {
            console.warn('Failed to reset audio engine:', error);
          }

          // Load audio pool if present
          if (file.audioPool && file.audioPool.length > 0) {
            console.log('[JS] Loading audio pool with', file.audioPool.length, 'entries');

            // Validate audioPool entries - skip if they don't have the expected structure
            const validEntries = file.audioPool.filter(entry => {
              // Check basic structure
              if (!entry || typeof entry.name !== 'string' || typeof entry.pool_index !== 'number') {
                console.warn('[JS] Skipping invalid audio pool entry (bad structure):', entry);
                return false;
              }

              // Log the full entry structure for debugging
              console.log('[JS] Validating entry:', JSON.stringify({
                name: entry.name,
                pool_index: entry.pool_index,
                has_embedded_data: !!entry.embedded_data,
                embedded_data_keys: entry.embedded_data ? Object.keys(entry.embedded_data) : [],
                relative_path: entry.relative_path,
                all_keys: Object.keys(entry)
              }, null, 2));

              // Check if it has either embedded data or a valid file path
              const hasEmbedded = entry.embedded_data &&
                                  entry.embedded_data.data_base64 &&
                                  entry.embedded_data.format;
              const hasValidPath = entry.relative_path &&
                                   entry.relative_path.length > 0 &&
                                   !entry.relative_path.startsWith('<embedded:');

              if (!hasEmbedded && !hasValidPath) {
                console.warn('[JS] Skipping invalid audio pool entry (no valid data or path):', {
                  name: entry.name,
                  pool_index: entry.pool_index,
                  hasEmbedded: !!entry.embedded_data,
                  relativePath: entry.relative_path
                });
                return false;
              }

              return true;
            });

            if (validEntries.length === 0) {
              console.warn('[JS] No valid audio pool entries found, skipping audio pool load');
            } else {
              validEntries.forEach((entry, i) => {
                console.log(`[JS] Entry ${i}:`, JSON.stringify({
                  pool_index: entry.pool_index,
                  name: entry.name,
                  hasEmbedded: !!entry.embedded_data,
                  hasPath: !!entry.relative_path,
                  relativePath: entry.relative_path,
                  embeddedFormat: entry.embedded_data?.format,
                  embeddedSize: entry.embedded_data?.data_base64?.length
                }, null, 2));
              });

              try {
                const missingIndices = await invoke('audio_load_pool', {
                  entries: validEntries,
                  projectPath: path
                });

                // If there are missing files, show a dialog to help user locate them
                if (missingIndices.length > 0) {
                  await handleMissingAudioFiles(missingIndices, validEntries, path);
                }
              } catch (error) {
                console.error('Failed to load audio pool:', error);
                await messageDialog(
                  `Failed to load audio files: ${error}`,
                  { title: "Audio Load Error", kind: "warning" }
                );
              }
            }
          }

          lastSaveIndex = undoStack.length;
          filePath = path;
          // Tauri thinks it is setting the title here, but it isn't getting updated
          await getCurrentWindow().setTitle(await basename(filePath));
          addRecentFile(path);

          // Ensure there's an active layer - set to first layer if none is active
          if (!context.activeObject.activeLayer && context.activeObject.layers.length > 0) {
            context.activeObject.activeLayer = context.activeObject.layers[0];
          }

          // Restore layout if saved and preference is enabled
          console.log('[JS] Layout restoration check:', {
            restoreLayoutFromFile: config.restoreLayoutFromFile,
            hasLayoutState: !!file.layoutState,
            layoutState: file.layoutState
          });

          if (config.restoreLayoutFromFile && file.layoutState) {
            try {
              console.log('[JS] Restoring saved layout:', file.layoutState);
              // Clear existing layout
              while (rootPane.firstChild) {
                rootPane.removeChild(rootPane.firstChild);
              }
              layoutElements.length = 0;
              canvases.length = 0;

              // Build layout from saved state
              buildLayout(rootPane, file.layoutState, panes, createPane, splitPane);

              // Update UI after layout change
              updateAll();
              updateUI();
              console.log('[JS] Layout restored successfully');
            } catch (error) {
              console.error('[JS] Failed to restore layout, using default:', error);
            }
          } else {
            console.log('[JS] Skipping layout restoration');
          }

          // Restore audio tracks and clips to the Rust backend
          // The fromJSON method only creates JavaScript objects,
          // but doesn't initialize them in the audio engine
          for (const audioTrack of context.activeObject.audioTracks) {
            // First, initialize the track in the Rust backend
            if (audioTrack.audioTrackId === null) {
              console.log(`[JS] Initializing track ${audioTrack.name} in audio engine`);
              try {
                await audioTrack.initializeTrack();
              } catch (error) {
                console.error(`[JS] Failed to initialize track ${audioTrack.name}:`, error);
                continue;
              }
            }

            // Then restore clips if any
            if (audioTrack.clips && audioTrack.clips.length > 0) {
              console.log(`[JS] Restoring ${audioTrack.clips.length} clips for track ${audioTrack.name}`);
              for (const clip of audioTrack.clips) {
                try {
                  // Handle MIDI clips differently from audio clips
                  if (audioTrack.type === 'midi') {
                    // For MIDI clips, restore the notes
                    if (clip.notes && clip.notes.length > 0) {
                      // Create the clip first
                      await invoke('audio_create_midi_clip', {
                        trackId: audioTrack.audioTrackId,
                        startTime: clip.startTime,
                        duration: clip.duration
                      });

                      // Update with notes
                      const noteData = clip.notes.map(note => [
                        note.startTime || note.start_time,
                        note.note,
                        note.velocity,
                        note.duration
                      ]);

                      await invoke('audio_update_midi_clip_notes', {
                        trackId: audioTrack.audioTrackId,
                        clipId: clip.clipId,
                        notes: noteData
                      });

                      console.log(`[JS] Restored MIDI clip ${clip.name} with ${clip.notes.length} notes`);
                    }
                  } else {
                    // For audio clips, restore from pool
                    await invoke('audio_add_clip', {
                      trackId: audioTrack.audioTrackId,
                      poolIndex: clip.poolIndex,
                      startTime: clip.startTime,
                      duration: clip.duration,
                      offset: clip.offset || 0.0
                    });
                    console.log(`[JS] Restored clip ${clip.name} at poolIndex ${clip.poolIndex}`);

                    // Generate waveform for the restored clip
                    try {
                      const fileInfo = await invoke('audio_get_pool_file_info', {
                        poolIndex: clip.poolIndex
                      });
                      const duration = fileInfo[0];
                      const targetPeaks = Math.floor(duration * 300);
                      const clampedPeaks = Math.max(1000, Math.min(20000, targetPeaks));

                      const waveform = await invoke('audio_get_pool_waveform', {
                        poolIndex: clip.poolIndex,
                        targetPeaks: clampedPeaks
                      });

                      clip.waveform = waveform;
                      console.log(`[JS] Generated waveform for clip ${clip.name} (${waveform.length} peaks)`);
                    } catch (waveformError) {
                      console.error(`[JS] Failed to generate waveform for clip ${clip.name}:`, waveformError);
                    }
                  }
                } catch (error) {
                  console.error(`[JS] Failed to restore clip ${clip.name}:`, error);
                }
              }
            }

            // Restore track graph (node graph for instruments/effects)
            if (file.trackGraphs && file.trackGraphs[audioTrack.idx]) {
              try {
                await invoke('audio_load_track_graph', {
                  trackId: audioTrack.audioTrackId,
                  presetJson: file.trackGraphs[audioTrack.idx],
                  projectPath: path
                });
                console.log(`[JS] Restored graph for track ${audioTrack.name}`);
              } catch (error) {
                console.error(`[JS] Failed to restore graph for track ${audioTrack.name}:`, error);
              }
            }
          }

          // Trigger UI and timeline redraw after all waveforms are loaded
          updateUI();
          updateLayers();
          if (context.timelineWidget) {
            context.timelineWidget.requestRedraw();
          }
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
  // Define filters in consistent order
  const allFilters = [
    {
      name: "Image files",
      extensions: imageExtensions,
    },
    {
      name: "Audio files",
      extensions: audioExtensions,
    },
    {
      name: "Video files",
      extensions: videoExtensions,
    },
    {
      name: "MIDI files",
      extensions: midiExtensions,
    },
    {
      name: "Lightningbeam files",
      extensions: beamExtensions,
    },
  ];

  // Reorder filters to put last used filter first
  const filterIndex = config.lastImportFilterIndex || 0;
  const reorderedFilters = [
    allFilters[filterIndex],
    ...allFilters.filter((_, i) => i !== filterIndex)
  ];

  const path = await openFileDialog({
    multiple: false,
    directory: false,
    filters: reorderedFilters,
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
    const ext = getFileExtension(filename);

    // Detect and save which filter was used based on file extension
    let usedFilterIndex = 0;
    if (audioExtensions.includes(ext)) {
      usedFilterIndex = 1; // Audio
    } else if (videoExtensions.includes(ext)) {
      usedFilterIndex = 2; // Video
    } else if (midiExtensions.includes(ext)) {
      usedFilterIndex = 3; // MIDI
    } else if (beamExtensions.includes(ext)) {
      usedFilterIndex = 4; // Lightningbeam
    } else {
      usedFilterIndex = 0; // Image (default)
    }

    // Save to config for next time
    config.lastImportFilterIndex = usedFilterIndex;
    saveConfig();

    if (ext == "beam") {
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
    } else if (audioExtensions.includes(ext)) {
      // Handle audio files - pass file path directly to backend
      actions.addAudio.create(path, context.activeObject, filename);
    } else if (videoExtensions.includes(ext)) {
      // Handle video files
      actions.addVideo.create(path, context.activeObject, filename);
    } else if (midiExtensions.includes(ext)) {
      // Handle MIDI files
      actions.addMIDI.create(path, context.activeObject, filename);
    } else {
      // Handle image files - convert to data URL
      const { dataURL, mimeType } = await convertToDataURL(
        path,
        imageMimeTypes,
      );
      if (imageMimeTypes.indexOf(mimeType) != -1) {
        actions.addImageObject.create(50, 50, dataURL, 0, context.activeObject);
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
  // Phase 6: Check if timeline has selected keyframes first
  if (context.timelineWidget && context.timelineWidget.copySelectedKeyframes()) {
    // Keyframes were copied, don't copy objects/shapes
    return;
  }

  // Otherwise, copy objects and shapes as usual
  clipboard = [];
  for (let object of context.selection) {
    clipboard.push(object.toJSON(true));
  }
  for (let shape of context.shapeselection) {
    clipboard.push(shape.toJSON(true));
  }
}

function paste() {
  // Phase 6: Check if timeline has keyframes in clipboard first
  if (context.timelineWidget && context.timelineWidget.pasteKeyframes()) {
    // Keyframes were pasted
    return;
  }

  // Otherwise, paste objects and shapes as usual
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

/**
 * Add keyframes to AnimationData curves at the current playhead position
 * For new timeline system (Phase 5)
 */
function addKeyframeAtPlayhead() {
  console.log('addKeyframeAtPlayhead called');

  // Get the timeline widget and current time
  if (!context.timelineWidget) {
    console.warn('Timeline widget not available');
    return;
  }

  const currentTime = context.timelineWidget.timelineState.currentTime;
  console.log(`Current time: ${currentTime}`);

  // Determine which object to add keyframes to based on selection
  let targetObjects = [];

  // If shapes are selected, add keyframes to those shapes
  if (context.shapeselection && context.shapeselection.length > 0) {
    console.log(`Found ${context.shapeselection.length} selected shapes`);
    targetObjects = context.shapeselection;
  }
  // If objects are selected, add keyframes to those objects
  else if (context.selection && context.selection.length > 0) {
    console.log(`Found ${context.selection.length} selected objects`);
    targetObjects = context.selection;
  }
  // Otherwise, if no selection, don't do anything
  else {
    console.log('No shapes or objects selected to add keyframes to');
    console.log('context.shapeselection:', context.shapeselection);
    console.log('context.selection:', context.selection);
    return;
  }

  // For each selected object/shape, add keyframes to all its curves
  for (let obj of targetObjects) {
    // Determine if this is a shape or an object
    const isShape = obj.constructor.name !== 'GraphicsObject';

    // Find which layer this object/shape belongs to
    let animationData = null;

    if (isShape) {
      // For shapes, find the layer recursively
      const findShapeLayer = (searchObj) => {
        for (let layer of searchObj.children) {
          if (layer.shapes && layer.shapes.includes(obj)) {
            animationData = layer.animationData;
            return true;
          }
          if (layer.children) {
            for (let child of layer.children) {
              if (findShapeLayer(child)) return true;
            }
          }
        }
        return false;
      };
      findShapeLayer(context.activeObject);
    } else {
      // For objects (groups), find the parent layer
      for (let layer of context.activeObject.allLayers) {
        if (layer.children && layer.children.includes(obj)) {
          animationData = layer.animationData;
          break;
        }
      }
    }

    if (!animationData) continue;

    // Special handling for shapes: duplicate shape with incremented shapeIndex
    if (isShape) {
      // Find the layer that contains this shape
      let parentLayer = null;
      const findShapeLayerObj = (searchObj) => {
        for (let layer of searchObj.children) {
          if (layer.shapes && layer.shapes.includes(obj)) {
            parentLayer = layer;
            return true;
          }
          if (layer.children) {
            for (let child of layer.children) {
              if (findShapeLayerObj(child)) return true;
            }
          }
        }
        return false;
      };
      findShapeLayerObj(context.activeObject);

      if (parentLayer) {
        // Find the highest shapeIndex for this shapeId
        const shapesWithSameId = parentLayer.shapes.filter(s => s.shapeId === obj.shapeId);
        let maxShapeIndex = 0;
        for (let shape of shapesWithSameId) {
          maxShapeIndex = Math.max(maxShapeIndex, shape.shapeIndex || 0);
        }
        const newShapeIndex = maxShapeIndex + 1;

        // Duplicate the shape with new shapeIndex
        const shapeJSON = obj.toJSON(false);  // Don't randomize UUIDs
        shapeJSON.idx = uuidv4();  // But do create a new idx for the duplicate
        shapeJSON.shapeIndex = newShapeIndex;
        const newShape = Shape.fromJSON(shapeJSON, parentLayer);
        parentLayer.shapes.push(newShape);

        // Add keyframes to all shape curves (exists, zOrder, shapeIndex)
        // This allows controlling timing, z-order, and morphing
        const existsCurve = animationData.getOrCreateCurve(`shape.${obj.shapeId}.exists`);
        const existsValue = existsCurve.interpolate(currentTime);
        if (existsValue === null) {
          // No previous keyframe, default to visible
          existsCurve.addKeyframe(new Keyframe(currentTime, 1, 'hold'));
        } else {
          // Add keyframe with current interpolated value
          existsCurve.addKeyframe(new Keyframe(currentTime, existsValue, 'hold'));
        }

        const zOrderCurve = animationData.getOrCreateCurve(`shape.${obj.shapeId}.zOrder`);
        const zOrderValue = zOrderCurve.interpolate(currentTime);
        if (zOrderValue === null) {
          // No previous keyframe, find current z-order from layer
          const currentZOrder = parentLayer.shapes.indexOf(obj);
          zOrderCurve.addKeyframe(new Keyframe(currentTime, currentZOrder, 'hold'));
        } else {
          // Add keyframe with current interpolated value
          zOrderCurve.addKeyframe(new Keyframe(currentTime, zOrderValue, 'hold'));
        }

        const shapeIndexCurve = animationData.getOrCreateCurve(`shape.${obj.shapeId}.shapeIndex`);
        // Check if a keyframe already exists at this time to preserve its interpolation type
        const framerate = context.config?.framerate || 24;
        const timeResolution = (1 / framerate) / 2;
        const existingShapeIndexKf = shapeIndexCurve.getKeyframeAtTime(currentTime, timeResolution);
        const interpolationType = existingShapeIndexKf ? existingShapeIndexKf.interpolation : 'linear';
        const shapeIndexKeyframe = new Keyframe(currentTime, newShapeIndex, interpolationType);
        // Preserve easeIn/easeOut if they exist
        if (existingShapeIndexKf && existingShapeIndexKf.easeIn) shapeIndexKeyframe.easeIn = existingShapeIndexKf.easeIn;
        if (existingShapeIndexKf && existingShapeIndexKf.easeOut) shapeIndexKeyframe.easeOut = existingShapeIndexKf.easeOut;
        shapeIndexCurve.addKeyframe(shapeIndexKeyframe);

        console.log(`Created new shape version with shapeIndex ${newShapeIndex} at time ${currentTime}`);
      }
    } else {
      // For objects (not shapes), add keyframes to all curves
      const curves = [];
      const prefix = `child.${obj.idx}.`;

      for (let curveName in animationData.curves) {
        if (curveName.startsWith(prefix)) {
          curves.push(animationData.curves[curveName]);
        }
      }

      // For each curve, add a keyframe at the current time with the interpolated value
      for (let curve of curves) {
        // Get the current interpolated value at this time
        const currentValue = curve.interpolate(currentTime);

        // Check if there's already a keyframe at this exact time
        const existingKeyframe = curve.keyframes.find(kf => Math.abs(kf.time - currentTime) < 0.001);

        if (existingKeyframe) {
          // Update the existing keyframe's value
          existingKeyframe.value = currentValue;
          console.log(`Updated keyframe at time ${currentTime} on ${curve.parameter}`);
        } else {
          // Create a new keyframe
          const newKeyframe = new Keyframe(
            currentTime,
            currentValue,
            'linear' // Default to linear interpolation
          );

          curve.addKeyframe(newKeyframe);
          console.log(`Added keyframe at time ${currentTime} on ${curve.parameter} with value ${currentValue}`);
        }
      }
    }
  }

  // Trigger a redraw of the timeline
  if (context.timelineWidget.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }

  console.log(`Added keyframes at time ${currentTime} for ${targetObjects.length} object(s)`);
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

    // Check if this is a pinch-zoom gesture (ctrlKey is set on trackpad pinch)
    if (event.ctrlKey) {
      // Pinch zoom - zoom in/out based on deltaY
      const zoomFactor = event.deltaY > 0 ? 0.95 : 1.05;
      const oldZoom = context.zoomLevel;
      context.zoomLevel = Math.max(1/8, Math.min(8, context.zoomLevel * zoomFactor));

      // Update scroll position to zoom towards mouse
      if (context.mousePos) {
        const actualZoomFactor = context.zoomLevel / oldZoom;
        stage.offsetX = (stage.offsetX + context.mousePos.x) * actualZoomFactor - context.mousePos.x;
        stage.offsetY = (stage.offsetY + context.mousePos.y) * actualZoomFactor - context.mousePos.y;
      }

      updateUI();
      updateMenu();
    } else {
      // Regular scroll
      const deltaX = event.deltaX * config.scrollSpeed;
      const deltaY = event.deltaY * config.scrollSpeed;

      stage.offsetX += deltaX;
      stage.offsetY += deltaY;
      const currentTime = Date.now();
      if (currentTime - lastResizeTime > throttleIntervalMs) {
        lastResizeTime = currentTime;
        updateUI();
      }
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
    const audioTypes = ["audio/mpeg"];
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
    console.log("POINTERDOWN EVENT - context.mode:", context.mode);
    let mouse = getMousePos(stage, e);
    console.log("Mouse position:", mouse);
    root.handleMouseEvent("mousedown", mouse.x, mouse.y)
    mouse = context.activeObject.transformMouse(mouse);
    let selection;
    switch (context.mode) {
      case "rectangle":
      case "ellipse":
      case "draw":
        // context.mouseDown = true;
        // context.activeShape = new Shape(mouse.x, mouse.y, context, uuidv4());
        // context.lastMouse = mouse;
        break;
      case "select":
        // No longer need keyframe check with AnimationData system
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
                  const layer = context.activeObject.activeLayer;
                  const time = context.activeObject.currentTime || 0;
                  context.activeAction = actions.moveObjects.initialize(
                    context.selection,
                    layer,
                    time,
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

                // Check if child exists using AnimationData curves
                let currentTime = context.activeObject.currentTime || 0;
                let childX = context.activeObject.activeLayer.animationData.interpolate(`child.${child.idx}.x`, currentTime);
                let childY = context.activeObject.activeLayer.animationData.interpolate(`child.${child.idx}.y`, currentTime);

                // Skip if child doesn't have position data at current time
                if (childX === null || childY === null) continue;

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
        // Paint bucket is now handled in Layer.mousedown (line ~3458)
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
    let mouse = getMousePos(stage, e);
    root.handleMouseEvent("mouseup", mouse.x, mouse.y)
    mouse = context.activeObject.transformMouse(mouse);
    switch (context.mode) {
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
          // Add the shape to selection after editing
          if (e.shiftKey) {
            if (!context.shapeselection.includes(context.activeCurve.shape)) {
              context.shapeselection.push(context.activeCurve.shape);
            }
          } else {
            context.shapeselection = [context.activeCurve.shape];
          }
          actions.select.create();
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
    switch (context.mode) {
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
            // TODO: Add user preference for keyframing behavior:
            // - Auto-keyframe (current): create/update keyframe at current time
            // - Edit previous (Flash-style): update most recent keyframe before current time
            // - Ephemeral (Blender-style): changes don't persist without manual keyframe
            // Could also add modifier key (e.g. Shift) to toggle between modes

            // Move selected children (groups) using AnimationData with auto-keyframing
            for (let child of context.selection) {
              let currentTime = context.activeObject.currentTime || 0;
              let layer = context.activeObject.activeLayer;

              // Get current position from AnimationData
              let childX = layer.animationData.interpolate(`child.${child.idx}.x`, currentTime);
              let childY = layer.animationData.interpolate(`child.${child.idx}.y`, currentTime);

              // Skip if child doesn't have position data
              if (childX === null || childY === null) continue;

              // Update position
              let newX = childX + (mouse.x - context.lastMouse.x);
              let newY = childY + (mouse.y - context.lastMouse.y);

              // Auto-keyframe: create/update keyframe at current time
              layer.animationData.addKeyframe(`child.${child.idx}.x`, new Keyframe(currentTime, newX, 'linear'));
              layer.animationData.addKeyframe(`child.${child.idx}.y`, new Keyframe(currentTime, newY, 'linear'));

              // Trigger timeline redraw
              if (context.timelineWidget && context.timelineWidget.requestRedraw) {
                context.timelineWidget.requestRedraw();
              }
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
          // Use getVisibleShapes instead of currentFrame.shapes
          let currentTime = context.activeObject?.currentTime || 0;
          let layer = context.activeObject?.activeLayer;
          if (layer) {
            for (let shape of layer.getVisibleShapes(currentTime)) {
              if (hitTest(regionToBbox(context.selectionRect), shape)) {
                context.shapeselection.push(shape);
              }
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
    modeswitcher: switch (context.mode) {
      case "select":
        for (let i = context.activeObject.activeLayer.children.length - 1; i >= 0; i--) {
          let child = context.activeObject.activeLayer.children[i];
          // Check if child exists at current time using AnimationData
          // null means no exists curve (defaults to visible)
          const existsValue = context.activeObject.activeLayer.animationData.interpolate(
            `object.${child.idx}.exists`,
            context.activeObject.currentTime
          );
          if (existsValue !== null && existsValue <= 0) continue;
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
          context.activeObject.setTime(0);
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
    toolbtn.setAttribute("data-tool", tool); // For UI testing
    let icon = document.createElement("img");
    icon.className = "icon";
    icon.src = tools[tool].icon;
    toolbtn.appendChild(icon);
    tools_scroller.appendChild(toolbtn);
    toolbtn.addEventListener("click", () => {
      context.mode = tool;
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

function timelineDeprecated() {
  let timeline_cvs = document.createElement("canvas");
  timeline_cvs.className = "timeline-deprecated";

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
      context.activeObject.audioTracks.length * layerHeight +
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
          context.activeObject.currentLayer = i - context.activeObject.audioTracks.length;
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

function timeline() {
  let canvas = document.createElement("canvas");
  canvas.className = "timeline";

  // Create TimelineWindowV2 widget
  const timelineWidget = new TimelineWindowV2(0, 0, context);

  // Store reference in context for zoom controls
  context.timelineWidget = timelineWidget;

  // Update canvas size based on container
  function updateCanvasSize() {
    const canvasStyles = window.getComputedStyle(canvas);
    canvas.width = parseInt(canvasStyles.width);
    canvas.height = parseInt(canvasStyles.height);

    // Update widget dimensions
    timelineWidget.width = canvas.width;
    timelineWidget.height = canvas.height;

    // Render
    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    timelineWidget.draw(ctx);
  }

  // Store updateCanvasSize on the widget so zoom controls can trigger redraw
  timelineWidget.requestRedraw = updateCanvasSize;

  // Add custom property to store the time format toggle button
  // so createPane can add it to the header
  canvas.headerControls = () => {
    const controls = [];

    // Playback controls group
    const playbackGroup = document.createElement("div");
    playbackGroup.className = "playback-controls-group";

    // Go to start button
    const startButton = document.createElement("button");
    startButton.className = "playback-btn playback-btn-start";
    startButton.title = "Go to Start";
    startButton.addEventListener("click", goToStart);
    playbackGroup.appendChild(startButton);

    // Rewind button
    const rewindButton = document.createElement("button");
    rewindButton.className = "playback-btn playback-btn-rewind";
    rewindButton.title = "Rewind";
    rewindButton.addEventListener("click", rewind);
    playbackGroup.appendChild(rewindButton);

    // Play/Pause button
    const playPauseButton = document.createElement("button");
    playPauseButton.className = context.playing ? "playback-btn playback-btn-pause" : "playback-btn playback-btn-play";
    playPauseButton.title = context.playing ? "Pause" : "Play";
    playPauseButton.addEventListener("click", playPause);

    // Store reference so playPause() can update it
    context.playPauseButton = playPauseButton;

    playbackGroup.appendChild(playPauseButton);

    // Fast-forward button
    const ffButton = document.createElement("button");
    ffButton.className = "playback-btn playback-btn-ff";
    ffButton.title = "Fast Forward";
    ffButton.addEventListener("click", advance);
    playbackGroup.appendChild(ffButton);

    // Go to end button
    const endButton = document.createElement("button");
    endButton.className = "playback-btn playback-btn-end";
    endButton.title = "Go to End";
    endButton.addEventListener("click", goToEnd);
    playbackGroup.appendChild(endButton);

    controls.push(playbackGroup);

    // Record button (separate group)
    const recordGroup = document.createElement("div");
    recordGroup.className = "playback-controls-group";

    const recordButton = document.createElement("button");
    recordButton.className = context.isRecording ? "playback-btn playback-btn-record recording" : "playback-btn playback-btn-record";
    recordButton.title = context.isRecording ? "Stop Recording" : "Record";
    recordButton.addEventListener("click", toggleRecording);
    recordGroup.appendChild(recordButton);

    controls.push(recordGroup);

    // Metronome button (only visible in measures mode)
    const metronomeGroup = document.createElement("div");
    metronomeGroup.className = "playback-controls-group";

    // Initially hide if not in measures mode
    if (timelineWidget.timelineState.timeFormat !== 'measures') {
      metronomeGroup.style.display = 'none';
    }

    const metronomeButton = document.createElement("button");
    metronomeButton.className = context.metronomeEnabled
      ? "playback-btn playback-btn-metronome active"
      : "playback-btn playback-btn-metronome";
    metronomeButton.title = context.metronomeEnabled ? "Disable Metronome" : "Enable Metronome";

    // Load SVG inline for currentColor support
    (async () => {
      try {
        const response = await fetch('./assets/metronome.svg');
        const svgText = await response.text();
        metronomeButton.innerHTML = svgText;
      } catch (error) {
        console.error('Failed to load metronome icon:', error);
      }
    })();

    metronomeButton.addEventListener("click", async () => {
      context.metronomeEnabled = !context.metronomeEnabled;
      const { invoke } = window.__TAURI__.core;
      try {
        await invoke('set_metronome_enabled', { enabled: context.metronomeEnabled });
        // Update button appearance
        metronomeButton.className = context.metronomeEnabled
          ? "playback-btn playback-btn-metronome active"
          : "playback-btn playback-btn-metronome";
        metronomeButton.title = context.metronomeEnabled ? "Disable Metronome" : "Enable Metronome";
      } catch (error) {
        console.error('Failed to set metronome:', error);
      }
    });
    metronomeGroup.appendChild(metronomeButton);

    // Store reference for state updates and visibility toggling
    context.metronomeButton = metronomeButton;
    context.metronomeGroup = metronomeGroup;

    controls.push(metronomeGroup);

    // Time display
    const timeDisplay = document.createElement("div");
    timeDisplay.className = "time-display";
    timeDisplay.style.cursor = "pointer";
    timeDisplay.title = "Click to change time format";

    // Function to update time display
    const updateTimeDisplay = () => {
      const currentTime = context.activeObject?.currentTime || 0;
      const timeFormat = timelineWidget.timelineState.timeFormat;
      const framerate = timelineWidget.timelineState.framerate;
      const bpm = timelineWidget.timelineState.bpm;
      const timeSignature = timelineWidget.timelineState.timeSignature;

      if (timeFormat === 'frames') {
        // Frames mode: show frame number and framerate
        const frameNumber = Math.floor(currentTime * framerate);

        timeDisplay.innerHTML = `
          <div class="time-value time-frame-clickable" data-action="toggle-format">${frameNumber}</div>
          <div class="time-label">FRAME</div>
          <div class="time-fps-group time-fps-clickable" data-action="edit-fps">
            <div class="time-value">${framerate}</div>
            <div class="time-label">FPS</div>
          </div>
        `;
      } else if (timeFormat === 'measures') {
        // Measures mode: show measure.beat, BPM, and time signature
        const { measure, beat } = timelineWidget.timelineState.timeToMeasure(currentTime);

        timeDisplay.innerHTML = `
          <div class="time-value time-frame-clickable" data-action="toggle-format">${measure}.${beat}</div>
          <div class="time-label">BAR</div>
          <div class="time-fps-group time-fps-clickable" data-action="edit-bpm">
            <div class="time-value">${bpm}</div>
            <div class="time-label">BPM</div>
          </div>
          <div class="time-fps-group time-fps-clickable" data-action="edit-time-signature">
            <div class="time-value">${timeSignature.numerator}/${timeSignature.denominator}</div>
            <div class="time-label">TIME</div>
          </div>
        `;
      } else {
        // Seconds mode: show MM:SS.mmm or HH:MM:SS.mmm
        const totalSeconds = Math.floor(currentTime);
        const milliseconds = Math.floor((currentTime - totalSeconds) * 1000);
        const seconds = totalSeconds % 60;
        const minutes = Math.floor(totalSeconds / 60) % 60;
        const hours = Math.floor(totalSeconds / 3600);

        if (hours > 0) {
          timeDisplay.innerHTML = `
            <div class="time-value">${hours}:${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}.${String(milliseconds).padStart(3, '0')}</div>
            <div class="time-label">SEC</div>
          `;
        } else {
          timeDisplay.innerHTML = `
            <div class="time-value">${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}.${String(milliseconds).padStart(3, '0')}</div>
            <div class="time-label">SEC</div>
          `;
        }
      }
    };

    // Click handler for time display
    timeDisplay.addEventListener("click", (e) => {
      const target = e.target.closest('[data-action]');

      if (!target) {
        // Clicked outside specific elements in frames mode or anywhere in seconds mode
        // Toggle format
        timelineWidget.toggleTimeFormat();
        updateTimeDisplay();
        updateCanvasSize();
        // Update metronome button visibility
        if (context.metronomeGroup) {
          context.metronomeGroup.style.display = timelineWidget.timelineState.timeFormat === 'measures' ? '' : 'none';
        }
        return;
      }

      const action = target.getAttribute('data-action');

      if (action === 'toggle-format') {
        // Clicked on frame number - toggle format
        timelineWidget.toggleTimeFormat();
        updateTimeDisplay();
        updateCanvasSize();
        // Update metronome button visibility
        if (context.metronomeGroup) {
          context.metronomeGroup.style.display = timelineWidget.timelineState.timeFormat === 'measures' ? '' : 'none';
        }
      } else if (action === 'edit-fps') {
        // Clicked on FPS - show input to edit framerate
        console.log('[FPS Edit] Starting FPS edit');
        const currentFps = timelineWidget.timelineState.framerate;
        console.log('[FPS Edit] Current FPS:', currentFps);

        const newFps = prompt('Enter framerate (FPS):', currentFps);
        console.log('[FPS Edit] Prompt returned:', newFps);

        if (newFps !== null && !isNaN(newFps) && newFps > 0) {
          const fps = parseFloat(newFps);
          console.log('[FPS Edit] Parsed FPS:', fps);

          console.log('[FPS Edit] Setting framerate on timeline state');
          timelineWidget.timelineState.framerate = fps;

          console.log('[FPS Edit] Setting frameRate on activeObject');
          context.activeObject.frameRate = fps;

          console.log('[FPS Edit] Updating time display');
          updateTimeDisplay();

          console.log('[FPS Edit] Requesting redraw');
          if (timelineWidget.requestRedraw) {
            timelineWidget.requestRedraw();
          }
          console.log('[FPS Edit] Done');
        }
      } else if (action === 'edit-bpm') {
        // Clicked on BPM - show input to edit BPM
        const currentBpm = timelineWidget.timelineState.bpm;
        const newBpm = prompt('Enter BPM (Beats Per Minute):', currentBpm);

        if (newBpm !== null && !isNaN(newBpm) && newBpm > 0) {
          const bpm = parseFloat(newBpm);
          timelineWidget.timelineState.bpm = bpm;
          context.config.bpm = bpm;
          updateTimeDisplay();
          if (timelineWidget.requestRedraw) {
            timelineWidget.requestRedraw();
          }
          // Notify all registered listeners of BPM change
          if (context.notifyBpmChange) {
            context.notifyBpmChange(bpm);
          }
        }
      } else if (action === 'edit-time-signature') {
        // Clicked on time signature - show custom dropdown with common options
        const currentTimeSig = timelineWidget.timelineState.timeSignature;
        const currentValue = `${currentTimeSig.numerator}/${currentTimeSig.denominator}`;

        // Create a custom dropdown list
        const dropdown = document.createElement('div');
        dropdown.className = 'time-signature-dropdown';
        dropdown.style.position = 'absolute';
        dropdown.style.left = e.clientX + 'px';
        dropdown.style.top = e.clientY + 'px';
        dropdown.style.fontSize = '14px';
        dropdown.style.backgroundColor = 'var(--background-color)';
        dropdown.style.color = 'var(--label-color)';
        dropdown.style.border = '1px solid var(--shadow)';
        dropdown.style.borderRadius = '4px';
        dropdown.style.zIndex = '10000';
        dropdown.style.maxHeight = '300px';
        dropdown.style.overflowY = 'auto';
        dropdown.style.boxShadow = '0 4px 8px rgba(0,0,0,0.3)';

        // Common time signatures
        const commonTimeSigs = ['2/4', '3/4', '4/4', '5/4', '6/8', '7/8', '9/8', '12/8', 'Other...'];

        commonTimeSigs.forEach(sig => {
          const item = document.createElement('div');
          item.textContent = sig;
          item.style.padding = '8px 12px';
          item.style.cursor = 'pointer';
          item.style.backgroundColor = 'var(--background-color)';
          item.style.color = 'var(--label-color)';

          if (sig === currentValue) {
            item.style.backgroundColor = 'var(--foreground-color)';
          }

          item.addEventListener('mouseenter', () => {
            item.style.backgroundColor = 'var(--foreground-color)';
          });

          item.addEventListener('mouseleave', () => {
            if (sig !== currentValue) {
              item.style.backgroundColor = 'var(--background-color)';
            }
          });

          item.addEventListener('click', () => {
            document.body.removeChild(dropdown);

            if (sig === 'Other...') {
              // Show prompt for custom time signature
              const newTimeSig = prompt(
                'Enter time signature (e.g., "4/4", "3/4", "6/8"):',
                currentValue
              );

              if (newTimeSig !== null) {
                const match = newTimeSig.match(/^(\d+)\/(\d+)$/);
                if (match) {
                  const numerator = parseInt(match[1]);
                  const denominator = parseInt(match[2]);

                  if (numerator > 0 && denominator > 0) {
                    timelineWidget.timelineState.timeSignature = { numerator, denominator };
                    context.config.timeSignature = { numerator, denominator };
                    updateTimeDisplay();
                    if (timelineWidget.requestRedraw) {
                      timelineWidget.requestRedraw();
                    }
                  }
                } else {
                  alert('Invalid time signature format. Please use format like "4/4" or "6/8".');
                }
              }
            } else {
              // Parse the selected common time signature
              const match = sig.match(/^(\d+)\/(\d+)$/);
              if (match) {
                const numerator = parseInt(match[1]);
                const denominator = parseInt(match[2]);
                timelineWidget.timelineState.timeSignature = { numerator, denominator };
                context.config.timeSignature = { numerator, denominator };
                updateTimeDisplay();
                if (timelineWidget.requestRedraw) {
                  timelineWidget.requestRedraw();
                }
              }
            }
          });

          dropdown.appendChild(item);
        });

        document.body.appendChild(dropdown);
        dropdown.focus();

        // Close dropdown when clicking outside
        const closeDropdown = (event) => {
          if (!dropdown.contains(event.target)) {
            if (document.body.contains(dropdown)) {
              document.body.removeChild(dropdown);
            }
            document.removeEventListener('click', closeDropdown);
          }
        };

        setTimeout(() => {
          document.addEventListener('click', closeDropdown);
        }, 0);
      }
    });

    // Initial update
    updateTimeDisplay();

    // Store reference for updates
    context.timeDisplay = timeDisplay;
    context.updateTimeDisplay = updateTimeDisplay;

    controls.push(timeDisplay);

    return controls;
  };

  // Set up ResizeObserver
  const resizeObserver = new ResizeObserver(() => {
    updateCanvasSize();
  });
  resizeObserver.observe(canvas);

  // Mouse event handlers
  canvas.addEventListener("pointerdown", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    // Prevent default drag behavior on canvas
    e.preventDefault();

    // Capture pointer to ensure we get move/up events even if cursor leaves canvas
    canvas.setPointerCapture(e.pointerId);

    // Store event for modifier key access during clicks (for Shift-click multi-select)
    timelineWidget.lastClickEvent = e;
    // Also store for drag operations initially
    timelineWidget.lastDragEvent = e;

    timelineWidget.handleMouseEvent("mousedown", x, y);
    updateCanvasSize(); // Redraw after interaction
  });

  canvas.addEventListener("pointermove", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    // Store event for modifier key access during drag (for Shift-drag constraint)
    timelineWidget.lastDragEvent = e;

    timelineWidget.handleMouseEvent("mousemove", x, y);

    // Update cursor based on widget's cursor property
    if (timelineWidget.cursor) {
      canvas.style.cursor = timelineWidget.cursor;
    }

    updateCanvasSize(); // Redraw after interaction
  });

  canvas.addEventListener("pointerup", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    // Release pointer capture
    canvas.releasePointerCapture(e.pointerId);

    timelineWidget.handleMouseEvent("mouseup", x, y);
    updateCanvasSize(); // Redraw after interaction
  });

  // Context menu (right-click) for deleting keyframes
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault(); // Prevent default browser context menu

    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    // Store event for access to clientX/clientY for menu positioning
    timelineWidget.lastEvent = e;
    // Also store as click event for consistency
    timelineWidget.lastClickEvent = e;

    timelineWidget.handleMouseEvent("contextmenu", x, y);
    updateCanvasSize(); // Redraw after interaction
  });

  // Add wheel event for pinch-zoom support
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();

    // Get mouse position
    const rect = canvas.getBoundingClientRect();
    const mouseX = event.clientX - rect.left;
    const mouseY = event.clientY - rect.top;

    // Check if this is a pinch-zoom gesture (ctrlKey is set on trackpad pinch)
    if (event.ctrlKey) {
      // Pinch zoom - zoom in/out based on deltaY
      const zoomFactor = event.deltaY > 0 ? 0.95 : 1.05;
      const oldPixelsPerSecond = timelineWidget.timelineState.pixelsPerSecond;

      // Adjust mouse position to account for track header offset
      const timelineMouseX = mouseX - timelineWidget.trackHeaderWidth;

      // Calculate the time under the mouse BEFORE zooming
      const mouseTimeBeforeZoom = timelineWidget.timelineState.pixelToTime(timelineMouseX);

      // Apply zoom
      timelineWidget.timelineState.pixelsPerSecond *= zoomFactor;

      // Clamp to reasonable range
      timelineWidget.timelineState.pixelsPerSecond = Math.max(10, Math.min(10000, timelineWidget.timelineState.pixelsPerSecond));

      // Adjust viewport so the time under the mouse stays in the same place
      // We want: pixelToTime(timelineMouseX) == mouseTimeBeforeZoom
      // pixelToTime(timelineMouseX) = (timelineMouseX / pixelsPerSecond) + viewportStartTime
      // So: viewportStartTime = mouseTimeBeforeZoom - (timelineMouseX / pixelsPerSecond)
      timelineWidget.timelineState.viewportStartTime = mouseTimeBeforeZoom - (timelineMouseX / timelineWidget.timelineState.pixelsPerSecond);
      timelineWidget.timelineState.viewportStartTime = Math.max(0, timelineWidget.timelineState.viewportStartTime);

      updateCanvasSize();
    } else {
      // Regular scroll - handle both horizontal and vertical scrolling everywhere
      const deltaX = event.deltaX * config.scrollSpeed;
      const deltaY = event.deltaY * config.scrollSpeed;

      // Horizontal scroll for timeline
      timelineWidget.timelineState.viewportStartTime += deltaX / timelineWidget.timelineState.pixelsPerSecond;
      timelineWidget.timelineState.viewportStartTime = Math.max(0, timelineWidget.timelineState.viewportStartTime);

      // Vertical scroll for tracks
      timelineWidget.trackScrollOffset -= deltaY;

      // Clamp scroll offset
      const trackAreaHeight = canvas.height - timelineWidget.ruler.height;
      const totalTracksHeight = timelineWidget.trackHierarchy.getTotalHeight();
      const maxScroll = Math.min(0, trackAreaHeight - totalTracksHeight);
      timelineWidget.trackScrollOffset = Math.max(maxScroll, Math.min(0, timelineWidget.trackScrollOffset));

      updateCanvasSize();
    }
  });

  updateCanvasSize();
  return canvas;
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

  // Create start screen with callback
  createStartScreen(async (options) => {
    hideStartScreen();

    if (options.type === 'new') {
      // Create new project with selected focus
      await _newFile(
        options.width || 800,
        options.height || 600,
        options.fps || 24,
        options.projectFocus
      );
    } else if (options.type === 'reopen' || options.type === 'recent') {
      // Open existing file
      await _open(options.filePath);
    }
  });

  console.log('[startup] window.openedFiles:', window.openedFiles);
  console.log('[startup] config.reopenLastSession:', config.reopenLastSession);
  console.log('[startup] config.recentFiles:', config.recentFiles);

  // Always update start screen data so it's ready when needed
  await updateStartScreen(config);

  if (!window.openedFiles?.length) {
    if (config.reopenLastSession && config.recentFiles?.length) {
      console.log('[startup] Reopening last session:', config.recentFiles[0]);
      document.body.style.cursor = "wait"
      setTimeout(()=>_open(config.recentFiles[0]), 10)
    } else {
      console.log('[startup] Showing start screen');
      // Show start screen
      showStartScreen();
    }
  } else {
    console.log('[startup] Files already opened, skipping start screen');
  }
}

startup();

// Track maximized pane state
let maximizedPane = null;
let savedPaneParent = null;
let savedRootPaneChildren = [];
let savedRootPaneClasses = null;

function toggleMaximizePane(paneDiv) {
  if (maximizedPane === paneDiv) {
    // Restore layout
    if (savedPaneParent && savedRootPaneChildren.length > 0) {
      // Remove pane from root
      rootPane.removeChild(paneDiv);

      // Restore all root pane children
      while (rootPane.firstChild) {
        rootPane.removeChild(rootPane.firstChild);
      }
      for (const child of savedRootPaneChildren) {
        rootPane.appendChild(child);
      }

      // Put pane back in its original parent
      savedPaneParent.appendChild(paneDiv);

      // Restore root pane classes
      if (savedRootPaneClasses) {
        rootPane.className = savedRootPaneClasses;
      }

      savedPaneParent = null;
      savedRootPaneChildren = [];
      savedRootPaneClasses = null;
    }
    maximizedPane = null;

    // Update button
    const btn = paneDiv.querySelector('.maximize-btn');
    if (btn) {
      btn.innerHTML = "⛶";
      btn.title = "Maximize Pane";
    }

    // Trigger updates
    updateAll();
  } else {
    // Maximize pane
    // Save pane's current parent
    savedPaneParent = paneDiv.parentElement;

    // Save all root pane children
    savedRootPaneChildren = Array.from(rootPane.children);
    savedRootPaneClasses = rootPane.className;

    // Remove pane from its parent
    savedPaneParent.removeChild(paneDiv);

    // Clear root pane
    while (rootPane.firstChild) {
      rootPane.removeChild(rootPane.firstChild);
    }

    // Add only the maximized pane to root
    rootPane.appendChild(paneDiv);
    maximizedPane = paneDiv;

    // Update button
    const btn = paneDiv.querySelector('.maximize-btn');
    if (btn) {
      btn.innerHTML = "⛶"; // Could use different icon for restore
      btn.title = "Restore Layout";
    }

    // Trigger updates
    updateAll();
  }
}

function createPaneMenu(div) {
  const menuItems = ["Item 1", "Item 2", "Item 3"]; // The items for the menu

  // Get the menu container (create a new div for the menu)
  const popupMenu = document.createElement("div");
  popupMenu.id = "popupMenu"; // Set the ID to ensure we can target it later

  // Create a <ul> element to hold the list items
  const ul = document.createElement("ul");

  // Loop through the menuItems array and create a <li> for each item
  for (let pane in panes) {
    // Skip deprecated panes
    if (pane === 'timelineDeprecated') {
      continue;
    }

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

      // Position the menu intelligently to stay onscreen
      const buttonRect = event.target.getBoundingClientRect();
      const menuRect = popupMenu.getBoundingClientRect();

      // Default: position below and to the right of the button
      let left = buttonRect.left;
      let top = buttonRect.bottom + window.scrollY;

      // Check if menu goes off the right edge
      if (left + menuRect.width > window.innerWidth) {
        // Align right edge of menu with right edge of button
        left = buttonRect.right - menuRect.width;
      }

      // Check if menu goes off the bottom edge
      if (buttonRect.bottom + menuRect.height > window.innerHeight) {
        // Position above the button instead
        top = buttonRect.top + window.scrollY - menuRect.height;
      }

      // Ensure menu doesn't go off the left edge
      left = Math.max(0, left);

      // Ensure menu doesn't go off the top edge
      top = Math.max(window.scrollY, top);

      popupMenu.style.left = `${left}px`;
      popupMenu.style.top = `${top}px`;
    }

    // Prevent the click event from propagating to the window click listener
    event.stopPropagation();
  });

  // Add custom header controls if the content element provides them
  if (content.headerControls && typeof content.headerControls === 'function') {
    const controls = content.headerControls();
    for (const control of controls) {
      header.appendChild(control);
    }
  }

  // Add maximize/restore button in top right
  const maximizeBtn = document.createElement("button");
  maximizeBtn.className = "maximize-btn";
  maximizeBtn.title = "Maximize Pane";
  maximizeBtn.innerHTML = "⛶"; // Maximize icon
  maximizeBtn.addEventListener("click", () => {
    toggleMaximizePane(div);
  });
  header.appendChild(maximizeBtn);

  div.className = "vertical-grid pane";
  div.setAttribute("data-pane-name", paneType.name);
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
        event.preventDefault(); // Prevent text selection during drag
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

      // Check if children contain nested splits to determine which joins are unambiguous
      const leftUpChild = div.children[0];
      const rightDownChild = div.children[1];

      // A child is a leaf if it's a panecontainer that directly contains another panecontainer
      // A child has nested splits if it's a panecontainer that contains a grid
      const leftUpHasSplit = leftUpChild &&
        leftUpChild.classList.contains("panecontainer") &&
        leftUpChild.firstElementChild &&
        (leftUpChild.firstElementChild.classList.contains("horizontal-grid") ||
         leftUpChild.firstElementChild.classList.contains("vertical-grid")) &&
        leftUpChild.firstElementChild.hasAttribute("lb-percent");

      const rightDownHasSplit = rightDownChild &&
        rightDownChild.classList.contains("panecontainer") &&
        rightDownChild.firstElementChild &&
        (rightDownChild.firstElementChild.classList.contains("horizontal-grid") ||
         rightDownChild.firstElementChild.classList.contains("vertical-grid")) &&
        rightDownChild.firstElementChild.hasAttribute("lb-percent");

      // Join Left/Up is unambiguous if we're keeping the left/up side (which may have splits)
      // and removing the right/down side (which should be a simple pane)
      const canJoinLeftUp = !rightDownHasSplit;

      // Join Right/Down is unambiguous if we're keeping the right/down side (which may have splits)
      // and removing the left/up side (which should be a simple pane)
      const canJoinRightDown = !leftUpHasSplit;

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
          {
            id: "ctx_option3",
            text: horiz ? "Join Left" : "Join Up",
            enabled: canJoinLeftUp,
            action: () => {
              // Join left/up: remove the left/up pane, keep the right/down pane
              const keepChild = div.children[1];

              // Move all children from the kept panecontainer to the parent
              const children = Array.from(keepChild.children);

              // Replace the split div with just the kept child's contents
              div.className = "panecontainer";
              div.innerHTML = "";
              children.forEach(child => {
                // Recursively clear explicit sizing on grid and panecontainer elements only
                function clearSizes(el) {
                  if (el.classList.contains("horizontal-grid") ||
                      el.classList.contains("vertical-grid") ||
                      el.classList.contains("panecontainer")) {
                    el.style.width = "";
                    el.style.height = "";
                    Array.from(el.children).forEach(clearSizes);
                  }
                }
                clearSizes(child);
                div.appendChild(child);
              });
              div.removeAttribute("lb-percent");

              setTimeout(() => {
                updateAll();
                updateUI();
                updateLayers();
              }, 20);
            }
          },
          {
            id: "ctx_option4",
            text: horiz ? "Join Right" : "Join Down",
            enabled: canJoinRightDown,
            action: () => {
              // Join right/down: remove the right/down pane, keep the left/up pane
              const keepChild = div.children[0];

              // Move all children from the kept panecontainer to the parent
              const children = Array.from(keepChild.children);

              // Replace the split div with just the kept child's contents
              div.className = "panecontainer";
              div.innerHTML = "";
              children.forEach(child => {
                // Recursively clear explicit sizing on grid and panecontainer elements only
                function clearSizes(el) {
                  if (el.classList.contains("horizontal-grid") ||
                      el.classList.contains("vertical-grid") ||
                      el.classList.contains("panecontainer")) {
                    el.style.width = "";
                    el.style.height = "";
                    Array.from(el.children).forEach(clearSizes);
                  }
                }
                clearSizes(child);
                div.appendChild(child);
              });
              div.removeAttribute("lb-percent");

              setTimeout(() => {
                updateAll();
                updateUI();
                updateLayers();
              }, 20);
            }
          },
        ],
      });
      await menu.popup(new PhysicalPosition(event.clientX, event.clientY));
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

// Add updateUI and updateMenu to context so widgets can call them
context.updateUI = updateUI;
context.updateMenu = updateMenu;

async function renderUI() {
  // Update video frames BEFORE drawing
  if (context.activeObject) {
    await updateVideoFrames(context.activeObject.currentTime);
  }

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
    root.draw(context)
    if (context.activeObject != root) {
      ctx.fillStyle = "rgba(255,255,255,0.5)";
      ctx.fillRect(0, 0, config.fileWidth, config.fileHeight);
      const transform = ctx.getTransform()
      context.activeObject.draw(context, true);
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
      const currentTime = context.activeObject.currentTime || 0;
      const visibleShapes = context.activeObject.activeLayer.getVisibleShapes(currentTime);
      for (let shape of visibleShapes) {
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
  if (context.mode == "transform") {
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
  // Also trigger TimelineV2 redraw if it exists
  if (context.timelineWidget?.requestRedraw) {
    context.timelineWidget.requestRedraw();
  }

  for (let canvas of document.querySelectorAll(".timeline-deprecated")) {
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
      // } else if (layer instanceof AudioTrack) {
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
    for (let audioTrack of context.activeObject.audioTracks) {
      let layerHeader = document.createElement("div");
      layerHeader.className = "layer-header";
      layerHeader.classList.add("audio");
      layerspanel.appendChild(layerHeader);
      let layerTrack = document.createElement("div");
      layerTrack.className = "layer-track";
      layerTrack.classList.add("audio");
      framescontainer.appendChild(layerTrack);
      for (let i in audioTrack.sounds) {
        let sound = audioTrack.sounds[i];
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
      layerName.innerText = audioTrack.name;
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
    for (let property in tools[context.mode].properties) {
      let prop = tools[context.mode].properties[property];
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
  console.log('[renderMenu] START - root.frameRate:', root.frameRate);
  let activeFrame;
  let activeKeyframe;
  let newFrameMenuItem;
  let newKeyframeMenuItem;
  let newBlankKeyframeMenuItem;
  let duplicateKeyframeMenuItem;
  let deleteFrameMenuItem;

  // Move this
  updateOutliner();
  console.log('[renderMenu] After updateOutliner - root.frameRate:', root.frameRate);

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

  // Legacy frame system removed - these are always false now
  activeFrame = false;
  activeKeyframe = false;
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
      {
        text: "Preferences",
        enabled: true,
        action: showPreferencesDialog,
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
        text: "Add Video Layer",
        enabled: true,
        action: addVideoLayer,
      },
      {
        text: "Add Audio Track",
        enabled: true,
        action: addEmptyAudioTrack,
        accelerator: getShortcut("addAudioTrack")
      },
      {
        text: "Add MIDI Track",
        enabled: true,
        action: addEmptyMIDITrack,
        accelerator: getShortcut("addMIDITrack")
      },
      {
        text: "Delete Layer",
        enabled: context.activeObject.layers.length > 1,
        action: actions.deleteLayer.create,
      },
      {
        text: context.activeObject.activeLayer?.visible
          ? "Hide Layer"
          : "Show Layer",
        enabled: !!context.activeObject.activeLayer,
        action: () => {
          context.activeObject.activeLayer?.toggleVisibility();
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
    enabled: (context.selection && context.selection.length > 0) ||
             (context.shapeselection && context.shapeselection.length > 0),
    accelerator: getShortcut("addKeyframe"),
    action: addKeyframeAtPlayhead,
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
        text: "Add Keyframe at Playhead",
        enabled: (context.selection && context.selection.length > 0) ||
                 (context.shapeselection && context.shapeselection.length > 0),
        action: addKeyframeAtPlayhead,
        accelerator: "K",
      },
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
        enabled: !context.playing,
        action: playPause,
        accelerator: getShortcut("playAnimation"),
      },
    ],
  });
  // Build layout submenu items
  const layoutMenuItems = [
    {
      text: "Next Layout",
      enabled: true,
      action: nextLayout,
      accelerator: getShortcut("nextLayout"),
    },
    {
      text: "Previous Layout",
      enabled: true,
      action: previousLayout,
      accelerator: getShortcut("previousLayout"),
    },
  ];

  // Add separator
  layoutMenuItems.push(await PredefinedMenuItem.new({ item: "Separator" }));

  // Add individual layouts
  for (const layoutKey of getLayoutNames()) {
    const layout = getLayout(layoutKey);
    const isCurrentLayout = config.currentLayout === layoutKey;
    layoutMenuItems.push({
      text: isCurrentLayout ? `✓ ${layout.name}` : layout.name,
      enabled: true,
      action: () => switchLayout(layoutKey),
    });
  }

  const layoutSubmenu = await Submenu.new({
    text: "Layout",
    items: layoutMenuItems,
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
      layoutSubmenu,
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
  console.log('[renderMenu] Before setAsWindowMenu - root.frameRate:', root.frameRate);
  await (macOS ? menu.setAsAppMenu() : menu.setAsWindowMenu());
  console.log('[renderMenu] END - root.frameRate:', root.frameRate);
}
updateMenu();

// Helper function to get the current track (MIDI or Audio) for node graph editing
function getCurrentTrack() {
  const activeLayer = context.activeObject?.activeLayer;
  if (!activeLayer || !(activeLayer instanceof AudioTrack)) {
    return null;
  }
  if (activeLayer.audioTrackId === null) {
    return null;
  }
  // Return both track ID and track type
  return {
    trackId: activeLayer.audioTrackId,
    trackType: activeLayer.type  // 'midi' or 'audio'
  };
}

// Backwards compatibility: function to get just the MIDI track ID
function getCurrentMidiTrack() {
  const trackInfo = getCurrentTrack();
  if (trackInfo && trackInfo.trackType === 'midi') {
    return trackInfo.trackId;
  }
  return null;
}

function nodeEditor() {
  // Create container for the node editor
  const container = document.createElement("div");
  container.id = "node-editor-container";

  // Prevent text selection during drag operations
  container.addEventListener('selectstart', (e) => {
    // Allow selection on input elements
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') {
      return;
    }
    e.preventDefault();
  });
  container.addEventListener('mousedown', (e) => {
    // Don't prevent default on inputs, textareas, or palette items (draggable)
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') {
      return;
    }
    // Don't prevent default on palette items or their children
    if (e.target.closest('.node-palette-item') || e.target.closest('.node-category-item')) {
      return;
    }
    e.preventDefault();
  });

  // Track editing context: null = main graph, {voiceAllocatorId, voiceAllocatorName} = editing template
  let editingContext = null;

  // Track palette navigation: null = showing categories, string = showing nodes in that category
  let selectedCategory = null;

  // Create breadcrumb/context header
  const header = document.createElement("div");
  header.className = "node-editor-header";
  // Initial header will be updated by updateBreadcrumb() after track info is available
  header.innerHTML = '<div class="context-breadcrumb">Node Graph</div>';
  container.appendChild(header);

  // Create the Drawflow canvas
  const editorDiv = document.createElement("div");
  editorDiv.id = "drawflow";
  editorDiv.style.position = "absolute";
  editorDiv.style.top = "40px"; // Start below header
  editorDiv.style.left = "0";
  editorDiv.style.right = "0";
  editorDiv.style.bottom = "0";
  container.appendChild(editorDiv);

  // Create node palette
  const palette = document.createElement("div");
  palette.className = "node-palette";
  container.appendChild(palette);

  // Create persistent search input
  const paletteSearch = document.createElement("div");
  paletteSearch.className = "palette-search";
  paletteSearch.innerHTML = `
    <input type="text" placeholder="Search nodes..." class="palette-search-input" value="">
    <button class="palette-search-clear" style="display: none;">×</button>
  `;
  palette.appendChild(paletteSearch);

  // Create content container that will be updated
  const paletteContent = document.createElement("div");
  paletteContent.className = "palette-content";
  palette.appendChild(paletteContent);

  // Get references to search elements
  const searchInput = paletteSearch.querySelector(".palette-search-input");
  const searchClearBtn = paletteSearch.querySelector(".palette-search-clear");

  // Create minimap
  const minimap = document.createElement("div");
  minimap.className = "node-minimap";
  minimap.style.display = 'none'; // Hidden by default
  minimap.innerHTML = `
    <canvas id="minimap-canvas"></canvas>
    <div class="minimap-viewport"></div>
  `;
  container.appendChild(minimap);

  // Category display names
  const categoryNames = {
    [NodeCategory.INPUT]: 'Inputs',
    [NodeCategory.GENERATOR]: 'Generators',
    [NodeCategory.EFFECT]: 'Effects',
    [NodeCategory.UTILITY]: 'Utilities',
    [NodeCategory.OUTPUT]: 'Outputs'
  };

  // Search state
  let searchQuery = '';

  // Handle search input changes
  searchInput.addEventListener('input', (e) => {
    searchQuery = e.target.value;
    searchClearBtn.style.display = searchQuery ? 'flex' : 'none';
    updatePalette();
  });

  // Handle search clear
  searchClearBtn.addEventListener('click', () => {
    searchQuery = '';
    searchInput.value = '';
    searchClearBtn.style.display = 'none';
    searchInput.focus();
    updatePalette();
  });

  // Function to update palette based on context and selected category
  function updatePalette() {
    const isTemplate = editingContext !== null;
    const trackInfo = getCurrentTrack();
    const isMIDI = trackInfo?.trackType === 'midi';
    const isAudio = trackInfo?.trackType === 'audio';

    if (selectedCategory === null && !searchQuery) {
      // Show categories when no search query
      const categories = getCategories().filter(category => {
        // Filter categories based on context
        if (isTemplate) {
          // In template: show all categories
          return true;
        } else {
          // In main graph: hide INPUT/OUTPUT categories that contain template nodes
          return true; // We'll filter nodes instead
        }
      });

      paletteContent.innerHTML = `
        <h3>Node Categories</h3>
        ${categories.map(category => `
          <div class="node-category-item" data-category="${category}">
            ${categoryNames[category] || category}
          </div>
        `).join('')}
      `;
    } else if (selectedCategory === null && searchQuery) {
      // Show all matching nodes across all categories when searching from main panel
      const allCategories = getCategories();
      let allNodes = [];

      allCategories.forEach(category => {
        const nodesInCategory = getNodesByCategory(category);
        allNodes = allNodes.concat(nodesInCategory);
      });

      // Filter based on context
      let filteredNodes = allNodes.filter(node => {
        if (isTemplate) {
          // In template: hide VoiceAllocator, AudioOutput, MidiInput
          return node.type !== 'VoiceAllocator' && node.type !== 'AudioOutput' && node.type !== 'MidiInput';
        } else if (isMIDI) {
          // MIDI track: hide AudioInput, show synth nodes
          return node.type !== 'TemplateInput' && node.type !== 'TemplateOutput' && node.type !== 'AudioInput';
        } else if (isAudio) {
          // Audio track: hide synth/MIDI nodes, show AudioInput
          const synthNodes = ['Oscillator', 'FMSynth', 'WavetableOscillator', 'SimpleSampler', 'MultiSampler', 'VoiceAllocator', 'MidiInput', 'MidiToCV'];
          return node.type !== 'TemplateInput' && node.type !== 'TemplateOutput' && !synthNodes.includes(node.type);
        } else {
          // Fallback: hide TemplateInput/TemplateOutput
          return node.type !== 'TemplateInput' && node.type !== 'TemplateOutput';
        }
      });

      // Apply search filter
      const query = searchQuery.toLowerCase();
      filteredNodes = filteredNodes.filter(node => {
        return node.name.toLowerCase().includes(query) ||
               node.description.toLowerCase().includes(query);
      });

      // Function to highlight search matches in text
      const highlightMatch = (text) => {
        const regex = new RegExp(`(${searchQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi');
        return text.replace(regex, '<mark>$1</mark>');
      };

      paletteContent.innerHTML = `
        <h3>Search Results</h3>
        ${filteredNodes.length > 0 ? filteredNodes.map(node => `
          <div class="node-palette-item" data-node-type="${node.type}" draggable="true" title="${node.description}">
            ${highlightMatch(node.name)}
          </div>
        `).join('') : '<div class="no-results">No matching nodes found</div>'}
      `;
    } else {
      // Show nodes in selected category
      const nodesInCategory = getNodesByCategory(selectedCategory);

      // Filter based on context
      let filteredNodes = nodesInCategory.filter(node => {
        if (isTemplate) {
          // In template: hide VoiceAllocator, AudioOutput, MidiInput
          return node.type !== 'VoiceAllocator' && node.type !== 'AudioOutput' && node.type !== 'MidiInput';
        } else if (isMIDI) {
          // MIDI track: hide AudioInput, show synth nodes
          return node.type !== 'TemplateInput' && node.type !== 'TemplateOutput' && node.type !== 'AudioInput';
        } else if (isAudio) {
          // Audio track: hide synth/MIDI nodes, show AudioInput
          const synthNodes = ['Oscillator', 'FMSynth', 'WavetableOscillator', 'SimpleSampler', 'MultiSampler', 'VoiceAllocator', 'MidiInput', 'MidiToCV'];
          return node.type !== 'TemplateInput' && node.type !== 'TemplateOutput' && !synthNodes.includes(node.type);
        } else {
          // Fallback: hide TemplateInput/TemplateOutput
          return node.type !== 'TemplateInput' && node.type !== 'TemplateOutput';
        }
      });

      // Apply search filter
      if (searchQuery) {
        const query = searchQuery.toLowerCase();
        filteredNodes = filteredNodes.filter(node => {
          return node.name.toLowerCase().includes(query) ||
                 node.description.toLowerCase().includes(query);
        });
      }

      // Function to highlight search matches in text
      const highlightMatch = (text) => {
        if (!searchQuery) return text;
        const regex = new RegExp(`(${searchQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi');
        return text.replace(regex, '<mark>$1</mark>');
      };

      paletteContent.innerHTML = `
        <div class="palette-header">
          <button class="palette-back-btn">← Back</button>
          <h3>${categoryNames[selectedCategory] || selectedCategory}</h3>
        </div>
        ${filteredNodes.length > 0 ? filteredNodes.map(node => `
          <div class="node-palette-item" data-node-type="${node.type}" draggable="true" title="${node.description}">
            ${highlightMatch(node.name)}
          </div>
        `).join('') : '<div class="no-results">No matching nodes found</div>'}
      `;
    }
  }

  updatePalette();

  // Initialize Drawflow editor (will be set up after DOM insertion)
  let editor = null;
  let nodeIdCounter = 1;

  // Track expanded VoiceAllocator nodes
  const expandedNodes = new Set(); // Set of node IDs that are expanded
  const nodeParents = new Map();   // Map of child node ID -> parent VoiceAllocator ID

  // Cache node data for undo/redo (nodeId -> {nodeType, backendId, position, parameters})
  const nodeDataCache = new Map();

  // Track node movement for undo/redo (nodeId -> {oldX, oldY})
  const nodeMoveTracker = new Map();

  // Flag to prevent recording actions during undo/redo operations
  let suppressActionRecording = false;

  // Wait for DOM insertion
  setTimeout(() => {
    const drawflowDiv = container.querySelector("#drawflow");
    if (!drawflowDiv) return;

    editor = new Drawflow(drawflowDiv);
    editor.reroute = true;
    editor.reroute_fix_curvature = true;
    editor.force_first_input = false;
    editor.start();

    // Store editor reference in context
    context.nodeEditor = editor;
    context.reloadNodeEditor = reloadGraph;
    context.nodeEditorState = {
      get suppressActionRecording() { return suppressActionRecording; },
      set suppressActionRecording(value) { suppressActionRecording = value; }
    };

    // Initialize BPM change notification system
    // This allows nodes to register callbacks to be notified when BPM changes
    const bpmChangeListeners = new Set();

    context.registerBpmChangeListener = (callback) => {
      bpmChangeListeners.add(callback);
      return () => bpmChangeListeners.delete(callback); // Return unregister function
    };

    context.notifyBpmChange = (newBpm) => {
      console.log(`BPM changed to ${newBpm}, notifying ${bpmChangeListeners.size} listeners`);
      bpmChangeListeners.forEach(callback => {
        try {
          callback(newBpm);
        } catch (error) {
          console.error('Error in BPM change listener:', error);
        }
      });
    };

    // Register a listener to update all synced Phaser nodes when BPM changes
    context.registerBpmChangeListener((newBpm) => {
      if (!editor) return;

      const module = editor.module;
      const allNodes = editor.drawflow.drawflow[module]?.data || {};

      // Beat division definitions for conversion
      const beatDivisions = [
        { label: '4 bars', multiplier: 16.0 },
        { label: '2 bars', multiplier: 8.0 },
        { label: '1 bar', multiplier: 4.0 },
        { label: '1/2', multiplier: 2.0 },
        { label: '1/4', multiplier: 1.0 },
        { label: '1/8', multiplier: 0.5 },
        { label: '1/16', multiplier: 0.25 },
        { label: '1/32', multiplier: 0.125 },
        { label: '1/2T', multiplier: 2.0/3.0 },
        { label: '1/4T', multiplier: 1.0/3.0 },
        { label: '1/8T', multiplier: 0.5/3.0 }
      ];

      // Iterate through all nodes to find synced Phaser nodes
      for (const [nodeId, nodeData] of Object.entries(allNodes)) {
        // Check if this is a Phaser node with sync enabled
        if (nodeData.name === 'Phaser' && nodeData.data.backendId !== null) {
          const nodeElement = document.getElementById(`node-${nodeId}`);
          if (!nodeElement) continue;

          const syncCheckbox = nodeElement.querySelector(`#sync-${nodeId}`);
          if (!syncCheckbox || !syncCheckbox.checked) continue;

          // Get the current rate slider value (beat division index)
          const rateSlider = nodeElement.querySelector(`input[data-param="0"]`); // rate is param 0
          if (!rateSlider) continue;

          const beatDivisionIndex = Math.min(10, Math.max(0, Math.round(parseFloat(rateSlider.value))));
          const beatsPerSecond = newBpm / 60.0;
          const quarterNotesPerCycle = beatDivisions[beatDivisionIndex].multiplier;
          const hz = beatsPerSecond / quarterNotesPerCycle;

          // Update the backend parameter
          const trackInfo = getCurrentTrack();
          if (trackInfo !== null) {
            invoke("graph_set_parameter", {
              trackId: trackInfo.trackId,
              nodeId: nodeData.data.backendId,
              paramId: 0, // rate parameter
              value: hz
            }).catch(err => {
              console.error("Failed to update Phaser rate after BPM change:", err);
            });
            console.log(`Updated Phaser node ${nodeId} rate to ${hz} Hz for BPM ${newBpm}`);
          }
        }
      }
    });

    // Initialize minimap
    const minimapCanvas = container.querySelector("#minimap-canvas");
    const minimapViewport = container.querySelector(".minimap-viewport");
    const minimapCtx = minimapCanvas.getContext('2d');

    // Set canvas size to match container
    minimapCanvas.width = 200;
    minimapCanvas.height = 150;

    function updateMinimap() {
      if (!editor) return;

      const ctx = minimapCtx;
      const canvas = minimapCanvas;

      // Clear canvas
      ctx.clearRect(0, 0, canvas.width, canvas.height);

      // Get all nodes
      const module = editor.module;
      const nodes = editor.drawflow.drawflow[module]?.data || {};
      const nodeList = Object.values(nodes);

      if (nodeList.length === 0) {
        minimap.style.display = 'none';
        return;
      }

      // Calculate bounding box of all nodes
      let minX = Infinity, minY = Infinity;
      let maxX = -Infinity, maxY = -Infinity;

      nodeList.forEach(node => {
        minX = Math.min(minX, node.pos_x);
        minY = Math.min(minY, node.pos_y);
        maxX = Math.max(maxX, node.pos_x + 160); // Approximate node width
        maxY = Math.max(maxY, node.pos_y + 100); // Approximate node height
      });

      // Add padding
      const padding = 20;
      minX -= padding;
      minY -= padding;
      maxX += padding;
      maxY += padding;

      // Calculate graph dimensions
      const graphWidth = maxX - minX;
      const graphHeight = maxY - minY;

      // Check if graph fits in viewport
      const zoom = editor.zoom || 1;
      const drawflowRect = drawflowDiv.getBoundingClientRect();
      const viewportWidth = drawflowRect.width / zoom;
      const viewportHeight = drawflowRect.height / zoom;

      // Only show minimap if graph is larger than viewport
      if (graphWidth <= viewportWidth && graphHeight <= viewportHeight) {
        minimap.style.display = 'none';
        return;
      } else {
        minimap.style.display = 'block';
      }

      // Calculate scale to fit in minimap
      const scale = Math.min(canvas.width / graphWidth, canvas.height / graphHeight);

      // Draw nodes
      ctx.fillStyle = '#666';
      nodeList.forEach(node => {
        const x = (node.pos_x - minX) * scale;
        const y = (node.pos_y - minY) * scale;
        const width = 160 * scale;
        const height = 100 * scale;

        ctx.fillRect(x, y, width, height);
      });

      // Update viewport indicator
      const canvasX = editor.canvas_x || 0;
      const canvasY = editor.canvas_y || 0;

      const viewportX = (-canvasX / zoom - minX) * scale;
      const viewportY = (-canvasY / zoom - minY) * scale;
      const viewportIndicatorWidth = (drawflowRect.width / zoom) * scale;
      const viewportIndicatorHeight = (drawflowRect.height / zoom) * scale;

      minimapViewport.style.left = Math.max(0, viewportX) + 'px';
      minimapViewport.style.top = Math.max(0, viewportY) + 'px';
      minimapViewport.style.width = viewportIndicatorWidth + 'px';
      minimapViewport.style.height = viewportIndicatorHeight + 'px';

      // Store scale info for click navigation
      minimapCanvas.dataset.scale = scale;
      minimapCanvas.dataset.minX = minX;
      minimapCanvas.dataset.minY = minY;
    }

    // Update minimap on various events
    editor.on('nodeCreated', () => setTimeout(updateMinimap, 100));
    editor.on('nodeRemoved', () => setTimeout(updateMinimap, 100));
    editor.on('nodeMoved', () => updateMinimap());

    // Update minimap on pan/zoom
    drawflowDiv.addEventListener('wheel', () => setTimeout(updateMinimap, 10));

    // Initial minimap render
    setTimeout(updateMinimap, 200);

    // Click-to-navigate on minimap
    minimapCanvas.addEventListener('mousedown', (e) => {
      const rect = minimapCanvas.getBoundingClientRect();
      const clickX = e.clientX - rect.left;
      const clickY = e.clientY - rect.top;

      const scale = parseFloat(minimapCanvas.dataset.scale || 1);
      const minX = parseFloat(minimapCanvas.dataset.minX || 0);
      const minY = parseFloat(minimapCanvas.dataset.minY || 0);

      // Convert click position to graph coordinates
      const graphX = (clickX / scale) + minX;
      const graphY = (clickY / scale) + minY;

      // Center the viewport on the clicked position
      const zoom = editor.zoom || 1;
      const drawflowRect = drawflowDiv.getBoundingClientRect();
      const viewportCenterX = drawflowRect.width / (2 * zoom);
      const viewportCenterY = drawflowRect.height / (2 * zoom);

      editor.canvas_x = -(graphX - viewportCenterX) * zoom;
      editor.canvas_y = -(graphY - viewportCenterY) * zoom;

      // Update the canvas transform
      const precanvas = drawflowDiv.querySelector('.drawflow');
      if (precanvas) {
        precanvas.style.transform = `translate(${editor.canvas_x}px, ${editor.canvas_y}px) scale(${zoom})`;
      }

      updateMinimap();
    });

    // Add reconnection support: dragging from a connected input disconnects and starts new connection
    drawflowDiv.addEventListener('mousedown', (e) => {
      // Check if clicking on an input port
      const inputPort = e.target.closest('.input');

      if (inputPort) {
        // Get the node and port information - the drawflow-node div has the id
        const drawflowNode = inputPort.closest('.drawflow-node');
        if (!drawflowNode) return;

        const nodeId = parseInt(drawflowNode.id.replace('node-', ''));

        // Access the node data directly from the current module
        const moduleName = editor.module;
        const node = editor.drawflow.drawflow[moduleName]?.data[nodeId];
        if (!node) return;

        // Get the port class (input_1, input_2, etc.)
        const portClasses = Array.from(inputPort.classList);
        const portClass = portClasses.find(c => c.startsWith('input_'));
        if (!portClass) return;

        // Check if this input has any connections
        const inputConnections = node.inputs[portClass];
        if (inputConnections && inputConnections.connections && inputConnections.connections.length > 0) {
          // Get the first connection (inputs should only have one connection)
          const connection = inputConnections.connections[0];

          // Prevent default to avoid interfering with the drag
          e.stopPropagation();
          e.preventDefault();

          // Remove the connection
          editor.removeSingleConnection(
            connection.node,
            nodeId,
            connection.input,
            portClass
          );

          // Now trigger Drawflow's connection drag from the output that was connected
          // We need to simulate starting a drag from the output port
          const outputNodeElement = document.getElementById(`node-${connection.node}`);
          if (outputNodeElement) {
            const outputPort = outputNodeElement.querySelector(`.${connection.input}`);
            if (outputPort) {
              // Dispatch a synthetic mousedown event on the output port
              // This will trigger Drawflow's normal connection start logic
              setTimeout(() => {
                const rect = outputPort.getBoundingClientRect();
                const syntheticEvent = new MouseEvent('mousedown', {
                  bubbles: true,
                  cancelable: true,
                  view: window,
                  clientX: rect.left + rect.width / 2,
                  clientY: rect.top + rect.height / 2,
                  button: 0
                });
                outputPort.dispatchEvent(syntheticEvent);

                // Then immediately dispatch a mousemove to the original cursor position
                // to start dragging the connection line
                setTimeout(() => {
                  const mousemoveEvent = new MouseEvent('mousemove', {
                    bubbles: true,
                    cancelable: true,
                    view: window,
                    clientX: e.clientX,
                    clientY: e.clientY,
                    button: 0
                  });
                  document.dispatchEvent(mousemoveEvent);
                }, 0);
              }, 0);
            }
          }
        }
      }
    }, true); // Use capture phase to intercept before Drawflow

    // Add trackpad/mousewheel scrolling support for panning
    drawflowDiv.addEventListener('wheel', (e) => {
      // Don't scroll if hovering over palette or other UI elements
      if (e.target.closest('.node-palette')) {
        return;
      }

      // Don't interfere with zoom (Ctrl+wheel)
      if (e.ctrlKey) return;

      // Prevent default scrolling behavior
      e.preventDefault();

      // Pan the canvas based on scroll direction
      const deltaX = e.deltaX;
      const deltaY = e.deltaY;

      // Update Drawflow's canvas position
      if (typeof editor.canvas_x === 'undefined') {
        editor.canvas_x = 0;
      }
      if (typeof editor.canvas_y === 'undefined') {
        editor.canvas_y = 0;
      }

      editor.canvas_x -= deltaX;
      editor.canvas_y -= deltaY;

      // Update the canvas transform
      const precanvas = drawflowDiv.querySelector('.drawflow');
      if (precanvas) {
        const zoom = editor.zoom || 1;
        precanvas.style.transform = `translate(${editor.canvas_x}px, ${editor.canvas_y}px) scale(${zoom})`;
      }
    }, { passive: false });

    // Add palette item drag-and-drop handlers using event delegation
    let draggedNodeType = null;

    // Use event delegation for click on palette items, categories, and back button
    palette.addEventListener("click", (e) => {
      // Handle back button
      const backBtn = e.target.closest(".palette-back-btn");
      if (backBtn) {
        selectedCategory = null;
        updatePalette();
        return;
      }

      // Handle category selection
      const categoryItem = e.target.closest(".node-category-item");
      if (categoryItem) {
        selectedCategory = categoryItem.getAttribute("data-category");
        updatePalette();
        return;
      }

      // Handle node selection
      const item = e.target.closest(".node-palette-item");
      if (item) {
        const nodeType = item.getAttribute("data-node-type");

        // Calculate center of visible canvas viewport
        const rect = drawflowDiv.getBoundingClientRect();
        const canvasX = editor.canvas_x || 0;
        const canvasY = editor.canvas_y || 0;
        const zoom = editor.zoom || 1;

        // Approximate node dimensions (nodes have min-width: 160px, typical height ~150px)
        const nodeWidth = 160;
        const nodeHeight = 150;

        // Center position in world coordinates, offset by half node size
        const centerX = (rect.width / 2 - canvasX) / zoom - nodeWidth / 2;
        const centerY = (rect.height / 2 - canvasY) / zoom - nodeHeight / 2;

        addNode(nodeType, centerX, centerY, null);
      }
    });

    // Use event delegation for drag events
    palette.addEventListener('dragstart', (e) => {
      const item = e.target.closest(".node-palette-item");
      if (item) {
        draggedNodeType = item.getAttribute('data-node-type');
        e.dataTransfer.effectAllowed = 'copy';
        e.dataTransfer.setData('text/plain', draggedNodeType);
        console.log('Drag started:', draggedNodeType);
      }
    });

    palette.addEventListener('dragend', (e) => {
      const item = e.target.closest(".node-palette-item");
      if (item) {
        console.log('Drag ended');
        draggedNodeType = null;
      }
    });

    // Add drop handler to drawflow canvas
    drawflowDiv.addEventListener('dragover', (e) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';

      // Check if dragging over a connection for insertion
      const nodeType = e.dataTransfer.getData('text/plain') || draggedNodeType;
      if (nodeType) {
        const nodeDef = nodeTypes[nodeType];
        if (nodeDef) {
          checkConnectionInsertionDuringDrag(e, nodeDef);
        }
      }
    });

    drawflowDiv.addEventListener('drop', (e) => {
      e.preventDefault();

      // Get node type from dataTransfer instead of global variable
      const nodeType = e.dataTransfer.getData('text/plain');
      console.log('Drop event fired, nodeType:', nodeType);

      if (!nodeType) {
        console.log('No nodeType in drop data, aborting');
        return;
      }

      // Get drop position relative to the editor
      const rect = drawflowDiv.getBoundingClientRect();

      // Use canvas_x and canvas_y which are set by the wheel scroll handler
      const canvasX = editor.canvas_x || 0;
      const canvasY = editor.canvas_y || 0;
      const zoom = editor.zoom || 1;

      // Approximate node dimensions (nodes have min-width: 160px, typical height ~150px)
      const nodeWidth = 160;
      const nodeHeight = 150;

      // Calculate position accounting for canvas pan offset, centered on cursor
      const x = (e.clientX - rect.left - canvasX) / zoom - nodeWidth / 2;
      const y = (e.clientY - rect.top - canvasY) / zoom - nodeHeight / 2;

      console.log('Position calculation:', JSON.stringify({
        clientX: e.clientX,
        clientY: e.clientY,
        rectLeft: rect.left,
        rectTop: rect.top,
        canvasX,
        canvasY,
        zoom,
        x,
        y
      }));

      // Check if dropping into an expanded VoiceAllocator
      let parentNodeId = null;
      for (const expandedNodeId of expandedNodes) {
        const contentsArea = document.getElementById(`voice-allocator-contents-${expandedNodeId}`);
        if (contentsArea) {
          const contentsRect = contentsArea.getBoundingClientRect();
          if (e.clientX >= contentsRect.left && e.clientX <= contentsRect.right &&
              e.clientY >= contentsRect.top && e.clientY <= contentsRect.bottom) {
            parentNodeId = expandedNodeId;
            console.log(`Dropping into VoiceAllocator ${expandedNodeId} at position (${x}, ${y})`);
            break;
          }
        }
      }

      // Add the node
      console.log(`Adding node ${nodeType} at (${x}, ${y}) with parent ${parentNodeId}`);
      const newNodeId = addNode(nodeType, x, y, parentNodeId);

      // Check if we should insert into a connection
      if (pendingInsertionFromDrag && newNodeId) {
        console.log('Pending insertion detected, will insert node into connection');
        // Defer insertion until after node is fully created
        setTimeout(() => {
          performConnectionInsertion(newNodeId, pendingInsertionFromDrag);
          pendingInsertionFromDrag = null;
        }, 100);
      }

      // Clear the draggedNodeType and highlights
      draggedNodeType = null;
      clearConnectionHighlights();
    });

    // Connection event handlers
    editor.on("connectionCreated", (connection) => {
      handleConnectionCreated(connection);
    });

    editor.on("connectionRemoved", (connection) => {
      handleConnectionRemoved(connection);
    });

    // Node events
    editor.on("nodeCreated", (nodeId) => {
      setupNodeParameters(nodeId);

      // Add double-click handler for VoiceAllocator expansion
      setTimeout(() => {
        const nodeElement = document.getElementById(`node-${nodeId}`);
        if (nodeElement) {
          nodeElement.addEventListener('dblclick', (e) => {
            // Prevent double-click from bubbling to canvas
            e.stopPropagation();
            handleNodeDoubleClick(nodeId);
          });
        }
      }, 50);
    });

    // Track which node is being dragged
    let draggingNodeId = null;

    // Track node drag start for undo/redo and connection insertion
    drawflowDiv.addEventListener('mousedown', (e) => {
      const nodeElement = e.target.closest('.drawflow-node');
      if (nodeElement && !e.target.closest('.input') && !e.target.closest('.output')) {
        const nodeId = parseInt(nodeElement.id.replace('node-', ''));
        const node = editor.getNodeFromId(nodeId);
        if (node) {
          nodeMoveTracker.set(nodeId, { x: node.pos_x, y: node.pos_y });
          draggingNodeId = nodeId;
        }
      }
    });

    // Check for connection insertion while dragging existing nodes
    drawflowDiv.addEventListener('mousemove', (e) => {
      if (draggingNodeId !== null) {
        checkConnectionInsertion(draggingNodeId);
      }
    });

    // Node moved - resize parent VoiceAllocator and check for connection insertion
    editor.on("nodeMoved", (nodeId) => {
      const node = editor.getNodeFromId(nodeId);
      if (node && node.data.parentNodeId) {
        resizeVoiceAllocatorToFit(node.data.parentNodeId);
      }

      // Check if node should be inserted into a connection
      checkConnectionInsertion(nodeId);
    });

    // Track node drag end for undo/redo and handle connection insertion
    drawflowDiv.addEventListener('mouseup', (e) => {
      // Check all tracked nodes for position changes and pending insertions
      for (const [nodeId, oldPos] of nodeMoveTracker.entries()) {
        const node = editor.getNodeFromId(nodeId);
        const hasPendingInsertion = pendingNodeInsertions.has(nodeId);

        if (node) {
          // Check for pending insertion first
          if (hasPendingInsertion) {
            const insertionMatch = pendingNodeInsertions.get(nodeId);
            performConnectionInsertion(nodeId, insertionMatch);
            pendingNodeInsertions.delete(nodeId);
          } else if (node.pos_x !== oldPos.x || node.pos_y !== oldPos.y) {
            // Position changed - record action
            redoStack.length = 0;
            undoStack.push({
              name: "graphMoveNode",
              action: {
                nodeId: nodeId,
                oldPosition: oldPos,
                newPosition: { x: node.pos_x, y: node.pos_y }
              }
            });
            updateMenu();
          }
        }
      }
      // Clear tracker, dragging state, and highlights
      nodeMoveTracker.clear();
      draggingNodeId = null;
      clearConnectionHighlights();
    });

    // Node removed - prevent deletion of template nodes
    editor.on("nodeRemoved", (nodeId) => {
      const nodeElement = document.getElementById(`node-${nodeId}`);
      if (nodeElement && nodeElement.getAttribute('data-template-node') === 'true') {
        console.warn('Cannot delete template nodes');
        // TODO: Re-add the node if it was deleted
        return;
      }

      // Get cached node data before removal
      const cachedData = nodeDataCache.get(nodeId);

      if (cachedData && cachedData.backendId) {
        // Call backend to remove the node
        invoke('graph_remove_node', {
          trackId: cachedData.trackId,
          nodeId: cachedData.backendId
        }).catch(err => {
          console.error("Failed to remove node from backend:", err);
        });

        // Record action for undo (don't call execute since node is already removed from frontend)
        redoStack.length = 0;
        undoStack.push({
          name: "graphRemoveNode",
          action: {
            trackId: cachedData.trackId,
            nodeId: nodeId,
            backendId: cachedData.backendId,
            nodeData: cachedData
          }
        });
        updateMenu();
      }

      // Stop oscilloscope visualization if this was an Oscilloscope node
      stopOscilloscopeVisualization(nodeId);

      // Clean up parent-child tracking
      const parentId = nodeParents.get(nodeId);
      nodeParents.delete(nodeId);

      // Clean up node data cache
      nodeDataCache.delete(nodeId);

      // Resize parent if needed
      if (parentId) {
        resizeVoiceAllocatorToFit(parentId);
      }
    });

  }, 100);

  // Add a node to the graph
  function addNode(nodeType, x, y, parentNodeId = null) {
    if (!editor) return;

    const nodeDef = nodeTypes[nodeType];
    if (!nodeDef) return;

    const nodeId = nodeIdCounter++;
    const html = nodeDef.getHTML(nodeId);

    // Count inputs and outputs by type
    const inputsCount = nodeDef.inputs.length;
    const outputsCount = nodeDef.outputs.length;

    // Add node to Drawflow
    const drawflowNodeId = editor.addNode(
      nodeType,
      inputsCount,
      outputsCount,
      x,
      y,
      `node-${nodeType.toLowerCase()}`,
      { nodeType, backendId: null, parentNodeId: parentNodeId },
      html
    );

    // Update all IDs in the HTML to use drawflowNodeId instead of nodeId
    // This ensures parameter setup can find the correct elements
    if (nodeId !== drawflowNodeId) {
      setTimeout(() => {
        const nodeElement = document.getElementById(`node-${drawflowNodeId}`);
        if (nodeElement) {
          // Update all elements with IDs containing the old nodeId
          const elementsWithIds = nodeElement.querySelectorAll('[id*="-' + nodeId + '"]');
          elementsWithIds.forEach(el => {
            const oldId = el.id;
            const newId = oldId.replace('-' + nodeId, '-' + drawflowNodeId);
            el.id = newId;
            console.log(`Updated element ID: ${oldId} -> ${newId}`);
          });
        }
      }, 10);
    }

    // Track parent-child relationship
    if (parentNodeId !== null) {
      nodeParents.set(drawflowNodeId, parentNodeId);
      console.log(`Node ${drawflowNodeId} (${nodeType}) is child of VoiceAllocator ${parentNodeId}`);

      // Mark template nodes as non-deletable
      const isTemplateNode = (nodeType === 'TemplateInput' || nodeType === 'TemplateOutput');

      // Add CSS class to mark as child node
      setTimeout(() => {
        const nodeElement = document.getElementById(`node-${drawflowNodeId}`);
        if (nodeElement) {
          nodeElement.classList.add('child-node');
          nodeElement.setAttribute('data-parent-node', parentNodeId);

          if (isTemplateNode) {
            nodeElement.classList.add('template-node');
            nodeElement.setAttribute('data-template-node', 'true');
          }

          // Only show if parent is currently expanded
          if (!expandedNodes.has(parentNodeId)) {
            nodeElement.style.display = 'none';
          }
        }

        // Auto-resize parent VoiceAllocator after adding child node
        resizeVoiceAllocatorToFit(parentNodeId);
      }, 10);
    }

    // Apply port styling based on signal types
    setTimeout(() => {
      styleNodePorts(drawflowNodeId, nodeDef);
    }, 10);

    // Send command to backend
    // Check editing context first (dedicated template view), then parent node (inline editing)
    const trackInfo = getCurrentTrack();
    if (trackInfo === null) {
      console.error('No track selected');
      alert('Please select a track first');
      editor.removeNodeId(`node-${drawflowNodeId}`);
      return;
    }
    const trackId = trackInfo.trackId;

    // Determine if we're adding to a template or main graph
    let commandName, commandArgs;
    if (editingContext) {
      // Adding to template in dedicated view
      commandName = "graph_add_node_to_template";
      commandArgs = {
        trackId: trackId,
        voiceAllocatorId: editingContext.voiceAllocatorId,
        nodeType: nodeType,
        x: x,
        y: y
      };
    } else if (parentNodeId) {
      // Adding to template inline (old approach, still supported for backwards compat)
      commandName = "graph_add_node_to_template";
      commandArgs = {
        trackId: trackId,
        voiceAllocatorId: editor.getNodeFromId(parentNodeId).data.backendId,
        nodeType: nodeType,
        x: x,
        y: y
      };
    } else {
      // Adding to main graph
      commandName = "graph_add_node";
      commandArgs = {
        trackId: trackId,
        nodeType: nodeType,
        x: x,
        y: y
      };
    }

    console.log(`[DEBUG] Invoking ${commandName} with args:`, commandArgs);

    // Create a promise that resolves when the GraphNodeAdded event arrives
    const eventPromise = new Promise((resolve) => {
      window.pendingNodeUpdate = {
        drawflowNodeId,
        nodeType,
        resolve: (backendNodeId) => {
          console.log(`[DEBUG] Event promise resolved with backend ID: ${backendNodeId}`);
          resolve(backendNodeId);
        }
      };
    });

    // Wait for both the invoke response and the event
    Promise.all([
      invoke(commandName, commandArgs),
      eventPromise
    ]).then(([invokeReturnedId, eventBackendId]) => {
      console.log(`[DEBUG] Both returned - invoke: ${invokeReturnedId}, event: ${eventBackendId}`);

      // Use the event's backend ID as it's the authoritative source
      const backendNodeId = eventBackendId;
      console.log(`Node ${nodeType} added with backend ID: ${backendNodeId} (parent: ${parentNodeId})`);

      // Store backend node ID using Drawflow's update method
      editor.updateNodeDataFromId(drawflowNodeId, { nodeType, backendId: backendNodeId, parentNodeId: parentNodeId });

      console.log("Verifying stored backend ID:", editor.getNodeFromId(drawflowNodeId).data.backendId);

      // Cache node data for undo/redo
      const trackInfo = getCurrentTrack();
      nodeDataCache.set(drawflowNodeId, {
        nodeType: nodeType,
        backendId: backendNodeId,
        position: { x, y },
        parentNodeId: parentNodeId,
        trackId: trackInfo ? trackInfo.trackId : null
      });

      // Record action for undo (node is already added to frontend and backend)
      redoStack.length = 0;
      undoStack.push({
        name: "graphAddNode",
        action: {
          trackId: getCurrentMidiTrack(),
          nodeType: nodeType,
          position: { x, y },
          nodeId: drawflowNodeId,
          backendId: backendNodeId
        }
      });
      updateMenu();

      // If this is an AudioOutput node, automatically set it as the graph output
      if (nodeType === "AudioOutput") {
        console.log(`Setting node ${backendNodeId} as graph output`);
        const trackInfo = getCurrentTrack();
        if (trackInfo !== null) {
          invoke("graph_set_output_node", {
            trackId: trackInfo.trackId,
            nodeId: backendNodeId
          }).then(() => {
            console.log("Output node set successfully");
          }).catch(err => {
            console.error("Failed to set output node:", err);
          });
        }
      }

      // If this is an AutomationInput node, create timeline curve
      if (nodeType === "AutomationInput" && !parentNodeId) {
        const trackInfo = getCurrentTrack();
        if (trackInfo !== null) {
          const currentTrackId = trackInfo.trackId;
          // Find the audio/MIDI track
          const track = root.audioTracks?.find(t => t.audioTrackId === currentTrackId);
          if (track) {
            // Create curve parameter name: "automation.{nodeId}"
            const curveName = `automation.${backendNodeId}`;

            // Check if curve already exists
            if (!track.animationData.curves[curveName]) {
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
            }
          }
        }
      }

      // If this is an Oscilloscope node, start the visualization
      if (nodeType === "Oscilloscope") {
        const trackInfo = getCurrentTrack();
        if (trackInfo !== null) {
          const currentTrackId = trackInfo.trackId;
          console.log(`Starting oscilloscope visualization for node ${drawflowNodeId} (backend ID: ${backendNodeId})`);
          // Wait for DOM to update before starting visualization
          setTimeout(() => {
            startOscilloscopeVisualization(drawflowNodeId, currentTrackId, backendNodeId, editor);
          }, 100);
        }
      }

      // If this is a VoiceAllocator, automatically create template I/O nodes inside it
      if (nodeType === "VoiceAllocator") {
        setTimeout(() => {
          // Get the node position
          const node = editor.getNodeFromId(drawflowNodeId);
          if (node) {
            // Create TemplateInput on the left
            addNode("TemplateInput", node.pos_x + 50, node.pos_y + 100, drawflowNodeId);
            // Create TemplateOutput on the right
            addNode("TemplateOutput", node.pos_x + 450, node.pos_y + 100, drawflowNodeId);
          }
        }, 100);
      }
    }).catch(err => {
      console.error("Failed to add node to backend:", err);
      showError("Failed to add node: " + err);
    });

    return drawflowNodeId;
  }

  // Auto-resize VoiceAllocator to fit its child nodes
  function resizeVoiceAllocatorToFit(voiceAllocatorNodeId) {
    if (!voiceAllocatorNodeId) return;

    const parentNode = editor.getNodeFromId(voiceAllocatorNodeId);
    const parentElement = document.getElementById(`node-${voiceAllocatorNodeId}`);
    if (!parentNode || !parentElement) return;

    // Find all child nodes
    const childNodeIds = [];
    for (const [childId, parentId] of nodeParents.entries()) {
      if (parentId === voiceAllocatorNodeId) {
        childNodeIds.push(childId);
      }
    }

    if (childNodeIds.length === 0) return;

    // Calculate bounding box of all child nodes
    let minX = Infinity, minY = Infinity;
    let maxX = -Infinity, maxY = -Infinity;

    for (const childId of childNodeIds) {
      const childNode = editor.getNodeFromId(childId);
      const childElement = document.getElementById(`node-${childId}`);
      if (!childNode || !childElement) continue;

      const childWidth = childElement.offsetWidth || 200;
      const childHeight = childElement.offsetHeight || 150;

      minX = Math.min(minX, childNode.pos_x);
      minY = Math.min(minY, childNode.pos_y);
      maxX = Math.max(maxX, childNode.pos_x + childWidth);
      maxY = Math.max(maxY, childNode.pos_y + childHeight);
    }

    // Add generous margin
    const margin = 60;
    const headerHeight = 120; // Space for VoiceAllocator header

    const requiredWidth = (maxX - minX) + (margin * 2);
    const requiredHeight = (maxY - minY) + (margin * 2) + headerHeight;

    // Set minimum size
    const finalWidth = Math.max(requiredWidth, 600);
    const finalHeight = Math.max(requiredHeight, 400);

    // Only resize if expanded
    if (expandedNodes.has(voiceAllocatorNodeId)) {
      parentElement.style.width = `${finalWidth}px`;
      parentElement.style.height = `${finalHeight}px`;
      parentElement.style.minWidth = `${finalWidth}px`;
      parentElement.style.minHeight = `${finalHeight}px`;

      console.log(`Resized VoiceAllocator ${voiceAllocatorNodeId} to ${finalWidth}x${finalHeight}`);
    }
  }

  // Style node ports based on signal types
  function styleNodePorts(nodeId, nodeDef) {
    const nodeElement = document.getElementById(`node-${nodeId}`);
    if (!nodeElement) return;

    // Style input ports
    const inputs = nodeElement.querySelectorAll(".input");
    inputs.forEach((input, index) => {
      if (index < nodeDef.inputs.length) {
        const portDef = nodeDef.inputs[index];
        // Add connector styling class directly to the input element
        input.classList.add(getPortClass(portDef.type));
        // Add label
        const label = document.createElement("span");
        label.textContent = portDef.name;
        input.insertBefore(label, input.firstChild);
      }
    });

    // Style output ports
    const outputs = nodeElement.querySelectorAll(".output");
    outputs.forEach((output, index) => {
      if (index < nodeDef.outputs.length) {
        const portDef = nodeDef.outputs[index];
        // Add connector styling class directly to the output element
        output.classList.add(getPortClass(portDef.type));
        // Add label
        const label = document.createElement("span");
        label.textContent = portDef.name;
        output.appendChild(label);
      }
    });
  }

  // Setup parameter event listeners for a node
  function setupNodeParameters(nodeId) {
    setTimeout(() => {
      const nodeElement = document.getElementById(`node-${nodeId}`);
      if (!nodeElement) return;

      const sliders = nodeElement.querySelectorAll('input[type="range"]');
      sliders.forEach(slider => {
        // Track parameter change action for undo/redo
        let paramAction = null;

        // Prevent node dragging when interacting with slider
        slider.addEventListener("mousedown", (e) => {
          e.stopPropagation();

          // Initialize undo action
          const paramId = parseInt(e.target.getAttribute("data-param"));
          const currentValue = parseFloat(e.target.value);
          const nodeData = editor.getNodeFromId(nodeId);

          if (nodeData && nodeData.data.backendId !== null) {
            const currentTrackId = getCurrentMidiTrack();
            if (currentTrackId !== null) {
              paramAction = actions.graphSetParameter.initialize(
                currentTrackId,
                nodeData.data.backendId,
                paramId,
                nodeId,
                currentValue
              );
            }
          }
        });
        slider.addEventListener("pointerdown", (e) => {
          e.stopPropagation();
        });

        slider.addEventListener("input", (e) => {
          const paramId = parseInt(e.target.getAttribute("data-param"));
          const value = parseFloat(e.target.value);

          console.log(`[setupNodeParameters] Slider input - nodeId: ${nodeId}, paramId: ${paramId}, value: ${value}`);

          // Update display
          const nodeData = editor.getNodeFromId(nodeId);
          if (nodeData) {
            const nodeDef = nodeTypes[nodeData.name];
            console.log(`[setupNodeParameters] Found node type: ${nodeData.name}, parameters:`, nodeDef?.parameters);
            if (nodeDef && nodeDef.parameters[paramId]) {
              const param = nodeDef.parameters[paramId];
              console.log(`[setupNodeParameters] Looking for span: #${param.name}-${nodeId}`);
              const displaySpan = nodeElement.querySelector(`#${param.name}-${nodeId}`);
              console.log(`[setupNodeParameters] Found span:`, displaySpan);
              if (displaySpan) {
                // Special formatting for oscilloscope trigger mode
                if (param.name === 'trigger_mode') {
                  const modes = ['Free', 'Rising', 'Falling', 'V/oct'];
                  displaySpan.textContent = modes[Math.round(value)] || 'Free';
                }
                // Special formatting for Phaser rate in sync mode
                else if (param.name === 'rate' && nodeData.name === 'Phaser') {
                  const syncCheckbox = nodeElement.querySelector(`#sync-${nodeId}`);
                  if (syncCheckbox && syncCheckbox.checked) {
                    const beatDivisions = [
                      '4 bars', '2 bars', '1 bar', '1/2', '1/4', '1/8', '1/16', '1/32', '1/2T', '1/4T', '1/8T'
                    ];
                    const idx = Math.round(value);
                    displaySpan.textContent = beatDivisions[Math.min(10, Math.max(0, idx))];
                  } else {
                    displaySpan.textContent = value.toFixed(param.unit === 'Hz' ? 0 : 2);
                  }
                }
                else {
                  displaySpan.textContent = value.toFixed(param.unit === 'Hz' ? 0 : 2);
                }
              }

              // Update oscilloscope time scale if this is a time_scale parameter
              if (param.name === 'time_scale' && oscilloscopeTimeScales) {
                oscilloscopeTimeScales.set(nodeId, value);
                console.log(`Updated oscilloscope time scale for node ${nodeId}: ${value}ms`);
              }
            }

            // Send to backend in real-time
            if (nodeData.data.backendId !== null) {
              const trackInfo = getCurrentTrack();
              if (trackInfo !== null) {
                // Convert beat divisions to Hz for Phaser rate in sync mode
                let backendValue = value;
                if (nodeDef && nodeDef.parameters[paramId]) {
                  const param = nodeDef.parameters[paramId];
                  if (param.name === 'rate' && nodeData.name === 'Phaser') {
                    const syncCheckbox = nodeElement.querySelector(`#sync-${nodeId}`);
                    if (syncCheckbox && syncCheckbox.checked && context.timelineWidget) {
                      const beatDivisions = [
                        { label: '4 bars', multiplier: 16.0 },
                        { label: '2 bars', multiplier: 8.0 },
                        { label: '1 bar', multiplier: 4.0 },
                        { label: '1/2', multiplier: 2.0 },
                        { label: '1/4', multiplier: 1.0 },
                        { label: '1/8', multiplier: 0.5 },
                        { label: '1/16', multiplier: 0.25 },
                        { label: '1/32', multiplier: 0.125 },
                        { label: '1/2T', multiplier: 2.0/3.0 },
                        { label: '1/4T', multiplier: 1.0/3.0 },
                        { label: '1/8T', multiplier: 0.5/3.0 }
                      ];
                      const idx = Math.min(10, Math.max(0, Math.round(value)));
                      const bpm = context.timelineWidget.timelineState.bpm;
                      const beatsPerSecond = bpm / 60.0;
                      const quarterNotesPerCycle = beatDivisions[idx].multiplier;
                      // Hz = how many cycles per second
                      backendValue = beatsPerSecond / quarterNotesPerCycle;
                    }
                  }
                }

                invoke("graph_set_parameter", {
                  trackId: trackInfo.trackId,
                  nodeId: nodeData.data.backendId,
                  paramId: paramId,
                  value: backendValue
                }).catch(err => {
                  console.error("Failed to set parameter:", err);
                });
              }
            }
          }
        });

        // Finalize parameter change for undo/redo when slider is released
        slider.addEventListener("change", (e) => {
          const newValue = parseFloat(e.target.value);

          if (paramAction) {
            actions.graphSetParameter.finalize(paramAction, newValue);
            paramAction = null;
          }
        });
      });

      // Handle select dropdowns
      const selects = nodeElement.querySelectorAll('select[data-param]');
      selects.forEach(select => {
        // Track parameter change action for undo/redo
        let paramAction = null;

        // Prevent node dragging when interacting with select
        select.addEventListener("mousedown", (e) => {
          e.stopPropagation();

          // Initialize undo action
          const paramId = parseInt(e.target.getAttribute("data-param"));
          const currentValue = parseFloat(e.target.value);
          const nodeData = editor.getNodeFromId(nodeId);

          if (nodeData && nodeData.data.backendId !== null) {
            const currentTrackId = getCurrentMidiTrack();
            if (currentTrackId !== null) {
              paramAction = actions.graphSetParameter.initialize(
                currentTrackId,
                nodeData.data.backendId,
                paramId,
                nodeId,
                currentValue
              );
            }
          }
        });
        select.addEventListener("pointerdown", (e) => {
          e.stopPropagation();
        });

        select.addEventListener("change", (e) => {
          const paramId = parseInt(e.target.getAttribute("data-param"));
          const value = parseFloat(e.target.value);

          console.log(`[setupNodeParameters] Select change - nodeId: ${nodeId}, paramId: ${paramId}, value: ${value}`);

          // Update display span if it exists
          const nodeData = editor.getNodeFromId(nodeId);
          if (nodeData) {
            const nodeDef = nodeTypes[nodeData.name];
            if (nodeDef && nodeDef.parameters[paramId]) {
              const param = nodeDef.parameters[paramId];
              const displaySpan = nodeElement.querySelector(`#${param.name}-${nodeId}`);
              if (displaySpan) {
                // Update the span with the selected option text
                displaySpan.textContent = e.target.options[e.target.selectedIndex].text;
              }
            }

            // Send to backend
            if (nodeData.data.backendId !== null) {
              const trackInfo = getCurrentTrack();
              if (trackInfo !== null) {
                invoke("graph_set_parameter", {
                  trackId: trackInfo.trackId,
                  nodeId: nodeData.data.backendId,
                  paramId: paramId,
                  value: value
                }).catch(err => {
                  console.error("Failed to set parameter:", err);
                });
              }
            }
          }

          // Finalize undo action
          if (paramAction) {
            actions.graphSetParameter.finalize(paramAction, value);
            paramAction = null;
          }
        });
      });

      // Handle number inputs
      const numberInputs = nodeElement.querySelectorAll('input[type="number"][data-param]');
      numberInputs.forEach(numberInput => {
        // Track parameter change action for undo/redo
        let paramAction = null;

        // Prevent node dragging when interacting with number input
        numberInput.addEventListener("mousedown", (e) => {
          e.stopPropagation();

          // Initialize undo action
          const paramId = parseInt(e.target.getAttribute("data-param"));
          const currentValue = parseFloat(e.target.value);
          const nodeData = editor.getNodeFromId(nodeId);

          if (nodeData && nodeData.data.backendId !== null) {
            const currentTrackId = getCurrentMidiTrack();
            if (currentTrackId !== null) {
              paramAction = actions.graphSetParameter.initialize(
                currentTrackId,
                nodeData.data.backendId,
                paramId,
                nodeId,
                currentValue
              );
            }
          }
        });
        numberInput.addEventListener("pointerdown", (e) => {
          e.stopPropagation();
        });

        numberInput.addEventListener("input", (e) => {
          const paramId = parseInt(e.target.getAttribute("data-param"));
          let value = parseFloat(e.target.value);

          // Validate and clamp to min/max
          const min = parseFloat(e.target.min);
          const max = parseFloat(e.target.max);
          if (!isNaN(value)) {
            value = Math.max(min, Math.min(max, value));
          } else {
            value = 0;
          }

          console.log(`[setupNodeParameters] Number input - nodeId: ${nodeId}, paramId: ${paramId}, value: ${value}`);

          // Update corresponding slider
          const slider = nodeElement.querySelector(`input[type="range"][data-param="${paramId}"]`);
          if (slider) {
            slider.value = value;
          }

          // Send to backend
          const nodeData = editor.getNodeFromId(nodeId);
          if (nodeData && nodeData.data.backendId !== null) {
            const trackInfo = getCurrentTrack();
            if (trackInfo !== null) {
              invoke("graph_set_parameter", {
                trackId: trackInfo.trackId,
                nodeId: nodeData.data.backendId,
                paramId: paramId,
                value: value
              }).catch(err => {
                console.error("Failed to set parameter:", err);
              });
            }
          }
        });

        numberInput.addEventListener("change", (e) => {
          const value = parseFloat(e.target.value);

          // Finalize undo action
          if (paramAction) {
            actions.graphSetParameter.finalize(paramAction, value);
            paramAction = null;
          }
        });
      });

      // Handle checkboxes
      const checkboxes = nodeElement.querySelectorAll('input[type="checkbox"][data-param]');
      checkboxes.forEach(checkbox => {
        checkbox.addEventListener("change", (e) => {
          const paramId = parseInt(e.target.getAttribute("data-param"));
          const value = e.target.checked ? 1.0 : 0.0;

          console.log(`[setupNodeParameters] Checkbox change - nodeId: ${nodeId}, paramId: ${paramId}, value: ${value}`);

          // Send to backend
          const nodeData = editor.getNodeFromId(nodeId);
          if (nodeData && nodeData.data.backendId !== null) {
            const trackInfo = getCurrentTrack();
            if (trackInfo !== null) {
              invoke("graph_set_parameter", {
                trackId: trackInfo.trackId,
                nodeId: nodeData.data.backendId,
                paramId: paramId,
                value: value
              }).then(() => {
                console.log(`Parameter ${paramId} set to ${value}`);
              }).catch(err => {
                console.error("Failed to set parameter:", err);
              });
            }
          }

          // Special handling for Phaser sync checkbox
          if (checkbox.id.startsWith('sync-')) {
            const rateSlider = nodeElement.querySelector(`#rate-slider-${nodeId}`);
            const rateDisplay = nodeElement.querySelector(`#rate-${nodeId}`);
            const rateUnit = nodeElement.querySelector(`#rate-unit-${nodeId}`);

            if (rateSlider && rateDisplay && rateUnit) {
              if (e.target.checked) {
                // Sync mode: Use beat divisions
                // Map slider 0-10 to different note divisions
                // 0: 4 bars, 1: 2 bars, 2: 1 bar, 3: 1/2, 4: 1/4, 5: 1/8, 6: 1/16, 7: 1/32, 8: 1/2T, 9: 1/4T, 10: 1/8T
                const beatDivisions = [
                  { label: '4 bars', multiplier: 16.0 },
                  { label: '2 bars', multiplier: 8.0 },
                  { label: '1 bar', multiplier: 4.0 },
                  { label: '1/2', multiplier: 2.0 },
                  { label: '1/4', multiplier: 1.0 },
                  { label: '1/8', multiplier: 0.5 },
                  { label: '1/16', multiplier: 0.25 },
                  { label: '1/32', multiplier: 0.125 },
                  { label: '1/2T', multiplier: 2.0/3.0 },
                  { label: '1/4T', multiplier: 1.0/3.0 },
                  { label: '1/8T', multiplier: 0.5/3.0 }
                ];

                rateSlider.min = '0';
                rateSlider.max = '10';
                rateSlider.step = '1';
                const idx = Math.round(parseFloat(rateSlider.value) * 10 / 10);
                rateSlider.value = Math.min(10, Math.max(0, idx));
                rateDisplay.textContent = beatDivisions[parseInt(rateSlider.value)].label;
                rateUnit.textContent = '';
              } else {
                // Free mode: Hz
                rateSlider.min = '0.1';
                rateSlider.max = '10.0';
                rateSlider.step = '0.1';
                rateDisplay.textContent = parseFloat(rateSlider.value).toFixed(1);
                rateUnit.textContent = ' Hz';
              }
            }
          }
        });
      });

      // Handle Load Sample button for SimpleSampler
      const loadSampleBtn = nodeElement.querySelector(".load-sample-btn");
      if (loadSampleBtn) {
        loadSampleBtn.addEventListener("mousedown", (e) => e.stopPropagation());
        loadSampleBtn.addEventListener("pointerdown", (e) => e.stopPropagation());
        loadSampleBtn.addEventListener("click", async (e) => {
          e.stopPropagation();

          const nodeData = editor.getNodeFromId(nodeId);
          if (!nodeData || nodeData.data.backendId === null) {
            showError("Node not yet created on backend");
            return;
          }

          const currentTrackId = getCurrentMidiTrack();
          if (currentTrackId === null) {
            showError("No MIDI track selected");
            return;
          }

          try {
            const filePath = await openFileDialog({
              title: "Load Audio Sample",
              filters: [{
                name: "Audio Files",
                extensions: audioExtensions
              }]
            });

            if (filePath) {
              await invoke("sampler_load_sample", {
                trackId: currentTrackId,
                nodeId: nodeData.data.backendId,
                filePath: filePath
              });

              // Update UI to show filename
              const sampleInfo = nodeElement.querySelector(`#sample-info-${nodeId}`);
              if (sampleInfo) {
                const filename = filePath.split('/').pop().split('\\').pop();
                sampleInfo.textContent = filename;
              }
            }
          } catch (err) {
            console.error("Failed to load sample:", err);
            showError(`Failed to load sample: ${err}`);
          }
        });
      }

      // Handle Add Layer button for MultiSampler
      const addLayerBtn = nodeElement.querySelector(".add-layer-btn");
      if (addLayerBtn) {
        addLayerBtn.addEventListener("mousedown", (e) => e.stopPropagation());
        addLayerBtn.addEventListener("pointerdown", (e) => e.stopPropagation());
        addLayerBtn.addEventListener("click", async (e) => {
          e.stopPropagation();

          const nodeData = editor.getNodeFromId(nodeId);
          if (!nodeData || nodeData.data.backendId === null) {
            showError("Node not yet created on backend");
            return;
          }

          const currentTrackId = getCurrentMidiTrack();
          if (currentTrackId === null) {
            showError("No MIDI track selected");
            return;
          }

          try {
            const filePath = await openFileDialog({
              title: "Add Sample Layer",
              filters: [{
                name: "Audio Files",
                extensions: audioExtensions
              }]
            });

            if (filePath) {
              // Show dialog to configure layer mapping
              const layerConfig = await showLayerConfigDialog(filePath);

              if (layerConfig) {
                await invoke("multi_sampler_add_layer", {
                  trackId: currentTrackId,
                  nodeId: nodeData.data.backendId,
                  filePath: filePath,
                  keyMin: layerConfig.keyMin,
                  keyMax: layerConfig.keyMax,
                  rootKey: layerConfig.rootKey,
                  velocityMin: layerConfig.velocityMin,
                  velocityMax: layerConfig.velocityMax,
                  loopStart: layerConfig.loopStart,
                  loopEnd: layerConfig.loopEnd,
                  loopMode: layerConfig.loopMode
                });

                // Wait a bit for the audio thread to process the add command
                await new Promise(resolve => setTimeout(resolve, 100));

                // Refresh the layers list
                await refreshSampleLayersList(nodeId);
              }
            }
          } catch (err) {
            console.error("Failed to add layer:", err);
            showError(`Failed to add layer: ${err}`);
          }
        });
      }
    }, 100);
  }

  // Handle double-click on nodes (for VoiceAllocator template editing)
  function handleNodeDoubleClick(nodeId) {
    const node = editor.getNodeFromId(nodeId);
    if (!node) return;

    // Only VoiceAllocator nodes can be opened for template editing
    if (node.data.nodeType !== 'VoiceAllocator') return;

    // Don't allow entering templates when already editing a template
    if (editingContext) {
      showError("Cannot nest template editing - exit current template first");
      return;
    }

    // Get the backend ID and node name
    if (node.data.backendId === null) {
      showError("VoiceAllocator not yet created on backend");
      return;
    }

    // Enter template editing mode
    const nodeName = node.name || 'VoiceAllocator';
    enterTemplate(node.data.backendId, nodeName);
  }

  // Refresh the layers list for a MultiSampler node
  async function refreshSampleLayersList(nodeId) {
    const nodeData = editor.getNodeFromId(nodeId);
    if (!nodeData || nodeData.data.backendId === null) {
      return;
    }

    const currentTrackId = getCurrentMidiTrack();
    if (currentTrackId === null) {
      return;
    }

    try {
      const layers = await invoke("multi_sampler_get_layers", {
        trackId: currentTrackId,
        nodeId: nodeData.data.backendId
      });

      const layersList = document.querySelector(`#sample-layers-list-${nodeId}`);
      const layersContainer = document.querySelector(`#sample-layers-container-${nodeId}`);

      if (!layersList) return;

      // Prevent scroll events from bubbling to canvas
      if (layersContainer && !layersContainer.dataset.scrollListenerAdded) {
        layersContainer.addEventListener('wheel', (e) => {
          e.stopPropagation();
        }, { passive: false });
        layersContainer.dataset.scrollListenerAdded = 'true';
      }

      if (layers.length === 0) {
        layersList.innerHTML = '<tr><td colspan="5" class="sample-layers-empty">No layers loaded</td></tr>';
      } else {
        layersList.innerHTML = layers.map((layer, index) => {
          const filename = layer.file_path.split('/').pop().split('\\').pop();
          const keyRange = `${midiToNoteName(layer.key_min)}-${midiToNoteName(layer.key_max)}`;
          const rootNote = midiToNoteName(layer.root_key);
          const velRange = `${layer.velocity_min}-${layer.velocity_max}`;

          return `
            <tr data-index="${index}">
              <td class="sample-layer-filename" title="${filename}">${filename}</td>
              <td>${keyRange}</td>
              <td>${rootNote}</td>
              <td>${velRange}</td>
              <td>
                <div class="sample-layer-actions">
                  <button class="btn-edit-layer" data-node="${nodeId}" data-index="${index}">Edit</button>
                  <button class="btn-delete-layer" data-node="${nodeId}" data-index="${index}">Del</button>
                </div>
              </td>
            </tr>
          `;
        }).join('');

        // Add event listeners for edit buttons
        const editButtons = layersList.querySelectorAll('.btn-edit-layer');
        editButtons.forEach(btn => {
          btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const index = parseInt(btn.dataset.index);
            const layer = layers[index];

            // Show edit dialog with current values
            const layerConfig = await showLayerConfigDialog(layer.file_path, {
              keyMin: layer.key_min,
              keyMax: layer.key_max,
              rootKey: layer.root_key,
              velocityMin: layer.velocity_min,
              velocityMax: layer.velocity_max,
              loopStart: layer.loop_start,
              loopEnd: layer.loop_end,
              loopMode: layer.loop_mode
            });

            if (layerConfig) {
              try {
                await invoke("multi_sampler_update_layer", {
                  trackId: currentTrackId,
                  nodeId: nodeData.data.backendId,
                  layerIndex: index,
                  keyMin: layerConfig.keyMin,
                  keyMax: layerConfig.keyMax,
                  rootKey: layerConfig.rootKey,
                  velocityMin: layerConfig.velocityMin,
                  velocityMax: layerConfig.velocityMax,
                  loopStart: layerConfig.loopStart,
                  loopEnd: layerConfig.loopEnd,
                  loopMode: layerConfig.loopMode
                });

                // Refresh the list
                await refreshSampleLayersList(nodeId);
              } catch (err) {
                console.error("Failed to update layer:", err);
                showError(`Failed to update layer: ${err}`);
              }
            }
          });
        });

        // Add event listeners for delete buttons
        const deleteButtons = layersList.querySelectorAll('.btn-delete-layer');
        deleteButtons.forEach(btn => {
          btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const index = parseInt(btn.dataset.index);
            const layer = layers[index];
            const filename = layer.file_path.split('/').pop().split('\\').pop();

            if (confirm(`Delete layer "${filename}"?`)) {
              try {
                await invoke("multi_sampler_remove_layer", {
                  trackId: currentTrackId,
                  nodeId: nodeData.data.backendId,
                  layerIndex: index
                });

                // Refresh the list
                await refreshSampleLayersList(nodeId);
              } catch (err) {
                console.error("Failed to remove layer:", err);
                showError(`Failed to remove layer: ${err}`);
              }
            }
          });
        });
      }
    } catch (err) {
      console.error("Failed to get layers:", err);
    }
  }

  // Push nodes away from a point using gaussian falloff
  function pushNodesAway(centerX, centerY, maxDistance, excludeNodeId) {
    const module = editor.module;
    const allNodes = editor.drawflow.drawflow[module]?.data || {};

    // Gaussian parameters
    const sigma = maxDistance / 3; // Standard deviation for falloff
    const maxPush = 150; // Maximum push distance at the center

    for (const [id, node] of Object.entries(allNodes)) {
      const nodeIdNum = parseInt(id);
      if (nodeIdNum === excludeNodeId) continue;

      // Calculate distance from center
      const dx = node.pos_x - centerX;
      const dy = node.pos_y - centerY;
      const distance = Math.sqrt(dx * dx + dy * dy);

      if (distance < maxDistance && distance > 0) {
        // Calculate push strength using gaussian falloff
        const falloff = Math.exp(-(distance * distance) / (2 * sigma * sigma));
        const pushStrength = maxPush * falloff;

        // Calculate push direction (normalized)
        const dirX = dx / distance;
        const dirY = dy / distance;

        // Calculate new position
        const newX = node.pos_x + dirX * pushStrength;
        const newY = node.pos_y + dirY * pushStrength;

        // Update position in the data structure
        node.pos_x = newX;
        node.pos_y = newY;

        // Update the DOM element position
        const nodeElement = document.getElementById(`node-${nodeIdNum}`);
        if (nodeElement) {
          nodeElement.style.left = newX + 'px';
          nodeElement.style.top = newY + 'px';
        }

        // Trigger connection redraw
        editor.updateConnectionNodes(`node-${nodeIdNum}`);
      }
    }
  }

  // Perform the actual connection insertion
  function performConnectionInsertion(nodeId, match) {

    const node = editor.getNodeFromId(nodeId);
    const sourceNode = editor.getNodeFromId(match.sourceNodeId);
    const targetNode = editor.getNodeFromId(match.targetNodeId);

    if (!node || !sourceNode || !targetNode) {
      console.error("Missing nodes for insertion");
      return;
    }

    // Position the node between source and target
    const sourceElement = document.getElementById(`node-${match.sourceNodeId}`);
    const targetElement = document.getElementById(`node-${match.targetNodeId}`);

    if (sourceElement && targetElement) {
      const sourceRect = sourceElement.getBoundingClientRect();
      const targetRect = targetElement.getBoundingClientRect();

      // Calculate midpoint position
      const newX = (sourceNode.pos_x + sourceRect.width + targetNode.pos_x) / 2 - 80; // Approximate node half-width
      const newY = (sourceNode.pos_y + targetNode.pos_y) / 2 - 50; // Approximate node half-height

      // Update node position in data structure
      node.pos_x = newX;
      node.pos_y = newY;

      // Update the DOM element position
      const nodeElement = document.getElementById(`node-${nodeId}`);
      if (nodeElement) {
        nodeElement.style.left = newX + 'px';
        nodeElement.style.top = newY + 'px';
      }

      // Trigger connection redraw for this node
      editor.updateConnectionNodes(`node-${nodeId}`);

      // Push surrounding nodes away with gaussian falloff
      pushNodesAway(newX, newY, 400, nodeId); // 400px influence radius
    }

    // Remove the old connection
    suppressActionRecording = true;
    editor.removeSingleConnection(
      match.sourceNodeId,
      match.targetNodeId,
      match.sourceOutputClass,
      match.targetInputClass
    );

    // Create new connections: source -> node -> target
    // Connection 1: source output -> node input
    setTimeout(() => {
      editor.addConnection(
        match.sourceNodeId,
        nodeId,
        match.sourceOutputClass,
        `input_${match.nodeInputPort + 1}`
      );

      // Connection 2: node output -> target input
      setTimeout(() => {
        editor.addConnection(
          nodeId,
          match.targetNodeId,
          `output_${match.nodeOutputPort + 1}`,
          match.targetInputClass
        );

        suppressActionRecording = false;
      }, 50);
    }, 50);
  }

  // Check if cursor position during drag is near a connection
  function checkConnectionInsertionDuringDrag(dragEvent, nodeDef) {
    const drawflowDiv = container.querySelector("#drawflow");
    if (!drawflowDiv || !editor) return;

    const rect = drawflowDiv.getBoundingClientRect();
    const canvasX = editor.canvas_x || 0;
    const canvasY = editor.canvas_y || 0;
    const zoom = editor.zoom || 1;

    // Calculate cursor position in canvas coordinates
    const cursorX = (dragEvent.clientX - rect.left - canvasX) / zoom;
    const cursorY = (dragEvent.clientY - rect.top - canvasY) / zoom;

    // Get all connections in the current module
    const module = editor.module;
    const allNodes = editor.drawflow.drawflow[module]?.data || {};

    // Distance threshold for insertion (in pixels)
    const insertionThreshold = 30;

    let bestMatch = null;
    let bestDistance = insertionThreshold;

    // Check each connection
    for (const [sourceNodeId, sourceNode] of Object.entries(allNodes)) {
      for (const [outputKey, outputData] of Object.entries(sourceNode.outputs)) {
        for (const connection of outputData.connections) {
          const targetNodeId = connection.node;
          const targetNode = allNodes[targetNodeId];

          if (!targetNode) continue;

          // Get source and target positions
          const sourceElement = document.getElementById(`node-${sourceNodeId}`);
          const targetElement = document.getElementById(`node-${targetNodeId}`);

          if (!sourceElement || !targetElement) continue;

          const sourceRect = sourceElement.getBoundingClientRect();
          const targetRect = targetElement.getBoundingClientRect();

          // Calculate output port position (right side of source node)
          const sourceX = sourceNode.pos_x + sourceRect.width;
          const sourceY = sourceNode.pos_y + sourceRect.height / 2;

          // Calculate input port position (left side of target node)
          const targetX = targetNode.pos_x;
          const targetY = targetNode.pos_y + targetRect.height / 2;

          // Calculate distance from cursor to connection line
          const distance = distanceToLineSegment(
            cursorX, cursorY,
            sourceX, sourceY,
            targetX, targetY
          );

          // Check if this is the closest connection
          if (distance < bestDistance) {
            // Check port compatibility
            const sourcePortIndex = parseInt(outputKey.replace('output_', '')) - 1;
            const targetPortIndex = parseInt(connection.output.replace('input_', '')) - 1;

            const sourceDef = nodeTypes[sourceNode.name];
            const targetDef = nodeTypes[targetNode.name];

            if (!sourceDef || !targetDef) continue;

            // Get the signal type of the connection
            if (sourcePortIndex >= sourceDef.outputs.length ||
                targetPortIndex >= targetDef.inputs.length) continue;

            const connectionType = sourceDef.outputs[sourcePortIndex].type;

            // Check if the dragged node has compatible input and output
            let compatibleInputIndex = -1;
            let compatibleOutputIndex = -1;

            // Find first compatible input and output
            for (let i = 0; i < nodeDef.inputs.length; i++) {
              if (nodeDef.inputs[i].type === connectionType) {
                compatibleInputIndex = i;
                break;
              }
            }

            for (let i = 0; i < nodeDef.outputs.length; i++) {
              if (nodeDef.outputs[i].type === connectionType) {
                compatibleOutputIndex = i;
                break;
              }
            }

            if (compatibleInputIndex !== -1 && compatibleOutputIndex !== -1) {
              bestDistance = distance;
              bestMatch = {
                sourceNodeId: parseInt(sourceNodeId),
                targetNodeId: parseInt(targetNodeId),
                sourcePort: sourcePortIndex,
                targetPort: targetPortIndex,
                nodeInputPort: compatibleInputIndex,
                nodeOutputPort: compatibleOutputIndex,
                connectionType: connectionType,
                sourceOutputClass: outputKey,
                targetInputClass: connection.output,
                insertX: cursorX,
                insertY: cursorY
              };
            }
          }
        }
      }
    }

    // If we found a match, highlight the connection and store it
    if (bestMatch) {
      highlightConnectionForInsertion(bestMatch);
      pendingInsertionFromDrag = bestMatch;
    } else {
      clearConnectionHighlights();
      pendingInsertionFromDrag = null;
    }
  }

  // Check if a node can be inserted into a connection
  function checkConnectionInsertion(nodeId) {
    const node = editor.getNodeFromId(nodeId);
    if (!node) return;

    const nodeDef = nodeTypes[node.name];
    if (!nodeDef) return;

    // Check if node has any connections - skip if it does
    let hasConnections = false;
    for (const [inputKey, inputData] of Object.entries(node.inputs)) {
      if (inputData.connections && inputData.connections.length > 0) {
        hasConnections = true;
        break;
      }
    }
    if (!hasConnections) {
      for (const [outputKey, outputData] of Object.entries(node.outputs)) {
        if (outputData.connections && outputData.connections.length > 0) {
          hasConnections = true;
          break;
        }
      }
    }

    if (hasConnections) {
      clearConnectionHighlights();
      pendingNodeInsertions.delete(nodeId);
      return;
    }

    // Get node center position
    const nodeElement = document.getElementById(`node-${nodeId}`);
    if (!nodeElement) return;

    const nodeRect = nodeElement.getBoundingClientRect();
    const nodeCenterX = node.pos_x + nodeRect.width / 2;
    const nodeCenterY = node.pos_y + nodeRect.height / 2;

    // Get all connections in the current module
    const module = editor.module;
    const allNodes = editor.drawflow.drawflow[module]?.data || {};

    // Distance threshold for insertion (in pixels)
    const insertionThreshold = 30;

    let bestMatch = null;
    let bestDistance = insertionThreshold;

    // Check each connection
    for (const [sourceNodeId, sourceNode] of Object.entries(allNodes)) {
      if (parseInt(sourceNodeId) === nodeId) continue; // Skip the node being dragged

      for (const [outputKey, outputData] of Object.entries(sourceNode.outputs)) {
        for (const connection of outputData.connections) {
          const targetNodeId = connection.node;
          const targetNode = allNodes[targetNodeId];

          if (!targetNode || parseInt(targetNodeId) === nodeId) continue;

          // Get source and target positions
          const sourceElement = document.getElementById(`node-${sourceNodeId}`);
          const targetElement = document.getElementById(`node-${targetNodeId}`);

          if (!sourceElement || !targetElement) continue;

          const sourceRect = sourceElement.getBoundingClientRect();
          const targetRect = targetElement.getBoundingClientRect();

          // Calculate output port position (right side of source node)
          const sourceX = sourceNode.pos_x + sourceRect.width;
          const sourceY = sourceNode.pos_y + sourceRect.height / 2;

          // Calculate input port position (left side of target node)
          const targetX = targetNode.pos_x;
          const targetY = targetNode.pos_y + targetRect.height / 2;

          // Calculate distance from node center to connection line
          const distance = distanceToLineSegment(
            nodeCenterX, nodeCenterY,
            sourceX, sourceY,
            targetX, targetY
          );

          // Check if this is the closest connection
          if (distance < bestDistance) {
            // Check port compatibility
            const sourcePortIndex = parseInt(outputKey.replace('output_', '')) - 1;
            const targetPortIndex = parseInt(connection.output.replace('input_', '')) - 1;

            const sourceDef = nodeTypes[sourceNode.name];
            const targetDef = nodeTypes[targetNode.name];

            if (!sourceDef || !targetDef) continue;

            // Get the signal type of the connection
            if (sourcePortIndex >= sourceDef.outputs.length ||
                targetPortIndex >= targetDef.inputs.length) continue;

            const connectionType = sourceDef.outputs[sourcePortIndex].type;

            // Check if the dragged node has compatible input and output
            let hasCompatibleInput = false;
            let hasCompatibleOutput = false;
            let compatibleInputIndex = -1;
            let compatibleOutputIndex = -1;

            // Find first compatible input and output
            for (let i = 0; i < nodeDef.inputs.length; i++) {
              if (nodeDef.inputs[i].type === connectionType) {
                hasCompatibleInput = true;
                compatibleInputIndex = i;
                break;
              }
            }

            for (let i = 0; i < nodeDef.outputs.length; i++) {
              if (nodeDef.outputs[i].type === connectionType) {
                hasCompatibleOutput = true;
                compatibleOutputIndex = i;
                break;
              }
            }

            if (hasCompatibleInput && hasCompatibleOutput) {
              bestDistance = distance;
              bestMatch = {
                sourceNodeId: parseInt(sourceNodeId),
                targetNodeId: parseInt(targetNodeId),
                sourcePort: sourcePortIndex,
                targetPort: targetPortIndex,
                nodeInputPort: compatibleInputIndex,
                nodeOutputPort: compatibleOutputIndex,
                connectionType: connectionType,
                sourceOutputClass: outputKey,
                targetInputClass: connection.output
              };
            }
          }
        }
      }
    }

    // If we found a match, highlight the connection
    if (bestMatch) {
      highlightConnectionForInsertion(bestMatch);
      // Store the match in the Map for use on mouseup
      pendingNodeInsertions.set(nodeId, bestMatch);
    } else {
      clearConnectionHighlights();
      pendingNodeInsertions.delete(nodeId);
    }
  }

  // Track which connection is highlighted for insertion
  let highlightedConnection = null;
  let highlightInterval = null;
  let pendingInsertionFromDrag = null;

  // Track pending insertions for existing nodes being dragged
  const pendingNodeInsertions = new Map(); // nodeId -> insertion match

  // Apply highlight to the tracked connection
  function applyConnectionHighlight() {
    if (!highlightedConnection) return;

    const connectionElement = document.querySelector(
      `.connection.node_in_node-${highlightedConnection.targetNodeId}.node_out_node-${highlightedConnection.sourceNodeId}`
    );

    if (connectionElement && !connectionElement.classList.contains('connection-insertion-highlight')) {
      connectionElement.classList.add('connection-insertion-highlight');
    }
  }

  // Highlight a connection that can receive the node
  function highlightConnectionForInsertion(match) {
    // Store the connection to highlight
    highlightedConnection = match;

    // Clear any existing interval
    if (highlightInterval) {
      clearInterval(highlightInterval);
    }

    // Apply highlight immediately
    applyConnectionHighlight();

    // Keep re-applying in case Drawflow redraws
    highlightInterval = setInterval(applyConnectionHighlight, 50);
  }

  // Clear connection insertion highlights
  function clearConnectionHighlights() {
    // Stop the interval
    if (highlightInterval) {
      clearInterval(highlightInterval);
      highlightInterval = null;
    }

    highlightedConnection = null;

    // Remove all highlight classes
    document.querySelectorAll('.connection-insertion-highlight').forEach(el => {
      el.classList.remove('connection-insertion-highlight');
    });
  }

  // Handle connection creation
  function handleConnectionCreated(connection) {
    console.log("handleConnectionCreated called:", connection);
    const outputNode = editor.getNodeFromId(connection.output_id);
    const inputNode = editor.getNodeFromId(connection.input_id);

    console.log("Output node:", outputNode, "Input node:", inputNode);
    if (!outputNode || !inputNode) {
      console.log("Missing node - returning");
      return;
    }

    console.log("Output node name:", outputNode.name, "Input node name:", inputNode.name);
    const outputDef = nodeTypes[outputNode.name];
    const inputDef = nodeTypes[inputNode.name];

    console.log("Output def:", outputDef, "Input def:", inputDef);
    if (!outputDef || !inputDef) {
      console.log("Missing node definition - returning");
      return;
    }

    // Extract port indices from connection class names
    // Drawflow uses 1-based indexing, but our arrays are 0-based
    const outputPort = parseInt(connection.output_class.replace("output_", "")) - 1;
    const inputPort = parseInt(connection.input_class.replace("input_", "")) - 1;

    console.log("Port indices (0-based) - output:", outputPort, "input:", inputPort);
    console.log("Output class:", connection.output_class, "Input class:", connection.input_class);

    // Validate signal types
    console.log("Checking port bounds - outputPort:", outputPort, "< outputs.length:", outputDef.outputs.length, "inputPort:", inputPort, "< inputs.length:", inputDef.inputs.length);
    if (outputPort < outputDef.outputs.length && inputPort < inputDef.inputs.length) {
      const outputType = outputDef.outputs[outputPort].type;
      const inputType = inputDef.inputs[inputPort].type;

      console.log("Signal types - output:", outputType, "input:", inputType);

      if (outputType !== inputType) {
        console.log("TYPE MISMATCH! Removing connection");
        // Type mismatch - remove connection
        editor.removeSingleConnection(
          connection.output_id,
          connection.input_id,
          connection.output_class,
          connection.input_class
        );
        showError(`Cannot connect ${outputType} to ${inputType}`);
        return;
      }

      console.log("Types match - proceeding with connection");

      // Auto-switch Oscilloscope to V/oct trigger mode when connecting to V/oct input
      if (inputNode.name === 'Oscilloscope' && inputPort === 1) {
        console.log(`Auto-switching Oscilloscope node ${connection.input_id} to V/oct trigger mode`);
        // Set trigger_mode parameter (id: 1) to value 3 (V/oct)
        const triggerModeSlider = document.querySelector(`#node-${connection.input_id} input[data-param="1"]`);
        const triggerModeSpan = document.querySelector(`#trigger_mode-${connection.input_id}`);
        if (triggerModeSlider) {
          triggerModeSlider.value = 3;
          if (triggerModeSpan) {
            triggerModeSpan.textContent = 'V/oct';
          }
          // Update backend parameter
          if (inputNode.data.backendId !== null) {
            const currentTrackId = getCurrentMidiTrack();
            if (currentTrackId !== null) {
              invoke("graph_set_parameter", {
                trackId: currentTrackId,
                nodeId: inputNode.data.backendId,
                paramId: 1,
                value: 3.0
              }).catch(err => console.error("Failed to set V/oct trigger mode:", err));
            }
          }
        }
      }

      // Style the connection based on signal type
      setTimeout(() => {
        const connectionElement = document.querySelector(
          `.connection.node_in_node-${connection.input_id}.node_out_node-${connection.output_id}`
        );
        if (connectionElement) {
          connectionElement.classList.add(`connection-${outputType}`);
        }
      }, 10);

      // Send to backend
      console.log("Backend IDs - output:", outputNode.data.backendId, "input:", inputNode.data.backendId);
      if (outputNode.data.backendId !== null && inputNode.data.backendId !== null) {
        const trackInfo = getCurrentTrack();
        if (trackInfo === null) return;
        const currentTrackId = trackInfo.trackId;

        // Check if we're in template editing mode (dedicated view)
        if (editingContext) {
          // Connecting in template view
          console.log(`Connecting in template ${editingContext.voiceAllocatorId}: node ${outputNode.data.backendId} port ${outputPort} -> node ${inputNode.data.backendId} port ${inputPort}`);
          invoke("graph_connect_in_template", {
            trackId: currentTrackId,
            voiceAllocatorId: editingContext.voiceAllocatorId,
            fromNode: outputNode.data.backendId,
            fromPort: outputPort,
            toNode: inputNode.data.backendId,
            toPort: inputPort
          }).then(() => {
            console.log("Template connection successful");
          }).catch(err => {
            console.error("Failed to connect nodes in template:", err);
            showError("Template connection failed: " + err);
            // Remove the connection
            editor.removeSingleConnection(
              connection.output_id,
              connection.input_id,
              connection.output_class,
              connection.input_class
            );
          });
        } else {
          // Check if both nodes are inside the same VoiceAllocator (inline editing)
          // Convert connection IDs to numbers to match Map keys
          const outputId = parseInt(connection.output_id);
          const inputId = parseInt(connection.input_id);
          const outputParent = nodeParents.get(outputId);
          const inputParent = nodeParents.get(inputId);
          console.log(`Parent detection - output node ${outputId} parent: ${outputParent}, input node ${inputId} parent: ${inputParent}`);

          if (outputParent && inputParent && outputParent === inputParent) {
            // Both nodes are inside the same VoiceAllocator - connect in template (inline editing)
            const parentNode = editor.getNodeFromId(outputParent);
            console.log(`Connecting in VoiceAllocator template ${parentNode.data.backendId}: node ${outputNode.data.backendId} port ${outputPort} -> node ${inputNode.data.backendId} port ${inputPort}`);
            invoke("graph_connect_in_template", {
              trackId: currentTrackId,
              voiceAllocatorId: parentNode.data.backendId,
              fromNode: outputNode.data.backendId,
              fromPort: outputPort,
              toNode: inputNode.data.backendId,
              toPort: inputPort
            }).then(() => {
              console.log("Template connection successful");
            }).catch(err => {
              console.error("Failed to connect nodes in template:", err);
              showError("Template connection failed: " + err);
              // Remove the connection
              editor.removeSingleConnection(
                connection.output_id,
                connection.input_id,
                connection.output_class,
                connection.input_class
              );
            });
          } else {
            // Normal connection in main graph (skip if action is handling it)
            console.log(`Connecting: node ${outputNode.data.backendId} port ${outputPort} -> node ${inputNode.data.backendId} port ${inputPort}`);
            invoke("graph_connect", {
              trackId: currentTrackId,
              fromNode: outputNode.data.backendId,
              fromPort: outputPort,
              toNode: inputNode.data.backendId,
              toPort: inputPort
            }).then(async () => {
              console.log("Connection successful");

              // Record action for undo (only if not suppressing)
              if (!suppressActionRecording) {
                redoStack.length = 0;
                undoStack.push({
                  name: "graphAddConnection",
                  action: {
                    trackId: currentTrackId,
                    fromNode: outputNode.data.backendId,
                    fromPort: outputPort,
                    toNode: inputNode.data.backendId,
                    toPort: inputPort,
                    // Store frontend IDs for disconnection
                    frontendFromId: connection.output_id,
                    frontendToId: connection.input_id,
                    fromPortClass: connection.output_class,
                    toPortClass: connection.input_class
                  }
                });
              }

              // Auto-name AutomationInput nodes when connected
              await updateAutomationName(
                currentTrackId,
                outputNode.data.backendId,
                inputNode.data.backendId,
                connection.input_class
              );

              updateMenu();
            }).catch(err => {
              console.error("Failed to connect nodes:", err);
              showError("Connection failed: " + err);
              // Remove the connection
              editor.removeSingleConnection(
                connection.output_id,
                connection.input_id,
                connection.output_class,
                connection.input_class
              );
            });
          }
        }
      }

    } else {
      console.log("Port validation FAILED - ports out of bounds");
    }
  }

  // Handle connection removal
  function handleConnectionRemoved(connection) {
    const outputNode = editor.getNodeFromId(connection.output_id);
    const inputNode = editor.getNodeFromId(connection.input_id);

    if (!outputNode || !inputNode) return;

    // Drawflow uses 1-based indexing, but our arrays are 0-based
    const outputPort = parseInt(connection.output_class.replace("output_", "")) - 1;
    const inputPort = parseInt(connection.input_class.replace("input_", "")) - 1;

    // Auto-switch Oscilloscope back to Free mode when disconnecting V/oct input
    if (inputNode.name === 'Oscilloscope' && inputPort === 1) {
      console.log(`Auto-switching Oscilloscope node ${connection.input_id} back to Free trigger mode`);
      const triggerModeSlider = document.querySelector(`#node-${connection.input_id} input[data-param="1"]`);
      const triggerModeSpan = document.querySelector(`#trigger_mode-${connection.input_id}`);
      if (triggerModeSlider) {
        triggerModeSlider.value = 0;
        if (triggerModeSpan) {
          triggerModeSpan.textContent = 'Free';
        }
        // Update backend parameter
        if (inputNode.data.backendId !== null) {
          const currentTrackId = getCurrentMidiTrack();
          if (currentTrackId !== null) {
            invoke("graph_set_parameter", {
              trackId: currentTrackId,
              nodeId: inputNode.data.backendId,
              paramId: 1,
              value: 0.0
            }).catch(err => console.error("Failed to set Free trigger mode:", err));
          }
        }
      }
    }

    // Send to backend
    if (outputNode.data.backendId !== null && inputNode.data.backendId !== null) {
      const trackInfo = getCurrentTrack();
      if (trackInfo !== null) {
        invoke("graph_disconnect", {
          trackId: trackInfo.trackId,
          fromNode: outputNode.data.backendId,
          fromPort: outputPort,
          toNode: inputNode.data.backendId,
          toPort: inputPort
        }).then(() => {
          // Record action for undo (only if not suppressing)
          if (!suppressActionRecording) {
            redoStack.length = 0;
            undoStack.push({
              name: "graphRemoveConnection",
              action: {
                trackId: trackInfo.trackId,
                fromNode: outputNode.data.backendId,
                fromPort: outputPort,
                toNode: inputNode.data.backendId,
                toPort: inputPort,
                // Store frontend IDs for reconnection
                frontendFromId: connection.output_id,
                frontendToId: connection.input_id,
                fromPortClass: connection.output_class,
                toPortClass: connection.input_class
              }
            });
            updateMenu();
          }
        }).catch(err => {
          console.error("Failed to disconnect nodes:", err);
        });
      }
    }
  }

  // Show error message
  function showError(message) {
    const errorDiv = document.createElement("div");
    errorDiv.className = "node-editor-error";
    errorDiv.textContent = message;
    container.appendChild(errorDiv);

    setTimeout(() => {
      errorDiv.remove();
    }, 3000);
  }

  // Function to update breadcrumb display
  function updateBreadcrumb() {
    const breadcrumb = header.querySelector('.context-breadcrumb');
    if (editingContext) {
      // Determine main graph name based on track type
      const trackInfo = getCurrentTrack();
      const mainGraphName = trackInfo?.trackType === 'audio' ? 'Effects Graph' : 'Instrument Graph';

      breadcrumb.innerHTML = `
        ${mainGraphName} &gt;
        <span class="template-name">${editingContext.voiceAllocatorName} Template</span>
        <button class="exit-template-btn">← Exit Template</button>
      `;
      const exitBtn = breadcrumb.querySelector('.exit-template-btn');
      exitBtn.addEventListener('click', exitTemplate);
    } else {
      // Not in template mode - show main graph name based on track type
      const trackInfo = getCurrentTrack();
      const graphName = trackInfo?.trackType === 'audio' ? 'Effects Graph' :
                        trackInfo?.trackType === 'midi' ? 'Instrument Graph' :
                        'Node Graph';
      breadcrumb.textContent = graphName;
    }
  }

  // Function to enter template editing mode
  async function enterTemplate(voiceAllocatorId, voiceAllocatorName) {
    editingContext = { voiceAllocatorId, voiceAllocatorName };
    updateBreadcrumb();
    updatePalette();
    await reloadGraph();
  }

  // Function to exit template editing mode
  async function exitTemplate() {
    editingContext = null;
    updateBreadcrumb();
    updatePalette();
    await reloadGraph();
  }

  // Function to reload graph from backend
  async function reloadGraph() {
    if (!editor) return;

    const trackInfo = getCurrentTrack();

    // Clear editor first
    editor.clearModuleSelected();
    editor.clear();

    // Update UI based on track type
    updateBreadcrumb();
    updatePalette();

    // If no track selected, just leave it cleared
    if (trackInfo === null) {
      console.log('No track selected, editor cleared');
      return;
    }

    const trackId = trackInfo.trackId;

    try {
      // Get graph based on editing context
      let graphJson;
      if (editingContext) {
        // Loading template graph
        graphJson = await invoke('graph_get_template_state', {
          trackId,
          voiceAllocatorId: editingContext.voiceAllocatorId
        });
      } else {
        // Loading main graph
        graphJson = await invoke('graph_get_state', { trackId });
      }

      const preset = JSON.parse(graphJson);

      // If graph is empty (no nodes), just leave cleared
      if (!preset.nodes || preset.nodes.length === 0) {
        console.log('Graph is empty, editor cleared');
        return;
      }

      // Rebuild from preset
      const nodeMap = new Map(); // Maps backend node ID to Drawflow node ID
      const setupPromises = []; // Track async setup operations

      // Add all nodes
      for (const serializedNode of preset.nodes) {
        const nodeType = serializedNode.node_type;
        const nodeDef = nodeTypes[nodeType];
        if (!nodeDef) continue;

        // Create node HTML using the node definition's getHTML function
        // Use backend node ID as the nodeId for unique element IDs
        const html = nodeDef.getHTML(serializedNode.id);

        // Add node to Drawflow
        const drawflowId = editor.addNode(
          nodeType,
          nodeDef.inputs.length,
          nodeDef.outputs.length,
          serializedNode.position[0],
          serializedNode.position[1],
          nodeType,
          { nodeType, backendId: serializedNode.id, parentNodeId: null },
          html,
          false
        );

        nodeMap.set(serializedNode.id, drawflowId);

        // Style ports (as Promise)
        setupPromises.push(new Promise(resolve => {
          setTimeout(() => {
            styleNodePorts(drawflowId, nodeDef);
            resolve();
          }, 10);
        }));

        // Wire up parameter controls and set values from preset (as Promise)
        setupPromises.push(new Promise(resolve => {
          setTimeout(() => {
          const nodeElement = container.querySelector(`#node-${drawflowId}`);
          if (!nodeElement) return;

          // Set parameter values from preset
          nodeElement.querySelectorAll('input[type="range"]').forEach(slider => {
            const paramId = parseInt(slider.dataset.param);
            const value = serializedNode.parameters[paramId];
            if (value !== undefined) {
              slider.value = value;
              // Update display span
              const param = nodeDef.parameters.find(p => p.id === paramId);
              const displaySpan = slider.previousElementSibling?.querySelector('span');
              if (displaySpan && param) {
                displaySpan.textContent = value.toFixed(param.unit === 'Hz' ? 0 : 2) + (param.unit ? ` ${param.unit}` : '');
              }
            }
          });

          // Set up event handlers for buttons

          // Handle Load Sample button for SimpleSampler
          const loadSampleBtn = nodeElement.querySelector(".load-sample-btn");
          if (loadSampleBtn) {
            loadSampleBtn.addEventListener("mousedown", (e) => e.stopPropagation());
            loadSampleBtn.addEventListener("pointerdown", (e) => e.stopPropagation());
            loadSampleBtn.addEventListener("click", async (e) => {
              e.stopPropagation();

              const nodeData = editor.getNodeFromId(drawflowId);
              if (!nodeData || nodeData.data.backendId === null) {
                showError("Node not yet created on backend");
                return;
              }

              const currentTrackId = getCurrentMidiTrack();
              if (currentTrackId === null) {
                showError("No MIDI track selected");
                return;
              }

              try {
                const filePath = await openFileDialog({
                  title: "Load Audio Sample",
                  filters: [{
                    name: "Audio Files",
                    extensions: audioExtensions
                  }]
                });

                if (filePath) {
                  await invoke("sampler_load_sample", {
                    trackId: currentTrackId,
                    nodeId: nodeData.data.backendId,
                    filePath: filePath
                  });

                  // Update UI to show filename
                  const sampleInfo = nodeElement.querySelector(`#sample-info-${drawflowId}`);
                  if (sampleInfo) {
                    const filename = filePath.split('/').pop().split('\\').pop();
                    sampleInfo.textContent = filename;
                  }
                }
              } catch (err) {
                console.error("Failed to load sample:", err);
                showError(`Failed to load sample: ${err}`);
              }
            });
          }

          // Handle Add Layer button for MultiSampler
          const addLayerBtn = nodeElement.querySelector(".add-layer-btn");
          if (addLayerBtn) {
            addLayerBtn.addEventListener("mousedown", (e) => e.stopPropagation());
            addLayerBtn.addEventListener("pointerdown", (e) => e.stopPropagation());
            addLayerBtn.addEventListener("click", async (e) => {
              e.stopPropagation();

              const nodeData = editor.getNodeFromId(drawflowId);
              if (!nodeData || nodeData.data.backendId === null) {
                showError("Node not yet created on backend");
                return;
              }

              const currentTrackId = getCurrentMidiTrack();
              if (currentTrackId === null) {
                showError("No MIDI track selected");
                return;
              }

              try {
                const filePath = await openFileDialog({
                  title: "Add Sample Layer",
                  filters: [{
                    name: "Audio Files",
                    extensions: audioExtensions
                  }]
                });

                if (filePath) {
                  // Show dialog to configure layer mapping
                  const layerConfig = await showLayerConfigDialog(filePath);

                  if (layerConfig) {
                    await invoke("multi_sampler_add_layer", {
                      trackId: currentTrackId,
                      nodeId: nodeData.data.backendId,
                      filePath: filePath,
                      keyMin: layerConfig.keyMin,
                      keyMax: layerConfig.keyMax,
                      rootKey: layerConfig.rootKey,
                      velocityMin: layerConfig.velocityMin,
                      velocityMax: layerConfig.velocityMax,
                      loopStart: layerConfig.loopStart,
                      loopEnd: layerConfig.loopEnd,
                      loopMode: layerConfig.loopMode
                    });

                    // Wait a bit for the audio thread to process the add command
                    await new Promise(resolve => setTimeout(resolve, 100));

                    // Refresh the layers list
                    await refreshSampleLayersList(drawflowId);
                  }
                }
              } catch (err) {
                console.error("Failed to add layer:", err);
                showError(`Failed to add layer: ${err}`);
              }
            });
          }

          // For MultiSampler nodes, populate the layers table from preset data
          if (nodeType === 'MultiSampler') {
            console.log(`[reloadGraph] Found MultiSampler node ${drawflowId}, sample_data:`, serializedNode.sample_data);
            if (serializedNode.sample_data) {
              console.log(`[reloadGraph] sample_data.type:`, serializedNode.sample_data.type);
              console.log(`[reloadGraph] sample_data keys:`, Object.keys(serializedNode.sample_data));
            }
          }

          if (nodeType === 'MultiSampler' && serializedNode.sample_data && serializedNode.sample_data.type === 'multi_sampler') {
            console.log(`[reloadGraph] Condition met for node ${drawflowId}, looking for layers list element with backend ID ${serializedNode.id}`);
            // Use backend ID (serializedNode.id) since that's what was used in getHTML
            const layersList = nodeElement.querySelector(`#sample-layers-list-${serializedNode.id}`);
            const layersContainer = nodeElement.querySelector(`#sample-layers-container-${serializedNode.id}`);
            console.log(`[reloadGraph] layersList:`, layersList);
            console.log(`[reloadGraph] layersContainer:`, layersContainer);

            if (layersList) {
              const layers = serializedNode.sample_data.layers || [];
              console.log(`[reloadGraph] Populating ${layers.length} layers for node ${drawflowId}`);

              // Prevent scroll events from bubbling to canvas
              if (layersContainer && !layersContainer.dataset.scrollListenerAdded) {
                layersContainer.addEventListener('wheel', (e) => {
                  e.stopPropagation();
                }, { passive: false });
                layersContainer.dataset.scrollListenerAdded = 'true';
              }

              if (layers.length === 0) {
                layersList.innerHTML = '<tr><td colspan="5" class="sample-layers-empty">No layers loaded</td></tr>';
              } else {
                layersList.innerHTML = layers.map((layer, index) => {
                  const filename = layer.file_path.split('/').pop().split('\\').pop();
                  const keyRange = `${midiToNoteName(layer.key_min)}-${midiToNoteName(layer.key_max)}`;
                  const rootNote = midiToNoteName(layer.root_key);
                  const velRange = `${layer.velocity_min}-${layer.velocity_max}`;

                  return `
                    <tr data-index="${index}">
                      <td class="sample-layer-filename" title="${filename}">${filename}</td>
                      <td>${keyRange}</td>
                      <td>${rootNote}</td>
                      <td>${velRange}</td>
                      <td>
                        <div class="sample-layer-actions">
                          <button class="btn-edit-layer" data-drawflow-node="${drawflowId}" data-index="${index}">Edit</button>
                          <button class="btn-delete-layer" data-drawflow-node="${drawflowId}" data-index="${index}">Del</button>
                        </div>
                      </td>
                    </tr>
                  `;
                }).join('');

                // Set up event handlers for edit/delete buttons
                layersList.querySelectorAll('.btn-edit-layer').forEach(btn => {
                  btn.addEventListener('click', async (e) => {
                    e.stopPropagation();
                    const drawflowNodeId = parseInt(btn.dataset.drawflowNode);
                    const layerIndex = parseInt(btn.dataset.index);
                    const layer = layers[layerIndex];
                    await showLayerEditDialog(drawflowNodeId, layerIndex, layer);
                  });
                });

                layersList.querySelectorAll('.btn-delete-layer').forEach(btn => {
                  btn.addEventListener('click', async (e) => {
                    e.stopPropagation();
                    const drawflowNodeId = parseInt(btn.dataset.drawflowNode);
                    const layerIndex = parseInt(btn.dataset.index);
                    if (confirm('Delete this sample layer?')) {
                      const nodeData = editor.getNodeFromId(drawflowNodeId);
                      const currentTrackId = getCurrentMidiTrack();
                      if (nodeData && currentTrackId !== null) {
                        try {
                          await invoke("multi_sampler_remove_layer", {
                            trackId: currentTrackId,
                            nodeId: nodeData.data.backendId,
                            layerIndex: layerIndex
                          });
                          await refreshSampleLayersList(drawflowNodeId);
                        } catch (err) {
                          showError(`Failed to remove layer: ${err}`);
                        }
                      }
                    }
                  });
                });
              }
            }
          }

          // For Oscilloscope nodes, start the visualization
          if (nodeType === 'Oscilloscope' && serializedNode.id && trackId) {
            startOscilloscopeVisualization(drawflowId, trackId, serializedNode.id, editor);
          }

          resolve();
        }, 100);
        }));
      }

      // Add all connections
      for (const conn of preset.connections) {
        const outputDrawflowId = nodeMap.get(conn.from_node);
        const inputDrawflowId = nodeMap.get(conn.to_node);

        if (outputDrawflowId && inputDrawflowId) {
          // Drawflow uses 1-based port indexing
          editor.addConnection(
            outputDrawflowId,
            inputDrawflowId,
            `output_${conn.from_port + 1}`,
            `input_${conn.to_port + 1}`
          );

          // Style the connection based on signal type
          // We need to look up the node type and get the output port signal type
          setupPromises.push(new Promise(resolve => {
            setTimeout(() => {
              const outputNode = editor.getNodeFromId(outputDrawflowId);
              if (outputNode) {
                const nodeType = outputNode.data.nodeType;
                const nodeDef = nodeTypes[nodeType];
                if (nodeDef && conn.from_port < nodeDef.outputs.length) {
                  const signalType = nodeDef.outputs[conn.from_port].type;
                  const connectionElement = document.querySelector(
                    `.connection.node_in_node-${inputDrawflowId}.node_out_node-${outputDrawflowId}`
                  );
                  if (connectionElement) {
                    connectionElement.classList.add(`connection-${signalType}`);
                  }
                }
              }
              resolve();
            }, 10);
          }));
        }
      }

      // Wait for all node setup operations to complete
      await Promise.all(setupPromises);

      console.log('Graph reloaded from backend');
    } catch (error) {
      console.error('Failed to reload graph:', error);
      showError(`Failed to reload graph: ${error}`);
    }
  }

  // Store reload function in context so it can be called from preset browser
  // Wrap it to track the promise
  context.reloadNodeEditor = async () => {
    context.reloadGraphPromise = reloadGraph();
    await context.reloadGraphPromise;
    context.reloadGraphPromise = null;
  };

  // Store refreshSampleLayersList in context so it can be called from event handlers
  context.refreshSampleLayersList = refreshSampleLayersList;

  // Initial load of graph
  setTimeout(() => reloadGraph(), 200);

  return container;
}

function piano() {
  let piano_cvs = document.createElement("canvas");
  piano_cvs.className = "piano";

  // Create the virtual piano widget
  piano_cvs.virtualPiano = new VirtualPiano();

  // Variable to store the last time updatePianoCanvasSize was called
  let lastResizeTime = 0;
  const throttleIntervalMs = 20;

  function updatePianoCanvasSize() {
    const canvasStyles = window.getComputedStyle(piano_cvs);
    const width = parseInt(canvasStyles.width);
    const height = parseInt(canvasStyles.height);

    // Set actual size in memory (scaled for retina displays)
    piano_cvs.width = width * window.devicePixelRatio;
    piano_cvs.height = height * window.devicePixelRatio;

    // Normalize coordinate system to use CSS pixels
    const ctx = piano_cvs.getContext("2d");
    ctx.scale(window.devicePixelRatio, window.devicePixelRatio);

    // Render the piano
    piano_cvs.virtualPiano.draw(ctx, width, height);
  }

  // Store references in context for global access
  context.pianoWidget = piano_cvs.virtualPiano;
  context.pianoCanvas = piano_cvs;
  context.pianoRedraw = updatePianoCanvasSize;

  const resizeObserver = new ResizeObserver((entries) => {
    const currentTime = Date.now();
    if (currentTime - lastResizeTime >= throttleIntervalMs) {
      lastResizeTime = currentTime;
      updatePianoCanvasSize();
    }
  });
  resizeObserver.observe(piano_cvs);

  // Mouse event handlers
  piano_cvs.addEventListener("mousedown", (e) => {
    const rect = piano_cvs.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const width = parseInt(window.getComputedStyle(piano_cvs).width);
    const height = parseInt(window.getComputedStyle(piano_cvs).height);
    piano_cvs.virtualPiano.mousedown(x, y, width, height);
    updatePianoCanvasSize(); // Redraw to show pressed state
  });

  piano_cvs.addEventListener("mousemove", (e) => {
    const rect = piano_cvs.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const width = parseInt(window.getComputedStyle(piano_cvs).width);
    const height = parseInt(window.getComputedStyle(piano_cvs).height);
    piano_cvs.virtualPiano.mousemove(x, y, width, height);
    updatePianoCanvasSize(); // Redraw to show hover state
  });

  piano_cvs.addEventListener("mouseup", (e) => {
    const rect = piano_cvs.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const width = parseInt(window.getComputedStyle(piano_cvs).width);
    const height = parseInt(window.getComputedStyle(piano_cvs).height);
    piano_cvs.virtualPiano.mouseup(x, y, width, height);
    updatePianoCanvasSize(); // Redraw to show released state
  });

  // Prevent text selection
  piano_cvs.addEventListener("selectstart", (e) => e.preventDefault());

  // Add header controls for octave and velocity
  piano_cvs.headerControls = function() {
    const controls = [];

    // Octave control
    const octaveLabel = document.createElement("span");
    octaveLabel.style.marginLeft = "auto";
    octaveLabel.style.marginRight = "10px";
    octaveLabel.style.fontSize = "12px";
    octaveLabel.textContent = `Octave: ${piano_cvs.virtualPiano.octaveOffset >= 0 ? '+' : ''}${piano_cvs.virtualPiano.octaveOffset} (Z/X)`;

    // Velocity control
    const velocityLabel = document.createElement("span");
    velocityLabel.style.marginRight = "10px";
    velocityLabel.style.fontSize = "12px";
    velocityLabel.textContent = `Velocity: ${piano_cvs.virtualPiano.velocity} (C/V)`;

    // Update function to refresh labels
    const updateLabels = () => {
      octaveLabel.textContent = `Octave: ${piano_cvs.virtualPiano.octaveOffset >= 0 ? '+' : ''}${piano_cvs.virtualPiano.octaveOffset} (Z/X)`;
      velocityLabel.textContent = `Velocity: ${piano_cvs.virtualPiano.velocity} (C/V)`;
    };

    // Listen for keyboard events to update labels
    window.addEventListener('keydown', (e) => {
      if (['z', 'x', 'c', 'v'].includes(e.key.toLowerCase())) {
        // Delay slightly to let the piano widget update first
        setTimeout(updateLabels, 10);
      }
    });

    controls.push(octaveLabel);
    controls.push(velocityLabel);

    return controls;
  };

  return piano_cvs;
}

function pianoRoll() {
  let canvas = document.createElement("canvas");
  canvas.className = "piano-roll";

  // Create the piano roll editor widget
  canvas.pianoRollEditor = new PianoRollEditor(0, 0, 0, 0);

  function updateCanvasSize() {
    const canvasStyles = window.getComputedStyle(canvas);
    const width = parseInt(canvasStyles.width);
    const height = parseInt(canvasStyles.height);

    // Update widget dimensions
    canvas.pianoRollEditor.width = width;
    canvas.pianoRollEditor.height = height;

    // Set actual size in memory (scaled for retina displays)
    canvas.width = width * window.devicePixelRatio;
    canvas.height = height * window.devicePixelRatio;

    // Normalize coordinate system to use CSS pixels
    const ctx = canvas.getContext("2d");
    ctx.scale(window.devicePixelRatio, window.devicePixelRatio);

    // Render the piano roll
    canvas.pianoRollEditor.draw(ctx);
  }

  // Store references in context for global access and playback updates
  context.pianoRollEditor = canvas.pianoRollEditor;
  context.pianoRollCanvas = canvas;
  context.pianoRollRedraw = updateCanvasSize;

  const resizeObserver = new ResizeObserver(() => {
    updateCanvasSize();
  });
  resizeObserver.observe(canvas);

  // Pointer event handlers (works with mouse and touch)
  canvas.addEventListener("pointerdown", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    canvas.pianoRollEditor.handleMouseEvent("mousedown", x, y);
    updateCanvasSize();
  });

  canvas.addEventListener("pointermove", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    canvas.pianoRollEditor.handleMouseEvent("mousemove", x, y);

    // Update cursor based on widget state
    if (canvas.pianoRollEditor.cursor) {
      canvas.style.cursor = canvas.pianoRollEditor.cursor;
    }

    updateCanvasSize();
  });

  canvas.addEventListener("pointerup", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    canvas.pianoRollEditor.handleMouseEvent("mouseup", x, y);
    updateCanvasSize();
  });

  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    canvas.pianoRollEditor.wheel(e);
    updateCanvasSize();
  });

  // Prevent text selection
  canvas.addEventListener("selectstart", (e) => e.preventDefault());

  return canvas;
}

function presetBrowser() {
  const container = document.createElement("div");
  container.className = "preset-browser-pane";

  container.innerHTML = `
    <div class="preset-browser-header">
      <h3>Instrument Presets</h3>
      <button class="preset-btn preset-save-btn" title="Save current graph as preset">
        <span>💾</span> Save Preset
      </button>
    </div>
    <div class="preset-filter">
      <input type="text" id="preset-search" placeholder="Search presets..." />
      <select id="preset-tag-filter">
        <option value="">All Tags</option>
      </select>
    </div>
    <div class="preset-categories">
      <div class="preset-category">
        <h4>Factory Presets</h4>
        <div class="preset-list" id="factory-preset-list">
          <div class="preset-loading">Loading...</div>
        </div>
      </div>
      <div class="preset-category">
        <h4>User Presets</h4>
        <div class="preset-list" id="user-preset-list">
          <div class="preset-empty">No user presets yet</div>
        </div>
      </div>
    </div>
  `;

  // Load presets after DOM insertion
  setTimeout(async () => {
    await loadPresetList(container);

    // Set up save button handler
    const saveBtn = container.querySelector('.preset-save-btn');
    if (saveBtn) {
      saveBtn.addEventListener('click', () => showSavePresetDialog(container));
    }

    // Set up search and filter
    const searchInput = container.querySelector('#preset-search');
    const tagFilter = container.querySelector('#preset-tag-filter');

    if (searchInput) {
      searchInput.addEventListener('input', () => filterPresets(container));
    }
    if (tagFilter) {
      tagFilter.addEventListener('change', () => filterPresets(container));
    }
  }, 0);

  return container;
}

async function loadPresetList(container) {
  try {
    const presets = await invoke('graph_list_presets');

    const factoryList = container.querySelector('#factory-preset-list');
    const userList = container.querySelector('#user-preset-list');
    const tagFilter = container.querySelector('#preset-tag-filter');

    // Collect all unique tags
    const allTags = new Set();
    presets.forEach(preset => {
      preset.tags.forEach(tag => allTags.add(tag));
    });

    // Populate tag filter
    if (tagFilter) {
      allTags.forEach(tag => {
        const option = document.createElement('option');
        option.value = tag;
        option.textContent = tag.charAt(0).toUpperCase() + tag.slice(1);
        tagFilter.appendChild(option);
      });
    }

    // Separate factory and user presets
    const factoryPresets = presets.filter(p => p.is_factory);
    const userPresets = presets.filter(p => !p.is_factory);

    // Render factory presets
    if (factoryList) {
      if (factoryPresets.length === 0) {
        factoryList.innerHTML = '<div class="preset-empty">No factory presets found</div>';
      } else {
        factoryList.innerHTML = factoryPresets.map(preset => createPresetItem(preset)).join('');
        addPresetItemHandlers(factoryList);
      }
    }

    // Render user presets
    if (userList) {
      if (userPresets.length === 0) {
        userList.innerHTML = '<div class="preset-empty">No user presets yet</div>';
      } else {
        userList.innerHTML = userPresets.map(preset => createPresetItem(preset)).join('');
        addPresetItemHandlers(userList);
      }
    }
  } catch (error) {
    console.error('Failed to load presets:', error);
    const factoryList = container.querySelector('#factory-preset-list');
    const userList = container.querySelector('#user-preset-list');
    if (factoryList) factoryList.innerHTML = '<div class="preset-error">Failed to load presets</div>';
    if (userList) userList.innerHTML = '';
  }
}

function createPresetItem(preset) {
  const tags = preset.tags.map(tag => `<span class="preset-tag">${tag}</span>`).join('');
  const deleteBtn = preset.is_factory ? '' : '<button class="preset-delete-btn" title="Delete preset">🗑️</button>';

  return `
    <div class="preset-item" data-preset-path="${preset.path}" data-preset-tags="${preset.tags.join(',')}">
      <div class="preset-item-header">
        <span class="preset-name">${preset.name}</span>
        <button class="preset-load-btn" title="Load preset">Load</button>
        ${deleteBtn}
      </div>
      <div class="preset-details">
        <div class="preset-description">${preset.description || 'No description'}</div>
        <div class="preset-tags">${tags}</div>
        <div class="preset-author">by ${preset.author || 'Unknown'}</div>
      </div>
    </div>
  `;
}

function addPresetItemHandlers(listElement) {
  // Toggle selection on preset item click
  listElement.querySelectorAll('.preset-item').forEach(item => {
    item.addEventListener('click', (e) => {
      // Don't trigger if clicking buttons
      if (e.target.classList.contains('preset-load-btn') ||
          e.target.classList.contains('preset-delete-btn')) {
        return;
      }

      // Toggle selection
      const wasSelected = item.classList.contains('selected');

      // Deselect all presets
      listElement.querySelectorAll('.preset-item').forEach(i => i.classList.remove('selected'));

      // Select this preset if it wasn't selected
      if (!wasSelected) {
        item.classList.add('selected');
      }
    });
  });

  // Load preset on Load button click
  listElement.querySelectorAll('.preset-load-btn').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const item = btn.closest('.preset-item');
      const presetPath = item.dataset.presetPath;
      await loadPreset(presetPath);
    });
  });

  // Delete preset on delete button click
  listElement.querySelectorAll('.preset-delete-btn').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const item = btn.closest('.preset-item');
      const presetPath = item.dataset.presetPath;
      const presetName = item.querySelector('.preset-name').textContent;

      if (confirm(`Delete preset "${presetName}"?`)) {
        try {
          await invoke('graph_delete_preset', { presetPath });
          // Reload preset list
          const container = btn.closest('.preset-browser-pane');
          await loadPresetList(container);
        } catch (error) {
          alert(`Failed to delete preset: ${error}`);
        }
      }
    });
  });
}

async function loadPreset(presetPath) {
  const trackInfo = getCurrentTrack();
  if (trackInfo === null) {
    alert('Please select a track first');
    return;
  }
  const trackId = trackInfo.trackId;

  try {
    await invoke('graph_load_preset', {
      trackId: trackId,
      presetPath
    });

    // Refresh the node editor to show the loaded preset
    await context.reloadNodeEditor?.();

    console.log('Preset loaded successfully');
  } catch (error) {
    alert(`Failed to load preset: ${error}`);
  }
}

function showSavePresetDialog(container) {
  const trackInfo = getCurrentTrack();
  if (trackInfo === null) {
    alert('Please select a track first');
    return;
  }

  // Create modal dialog
  const dialog = document.createElement('div');
  dialog.className = 'modal-overlay';
  dialog.innerHTML = `
    <div class="modal-dialog">
      <h3>Save Preset</h3>
      <form id="save-preset-form">
        <div class="form-group">
          <label>Preset Name</label>
          <input type="text" id="preset-name" required placeholder="My Awesome Synth" />
        </div>
        <div class="form-group">
          <label>Description</label>
          <textarea id="preset-description" placeholder="Describe the sound..." rows="3"></textarea>
        </div>
        <div class="form-group">
          <label>Tags (comma-separated)</label>
          <input type="text" id="preset-tags" placeholder="bass, lead, pad" />
        </div>
        <div class="form-actions">
          <button type="button" class="btn-cancel">Cancel</button>
          <button type="submit" class="btn-primary">Save</button>
        </div>
      </form>
    </div>
  `;

  document.body.appendChild(dialog);

  // Focus name input
  setTimeout(() => dialog.querySelector('#preset-name')?.focus(), 100);

  // Handle cancel
  dialog.querySelector('.btn-cancel').addEventListener('click', () => {
    dialog.remove();
  });

  // Handle save
  dialog.querySelector('#save-preset-form').addEventListener('submit', async (e) => {
    e.preventDefault();

    const name = dialog.querySelector('#preset-name').value.trim();
    const description = dialog.querySelector('#preset-description').value.trim();
    const tagsInput = dialog.querySelector('#preset-tags').value.trim();
    const tags = tagsInput ? tagsInput.split(',').map(t => t.trim()).filter(t => t) : [];

    if (!name) {
      alert('Please enter a preset name');
      return;
    }

    try {
      await invoke('graph_save_preset', {
        trackId: trackInfo.trackId,
        presetName: name,
        description,
        tags
      });

      dialog.remove();

      // Reload preset list
      await loadPresetList(container);

      alert(`Preset "${name}" saved successfully!`);
    } catch (error) {
      alert(`Failed to save preset: ${error}`);
    }
  });

  // Close on background click
  dialog.addEventListener('click', (e) => {
    if (e.target === dialog) {
      dialog.remove();
    }
  });
}

// Show preferences dialog
function showPreferencesDialog() {
  const dialog = document.createElement('div');
  dialog.className = 'modal-overlay';
  dialog.innerHTML = `
    <div class="modal-dialog preferences-dialog">
      <h3>Preferences</h3>
      <form id="preferences-form">
        <div class="form-group">
          <label>Default BPM</label>
          <input type="number" id="pref-bpm" min="20" max="300" value="${config.bpm}" />
        </div>
        <div class="form-group">
          <label>Default Framerate</label>
          <input type="number" id="pref-framerate" min="1" max="120" value="${config.framerate}" />
        </div>
        <div class="form-group">
          <label>Default File Width</label>
          <input type="number" id="pref-width" min="100" max="10000" value="${config.fileWidth}" />
        </div>
        <div class="form-group">
          <label>Default File Height</label>
          <input type="number" id="pref-height" min="100" max="10000" value="${config.fileHeight}" />
        </div>
        <div class="form-group">
          <label>Scroll Speed</label>
          <input type="number" id="pref-scroll-speed" min="0.1" max="10" step="0.1" value="${config.scrollSpeed}" />
        </div>
        <div class="form-group">
          <label>Audio Buffer Size (frames)</label>
          <select id="pref-audio-buffer-size">
            <option value="128" ${config.audioBufferSize === 128 ? 'selected' : ''}>128 (~3ms - Low latency)</option>
            <option value="256" ${config.audioBufferSize === 256 ? 'selected' : ''}>256 (~6ms - Balanced)</option>
            <option value="512" ${config.audioBufferSize === 512 ? 'selected' : ''}>512 (~12ms - Stable)</option>
            <option value="1024" ${config.audioBufferSize === 1024 ? 'selected' : ''}>1024 (~23ms - Very stable)</option>
            <option value="2048" ${config.audioBufferSize === 2048 ? 'selected' : ''}>2048 (~46ms - Low-end systems)</option>
            <option value="4096" ${config.audioBufferSize === 4096 ? 'selected' : ''}>4096 (~93ms - Very low-end systems)</option>
          </select>
          <small style="display: block; margin-top: 4px; color: #888;">Requires app restart to take effect</small>
        </div>
        <div class="form-group">
          <label>
            <input type="checkbox" id="pref-reopen-session" ${config.reopenLastSession ? 'checked' : ''} />
            Reopen last session on startup
          </label>
        </div>
        <div class="form-group">
          <label>
            <input type="checkbox" id="pref-restore-layout" ${config.restoreLayoutFromFile ? 'checked' : ''} />
            Restore layout when opening files
          </label>
        </div>
        <div class="form-group">
          <label>
            <input type="checkbox" id="pref-debug" ${config.debug ? 'checked' : ''} />
            Enable debug mode
          </label>
        </div>
        <div class="form-actions">
          <button type="button" class="btn-cancel">Cancel</button>
          <button type="submit" class="btn-primary">Save</button>
        </div>
      </form>
    </div>
  `;

  document.body.appendChild(dialog);

  // Focus first input
  setTimeout(() => dialog.querySelector('#pref-bpm')?.focus(), 100);

  // Handle cancel
  dialog.querySelector('.btn-cancel').addEventListener('click', () => {
    dialog.remove();
  });

  // Handle save
  dialog.querySelector('#preferences-form').addEventListener('submit', async (e) => {
    e.preventDefault();

    // Update config values
    config.bpm = parseInt(dialog.querySelector('#pref-bpm').value);
    config.framerate = parseInt(dialog.querySelector('#pref-framerate').value);
    config.fileWidth = parseInt(dialog.querySelector('#pref-width').value);
    config.fileHeight = parseInt(dialog.querySelector('#pref-height').value);
    config.scrollSpeed = parseFloat(dialog.querySelector('#pref-scroll-speed').value);
    config.audioBufferSize = parseInt(dialog.querySelector('#pref-audio-buffer-size').value);
    config.reopenLastSession = dialog.querySelector('#pref-reopen-session').checked;
    config.restoreLayoutFromFile = dialog.querySelector('#pref-restore-layout').checked;
    config.debug = dialog.querySelector('#pref-debug').checked;

    // Save config to localStorage
    await saveConfig();

    dialog.remove();

    console.log('Preferences saved:', config);
  });

  // Close on background click
  dialog.addEventListener('click', (e) => {
    if (e.target === dialog) {
      dialog.remove();
    }
  });
}

// Helper function to convert MIDI note number to note name
function midiToNoteName(midiNote) {
  const noteNames = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  const octave = Math.floor(midiNote / 12) - 1;
  const noteName = noteNames[midiNote % 12];
  return `${noteName}${octave}`;
}

// Show dialog to configure MultiSampler layer zones
function showLayerConfigDialog(filePath, existingConfig = null) {
  return new Promise((resolve) => {
    const filename = filePath.split('/').pop().split('\\').pop();
    const isEdit = existingConfig !== null;

    // Use existing values or defaults
    const keyMin = existingConfig?.keyMin ?? 0;
    const keyMax = existingConfig?.keyMax ?? 127;
    const rootKey = existingConfig?.rootKey ?? 60;
    const velocityMin = existingConfig?.velocityMin ?? 0;
    const velocityMax = existingConfig?.velocityMax ?? 127;
    const loopMode = existingConfig?.loopMode ?? 'oneshot';
    const loopStart = existingConfig?.loopStart ?? null;
    const loopEnd = existingConfig?.loopEnd ?? null;

    // Create modal dialog
    const dialog = document.createElement('div');
    dialog.className = 'modal-overlay';
    dialog.innerHTML = `
      <div class="modal-dialog">
        <h3>${isEdit ? 'Edit' : 'Configure'} Sample Layer</h3>
        <p style="font-size: 12px; color: #666; margin-bottom: 16px;">
          File: <strong>${filename}</strong>
        </p>
        <form id="layer-config-form">
          <div class="form-group">
            <label>Key Range</label>
            <div class="form-group-inline">
              <div>
                <label style="font-size: 11px; color: #888;">Min</label>
                <input type="number" id="key-min" min="0" max="127" value="${keyMin}" required />
                <div id="key-min-name" class="form-note-name">${midiToNoteName(keyMin)}</div>
              </div>
              <span>-</span>
              <div>
                <label style="font-size: 11px; color: #888;">Max</label>
                <input type="number" id="key-max" min="0" max="127" value="${keyMax}" required />
                <div id="key-max-name" class="form-note-name">${midiToNoteName(keyMax)}</div>
              </div>
            </div>
          </div>
          <div class="form-group">
            <label>Root Key (original pitch)</label>
            <input type="number" id="root-key" min="0" max="127" value="${rootKey}" required />
            <div id="root-key-name" class="form-note-name">${midiToNoteName(rootKey)}</div>
          </div>
          <div class="form-group">
            <label>Velocity Range</label>
            <div class="form-group-inline">
              <div>
                <label style="font-size: 11px; color: #888;">Min</label>
                <input type="number" id="velocity-min" min="0" max="127" value="${velocityMin}" required />
              </div>
              <span>-</span>
              <div>
                <label style="font-size: 11px; color: #888;">Max</label>
                <input type="number" id="velocity-max" min="0" max="127" value="${velocityMax}" required />
              </div>
            </div>
          </div>
          <div class="form-group">
            <label>Loop Mode</label>
            <select id="loop-mode">
              <option value="oneshot" ${loopMode === 'oneshot' ? 'selected' : ''}>One-Shot (play once)</option>
              <option value="continuous" ${loopMode === 'continuous' ? 'selected' : ''}>Continuous (loop)</option>
            </select>
            <div class="form-note" style="font-size: 11px; color: #888; margin-top: 4px;">
              Continuous mode will auto-detect loop points if not specified
            </div>
          </div>
          <div id="loop-points-group" class="form-group" style="display: ${loopMode === 'continuous' ? 'block' : 'none'};">
            <label>Loop Points (optional, samples)</label>
            <div class="form-group-inline">
              <div>
                <label style="font-size: 11px; color: #888;">Start</label>
                <input type="number" id="loop-start" min="0" value="${loopStart ?? ''}" placeholder="Auto" />
              </div>
              <span>-</span>
              <div>
                <label style="font-size: 11px; color: #888;">End</label>
                <input type="number" id="loop-end" min="0" value="${loopEnd ?? ''}" placeholder="Auto" />
              </div>
            </div>
            <div class="form-note" style="font-size: 11px; color: #888; margin-top: 4px;">
              Leave empty to auto-detect optimal loop points
            </div>
          </div>
          <div class="form-actions">
            <button type="button" class="btn-cancel">Cancel</button>
            <button type="submit" class="btn-primary">${isEdit ? 'Update' : 'Add'} Layer</button>
          </div>
        </form>
      </div>
    `;

    document.body.appendChild(dialog);

    // Update note names when inputs change
    const keyMinInput = dialog.querySelector('#key-min');
    const keyMaxInput = dialog.querySelector('#key-max');
    const rootKeyInput = dialog.querySelector('#root-key');
    const loopModeSelect = dialog.querySelector('#loop-mode');
    const loopPointsGroup = dialog.querySelector('#loop-points-group');

    const updateKeyMinName = () => {
      const note = parseInt(keyMinInput.value) || 0;
      dialog.querySelector('#key-min-name').textContent = midiToNoteName(note);
    };

    const updateKeyMaxName = () => {
      const note = parseInt(keyMaxInput.value) || 127;
      dialog.querySelector('#key-max-name').textContent = midiToNoteName(note);
    };

    const updateRootKeyName = () => {
      const note = parseInt(rootKeyInput.value) || 60;
      dialog.querySelector('#root-key-name').textContent = midiToNoteName(note);
    };

    keyMinInput.addEventListener('input', updateKeyMinName);
    keyMaxInput.addEventListener('input', updateKeyMaxName);
    rootKeyInput.addEventListener('input', updateRootKeyName);

    // Toggle loop points visibility based on loop mode
    loopModeSelect.addEventListener('change', () => {
      const isContinuous = loopModeSelect.value === 'continuous';
      loopPointsGroup.style.display = isContinuous ? 'block' : 'none';
    });

    // Focus first input
    setTimeout(() => dialog.querySelector('#key-min')?.focus(), 100);

    // Handle cancel
    dialog.querySelector('.btn-cancel').addEventListener('click', () => {
      dialog.remove();
      resolve(null);
    });

    // Handle submit
    dialog.querySelector('#layer-config-form').addEventListener('submit', (e) => {
      e.preventDefault();

      const keyMin = parseInt(keyMinInput.value);
      const keyMax = parseInt(keyMaxInput.value);
      const rootKey = parseInt(rootKeyInput.value);
      const velocityMin = parseInt(dialog.querySelector('#velocity-min').value);
      const velocityMax = parseInt(dialog.querySelector('#velocity-max').value);
      const loopMode = loopModeSelect.value;

      // Get loop points (null if empty)
      const loopStartInput = dialog.querySelector('#loop-start');
      const loopEndInput = dialog.querySelector('#loop-end');
      const loopStart = loopStartInput.value ? parseInt(loopStartInput.value) : null;
      const loopEnd = loopEndInput.value ? parseInt(loopEndInput.value) : null;

      // Validate ranges
      if (keyMin > keyMax) {
        alert('Key Min must be less than or equal to Key Max');
        return;
      }

      if (velocityMin > velocityMax) {
        alert('Velocity Min must be less than or equal to Velocity Max');
        return;
      }

      if (rootKey < keyMin || rootKey > keyMax) {
        alert('Root Key must be within the key range');
        return;
      }

      // Validate loop points if both are specified
      if (loopStart !== null && loopEnd !== null && loopStart >= loopEnd) {
        alert('Loop Start must be less than Loop End');
        return;
      }

      dialog.remove();
      resolve({
        keyMin,
        keyMax,
        rootKey,
        velocityMin,
        velocityMax,
        loopMode,
        loopStart,
        loopEnd
      });
    });

    // Close on background click
    dialog.addEventListener('click', (e) => {
      if (e.target === dialog) {
        dialog.remove();
        resolve(null);
      }
    });
  });
}

function filterPresets(container) {
  const searchTerm = container.querySelector('#preset-search')?.value.toLowerCase() || '';
  const selectedTag = container.querySelector('#preset-tag-filter')?.value || '';

  const allItems = container.querySelectorAll('.preset-item');

  allItems.forEach(item => {
    const name = item.querySelector('.preset-name').textContent.toLowerCase();
    const description = item.querySelector('.preset-description').textContent.toLowerCase();
    const tags = item.dataset.presetTags.split(',');

    const matchesSearch = !searchTerm || name.includes(searchTerm) || description.includes(searchTerm);
    const matchesTag = !selectedTag || tags.includes(selectedTag);

    item.style.display = (matchesSearch && matchesTag) ? 'block' : 'none';
  });
}

const panes = {
  stage: {
    name: "stage",
    func: stage,
  },
  toolbar: {
    name: "toolbar",
    func: toolbar,
  },
  timelineDeprecated: {
    name: "timeline-deprecated",
    func: timelineDeprecated,
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
  piano: {
    name: "piano",
    func: piano,
  },
  pianoRoll: {
    name: "piano-roll",
    func: pianoRoll,
  },
  nodeEditor: {
    name: "node-editor",
    func: nodeEditor,
  },
  presetBrowser: {
    name: "preset-browser",
    func: presetBrowser,
  },
};

/**
 * Switch to a different layout
 * @param {string} layoutKey - The key of the layout to switch to
 */
function switchLayout(layoutKey) {
  try {
    console.log(`Switching to layout: ${layoutKey}`);

    // Load the layout definition
    const layoutDef = loadLayoutByKeyOrName(layoutKey);
    if (!layoutDef) {
      console.error(`Layout not found: ${layoutKey}`);
      return;
    }

    // Clear existing layout (except root element)
    while (rootPane.firstChild) {
      rootPane.removeChild(rootPane.firstChild);
    }

    // Clear layoutElements array
    layoutElements.length = 0;

    // Clear canvases array (will be repopulated when stage pane is created)
    canvases.length = 0;

    // Build new layout from definition directly into rootPane
    buildLayout(rootPane, layoutDef, panes, createPane, splitPane);

    // Update config
    config.currentLayout = layoutKey;
    saveConfig();

    // Trigger layout update
    updateAll();
    updateUI();
    updateLayers();
    updateMenu();

    // Update metronome button visibility based on timeline format
    // (especially important when switching to audioDaw layout)
    if (context.metronomeGroup && context.timelineWidget?.timelineState) {
      const shouldShow = context.timelineWidget.timelineState.timeFormat === 'measures';
      context.metronomeGroup.style.display = shouldShow ? '' : 'none';
    }

    console.log(`Layout switched to: ${layoutDef.name}`);
  } catch (error) {
    console.error(`Error switching layout:`, error);
  }
}

/**
 * Switch to the next layout in the list
 */
function nextLayout() {
  const layoutKeys = getLayoutNames();
  const currentIndex = layoutKeys.indexOf(config.currentLayout);
  const nextIndex = (currentIndex + 1) % layoutKeys.length;
  switchLayout(layoutKeys[nextIndex]);
}

/**
 * Switch to the previous layout in the list
 */
function previousLayout() {
  const layoutKeys = getLayoutNames();
  const currentIndex = layoutKeys.indexOf(config.currentLayout);
  const prevIndex = (currentIndex - 1 + layoutKeys.length) % layoutKeys.length;
  switchLayout(layoutKeys[prevIndex]);
}

// Make layout functions available globally for menu actions
window.switchLayout = switchLayout;
window.nextLayout = nextLayout;
window.previousLayout = previousLayout;

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


let renderInProgress = false;
let rafScheduled = false;

// FPS tracking
let lastFpsLogTime = 0;
let frameCount = 0;
let fpsHistory = [];

async function renderAll() {
  rafScheduled = false;

  // Skip if a render is already in progress (prevent stacking async calls)
  if (renderInProgress) {
    // Schedule another attempt if not already scheduled
    if (!rafScheduled) {
      rafScheduled = true;
      requestAnimationFrame(renderAll);
    }
    return;
  }

  renderInProgress = true;
  const renderStartTime = performance.now();

  try {
    if (uiDirty) {
      await renderUI();
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
    renderInProgress = false;

    // FPS logging (only when playing)
    if (context.playing) {
      frameCount++;
      const now = performance.now();
      const renderTime = now - renderStartTime;

      if (now - lastFpsLogTime >= 1000) {
        const fps = frameCount / ((now - lastFpsLogTime) / 1000);
        fpsHistory.push({ fps, renderTime });
        console.log(`[FPS] ${fps.toFixed(1)} fps | Render time: ${renderTime.toFixed(1)}ms`);
        frameCount = 0;
        lastFpsLogTime = now;

        // Keep only last 10 samples
        if (fpsHistory.length > 10) {
          fpsHistory.shift();
        }
      }
    }

    // Schedule next frame if not already scheduled
    if (!rafScheduled) {
      rafScheduled = true;
      requestAnimationFrame(renderAll);
    }
  }
}

// Initialize actions module with dependencies
initializeActions({
  undoStack,
  redoStack,
  updateMenu,
  updateLayers,
  updateUI,
  updateVideoFrames,
  updateInfopanel,
  invoke,
  config
});

renderAll();

if (window.openedFiles?.length>0) {
  document.body.style.cursor = "wait"
  setTimeout(()=>_open(window.openedFiles[0]),10)
  for (let i=1; i<window.openedFiles.length; i++) {
    newWindow(window.openedFiles[i])
  }
}

async function addEmptyAudioTrack() {
  console.log('[addEmptyAudioTrack] BEFORE - root.frameRate:', root.frameRate);
  const trackName = `Audio Track ${context.activeObject.audioTracks.length + 1}`;
  const trackUuid = uuidv4();

  try {
    // Create new AudioTrack with DAW backend
    const newAudioTrack = new AudioTrack(trackUuid, trackName);

    // Initialize track in backend (creates empty audio track)
    await newAudioTrack.initializeTrack();

    console.log('[addEmptyAudioTrack] After initializeTrack - root.frameRate:', root.frameRate);

    // Add track to active object
    context.activeObject.audioTracks.push(newAudioTrack);

    console.log('[addEmptyAudioTrack] After push - root.frameRate:', root.frameRate);

    // Select the newly created track
    context.activeObject.activeLayer = newAudioTrack;

    console.log('[addEmptyAudioTrack] After setting activeLayer - root.frameRate:', root.frameRate);

    // Update UI
    updateLayers();
    if (context.timelineWidget) {
      context.timelineWidget.requestRedraw();
    }

    console.log('[addEmptyAudioTrack] AFTER - root.frameRate:', root.frameRate);
    console.log('Empty audio track created:', trackName, 'with ID:', newAudioTrack.audioTrackId);
  } catch (error) {
    console.error('Failed to create empty audio track:', error);
  }
}

async function addEmptyMIDITrack() {
  console.log('[addEmptyMIDITrack] Creating new MIDI track');
  const trackName = `MIDI Track ${context.activeObject.audioTracks.filter(t => t.type === 'midi').length + 1}`;
  const trackUuid = uuidv4();

  try {
    // Note: MIDI tracks now use node-based instruments via instrument_graph

    // Create new AudioTrack with type='midi'
    const newMIDITrack = new AudioTrack(trackUuid, trackName, 'midi');

    // Initialize track in backend (creates MIDI track with node graph)
    await newMIDITrack.initializeTrack();

    console.log('[addEmptyMIDITrack] After initializeTrack - track created with node graph');

    // Add track to active object
    context.activeObject.audioTracks.push(newMIDITrack);

    // Select the newly created track
    context.activeObject.activeLayer = newMIDITrack;

    // Update UI
    updateLayers();
    if (context.timelineWidget) {
      context.timelineWidget.requestRedraw();
    }

    // Refresh node editor to show empty graph
    setTimeout(() => context.reloadNodeEditor?.(), 100);

    console.log('Empty MIDI track created:', trackName, 'with ID:', newMIDITrack.audioTrackId);
  } catch (error) {
    console.error('Failed to create empty MIDI track:', error);
  }
}

async function addVideoLayer() {
  console.log('[addVideoLayer] Creating new video layer');
  const layerName = `Video ${context.activeObject.layers.filter(l => l.type === 'video').length + 1}`;
  const layerUuid = uuidv4();

  try {
    // Create new VideoLayer
    const newVideoLayer = new VideoLayer(layerUuid, layerName);

    // Add layer to active object
    context.activeObject.layers.push(newVideoLayer);

    // Select the newly created layer
    context.activeObject.activeLayer = newVideoLayer;

    // Update UI
    updateLayers();
    if (context.timelineWidget) {
      context.timelineWidget.requestRedraw();
    }

    console.log('Empty video layer created:', layerName);
  } catch (error) {
    console.error('Failed to create video layer:', error);
  }
}

// MIDI Command Wrappers
// Note: getAvailableInstruments() removed - now using node-based instruments

async function createMIDITrack(name, instrument) {
  try {
    const trackId = await invoke('audio_create_track', { name, trackType: 'midi', instrument });
    console.log('MIDI track created:', name, 'with instrument:', instrument, 'ID:', trackId);
    return trackId;
  } catch (error) {
    console.error('Failed to create MIDI track:', error);
    throw error;
  }
}

async function createMIDIClip(trackId, startTime, duration) {
  try {
    const clipId = await invoke('audio_create_midi_clip', { trackId, startTime, duration });
    console.log('MIDI clip created on track', trackId, 'with ID:', clipId);
    return clipId;
  } catch (error) {
    console.error('Failed to create MIDI clip:', error);
    throw error;
  }
}

async function addMIDINote(trackId, clipId, timeOffset, note, velocity, duration) {
  try {
    await invoke('audio_add_midi_note', { trackId, clipId, timeOffset, note, velocity, duration });
    console.log('MIDI note added:', note, 'at', timeOffset);
  } catch (error) {
    console.error('Failed to add MIDI note:', error);
    throw error;
  }
}

async function loadMIDIFile(trackId, path, startTime) {
  try {
    const duration = await invoke('audio_load_midi_file', { trackId, path, startTime });
    console.log('MIDI file loaded:', path, 'duration:', duration);
    return duration;
  } catch (error) {
    console.error('Failed to load MIDI file:', error);
    throw error;
  }
}

// ========== Oscilloscope Visualization ==========

// Store oscilloscope update intervals by node ID
const oscilloscopeIntervals = new Map();
// Store oscilloscope time scales by node ID
const oscilloscopeTimeScales = new Map();

// Start oscilloscope visualization for a node
function startOscilloscopeVisualization(nodeId, trackId, backendNodeId, editorRef) {
  // Clear any existing interval for this node
  stopOscilloscopeVisualization(nodeId);

  // Find the canvas by traversing from the node element
  const nodeElement = document.getElementById(`node-${nodeId}`);
  if (!nodeElement) {
    console.warn(`Node element not found for node ${nodeId}`);
    return;
  }

  const canvas = nodeElement.querySelector('canvas[id^="oscilloscope-canvas-"]');
  if (!canvas) {
    console.warn(`Oscilloscope canvas not found in node ${nodeId}`);
    return;
  }

  console.log(`Found oscilloscope canvas for node ${nodeId}:`, canvas.id);

  const ctx = canvas.getContext('2d');
  const width = canvas.width;
  const height = canvas.height;

  // Initialize time scale to default (100ms)
  if (!oscilloscopeTimeScales.has(nodeId)) {
    oscilloscopeTimeScales.set(nodeId, 100);
  }

  // Update function to fetch and draw oscilloscope data
  const updateOscilloscope = async () => {
    try {
      // Calculate samples needed based on time scale
      // Assuming 48kHz sample rate
      const timeScaleMs = oscilloscopeTimeScales.get(nodeId) || 100;
      const sampleRate = 48000;
      const samplesNeeded = Math.floor((timeScaleMs / 1000) * sampleRate);
      // Cap at 2 seconds worth of samples to avoid excessive memory usage
      const sampleCount = Math.min(samplesNeeded, sampleRate * 2);

      // Fetch oscilloscope data
      const data = await invoke('get_oscilloscope_data', {
        trackId: trackId,
        nodeId: backendNodeId,
        sampleCount: sampleCount
      });

      // Clear canvas
      ctx.fillStyle = '#1a1a1a';
      ctx.fillRect(0, 0, width, height);

      // Draw grid lines
      ctx.strokeStyle = '#2a2a2a';
      ctx.lineWidth = 1;

      // Horizontal grid lines
      ctx.beginPath();
      ctx.moveTo(0, height / 2);
      ctx.lineTo(width, height / 2);
      ctx.stroke();

      // Draw audio waveform
      if (data && data.audio && data.audio.length > 0) {
        ctx.strokeStyle = '#4CAF50';
        ctx.lineWidth = 2;
        ctx.beginPath();

        const xStep = width / data.audio.length;
        for (let i = 0; i < data.audio.length; i++) {
          const x = i * xStep;
          // Map sample value from [-1, 1] to canvas height
          const y = height / 2 - (data.audio[i] * height / 2);

          if (i === 0) {
            ctx.moveTo(x, y);
          } else {
            ctx.lineTo(x, y);
          }
        }
        ctx.stroke();
      }

      // Draw CV trace in orange if present and CV input is connected
      if (data && data.cv && data.cv.length > 0 && editorRef) {
        // Check if CV input (port index 2 = input_3 in drawflow) is connected
        const node = editorRef.getNodeFromId(nodeId);
        const cvInput = node?.inputs?.input_3;
        const isCvConnected = cvInput && cvInput.connections && cvInput.connections.length > 0;

        if (isCvConnected) {
          ctx.strokeStyle = '#FF9800';  // Orange color
          ctx.lineWidth = 2;
          ctx.beginPath();

          const xStep = width / data.cv.length;
          for (let i = 0; i < data.cv.length; i++) {
            const x = i * xStep;
            // Map CV value from [-1, 1] to canvas height
            const y = height / 2 - (data.cv[i] * height / 2);

            if (i === 0) {
              ctx.moveTo(x, y);
            } else {
              ctx.lineTo(x, y);
            }
          }
          ctx.stroke();
        }
      }
    } catch (error) {
      console.error('Failed to update oscilloscope:', error);
    }
  };

  // Initial update
  updateOscilloscope();

  // Update every 50ms (20 FPS)
  const interval = setInterval(updateOscilloscope, 50);
  oscilloscopeIntervals.set(nodeId, interval);
}

// Stop oscilloscope visualization for a node
function stopOscilloscopeVisualization(nodeId) {
  const interval = oscilloscopeIntervals.get(nodeId);
  if (interval) {
    clearInterval(interval);
    oscilloscopeIntervals.delete(nodeId);
  }
}

// ========== End Oscilloscope Visualization ==========

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
// testAudio()
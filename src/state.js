// Global state management for Lightningbeam
// This module centralizes all global state that was previously scattered in main.js

import { deepMerge } from "./utils.js";

// Core application context
// Contains UI state, selections, tool settings, etc.
export let context = {
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
  timelineWidget: null, // Reference to TimelineWindowV2 widget for zoom controls
  config: null, // Reference to config object (set after config is initialized)
  mode: "select", // Current tool mode
  // Recording state
  isRecording: false,
  recordingTrackId: null,
  recordingClipId: null,
  playPauseButton: null, // Reference to play/pause button for updating appearance
};

// Application configuration
// Contains settings, shortcuts, file properties, etc.
export let config = {
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
    selectNone: "<mod>A",
    group: "<mod>g",
    addLayer: "<mod>l",
    addAudioTrack: "<mod>t",
    addKeyframe: "F6",
    addBlankKeyframe: "F7",
    zoomIn: "<mod>+",
    zoomOut: "<mod>-",
    resetZoom: "<mod>0",
    nextLayout: "<mod>Tab",
    previousLayout: "<mod><shift>Tab",
  },
  fileWidth: 800,
  fileHeight: 600,
  framerate: 24,
  bpm: 120,
  timeSignature: { numerator: 4, denominator: 4 },
  recentFiles: [],
  scrollSpeed: 1,
  debug: false,
  reopenLastSession: false,
  lastImportFilterIndex: 0,  // Index of last used filter in import dialog (0=Image, 1=Audio, 2=Lightningbeam)
  audioBufferSize: 256,  // Audio buffer size in frames (128, 256, 512, 1024, etc. - requires restart)
  minClipDuration: 0.1,  // Minimum clip duration in seconds when trimming
  // Layout settings
  currentLayout: "animation",  // Current active layout key
  defaultLayout: "animation",  // Default layout for new files
  showStartScreen: false,  // Show layout picker on startup (disabled for now)
  restoreLayoutFromFile: false,  // Restore layout when opening files
  customLayouts: []  // User-saved custom layouts
};

// Object pointer registry
// Maps UUIDs to object instances for quick lookup
export let pointerList = {};

// Undo/redo state tracking
// Stores initial property values when starting an action
export let startProps = {};

// Helper function to get keyboard shortcut in platform format
export function getShortcut(shortcut) {
  if (!(shortcut in config.shortcuts)) return undefined;

  let shortcutValue = config.shortcuts[shortcut].replace("<mod>", "CmdOrCtrl+");
  const key = shortcutValue.slice(-1);

  // If the last character is uppercase, prepend "Shift+" to it
  return key === key.toUpperCase() && key !== key.toLowerCase()
    ? shortcutValue.replace(key, `Shift+${key}`)
    : shortcutValue.replace("++", "+Shift+="); // Hardcode uppercase from = to +
}

// Configuration file management
const CONFIG_FILE_PATH = "config.json";

// Load configuration from localStorage
export async function loadConfig() {
  try {
    const configData = localStorage.getItem("lightningbeamConfig") || "{}";
    const loaded = JSON.parse(configData);

    // Merge loaded config with defaults
    Object.assign(config, deepMerge({ ...config }, loaded));

    // Ensure recentFiles is always an array (fix legacy string format)
    let needsResave = false;
    if (typeof config.recentFiles === 'string') {
      config.recentFiles = config.recentFiles.split(',').filter(f => f.length > 0);
      needsResave = true;
    } else if (!Array.isArray(config.recentFiles)) {
      config.recentFiles = [];
      needsResave = true;
    }

    // Make config accessible to widgets via context
    context.config = config;

    console.log('[loadConfig] Loaded config.recentFiles:', config.recentFiles);

    // Re-save config if we had to fix the format
    if (needsResave) {
      console.log('[loadConfig] Re-saving config to fix array format');
      await saveConfig();
    }

    return config;
  } catch (error) {
    console.log("Error loading config, using defaults:", error);
    context.config = config;
    return config;
  }
}

// Save configuration to localStorage
export async function saveConfig() {
  try {
    localStorage.setItem(
      "lightningbeamConfig",
      JSON.stringify(config, null, 2),
    );
  } catch (error) {
    console.error("Error saving config:", error);
  }
}

// Add a file to recent files list
export async function addRecentFile(filePath) {
  config.recentFiles = [
    filePath,
    ...config.recentFiles.filter(file => file !== filePath)
  ].slice(0, 10);
  console.log('[addRecentFile] Added file, recentFiles now:', config.recentFiles);
  await saveConfig();
}

// Utility to reset pointer list (useful for testing)
export function clearPointerList() {
  pointerList = {};
}

// Utility to reset start props (useful for testing)
export function clearStartProps() {
  startProps = {};
}

// Helper to register an object in the pointer list
export function registerObject(uuid, object) {
  pointerList[uuid] = object;
}

// Helper to unregister an object from the pointer list
export function unregisterObject(uuid) {
  delete pointerList[uuid];
}

// Helper to get an object from the pointer list
export function getObject(uuid) {
  return pointerList[uuid];
}

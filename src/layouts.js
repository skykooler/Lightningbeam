// Layout definitions for Lightningbeam
// Each layout defines a workspace preset optimized for different workflows

/**
 * Layout Schema:
 * {
 *   name: string,
 *   description: string,
 *   layout: {
 *     type: "pane" | "horizontal-grid" | "vertical-grid",
 *     name?: string,  // for type="pane"
 *     percent?: number,  // for grid types (split percentage)
 *     children?: [LayoutNode, LayoutNode]  // for grid types
 *   }
 * }
 */

export const defaultLayouts = {
  animation: {
    name: "Animation",
    description: "Drawing tools, timeline, and layers front and center",
    layout: {
      type: "horizontal-grid",
      percent: 10,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "vertical-grid",
          percent: 70,
          children: [
            {
              type: "vertical-grid",
              percent: 30,
              children: [
                { type: "pane", name: "timeline" },
                { type: "pane", name: "stage" }
              ]
            },
            { type: "pane", name: "infopanel" }
          ]
        }
      ]
    }
  },

  videoEditing: {
    name: "Video Editing",
    description: "Clip timeline, source monitor, and effects panel",
    layout: {
      type: "vertical-grid",
      percent: 10,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "vertical-grid",
          percent: 65,
          children: [
            {
              type: "horizontal-grid",
              percent: 50,
              children: [
                { type: "pane", name: "stage" },
                { type: "pane", name: "infopanel" }
              ]
            },
            { type: "pane", name: "timeline" }
          ]
        }
      ]
    }
  },

  audioDaw: {
    name: "Audio/DAW",
    description: "Audio tracks prominent with mixer, node editor, and preset browser",
    layout: {
      type: "horizontal-grid",
      percent: 75,
      children: [
        {
          type: "vertical-grid",
          percent: 50,
          children: [
            { type: "pane", name: "timeline" },
            { type: "pane", name: "nodeEditor"}
          ]
        },
        { type: "pane", name: "presetBrowser" }
      ]
    }
  },

  scripting: {
    name: "Scripting",
    description: "Code editor, object hierarchy, and console",
    layout: {
      type: "vertical-grid",
      percent: 10,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "horizontal-grid",
          percent: 70,
          children: [
            {
              type: "vertical-grid",
              percent: 50,
              children: [
                { type: "pane", name: "stage" },
                { type: "pane", name: "timeline" }
              ]
            },
            {
              type: "vertical-grid",
              percent: 50,
              children: [
                { type: "pane", name: "infopanel" },
                { type: "pane", name: "outlineer" }
              ]
            }
          ]
        }
      ]
    }
  },

  rigging: {
    name: "Rigging",
    description: "Viewport focused with bone controls and weight painting",
    layout: {
      type: "vertical-grid",
      percent: 10,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "horizontal-grid",
          percent: 75,
          children: [
            { type: "pane", name: "stage" },
            {
              type: "vertical-grid",
              percent: 50,
              children: [
                { type: "pane", name: "infopanel" },
                { type: "pane", name: "timeline" }
              ]
            }
          ]
        }
      ]
    }
  },

  threeD: {
    name: "3D",
    description: "3D viewport, camera controls, and lighting panel",
    layout: {
      type: "vertical-grid",
      percent: 10,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "horizontal-grid",
          percent: 70,
          children: [
            {
              type: "vertical-grid",
              percent: 70,
              children: [
                { type: "pane", name: "stage" },
                { type: "pane", name: "timeline" }
              ]
            },
            { type: "pane", name: "infopanel" }
          ]
        }
      ]
    }
  },

  drawingPainting: {
    name: "Drawing/Painting",
    description: "Minimal UI - just canvas and drawing tools",
    layout: {
      type: "vertical-grid",
      percent: 8,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "horizontal-grid",
          percent: 85,
          children: [
            { type: "pane", name: "stage" },
            {
              type: "vertical-grid",
              percent: 70,
              children: [
                { type: "pane", name: "infopanel" },
                { type: "pane", name: "timeline" }
              ]
            }
          ]
        }
      ]
    }
  },

  shaderEditor: {
    name: "Shader Editor",
    description: "Split between viewport preview and code editor",
    layout: {
      type: "vertical-grid",
      percent: 10,
      children: [
        { type: "pane", name: "toolbar" },
        {
          type: "horizontal-grid",
          percent: 50,
          children: [
            { type: "pane", name: "stage" },
            {
              type: "vertical-grid",
              percent: 60,
              children: [
                { type: "pane", name: "infopanel" },
                { type: "pane", name: "timeline" }
              ]
            }
          ]
        }
      ]
    }
  }
};

// Get all layout names
export function getLayoutNames() {
  return Object.keys(defaultLayouts);
}

// Get a layout by key
export function getLayout(key) {
  return defaultLayouts[key];
}

// Get a layout by name
export function getLayoutByName(name) {
  for (const [key, layout] of Object.entries(defaultLayouts)) {
    if (layout.name === name) {
      return layout;
    }
  }
  return null;
}

// Check if a layout exists
export function layoutExists(key) {
  return key in defaultLayouts;
}

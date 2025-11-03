// Layout Manager - Handles layout serialization and loading
import { getLayout, getLayoutByName } from "./layouts.js";
import { config } from "./state.js";

/**
 * Builds a UI layout from a layout definition using the same approach as DOMContentLoaded
 * @param {HTMLElement} rootElement - The root container element
 * @param {Object} layoutDef - Layout definition object
 * @param {Object} panes - Panes registry with pane functions
 * @param {Function} createPane - Function to create a pane element
 * @param {Function} splitPane - Function to create a split pane
 */
export function buildLayout(rootElement, layoutDef, panes, createPane, splitPane) {
  if (!layoutDef || !layoutDef.layout) {
    throw new Error("Invalid layout definition");
  }

  // Start by creating the first pane and adding it to root
  const firstPane = buildLayoutNode(layoutDef.layout, panes, createPane);
  rootElement.appendChild(firstPane);

  // Then recursively split it according to the layout definition
  splitLayoutNode(rootElement, layoutDef.layout, panes, createPane, splitPane);
}

/**
 * Creates a pane element for a leaf node (doesn't split anything, just creates the pane)
 * @private
 */
function buildLayoutNode(node, panes, createPane) {
  if (node.type === "pane") {
    if (!node.name || !panes[node.name]) {
      console.warn(`Pane "${node.name}" not found, using placeholder`);
      return createPlaceholderPane(node.name);
    }
    return createPane(panes[node.name]);
  }

  // For grid nodes, find the leftmost/topmost leaf pane
  if (node.type === "horizontal-grid" || node.type === "vertical-grid") {
    return buildLayoutNode(node.children[0], panes, createPane);
  }

  throw new Error(`Unknown node type: ${node.type}`);
}

/**
 * Recursively splits panes according to the layout definition
 * @private
 */
function splitLayoutNode(container, node, panes, createPane, splitPane) {
  if (node.type === "pane") {
    // Leaf node - nothing to split
    return;
  }

  if (node.type === "horizontal-grid" || node.type === "vertical-grid") {
    const isHorizontal = node.type === "horizontal-grid";
    const percent = node.percent || 50;

    // Build the second child pane
    const child2Pane = buildLayoutNode(node.children[1], panes, createPane);

    // Split the container
    const [container1, container2] = splitPane(container, percent, isHorizontal, child2Pane);

    // Recursively split both children
    splitLayoutNode(container1, node.children[0], panes, createPane, splitPane);
    splitLayoutNode(container2, node.children[1], panes, createPane, splitPane);
  }
}

/**
 * Creates a placeholder pane for missing pane types
 * @private
 */
function createPlaceholderPane(paneName) {
  const div = document.createElement("div");
  div.className = "pane panecontainer";
  div.style.display = "flex";
  div.style.alignItems = "center";
  div.style.justifyContent = "center";
  div.style.flexDirection = "column";
  div.style.color = "#888";
  div.style.fontSize = "14px";

  const title = document.createElement("div");
  title.textContent = paneName || "Unknown Pane";
  title.style.fontSize = "18px";
  title.style.marginBottom = "8px";

  const message = document.createElement("div");
  message.textContent = "Coming Soon";

  div.appendChild(title);
  div.appendChild(message);

  return div;
}

/**
 * Serializes the current layout to a layout definition
 * @param {HTMLElement} rootElement - The root element containing the layout
 * @returns {Object} Layout definition object
 */
export function serializeLayout(rootElement) {
  if (!rootElement.firstChild) {
    throw new Error("No layout to serialize");
  }

  console.log('[serializeLayout] rootElement has', rootElement.children.length, 'children:');
  for (let i = 0; i < rootElement.children.length; i++) {
    console.log(`  [${i}]:`, rootElement.children[i].className);
  }

  let layoutNode;

  // Check if rootElement itself acts as a grid (has 2 children from a split)
  if (rootElement.children.length === 2) {
    // rootElement is acting as a grid container
    // Check if it has grid attributes
    const isHorizontal = rootElement.classList.contains("horizontal-grid");
    const isVertical = rootElement.classList.contains("vertical-grid");
    const percent = parseFloat(rootElement.getAttribute("lb-percent")) || 50;

    if (isHorizontal || isVertical) {
      console.log('[serializeLayout] rootElement is a grid, serializing both children');
      layoutNode = {
        type: isHorizontal ? "horizontal-grid" : "vertical-grid",
        percent: percent,
        children: [
          serializeLayoutNode(rootElement.children[0]),
          serializeLayoutNode(rootElement.children[1])
        ]
      };
    } else {
      // No grid classes, but has 2 children - this shouldn't happen but handle it
      console.warn('[serializeLayout] rootElement has 2 children but no grid classes, serializing first child only');
      layoutNode = serializeLayoutNode(rootElement.firstChild);
    }
  } else {
    // Single child, serialize it directly
    console.log('[serializeLayout] Starting from element:', rootElement.firstChild.className);
    layoutNode = serializeLayoutNode(rootElement.firstChild);
  }

  console.log('[serializeLayout] Serialized layout:', JSON.stringify(layoutNode, null, 2));

  return {
    name: "Custom Layout",
    description: "User-created layout",
    layout: layoutNode
  };
}

/**
 * Recursively serializes a layout node
 * @private
 */
function serializeLayoutNode(element, depth = 0) {
  const indent = '  '.repeat(depth);
  console.log(`${indent}[serializeLayoutNode depth=${depth}] element:`, element.className, 'children:', element.children.length);

  if (!element) {
    throw new Error("Cannot serialize null element");
  }

  // Check if this is a pane (has data-pane-name attribute)
  // This check must come first, as panes may also have grid classes for internal layout
  if (element.hasAttribute("data-pane-name")) {
    // The data-pane-name is kebab-case, but we need to save the camelCase key
    // that matches the panes object keys, not the name property
    const dataName = element.getAttribute("data-pane-name");

    // Convert kebab-case to camelCase (e.g., "timeline-v2" -> "timelineV2")
    const camelCaseName = dataName.replace(/-([a-z0-9])/g, (g) => g[1].toUpperCase());

    console.log(`${indent}  -> Found pane: ${camelCaseName}`);

    return {
      type: "pane",
      name: camelCaseName
    };
  }

  // Check if this is a grid (split pane structure)
  if (element.classList.contains("horizontal-grid") || element.classList.contains("vertical-grid")) {
    const isHorizontal = element.classList.contains("horizontal-grid");
    const percent = parseFloat(element.getAttribute("lb-percent")) || 50;

    console.log(`${indent}  -> Found ${isHorizontal ? 'horizontal' : 'vertical'} grid with ${percent}% split`);

    if (element.children.length !== 2) {
      throw new Error("Grid must have exactly 2 children");
    }

    return {
      type: isHorizontal ? "horizontal-grid" : "vertical-grid",
      percent: percent,
      children: [
        serializeLayoutNode(element.children[0], depth + 1),
        serializeLayoutNode(element.children[1], depth + 1)
      ]
    };
  }

  // Check if this is a panecontainer wrapper - recurse into it
  if (element.classList.contains("panecontainer")) {
    console.log(`${indent}  -> Found panecontainer, recursing into child`);
    if (element.children.length === 1) {
      return serializeLayoutNode(element.children[0], depth);
    }
  }

  // If element has only one child, recurse into it
  if (element.children.length === 1) {
    console.log(`${indent}  -> Element has 1 child, recursing`);
    return serializeLayoutNode(element.children[0], depth);
  }

  throw new Error(`Cannot serialize element: ${element.className}`);
}

/**
 * Loads a layout by key or name
 * @param {string} keyOrName - Layout key or name
 * @returns {Object|null} Layout definition or null if not found
 */
export function loadLayoutByKeyOrName(keyOrName) {
  // First try as a key
  let layout = getLayout(keyOrName);

  // If not found, try as a name
  if (!layout) {
    layout = getLayoutByName(keyOrName);
  }

  // If still not found, check custom layouts
  if (!layout && config.customLayouts) {
    layout = config.customLayouts.find(l => l.name === keyOrName);
  }

  return layout;
}

/**
 * Saves a custom layout
 * @param {string} name - Name for the custom layout
 * @param {Object} layoutDef - Layout definition
 */
export function saveCustomLayout(name, layoutDef) {
  if (!config.customLayouts) {
    config.customLayouts = [];
  }

  // Check if layout with this name already exists
  const existingIndex = config.customLayouts.findIndex(l => l.name === name);

  const customLayout = {
    ...layoutDef,
    name: name,
    custom: true
  };

  if (existingIndex >= 0) {
    // Update existing
    config.customLayouts[existingIndex] = customLayout;
  } else {
    // Add new
    config.customLayouts.push(customLayout);
  }
}

/**
 * Deletes a custom layout
 * @param {string} name - Name of the layout to delete
 * @returns {boolean} True if deleted, false if not found
 */
export function deleteCustomLayout(name) {
  if (!config.customLayouts) {
    return false;
  }

  const index = config.customLayouts.findIndex(l => l.name === name);
  if (index >= 0) {
    config.customLayouts.splice(index, 1);
    return true;
  }
  return false;
}

/**
 * Canvas interaction utilities for UI testing
 */

/**
 * Click at specific coordinates on the canvas
 * @param {number} x - X coordinate relative to canvas
 * @param {number} y - Y coordinate relative to canvas
 */
export async function clickCanvas(x, y) {
  await browser.clickCanvas(x, y);
  await browser.pause(100); // Wait for render
}

/**
 * Drag from one point to another on the canvas
 * @param {number} fromX - Starting X coordinate
 * @param {number} fromY - Starting Y coordinate
 * @param {number} toX - Ending X coordinate
 * @param {number} toY - Ending Y coordinate
 */
export async function dragCanvas(fromX, fromY, toX, toY) {
  await browser.dragCanvas(fromX, fromY, toX, toY);
  await browser.pause(200); // Wait for render
}

/**
 * Draw a rectangle on the canvas
 * @param {number} x - Top-left X coordinate
 * @param {number} y - Top-left Y coordinate
 * @param {number} width - Rectangle width
 * @param {number} height - Rectangle height
 * @param {boolean} filled - Whether to fill the shape (default: true)
 */
export async function drawRectangle(x, y, width, height, filled = true) {
  // Select the rectangle tool
  await selectTool('rectangle');

  // Set fill option
  await browser.execute((filled) => {
    if (window.context) {
      window.context.fillShape = filled;
    }
  }, filled);

  // Draw by dragging from start to end point
  await dragCanvas(x, y, x + width, y + height);

  // Wait for shape to be created
  await browser.pause(300);
}

/**
 * Draw an ellipse on the canvas
 * @param {number} x - Top-left X coordinate
 * @param {number} y - Top-left Y coordinate
 * @param {number} width - Ellipse width
 * @param {number} height - Ellipse height
 * @param {boolean} filled - Whether to fill the shape (default: true)
 */
export async function drawEllipse(x, y, width, height, filled = true) {
  // Select the ellipse tool
  await selectTool('ellipse');

  // Set fill option
  await browser.execute((filled) => {
    if (window.context) {
      window.context.fillShape = filled;
    }
  }, filled);

  // Draw by dragging from start to end point
  await dragCanvas(x, y, x + width, y + height);

  // Wait for shape to be created
  await browser.pause(300);
}

/**
 * Select a tool from the toolbar
 * @param {string} toolName - Name of the tool ('rectangle', 'ellipse', 'brush', etc.)
 */
export async function selectTool(toolName) {
  const toolButton = await browser.$(`[data-tool="${toolName}"]`);
  await toolButton.click();
  await browser.pause(100);
}

/**
 * Select multiple shapes by dragging a selection box over them
 * @param {Array<{x: number, y: number}>} points - Array of points representing shapes to select
 */
export async function selectMultipleShapes(points) {
  // First, make sure we're in select mode
  await selectTool('select');

  // Calculate bounding box that encompasses all points
  const minX = Math.min(...points.map(p => p.x)) - 10;
  const minY = Math.min(...points.map(p => p.y)) - 10;
  const maxX = Math.max(...points.map(p => p.x)) + 10;
  const maxY = Math.max(...points.map(p => p.y)) + 10;

  // Drag a selection box from top-left to bottom-right
  await dragCanvas(minX, minY, maxX, maxY);
  await browser.pause(200);
}

/**
 * Use keyboard shortcut / menu action
 * Since Tauri menu shortcuts don't reach the browser, we invoke actions directly
 * @param {string} key - Key to press (e.g., 'g' for group)
 * @param {boolean} withCtrl - Whether to hold Ctrl (ignored, kept for compatibility)
 */
export async function useKeyboardShortcut(key, withCtrl = true) {
  if (key === 'g') {
    // Call group action directly without serializing the whole function
    await browser.execute('window.actions.group.create()');
  }

  await browser.pause(300); // Give time for the action to process
}

/**
 * Get pixel color at specific coordinates (requires canvas access)
 * @param {number} x - X coordinate
 * @param {number} y - Y coordinate
 * @returns {Promise<string>} Color in hex format
 */
export async function getPixelColor(x, y) {
  const result = await browser.execute(function(x, y) {
    const canvas = document.querySelector('canvas.stage');
    if (!canvas) return null;

    const ctx = canvas.getContext('2d');
    const imageData = ctx.getImageData(x, y, 1, 1);
    const data = imageData.data;

    // Convert to hex
    const r = data[0].toString(16).padStart(2, '0');
    const g = data[1].toString(16).padStart(2, '0');
    const b = data[2].toString(16).padStart(2, '0');

    return `#${r}${g}${b}`;
  }, x, y);

  return result;
}

/**
 * Check if a shape exists at given coordinates by checking if pixel is not background
 * @param {number} x - X coordinate
 * @param {number} y - Y coordinate
 * @param {string} backgroundColor - Expected background color (default white)
 * @returns {Promise<boolean>}
 */
export async function hasShapeAt(x, y, backgroundColor = '#ffffff') {
  const color = await getPixelColor(x, y);
  return color && color.toLowerCase() !== backgroundColor.toLowerCase();
}

/**
 * Double-click at specific coordinates on the canvas (to enter group editing mode)
 * @param {number} x - X coordinate relative to canvas
 * @param {number} y - Y coordinate relative to canvas
 */
export async function doubleClickCanvas(x, y) {
  const canvas = await browser.$('canvas.stage');
  const location = await canvas.getLocation();

  // Perform double-click using performActions
  await browser.performActions([{
    type: 'pointer',
    id: 'mouse',
    parameters: { pointerType: 'mouse' },
    actions: [
      { type: 'pointerMove', duration: 0, x: location.x + x, y: location.y + y },
      { type: 'pointerDown', button: 0 },
      { type: 'pointerUp', button: 0 },
      { type: 'pause', duration: 50 },
      { type: 'pointerDown', button: 0 },
      { type: 'pointerUp', button: 0 }
    ]
  }]);

  await browser.pause(300); // Wait for group to be entered
}

/**
 * Set the timeline playhead to a specific time
 * @param {number} time - Time in seconds
 */
export async function setPlayheadTime(time) {
  await browser.execute(function(timeValue) {
    if (window.context && window.context.activeObject) {
      window.context.activeObject.currentTime = timeValue;
      // Update timeline widget if it exists
      if (window.context.timelineWidget && window.context.timelineWidget.timelineState) {
        window.context.timelineWidget.timelineState.currentTime = timeValue;
      }
    }
  }, time);
  await browser.pause(100);
}

/**
 * Get the current playhead time
 * @returns {Promise<number>} Current time in seconds
 */
export async function getPlayheadTime() {
  return await browser.execute(function() {
    if (window.context && window.context.activeObject) {
      return window.context.activeObject.currentTime;
    }
    return 0;
  });
}

/**
 * Add a keyframe at the current playhead position for selected shapes/objects
 */
export async function addKeyframe() {
  await browser.execute('window.addKeyframeAtPlayhead && window.addKeyframeAtPlayhead()');
  await browser.pause(200);
}

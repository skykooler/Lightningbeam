// Root object initialization
// Creates and configures the root GraphicsObject and context properties

import { context } from '../state.js';
import { GraphicsObject } from './graphics-object.js';

/**
 * Creates and initializes the root GraphicsObject.
 * Sets up context properties for active object and layer access.
 *
 * @returns {GraphicsObject} The root graphics object
 */
export function createRoot() {
  const root = new GraphicsObject("root");

  // Define getter for active object (top of stack)
  Object.defineProperty(context, "activeObject", {
    get: function () {
      return this.objectStack.at(-1);
    },
  });

  // Define getter for active layer (active layer of top object)
  Object.defineProperty(context, "activeLayer", {
    get: function () {
      return this.objectStack.at(-1).activeLayer;
    }
  });

  // Initialize object stack with root
  context.objectStack = [root];

  return root;
}

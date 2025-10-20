/**
 * Custom assertion helpers for Lightningbeam UI tests
 */

import { getPixelColor, hasShapeAt } from './canvas.js';
import { expect } from '@wdio/globals';

/**
 * Assert that a shape exists at the given coordinates
 * @param {number} x - X coordinate
 * @param {number} y - Y coordinate
 * @param {string} message - Custom error message
 */
export async function assertShapeExists(x, y, message = 'Expected shape to exist at coordinates') {
  const shapeExists = await hasShapeAt(x, y);
  expect(shapeExists).toBe(true, `${message} (${x}, ${y})`);
}

/**
 * Assert that no shape exists at the given coordinates
 * @param {number} x - X coordinate
 * @param {number} y - Y coordinate
 * @param {string} message - Custom error message
 */
export async function assertNoShapeAt(x, y, message = 'Expected no shape at coordinates') {
  const shapeExists = await hasShapeAt(x, y);
  expect(shapeExists).toBe(false, `${message} (${x}, ${y})`);
}

/**
 * Assert that a pixel has a specific color
 * @param {number} x - X coordinate
 * @param {number} y - Y coordinate
 * @param {string} expectedColor - Expected color in hex format
 * @param {string} message - Custom error message
 */
export async function assertPixelColor(x, y, expectedColor, message = 'Expected pixel color to match') {
  const actualColor = await getPixelColor(x, y);
  expect(actualColor.toLowerCase()).toBe(expectedColor.toLowerCase(),
    `${message}. Expected ${expectedColor}, got ${actualColor} at (${x}, ${y})`);
}

/**
 * Assert that a color is approximately equal to another (with tolerance)
 * Useful for anti-aliasing and rendering differences
 * @param {string} color1 - First color in hex format
 * @param {string} color2 - Second color in hex format
 * @param {number} tolerance - Tolerance per channel (0-255)
 */
export function assertColorApproximately(color1, color2, tolerance = 10) {
  const rgb1 = hexToRgb(color1);
  const rgb2 = hexToRgb(color2);

  const rDiff = Math.abs(rgb1.r - rgb2.r);
  const gDiff = Math.abs(rgb1.g - rgb2.g);
  const bDiff = Math.abs(rgb1.b - rgb2.b);

  expect(rDiff).toBeLessThanOrEqual(tolerance, `Red channel difference ${rDiff} exceeds tolerance ${tolerance}`);
  expect(gDiff).toBeLessThanOrEqual(tolerance, `Green channel difference ${gDiff} exceeds tolerance ${tolerance}`);
  expect(bDiff).toBeLessThanOrEqual(tolerance, `Blue channel difference ${bDiff} exceeds tolerance ${tolerance}`);
}

/**
 * Convert hex color to RGB object
 * @param {string} hex - Hex color string
 * @returns {{r: number, g: number, b: number}}
 */
function hexToRgb(hex) {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  return result ? {
    r: parseInt(result[1], 16),
    g: parseInt(result[2], 16),
    b: parseInt(result[3], 16)
  } : null;
}

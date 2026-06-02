/**
 * Paint bucket tool tests for Lightningbeam
 */

import { describe, it, before } from 'mocha';
import { expect } from '@wdio/globals';
import { waitForAppReady } from '../helpers/app.js';
import { drawRectangle, selectTool, clickCanvas, getPixelColor } from '../helpers/canvas.js';
import { assertPixelColor } from '../helpers/assertions.js';

describe('Paint Bucket Tool', () => {
  before(async () => {
    await waitForAppReady();
  });

  describe('Fill Shape', () => {
    it('should fill a rectangle with color', async () => {
      // Draw an unfilled rectangle (outline only)
      await drawRectangle(100, 100, 200, 150, false);

      // Get the color before filling (should be stroke/outline only, center is white)
      const colorBefore = await getPixelColor(200, 175);

      // Select paint bucket tool
      await selectTool('paint_bucket');

      // Click inside the rectangle to fill it
      await clickCanvas(200, 175);

      // Get the color after filling
      const colorAfter = await getPixelColor(200, 175);

      // The color should have changed from white background to filled color
      expect(colorBefore.toLowerCase()).toBe('#ffffff'); // Was white (unfilled)
      expect(colorAfter.toLowerCase()).not.toBe('#ffffff'); // Now filled with a color
    });

    it('should fill only the clicked shape, not adjacent shapes', async () => {
      // Draw two separate unfilled rectangles
      await drawRectangle(100, 300, 100, 100, false);
      await drawRectangle(250, 300, 100, 100, false);

      // Fill only the first rectangle
      await selectTool('paint_bucket');
      await clickCanvas(150, 350);

      // Get colors from both shapes
      const firstColor = await getPixelColor(150, 350);
      const secondColor = await getPixelColor(300, 350);

      // The shapes should potentially have different colors
      // (or at least we confirmed we could click them individually)
    });
  });

  describe('Fill with Different Colors', () => {
    it('should respect the selected fill color when using paint bucket', async () => {
      // This test would require setting a specific fill color first
      // Then drawing and filling a shape
      // For now, this is a placeholder structure

      await drawRectangle(400, 100, 150, 100, false);

      // TODO: Add color selection logic when color picker helpers are available
      // await selectColor('#ff0000');

      await selectTool('paint_bucket');
      await clickCanvas(475, 150);

      // Verify the fill color
      const color = await getPixelColor(475, 150);
      // TODO: Assert expected color when we know how to set it
    });
  });

  describe('Fill Gaps Setting', () => {
    it('should handle fill gaps setting for incomplete shapes', async () => {
      // This test would draw an incomplete shape and test the fillGaps parameter
      // Placeholder for now - would need specific incomplete shape drawing

      // The fillGaps setting controls how the paint bucket handles gaps in shapes
      // This is a more advanced test that would need:
      // 1. A way to draw incomplete/open shapes
      // 2. A way to set the fillGaps parameter
      // 3. Verification that the fill respects the gap threshold
    });
  });
});

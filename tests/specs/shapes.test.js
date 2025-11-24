/**
 * Shape drawing tests for Lightningbeam
 */

import { describe, it, before } from 'mocha';
import { waitForAppReady } from '../helpers/app.js';
import { drawRectangle, drawEllipse } from '../helpers/canvas.js';
import { assertShapeExists } from '../helpers/assertions.js';

describe('Shape Drawing', () => {
  before(async () => {
    await waitForAppReady();
  });

  describe('Rectangle Tool', () => {
    it('should draw a rectangle on the canvas', async () => {
      // Draw a rectangle at (100, 100) with size 200x150
      await drawRectangle(100, 100, 200, 150);

      // Verify the shape exists by checking pixels at various points
      // Check center of the rectangle
      await assertShapeExists(200, 175, 'Rectangle should be drawn at center');

      // Check edges
      await assertShapeExists(110, 110, 'Rectangle should exist at top-left');
      await assertShapeExists(290, 110, 'Rectangle should exist at top-right');
      await assertShapeExists(110, 240, 'Rectangle should exist at bottom-left');
      await assertShapeExists(290, 240, 'Rectangle should exist at bottom-right');
    });

    it('should draw multiple rectangles without interference', async () => {
      // Draw first rectangle
      await drawRectangle(50, 50, 100, 100);
      await assertShapeExists(100, 100, 'First rectangle should exist');

      // Draw second rectangle in different location
      await drawRectangle(300, 300, 100, 100);
      await assertShapeExists(350, 350, 'Second rectangle should exist');

      // Verify first rectangle still exists
      await assertShapeExists(100, 100, 'First rectangle should still exist');
    });

    it('should draw small rectangles', async () => {
      // Draw a small rectangle
      await drawRectangle(400, 100, 20, 20);
      await assertShapeExists(410, 110, 'Small rectangle should exist');
    });

    it('should draw large rectangles', async () => {
      // Draw a large rectangle (canvas is ~350px tall, so keep within bounds)
      await drawRectangle(50, 50, 400, 250);
      await assertShapeExists(250, 175, 'Large rectangle should exist at center');
    });
  });

  describe('Ellipse Tool', () => {
    it('should draw an ellipse on the canvas', async () => {
      // Draw an ellipse at (100, 100) with size 200x150
      await drawEllipse(100, 100, 200, 150);

      // Check center of the ellipse
      await assertShapeExists(200, 175, 'Ellipse should be drawn at center');
    });

    it('should draw a circle (equal width and height)', async () => {
      // Draw a circle
      await drawEllipse(300, 100, 150, 150);

      // Check center
      await assertShapeExists(375, 175, 'Circle should exist at center');
    });

    it('should draw wide ellipses', async () => {
      // Draw a wide ellipse
      await drawEllipse(50, 400, 300, 100);
      await assertShapeExists(200, 450, 'Wide ellipse should exist');
    });

    it('should draw tall ellipses', async () => {
      // Draw a tall ellipse
      await drawEllipse(500, 100, 100, 300);
      await assertShapeExists(550, 250, 'Tall ellipse should exist');
    });
  });

  describe('Mixed Shapes', () => {
    it('should draw both rectangles and ellipses in the same scene', async () => {
      // Draw a rectangle
      await drawRectangle(100, 100, 150, 100);

      // Draw an ellipse
      await drawEllipse(300, 100, 150, 100);

      // Verify both exist
      await assertShapeExists(175, 150, 'Rectangle should exist');
      await assertShapeExists(375, 150, 'Ellipse should exist');
    });
  });
});

/**
 * Group editing tests for Lightningbeam
 * Tests that shapes can be edited inside groups with correct relative positioning
 */

import { describe, it, before } from 'mocha';
import { expect } from '@wdio/globals';
import { waitForAppReady } from '../helpers/app.js';
import {
  drawRectangle,
  selectMultipleShapes,
  useKeyboardShortcut,
  doubleClickCanvas,
  clickCanvas
} from '../helpers/canvas.js';
import { assertShapeExists } from '../helpers/assertions.js';
import { verifyManually, logStep } from '../helpers/manual.js';

describe('Group Editing', () => {
  before(async () => {
    await waitForAppReady();
  });

  describe('Entering and Editing Groups', () => {
    it('should maintain shape positions when editing inside a group', async () => {
      // Draw a rectangle
      await drawRectangle(200, 200, 100, 100);

      // Verify it exists at the expected location
      await assertShapeExists(250, 250, 'Rectangle should exist at center before grouping');

      // Select it (click on the center)
      await clickCanvas(250, 250);
      await browser.pause(200);

      // Group it (even though it's just one shape)
      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      // The shape should still be visible at the same location
      await assertShapeExists(250, 250, 'Rectangle should still exist at same position after grouping');

      // Double-click on the group to enter editing mode
      await doubleClickCanvas(250, 250);

      // The shape should STILL be at the same position when editing the group
      await assertShapeExists(250, 250, 'Rectangle should remain at same position when editing group');
    });

    it('should correctly position new shapes drawn inside a group', async () => {
      // Draw a rectangle
      await drawRectangle(100, 400, 80, 80);

      // Select and group it
      await clickCanvas(140, 440);
      await browser.pause(200);
      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      // Double-click to enter group editing mode
      await doubleClickCanvas(140, 440);

      // Draw another rectangle inside the group at a specific location
      await drawRectangle(200, 400, 80, 80);

      // Verify the new shape is where we drew it
      await assertShapeExists(240, 440, 'New shape should be at the coordinates where it was drawn');

      // Verify the original shape still exists
      await assertShapeExists(140, 440, 'Original shape should still exist');
    });

    it('should handle nested group editing with correct positioning', async () => {
      // Create first group with two shapes
      await logStep('Drawing two rectangles for inner group');
      await drawRectangle(400, 100, 60, 60);
      await drawRectangle(480, 100, 60, 60);
      await selectMultipleShapes([
        { x: 430, y: 130 },
        { x: 510, y: 130 }
      ]);
      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      await verifyManually('VERIFY: Do you see two rectangles grouped together?\nClick OK if yes, Cancel if no');

      // Verify both shapes exist
      await assertShapeExists(430, 130, 'First shape should exist');
      await assertShapeExists(510, 130, 'Second shape should exist');

      // Create another shape and group everything together
      await logStep('Drawing third rectangle and creating nested group');
      await drawRectangle(400, 180, 60, 60);

      // Select both the group and the new shape by dragging a selection box
      // We need to start from well outside the shapes to avoid hitting them
      // The first group spans x=400-540, y=100-160
      // The third shape spans x=400-460, y=180-240
      await selectTool('select');
      await dragCanvas(390, 90, 550, 250); // Start from outside all shapes
      await browser.pause(200);

      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      await verifyManually('VERIFY: All three rectangles now grouped together (nested group)?\nClick OK if yes, Cancel if no');

      // Double-click to enter outer group
      await logStep('Double-clicking to enter outer group');
      await doubleClickCanvas(470, 130);
      await browser.pause(300);

      await verifyManually('VERIFY: Are we now inside the outer group?\nClick OK if yes, Cancel if no');

      // Double-click again to enter inner group
      await logStep('Double-clicking again to enter inner group');
      await doubleClickCanvas(470, 130);
      await browser.pause(300);

      await verifyManually(
        'VERIFY: Are we now inside the inner group?\n' +
        'Can you see the two original rectangles at their original positions?\n' +
        'First at (430, 130), second at (510, 130)?\n\n' +
        'Click OK if yes, Cancel if no'
      );

      // All shapes should still be at their original positions
      await assertShapeExists(430, 130, 'First shape should maintain position in nested group');
      await assertShapeExists(510, 130, 'Second shape should maintain position in nested group');
    });
  });

  describe('Mouse Coordinate Transformation', () => {
    it('should correctly translate mouse coordinates when drawing in groups', async () => {
      // Draw and group a shape
      await drawRectangle(300, 300, 50, 50);
      await clickCanvas(325, 325);
      await browser.pause(200);
      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      // Enter group
      await doubleClickCanvas(325, 325);

      // Draw shapes at precise coordinates to test mouse transformation
      await drawRectangle(350, 300, 30, 30);
      await drawRectangle(300, 350, 30, 30);
      await drawRectangle(350, 350, 30, 30);

      // Verify all shapes are at expected positions
      await assertShapeExists(365, 315, 'Shape to the right should be at correct position');
      await assertShapeExists(315, 365, 'Shape below should be at correct position');
      await assertShapeExists(365, 365, 'Shape diagonal should be at correct position');
      await assertShapeExists(325, 325, 'Original shape should still exist');
    });
  });
});

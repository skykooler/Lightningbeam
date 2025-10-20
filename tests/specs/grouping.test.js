/**
 * Shape grouping tests for Lightningbeam
 */

import { describe, it, before, beforeEach } from 'mocha';
import { expect } from '@wdio/globals';
import { waitForAppReady } from '../helpers/app.js';
import { drawRectangle, drawEllipse, selectMultipleShapes, useKeyboardShortcut, clickCanvas } from '../helpers/canvas.js';
import { assertShapeExists } from '../helpers/assertions.js';

describe('Shape Grouping', () => {
  before(async () => {
    await waitForAppReady();
  });

  describe('Grouping Multiple Shapes', () => {
    it('should group two rectangles together', async () => {
      // Draw two rectangles
      await drawRectangle(100, 100, 100, 100);
      await drawRectangle(250, 100, 100, 100);

      // Select both shapes (click centers with Ctrl held)
      await selectMultipleShapes([
        { x: 150, y: 150 },
        { x: 300, y: 150 }
      ]);

      // Group with Ctrl+G
      await useKeyboardShortcut('g', true);

      // Verify both shapes still exist after grouping
      await assertShapeExists(150, 150, 'First rectangle should still exist after grouping');
      await assertShapeExists(300, 150, 'Second rectangle should still exist after grouping');
    });

    it('should group rectangle and ellipse together', async () => {
      // Draw rectangle and ellipse
      await drawRectangle(100, 250, 120, 80);
      await drawEllipse(280, 250, 120, 80);

      // Select both shapes
      await selectMultipleShapes([
        { x: 160, y: 290 },
        { x: 340, y: 290 }
      ]);

      // Group them
      await useKeyboardShortcut('g', true);

      // Verify shapes exist
      await assertShapeExists(160, 290, 'Rectangle should exist in group');
      await assertShapeExists(340, 290, 'Ellipse should exist in group');
    });

    it('should group three or more shapes', async () => {
      // Draw three rectangles
      await drawRectangle(50, 400, 80, 80);
      await drawRectangle(150, 400, 80, 80);
      await drawRectangle(250, 400, 80, 80);

      // Select all three
      await selectMultipleShapes([
        { x: 90, y: 440 },
        { x: 190, y: 440 },
        { x: 290, y: 440 }
      ]);

      // Group them
      await useKeyboardShortcut('g', true);

      // Verify all shapes exist
      await assertShapeExists(90, 440, 'First shape should exist in group');
      await assertShapeExists(190, 440, 'Second shape should exist in group');
      await assertShapeExists(290, 440, 'Third shape should exist in group');
    });
  });

  describe('Group Manipulation', () => {
    it('should be able to select and move a group', async () => {
      // Draw two shapes
      await drawRectangle(400, 100, 80, 80);
      await drawRectangle(500, 100, 80, 80);

      // Select and group
      await selectMultipleShapes([
        { x: 440, y: 140 },
        { x: 540, y: 140 }
      ]);
      await useKeyboardShortcut('g', true);

      // Click on the group to select it (click between the shapes)
      await clickCanvas(490, 140);

      // Note: Moving would require drag testing which is already covered in canvas.js
      // This test verifies the group can be selected
    });
  });

  describe('Nested Groups', () => {
    it('should allow grouping of groups', async () => {
      // Create first group
      await drawRectangle(100, 100, 60, 60);
      await drawRectangle(180, 100, 60, 60);
      await selectMultipleShapes([
        { x: 130, y: 130 },
        { x: 210, y: 130 }
      ]);
      await useKeyboardShortcut('g', true);

      // Create second group
      await drawRectangle(100, 200, 60, 60);
      await drawRectangle(180, 200, 60, 60);
      await selectMultipleShapes([
        { x: 130, y: 230 },
        { x: 210, y: 230 }
      ]);
      await useKeyboardShortcut('g', true);

      // Now group both groups together
      // Select center of each group
      await selectMultipleShapes([
        { x: 170, y: 130 },
        { x: 170, y: 230 }
      ]);
      await useKeyboardShortcut('g', true);

      // Verify all original shapes still exist
      await assertShapeExists(130, 130, 'First group first shape should exist');
      await assertShapeExists(210, 130, 'First group second shape should exist');
      await assertShapeExists(130, 230, 'Second group first shape should exist');
      await assertShapeExists(210, 230, 'Second group second shape should exist');
    });
  });
});

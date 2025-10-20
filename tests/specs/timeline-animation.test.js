/**
 * Timeline animation tests for Lightningbeam
 * Tests shape and object animations across keyframes
 */

import { describe, it, before } from 'mocha';
import { expect } from '@wdio/globals';
import { waitForAppReady } from '../helpers/app.js';
import {
  drawRectangle,
  drawEllipse,
  clickCanvas,
  dragCanvas,
  selectMultipleShapes,
  useKeyboardShortcut,
  setPlayheadTime,
  getPlayheadTime,
  addKeyframe,
  getPixelColor
} from '../helpers/canvas.js';
import { assertShapeExists } from '../helpers/assertions.js';

describe('Timeline Animation', () => {
  before(async () => {
    await waitForAppReady();
  });

  describe('Shape Keyframe Animation', () => {
    it('should animate shape position across keyframes', async () => {
      // Draw a rectangle at frame 1 (time 0)
      await drawRectangle(100, 100, 100, 100);

      // Select the shape by dragging a selection box over it
      await selectMultipleShapes([{ x: 150, y: 150 }]);
      await browser.pause(200);

      // Verify it exists at original position
      await assertShapeExists(150, 150, 'Shape should exist at frame 1');

      // Move to frame 10 (time in seconds, assuming 30fps: frame 10 = 10/30 ≈ 0.333s)
      await setPlayheadTime(0.333);

      // Add a keyframe at this position
      await addKeyframe();
      await browser.pause(200);

      // Shape is selected, so dragging from its center will move it
      await dragCanvas(150, 150, 250, 150);
      await browser.pause(300);

      // At frame 10, shape should be at the new position (moved 100px to the right)
      await assertShapeExists(250, 150, 'Shape should be at new position at frame 10');

      // Go back to frame 1
      await setPlayheadTime(0);
      await browser.pause(200);

      // Shape should be at original position
      await assertShapeExists(150, 150, 'Shape should be at original position at frame 1');

      // Go to middle frame (frame 5, time ≈ 0.166s)
      await setPlayheadTime(0.166);
      await browser.pause(200);

      // Shape should be interpolated between the two positions
      // At frame 5 (halfway), shape should be around x=200 (halfway between 150 and 250)
      await assertShapeExists(200, 150, 'Shape should be interpolated at frame 5');
    });

    it('should modify shape edges when dragging edge of unselected shape', async () => {
      // Draw a rectangle
      await drawRectangle(400, 100, 100, 100);

      // Get color at center
      const centerColorBefore = await getPixelColor(450, 150);

      // WITHOUT selecting the shape, drag the right edge
      // The right edge is at x=500, so drag from there
      await dragCanvas(500, 150, 550, 150);
      await browser.pause(300);

      // The shape should now be modified - it's been curved/stretched
      // The original center should still have the shape
      await assertShapeExists(450, 150, 'Center should still have shape');

      // And there should be shape data extended to the right
      const rightColorAfter = await getPixelColor(525, 150);
      // This should have color (not white) since we dragged the edge there
      expect(rightColorAfter.toLowerCase()).not.toBe('#ffffff');
    });

    it('should handle multiple keyframes on the same shape', async () => {
      // Draw a shape
      await drawRectangle(100, 300, 80, 80);

      // Select it
      await selectMultipleShapes([{ x: 140, y: 340 }]);
      await browser.pause(200);

      // Keyframe 1: time 0 (original position at x=140, y=340)

      // Keyframe 2: time 0.333 (move right)
      await setPlayheadTime(0.333);
      await addKeyframe();
      await browser.pause(200);
      // Shape should still be selected, drag to move
      await dragCanvas(140, 340, 200, 340);
      await browser.pause(300);

      // Keyframe 3: time 0.666 (move down but stay within canvas)
      await setPlayheadTime(0.666);
      await addKeyframe();
      await browser.pause(200);
      // Drag to move down (y=380 instead of 400 to stay in canvas)
      await dragCanvas(200, 340, 200, 380);
      await browser.pause(300);

      // Verify positions at each keyframe
      await setPlayheadTime(0);
      await browser.pause(200);
      await assertShapeExists(140, 340, 'Shape at keyframe 1 (x=140, y=340)');

      await setPlayheadTime(0.333);
      await browser.pause(200);
      await assertShapeExists(200, 340, 'Shape at keyframe 2 (x=200, y=340)');

      await setPlayheadTime(0.666);
      await browser.pause(200);
      await assertShapeExists(200, 380, 'Shape at keyframe 3 (x=200, y=380)');

      // Check interpolation between keyframe 1 and 2 (at t=0.166, halfway)
      await setPlayheadTime(0.166);
      await browser.pause(200);
      await assertShapeExists(170, 340, 'Shape interpolated between kf1 and kf2');

      // Check interpolation between keyframe 2 and 3 (at t=0.5, halfway)
      await setPlayheadTime(0.5);
      await browser.pause(200);
      await assertShapeExists(200, 360, 'Shape interpolated between kf2 and kf3');
    });
  });

  describe('Group/Object Animation', () => {
    it('should animate group position across keyframes', async () => {
      // Create a group with two shapes
      await drawRectangle(300, 300, 60, 60);
      await drawRectangle(380, 300, 60, 60);

      await selectMultipleShapes([
        { x: 330, y: 330 },
        { x: 410, y: 330 }
      ]);
      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      // Verify both shapes exist at frame 1
      await assertShapeExists(330, 330, 'First shape at frame 1');
      await assertShapeExists(410, 330, 'Second shape at frame 1');

      // Select the group by dragging a selection box over it
      await selectMultipleShapes([{ x: 370, y: 330 }]);
      await browser.pause(200);

      // Move to frame 10 and add keyframe
      await setPlayheadTime(0.333);
      await addKeyframe();
      await browser.pause(200);

      // Group is selected, so dragging will move it
      // Drag from center of group down (but keep it within canvas bounds)
      await dragCanvas(370, 330, 370, 380);
      await browser.pause(300);

      // At frame 10, group should be at new position (moved 50px down)
      await assertShapeExists(330, 380, 'First shape at new position at frame 10');
      await assertShapeExists(410, 380, 'Second shape at new position at frame 10');

      // Go to frame 1
      await setPlayheadTime(0);
      await browser.pause(200);

      // Group should be at original position
      await assertShapeExists(330, 330, 'First shape at original position at frame 1');
      await assertShapeExists(410, 330, 'Second shape at original position at frame 1');

      // Go to frame 5 (middle, t=0.166)
      await setPlayheadTime(0.166);
      await browser.pause(200);

      // Group should be interpolated (halfway between y=330 and y=380, so y=355)
      await assertShapeExists(330, 355, 'First shape interpolated at frame 5');
      await assertShapeExists(410, 355, 'Second shape interpolated at frame 5');
    });

    it('should maintain relative positions of shapes within animated group', async () => {
      // Create a group (using safer y coordinates)
      await drawRectangle(100, 250, 50, 50);
      await drawRectangle(170, 250, 50, 50);

      await selectMultipleShapes([
        { x: 125, y: 275 },
        { x: 195, y: 275 }
      ]);
      await useKeyboardShortcut('g', true);
      await browser.pause(300);

      // Select group
      await selectMultipleShapes([{ x: 160, y: 275 }]);
      await browser.pause(200);

      // Add keyframe and move
      await setPlayheadTime(0.333);
      await addKeyframe();
      await browser.pause(200);

      await dragCanvas(160, 275, 260, 275);
      await browser.pause(300);

      // At both keyframes, shapes should maintain 70px horizontal distance
      await setPlayheadTime(0);
      await browser.pause(200);
      await assertShapeExists(125, 275, 'First shape at frame 1');
      await assertShapeExists(195, 275, 'Second shape at frame 1 (70px apart)');

      await setPlayheadTime(0.333);
      await browser.pause(200);
      // Both shapes moved 100px to the right
      await assertShapeExists(225, 275, 'First shape at frame 10');
      await assertShapeExists(295, 275, 'Second shape at frame 10 (still 70px apart)');
    });
  });

  describe('Interpolation', () => {
    it('should smoothly interpolate between keyframes', async () => {
      // Draw a simple shape
      await drawRectangle(500, 100, 50, 50);

      // Select it
      await selectMultipleShapes([{ x: 525, y: 125 }]);
      await browser.pause(200);

      // Keyframe at start (x=525)
      await setPlayheadTime(0);
      await browser.pause(100);

      // Keyframe at end (1 second = frame 30, move to x=725)
      await setPlayheadTime(1.0);
      await addKeyframe();
      await browser.pause(200);

      await dragCanvas(525, 125, 725, 125);
      await browser.pause(300);

      // Check multiple intermediate frames for smooth interpolation
      // Total movement: 200px over 1 second

      // At 25% (0.25s), x should be 525 + 50 = 575
      await setPlayheadTime(0.25);
      await browser.pause(200);
      await assertShapeExists(575, 125, 'Shape at 25% interpolation');

      // At 50% (0.5s), x should be 525 + 100 = 625
      await setPlayheadTime(0.5);
      await browser.pause(200);
      await assertShapeExists(625, 125, 'Shape at 50% interpolation');

      // At 75% (0.75s), x should be 525 + 150 = 675
      await setPlayheadTime(0.75);
      await browser.pause(200);
      await assertShapeExists(675, 125, 'Shape at 75% interpolation');
    });
  });
});

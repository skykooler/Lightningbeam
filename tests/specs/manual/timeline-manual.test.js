/**
 * MANUAL Timeline Animation Tests
 * Run these with visual verification - watch the app window as tests execute
 *
 * To run: pnpm wdio run wdio.conf.js --spec tests/specs/manual/timeline-manual.test.js
 */

import { describe, it, before, afterEach } from 'mocha';
import { waitForAppReady } from '../../helpers/app.js';
import {
  drawRectangle,
  selectMultipleShapes,
  dragCanvas,
  setPlayheadTime,
  getPlayheadTime,
  addKeyframe,
  useKeyboardShortcut
} from '../../helpers/canvas.js';
import { verifyManually, logStep, pauseAndDescribe } from '../../helpers/manual.js';

describe('MANUAL: Timeline Animation', () => {
  before(async () => {
    await waitForAppReady();
  });

  afterEach(async () => {
    // Close any open dialogs by accepting them
    try {
      await browser.execute(function() {
        // Close any open confirm/alert dialogs
        // This is a no-op if no dialog is open
      });
    } catch (e) {
      // Ignore errors
    }

    // Pause briefly to show final state before ending
    await browser.pause(1000);
    console.log('\n>>> Test completed. Session will restart for next test.\n');
  });

  it('TEST 1: Group animation - draw, group, keyframe, move group', async () => {
    await logStep('Drawing a RED rectangle at (100, 100) with size 100x100');
    await drawRectangle(100, 100, 100, 100, true, '#ff0000');
    await pauseAndDescribe('RED rectangle drawn', 2000);

    await verifyManually(
      'VERIFY: Do you see a RED filled rectangle at the top-left area?\n' +
      'It should be centered around (150, 150)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Selecting the RED rectangle by dragging a selection box over it');
    await selectMultipleShapes([{ x: 150, y: 150 }]);
    await pauseAndDescribe('RED rectangle selected', 2000);

    await verifyManually(
      'VERIFY: Is the RED rectangle now selected? (Should have selection indicators)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Grouping the selected rectangle (Ctrl+G)');
    await useKeyboardShortcut('g', true);
    await pauseAndDescribe('RED rectangle grouped', 2000);

    await verifyManually(
      'VERIFY: Was the rectangle grouped? (May look similar but is now a group)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Selecting the group by dragging a selection box over it');
    await selectMultipleShapes([{ x: 150, y: 150 }]);
    await pauseAndDescribe('Group selected', 2000);

    await logStep('Moving playhead to time 0.333 (frame 10 at 30fps)');
    await setPlayheadTime(0.333);
    await pauseAndDescribe('Playhead moved to 0.333s - WAIT for UI to update', 3000);

    await verifyManually(
      'VERIFY: Did the playhead indicator move on the timeline?\n' +
      'It should be at approximately frame 10\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Adding a keyframe at current position');
    await addKeyframe();
    await pauseAndDescribe('Keyframe added', 2000);

    await verifyManually(
      'VERIFY: Was a keyframe added? (Should see a keyframe marker on timeline)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Dragging the selected group to move it right (from x=150 to x=250)');
    await dragCanvas(150, 150, 250, 150);
    await pauseAndDescribe('Group moved to the right', 3000);

    await verifyManually(
      'VERIFY: Did the RED rectangle move to the right?\n' +
      'It should now be centered around (250, 150)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Moving playhead back to time 0 (frame 1)');
    await setPlayheadTime(0);
    await pauseAndDescribe('Playhead back at start', 3000);

    await verifyManually(
      'VERIFY: Did the RED rectangle jump back to its original position (x=150)?\n' +
      'This confirms the group animation is working!\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Moving playhead to middle (time 0.166, frame 5)');
    await setPlayheadTime(0.166);
    await pauseAndDescribe('Playhead at middle frame', 3000);

    await verifyManually(
      'VERIFY: Is the RED rectangle now between the two positions?\n' +
      'It should be around x=200 (interpolated halfway)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Moving playhead back and forth to show animation');
    await setPlayheadTime(0);
    await browser.pause(1000);
    await setPlayheadTime(0.333);
    await browser.pause(1000);
    await setPlayheadTime(0);
    await browser.pause(1000);
    await setPlayheadTime(0.333);
    await browser.pause(1000);

    await verifyManually(
      'VERIFY: Did you see the RED rectangle animate back and forth?\n' +
      'This demonstrates the timeline animation is working!\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('TEST 1 COMPLETE - Showing completion alert');
    const completionShown = await browser.execute(function() {
      alert('TEST 1 COMPLETE - Click OK to finish');
      return true;
    });
    await browser.pause(2000); // Wait for alert to be dismissed before ending test
  });

  it('TEST 2: Shape tween - draw shape, add keyframes, modify edges', async () => {
    await logStep('Drawing a BLUE rectangle at (400, 100)');
    await drawRectangle(400, 100, 80, 80, true, '#0000ff');
    await pauseAndDescribe('BLUE rectangle drawn', 2000);

    await verifyManually(
      'VERIFY: Do you see a BLUE filled rectangle?\n' +
      'It should be at (400, 100) with size 80x80\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Selecting the BLUE rectangle');
    await selectMultipleShapes([{ x: 440, y: 140 }]);
    await pauseAndDescribe('BLUE rectangle selected', 2000);

    await verifyManually(
      'VERIFY: Is the BLUE rectangle selected?\n' +
      '(An initial keyframe should be automatically added at time 0)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Moving playhead to time 0.5');
    await setPlayheadTime(0.5);
    await pauseAndDescribe('Playhead moved to 0.5s - WAIT for UI to update', 3000);

    await verifyManually(
      'VERIFY: Did the playhead move to 0.5s on the timeline?\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Adding a keyframe at time 0.5');
    await addKeyframe();
    await pauseAndDescribe('Keyframe added at 0.5s', 2000);

    await verifyManually(
      'VERIFY: Was a keyframe added at 0.5s?\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Dragging the right edge of the BLUE rectangle to curve/extend it');
    await dragCanvas(480, 140, 530, 140);
    await pauseAndDescribe('Dragged right edge of BLUE rectangle', 3000);

    await verifyManually(
      'VERIFY: Did the right edge of the BLUE rectangle get curved/pulled out?\n' +
      'The shape should now be modified/stretched to the right\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Moving playhead back to time 0');
    await setPlayheadTime(0);
    await pauseAndDescribe('Playhead back at start', 3000);

    await verifyManually(
      'VERIFY: Did the BLUE rectangle return to its original rectangular shape?\n' +
      'The edge modification should not be visible at time 0\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Moving playhead to middle (time 0.25)');
    await setPlayheadTime(0.25);
    await pauseAndDescribe('Playhead at middle (0.25s)', 3000);

    await verifyManually(
      'VERIFY: Is the BLUE rectangle shape somewhere between the two versions?\n' +
      'It should be partially morphed (shape tween interpolation)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('TEST 2 COMPLETE - Showing completion alert');
    const completionShown = await browser.execute(function() {
      alert('TEST 2 COMPLETE - Click OK to finish');
      return true;
    });
    await browser.pause(2000); // Wait for alert to be dismissed before ending test
  });

  it('TEST 3: Test dragging unselected shape edge', async () => {
    await logStep('Drawing a GREEN rectangle at (200, 250) WITHOUT selecting it');
    await drawRectangle(200, 250, 100, 100, true, '#00ff00');
    await pauseAndDescribe('GREEN rectangle drawn (not selected)', 2000);

    await verifyManually(
      'VERIFY: GREEN rectangle should be visible but NOT selected\n' +
      '(No selection indicators around it)\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('Dragging from the right edge (x=300) of GREEN rectangle to extend it');
    await dragCanvas(300, 300, 350, 300);
    await pauseAndDescribe('Dragged the right edge of GREEN rectangle', 3000);

    await verifyManually(
      'VERIFY: What happened to the GREEN rectangle?\n\n' +
      'Expected: The right edge should be curved/pulled out to x=350\n' +
      'Did the edge get modified as expected?\n\n' +
      'Click OK if yes, Cancel if no'
    );

    await logStep('TEST 3 COMPLETE - Showing completion alert');
    const completionShown = await browser.execute(function() {
      alert('TEST 3 COMPLETE - Click OK to finish');
      return true;
    });
    await browser.pause(2000); // Wait for alert to be dismissed before ending test
  });
});

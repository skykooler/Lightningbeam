/**
 * Manual testing utilities for user-in-the-loop verification
 * These helpers pause execution and wait for user confirmation
 */

/**
 * Pause and wait for user to verify something visually with a confirm dialog
 * @param {string} message - What the user should verify
 * @param {boolean} waitForConfirm - If true, show confirm dialog and wait for user input
 * @throws {Error} If user clicks Cancel to indicate verification failed
 */
export async function verifyManually(message, waitForConfirm = true) {
  console.log('\n=== MANUAL VERIFICATION ===');
  console.log(message);
  console.log('===========================\n');

  if (waitForConfirm) {
    // Show a confirm dialog in the browser and wait for user response
    const result = await browser.execute(function(msg) {
      return confirm(msg);
    }, message);

    if (!result) {
      console.log('User clicked Cancel - verification failed');
      throw new Error('Manual verification failed: User clicked Cancel');
    } else {
      console.log('User clicked OK - verification passed');
    }

    return result;
  } else {
    // Just pause for observation
    await browser.pause(3000);
    return true;
  }
}

/**
 * Add a visual marker/annotation to describe what should be visible
 * @param {string} description - Description of current state
 */
export async function logStep(description) {
  console.log(`\n>>> STEP: ${description}`);
}

/**
 * Extended pause with a description of what's happening
 * @param {string} action - What action just occurred
 * @param {number} pauseTime - How long to pause
 */
export async function pauseAndDescribe(action, pauseTime = 2000) {
  console.log(`>>> ${action}`);
  await browser.pause(pauseTime);
}

/**
 * Ask user a yes/no question via confirm dialog
 * @param {string} question - Question to ask the user
 * @returns {Promise<boolean>} True if user clicked OK, false if Cancel
 */
export async function askUser(question) {
  console.log(`\n>>> QUESTION: ${question}`);
  const result = await browser.execute(function(msg) {
    return confirm(msg);
  }, question);
  console.log(`User answered: ${result ? 'YES (OK)' : 'NO (Cancel)'}`);
  return result;
}

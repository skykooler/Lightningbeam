/**
 * App helper utilities for Tauri application testing
 */

/**
 * Wait for the Lightningbeam app to be fully loaded and ready
 * @param {number} timeout - Maximum time to wait in ms
 */
export async function waitForAppReady(timeout = 5000) {
  await browser.waitForApp();

  // Check for "Animation" card on start screen and click it if present
  // The card has a label div with text "Animation"
  const animationCard = await browser.$('.focus-card-label*=Animation');
  if (await animationCard.isExisting()) {
    // Click the parent focus-card element
    const card = await animationCard.parentElement();
    await card.waitForClickable({ timeout: 2000 });
    await card.click();
    await browser.pause(1000); // Wait longer for animation view to load
  } else {
    // Legacy: Check for "Create New File" dialog and click Create if present
    const createButton = await browser.$('button*=Create');
    if (await createButton.isExisting()) {
      await createButton.waitForClickable({ timeout: 2000 });
      await createButton.click();
      await browser.pause(500); // Wait for dialog to close
    }
  }

  // Wait for the main canvas to be present
  const canvas = await browser.$('canvas');
  await canvas.waitForExist({ timeout });

  // Additional wait for any initialization
  await browser.pause(500);
}

/**
 * Get the main canvas element
 * @returns {Promise<WebdriverIO.Element>}
 */
export async function getCanvas() {
  return await browser.$('canvas');
}

/**
 * Get canvas dimensions
 * @returns {Promise<{width: number, height: number}>}
 */
export async function getCanvasSize() {
  const canvas = await getCanvas();
  const size = await canvas.getSize();
  return size;
}

/**
 * Take a screenshot of the canvas
 * @param {string} filename - Name for the screenshot file
 */
export async function takeCanvasScreenshot(filename) {
  const canvas = await getCanvas();
  return await canvas.saveScreenshot(`./tests/screenshots/${filename}`);
}

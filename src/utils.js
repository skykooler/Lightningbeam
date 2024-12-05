function titleCase(str) {
    return str.charAt(0).toUpperCase() + str.slice(1).toLowerCase();
}

function getMousePositionFraction(event, element) {
    const rect = element.getBoundingClientRect(); // Get the element's position and size
    
    if (element.classList.contains('horizontal-grid')) {
      // If the element has the "horizontal-grid" class, calculate the horizontal position (X)
      const xPos = event.clientX - rect.left; // Mouse X position relative to the element
      const fraction = xPos / rect.width; // Fraction of the width
      return Math.min(Math.max(fraction, 0), 1); // Ensure the fraction is between 0 and 1
    } else if (element.classList.contains('vertical-grid')) {
      // If the element has the "vertical-grid" class, calculate the vertical position (Y)
      const yPos = event.clientY - rect.top; // Mouse Y position relative to the element
      const fraction = yPos / rect.height; // Fraction of the height
      return Math.min(Math.max(fraction, 0), 1); // Ensure the fraction is between 0 and 1
    }
    return 0; // If neither class is present, return 0 (or handle as needed)
  }

function getKeyframesSurrounding(frames, index) {
    let lastKeyframeBefore = undefined;
    let firstKeyframeAfter = undefined;

    // Find the last keyframe before the given index
    for (let i = index - 1; i >= 0; i--) {
        if (frames[i].frameType === "keyframe") {
        lastKeyframeBefore = i;
        break;
        }
    }

    // Find the first keyframe after the given index
    for (let i = index + 1; i < frames.length; i++) {
        if (frames[i].frameType === "keyframe") {
        firstKeyframeAfter = i;
        break;
        }
    }
    return { lastKeyframeBefore, firstKeyframeAfter };
}

function invertPixels(ctx, width, height) {
    // Create an off-screen canvas for the pattern
    const patternCanvas = document.createElement('canvas');
    const patternContext = patternCanvas.getContext('2d');

    // Define the size of the repeating pattern (2x2 pixels)
    const patternSize = 2;
    patternCanvas.width = patternSize;
    patternCanvas.height = patternSize;

    // Create the alternating pattern (regular and inverted pixels)
    function createInvertedPattern() {
      const patternData = patternContext.createImageData(patternSize, patternSize);
      const data = patternData.data;

      // Fill the pattern with alternating colors (inverted every other pixel)
      for (let i = 0; i < patternSize; i++) {
        for (let j = 0; j < patternSize; j++) {
          const index = (i * patternSize + j) * 4;
          // Determine if we should invert the color
          if ((i + j) % 2 === 0) {
            data[index] = 255; // Red
            data[index + 1] = 0; // Green
            data[index + 2] = 0; // Blue
            data[index + 3] = 255; // Alpha
          } else {
            data[index] = 0; // Red (inverted)
            data[index + 1] = 255; // Green (inverted)
            data[index + 2] = 255; // Blue (inverted)
            data[index + 3] = 255; // Alpha
          }
        }
      }

      // Set the pattern on the off-screen canvas
      patternContext.putImageData(patternData, 0, 0);
      return patternCanvas;
    }

    // Create the pattern using the function
    const pattern = ctx.createPattern(createInvertedPattern(), 'repeat');

    // Draw a rectangle with the pattern
    ctx.globalCompositeOperation = "difference"
    ctx.fillStyle = pattern;
    ctx.fillRect(0, 0, width, height);

    ctx.globalCompositeOperation = "source-over"
}

function lerp(a, b, t) {
  return a + (b - a) * t;
}

function lerpColor(color1, color2, t) {
  // Convert hex color to RGB
  const hexToRgb = (hex) => {
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return { r, g, b };
  };

  // Convert RGB to hex color
  const rgbToHex = (r, g, b) => {
    return `#${(1 << 24 | (r << 16) | (g << 8) | b).toString(16).slice(1).toUpperCase()}`;
  };

  // Get RGB values of both colors
  const start = hexToRgb(color1);
  const end = hexToRgb(color2);

  // Calculate the interpolated RGB values
  const r = Math.round(start.r + (end.r - start.r) * t);
  const g = Math.round(start.g + (end.g - start.g) * t);
  const b = Math.round(start.b + (end.b - start.b) * t);

  // Convert the interpolated RGB back to hex
  return rgbToHex(r, g, b);
}

export { titleCase, getMousePositionFraction, getKeyframesSurrounding, invertPixels, lerp, lerpColor };
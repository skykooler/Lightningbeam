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
          if ((i + j) % 2 === 0 || j%2===0) {
            data[index] = 0; // Red
            data[index + 1] = 0; // Green
            data[index + 2] = 0; // Blue
            data[index + 3] = 255; // Alpha
          } else {
            data[index] = 255; // Red (inverted)
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

function camelToWords(camelCaseString) {
  // Insert a space before each uppercase letter and make it lowercase
  const words = camelCaseString.replace(/([A-Z])/g, ' $1').toLowerCase();
  
  // Capitalize the first letter of each word
  return words.replace(/\b\w/g, char => char.toUpperCase());
}

function generateWaveform(img, buffer, imgHeight, frameWidth, framesPerSecond) {
  // Total duration of the audio in seconds
  const duration = buffer.duration;
  const canvasWidth = Math.ceil(frameWidth * framesPerSecond * duration);
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d');
  canvas.width = canvasWidth;
  canvas.height = imgHeight;

  // Get the audio buffer's data (mono or stereo channels)
  const channels = buffer.numberOfChannels;
  const leftChannelData = buffer.getChannelData(0); // Left channel
  const rightChannelData = channels > 1 ? buffer.getChannelData(1) : null;  // Right channel, if stereo
  const width = canvas.width;
  const step = Math.ceil(leftChannelData.length / width); // Step size for drawing
  const halfHeight = canvas.height / 2;
  ctx.fillStyle = '#000';
    
  function drawChannel(channelData) {
    const samples = [];

    // Draw the waveform by taking the maximum value of samples in each window
    for (let i = 0; i < width; i++) {
      let maxSample = -Infinity;

      // Calculate the maximum value within the window
      for (let j = i * step; j < (i + 1) * step && j < channelData.length; j++) {
        maxSample = Math.max(maxSample, Math.abs(channelData[j])); // Find the maximum absolute sample
      }

      // Normalize and scale the max sample to fit within the canvas height
      const y = maxSample * halfHeight;

      samples.push([i, y]);
    }

    // Fill the waveform
    if (samples.length > 0) {
      ctx.beginPath();
      ctx.moveTo(samples[0][0], samples[0][1]);
      for (let i = 0; i < samples.length; i++) {
        ctx.lineTo(samples[i][0], samples[i][1]);
      }
      for (let i = samples.length - 1; i >= 0; i--) {
        ctx.lineTo(samples[i][0], -samples[i][1]);
      }
      ctx.fill();
    }
  }

  if (channels>1) {
    ctx.save();
    ctx.translate(0, halfHeight*0.5);
    drawChannel(leftChannelData);
    ctx.restore();
    ctx.save();
    ctx.translate(0, halfHeight*1.5);
    drawChannel(rightChannelData);
    ctx.restore();
  } else {
    ctx.save();
    ctx.translate(0, halfHeight);
    drawChannel(leftChannelData);
    ctx.restore();
  }
  
  const dataUrl = canvas.toDataURL("image/png");
  img.src = dataUrl;
}


export {
  titleCase,
  getMousePositionFraction,
  getKeyframesSurrounding,
  invertPixels,
  lerp,
  lerpColor,
  camelToWords,
  generateWaveform
};
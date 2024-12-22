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

function floodFillRegion(startPoint, epsilon, fileWidth, fileHeight, context, debugPoints, debugPaintbucket) {
  // Helper function to check if the point is at the boundary of the region
  function isBoundaryPoint(point) {
    return point.x <= 0 || point.x >= fileWidth || point.y <= 0 || point.y >= fileHeight;
  }
  let halfEpsilon = epsilon/2

  // Helper function to check if a point is near any curve in the shape
  function isNearCurve(point, shape) {
    // Generate bounding box around the point for quadtree query
    const bbox = {
      x: { min: point.x - halfEpsilon, max: point.x + halfEpsilon },
      y: { min: point.y - halfEpsilon, max: point.y + halfEpsilon }
    };
    // Get the list of curve indices that are near the point
    const nearbyCurveIndices = shape.quadtree.query(bbox);
    // const nearbyCurveIndices = shape.curves.keys()
    // Check if any of the curves are close enough to the point
    for (const idx of nearbyCurveIndices) {
      const curve = shape.curves[idx];
      const projection = curve.project(point);
      if (projection.d < epsilon) {
        return projection;
      }
    }
    return false;
  }

  const shapes = context.activeObject.currentFrame.shapes;
  const visited = new Set();
  const stack = [startPoint];
  const regionPoints = [];

  // Begin the flood fill process
  while (stack.length > 0) {
    const currentPoint = stack.pop();

    // If we reach the boundary of the region, throw an exception
    if (isBoundaryPoint(currentPoint)) {
      throw new Error("Flood fill reached the boundary of the area.");
    }

    // If the current point is already visited, skip it
    const pointKey = `${currentPoint.x},${currentPoint.y}`;
    if (visited.has(pointKey)) {
      continue;
    }
    visited.add(pointKey);
    if (debugPaintbucket) {
      debugPoints.push(currentPoint)
    }

    let isNearAnyCurve = false;
    for (const shape of shapes) {
      let projection = isNearCurve(currentPoint, shape)
      if (projection) {
        isNearAnyCurve = true;
        regionPoints.push(projection)
        break;
      }
    }

    // Skip the points that are near curves, to prevent jumping past them
    if (!isNearAnyCurve) {
      const neighbors = [
        { x: currentPoint.x - epsilon, y: currentPoint.y },
        { x: currentPoint.x + epsilon, y: currentPoint.y },
        { x: currentPoint.x, y: currentPoint.y - epsilon },
        { x: currentPoint.x, y: currentPoint.y + epsilon }
      ];
      // Add unvisited neighbors to the stack
      for (const neighbor of neighbors) {
        const neighborKey = `${neighbor.x},${neighbor.y}`;
        if (!visited.has(neighborKey)) {
          stack.push(neighbor);
        }
      }
    }
  }

  // Return the region points in connected order
  return sortPointsByProximity(regionPoints)
}

function sortPointsByProximity(points) {
  if (points.length <= 1) return points;

  // Start with the first point as the initial sorted point
  const sortedPoints = [points[0]];
  points.splice(0, 1); // Remove the first point from the original list

  // Iterate through the remaining points and find the nearest neighbor
  while (points.length > 0) {
    const lastPoint = sortedPoints[sortedPoints.length - 1];
    
    // Find the closest point to the last point
    let closestIndex = -1;
    let closestDistance = Infinity;

    for (let i = 0; i < points.length; i++) {
      const currentPoint = points[i];
      const distance = Math.sqrt(Math.pow(currentPoint.x - lastPoint.x, 2) + Math.pow(currentPoint.y - lastPoint.y, 2));

      if (distance < closestDistance) {
        closestDistance = distance;
        closestIndex = i;
      }
    }

    // Add the closest point to the sorted points
    sortedPoints.push(points[closestIndex]);
    points.splice(closestIndex, 1); // Remove the closest point from the original list
  }

  return sortedPoints;
}

function getShapeAtPoint(point, shapes) {
  // Create a 1x1 off-screen canvas and translate so it is in the first pixel
  const offscreenCanvas = document.createElement('canvas');
  offscreenCanvas.width = 1;
  offscreenCanvas.height = 1;
  const ctx = offscreenCanvas.getContext('2d');

  ctx.translate(-point.x, -point.y);
  const colorToShapeMap = {};
  
  // Generate a unique color for each shape (start from #000001 and increment)
  let colorIndex = 1;

  // Draw all shapes to the off-screen canvas with their unique colors
  shapes.forEach(shape => {
      // Generate a unique color for this shape
      const debugColor = intToHexColor(colorIndex);
      colorToShapeMap[debugColor] = shape;

      const context = {
          ctx: ctx,
          debugColor: debugColor
      };
      shape.draw(context);
      colorIndex++;
  });

  const pixel = ctx.getImageData(0, 0, 1, 1).data;
  const sampledColor = rgbToHex(pixel[0], pixel[1], pixel[2]);
  return colorToShapeMap[sampledColor] || null;
}

// Helper function to convert a number (0-16777215) to a hex color code
function intToHexColor(value) {
  // Ensure the value is between 0 and 16777215 (0xFFFFFF)
  value = value & 0xFFFFFF;
  return '#' + value.toString(16).padStart(6, '0').toUpperCase();
}

function hslToRgb(h, s, l) {
  // Ensure that the input values are within the expected range [0, 1]
  h = h % 1;  // Hue wraps around at 1
  s = Math.min(Math.max(s, 0), 1);  // Saturation should be between 0 and 1
  l = Math.min(Math.max(l, 0), 1);  // Lightness should be between 0 and 1

  // Handle case where saturation is 0 (the color is gray)
  if (s === 0) {
      const gray = Math.round(l * 255);  // All RGB values are equal to the lightness value
      return { r: gray, g: gray, b: gray };
  }

  // Calculate temporary values
  const temp2 = (l < 0.5) ? (l * (1 + s)) : (l + s - l * s);
  const temp1 = 2 * l - temp2;

  // Pre-calculate hues at the different points to avoid repeating calculations
  const r = hueToRgb(temp1, temp2, h + 1 / 3);
  const g = hueToRgb(temp1, temp2, h);
  const b = hueToRgb(temp1, temp2, h - 1 / 3);

  // Return RGB values in 0-255 range, rounding the result
  return {
      r: Math.round(r * 255),
      g: Math.round(g * 255),
      b: Math.round(b * 255)
  };
}

function hueToRgb(t1, t2, t3) {
  // Normalize hue to be between 0 and 1
  if (t3 < 0) t3 += 1;
  if (t3 > 1) t3 -= 1;

  // Efficient calculation of RGB component
  if (6 * t3 < 1) return t1 + (t2 - t1) * 6 * t3;
  if (2 * t3 < 1) return t2;
  if (3 * t3 < 2) return t1 + (t2 - t1) * (2 / 3 - t3) * 6;
  return t1;
}

function hsvToRgb(h, s, v) {
  let r, g, b;

  if (s === 0) {
      // If saturation is 0, the color is a shade of gray
      r = g = b = v;  // All channels are equal
  } else {
      // Calculate the hue sector (6 sectors, for each of the primary and secondary colors)
      const i = Math.floor(h * 6); // The integer part of the hue value
      const f = h * 6 - i;         // The fractional part of the hue
      const p = v * (1 - s);        // The value at the lower boundary
      const q = v * (1 - f * s);    // Intermediate value
      const t = v * (1 - (1 - f) * s); // Another intermediate value

      // Use the hue sector index (i) to determine which RGB component will be maximum
      switch (i % 6) {
          case 0: r = v; g = t; b = p; break;
          case 1: r = q; g = v; b = p; break;
          case 2: r = p; g = v; b = t; break;
          case 3: r = p; g = q; b = v; break;
          case 4: r = t; g = p; b = v; break;
          case 5: r = v; g = p; b = q; break;
      }
  }

  // Return RGB values between 0 and 255 (scaled from 0-1 to 0-255)
  return {
      r: Math.round(r * 255),
      g: Math.round(g * 255),
      b: Math.round(b * 255)
  };
}

let cachedPattern = null; // Cache the pattern

function drawCheckerboardBackground(ctx, x, y, width, height, squareSize) {
    // If the pattern is not cached, create and cache it
    if (!cachedPattern) {
        // Define two shades of gray for the checkerboard
        const color1 = '#E0E0E0';  // Light gray
        const color2 = '#B0B0B0';  // Dark gray
        
        // Create a 2x2 checkerboard pattern with four squares
        const patternCanvas = document.createElement('canvas');
        const patternCtx = patternCanvas.getContext('2d');
        
        // Set the pattern canvas size to 2x2 squares (width and height)
        patternCanvas.width = 2 * squareSize;
        patternCanvas.height = 2 * squareSize;

        // Fill the four squares to create the checkerboard pattern
        patternCtx.fillStyle = color1; // Light gray for the first square
        patternCtx.fillRect(0, 0, squareSize, squareSize); // Top-left square

        patternCtx.fillStyle = color2; // Dark gray for the second square
        patternCtx.fillRect(squareSize, 0, squareSize, squareSize); // Top-right square

        patternCtx.fillStyle = color2; // Dark gray for the third square
        patternCtx.fillRect(0, squareSize, squareSize, squareSize); // Bottom-left square

        patternCtx.fillStyle = color1; // Light gray for the fourth square
        patternCtx.fillRect(squareSize, squareSize, squareSize, squareSize); // Bottom-right square

        // Cache the repeating pattern
        cachedPattern = ctx.createPattern(patternCanvas, 'repeat');
    }
    
    // Set the cached pattern as the fill style for the rectangle
    ctx.fillStyle = cachedPattern;

    // Draw the rectangle with the repeating checkerboard pattern
    ctx.fillRect(x, y, width, height);
}

function hexToHsl(hex) {
  // Step 1: Convert hex to RGB
  let r = parseInt(hex.substring(1, 3), 16) / 255;
  let g = parseInt(hex.substring(3, 5), 16) / 255;
  let b = parseInt(hex.substring(5, 7), 16) / 255;

  // Step 2: Find the maximum and minimum values of r, g, and b
  let max = Math.max(r, g, b);
  let min = Math.min(r, g, b);

  // Step 3: Calculate Lightness (L)
  let l = (max + min) / 2;

  // Step 4: Calculate Saturation (S)
  let s = 0;
  if (max !== min) {
      s = (l > 0.5) ? (max - min) / (2 - max - min) : (max - min) / (max + min);
  }

  // Step 5: Calculate Hue (H)
  let h = 0;
  if (max === r) {
      h = (g - b) / (max - min);
  } else if (max === g) {
      h = (b - r) / (max - min) + 2;
  } else {
      h = (r - g) / (max - min) + 4;
  }

  h = (h / 6) % 1;  // Normalize hue to be between 0 and 1

  // Return HSL values with H, S, and L scaled to [0.0, 1.0]
  return { h: h, s: s, l: l };
}

function hexToHsv(hex) {
  // Step 1: Convert hex to RGB
  let r = parseInt(hex.substring(1, 3), 16) / 255;
  let g = parseInt(hex.substring(3, 5), 16) / 255;
  let b = parseInt(hex.substring(5, 7), 16) / 255;

  // Step 2: Calculate Min and Max RGB values
  let min = Math.min(r, g, b);
  let max = Math.max(r, g, b);
  let delta = max - min;

  // Step 3: Calculate Hue
  let h = 0;
  if (delta !== 0) {
      if (max === r) {
          h = (g - b) / delta; // Red is max
      } else if (max === g) {
          h = (b - r) / delta + 2; // Green is max
      } else {
          h = (r - g) / delta + 4; // Blue is max
      }
      h = (h / 6 + 1) % 1;  // Normalize to [0, 1]
  }

  // Step 4: Calculate Saturation
  let s = 0;
  if (max !== 0) {
      s = delta / max;
  }

  // Step 5: Calculate Value
  let v = max;

  // Return HSV values, with H, S, and V between 0.0 and 1.0
  return { h: h, s: s, v: v };
}

const rgbToHex = (r, g, b) => {
  return `#${(1 << 24 | (r << 16) | (g << 8) | b).toString(16).slice(1).toUpperCase()}`;
};

function clamp(n) {
  // Clamps a value between 0 and 1
  return Math.min(Math.max(n,0),1)
}

function drawBorderedRect(ctx, x, y, width, height, top, bottom, left, right) {
  ctx.fillRect(x, y, width, height)
  if (top) {
    ctx.strokeStyle = top
    ctx.beginPath()
    ctx.moveTo(x, y)
    ctx.lineTo(x+width, y)
    ctx.stroke()
  }
  if (bottom) {
    ctx.strokeStyle = bottom
    ctx.beginPath()
    ctx.moveTo(x, y+height)
    ctx.lineTo(x+width, y+height)
    ctx.stroke()
  }
  if (left) {
    ctx.strokeStyle = left
    ctx.beginPath()
    ctx.moveTo(x, y)
    ctx.lineTo(x, y+height)
    ctx.stroke()
  }
  if (right) {
    ctx.strokeStyle = right
    ctx.beginPath()
    ctx.moveTo(x+width, y)
    ctx.lineTo(x+width, y+height)
    ctx.stroke()
  }
}

function drawCenteredText(ctx, text, x, y, height) {
  ctx.font = `${height}px Arial`; // TODO: allow configuring font somewhere

  // Calculate the width of the text
  const textWidth = ctx.measureText(text).width;
  
  // Calculate the position to center the text
  const centerX = x - textWidth / 2;
  const centerY = y + height / 4; // Adjust for vertical centering

  // Draw the text centered at (x, y) with the specified font size
  ctx.fillText(text, centerX, centerY);
}

function drawHorizontallyCenteredText(ctx, text, x, y, height) {
  ctx.font = `${height}px Arial`; // TODO: allow configuring font somewhere
  const centerY = y + height / 4; // Adjust for vertical centering

  ctx.fillText(text, x, centerY);
}

function deepMerge(target, source) {
  // If either target or source is not an object, return source (base case)
  if (typeof target !== 'object' || target === null) {
    return source;
  }

  // If target is an object, recursively merge
  if (typeof source === 'object' && source !== null) {
    for (let key in source) {
      // If the key exists in both objects, and both are objects, recursively merge
      if (target.hasOwnProperty(key) && typeof target[key] === 'object' && typeof source[key] === 'object') {
        target[key] = deepMerge(target[key], source[key]);
      } else {
        // Otherwise, just assign the source value to target
        target[key] = source[key];
      }
    }
  }

  return target;
}

function getPointNearBox(boundingBox, point, threshold = 5, checkCenters = true) {
  const { x: { min: xMin, max: xMax }, y: { min: yMin, max: yMax } } = boundingBox;
  const { x, y } = point;
  
  // List of points to check (corners and centers of sides) with their names
  const pointsToCheck = [
    { name: 'nw', x: xMin, y: yMin }, // top-left corner
    { name: 'ne', x: xMax, y: yMin }, // top-right corner
    { name: 'sw', x: xMin, y: yMax }, // bottom-left corner
    { name: 'se', x: xMax, y: yMax }, // bottom-right corner
  ];

  // Optionally add the center points if checkCenters is true
  if (checkCenters) {
    pointsToCheck.push(
      { name: 'n', x: (xMin + xMax) / 2, y: yMin }, // center of top side
      { name: 's', x: (xMin + xMax) / 2, y: yMax }, // center of bottom side
      { name: 'w', x: xMin, y: (yMin + yMax) / 2 }, // center of left side
      { name: 'e', x: xMax, y: (yMin + yMax) / 2 }  // center of right side
    );
  }

  // Check if the point is within the threshold distance of any of the points
  for (let i = 0; i < pointsToCheck.length; i++) {
    const pt = pointsToCheck[i];
    const manhattanDistance = Math.abs(pt.x - x) + Math.abs(pt.y - y);

    if (manhattanDistance <= threshold) {
      return pt.name; // Return the name of the point that is close to the input point
    }
  }

  return null; // Point is not within the threshold distance of any relevant point
}


export {
  titleCase,
  getMousePositionFraction,
  getKeyframesSurrounding,
  invertPixels,
  lerp,
  lerpColor,
  camelToWords,
  generateWaveform,
  floodFillRegion,
  getShapeAtPoint,
  hslToRgb,
  hsvToRgb,
  hexToHsl,
  hexToHsv,
  rgbToHex,
  drawCheckerboardBackground,
  clamp,
  drawBorderedRect,
  drawCenteredText,
  drawHorizontallyCenteredText,
  deepMerge,
  getPointNearBox
};
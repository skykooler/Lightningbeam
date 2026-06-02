import { backgroundColor, foregroundColor, frameWidth, highlight, layerHeight, shade, shadow, labelColor } from "./styles.js";
import { clamp, drawBorderedRect, drawCheckerboardBackground, hslToRgb, hsvToRgb, rgbToHex } from "./utils.js"
import { TimelineState, TimeRuler, TrackHierarchy } from "./timeline.js"
const { invoke } = window.__TAURI__.core

function growBoundingBox(bboxa, bboxb) {
  bboxa.x.min = Math.min(bboxa.x.min, bboxb.x.min);
  bboxa.y.min = Math.min(bboxa.y.min, bboxb.y.min);
  bboxa.x.max = Math.max(bboxa.x.max, bboxb.x.max);
  bboxa.y.max = Math.max(bboxa.y.max, bboxb.y.max);
}

const SCROLL = {
  HORIZONTAL: 1,
  VERTICAL: 2,
}

class Widget {
  constructor(x, y) {
    this._globalEvents = new Set()
    this.x = x
    this.y = y
    this.scale_x = 1
    this.scale_y = 1
    this.rotation = 0
    this.children = []
  }
  handleMouseEvent(eventType, x, y) {
    for (let child of this.children) {
      // Adjust for translation
      const dx = x - child.x;
      const dy = y - child.y;
      
      // Apply inverse rotation
      const cosTheta = Math.cos(child.rotation);
      const sinTheta = Math.sin(child.rotation);
      
      // Rotate coordinates to child's local space
      const rotatedX = dx * cosTheta + dy * sinTheta;
      const rotatedY = -dx * sinTheta + dy * cosTheta;
      
      // First, perform hit test using original (global) coordinates
      if (child.hitTest(rotatedX, rotatedY) || child._globalEvents.has(eventType)) {
        child.handleMouseEvent(eventType, rotatedX, rotatedY);
      }
    }
    const eventTypes = [
      "mousedown",
      "mousemove",
      "mouseup",
      "dblclick",
      "contextmenu"
    ]
    if (eventTypes.indexOf(eventType)!=-1) {
      if (typeof(this[eventType]) == "function") {
        this[eventType](x, y)
      }
    }
  }
  hitTest(x, y) {
    // if ((x >= this.x) && (x <= this.x+this.width) &&
    //     (y >= this.y) && (y <= this.y+this.height)) {
    if ((x>=0) && (x <= this.width) && (y >= 0) && (y <= this.height)) {
      return true
    }
    return false
  }
  bbox() {
    let bbox;
    if (this.children.length > 0) {
      if (!bbox) {
        bbox = structuredClone(this.children[0].bbox());
      }
      for (let child of this.children) {
        growBoundingBox(bbox, child.bbox());
      }
    }
    if (bbox == undefined) {
      bbox = { x: { min: 0, max: 0 }, y: { min: 0, max: 0 } };
    }
    bbox.x.max *= this.scale_x;
    bbox.y.max *= this.scale_y;
    bbox.x.min += this.x;
    bbox.x.max += this.x;
    bbox.y.min += this.y;
    bbox.y.max += this.y;
    return bbox;
  }
  draw(ctx) {
    for (let child of this.children) {
      const transform = ctx.getTransform()
      ctx.translate(child.x, child.y)
      ctx.scale(child.scale_x, child.scale_y)
      ctx.rotate(child.rotation)
      child.draw(ctx)
      ctx.setTransform(transform)
    }
  }
}
class HueSelectionBar extends Widget {
  constructor(width, height, x, y, colorCvs) {
    super(x, y)
    this.width = width
    this.height = height
    this.colorCvs = colorCvs
  }
  
  draw(ctx) {
    const [h, s, v] = this.colorCvs.currentHSV
    const hueGradient = ctx.createImageData(this.width, this.height);
    const data = hueGradient.data;
    for (let i = 0; i < data.length; i += 4) {
      const x = ((i / 4) % this.width) / this.width;
      const y = Math.floor(i / 4 / this.height);
      const rgb = hslToRgb(x, 1, 0.5);
      data[i + 0] = rgb.r;
      data[i + 1] = rgb.g;
      data[i + 2] = rgb.b;
      data[i + 3] = 255;
    }
    const transform = ctx.getTransform();
    ctx.putImageData(hueGradient, transform.e, transform.f);
    // draw pointer
    ctx.beginPath();
    ctx.rect(
      h * this.width - 2,
      0,
      4,
      this.height,
    );
    ctx.strokeStyle = "white";
    ctx.stroke();
  }
  updateColorFromMouse(x, y) {
    let [h, s, v] = this.colorCvs.currentHSV
    x = clamp(x / this.width);
    let rgb = hsvToRgb(x, s, v);
    let alpha = this.colorCvs.currentColor.slice(7, 9) || "ff";
    this.colorCvs.currentColor = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha;
    this.colorCvs.currentHSV = [x, s, v]
    this.colorCvs.currentAlpha = alpha
  }
  mousedown(x, y) {
    this._globalEvents.add("mousemove")
    this._globalEvents.add("mouseup")
    
    this.updateColorFromMouse(x, y)
    this.clicked = true;
  }
  mousemove(x, y) {
    if (this.clicked) {
      this.updateColorFromMouse(x, y)
    }
  }
  mouseup(x, y) {
    this._globalEvents.delete("mousemove")
    this._globalEvents.delete("mouseup")
    this.clicked = false
  }
}

class SaturationValueSelectionGradient extends Widget {
  constructor(width, height, x, y, colorCvs) {
    super(x, y)
    this.width = width
    this.height = height
    this.colorCvs = colorCvs
  }
  draw(ctx) {
    let mainGradient = ctx.createImageData(this.width, this.height);
    let data = mainGradient.data;
    // let { h, s, v } = hexToHsv(colorCvs.currentColor);
    let [h, s, v] = this.colorCvs.currentHSV
    for (let i = 0; i < data.length; i += 4) {
      let x = ((i / 4) % this.width) / this.width;
      let y = Math.floor(i / 4 / this.height) / this.height;
      let hue = h;
      let rgb = hsvToRgb(hue, x, 1 - y);
      data[i + 0] = rgb.r;
      data[i + 1] = rgb.g;
      data[i + 2] = rgb.b;
      data[i + 3] = 255;
    }
    const transform = ctx.getTransform();
    ctx.putImageData(mainGradient, transform.e, transform.f);
    // draw pointer
    ctx.beginPath();
    ctx.arc(
      s * this.width,
      (1 - v) * this.height,
      3,
      0,
      2 * Math.PI,
    );
    ctx.strokeStyle = "white";
    ctx.stroke();
  }
  updateColorFromMouse(x, y) {
    const [h, s, v] = this.colorCvs.currentHSV
    const _x = clamp(x / this.width);
    const _y = clamp(y / this.height);
    const rgb = hsvToRgb(h, _x, 1 - _y);
    const alpha = this.colorCvs.currentColor.slice(7, 9) || "ff";
    this.colorCvs.currentColor = rgbToHex(rgb.r, rgb.g, rgb.b) + alpha;
    this.colorCvs.currentHSV = [h, _x, 1 - _y]
    this.colorCvs.currentAlpha = alpha
  }
  
  mousedown(x, y) {
    this._globalEvents.add("mousemove")
    this._globalEvents.add("mouseup")
    this.updateColorFromMouse(x, y)
    this.clicked = true;
  }
  mousemove(x, y) {
    if (this.clicked) {
      this.updateColorFromMouse(x, y)
    }
  }
  mouseup(x, y) {
    this._globalEvents.delete("mousemove")
    this._globalEvents.delete("mouseup")
    this.clicked = false
  }
}

class AlphaSelectionBar extends Widget {
  constructor(width, height, x, y, colorCvs) {
    super(x, y)
    this.width = width
    this.height = height
    this.colorCvs = colorCvs
  }
  
  draw(ctx) {
    drawCheckerboardBackground(ctx, 0, 0, this.width, this.height, 10);
    // Vertical gradient
    const gradient = ctx.createLinearGradient( 0, 0, 0, this.height);
    gradient.addColorStop(0, `${this.colorCvs.currentColor.slice(0, 7)}ff`); // Full color at the top
    gradient.addColorStop(1, `${this.colorCvs.currentColor.slice(0, 7)}00`);
    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, this.width, this.height);
    let alpha =
    parseInt(this.colorCvs.currentColor.slice(7, 9) || "ff", 16) / 255;
    // draw pointer
    ctx.beginPath();
    ctx.rect(0, (1 - alpha) * this.height - 2, this.width, 4);
    ctx.strokeStyle = "white";
    ctx.stroke();
  }
  updateColorFromMouse(x, y) {
    y = 1 - y / this.height;
    const alpha = Math.round(clamp(y) * 255).toString(16);
    this.colorCvs.currentColor = `${this.colorCvs.currentColor.slice(0, 7)}${alpha}`;
    this.colorCvs.currentAlpha = alpha
  }
  mousedown(x, y) {
    this._globalEvents.add("mousemove")
    this._globalEvents.add("mouseup")
    this.updateColorFromMouse(x, y)
    this.clicked = true;
  }
  mousemove(x, y) {
    if (this.clicked) {
      this.updateColorFromMouse(x, y)
    }
  }
  mouseup(x, y) {
    this._globalEvents.delete("mousemove")
    this._globalEvents.delete("mouseup")
    this.clicked = false
  }
}

class ColorWidget extends Widget {
  constructor(width, height, x, y, colorCvs) {
    super(x, y)
    this.width = width
    this.height = height
    this.colorCvs = colorCvs
  }
  draw(ctx) {
    drawCheckerboardBackground(ctx, 0, 0, this.width, this.height, 10);
    ctx.fillStyle = this.colorCvs.currentColor;
    ctx.fillRect(0, 0, this.width, this.height);
  }
}
class ColorSelectorWidget extends Widget {
  constructor(x, y, colorCvs) {
    super(x, y)
    this.colorCvs = colorCvs
    const padding = 10;
    const gradwidth = 25;
    const ccwidth = 300;
    const mainSize = ccwidth - (3 * padding + gradwidth);
    this.children = [
      new ColorWidget(
        colorCvs.width - 2 * padding,
        50,
        padding,
        padding,
        colorCvs
      ),
      new HueSelectionBar(
        mainSize,
        gradwidth,
        padding,
        3 * padding + 50 + mainSize, colorCvs
      ),
      new SaturationValueSelectionGradient(
        mainSize,
        mainSize,
        padding,
        2 * padding + 50,
        colorCvs
      ),
      new AlphaSelectionBar(
        gradwidth,
        mainSize,
        colorCvs.width - (padding + gradwidth),
        2 * padding + 50,
        colorCvs
      )
    ]
  }
  draw(ctx) {
    const darkMode =
    window.matchMedia &&
    window.matchMedia("(prefers-color-scheme: dark)").matches;
    ctx.lineWidth = 2;
    if (darkMode) {
      ctx.fillStyle = "#333";
    } else {
      ctx.fillStyle = "#ccc"; //TODO
    }
    ctx.fillRect(0, 0, this.colorCvs.width, this.colorCvs.height);
    super.draw(ctx)
  }
}

class HBox extends Widget {
  constructor(x, y) {
    super(x, y)
    this.width = 0;
    this.height = 0;
  }
  add(child) {
    child.x = this.width
    child.y = 0
    this.children.push(child)
    this.width += child.width
  }
}
class VBox extends Widget {
  constructor(x, y) {
    super(x, y)
    this.width = 0;
    this.height = 0;
  }
  add(child) {
    child.x = 0
    child.y = this.height
    this.children.push(child)
    this.height += child.height
  }
}

class ScrollableWindowHeaders extends Widget {
  constructor(x, y, scrollableWindow, scrollDirection, headers) {
    this.scrollableWindow = scrollableWindow
    this.children = [this.scrollableWindow]
    if (scrollDirection & SCROLL.HORIZONTAL) {
      this.vbox = new VBox(0, headers.y)
      this.children.push(this.vbox)
    }
    if (scrollDirection & SCROLL.VERTICAL) {
      this.hbox = new HBox(0, headers.y)
      this.children.push(this.hbox)
    }
  }
  wheel(dx, dy) {
    
  }
}

class ScrollableWindow extends Widget {
  constructor(x, y) {
    super(x, y)
    this.offsetX = 0
    this.offsetY = 0
  }
  draw(ctx) {
    ctx.save()
    ctx.beginPath()
    ctx.rect(0, 0, this.width, this.height)
    ctx.clip()
    ctx.translate(this.offsetX, this.offsetY)
    this.drawContents(ctx)
    ctx.restore()
  }
  drawContents(ctx) {}
}

class TimelineWindow extends ScrollableWindow {
  constructor(x, y, context) {
    super(x, y)
    this.context = context
    this.width = 100
    this.height = 100
  }
  drawContents(ctx) {
    const startFrame = Math.floor(-this.offsetX / frameWidth)
    const frameCount = (this.width / frameWidth) + 1
    for (let k = this.context.activeObject.allLayers.length - 1; k >= 0; k--) {
      let layer = this.context.activeObject.allLayers[k];
      // if (layer instanceof Layer) {
      if (layer.frames) {
        // Draw background
        for (let j = startFrame; j < startFrame + frameCount; j++) {
          ctx.fillStyle = (j + 1) % 5 == 0 ? shade : backgroundColor;
          drawBorderedRect(
            ctx,
            j * frameWidth,
            0,
            frameWidth,
            layerHeight,
            shadow,
            highlight,
            shadow,
            shadow,
          );
        }
        // Draw frames
        for (let j=0; j<layer.frames.length; j++) {
          const frameInfo = layer.getFrameValue(j)
          if (frameInfo.valueAtN) {
            ctx.fillStyle = foregroundColor;
            drawBorderedRect(
              ctx,
              j * frameWidth,
              0,
              frameWidth,
              layerHeight,
              highlight,
              shadow,
              shadow,
              shadow,
            );
            ctx.fillStyle = "#111";
            ctx.beginPath();
            ctx.arc(
              (j + 0.5) * frameWidth,
              layerHeight * 0.75,
              frameWidth * 0.25,
              0,
              2 * Math.PI,
            );
            ctx.fill();
            if (frameInfo.valueAtN.keyTypes.has("motion")) {
              ctx.strokeStyle = "#7a00b3";
              ctx.lineWidth = 2;
              ctx.beginPath()
              ctx.moveTo(j*frameWidth, layerHeight*0.25)
              ctx.lineTo((j+1)*frameWidth, layerHeight*0.25)
              ctx.stroke()
            }
            if (frameInfo.valueAtN.keyTypes.has("shape")) {
              ctx.strokeStyle = "#9bff9b";
              ctx.lineWidth = 2;
              ctx.beginPath()
              ctx.moveTo(j*frameWidth, layerHeight*0.35)
              ctx.lineTo((j+1)*frameWidth, layerHeight*0.35)
              ctx.stroke()
            }
          } else if (frameInfo.prev && frameInfo.next) {
            ctx.fillStyle = foregroundColor;
            drawBorderedRect(
              ctx,
              j * frameWidth,
              0,
              frameWidth,
              layerHeight,
              highlight,
              shadow,
              backgroundColor,
              backgroundColor,
            );
            if (frameInfo.prev.keyTypes.has("motion")) {
              ctx.strokeStyle = "#7a00b3";
              ctx.lineWidth = 2;
              ctx.beginPath()
              ctx.moveTo(j*frameWidth, layerHeight*0.25)
              ctx.lineTo((j+1)*frameWidth, layerHeight*0.25)
              ctx.stroke()
            }
            if (frameInfo.prev.keyTypes.has("shape")) {
              ctx.strokeStyle = "#9bff9b";
              ctx.lineWidth = 2;
              ctx.beginPath()
              ctx.moveTo(j*frameWidth, layerHeight*0.35)
              ctx.lineTo((j+1)*frameWidth, layerHeight*0.35)
              ctx.stroke()
            }
          }
        }
      // } else if (layer instanceof AudioTrack) {
      } else if (layer.sounds) {
        // TODO: split waveform into chunks
        for (let i in layer.sounds) {
          let sound = layer.sounds[i];
          ctx.drawImage(sound.img, 0, 0);
        }
      }
      ctx.translate(0,layerHeight)
    }
  }
  mousedown(x, y) {

  }
}

/**
 * TimelineWindowV2 - New timeline widget using AnimationData curve-based system
 * Phase 1: Time ruler with zoom-adaptive intervals and playhead
 * Phase 2: Track hierarchy display
 */
class TimelineWindowV2 extends Widget {
  constructor(x, y, context) {
    super(x, y)
    this.context = context
    this.width = 800
    this.height = 400

    // Track header column width (fixed on left side)
    this.trackHeaderWidth = 150

    // Create shared timeline state using config framerate
    this.timelineState = new TimelineState(
      context.config?.framerate || 24,
      context.config?.bpm || 120,
      context.config?.timeSignature || { numerator: 4, denominator: 4 }
    )

    // Create time ruler widget
    this.ruler = new TimeRuler(this.timelineState)

    // Create track hierarchy manager
    this.trackHierarchy = new TrackHierarchy()

    // Track if we're dragging playhead
    this.draggingPlayhead = false

    // Vertical scroll offset for track hierarchy
    this.trackScrollOffset = 0

    // Phase 5: Curve interaction state
    this.draggingKeyframe = null  // {curve, keyframe, track}
    this.selectedKeyframes = new Set()  // Set of selected keyframe objects for multi-select

    // Hover state for showing keyframe values
    this.hoveredKeyframe = null  // {keyframe, x, y} - keyframe being hovered over and its screen position

    // Hidden curves (Phase 6) - Set of curve parameter names
    this.hiddenCurves = new Set()

    // Phase 6: Segment dragging state
    this.draggingSegment = null  // {track, initialMouseTime, segmentStartTime, animationData}

    // Phase 6: Segment edge dragging state
    this.draggingEdge = null  // {track, edge: 'left'|'right', keyframe, animationData, curveName, initialTime}

    // Phase 6: Tangent handle dragging state
    this.draggingTangent = null  // {keyframe, handle: 'in'|'out', curve, track, initialEase}

    // Phase 6: Keyframe clipboard
    this.keyframeClipboard = null  // {keyframes: [{keyframe, curve, relativeTime}], baseTime}

    // Selected audio track (for recording)
    this.selectedTrack = null

    // Cache for automation node names (maps "trackId:nodeId" -> friendly name)
    this.automationNameCache = new Map()
  }

  /**
   * Quantize a time value to the nearest beat/measure division based on zoom level.
   * Only applies when in measures mode and snapping is enabled.
   * @param {number} time - The time value to quantize (in seconds)
   * @returns {number} - The quantized time value
   */
  quantizeTime(time) {
    // Only quantize in measures mode with snapping enabled
    if (this.timelineState.timeFormat !== 'measures' || !this.timelineState.snapToFrames) {
      return time
    }

    const bpm = this.timelineState.bpm || 120
    const beatsPerSecond = bpm / 60
    const beatDuration = 1 / beatsPerSecond  // Duration of one beat in seconds
    const beatsPerMeasure = this.timelineState.timeSignature?.numerator || 4

    // Calculate beat width in pixels
    const beatWidth = beatDuration * this.timelineState.pixelsPerSecond

    // Base threshold for zoom level detection (adjustable)
    const zoomThreshold = 30

    // Determine quantization level based on zoom (beat width in pixels)
    // When zoomed out (small beat width), quantize to measures
    // When zoomed in (large beat width), quantize to smaller divisions
    let quantizeDuration
    if (beatWidth < zoomThreshold * 0.5) {
      // Very zoomed out: quantize to whole measures
      quantizeDuration = beatDuration * beatsPerMeasure
    } else if (beatWidth < zoomThreshold) {
      // Zoomed out: quantize to half measures (2 beats in 4/4)
      quantizeDuration = beatDuration * (beatsPerMeasure / 2)
    } else if (beatWidth < zoomThreshold * 2) {
      // Medium zoom: quantize to beats
      quantizeDuration = beatDuration
    } else if (beatWidth < zoomThreshold * 4) {
      // Zoomed in: quantize to half beats (eighth notes in 4/4)
      quantizeDuration = beatDuration / 2
    } else {
      // Very zoomed in: quantize to quarter beats (sixteenth notes in 4/4)
      quantizeDuration = beatDuration / 4
    }

    // Round time to nearest quantization unit
    return Math.round(time / quantizeDuration) * quantizeDuration
  }

  draw(ctx) {
    ctx.save()

    // Update time display if it exists
    if (this.context.updateTimeDisplay) {
      this.context.updateTimeDisplay();
    }

    // Draw background
    ctx.fillStyle = backgroundColor
    ctx.fillRect(0, 0, this.width, this.height)

    // Draw time ruler at top, offset by track header width
    ctx.save()
    ctx.translate(this.trackHeaderWidth, 0)
    this.ruler.draw(ctx, this.width - this.trackHeaderWidth)
    ctx.restore()

    // Phase 2: Build and draw track hierarchy
    if (this.context.activeObject) {
      this.trackHierarchy.buildTracks(this.context.activeObject)
      this.drawTrackHeaders(ctx)
      this.drawTracks(ctx)

      // Phase 3: Draw segments
      this.drawSegments(ctx)

      // Phase 4: Draw curves
      this.drawCurves(ctx)
    }

    // Draw curve mode button tooltip if hovering
    if (this.hoveredCurveModeButton) {
      const text = this.hoveredCurveModeButton.modeName

      // Measure text to size the tooltip
      ctx.font = '11px sans-serif'
      const textMetrics = ctx.measureText(text)
      const textWidth = textMetrics.width
      const tooltipPadding = 4
      const tooltipWidth = textWidth + tooltipPadding * 2
      const tooltipHeight = 16

      // Position tooltip near mouse
      let tooltipX = this.hoveredCurveModeButton.x + 10
      let tooltipY = this.hoveredCurveModeButton.y - tooltipHeight - 5

      // Clamp to stay within bounds
      if (tooltipX + tooltipWidth > this.width) {
        tooltipX = this.hoveredCurveModeButton.x - tooltipWidth - 10
      }
      if (tooltipY < 0) {
        tooltipY = this.hoveredCurveModeButton.y + 5
      }

      // Draw tooltip background
      ctx.fillStyle = backgroundColor
      ctx.fillRect(tooltipX, tooltipY, tooltipWidth, tooltipHeight)

      // Draw tooltip border
      ctx.strokeStyle = foregroundColor
      ctx.lineWidth = 1
      ctx.strokeRect(tooltipX, tooltipY, tooltipWidth, tooltipHeight)

      // Draw text
      ctx.fillStyle = labelColor
      ctx.textAlign = 'left'
      ctx.textBaseline = 'middle'
      ctx.fillText(text, tooltipX + tooltipPadding, tooltipY + tooltipHeight / 2)
    }

    ctx.restore()
  }

  /**
   * Draw fixed track headers on the left (names, expand/collapse)
   */
  drawTrackHeaders(ctx) {
    ctx.save()
    ctx.translate(0, this.ruler.height)  // Start below ruler

    // Clip to track header area
    const trackAreaHeight = this.height - this.ruler.height
    ctx.beginPath()
    ctx.rect(0, 0, this.trackHeaderWidth, trackAreaHeight)
    ctx.clip()

    // Apply vertical scroll offset
    ctx.translate(0, this.trackScrollOffset)

    const indentSize = 20  // Pixels per indent level

    for (let i = 0; i < this.trackHierarchy.tracks.length; i++) {
      const track = this.trackHierarchy.tracks[i]
      const y = this.trackHierarchy.getTrackY(i)
      const trackHeight = this.trackHierarchy.getTrackHeight(track)

      // Check if this track is selected
      const isSelected = this.isTrackSelected(track)

      // Draw track header background
      if (isSelected) {
        ctx.fillStyle = highlight
      } else {
        ctx.fillStyle = shade
      }
      ctx.fillRect(0, y, this.trackHeaderWidth, trackHeight)

      // Draw border
      ctx.strokeStyle = shadow
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, y + trackHeight)
      ctx.lineTo(this.trackHeaderWidth, y + trackHeight)
      ctx.stroke()

      // Calculate indent
      const indent = track.indent * indentSize

      // Draw expand/collapse indicator
      if (track.type === 'layer' || (track.type === 'object' && track.object.children && track.object.children.length > 0)) {
        const triangleX = indent + 8
        const triangleY = y + this.trackHierarchy.trackHeight / 2  // Use base height for triangle position

        ctx.fillStyle = foregroundColor
        ctx.beginPath()
        if (track.collapsed) {
          ctx.moveTo(triangleX, triangleY - 4)
          ctx.lineTo(triangleX + 6, triangleY)
          ctx.lineTo(triangleX, triangleY + 4)
        } else {
          ctx.moveTo(triangleX - 4, triangleY - 2)
          ctx.lineTo(triangleX + 4, triangleY - 2)
          ctx.lineTo(triangleX, triangleY + 4)
        }
        ctx.closePath()
        ctx.fill()
      }

      // Draw track name with ellipsis if needed to avoid button overlap
      ctx.fillStyle = labelColor
      ctx.font = '12px sans-serif'
      ctx.textAlign = 'left'
      ctx.textBaseline = 'middle'

      // Calculate available width for text (leave space for buttons if present)
      const textStartX = indent + 20
      let maxTextWidth = this.trackHeaderWidth - textStartX - 10  // 10px right padding

      // If this track has buttons, reserve space for them
      if (track.type === 'object' || track.type === 'shape') {
        const buttonSize = 14
        const twoButtonsWidth = (buttonSize * 2) + 4 + 10  // Two buttons + gap + padding
        maxTextWidth = this.trackHeaderWidth - textStartX - twoButtonsWidth
      } else if (track.type === 'audio') {
        const buttonSize = 14
        const oneButtonWidth = buttonSize + 10  // One button (curves mode) + padding
        maxTextWidth = this.trackHeaderWidth - textStartX - oneButtonWidth
      }

      // Truncate text with ellipsis if needed
      let displayName = track.name
      let nameWidth = ctx.measureText(displayName).width
      if (nameWidth > maxTextWidth) {
        // Add ellipsis
        while (nameWidth > maxTextWidth && displayName.length > 0) {
          displayName = displayName.slice(0, -1)
          nameWidth = ctx.measureText(displayName + '...').width
        }
        displayName += '...'
      }

      ctx.fillText(displayName, textStartX, y + this.trackHierarchy.trackHeight / 2)

      // Draw type indicator (only if there's space)
      ctx.fillStyle = foregroundColor
      ctx.font = '10px sans-serif'
      const typeText = track.type === 'layer' ? '[L]' :
                       track.type === 'object' ? '[G]' :
                       track.type === 'audio' ? '[A]' : '[S]'
      const typeX = textStartX + ctx.measureText(displayName).width + 8
      const buttonSpaceNeeded = (track.type === 'object' || track.type === 'shape') ? 50 :
                                 (track.type === 'audio') ? 25 : 10
      if (typeX + ctx.measureText(typeText).width < this.trackHeaderWidth - buttonSpaceNeeded) {
        ctx.fillText(typeText, typeX, y + this.trackHierarchy.trackHeight / 2)
      }


      // Draw MIDI activity indicator for active MIDI track
      if (track.type === 'audio' && track.object && track.object.type === 'midi') {

        if (this.context && this.context.lastMidiInputTime > 0) {

          // Check if this is the selected/active MIDI track
          const isActiveMidiTrack = isSelected && track.object && track.object.audioTrackId !== undefined


          if (isActiveMidiTrack) {
            const elapsed = Date.now() - this.context.lastMidiInputTime
            const fadeTime = 1000 // Fade out over 1 second (increased for visibility)

            if (elapsed < fadeTime) {
              const alpha = Math.max(0.2, 1 - (elapsed / fadeTime)) // Minimum alpha of 0.3 for visibility
              const indicatorSize = 10
              const indicatorX = this.trackHeaderWidth - 35 // Position to the left of buttons
              const indicatorY = y + this.trackHierarchy.trackHeight / 2


              // Draw pulsing circle with border
              ctx.strokeStyle = `rgba(0, 255, 0, ${alpha})`
              ctx.fillStyle = `rgba(0, 255, 0, ${alpha})`
              ctx.lineWidth = 2
              ctx.beginPath()
              ctx.arc(indicatorX, indicatorY, indicatorSize / 2, 0, Math.PI * 2)
              ctx.fill()
              ctx.stroke()
            }
          }
        }
      }

      // Draw toggle buttons for object/shape/audio/midi tracks (Phase 3)
      if (track.type === 'object' || track.type === 'shape' || track.type === 'audio' || track.type === 'midi') {
        const buttonSize = 14
        const buttonY = y + (this.trackHierarchy.trackHeight - buttonSize) / 2  // Use base height for button position
        let buttonX = this.trackHeaderWidth - 10  // Start from right edge

        // Curves mode button (rightmost)
        buttonX -= buttonSize
        ctx.strokeStyle = foregroundColor
        ctx.lineWidth = 1
        ctx.strokeRect(buttonX, buttonY, buttonSize, buttonSize)

        // Draw symbol based on curves mode
        ctx.fillStyle = foregroundColor
        ctx.font = '10px sans-serif'
        ctx.textAlign = 'center'
        ctx.textBaseline = 'middle'
        const curveSymbol = track.object.curvesMode === 'curve' ? '~' :
                           track.object.curvesMode === 'keyframe' ? '≈' : '-'
        ctx.fillText(curveSymbol, buttonX + buttonSize / 2, buttonY + buttonSize / 2)

        // Segment visibility button (only for object/shape tracks, not audio/midi)
        if (track.type !== 'audio' && track.type !== 'midi') {
          buttonX -= (buttonSize + 4)
          ctx.strokeStyle = foregroundColor
          ctx.lineWidth = 1
          ctx.strokeRect(buttonX, buttonY, buttonSize, buttonSize)

          // Fill if segment is visible
          if (track.object.showSegment) {
            ctx.fillStyle = foregroundColor
            ctx.fillRect(buttonX + 2, buttonY + 2, buttonSize - 4, buttonSize - 4)
          }
        }

        // Draw legend for expanded curves (Phase 6)
        if (track.object.curvesMode === 'curve') {
          // Get curves for this track
          const curves = []
          const obj = track.object
          let animationData = null

          // Find the AnimationData for this track
          if (track.type === 'audio' || track.type === 'midi') {
            // For audio/MIDI tracks, animation data is directly on the track object
            animationData = obj.animationData
          } else if (track.type === 'object') {
            for (let layer of this.context.activeObject.allLayers) {
              if (layer.children && layer.children.includes(obj)) {
                animationData = layer.animationData
                break
              }
            }
          } else if (track.type === 'shape') {
            for (let layer of this.context.activeObject.allLayers) {
              if (layer.shapes && layer.shapes.some(s => s.shapeId === obj.shapeId)) {
                animationData = layer.animationData
                break
              }
            }
          }

          if (animationData) {
            if (track.type === 'audio' || track.type === 'midi') {
              // For audio/MIDI tracks, include all automation curves
              for (let curveName in animationData.curves) {
                curves.push(animationData.curves[curveName])
              }
            } else {
              // For objects/shapes, filter by prefix
              const prefix = track.type === 'object' ? `child.${obj.idx}.` : `shape.${obj.shapeId}.`
              for (let curveName in animationData.curves) {
                if (curveName.startsWith(prefix)) {
                  curves.push(animationData.curves[curveName])
                }
              }
            }
          }

          if (curves.length > 0) {
            ctx.save()
            const legendPadding = 3
            const legendLineHeight = 12
            const legendHeight = curves.length * legendLineHeight + legendPadding * 2
            const legendY = y + this.trackHierarchy.trackHeight + 5  // Below track name row

            // Draw legend items (no background box)
            ctx.font = '9px sans-serif'
            ctx.textAlign = 'left'
            ctx.textBaseline = 'top'

            for (let i = 0; i < curves.length; i++) {
              const curve = curves[i]
              const itemY = legendY + legendPadding + i * legendLineHeight
              const isHidden = this.hiddenCurves.has(curve.parameter)

              // Draw color dot (grayed out if hidden)
              ctx.fillStyle = isHidden ? foregroundColor : curve.displayColor
              ctx.beginPath()
              ctx.arc(10, itemY + 5, 3, 0, 2 * Math.PI)
              ctx.fill()

              // Draw parameter name
              ctx.fillStyle = isHidden ? foregroundColor : labelColor
              let paramName = curve.parameter.split('.').pop()

              // For automation curves, fetch the friendly name from backend
              if (curve.parameter.startsWith('automation.') && (track.type === 'audio' || track.type === 'midi')) {
                const nodeId = parseInt(paramName, 10)
                if (!isNaN(nodeId) && obj.audioTrackId !== null) {
                  paramName = this.getAutomationName(obj.audioTrackId, nodeId)
                }
              }

              const truncatedName = paramName.length > 12 ? paramName.substring(0, 10) + '...' : paramName
              ctx.fillText(truncatedName, 18, itemY)

              // Draw strikethrough if hidden
              if (isHidden) {
                ctx.strokeStyle = foregroundColor
                ctx.lineWidth = 1
                ctx.beginPath()
                const textWidth = ctx.measureText(truncatedName).width
                ctx.moveTo(18, itemY + 5)
                ctx.lineTo(18 + textWidth, itemY + 5)
                ctx.stroke()
              }
            }
            ctx.restore()
          }
        }
      }
    }

    // Draw right border of header column
    ctx.strokeStyle = shadow
    ctx.lineWidth = 2
    ctx.beginPath()
    ctx.moveTo(this.trackHeaderWidth, 0)
    ctx.lineTo(this.trackHeaderWidth, this.trackHierarchy.getTotalHeight())
    ctx.stroke()

    ctx.restore()
  }

  /**
   * Draw track backgrounds in timeline area (Phase 2)
   */
  // Create a cached pattern for the timeline grid
  createTimelinePattern(trackHeight) {
    const cacheKey = `${this.timelineState.timeFormat}_${this.timelineState.pixelsPerSecond}_${this.timelineState.framerate}_${this.timelineState.bpm}_${trackHeight}`

    // Return cached pattern if available
    if (this.cachedPattern && this.cachedPatternKey === cacheKey) {
      return this.cachedPattern
    }

    let patternWidth, patternHeight = trackHeight

    if (this.timelineState.timeFormat === 'frames') {
      // Pattern for 5 frames
      const frameDuration = 1 / this.timelineState.framerate
      const frameWidth = frameDuration * this.timelineState.pixelsPerSecond
      patternWidth = frameWidth * 5
    } else if (this.timelineState.timeFormat === 'measures') {
      // Pattern for one measure
      const beatsPerSecond = this.timelineState.bpm / 60
      const beatsPerMeasure = this.timelineState.timeSignature.numerator
      const beatWidth = (1 / beatsPerSecond) * this.timelineState.pixelsPerSecond
      patternWidth = beatWidth * beatsPerMeasure
    } else {
      // Pattern for seconds - use 10 second intervals
      patternWidth = this.timelineState.pixelsPerSecond * 10
    }

    // Create pattern canvas
    const patternCanvas = document.createElement('canvas')
    patternCanvas.width = Math.ceil(patternWidth)
    patternCanvas.height = patternHeight
    const pctx = patternCanvas.getContext('2d')

    // Fill background
    pctx.fillStyle = shade
    pctx.fillRect(0, 0, patternWidth, patternHeight)

    if (this.timelineState.timeFormat === 'frames') {
      const frameDuration = 1 / this.timelineState.framerate
      const frameWidth = frameDuration * this.timelineState.pixelsPerSecond

      for (let i = 0; i < 5; i++) {
        const x = i * frameWidth
        if (i === 0) {
          // First frame in pattern (every 5th): shade it
          pctx.fillStyle = shadow
          pctx.fillRect(x, 0, frameWidth, patternHeight)
        } else {
          // Regular frame: draw edge line
          pctx.strokeStyle = shadow
          pctx.lineWidth = 1
          pctx.beginPath()
          pctx.moveTo(x, 0)
          pctx.lineTo(x, patternHeight)
          pctx.stroke()
        }
      }
    } else if (this.timelineState.timeFormat === 'measures') {
      const beatsPerSecond = this.timelineState.bpm / 60
      const beatsPerMeasure = this.timelineState.timeSignature.numerator
      const beatWidth = (1 / beatsPerSecond) * this.timelineState.pixelsPerSecond

      for (let i = 0; i < beatsPerMeasure; i++) {
        const x = i * beatWidth
        const isMeasureBoundary = i === 0
        const isEvenBeat = (i % 2) === 0

        pctx.save()
        if (isMeasureBoundary) {
          pctx.globalAlpha = 1.0
        } else if (isEvenBeat) {
          pctx.globalAlpha = 0.5
        } else {
          pctx.globalAlpha = 0.25
        }

        pctx.strokeStyle = shadow
        pctx.lineWidth = 1
        pctx.beginPath()
        pctx.moveTo(x, 0)
        pctx.lineTo(x, patternHeight)
        pctx.stroke()
        pctx.restore()
      }
    } else {
      // Seconds mode: draw lines every second for 10 seconds
      const secondWidth = this.timelineState.pixelsPerSecond

      for (let i = 0; i < 10; i++) {
        const x = i * secondWidth
        pctx.strokeStyle = shadow
        pctx.lineWidth = 1
        pctx.beginPath()
        pctx.moveTo(x, 0)
        pctx.lineTo(x, patternHeight)
        pctx.stroke()
      }
    }

    // Cache the pattern
    this.cachedPatternKey = cacheKey
    this.cachedPattern = pctx.createPattern(patternCanvas, 'repeat')

    return this.cachedPattern
  }

  drawTracks(ctx) {
    ctx.save()
    ctx.translate(this.trackHeaderWidth, this.ruler.height)  // Start after headers, below ruler

    // Clip to available track area
    const trackAreaHeight = this.height - this.ruler.height
    const trackAreaWidth = this.width - this.trackHeaderWidth
    ctx.beginPath()
    ctx.rect(0, 0, trackAreaWidth, trackAreaHeight)
    ctx.clip()

    // Apply vertical scroll offset
    ctx.translate(0, this.trackScrollOffset)

    for (let i = 0; i < this.trackHierarchy.tracks.length; i++) {
      const track = this.trackHierarchy.tracks[i]
      const y = this.trackHierarchy.getTrackY(i)
      const trackHeight = this.trackHierarchy.getTrackHeight(track)

      // Create and apply pattern for this track
      const pattern = this.createTimelinePattern(trackHeight)

      // Calculate pattern offset based on viewport start time
      const visibleStartTime = this.timelineState.viewportStartTime
      const patternOffsetX = -this.timelineState.timeToPixel(visibleStartTime)

      ctx.save()
      ctx.translate(patternOffsetX, y)
      ctx.fillStyle = pattern
      ctx.fillRect(-patternOffsetX, 0, trackAreaWidth, trackHeight)
      ctx.restore()

      // Draw track border
      ctx.strokeStyle = shadow
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, y + trackHeight)
      ctx.lineTo(trackAreaWidth, y + trackHeight)
      ctx.stroke()
    }

    ctx.restore()
  }

  /**
   * Draw segments for shapes (Phase 3)
   * Segments show the lifetime of shapes based on their exists curve keyframes
   */
  drawSegments(ctx) {
    ctx.save()
    ctx.translate(this.trackHeaderWidth, this.ruler.height)  // Start after headers, below ruler

    // Clip to available track area
    const trackAreaHeight = this.height - this.ruler.height
    const trackAreaWidth = this.width - this.trackHeaderWidth
    ctx.beginPath()
    ctx.rect(0, 0, trackAreaWidth, trackAreaHeight)
    ctx.clip()

    // Apply vertical scroll offset
    ctx.translate(0, this.trackScrollOffset)

    const frameDuration = 1 / this.timelineState.framerate
    const minSegmentDuration = frameDuration  // Minimum 1 frame

    // Iterate through tracks and draw segments
    for (let i = 0; i < this.trackHierarchy.tracks.length; i++) {
      const track = this.trackHierarchy.tracks[i]

      if (track.type === 'object') {
        // Draw segments for GraphicsObjects (groups) using frameNumber curve
        const obj = track.object

        // Skip if segment is hidden (Phase 3)
        if (!obj.showSegment) continue

        const y = this.trackHierarchy.getTrackY(i)
        const trackHeight = this.trackHierarchy.trackHeight  // Use base height for segment

        // Find the parent layer that contains this object
        let parentLayer = null
        for (let layer of this.context.activeObject.allLayers) {
          if (layer.children && layer.children.includes(obj)) {
            parentLayer = layer
            break
          }
        }

        if (!parentLayer || !parentLayer.animationData) continue

        // Get the frameNumber curve for this object
        const frameNumberKey = `child.${obj.idx}.frameNumber`
        const frameNumberCurve = parentLayer.animationData.curves[frameNumberKey]

        if (!frameNumberCurve || !frameNumberCurve.keyframes || frameNumberCurve.keyframes.length === 0) continue

        // Build segments from consecutive keyframes where frameNumber > 0
        let segmentStart = null
        for (let j = 0; j < frameNumberCurve.keyframes.length; j++) {
          const keyframe = frameNumberCurve.keyframes[j]

          if (keyframe.value > 0) {
            // Start of a new segment or continuation
            if (segmentStart === null) {
              segmentStart = keyframe.time
            }

            // Check if this is the last keyframe or if the next one ends the segment
            const isLast = (j === frameNumberCurve.keyframes.length - 1)
            const nextEndsSegment = !isLast && frameNumberCurve.keyframes[j + 1].value === 0

            if (isLast || nextEndsSegment) {
              // End of segment - draw it
              const segmentEnd = nextEndsSegment ? frameNumberCurve.keyframes[j + 1].time : keyframe.time + minSegmentDuration

              const startX = this.timelineState.timeToPixel(segmentStart)
              const endX = this.timelineState.timeToPixel(segmentEnd)
              const segmentWidth = Math.max(endX - startX, this.timelineState.pixelsPerSecond * minSegmentDuration)

              // Draw segment with object's color
              ctx.fillStyle = obj.segmentColor
              ctx.fillRect(
                startX,
                y + 5,
                segmentWidth,
                trackHeight - 10
              )

              // Draw border
              ctx.strokeStyle = shadow
              ctx.lineWidth = 1
              ctx.strokeRect(
                startX,
                y + 5,
                segmentWidth,
                trackHeight - 10
              )

              // Draw object name if there's enough space
              const minWidthForLabel = 40  // Minimum pixels to show label
              if (segmentWidth >= minWidthForLabel) {
                ctx.fillStyle = labelColor
                ctx.font = '11px sans-serif'
                ctx.textAlign = 'left'
                ctx.textBaseline = 'middle'

                // Clip text to segment bounds
                ctx.save()
                ctx.beginPath()
                ctx.rect(startX + 2, y + 5, segmentWidth - 4, trackHeight - 10)
                ctx.clip()

                ctx.fillText(obj.name, startX + 4, y + trackHeight / 2)
                ctx.restore()
              }

              segmentStart = null  // Reset for next segment
            }
          }
        }
      } else if (track.type === 'shape') {
        const shape = track.object

        // Skip if segment is hidden (Phase 3)
        if (!shape.showSegment) continue

        const y = this.trackHierarchy.getTrackY(i)
        const trackHeight = this.trackHierarchy.trackHeight  // Use base height for segment

        // Find the layer this shape belongs to (including nested layers in groups)
        let shapeLayer = null
        const findShapeLayer = (obj) => {
          for (let layer of obj.children) {
            if (layer.shapes && layer.shapes.includes(shape)) {
              shapeLayer = layer
              return true
            }
            // Recursively search in child objects
            if (layer.children) {
              for (let child of layer.children) {
                if (findShapeLayer(child)) return true
              }
            }
          }
          return false
        }
        findShapeLayer(this.context.activeObject)

        if (!shapeLayer || !shapeLayer.animationData) continue

        // Get the exists curve for this shape (using shapeId, not idx)
        const existsCurveKey = `shape.${shape.shapeId}.exists`
        const existsCurve = shapeLayer.animationData.curves[existsCurveKey]

        if (!existsCurve || !existsCurve.keyframes || existsCurve.keyframes.length === 0) continue

        // Build segments from consecutive keyframes where exists > 0
        let segmentStart = null
        for (let j = 0; j < existsCurve.keyframes.length; j++) {
          const keyframe = existsCurve.keyframes[j]

          if (keyframe.value > 0) {
            // Start of a new segment or continuation
            if (segmentStart === null) {
              segmentStart = keyframe.time
            }

            // Check if this is the last keyframe or if the next one ends the segment
            const isLast = (j === existsCurve.keyframes.length - 1)
            const nextEndsSegment = !isLast && existsCurve.keyframes[j + 1].value === 0

            if (isLast || nextEndsSegment) {
              // End of segment - draw it
              const segmentEnd = nextEndsSegment ? existsCurve.keyframes[j + 1].time : keyframe.time + minSegmentDuration

              const startX = this.timelineState.timeToPixel(segmentStart)
              const endX = this.timelineState.timeToPixel(segmentEnd)
              const segmentWidth = Math.max(endX - startX, this.timelineState.pixelsPerSecond * minSegmentDuration)

              // Draw segment with shape's color
              ctx.fillStyle = shape.segmentColor
              ctx.fillRect(
                startX,
                y + 5,
                segmentWidth,
                trackHeight - 10
              )

              // Draw border
              ctx.strokeStyle = shadow
              ctx.lineWidth = 1
              ctx.strokeRect(
                startX,
                y + 5,
                segmentWidth,
                trackHeight - 10
              )

              // Draw shape name (constructor name) if there's enough space
              const minWidthForLabel = 50  // Minimum pixels to show label
              if (segmentWidth >= minWidthForLabel) {
                const shapeName = shape.constructor.name || 'Shape'
                ctx.fillStyle = labelColor
                ctx.font = '11px sans-serif'
                ctx.textAlign = 'left'
                ctx.textBaseline = 'middle'

                // Clip text to segment bounds
                ctx.save()
                ctx.beginPath()
                ctx.rect(startX + 2, y + 5, segmentWidth - 4, trackHeight - 10)
                ctx.clip()

                ctx.fillText(shapeName, startX + 4, y + trackHeight / 2)
                ctx.restore()
              }

              segmentStart = null  // Reset for next segment
            }
          }
        }
      } else if (track.type === 'audio') {
        // Draw audio clips for AudioTrack
        const audioTrack = track.object
        const y = this.trackHierarchy.getTrackY(i)
        const trackHeight = this.trackHierarchy.trackHeight  // Use base height for clips

        // Draw each clip
        for (let clip of audioTrack.clips) {
          const startX = this.timelineState.timeToPixel(clip.startTime)
          const endX = this.timelineState.timeToPixel(clip.startTime + clip.duration)
          const clipWidth = endX - startX

          // Determine clip color based on track type
          const isMIDI = audioTrack.type === 'midi'
          let clipColor
          if (clip.loading) {
            clipColor = '#666666'  // Gray for loading
          } else if (isMIDI) {
            clipColor = '#2d5016'  // Dark green background for MIDI clips
          } else {
            clipColor = '#4a90e2'  // Blue for audio clips
          }

          // Draw clip rectangle
          ctx.fillStyle = clipColor
          ctx.fillRect(
            startX,
            y + 5,
            clipWidth,
            trackHeight - 10
          )

          // Draw border
          ctx.strokeStyle = shadow
          ctx.lineWidth = 1
          ctx.strokeRect(
            startX,
            y + 5,
            clipWidth,
            trackHeight - 10
          )

          // Highlight selected MIDI clip
          if (isMIDI && context.pianoRollEditor && clip.clipId === context.pianoRollEditor.selectedClipId) {
            ctx.strokeStyle = '#6fdc6f'  // Bright green for selected MIDI clip
            ctx.lineWidth = 2
            ctx.strokeRect(
              startX,
              y + 5,
              clipWidth,
              trackHeight - 10
            )
          }

          // Draw clip name if there's enough space
          const minWidthForLabel = 40
          if (clipWidth >= minWidthForLabel) {
            ctx.fillStyle = labelColor
            ctx.font = '11px sans-serif'
            ctx.textAlign = 'left'
            ctx.textBaseline = 'middle'

            // Clip text to clip bounds
            ctx.save()
            ctx.beginPath()
            ctx.rect(startX + 2, y + 5, clipWidth - 4, trackHeight - 10)
            ctx.clip()

            ctx.fillText(clip.name, startX + 4, y + trackHeight / 2)
            ctx.restore()
          }

          // Draw MIDI clip visualization (piano roll bars) or audio waveform
          if (!clip.loading) {
            if (isMIDI && clip.notes && clip.notes.length > 0) {
              // Draw piano roll notes for MIDI clips
              // Divide track height by 12 to represent chromatic notes (C, C#, D, etc.)
              // Leave 2px padding at top and bottom
              const verticalPadding = 2
              const availableHeight = trackHeight - 10 - (verticalPadding * 2)
              const noteHeight = availableHeight / 12

              // Get clip trim boundaries (internal_start = offset, internal_end depends on source)
              const clipOffset = clip.offset || 0
              // Use stored internalDuration if available (set when trimming), otherwise calculate from notes
              let internalDuration
              if (clip.internalDuration !== undefined) {
                internalDuration = clip.internalDuration
              } else {
                // Fallback: calculate from actual notes (for clips that haven't been trimmed)
                let contentEndTime = clipOffset
                for (const note of clip.notes) {
                  const noteEnd = note.start_time + note.duration
                  if (noteEnd > contentEndTime) {
                    contentEndTime = noteEnd
                  }
                }
                internalDuration = contentEndTime - clipOffset
              }
              const contentEndTime = clipOffset + internalDuration
              // If clip.duration exceeds internal duration, we're looping
              const isLooping = clip.duration > internalDuration && internalDuration > 0

              // Calculate visible time range within the clip (in clip-local time)
              const clipEndX = startX + clipWidth
              const visibleStartTime = this.timelineState.pixelToTime(Math.max(startX, 0)) - clip.startTime
              const visibleEndTime = this.timelineState.pixelToTime(Math.min(clipEndX, this.width)) - clip.startTime

              // Helper function to draw notes for a given loop iteration
              const drawNotesForIteration = (loopOffset, opacity) => {
                ctx.fillStyle = opacity < 1 ? `rgba(111, 220, 111, ${opacity})` : '#6fdc6f'

                for (let i = 0; i < clip.notes.length; i++) {
                  const note = clip.notes[i]
                  const noteEndTime = note.start_time + note.duration

                  // Skip notes that are outside the trimmed region
                  if (noteEndTime <= clipOffset || note.start_time >= contentEndTime) {
                    continue
                  }

                  // Calculate note position in this loop iteration
                  const noteDisplayStart = note.start_time - clipOffset + loopOffset
                  const noteDisplayEnd = noteEndTime - clipOffset + loopOffset

                  // Skip if this iteration's note is beyond clip duration
                  if (noteDisplayStart >= clip.duration) {
                    continue
                  }

                  // Exit early if note starts after visible range
                  if (noteDisplayStart > visibleEndTime) {
                    continue
                  }

                  // Skip if note ends before visible range
                  if (noteDisplayEnd < visibleStartTime) {
                    continue
                  }

                  // Calculate note position (pitch mod 12 for chromatic representation)
                  const pitchClass = note.note % 12
                  // Invert Y so higher pitches appear at top
                  const noteY = y + 5 + ((11 - pitchClass) * noteHeight)

                  // Calculate note timing on timeline
                  const noteStartX = this.timelineState.timeToPixel(clip.startTime + noteDisplayStart)
                  let noteEndX = this.timelineState.timeToPixel(clip.startTime + Math.min(noteDisplayEnd, clip.duration))

                  // Clip to visible bounds
                  const visibleStartX = Math.max(noteStartX, startX + 2)
                  const visibleEndX = Math.min(noteEndX, startX + clipWidth - 2)
                  const visibleWidth = visibleEndX - visibleStartX

                  if (visibleWidth > 0) {
                    // Draw note rectangle
                    ctx.fillRect(
                      visibleStartX,
                      noteY,
                      visibleWidth,
                      noteHeight - 1  // Small gap between notes
                    )
                  }
                }
              }

              // Draw primary notes at full opacity
              drawNotesForIteration(0, 1.0)

              // Draw looped iterations at 50% opacity
              if (isLooping) {
                let loopOffset = internalDuration
                while (loopOffset < clip.duration) {
                  drawNotesForIteration(loopOffset, 0.5)
                  loopOffset += internalDuration
                }
              }
            } else if (!isMIDI && clip.waveform && clip.waveform.length > 0) {
              // Draw waveform for audio clips
            ctx.fillStyle = 'rgba(255, 255, 255, 0.3)'

            // Only draw waveform within visible area
            const visibleStart = Math.max(startX + 2, 0)
            const visibleEnd = Math.min(startX + clipWidth - 2, this.width - this.trackHeaderWidth)

            if (visibleEnd > visibleStart) {
              const centerY = y + trackHeight / 2
              const waveformHeight = trackHeight - 14  // Leave padding at top/bottom
              const waveformData = clip.waveform

              // Calculate the full source audio duration and pixels per peak based on that
              const sourceDuration = clip.sourceDuration || clip.duration
              const pixelsPerSecond = this.timelineState.pixelsPerSecond
              const fullSourceWidth = sourceDuration * pixelsPerSecond
              const pixelsPerPeak = fullSourceWidth / waveformData.length

              // Calculate which peak corresponds to the clip's offset (trimmed left edge)
              const offsetPeakIndex = Math.floor((clip.offset / sourceDuration) * waveformData.length)

              // Calculate the range of visible peaks, accounting for offset
              const firstVisiblePeak = Math.max(offsetPeakIndex, Math.floor((visibleStart - startX) / pixelsPerPeak) + offsetPeakIndex)
              const lastVisiblePeak = Math.min(waveformData.length - 1, Math.ceil((visibleEnd - startX) / pixelsPerPeak) + offsetPeakIndex)

              // Draw waveform as a filled path
              ctx.beginPath()

              // Trace along the max values (left to right)
              for (let i = firstVisiblePeak; i <= lastVisiblePeak; i++) {
                const peakX = startX + ((i - offsetPeakIndex) * pixelsPerPeak)
                const peak = waveformData[i]
                const maxY = centerY + (peak.max * waveformHeight * 0.5)

                if (i === firstVisiblePeak) {
                  ctx.moveTo(peakX, maxY)
                } else {
                  ctx.lineTo(peakX, maxY)
                }
              }

              // Trace back along the min values (right to left)
              for (let i = lastVisiblePeak; i >= firstVisiblePeak; i--) {
                const peakX = startX + ((i - offsetPeakIndex) * pixelsPerPeak)
                const peak = waveformData[i]
                const minY = centerY + (peak.min * waveformHeight * 0.5)
                ctx.lineTo(peakX, minY)
              }

              ctx.closePath()
              ctx.fill()
            }
            }
          }
        }
      } else if (track.type === 'video') {
        // Draw video clips for VideoLayer
        const videoLayer = track.object
        const y = this.trackHierarchy.getTrackY(i)
        const trackHeight = this.trackHierarchy.trackHeight  // Use base height for clips

        // Draw each clip
        for (let clip of videoLayer.clips) {
          const startX = this.timelineState.timeToPixel(clip.startTime)
          const endX = this.timelineState.timeToPixel(clip.startTime + clip.duration)
          const clipWidth = endX - startX

          // Video clips use purple/magenta color
          const clipColor = '#9b59b6'  // Purple for video clips

          // Draw clip rectangle
          ctx.fillStyle = clipColor
          ctx.fillRect(
            startX,
            y + 5,
            clipWidth,
            trackHeight - 10
          )

          // Draw border
          ctx.strokeStyle = shadow
          ctx.lineWidth = 1
          ctx.strokeRect(
            startX,
            y + 5,
            clipWidth,
            trackHeight - 10
          )

          // Draw clip name if there's enough space
          const minWidthForLabel = 40
          if (clipWidth >= minWidthForLabel) {
            ctx.fillStyle = labelColor
            ctx.font = '11px sans-serif'
            ctx.textAlign = 'left'
            ctx.textBaseline = 'middle'

            // Clip text to clip bounds
            ctx.save()
            ctx.beginPath()
            ctx.rect(startX + 2, y + 5, clipWidth - 4, trackHeight - 10)
            ctx.clip()

            ctx.fillText(clip.name, startX + 4, y + trackHeight / 2)
            ctx.restore()
          }
        }
      }
    }

    ctx.restore()
  }

  /**
   * Draw curves for animation parameters (Phase 4)
   * Shows keyframe dots in minimized mode, full curves in expanded mode
   */
  drawCurves(ctx) {
    ctx.save()
    ctx.translate(this.trackHeaderWidth, this.ruler.height)  // Start after headers, below ruler

    // Clip to available track area
    const trackAreaHeight = this.height - this.ruler.height
    const trackAreaWidth = this.width - this.trackHeaderWidth
    ctx.beginPath()
    ctx.rect(0, 0, trackAreaWidth, trackAreaHeight)
    ctx.clip()

    // Apply vertical scroll offset
    ctx.translate(0, this.trackScrollOffset)

    // Iterate through tracks and draw curves
    for (let i = 0; i < this.trackHierarchy.tracks.length; i++) {
      const track = this.trackHierarchy.tracks[i]

      // Only draw curves for objects, shapes, audio tracks, and MIDI tracks
      if (track.type !== 'object' && track.type !== 'shape' && track.type !== 'audio' && track.type !== 'midi') continue

      const obj = track.object

      // Skip if curves are hidden
      if (obj.curvesMode === 'segment') continue

      const y = this.trackHierarchy.getTrackY(i)

      // Find the layer containing this object/shape to get AnimationData
      let animationData = null
      if (track.type === 'audio' || track.type === 'midi') {
        // For audio/MIDI tracks, animation data is directly on the track object
        animationData = obj.animationData
      } else if (track.type === 'object') {
        // For objects, get curves from parent layer
        for (let layer of this.context.activeObject.allLayers) {
          if (layer.children && layer.children.includes(obj)) {
            animationData = layer.animationData
            break
          }
        }
      } else if (track.type === 'shape') {
        // For shapes, find the layer recursively
        const findShapeLayer = (searchObj) => {
          for (let layer of searchObj.children) {
            if (layer.shapes && layer.shapes.includes(obj)) {
              animationData = layer.animationData
              return true
            }
            if (layer.children) {
              for (let child of layer.children) {
                if (findShapeLayer(child)) return true
              }
            }
          }
          return false
        }
        findShapeLayer(this.context.activeObject)
      }

      if (!animationData) continue

      // Get all curves for this object/shape/audio
      const curves = []
      for (let curveName in animationData.curves) {
        const curve = animationData.curves[curveName]

        // Filter to only curves for this specific object/shape/audio/MIDI
        if (track.type === 'audio' || track.type === 'midi') {
          // Audio/MIDI tracks: include all automation curves
          curves.push(curve)
        } else if (track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
          curves.push(curve)
        } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.shapeId}.`)) {
          curves.push(curve)
        }
      }

      if (curves.length === 0) continue

      // Draw based on curves mode
      if (obj.curvesMode === 'keyframe') {
        this.drawMinimizedCurves(ctx, curves, y)
      } else if (obj.curvesMode === 'curve') {
        this.drawExpandedCurves(ctx, curves, y)
      }
    }

    ctx.restore()
  }

  /**
   * Draw minimized curves (keyframe dots only) - Phase 6: Compact overlay mode
   * All keyframes are overlaid at the same vertical position (on the segment bar)
   */
  drawMinimizedCurves(ctx, curves, trackY) {
    const dotRadius = 3
    const yPosition = trackY + (this.trackHierarchy.trackHeight / 2)  // Center vertically in track

    // Draw keyframe dots for each curve, color-coded but overlaid
    for (let curve of curves) {
      ctx.fillStyle = curve.displayColor
      ctx.strokeStyle = shadow
      ctx.lineWidth = 1

      for (let keyframe of curve.keyframes) {
        const x = this.timelineState.timeToPixel(keyframe.time)

        // Draw with outline for better visibility when overlapping
        ctx.beginPath()
        ctx.arc(x, yPosition, dotRadius, 0, 2 * Math.PI)
        ctx.fill()
        ctx.stroke()
      }
    }
  }

  /**
   * Draw expanded curves (full Bezier visualization)
   */
  drawExpandedCurves(ctx, curves, trackY) {
    const curveHeight = 80  // Height allocated for curve visualization
    const startY = trackY + 10  // Start below segment area
    const padding = 5

    // Calculate value range across all curves for auto-scaling
    let minValue = Infinity
    let maxValue = -Infinity

    for (let curve of curves) {
      for (let keyframe of curve.keyframes) {
        minValue = Math.min(minValue, keyframe.value)
        maxValue = Math.max(maxValue, keyframe.value)
      }
    }

    // Add padding to the range
    const valueRange = maxValue - minValue
    const rangePadding = valueRange * 0.1 || 1  // 10% padding, or 1 if range is 0
    minValue -= rangePadding
    maxValue += rangePadding

    // Draw background for curve area
    ctx.fillStyle = shade
    ctx.fillRect(0, startY, this.width - this.trackHeaderWidth, curveHeight)

    // Draw grid lines
    ctx.strokeStyle = shadow
    ctx.lineWidth = 1

    // Horizontal grid lines (value axis)
    for (let i = 0; i <= 4; i++) {
      const y = startY + padding + (i * (curveHeight - 2 * padding) / 4)
      ctx.beginPath()
      ctx.moveTo(0, y)
      ctx.lineTo(this.width - this.trackHeaderWidth, y)
      ctx.stroke()
    }

    // Helper function to convert value to Y position
    const valueToY = (value) => {
      const normalizedValue = (value - minValue) / (maxValue - minValue)
      return startY + curveHeight - padding - (normalizedValue * (curveHeight - 2 * padding))
    }

    // Draw each curve
    for (let curve of curves) {
      if (curve.keyframes.length === 0) continue
      if (this.hiddenCurves.has(curve.parameter)) continue  // Skip hidden curves

      ctx.strokeStyle = curve.displayColor
      ctx.fillStyle = curve.displayColor
      ctx.lineWidth = 2

      // Draw keyframe dots
      for (let keyframe of curve.keyframes) {
        const x = this.timelineState.timeToPixel(keyframe.time)
        const y = valueToY(keyframe.value)

        // Draw selected keyframes 50% bigger
        const isSelected = this.selectedKeyframes.has(keyframe)
        const radius = isSelected ? 6 : 4

        ctx.beginPath()
        ctx.arc(x, y, radius, 0, 2 * Math.PI)
        ctx.fill()
      }

      // Handle single keyframe case - draw horizontal hold line
      if (curve.keyframes.length === 1) {
        const keyframe = curve.keyframes[0]
        const keyframeX = this.timelineState.timeToPixel(keyframe.time)
        const keyframeY = valueToY(keyframe.value)

        // Draw horizontal line extending to the right edge of visible area
        const rightEdge = this.width - this.trackHeaderWidth

        ctx.beginPath()
        ctx.moveTo(keyframeX, keyframeY)
        ctx.lineTo(rightEdge, keyframeY)
        ctx.stroke()

        // Optionally draw a lighter line extending to the left if keyframe is after t=0
        if (keyframe.time > 0) {
          ctx.strokeStyle = curve.displayColor + '40'  // More transparent
          ctx.beginPath()
          ctx.moveTo(0, keyframeY)
          ctx.lineTo(keyframeX, keyframeY)
          ctx.stroke()

          // Reset stroke style
          ctx.strokeStyle = curve.displayColor
        }
      }

      // Draw curves between keyframes based on interpolation mode
      for (let i = 0; i < curve.keyframes.length - 1; i++) {
        const kf1 = curve.keyframes[i]
        const kf2 = curve.keyframes[i + 1]

        const x1 = this.timelineState.timeToPixel(kf1.time)
        const y1 = valueToY(kf1.value)
        const x2 = this.timelineState.timeToPixel(kf2.time)
        const y2 = valueToY(kf2.value)

        // Draw based on interpolation mode
        ctx.beginPath()
        ctx.moveTo(x1, y1)

        switch (kf1.interpolation) {
          case 'linear':
            // Draw straight line
            ctx.lineTo(x2, y2)
            ctx.stroke()
            break

          case 'step':
          case 'hold':
            // Draw horizontal hold line then vertical jump
            ctx.lineTo(x2, y1)
            ctx.lineTo(x2, y2)
            ctx.stroke()
            break

          case 'zero':
            // Draw line to zero, hold at zero, then line to next value
            const zeroY = valueToY(0)
            ctx.lineTo(x1, zeroY)
            ctx.lineTo(x2, zeroY)
            ctx.lineTo(x2, y2)
            ctx.stroke()
            break

          case 'bezier':
          default:
            // Calculate control points for Bezier curve using easeIn/easeOut
            // easeIn/easeOut are like CSS cubic-bezier: {x: 0-1, y: 0-1}
            const dx = x2 - x1
            const dy = y2 - y1

            // Use default ease if not specified
            const easeOut = kf1.easeOut || { x: 0.42, y: 0 }
            const easeIn = kf2.easeIn || { x: 0.58, y: 1 }

            // Calculate control points
            // easeOut.x controls horizontal offset from kf1, easeOut.y controls vertical
            const cp1x = x1 + (easeOut.x * dx)
            const cp1y = y1 + (easeOut.y * dy)

            // easeIn.x controls horizontal offset from kf2, easeIn.y controls vertical
            // Note: easeIn is relative to the end point, so we subtract from x2
            const cp2x = x1 + (easeIn.x * dx)
            const cp2y = y1 + (easeIn.y * dy)

            ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, x2, y2)
            ctx.stroke()

            // Phase 6: Draw tangent handles only for selected keyframes
            const kf1Selected = this.selectedKeyframes.has(kf1)
            const kf2Selected = this.selectedKeyframes.has(kf2)

            if (kf1Selected || kf2Selected) {
              ctx.strokeStyle = curve.displayColor + '80'  // Semi-transparent
              ctx.lineWidth = 1

              // Out tangent handle (from kf1)
              if (kf1Selected) {
                ctx.beginPath()
                ctx.moveTo(x1, y1)
                ctx.lineTo(cp1x, cp1y)
                ctx.stroke()

                // Draw handle point
                ctx.fillStyle = curve.displayColor
                ctx.beginPath()
                ctx.arc(cp1x, cp1y, 4, 0, 2 * Math.PI)
                ctx.fill()
              }

              // In tangent handle (to kf2)
              if (kf2Selected) {
                ctx.beginPath()
                ctx.moveTo(x2, y2)
                ctx.lineTo(cp2x, cp2y)
                ctx.stroke()

                // Draw handle point
                ctx.fillStyle = curve.displayColor
                ctx.beginPath()
                ctx.arc(cp2x, cp2y, 4, 0, 2 * Math.PI)
                ctx.fill()
              }

              // Reset for next curve segment
              ctx.strokeStyle = curve.displayColor
              ctx.lineWidth = 2
            }
            break
        }
      }
    }

    // Draw value labels on the left
    ctx.fillStyle = labelColor
    ctx.font = '10px sans-serif'
    ctx.textAlign = 'right'
    ctx.textBaseline = 'middle'

    for (let i = 0; i <= 4; i++) {
      const value = minValue + (i * (maxValue - minValue) / 4)
      const y = startY + curveHeight - padding - (i * (curveHeight - 2 * padding) / 4)
      ctx.fillText(value.toFixed(2), -5, y)
    }

    // Draw keyframe value tooltip if hovering (check if hover position is in this track's curve area)
    if (this.hoveredKeyframe && this.hoveredKeyframe.trackY === trackY) {
      const hoverX = this.hoveredKeyframe.x
      const hoverY = this.hoveredKeyframe.y
      const hoverValue = this.hoveredKeyframe.keyframe.value

      // Format the value
      const valueText = hoverValue.toFixed(2)

      // Measure text to size the tooltip
      ctx.font = '11px sans-serif'
      const textMetrics = ctx.measureText(valueText)
      const textWidth = textMetrics.width
      const tooltipPadding = 4
      const tooltipWidth = textWidth + tooltipPadding * 2
      const tooltipHeight = 16

      // Position tooltip above and to the right of keyframe
      let tooltipX = hoverX + 8
      let tooltipY = hoverY - tooltipHeight - 8

      // Clamp to stay within bounds
      const maxX = this.width - this.trackHeaderWidth
      if (tooltipX + tooltipWidth > maxX) {
        tooltipX = hoverX - tooltipWidth - 8  // Show on left instead
      }
      if (tooltipY < startY) {
        tooltipY = hoverY + 8  // Show below instead
      }

      // Draw tooltip background
      ctx.fillStyle = backgroundColor
      ctx.fillRect(tooltipX, tooltipY, tooltipWidth, tooltipHeight)

      // Draw tooltip border
      ctx.strokeStyle = foregroundColor
      ctx.lineWidth = 1
      ctx.strokeRect(tooltipX, tooltipY, tooltipWidth, tooltipHeight)

      // Draw value text
      ctx.fillStyle = labelColor
      ctx.textAlign = 'left'
      ctx.textBaseline = 'middle'
      ctx.fillText(valueText, tooltipX + tooltipPadding, tooltipY + tooltipHeight / 2)
    }
  }

  mousedown(x, y) {
    // Check if clicking in ruler area (after track headers)
    if (y <= this.ruler.height && x >= this.trackHeaderWidth) {
      // Adjust x for ruler (remove track header offset)
      const rulerX = x - this.trackHeaderWidth
      const hitPlayhead = this.ruler.mousedown(rulerX, y);
      if (hitPlayhead) {
        // Sync activeObject currentTime with the new playhead position
        if (this.context.activeObject) {
          this.context.activeObject.currentTime = this.timelineState.currentTime
          // Sync DAW backend
          invoke('audio_seek', { seconds: this.timelineState.currentTime });
        }

        // Trigger stage redraw to show animation at new time
        if (this.context.updateUI) {
          this.context.updateUI()
        }

        this.draggingPlayhead = true
        this._globalEvents.add("mousemove")
        this._globalEvents.add("mouseup")
        return true
      }
    }

    // Check if clicking in track header area
    const trackY = y - this.ruler.height
    if (trackY >= 0 && x < this.trackHeaderWidth) {
      // Adjust for vertical scroll offset
      const adjustedY = trackY - this.trackScrollOffset
      const track = this.trackHierarchy.getTrackAtY(adjustedY)
      if (track) {
        const indentSize = 20
        const indent = track.indent * indentSize
        const triangleX = indent + 8

        // Check if clicking on expand/collapse triangle
        if (x >= triangleX - 8 && x <= triangleX + 14) {
          // Toggle collapsed state
          if (track.type === 'layer') {
            track.object.collapsed = !track.object.collapsed
          } else if (track.type === 'object') {
            track.object.trackCollapsed = !track.object.trackCollapsed
          }
          // Rebuild tracks after collapsing/expanding
          this.trackHierarchy.buildTracks(this.context.activeObject)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Check if clicking on toggle buttons (Phase 3)
        if (track.type === 'object' || track.type === 'shape' || track.type === 'audio' || track.type === 'midi') {
          const buttonSize = 14
          const trackIndex = this.trackHierarchy.tracks.indexOf(track)
          const trackY = this.trackHierarchy.getTrackY(trackIndex)
          const buttonY = trackY + (this.trackHierarchy.trackHeight - buttonSize) / 2  // Use base height for button

          // Calculate button positions (same as in draw)
          let buttonX = this.trackHeaderWidth - 10

          // Curves mode button (rightmost)
          const curveButtonX = buttonX - buttonSize
          if (x >= curveButtonX && x <= curveButtonX + buttonSize &&
              adjustedY >= buttonY && adjustedY <= buttonY + buttonSize) {
            // Cycle through curves modes: segment -> keyframe -> curve -> segment
            if (track.object.curvesMode === 'segment') {
              track.object.curvesMode = 'keyframe'
            } else if (track.object.curvesMode === 'keyframe') {
              track.object.curvesMode = 'curve'
            } else {
              track.object.curvesMode = 'segment'
            }

            // Update hover tooltip with new mode name
            if (this.hoveredCurveModeButton) {
              const modeName = track.object.curvesMode === 'curve' ? 'Curve View' :
                              track.object.curvesMode === 'keyframe' ? 'Keyframe View' : 'Segment View'
              this.hoveredCurveModeButton.modeName = modeName
            }

            if (this.requestRedraw) this.requestRedraw()
            return true
          }

          // Segment visibility button
          const segmentButtonX = curveButtonX - (buttonSize + 4)
          if (x >= segmentButtonX && x <= segmentButtonX + buttonSize &&
              adjustedY >= buttonY && adjustedY <= buttonY + buttonSize) {
            // Toggle segment visibility
            track.object.showSegment = !track.object.showSegment
            if (this.requestRedraw) this.requestRedraw()
            return true
          }

          // Check if clicking on legend items (Phase 6)
          if (track.object.curvesMode === 'curve') {
            const trackIndex = this.trackHierarchy.tracks.indexOf(track)
            const trackYPos = this.trackHierarchy.getTrackY(trackIndex)
            const legendPadding = 3
            const legendLineHeight = 12
            const legendY = trackYPos + this.trackHierarchy.trackHeight + 5

            // Get curves for this track
            const curves = []
            const obj = track.object
            let animationData = null

            if (track.type === 'object') {
              for (let layer of this.context.activeObject.allLayers) {
                if (layer.children && layer.children.includes(obj)) {
                  animationData = layer.animationData
                  break
                }
              }
            } else if (track.type === 'shape') {
              for (let layer of this.context.activeObject.allLayers) {
                if (layer.shapes && layer.shapes.some(s => s.shapeId === obj.shapeId)) {
                  animationData = layer.animationData
                  break
                }
              }
            }

            if (animationData) {
              const prefix = track.type === 'object' ? `child.${obj.idx}.` : `shape.${obj.shapeId}.`
              for (let curveName in animationData.curves) {
                if (curveName.startsWith(prefix)) {
                  curves.push(animationData.curves[curveName])
                }
              }
            }

            // Check if clicking on any legend item
            for (let i = 0; i < curves.length; i++) {
              const curve = curves[i]
              const itemY = legendY + legendPadding + i * legendLineHeight

              // Legend items are from x=5 to x=145, height of 12px
              if (x >= 5 && x <= 145 && adjustedY >= itemY && adjustedY <= itemY + legendLineHeight) {
                // Toggle visibility of this curve
                if (this.hiddenCurves.has(curve.parameter)) {
                  this.hiddenCurves.delete(curve.parameter)
                  console.log(`Showing curve: ${curve.parameter}`)
                } else {
                  this.hiddenCurves.add(curve.parameter)
                  console.log(`Hiding curve: ${curve.parameter}`)
                }
                if (this.requestRedraw) this.requestRedraw()
                return true
              }
            }
          }
        }

        // Clicking elsewhere on track header selects it
        this.selectTrack(track)
        if (this.requestRedraw) this.requestRedraw()
        return true
      }
    }

    // Check if clicking in timeline area (segments or curves)
    if (trackY >= 0 && x >= this.trackHeaderWidth) {
      const adjustedY = trackY - this.trackScrollOffset
      const adjustedX = x - this.trackHeaderWidth
      const track = this.trackHierarchy.getTrackAtY(adjustedY)

      if (track) {
        // Phase 6: Check if clicking on tangent handle (highest priority for curves)
        if ((track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'curve') {
          const tangentInfo = this.getTangentHandleAtPoint(track, adjustedX, adjustedY)
          console.log(`Tangent handle check result:`, tangentInfo)
          if (tangentInfo) {
            // Start tangent dragging
            this.draggingTangent = {
              keyframe: tangentInfo.keyframe,
              handle: tangentInfo.handle,
              curve: tangentInfo.curve,
              track: track,
              initialEase: tangentInfo.handle === 'out'
                ? { ...tangentInfo.keyframe.easeOut }
                : { ...tangentInfo.keyframe.easeIn },
              adjacentKeyframe: tangentInfo.handle === 'out'
                ? tangentInfo.nextKeyframe
                : tangentInfo.prevKeyframe
            }

            // Enable global mouse events for dragging
            this._globalEvents.add("mousemove")
            this._globalEvents.add("mouseup")

            console.log('Started dragging', tangentInfo.handle, 'tangent handle')
            if (this.requestRedraw) this.requestRedraw()
            return true
          }
        }

        // Phase 5: Check if clicking on expanded curves
        if ((track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'curve') {
          const curveClickResult = this.handleCurveClick(track, adjustedX, adjustedY)
          if (curveClickResult) {
            return true
          }
        }

        // Phase 6: Check if clicking on segment edge to start edge dragging (priority over segment dragging)
        const edgeInfo = this.getSegmentEdgeAtPoint(track, adjustedX, adjustedY)
        if (edgeInfo && edgeInfo.keyframe) {
          // Select the track
          this.selectTrack(track)

          // Start edge dragging
          this.draggingEdge = {
            track: track,
            edge: edgeInfo.edge,
            keyframe: edgeInfo.keyframe,
            animationData: edgeInfo.animationData,
            curveName: edgeInfo.curveName,
            initialTime: edgeInfo.keyframe.time,
            otherEdgeTime: edgeInfo.edge === 'left' ? edgeInfo.endTime : edgeInfo.startTime
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging', edgeInfo.edge, 'edge at time', edgeInfo.keyframe.time)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Check if clicking on loop corner (top-right) to extend/loop clip
        const loopCornerInfo = this.getAudioClipLoopCornerAtPoint(track, adjustedX, adjustedY)
        if (loopCornerInfo) {
          // Skip if right-clicking (button 2)
          if (this.lastClickEvent?.button === 2) {
            return false
          }

          // Select the track
          this.selectTrack(track)

          // Start loop corner dragging
          this.draggingLoopCorner = {
            track: track,
            clip: loopCornerInfo.clip,
            clipIndex: loopCornerInfo.clipIndex,
            audioTrack: loopCornerInfo.audioTrack,
            isMIDI: loopCornerInfo.isMIDI,
            initialDuration: loopCornerInfo.clip.duration
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging loop corner')
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Check if clicking on audio clip edge to start trimming
        const audioEdgeInfo = this.getAudioClipEdgeAtPoint(track, adjustedX, adjustedY)
        if (audioEdgeInfo) {
          // Skip if right-clicking (button 2)
          if (this.lastClickEvent?.button === 2) {
            return false
          }

          // Select the track
          this.selectTrack(track)

          // Start audio clip edge dragging
          this.draggingAudioClipEdge = {
            track: track,
            edge: audioEdgeInfo.edge,
            clip: audioEdgeInfo.clip,
            clipIndex: audioEdgeInfo.clipIndex,
            audioTrack: audioEdgeInfo.audioTrack,
            initialClipStart: audioEdgeInfo.clip.startTime,
            initialClipDuration: audioEdgeInfo.clip.duration,
            initialClipOffset: audioEdgeInfo.clip.offset,
            initialLinkedVideoOffset: audioEdgeInfo.clip.linkedVideoClip?.offset || 0
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging audio clip', audioEdgeInfo.edge, 'edge')
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Check if clicking on audio clip to start dragging
        const audioClipInfo = this.getAudioClipAtPoint(track, adjustedX, adjustedY)
        if (audioClipInfo) {
          // Skip drag if right-clicking (button 2)
          if (this.lastClickEvent?.button === 2) {
            return false
          }

          // Select the track
          this.selectTrack(track)

          // If this is a MIDI clip, update piano roll selection
          if (audioClipInfo.isMIDI && context.pianoRollEditor) {
            context.pianoRollEditor.selectedClipId = audioClipInfo.clip.clipId
            context.pianoRollEditor.selectedNotes.clear()
            // Trigger piano roll redraw to show the selection change
            if (context.pianoRollRedraw) {
              context.pianoRollRedraw()
            }
          }

          // Start audio clip dragging
          const clickTime = this.timelineState.pixelToTime(adjustedX)
          this.draggingAudioClip = {
            track: track,
            clip: audioClipInfo.clip,
            clipIndex: audioClipInfo.clipIndex,
            audioTrack: audioClipInfo.audioTrack,
            initialMouseTime: clickTime,
            initialClipStartTime: audioClipInfo.clip.startTime
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging audio clip at time', audioClipInfo.clip.startTime)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Check if clicking on video clip edge to start trimming
        const videoEdgeInfo = this.getVideoClipEdgeAtPoint(track, adjustedX, adjustedY)
        if (videoEdgeInfo) {
          // Skip if right-clicking (button 2)
          if (this.lastClickEvent?.button === 2) {
            return false
          }

          // Select the track
          this.selectTrack(track)

          // Start video clip edge dragging
          this.draggingVideoClipEdge = {
            track: track,
            edge: videoEdgeInfo.edge,
            clip: videoEdgeInfo.clip,
            clipIndex: videoEdgeInfo.clipIndex,
            videoLayer: videoEdgeInfo.videoLayer,
            initialClipStart: videoEdgeInfo.clip.startTime,
            initialClipDuration: videoEdgeInfo.clip.duration,
            initialClipOffset: videoEdgeInfo.clip.offset,
            initialLinkedAudioOffset: videoEdgeInfo.clip.linkedAudioClip?.offset || 0
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging video clip', videoEdgeInfo.edge, 'edge')
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Check if clicking on video clip to start dragging
        const videoClipInfo = this.getVideoClipAtPoint(track, adjustedX, adjustedY)
        if (videoClipInfo) {
          // Skip drag if right-clicking (button 2)
          if (this.lastClickEvent?.button === 2) {
            return false
          }

          // Select the track
          this.selectTrack(track)

          // Start video clip dragging
          const clickTime = this.timelineState.pixelToTime(adjustedX)
          this.draggingVideoClip = {
            track: track,
            clip: videoClipInfo.clip,
            clipIndex: videoClipInfo.clipIndex,
            videoLayer: videoClipInfo.videoLayer,
            initialMouseTime: clickTime,
            initialClipStartTime: videoClipInfo.clip.startTime
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging video clip at time', videoClipInfo.clip.startTime)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Phase 6: Check if clicking on segment to start dragging
        const segmentInfo = this.getSegmentAtPoint(track, adjustedX, adjustedY)
        if (segmentInfo) {
          // Select the track
          this.selectTrack(track)

          // Start segment dragging
          const clickTime = this.timelineState.pixelToTime(adjustedX)
          this.draggingSegment = {
            track: track,
            initialMouseTime: clickTime,
            segmentStartTime: segmentInfo.startTime,
            segmentEndTime: segmentInfo.endTime,
            animationData: segmentInfo.animationData,
            objectIdx: track.object.idx
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging segment at time', segmentInfo.startTime)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }

        // Fallback: clicking anywhere on track in timeline area selects it
        // This is especially important for audio tracks that may not have clips yet
        this.selectTrack(track)
        if (this.requestRedraw) this.requestRedraw()
        return true
      }
    }

    return false
  }

  /**
   * Handle click on curve area in expanded mode (Phase 5)
   * Returns true if click was handled
   */
  handleCurveClick(track, x, y) {
    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    const trackY = this.trackHierarchy.getTrackY(trackIndex)

    const curveHeight = 80
    const startY = trackY + 10  // Start below segment area
    const padding = 5

    // Check if y is within curve area
    if (y < startY || y > startY + curveHeight) {
      return false
    }

    // Get AnimationData and curves for this track
    const obj = track.object
    let animationData = null

    if (track.type === 'object') {
      for (let layer of this.context.activeObject.allLayers) {
        if (layer.children && layer.children.includes(obj)) {
          animationData = layer.animationData
          break
        }
      }
    } else if (track.type === 'shape') {
      const findShapeLayer = (searchObj) => {
        for (let layer of searchObj.children) {
          if (layer.shapes && layer.shapes.includes(obj)) {
            animationData = layer.animationData
            return true
          }
          if (layer.children) {
            for (let child of layer.children) {
              if (findShapeLayer(child)) return true
            }
          }
        }
        return false
      }
      findShapeLayer(this.context.activeObject)
    }

    if (!animationData) return false

    // Get all curves for this object/shape
    const curves = []
    for (let curveName in animationData.curves) {
      const curve = animationData.curves[curveName]
      if (track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
        curves.push(curve)
      } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.shapeId}.`)) {
        curves.push(curve)
      }
    }

    if (curves.length === 0) return false

    // Calculate value range for scaling
    let minValue = Infinity
    let maxValue = -Infinity
    for (let curve of curves) {
      for (let keyframe of curve.keyframes) {
        minValue = Math.min(minValue, keyframe.value)
        maxValue = Math.max(maxValue, keyframe.value)
      }
    }
    const valueRange = maxValue - minValue
    const rangePadding = valueRange * 0.1 || 1
    minValue -= rangePadding
    maxValue += rangePadding

    // Helper to convert Y position to value
    const yToValue = (yPos) => {
      const normalizedY = (startY + curveHeight - padding - yPos) / (curveHeight - 2 * padding)
      return minValue + (normalizedY * (maxValue - minValue))
    }

    // Convert click position to time and value
    let clickTime = this.timelineState.pixelToTime(x)
    const clickValue = yToValue(y)

    // Apply snapping to click time
    clickTime = this.timelineState.snapTime(clickTime)

    // Check if clicking close to an existing keyframe on ANY curve (within 8px)
    // First pass: check all curves for keyframe hits
    for (let curve of curves) {
      // Skip hidden curves
      if (this.hiddenCurves.has(curve.parameter)) continue

      for (let keyframe of curve.keyframes) {
        const kfX = this.timelineState.timeToPixel(keyframe.time)
        const kfY = startY + curveHeight - padding - ((keyframe.value - minValue) / (maxValue - minValue) * (curveHeight - 2 * padding))
        const distance = Math.sqrt((x - kfX) ** 2 + (y - kfY) ** 2)

        if (distance < 8) {
          // Check for multi-select modifier keys from click event
          const shiftKey = this.lastClickEvent?.shiftKey || false
          const ctrlKey = this.lastClickEvent?.ctrlKey || this.lastClickEvent?.metaKey || false

          if (shiftKey) {
            // Shift: Add to selection
            this.selectedKeyframes.add(keyframe)
            console.log(`Added keyframe to selection, now have ${this.selectedKeyframes.size} selected`)
          } else if (ctrlKey) {
            // Ctrl/Cmd: Toggle selection
            if (this.selectedKeyframes.has(keyframe)) {
              this.selectedKeyframes.delete(keyframe)
              console.log(`Removed keyframe from selection, now have ${this.selectedKeyframes.size} selected`)
            } else {
              this.selectedKeyframes.add(keyframe)
              console.log(`Added keyframe to selection, now have ${this.selectedKeyframes.size} selected`)
            }
          } else {
            // No modifier: Select only this keyframe
            this.selectedKeyframes.clear()
            this.selectedKeyframes.add(keyframe)
            console.log(`Selected single keyframe`)
          }

          // Don't start dragging if this was a right-click
          if (this.lastClickEvent?.button === 2) {
            console.log(`Skipping drag - right-click detected (button=${this.lastClickEvent.button})`)
            return true
          }

          // Start dragging this keyframe (and all selected keyframes)
          this.draggingKeyframe = {
            curve: curve,  // Use the actual curve we clicked on
            keyframe: keyframe,
            track: track,
            initialTime: keyframe.time,
            initialValue: keyframe.value,
            minValue: minValue,
            maxValue: maxValue,
            curveHeight: curveHeight,
            startY: startY,
            padding: padding,
            yToValue: yToValue  // Store the conversion function
          }

          // Enable global mouse events for dragging
          this._globalEvents.add("mousemove")
          this._globalEvents.add("mouseup")

          console.log('Started dragging keyframe at time', keyframe.time, 'on curve', curve.parameter)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }
      }
    }

    // No keyframe was clicked, so add a new one
    // Find the closest curve to the click position (only visible curves)
    let targetCurve = null
    let minDistance = Infinity

    for (let curve of curves) {
      // Skip hidden curves
      if (this.hiddenCurves.has(curve.parameter)) continue

      // For each curve, find the value at this time
      const curveValue = curve.interpolate(clickTime)
      if (curveValue !== null) {
        const curveY = startY + curveHeight - padding - ((curveValue - minValue) / (maxValue - minValue) * (curveHeight - 2 * padding))
        const distance = Math.abs(y - curveY)

        if (distance < minDistance) {
          minDistance = distance
          targetCurve = curve
        }
      }
    }

    // If all curves are hidden, don't add a keyframe
    if (!targetCurve) return false

    console.log('Adding keyframe at time', clickTime, 'with value', clickValue, 'to curve', targetCurve.parameter)

    // Create keyframe directly
    const newKeyframe = {
      time: clickTime,
      value: clickValue,
      interpolation: 'linear',
      easeIn: { x: 0.42, y: 0 },
      easeOut: { x: 0.58, y: 1 },
      idx: this.generateUUID()
    }

    targetCurve.addKeyframe(newKeyframe)

    if (this.requestRedraw) this.requestRedraw()
    return true
  }

  /**
   * Generate UUID (Phase 5)
   */
  generateUUID() {
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
      const r = Math.random() * 16 | 0
      const v = c === 'x' ? r : (r & 0x3 | 0x8)
      return v.toString(16)
    })
  }

  /**
   * Check if a point (in timeline area coordinates) is inside a segment for the given track
   */
  isPointInSegment(track, x, y) {
    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    if (trackIndex === -1) return false

    const trackY = this.trackHierarchy.getTrackY(trackIndex)
    const trackHeight = this.trackHierarchy.trackHeight  // Use base height for segment bounds
    const segmentTop = trackY + 5
    const segmentBottom = trackY + trackHeight - 5

    // Check if y is within segment bounds
    if (y < segmentTop || y > segmentBottom) return false

    const clickTime = this.timelineState.pixelToTime(x)
    const frameDuration = 1 / this.timelineState.framerate
    const minSegmentDuration = frameDuration

    if (track.type === 'object') {
      // Check frameNumber curve for objects
      const obj = track.object
      let parentLayer = null
      for (let layer of this.context.activeObject.allLayers) {
        if (layer.children && layer.children.includes(obj)) {
          parentLayer = layer
          break
        }
      }

      if (!parentLayer || !parentLayer.animationData) return false

      const frameNumberKey = `child.${obj.idx}.frameNumber`
      const frameNumberCurve = parentLayer.animationData.curves[frameNumberKey]

      if (!frameNumberCurve || !frameNumberCurve.keyframes) return false

      // Check if clickTime is within any segment
      let segmentStart = null
      for (let j = 0; j < frameNumberCurve.keyframes.length; j++) {
        const keyframe = frameNumberCurve.keyframes[j]

        if (keyframe.value > 0) {
          if (segmentStart === null) {
            segmentStart = keyframe.time
          }

          const isLast = (j === frameNumberCurve.keyframes.length - 1)
          const nextEndsSegment = !isLast && frameNumberCurve.keyframes[j + 1].value === 0

          if (isLast || nextEndsSegment) {
            const segmentEnd = nextEndsSegment ? frameNumberCurve.keyframes[j + 1].time : keyframe.time + minSegmentDuration

            if (clickTime >= segmentStart && clickTime <= segmentEnd) {
              return true
            }
            segmentStart = null
          }
        }
      }
    } else if (track.type === 'shape') {
      // Check exists curve for shapes
      const shape = track.object
      let shapeLayer = null
      const findShapeLayer = (obj) => {
        for (let layer of obj.children) {
          if (layer.shapes && layer.shapes.includes(shape)) {
            shapeLayer = layer
            return true
          }
          if (layer.children) {
            for (let child of layer.children) {
              if (findShapeLayer(child)) return true
            }
          }
        }
        return false
      }
      findShapeLayer(this.context.activeObject)

      if (!shapeLayer || !shapeLayer.animationData) return false

      const existsCurveKey = `shape.${shape.shapeId}.exists`
      const existsCurve = shapeLayer.animationData.curves[existsCurveKey]

      if (!existsCurve || !existsCurve.keyframes) return false

      // Check if clickTime is within any segment
      let segmentStart = null
      for (let j = 0; j < existsCurve.keyframes.length; j++) {
        const keyframe = existsCurve.keyframes[j]

        if (keyframe.value > 0) {
          if (segmentStart === null) {
            segmentStart = keyframe.time
          }

          const isLast = (j === existsCurve.keyframes.length - 1)
          const nextEndsSegment = !isLast && existsCurve.keyframes[j + 1].value === 0

          if (isLast || nextEndsSegment) {
            const segmentEnd = nextEndsSegment ? existsCurve.keyframes[j + 1].time : keyframe.time + minSegmentDuration

            if (clickTime >= segmentStart && clickTime <= segmentEnd) {
              return true
            }
            segmentStart = null
          }
        }
      }
    }

    return false
  }

  /**
   * Get segment information at a point (Phase 6)
   * Returns {startTime, endTime, animationData} if point is in a segment, null otherwise
   */
  getSegmentAtPoint(track, x, y) {
    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    if (trackIndex === -1) return null

    const trackY = this.trackHierarchy.getTrackY(trackIndex)
    const trackHeight = this.trackHierarchy.trackHeight
    const segmentTop = trackY + 5
    const segmentBottom = trackY + trackHeight - 5

    // Check if y is within segment bounds
    if (y < segmentTop || y > segmentBottom) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const frameDuration = 1 / this.timelineState.framerate
    const minSegmentDuration = frameDuration

    if (track.type === 'object') {
      // Check frameNumber curve for objects
      const obj = track.object
      let parentLayer = null
      for (let layer of this.context.activeObject.allLayers) {
        if (layer.children && layer.children.includes(obj)) {
          parentLayer = layer
          break
        }
      }

      if (!parentLayer || !parentLayer.animationData) return null

      const frameNumberKey = `child.${obj.idx}.frameNumber`
      const frameNumberCurve = parentLayer.animationData.curves[frameNumberKey]

      if (!frameNumberCurve || !frameNumberCurve.keyframes) return null

      // Check if clickTime is within any segment
      let segmentStart = null
      for (let j = 0; j < frameNumberCurve.keyframes.length; j++) {
        const keyframe = frameNumberCurve.keyframes[j]

        if (keyframe.value > 0) {
          if (segmentStart === null) {
            segmentStart = keyframe.time
          }

          const isLast = (j === frameNumberCurve.keyframes.length - 1)
          const nextEndsSegment = !isLast && frameNumberCurve.keyframes[j + 1].value === 0

          if (isLast || nextEndsSegment) {
            const segmentEnd = nextEndsSegment ? frameNumberCurve.keyframes[j + 1].time : keyframe.time + minSegmentDuration

            if (clickTime >= segmentStart && clickTime <= segmentEnd) {
              return {
                startTime: segmentStart,
                endTime: segmentEnd,
                animationData: parentLayer.animationData
              }
            }
            segmentStart = null
          }
        }
      }
    } else if (track.type === 'shape') {
      // Check exists curve for shapes
      const shape = track.object
      let shapeLayer = null
      const findShapeLayer = (obj) => {
        for (let layer of obj.children) {
          if (layer.shapes && layer.shapes.includes(shape)) {
            shapeLayer = layer
            return true
          }
          if (layer.children) {
            for (let child of layer.children) {
              if (findShapeLayer(child)) return true
            }
          }
        }
        return false
      }
      findShapeLayer(this.context.activeObject)

      if (!shapeLayer || !shapeLayer.animationData) return null

      const existsCurveKey = `shape.${shape.shapeId}.exists`
      const existsCurve = shapeLayer.animationData.curves[existsCurveKey]

      if (!existsCurve || !existsCurve.keyframes) return null

      // Check if clickTime is within any segment
      let segmentStart = null
      for (let j = 0; j < existsCurve.keyframes.length; j++) {
        const keyframe = existsCurve.keyframes[j]

        if (keyframe.value > 0) {
          if (segmentStart === null) {
            segmentStart = keyframe.time
          }

          const isLast = (j === existsCurve.keyframes.length - 1)
          const nextEndsSegment = !isLast && existsCurve.keyframes[j + 1].value === 0

          if (isLast || nextEndsSegment) {
            const segmentEnd = nextEndsSegment ? existsCurve.keyframes[j + 1].time : keyframe.time + minSegmentDuration

            if (clickTime >= segmentStart && clickTime <= segmentEnd) {
              return {
                startTime: segmentStart,
                endTime: segmentEnd,
                animationData: shapeLayer.animationData
              }
            }
            segmentStart = null
          }
        }
      }
    }

    return null
  }

  /**
   * Get audio clip at a point
   * Returns {clip, clipIndex, audioTrack} if clicking on an audio clip
   */
  getAudioClipAtPoint(track, x, y) {
    if (track.type !== 'audio') return null

    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    if (trackIndex === -1) return null

    const trackY = this.trackHierarchy.getTrackY(trackIndex)
    const trackHeight = this.trackHierarchy.trackHeight
    const clipTop = trackY + 5
    const clipBottom = trackY + trackHeight - 5

    // Check if y is within clip bounds
    if (y < clipTop || y > clipBottom) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const audioTrack = track.object

    // Check each clip
    for (let i = 0; i < audioTrack.clips.length; i++) {
      const clip = audioTrack.clips[i]
      const clipStart = clip.startTime
      const clipEnd = clip.startTime + clip.duration

      if (clickTime >= clipStart && clickTime <= clipEnd) {
        return {
          clip: clip,
          clipIndex: i,
          audioTrack: audioTrack,
          isMIDI: audioTrack.type === 'midi'
        }
      }
    }

    return null
  }

  getAudioClipEdgeAtPoint(track, x, y) {
    const clipInfo = this.getAudioClipAtPoint(track, x, y)
    if (!clipInfo) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const edgeThreshold = 8 / this.timelineState.pixelsPerSecond  // 8 pixels in time units

    const clipStart = clipInfo.clip.startTime
    const clipEnd = clipInfo.clip.startTime + clipInfo.clip.duration

    // Check if near left edge
    if (Math.abs(clickTime - clipStart) <= edgeThreshold) {
      return {
        edge: 'left',
        clip: clipInfo.clip,
        clipIndex: clipInfo.clipIndex,
        audioTrack: clipInfo.audioTrack,
        clipStart: clipStart,
        clipEnd: clipEnd
      }
    }

    // Check if near right edge
    if (Math.abs(clickTime - clipEnd) <= edgeThreshold) {
      return {
        edge: 'right',
        clip: clipInfo.clip,
        clipIndex: clipInfo.clipIndex,
        audioTrack: clipInfo.audioTrack,
        clipStart: clipStart,
        clipEnd: clipEnd
      }
    }

    return null
  }

  /**
   * Check if hovering over the loop corner (top-right) of an audio/MIDI clip
   * Returns clip info if in the loop corner zone
   */
  getAudioClipLoopCornerAtPoint(track, x, y) {
    if (track.type !== 'audio') return null

    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    if (trackIndex === -1) return null

    const trackY = this.trackHierarchy.getTrackY(trackIndex)
    const trackHeight = this.trackHierarchy.trackHeight
    const clipTop = trackY + 5
    const cornerSize = 12  // Size of the corner hot zone in pixels

    // Check if y is in the top portion of the clip
    if (y < clipTop || y > clipTop + cornerSize) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const audioTrack = track.object

    // Check each clip
    for (let i = 0; i < audioTrack.clips.length; i++) {
      const clip = audioTrack.clips[i]
      const clipEnd = clip.startTime + clip.duration
      const clipEndX = this.timelineState.timeToPixel(clipEnd)

      // Check if x is near the right edge (within corner zone)
      if (x >= clipEndX - cornerSize && x <= clipEndX) {
        return {
          clip: clip,
          clipIndex: i,
          audioTrack: audioTrack,
          isMIDI: audioTrack.type === 'midi'
        }
      }
    }

    return null
  }

  getVideoClipAtPoint(track, x, y) {
    if (track.type !== 'video') return null

    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    if (trackIndex === -1) return null

    const trackY = this.trackHierarchy.getTrackY(trackIndex)
    const trackHeight = this.trackHierarchy.trackHeight
    const clipTop = trackY + 5
    const clipBottom = trackY + trackHeight - 5

    // Check if y is within clip bounds
    if (y < clipTop || y > clipBottom) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const videoLayer = track.object

    // Check each clip
    for (let i = 0; i < videoLayer.clips.length; i++) {
      const clip = videoLayer.clips[i]
      const clipStart = clip.startTime
      const clipEnd = clip.startTime + clip.duration

      if (clickTime >= clipStart && clickTime <= clipEnd) {
        return {
          clip: clip,
          clipIndex: i,
          videoLayer: videoLayer
        }
      }
    }

    return null
  }

  getVideoClipEdgeAtPoint(track, x, y) {
    const clipInfo = this.getVideoClipAtPoint(track, x, y)
    if (!clipInfo) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const edgeThreshold = 8 / this.timelineState.pixelsPerSecond  // 8 pixels in time units

    const clipStart = clipInfo.clip.startTime
    const clipEnd = clipInfo.clip.startTime + clipInfo.clip.duration

    // Check if near left edge
    if (Math.abs(clickTime - clipStart) <= edgeThreshold) {
      return {
        edge: 'left',
        clip: clipInfo.clip,
        clipIndex: clipInfo.clipIndex,
        videoLayer: clipInfo.videoLayer,
        clipStart: clipStart,
        clipEnd: clipEnd
      }
    }

    // Check if near right edge
    if (Math.abs(clickTime - clipEnd) <= edgeThreshold) {
      return {
        edge: 'right',
        clip: clipInfo.clip,
        clipIndex: clipInfo.clipIndex,
        videoLayer: clipInfo.videoLayer,
        clipStart: clipStart,
        clipEnd: clipEnd
      }
    }

    return null
  }

  /**
   * Get segment edge at a point (Phase 6)
   * Returns {edge: 'left'|'right', startTime, endTime, keyframe, animationData, curveName} if near an edge
   */
  getSegmentEdgeAtPoint(track, x, y) {
    const segmentInfo = this.getSegmentAtPoint(track, x, y)
    if (!segmentInfo) return null

    const clickTime = this.timelineState.pixelToTime(x)
    const edgeThreshold = 8 / this.timelineState.pixelsPerSecond  // 8 pixels in time units

    // Determine which curve to look at
    let curveName
    if (track.type === 'object') {
      curveName = `child.${track.object.idx}.frameNumber`
    } else if (track.type === 'shape') {
      curveName = `shape.${track.object.shapeId}.exists`
    } else {
      return null
    }

    const curve = segmentInfo.animationData.curves[curveName]
    if (!curve || !curve.keyframes) return null

    // Find the keyframes that define this segment's edges
    let startKeyframe = null
    let endKeyframe = null

    for (let keyframe of curve.keyframes) {
      if (Math.abs(keyframe.time - segmentInfo.startTime) < 0.0001 && keyframe.value > 0) {
        startKeyframe = keyframe
      }
      // For end keyframe, check both at exact endTime AND just before it (for natural segment ends)
      if (Math.abs(keyframe.time - segmentInfo.endTime) < 0.0001) {
        endKeyframe = keyframe
      } else if (keyframe.value > 0 && keyframe.time < segmentInfo.endTime && keyframe.time >= segmentInfo.startTime) {
        // Track the last positive keyframe in case segment ends naturally
        if (!endKeyframe || keyframe.time > endKeyframe.time) {
          endKeyframe = keyframe
        }
      }
    }

    // Check if click is near left edge
    if (Math.abs(clickTime - segmentInfo.startTime) <= edgeThreshold) {
      return {
        edge: 'left',
        startTime: segmentInfo.startTime,
        endTime: segmentInfo.endTime,
        keyframe: startKeyframe,
        animationData: segmentInfo.animationData,
        curveName: curveName
      }
    }

    // Check if click is near right edge
    // For natural segment ends, the endKeyframe is at an earlier time than segmentInfo.endTime
    const rightEdgeTime = endKeyframe ? endKeyframe.time : segmentInfo.endTime
    if (Math.abs(clickTime - rightEdgeTime) <= edgeThreshold) {
      return {
        edge: 'right',
        startTime: segmentInfo.startTime,
        endTime: segmentInfo.endTime,
        keyframe: endKeyframe,
        animationData: segmentInfo.animationData,
        curveName: curveName
      }
    }

    return null
  }

  /**
   * Check if clicking on a tangent handle (Phase 6)
   * Returns {keyframe, handle: 'in'|'out', curve, nextKeyframe|prevKeyframe} if hitting a handle
   */
  getTangentHandleAtPoint(track, x, y) {
    if (track.type !== 'object' && track.type !== 'shape') return null
    if (track.object.curvesMode !== 'curve') return null

    const trackIndex = this.trackHierarchy.tracks.indexOf(track)
    const trackY = this.trackHierarchy.getTrackY(trackIndex)

    const curveHeight = 80
    const startY = trackY + 10
    const padding = 5

    // Check if y is within curve area
    if (y < startY || y > startY + curveHeight) return null

    // Get all curves for this track
    const obj = track.object
    let animationData = null

    if (track.type === 'object') {
      for (let layer of this.context.activeObject.allLayers) {
        if (layer.children && layer.children.includes(obj)) {
          animationData = layer.animationData
          break
        }
      }
    } else if (track.type === 'shape') {
      const findShapeLayer = (searchObj) => {
        for (let layer of searchObj.children) {
          if (layer.shapes && layer.shapes.includes(obj)) {
            animationData = layer.animationData
            return true
          }
          if (layer.children) {
            for (let child of layer.children) {
              if (findShapeLayer(child)) return true
            }
          }
        }
        return false
      }
      findShapeLayer(this.context.activeObject)
    }

    if (!animationData) return null

    // Get all curves
    const curves = []
    for (let curveName in animationData.curves) {
      const curve = animationData.curves[curveName]
      if (track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
        curves.push(curve)
      } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.shapeId}.`)) {
        curves.push(curve)
      }
    }

    // Calculate value range
    let minValue = Infinity
    let maxValue = -Infinity
    for (let curve of curves) {
      for (let keyframe of curve.keyframes) {
        minValue = Math.min(minValue, keyframe.value)
        maxValue = Math.max(maxValue, keyframe.value)
      }
    }
    const valueRange = maxValue - minValue
    const rangePadding = valueRange * 0.1 || 1
    minValue -= rangePadding
    maxValue += rangePadding

    const valueToY = (value) => {
      const normalizedValue = (value - minValue) / (maxValue - minValue)
      return startY + curveHeight - padding - (normalizedValue * (curveHeight - 2 * padding))
    }

    // Check each curve for tangent handles
    for (let curve of curves) {
      // Skip hidden curves
      if (this.hiddenCurves.has(curve.parameter)) continue

      // Only check bezier keyframes that are selected
      for (let i = 0; i < curve.keyframes.length; i++) {
        const kf = curve.keyframes[i]

        // Only show handles for selected keyframes with bezier interpolation
        if (!this.selectedKeyframes.has(kf) || kf.interpolation !== 'bezier') continue

        const kfX = this.timelineState.timeToPixel(kf.time)
        const kfY = valueToY(kf.value)

        // Check out handle (if there's a next keyframe)
        if (i < curve.keyframes.length - 1) {
          const nextKf = curve.keyframes[i + 1]
          const nextX = this.timelineState.timeToPixel(nextKf.time)
          const nextY = valueToY(nextKf.value)

          const dx = nextX - kfX
          const dy = nextY - kfY

          const easeOut = kf.easeOut || { x: 0.42, y: 0 }
          const handleX = kfX + (easeOut.x * dx)
          const handleY = kfY + (easeOut.y * dy)

          const distance = Math.sqrt((x - handleX) ** 2 + (y - handleY) ** 2)
          if (distance < 8) {  // 8px hit radius
            return {
              keyframe: kf,
              handle: 'out',
              curve: curve,
              nextKeyframe: nextKf
            }
          }
        }

        // Check in handle (if there's a previous keyframe)
        if (i > 0) {
          const prevKf = curve.keyframes[i - 1]
          const prevX = this.timelineState.timeToPixel(prevKf.time)
          const prevY = valueToY(prevKf.value)

          const dx = kfX - prevX
          const dy = kfY - prevY

          const easeIn = kf.easeIn || { x: 0.58, y: 1 }
          const handleX = prevX + (easeIn.x * dx)
          const handleY = prevY + (easeIn.y * dy)

          const distance = Math.sqrt((x - handleX) ** 2 + (y - handleY) ** 2)
          if (distance < 8) {  // 8px hit radius
            return {
              keyframe: kf,
              handle: 'in',
              curve: curve,
              prevKeyframe: prevKf
            }
          }
        }
      }
    }

    return null
  }

  /**
   * Check if a track is currently selected
   */
  isTrackSelected(track) {
    if (track.type === 'layer') {
      return this.context.activeObject.activeLayer === track.object
    } else if (track.type === 'shape') {
      return this.context.shapeselection?.includes(track.object)
    } else if (track.type === 'object') {
      return this.context.selection?.includes(track.object)
    } else if (track.type === 'audio') {
      // Audio tracks use activeLayer like regular layers
      return this.context.activeObject.activeLayer === track.object
    }
    return false
  }

  /**
   * Select a track and update the stage selection
   */
  selectTrack(track) {
    // Store old selection before changing
    this.context.oldselection = this.context.selection
    this.context.oldshapeselection = this.context.shapeselection

    if (track.type === 'layer') {
      // Set the layer as active (this will clear _activeAudioTrack)
      this.context.activeObject.activeLayer = track.object
      // Clear selections when selecting layer
      this.context.selection = []
      this.context.shapeselection = []

      // Clear node editor when selecting a non-audio layer
      setTimeout(() => this.context.reloadNodeEditor?.(), 50);
    } else if (track.type === 'shape') {
      // Find the layer this shape belongs to and select it
      for (let i = 0; i < this.context.activeObject.allLayers.length; i++) {
        const layer = this.context.activeObject.allLayers[i]
        if (layer.shapes && layer.shapes.includes(track.object)) {
          // Set the layer as active (this will clear _activeAudioTrack)
          this.context.activeObject.activeLayer = layer
          // Set shape selection
          this.context.shapeselection = [track.object]
          this.context.selection = []
          break
        }
      }
    } else if (track.type === 'object') {
      // Select the GraphicsObject
      this.context.selection = [track.object]
      this.context.shapeselection = []
    } else if (track.type === 'audio') {
      // Audio track selected - set as active layer and clear other selections
      // Audio tracks can act as layers (they have animationData, shapes=[], children=[])
      this.context.activeObject.activeLayer = track.object
      this.context.selection = []
      this.context.shapeselection = []

      // Reload the node editor for both MIDI and audio tracks
      if (track.object.type === 'midi' || track.object.type === 'audio') {
        setTimeout(() => this.context.reloadNodeEditor?.(), 50);
      }

      // Set active MIDI track for external MIDI input routing
      if (track.object.type === 'midi') {
        invoke('audio_set_active_midi_track', { trackId: track.object.audioTrackId }).catch(err => {
          console.error('Failed to set active MIDI track:', err);
        });
      }
    } else {
      // Non-audio track selected, clear active MIDI track
      invoke('audio_set_active_midi_track', { trackId: null }).catch(err => {
        console.error('Failed to clear active MIDI track:', err);
      });
    }

    // Update the stage UI to reflect selection changes
    if (this.context.updateUI) {
      this.context.updateUI()
    }

    // Update menu to enable/disable menu items based on selection
    if (this.context.updateMenu) {
      this.context.updateMenu()
    }
  }

  mousemove(x, y) {
    // Check for curve mode button hover (in track header area)
    const trackY = y - this.ruler.height
    if (trackY >= 0 && x < this.trackHeaderWidth) {
      const adjustedY = trackY - this.trackScrollOffset
      const track = this.trackHierarchy.getTrackAtY(adjustedY)

      if (track && (track.type === 'object' || track.type === 'shape' || track.type === 'audio')) {
        const trackIndex = this.trackHierarchy.tracks.indexOf(track)
        const trackYPos = this.trackHierarchy.getTrackY(trackIndex)
        const buttonSize = 16
        const buttonY = trackYPos + (this.trackHierarchy.trackHeight - buttonSize) / 2
        let buttonX = this.trackHeaderWidth - 10 - buttonSize // Rightmost button

        // Check if hovering over curve mode button
        if (x >= buttonX && x <= buttonX + buttonSize &&
            adjustedY >= buttonY && adjustedY <= buttonY + buttonSize) {
          // Get the mode name for tooltip
          const modeName = track.object.curvesMode === 'curve' ? 'Curve View' :
                          track.object.curvesMode === 'keyframe' ? 'Keyframe View' : 'Segment View'
          this.hoveredCurveModeButton = {
            x: x,
            y: y,
            modeName: modeName
          }
          if (this.requestRedraw) this.requestRedraw()
        } else if (this.hoveredCurveModeButton) {
          this.hoveredCurveModeButton = null
          if (this.requestRedraw) this.requestRedraw()
        }
      } else if (this.hoveredCurveModeButton) {
        this.hoveredCurveModeButton = null
        if (this.requestRedraw) this.requestRedraw()
      }
    } else if (this.hoveredCurveModeButton) {
      this.hoveredCurveModeButton = null
      if (this.requestRedraw) this.requestRedraw()
    }

    // Update hover state for keyframe tooltips (even when not dragging)
    // Clear hover if mouse is outside timeline curve areas
    let foundHover = false

    if (!this.draggingKeyframe && !this.draggingPlayhead) {
      const trackY = y - this.ruler.height
      if (trackY >= 0 && x >= this.trackHeaderWidth) {
        const adjustedY = trackY - this.trackScrollOffset
        const adjustedX = x - this.trackHeaderWidth
        const track = this.trackHierarchy.getTrackAtY(adjustedY)

        if (track && (track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'curve') {
          const trackIndex = this.trackHierarchy.tracks.indexOf(track)
          const trackYPos = this.trackHierarchy.getTrackY(trackIndex)

          const curveHeight = 80
          const startY = trackYPos + 10
          const padding = 5

          // Check if within curve area
          if (adjustedY >= startY && adjustedY <= startY + curveHeight) {
            // Get AnimationData and curves for this track
            const obj = track.object
            let animationData = null

            if (track.type === 'object') {
              for (let layer of this.context.activeObject.allLayers) {
                if (layer.children && layer.children.includes(obj)) {
                  animationData = layer.animationData
                  break
                }
              }
            } else if (track.type === 'shape') {
              const findShapeLayer = (searchObj) => {
                for (let layer of searchObj.children) {
                  if (layer.shapes && layer.shapes.includes(obj)) {
                    animationData = layer.animationData
                    return true
                  }
                  if (layer.children) {
                    for (let child of layer.children) {
                      if (findShapeLayer(child)) return true
                    }
                  }
                }
                return false
              }
              findShapeLayer(this.context.activeObject)
            }

            if (animationData) {
              // Get all curves for this object/shape
              const curves = []
              for (let curveName in animationData.curves) {
                const curve = animationData.curves[curveName]
                if (track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
                  curves.push(curve)
                } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.shapeId}.`)) {
                  curves.push(curve)
                }
              }

              if (curves.length > 0) {
                // Calculate value range for scaling
                let minValue = Infinity
                let maxValue = -Infinity
                for (let curve of curves) {
                  for (let keyframe of curve.keyframes) {
                    minValue = Math.min(minValue, keyframe.value)
                    maxValue = Math.max(maxValue, keyframe.value)
                  }
                }
                const valueRange = maxValue - minValue
                const rangePadding = valueRange * 0.1 || 1
                minValue -= rangePadding
                maxValue += rangePadding

                // Check if hovering over any keyframe
                for (let curve of curves) {
                  for (let keyframe of curve.keyframes) {
                    const kfX = this.timelineState.timeToPixel(keyframe.time)
                    const kfY = startY + curveHeight - padding - ((keyframe.value - minValue) / (maxValue - minValue) * (curveHeight - 2 * padding))
                    const distance = Math.sqrt((adjustedX - kfX) ** 2 + (adjustedY - kfY) ** 2)

                    if (distance < 8) {
                      // Found a hover!
                      this.hoveredKeyframe = {
                        keyframe: keyframe,
                        x: kfX,
                        y: kfY,
                        trackY: trackYPos  // Store track Y for comparison in draw
                      }
                      foundHover = true
                      if (this.requestRedraw) this.requestRedraw()
                      break
                    }
                  }
                  if (foundHover) break
                }
              }
            }
          }
        }
      }
    }

    // Clear hover if not found
    if (!foundHover && this.hoveredKeyframe) {
      this.hoveredKeyframe = null
      if (this.requestRedraw) this.requestRedraw()
    }

    if (this.draggingPlayhead) {
      // Adjust x for ruler (remove track header offset)
      const rulerX = x - this.trackHeaderWidth
      this.ruler.mousemove(rulerX, y)

      // Sync GraphicsObject currentTime with timeline playhead
      if (this.context.activeObject) {
        this.context.activeObject.currentTime = this.timelineState.currentTime
        // Sync DAW backend
        invoke('audio_seek', { seconds: this.timelineState.currentTime });
      }

      // Trigger stage redraw to update object positions based on new time
      if (this.context.updateUI) {
        this.context.updateUI()
      }

      return true
    }

    // Phase 5: Handle keyframe dragging
    if (this.draggingKeyframe) {
      // Adjust coordinates to timeline area
      const trackY = y - this.ruler.height
      const adjustedX = x - this.trackHeaderWidth
      const adjustedY = trackY - this.trackScrollOffset

      // Convert mouse position to time and value
      const newTime = this.timelineState.pixelToTime(adjustedX)
      const newValue = this.draggingKeyframe.yToValue(adjustedY)

      // Clamp time to not go negative, then apply snapping
      let clampedTime = Math.max(0, newTime)
      clampedTime = this.timelineState.snapTime(clampedTime)

      // Check for constrained dragging modifiers from drag event
      const shiftKey = this.lastDragEvent?.shiftKey || false
      const ctrlKey = this.lastDragEvent?.ctrlKey || this.lastDragEvent?.metaKey || false

      // Calculate deltas from the initial position
      let timeDelta = clampedTime - this.draggingKeyframe.initialTime
      let valueDelta = newValue - this.draggingKeyframe.initialValue

      // Apply constraints based on modifier keys
      if (shiftKey && !ctrlKey) {
        // Shift: vertical only (constrain time)
        timeDelta = 0
      } else if (ctrlKey && !shiftKey) {
        // Ctrl/Cmd: horizontal only (constrain value)
        valueDelta = 0
      }

      // Update all selected keyframes
      for (let selectedKeyframe of this.selectedKeyframes) {
        // Get the initial position of this keyframe (stored when dragging started)
        if (!selectedKeyframe.initialDragTime) {
          selectedKeyframe.initialDragTime = selectedKeyframe.time
          selectedKeyframe.initialDragValue = selectedKeyframe.value
        }

        // Apply the delta
        selectedKeyframe.time = Math.max(0, selectedKeyframe.initialDragTime + timeDelta)
        let newValue = selectedKeyframe.initialDragValue + valueDelta

        // Special validation for shapeIndex curves: only allow values that correspond to actual shapes
        if (this.draggingKeyframe.curve.parameter.endsWith('.shapeIndex')) {
          // Extract shapeId from parameter name: "shape.{shapeId}.shapeIndex"
          const match = this.draggingKeyframe.curve.parameter.match(/^shape\.([^.]+)\.shapeIndex$/)
          if (match) {
            const shapeId = match[1]

            // Find all shapes with this shapeId and get their shapeIndex values
            const track = this.draggingKeyframe.track
            let layer = null

            if (track.type === 'shape') {
              // Find the layer containing this shape
              for (let l of this.context.activeObject.allLayers) {
                if (l.shapes && l.shapes.some(s => s.shapeId === shapeId)) {
                  layer = l
                  break
                }
              }
            }

            if (layer) {
              const validIndexes = layer.shapes
                .filter(s => s.shapeId === shapeId)
                .map(s => s.shapeIndex)
                .sort((a, b) => a - b)

              if (validIndexes.length > 0) {
                // Round to nearest integer first
                const roundedValue = Math.round(newValue)

                // Find the closest valid index
                let closestIndex = validIndexes[0]
                let closestDist = Math.abs(roundedValue - closestIndex)

                for (let validIndex of validIndexes) {
                  const dist = Math.abs(roundedValue - validIndex)
                  if (dist < closestDist) {
                    closestDist = dist
                    closestIndex = validIndex
                  }
                }

                newValue = closestIndex
              }
            }
          }
        }

        selectedKeyframe.value = newValue
      }

      // Resort keyframes in all affected curves
      // We need to find all unique curves that contain selected keyframes
      const affectedCurves = new Set()
      for (let selectedKeyframe of this.selectedKeyframes) {
        // Find which curve this keyframe belongs to
        // This is a bit inefficient but works
        const track = this.draggingKeyframe.track
        const obj = track.object

        let animationData = null
        if (track.type === 'object') {
          for (let layer of this.context.activeObject.allLayers) {
            if (layer.children && layer.children.includes(obj)) {
              animationData = layer.animationData
              break
            }
          }
        } else if (track.type === 'shape') {
          const findShapeLayer = (searchObj) => {
            for (let layer of searchObj.children) {
              if (layer.shapes && layer.shapes.includes(obj)) {
                animationData = layer.animationData
                return true
              }
              if (layer.children) {
                for (let child of layer.children) {
                  if (findShapeLayer(child)) return true
                }
              }
            }
            return false
          }
          findShapeLayer(this.context.activeObject)
        }

        if (animationData) {
          for (let curveName in animationData.curves) {
            const curve = animationData.curves[curveName]
            if (curve.keyframes.includes(selectedKeyframe)) {
              affectedCurves.add(curve)
            }
          }
        }
      }

      // Resort all affected curves
      for (let curve of affectedCurves) {
        curve.keyframes.sort((a, b) => a.time - b.time)
      }

      // Sync the activeObject's currentTime with the timeline playhead
      // This ensures the stage shows the animation at the correct time
      if (this.context.activeObject) {
        this.context.activeObject.currentTime = this.timelineState.currentTime
        // Sync DAW backend
        invoke('audio_seek', { seconds: this.timelineState.currentTime });
      }

      // Trigger stage redraw to update object positions based on new keyframe values
      if (this.context.updateUI) {
        console.log('[Timeline] Calling updateUI() to redraw stage after keyframe drag, syncing currentTime =', this.timelineState.currentTime)
        this.context.updateUI()
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Phase 6: Handle tangent handle dragging
    if (this.draggingTangent) {
      const trackY = y - this.ruler.height
      const adjustedX = x - this.trackHeaderWidth
      const adjustedY = trackY - this.trackScrollOffset

      // Get curve visualization parameters
      const trackIndex = this.trackHierarchy.tracks.indexOf(this.draggingTangent.track)
      const trackYPos = this.trackHierarchy.getTrackY(trackIndex)
      const curveHeight = 80
      const startY = trackYPos + 10
      const padding = 5

      // Calculate value range (need to get all curves for this track)
      const obj = this.draggingTangent.track.object
      let animationData = null

      if (this.draggingTangent.track.type === 'object') {
        for (let layer of this.context.activeObject.allLayers) {
          if (layer.children && layer.children.includes(obj)) {
            animationData = layer.animationData
            break
          }
        }
      } else if (this.draggingTangent.track.type === 'shape') {
        const findShapeLayer = (searchObj) => {
          for (let layer of searchObj.children) {
            if (layer.shapes && layer.shapes.includes(obj)) {
              animationData = layer.animationData
              return true
            }
            if (layer.children) {
              for (let child of layer.children) {
                if (findShapeLayer(child)) return true
              }
            }
          }
          return false
        }
        findShapeLayer(this.context.activeObject)
      }

      if (animationData) {
        // Get all curves for value range calculation
        const curves = []
        for (let curveName in animationData.curves) {
          const curve = animationData.curves[curveName]
          if (this.draggingTangent.track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
            curves.push(curve)
          } else if (this.draggingTangent.track.type === 'shape' && curveName.startsWith(`shape.${obj.shapeId}.`)) {
            curves.push(curve)
          }
        }

        // Calculate value range
        let minValue = Infinity
        let maxValue = -Infinity
        for (let curve of curves) {
          for (let keyframe of curve.keyframes) {
            minValue = Math.min(minValue, keyframe.value)
            maxValue = Math.max(maxValue, keyframe.value)
          }
        }
        const valueRange = maxValue - minValue
        const rangePadding = valueRange * 0.1 || 1
        minValue -= rangePadding
        maxValue += rangePadding

        // Get keyframe and adjacent keyframe positions
        const kf = this.draggingTangent.keyframe
        const adj = this.draggingTangent.adjacentKeyframe

        const kfX = this.timelineState.timeToPixel(kf.time)
        const adjX = this.timelineState.timeToPixel(adj.time)

        const valueToY = (value) => {
          const normalizedValue = (value - minValue) / (maxValue - minValue)
          return startY + curveHeight - padding - (normalizedValue * (curveHeight - 2 * padding))
        }

        const kfY = valueToY(kf.value)
        const adjY = valueToY(adj.value)

        // Calculate the new ease values based on mouse position
        const dx = adjX - kfX
        const dy = adjY - kfY

        // Prevent division by zero
        if (Math.abs(dx) > 1 && Math.abs(dy) > 1) {
          let newEaseX, newEaseY

          if (this.draggingTangent.handle === 'out') {
            // Out handle: relative to the keyframe
            newEaseX = (adjustedX - kfX) / dx
            newEaseY = (adjustedY - kfY) / dy
          } else {
            // In handle: relative to the start of the segment (previous keyframe)
            newEaseX = (adjustedX - kfX) / dx
            newEaseY = (adjustedY - kfY) / dy
          }

          // Clamp ease values to reasonable ranges
          // X should be between 0 and 1 (time must be between the two keyframes)
          newEaseX = Math.max(0, Math.min(1, newEaseX))
          // Y can be outside 0-1 for overshoot/undershoot effects
          newEaseY = Math.max(-2, Math.min(3, newEaseY))

          // Update the keyframe's ease
          if (this.draggingTangent.handle === 'out') {
            kf.easeOut = { x: newEaseX, y: newEaseY }
          } else {
            kf.easeIn = { x: newEaseX, y: newEaseY }
          }
        }
      }

      // Trigger redraws
      if (this.context.updateUI) {
        this.context.updateUI()
      }
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Phase 6: Handle segment edge dragging
    if (this.draggingEdge) {
      // Convert mouse position to time
      const adjustedX = x - this.trackHeaderWidth
      let newTime = this.timelineState.pixelToTime(adjustedX)

      // Apply snapping
      newTime = this.timelineState.snapTime(newTime)

      // Ensure time doesn't go negative
      newTime = Math.max(0, newTime)

      // Get the curve to find adjacent segments
      const curve = this.draggingEdge.animationData.curves[this.draggingEdge.curveName]
      if (curve) {
        const frameDuration = 1 / this.timelineState.framerate
        const minGap = frameDuration

        // Find the index of the keyframe we're dragging
        const keyframeIndex = curve.keyframes.indexOf(this.draggingEdge.keyframe)

        if (this.draggingEdge.edge === 'left') {
          // Left edge constraints:
          // 1. Can't go past the right edge of this segment (leave at least 1 frame gap)
          newTime = Math.min(newTime, this.draggingEdge.otherEdgeTime - minGap)

          // 2. Can't go before the end of the previous segment (no gap needed)
          // The previous keyframe (if it has value === 0) is the end of the previous segment
          if (keyframeIndex > 0) {
            const prevKeyframe = curve.keyframes[keyframeIndex - 1]
            if (prevKeyframe.value === 0) {
              newTime = Math.max(newTime, prevKeyframe.time)
            }
          }
        } else {
          // Right edge constraints:
          // 1. Can't go before the left edge of this segment (leave at least 1 frame gap)
          newTime = Math.max(newTime, this.draggingEdge.otherEdgeTime + minGap)

          // 2. Can't go past the start of the next segment (no gap needed)
          // The next keyframe (if it has value > 0) is the start of the next segment
          if (keyframeIndex < curve.keyframes.length - 1) {
            const nextKeyframe = curve.keyframes[keyframeIndex + 1]
            if (nextKeyframe.value > 0) {
              newTime = Math.min(newTime, nextKeyframe.time)
            }
          }
        }

        // Update the keyframe time
        this.draggingEdge.keyframe.time = newTime

        // Resort keyframes in the curve
        curve.keyframes.sort((a, b) => a.time - b.time)
      }

      // Sync with animation playhead
      if (this.context.activeObject) {
        this.context.activeObject.currentTime = this.timelineState.currentTime
        // Sync DAW backend
        invoke('audio_seek', { seconds: this.timelineState.currentTime });
      }

      // Trigger stage redraw
      if (this.context.updateUI) {
        this.context.updateUI()
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Handle audio clip edge dragging (trimming)
    if (this.draggingAudioClipEdge) {
      const adjustedX = x - this.trackHeaderWidth
      const rawTime = this.timelineState.pixelToTime(adjustedX)
      const minClipDuration = this.context.config.minClipDuration

      if (this.draggingAudioClipEdge.edge === 'left') {
        // Dragging left edge - adjust startTime and offset
        const initialEnd = this.draggingAudioClipEdge.initialClipStart + this.draggingAudioClipEdge.initialClipDuration
        const maxStartTime = initialEnd - minClipDuration
        // Quantize the new start time
        let newStartTime = Math.max(0, Math.min(rawTime, maxStartTime))
        newStartTime = this.quantizeTime(newStartTime)
        const startTimeDelta = newStartTime - this.draggingAudioClipEdge.initialClipStart

        this.draggingAudioClipEdge.clip.startTime = newStartTime
        this.draggingAudioClipEdge.clip.offset = this.draggingAudioClipEdge.initialClipOffset + startTimeDelta
        this.draggingAudioClipEdge.clip.duration = this.draggingAudioClipEdge.initialClipDuration - startTimeDelta
        // Also update internalDuration when trimming (this is the content length before looping)
        this.draggingAudioClipEdge.clip.internalDuration = this.draggingAudioClipEdge.initialClipDuration - startTimeDelta

        // Also trim linked video clip if it exists
        if (this.draggingAudioClipEdge.clip.linkedVideoClip) {
          const videoClip = this.draggingAudioClipEdge.clip.linkedVideoClip
          videoClip.startTime = newStartTime
          videoClip.offset = (this.draggingAudioClipEdge.initialLinkedVideoOffset || 0) + startTimeDelta
          videoClip.duration = this.draggingAudioClipEdge.initialClipDuration - startTimeDelta
        }
      } else {
        // Dragging right edge - adjust duration
        const minEndTime = this.draggingAudioClipEdge.initialClipStart + minClipDuration
        // Quantize the new end time
        let newEndTime = Math.max(minEndTime, rawTime)
        newEndTime = this.quantizeTime(newEndTime)
        let newDuration = newEndTime - this.draggingAudioClipEdge.clip.startTime

        // Constrain duration to not exceed source file duration minus offset (for audio clips only)
        // MIDI clips don't have sourceDuration and can be extended freely
        if (this.draggingAudioClipEdge.clip.sourceDuration !== undefined) {
          const maxAvailableDuration = this.draggingAudioClipEdge.clip.sourceDuration - (this.draggingAudioClipEdge.clip.offset || 0)
          newDuration = Math.min(newDuration, maxAvailableDuration)
        }

        this.draggingAudioClipEdge.clip.duration = newDuration
        // Also update internalDuration when trimming (this is the content length before looping)
        this.draggingAudioClipEdge.clip.internalDuration = newDuration

        // Also trim linked video clip if it exists
        if (this.draggingAudioClipEdge.clip.linkedVideoClip) {
          const linkedMaxDuration = this.draggingAudioClipEdge.clip.linkedVideoClip.sourceDuration - this.draggingAudioClipEdge.clip.linkedVideoClip.offset
          this.draggingAudioClipEdge.clip.linkedVideoClip.duration = Math.min(newDuration, linkedMaxDuration)
        }
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Handle loop corner dragging (extending/looping clip)
    if (this.draggingLoopCorner) {
      const adjustedX = x - this.trackHeaderWidth
      const newTime = this.timelineState.pixelToTime(adjustedX)
      const minClipDuration = this.context.config.minClipDuration

      // Calculate new end time and quantize it
      let newEndTime = Math.max(this.draggingLoopCorner.clip.startTime + minClipDuration, newTime)
      newEndTime = this.quantizeTime(newEndTime)
      const newDuration = newEndTime - this.draggingLoopCorner.clip.startTime

      // Update clip duration (no maximum constraint - allows looping)
      this.draggingLoopCorner.clip.duration = newDuration

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Handle audio clip dragging
    if (this.draggingAudioClip) {
      // Adjust coordinates to timeline area
      const adjustedX = x - this.trackHeaderWidth

      // Convert mouse position to time
      const newTime = this.timelineState.pixelToTime(adjustedX)

      // Calculate time delta
      const timeDelta = newTime - this.draggingAudioClip.initialMouseTime

      // Update clip's start time (ensure it doesn't go negative)
      this.draggingAudioClip.clip.startTime = Math.max(0, this.draggingAudioClip.initialClipStartTime + timeDelta)

      // Also move linked video clip if it exists
      if (this.draggingAudioClip.clip.linkedVideoClip) {
        this.draggingAudioClip.clip.linkedVideoClip.startTime = this.draggingAudioClip.clip.startTime
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Handle video clip edge dragging (trimming)
    if (this.draggingVideoClipEdge) {
      const adjustedX = x - this.trackHeaderWidth
      const newTime = this.timelineState.pixelToTime(adjustedX)
      const minClipDuration = this.context.config.minClipDuration

      if (this.draggingVideoClipEdge.edge === 'left') {
        // Dragging left edge - adjust startTime and offset
        const initialEnd = this.draggingVideoClipEdge.initialClipStart + this.draggingVideoClipEdge.initialClipDuration
        const maxStartTime = initialEnd - minClipDuration
        const newStartTime = Math.max(0, Math.min(newTime, maxStartTime))
        const startTimeDelta = newStartTime - this.draggingVideoClipEdge.initialClipStart

        this.draggingVideoClipEdge.clip.startTime = newStartTime
        this.draggingVideoClipEdge.clip.offset = this.draggingVideoClipEdge.initialClipOffset + startTimeDelta
        this.draggingVideoClipEdge.clip.duration = this.draggingVideoClipEdge.initialClipDuration - startTimeDelta

        // Also trim linked audio clip if it exists
        if (this.draggingVideoClipEdge.clip.linkedAudioClip) {
          const audioClip = this.draggingVideoClipEdge.clip.linkedAudioClip
          audioClip.startTime = newStartTime
          audioClip.offset = (this.draggingVideoClipEdge.initialLinkedAudioOffset || 0) + startTimeDelta
          audioClip.duration = this.draggingVideoClipEdge.initialClipDuration - startTimeDelta
        }
      } else {
        // Dragging right edge - adjust duration
        const minEndTime = this.draggingVideoClipEdge.initialClipStart + minClipDuration
        const newEndTime = Math.max(minEndTime, newTime)
        let newDuration = newEndTime - this.draggingVideoClipEdge.clip.startTime

        // Constrain duration to not exceed source file duration minus offset
        const maxAvailableDuration = this.draggingVideoClipEdge.clip.sourceDuration - this.draggingVideoClipEdge.clip.offset
        newDuration = Math.min(newDuration, maxAvailableDuration)

        this.draggingVideoClipEdge.clip.duration = newDuration

        // Also trim linked audio clip if it exists
        if (this.draggingVideoClipEdge.clip.linkedAudioClip) {
          const linkedMaxDuration = this.draggingVideoClipEdge.clip.linkedAudioClip.sourceDuration - this.draggingVideoClipEdge.clip.linkedAudioClip.offset
          this.draggingVideoClipEdge.clip.linkedAudioClip.duration = Math.min(newDuration, linkedMaxDuration)
        }
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Handle video clip dragging
    if (this.draggingVideoClip) {
      // Adjust coordinates to timeline area
      const adjustedX = x - this.trackHeaderWidth

      // Convert mouse position to time
      const newTime = this.timelineState.pixelToTime(adjustedX)

      // Calculate time delta
      const timeDelta = newTime - this.draggingVideoClip.initialMouseTime

      // Update clip's start time (ensure it doesn't go negative)
      this.draggingVideoClip.clip.startTime = Math.max(0, this.draggingVideoClip.initialClipStartTime + timeDelta)

      // Also move linked audio clip if it exists
      if (this.draggingVideoClip.clip.linkedAudioClip) {
        this.draggingVideoClip.clip.linkedAudioClip.startTime = this.draggingVideoClip.clip.startTime
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Phase 6: Handle segment dragging
    if (this.draggingSegment) {
      // Adjust coordinates to timeline area
      const trackY = y - this.ruler.height
      const adjustedX = x - this.trackHeaderWidth

      // Convert mouse position to time
      const newTime = this.timelineState.pixelToTime(adjustedX)

      // Calculate time delta
      const timeDelta = newTime - this.draggingSegment.initialMouseTime

      // Get all curves for this object/shape from the animationData
      const prefix = this.draggingSegment.track.type === 'object'
        ? `child.${this.draggingSegment.objectIdx}.`
        : `shape.${this.draggingSegment.objectIdx}.`

      // Shift all keyframes by the time delta
      for (let curveName in this.draggingSegment.animationData.curves) {
        if (curveName.startsWith(prefix)) {
          const curve = this.draggingSegment.animationData.curves[curveName]

          for (let keyframe of curve.keyframes) {
            // Store initial time if not already stored
            if (!keyframe.initialSegmentDragTime) {
              keyframe.initialSegmentDragTime = keyframe.time
            }

            // Apply delta and ensure time doesn't go negative
            keyframe.time = Math.max(0, keyframe.initialSegmentDragTime + timeDelta)
          }

          // Resort keyframes after time shift
          curve.keyframes.sort((a, b) => a.time - b.time)
        }
      }

      // Sync with animation playhead
      if (this.context.activeObject) {
        this.context.activeObject.currentTime = this.timelineState.currentTime
        // Sync DAW backend
        invoke('audio_seek', { seconds: this.timelineState.currentTime });
      }

      // Trigger stage redraw
      if (this.context.updateUI) {
        this.context.updateUI()
      }

      // Trigger timeline redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Update cursor based on hover position (when not dragging)
    if (!this.draggingAudioClip && !this.draggingVideoClip &&
        !this.draggingAudioClipEdge && !this.draggingVideoClipEdge &&
        !this.draggingKeyframe && !this.draggingPlayhead && !this.draggingSegment &&
        !this.draggingLoopCorner) {
      const trackY = y - this.ruler.height
      if (trackY >= 0 && x >= this.trackHeaderWidth) {
        const adjustedY = trackY - this.trackScrollOffset
        const adjustedX = x - this.trackHeaderWidth
        const track = this.trackHierarchy.getTrackAtY(adjustedY)

        if (track) {
          // Check for audio/MIDI clip loop corner (top-right) - must check before edge detection
          if (track.type === 'audio') {
            const loopCornerInfo = this.getAudioClipLoopCornerAtPoint(track, adjustedX, adjustedY)
            if (loopCornerInfo) {
              // Use the same rotate cursor as the transform tool corner handles
              this.cursor = "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24' fill='currentColor' viewBox='0 0 16 16'%3E%3Cpath fill-rule='evenodd' d='M8 3a5 5 0 1 1-4.546 2.914.5.5 0 0 0-.908-.417A6 6 0 1 0 8 2z'/%3E%3Cpath d='M8 4.466V.534a.25.25 0 0 0-.41-.192L5.23 2.308a.25.25 0 0 0 0 .384l2.36 1.966A.25.25 0 0 0 8 4.466'/%3E%3C/svg%3E\") 12 12, auto"
              return false
            }
          }

          // Check for audio clip edge
          if (track.type === 'audio') {
            const audioEdgeInfo = this.getAudioClipEdgeAtPoint(track, adjustedX, adjustedY)
            if (audioEdgeInfo) {
              this.cursor = audioEdgeInfo.edge === 'left' ? 'w-resize' : 'e-resize'
              return false
            }
          }
          // Check for video clip edge
          else if (track.type === 'video') {
            const videoEdgeInfo = this.getVideoClipEdgeAtPoint(track, adjustedX, adjustedY)
            if (videoEdgeInfo) {
              this.cursor = videoEdgeInfo.edge === 'left' ? 'w-resize' : 'e-resize'
              return false
            }
          }
        }
      }
      // Reset cursor if not over an edge
      this.cursor = 'default'
    }

    return false
  }

  mouseup(x, y) {
    if (this.draggingPlayhead) {
      // Let the ruler handle the mouseup
      this.ruler.mouseup(x, y)

      this.draggingPlayhead = false
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")
      return true
    }

    // Phase 5: Complete keyframe dragging
    if (this.draggingKeyframe) {
      console.log(`Finished dragging ${this.selectedKeyframes.size} keyframe(s)`)

      // Clean up initial drag positions from all selected keyframes
      for (let selectedKeyframe of this.selectedKeyframes) {
        delete selectedKeyframe.initialDragTime
        delete selectedKeyframe.initialDragValue
      }

      // Clean up dragging state
      this.draggingKeyframe = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Phase 6: Complete tangent dragging
    if (this.draggingTangent) {
      console.log('Finished dragging', this.draggingTangent.handle, 'tangent handle')

      // Clean up dragging state
      this.draggingTangent = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Phase 6: Complete edge dragging
    if (this.draggingEdge) {
      console.log('Finished dragging', this.draggingEdge.edge, 'edge')

      // Clean up dragging state
      this.draggingEdge = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Complete audio clip edge dragging (trimming)
    if (this.draggingAudioClipEdge) {
      console.log('Finished trimming audio clip edge')

      const clip = this.draggingAudioClipEdge.clip
      const trackId = this.draggingAudioClipEdge.audioTrack.audioTrackId
      const clipId = clip.clipId

      // If dragging left edge, also move the clip's timeline position
      if (this.draggingAudioClipEdge.edge === 'left') {
        invoke('audio_move_clip', {
          trackId: trackId,
          clipId: clipId,
          newStartTime: clip.startTime
        }).catch(error => {
          console.error('Failed to move audio clip in backend:', error)
        })
      }

      // Update the internal trim boundaries
      // internal_start = offset, internal_end = offset + duration (content region)
      invoke('audio_trim_clip', {
        trackId: trackId,
        clipId: clipId,
        internalStart: clip.offset,
        internalEnd: clip.offset + clip.duration
      }).catch(error => {
        console.error('Failed to trim audio clip in backend:', error)
      })

      // Also update linked video clip if it exists
      if (this.draggingAudioClipEdge.clip.linkedVideoClip) {
        console.log('Linked video clip also trimmed')
      }

      // Clean up dragging state
      this.draggingAudioClipEdge = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Complete loop corner dragging (extending/looping clip)
    if (this.draggingLoopCorner) {
      console.log('Finished extending clip via loop corner')

      const clip = this.draggingLoopCorner.clip
      const trackId = this.draggingLoopCorner.audioTrack.audioTrackId
      const clipId = clip.clipId

      // Call audio_extend_clip to update the external duration in the backend
      invoke('audio_extend_clip', {
        trackId: trackId,
        clipId: clipId,
        newExternalDuration: clip.duration
      }).catch(error => {
        console.error('Failed to extend audio clip in backend:', error)
      })

      // Clean up dragging state
      this.draggingLoopCorner = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Complete video clip edge dragging (trimming)
    if (this.draggingVideoClipEdge) {
      console.log('Finished trimming video clip edge')

      // Update linked audio clip in backend if it exists
      if (this.draggingVideoClipEdge.clip.linkedAudioClip) {
        const linkedAudioClip = this.draggingVideoClipEdge.clip.linkedAudioClip
        const audioTrack = this.draggingVideoClipEdge.videoLayer.linkedAudioTrack
        if (audioTrack) {
          const trackId = audioTrack.audioTrackId
          const clipId = linkedAudioClip.clipId

          // If dragging left edge, also move the clip's timeline position
          if (this.draggingVideoClipEdge.edge === 'left') {
            invoke('audio_move_clip', {
              trackId: trackId,
              clipId: clipId,
              newStartTime: linkedAudioClip.startTime
            }).catch(error => {
              console.error('Failed to move linked audio clip in backend:', error)
            })
          }

          // Update the internal trim boundaries
          invoke('audio_trim_clip', {
            trackId: trackId,
            clipId: clipId,
            internalStart: linkedAudioClip.offset,
            internalEnd: linkedAudioClip.offset + linkedAudioClip.duration
          }).catch(error => {
            console.error('Failed to trim linked audio clip in backend:', error)
          })
        }
      }

      // Clean up dragging state
      this.draggingVideoClipEdge = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Complete audio clip dragging
    if (this.draggingAudioClip) {
      console.log('Finished dragging audio clip')

      // Update backend with new clip position
      invoke('audio_move_clip', {
        trackId: this.draggingAudioClip.audioTrack.audioTrackId,
        clipId: this.draggingAudioClip.clip.clipId,
        newStartTime: this.draggingAudioClip.clip.startTime
      }).catch(error => {
        console.error('Failed to move clip in backend:', error)
      })

      // Also update linked video clip in backend if it exists
      if (this.draggingAudioClip.clip.linkedVideoClip) {
        // Video clips don't have a backend move command yet, so just log for now
        console.log('Linked video clip also moved to time', this.draggingAudioClip.clip.startTime)
      }

      // Clean up dragging state
      this.draggingAudioClip = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Complete video clip dragging
    if (this.draggingVideoClip) {
      console.log('Finished dragging video clip')

      // Video clips don't have a backend position yet (they're just visual)
      // But we need to update the linked audio clip in the backend
      if (this.draggingVideoClip.clip.linkedAudioClip) {
        const linkedAudioClip = this.draggingVideoClip.clip.linkedAudioClip
        // Find the audio track that contains this clip
        const audioTrack = this.draggingVideoClip.videoLayer.linkedAudioTrack
        if (audioTrack) {
          invoke('audio_move_clip', {
            trackId: audioTrack.audioTrackId,
            clipId: linkedAudioClip.clipId,
            newStartTime: linkedAudioClip.startTime
          }).catch(error => {
            console.error('Failed to move linked audio clip in backend:', error)
          })
        }
      }

      // Clean up dragging state
      this.draggingVideoClip = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    // Phase 6: Complete segment dragging
    if (this.draggingSegment) {
      console.log('Finished dragging segment')

      // Clean up initial drag times from all affected keyframes
      const prefix = this.draggingSegment.track.type === 'object'
        ? `child.${this.draggingSegment.objectIdx}.`
        : `shape.${this.draggingSegment.objectIdx}.`

      for (let curveName in this.draggingSegment.animationData.curves) {
        if (curveName.startsWith(prefix)) {
          const curve = this.draggingSegment.animationData.curves[curveName]
          for (let keyframe of curve.keyframes) {
            delete keyframe.initialSegmentDragTime
          }
        }
      }

      // Clean up dragging state
      this.draggingSegment = null
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")

      // Final redraw
      if (this.requestRedraw) this.requestRedraw()
      return true
    }

    return false
  }

  /**
   * Handle right-click context menu (Phase 5/6)
   * Shows menu with interpolation options and delete for keyframes
   * Shift+right-click for quick delete
   */
  contextmenu(x, y, event) {
    // Check if right-clicking in timeline area
    const trackY = y - this.ruler.height
    if (trackY >= 0 && x >= this.trackHeaderWidth) {
      const adjustedY = trackY - this.trackScrollOffset
      const adjustedX = x - this.trackHeaderWidth
      const track = this.trackHierarchy.getTrackAtY(adjustedY)

      // First check if clicking on a clip (audio or MIDI)
      if (track && (track.type === 'audio')) {
        const clipInfo = this.getAudioClipAtPoint(track, adjustedX, adjustedY)
        if (clipInfo) {
          this.showClipContextMenu(clipInfo.clip, clipInfo.audioTrack)
          return true
        }
      }

      if (track && (track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'curve') {
        // Use similar logic to handleCurveClick to find if we're clicking on a keyframe
        const trackIndex = this.trackHierarchy.tracks.indexOf(track)
        const trackYPos = this.trackHierarchy.getTrackY(trackIndex)

        const curveHeight = 80
        const startY = trackYPos + 10
        const padding = 5

        // Check if y is within curve area
        if (adjustedY >= startY && adjustedY <= startY + curveHeight) {
          // Get AnimationData and curves for this track
          const obj = track.object
          let animationData = null

          if (track.type === 'object') {
            for (let layer of this.context.activeObject.allLayers) {
              if (layer.children && layer.children.includes(obj)) {
                animationData = layer.animationData
                break
              }
            }
          } else if (track.type === 'shape') {
            const findShapeLayer = (searchObj) => {
              for (let layer of searchObj.children) {
                if (layer.shapes && layer.shapes.includes(obj)) {
                  animationData = layer.animationData
                  return true
                }
                if (layer.children) {
                  for (let child of layer.children) {
                    if (findShapeLayer(child)) return true
                  }
                }
              }
              return false
            }
            findShapeLayer(this.context.activeObject)
          }

          if (!animationData) return false

          // Get all curves for this object/shape
          const curves = []
          for (let curveName in animationData.curves) {
            const curve = animationData.curves[curveName]
            if (track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
              curves.push(curve)
            } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.shapeId}.`)) {
              curves.push(curve)
            }
          }

          if (curves.length === 0) return false

          // Calculate value range for scaling
          let minValue = Infinity
          let maxValue = -Infinity
          for (let curve of curves) {
            for (let keyframe of curve.keyframes) {
              minValue = Math.min(minValue, keyframe.value)
              maxValue = Math.max(maxValue, keyframe.value)
            }
          }
          const valueRange = maxValue - minValue
          const rangePadding = valueRange * 0.1 || 1
          minValue -= rangePadding
          maxValue += rangePadding

          // Check if right-clicking on a keyframe (within 8px)
          // Find the CLOSEST keyframe, not just the first one (and skip hidden curves)
          let closestKeyframe = null
          let closestCurve = null
          let closestDistance = 8  // Maximum hit distance

          for (let curve of curves) {
            // Skip hidden curves
            if (this.hiddenCurves.has(curve.parameter)) continue

            for (let i = 0; i < curve.keyframes.length; i++) {
              const keyframe = curve.keyframes[i]
              const kfX = this.timelineState.timeToPixel(keyframe.time)
              const kfY = startY + curveHeight - padding - ((keyframe.value - minValue) / (maxValue - minValue) * (curveHeight - 2 * padding))
              const distance = Math.sqrt((adjustedX - kfX) ** 2 + (adjustedY - kfY) ** 2)

              if (distance < closestDistance) {
                closestDistance = distance
                closestKeyframe = keyframe
                closestCurve = curve
              }
            }
          }

          if (closestKeyframe) {
            const keyframe = closestKeyframe
            const curve = closestCurve

            // Phase 6: Check if shift key is pressed for quick delete
            const shiftPressed = event && event.shiftKey

            if (shiftPressed) {
                  // Shift+right-click: quick delete
                  if (this.selectedKeyframes.size > 1) {
                    // Delete all selected keyframes
                    const keyframesToDelete = Array.from(this.selectedKeyframes)
                    for (let kf of keyframesToDelete) {
                      for (let c of curves) {
                        const idx = c.keyframes.indexOf(kf)
                        if (idx !== -1 && c.keyframes.length > 1) {
                          c.keyframes.splice(idx, 1)
                        }
                      }
                      this.selectedKeyframes.delete(kf)
                    }
                    console.log(`Deleted ${keyframesToDelete.length} keyframes`)
                  } else {
                    // Single keyframe deletion
                    if (curve.keyframes.length > 1) {
                      console.log(`Deleting keyframe at time ${keyframe.time}`)
                      curve.keyframes.splice(i, 1)
                      this.selectedKeyframes.delete(keyframe)
                    }
                  }
                  if (this.requestRedraw) this.requestRedraw()
                  return true
                } else {
                  // Regular right-click: show context menu
                  if (this.selectedKeyframes.size > 1) {
                    // If right-clicking on a selected keyframe, show menu for all selected
                    if (this.selectedKeyframes.has(keyframe)) {
                      this.showKeyframeContextMenu(Array.from(this.selectedKeyframes), curves)
                    } else {
                      // Right-clicking on unselected keyframe: select it and show menu
                      this.selectedKeyframes.clear()
                      this.selectedKeyframes.add(keyframe)
                      this.showKeyframeContextMenu([keyframe], curves, curve)
                    }
                  } else {
                    // No multi-selection: select this keyframe and show menu
                    this.selectedKeyframes.clear()
                    this.selectedKeyframes.add(keyframe)
                    this.showKeyframeContextMenu([keyframe], curves, curve)
                  }
                  return true
                }
          }  // end if (closestKeyframe)
        }
      }
    }

    return false
  }

  /**
   * Show Tauri context menu for keyframe operations (Phase 6)
   * Includes interpolation type options and delete
   */
  async showKeyframeContextMenu(keyframesToDelete, curves, singleCurve = null) {
    const { Menu, MenuItem, Submenu } = window.__TAURI__.menu
    const { PhysicalPosition, LogicalPosition } = window.__TAURI__.dpi

    // Build menu items
    const items = []

    // Phase 6: Add interpolation type submenu (only for single keyframe)
    if (keyframesToDelete.length === 1 && singleCurve) {
      const keyframe = keyframesToDelete[0]
      const currentType = keyframe.interpolation || 'linear'

      const interpolationSubmenu = await Submenu.new({
        text: 'Interpolation',
        items: [
          await MenuItem.new({
            text: currentType === 'linear' ? '✓ Linear' : 'Linear',
            action: async () => {
              keyframe.interpolation = 'linear'
              console.log('Changed interpolation to linear')
              // Keep flag set until next mousedown processes it
              if (this.context.updateUI) this.context.updateUI()
              if (this.requestRedraw) this.requestRedraw()
            }
          }),
          await MenuItem.new({
            text: currentType === 'bezier' ? '✓ Bezier' : 'Bezier',
            action: async () => {
              keyframe.interpolation = 'bezier'
              if (!keyframe.easeIn) keyframe.easeIn = { x: 0.42, y: 0 }
              if (!keyframe.easeOut) keyframe.easeOut = { x: 0.58, y: 1 }
              if (this.context.updateUI) this.context.updateUI()
              if (this.requestRedraw) this.requestRedraw()
            }
          }),
          await MenuItem.new({
            text: currentType === 'step' || currentType === 'hold' ? '✓ Step (Hold)' : 'Step (Hold)',
            action: async () => {
              keyframe.interpolation = 'step'
              console.log('Changed interpolation to step')
              // Keep flag set until next mousedown processes it
              if (this.context.updateUI) this.context.updateUI()
              if (this.requestRedraw) this.requestRedraw()
            }
          }),
          await MenuItem.new({
            text: currentType === 'zero' ? '✓ Zero' : 'Zero',
            action: async () => {
              keyframe.interpolation = 'zero'
              console.log('Changed interpolation to zero')
              // Keep flag set until next mousedown processes it
              if (this.context.updateUI) this.context.updateUI()
              if (this.requestRedraw) this.requestRedraw()
            }
          })
        ]
      })

      items.push(interpolationSubmenu)
    }

    // Add delete option
    items.push(await MenuItem.new({
      text: `Delete ${keyframesToDelete.length} keyframe${keyframesToDelete.length > 1 ? 's' : ''}`,
      action: async () => {
          // Perform deletion
          console.log(`Deleting ${keyframesToDelete.length} selected keyframes`)

          // For each keyframe to delete
          for (let keyframe of keyframesToDelete) {
            // Find which curve(s) contain this keyframe
            for (let curve of curves) {
              const index = curve.keyframes.indexOf(keyframe)
              if (index !== -1) {
                // Check if this is the last keyframe in this curve
                if (curve.keyframes.length > 1) {
                  curve.keyframes.splice(index, 1)
                  console.log(`Deleted keyframe from curve ${curve.parameter}`)
                } else {
                  console.log(`Skipped deleting last keyframe in curve ${curve.parameter}`)
                }
                break
              }
            }
          }

          // Clear the selection
          this.selectedKeyframes.clear()

          // Trigger redraw
          if (this.requestRedraw) this.requestRedraw()
        }
      }))

    const menu = await Menu.new({ items })

    // Show menu at mouse position (using lastEvent for clientX/clientY)
    const clientX = this.lastEvent?.clientX || 0
    const clientY = this.lastEvent?.clientY || 0
    const position = new PhysicalPosition(clientX, clientY)
    console.log(position)
    // await menu.popup({ at: position })
    await menu.popup(position)
  }

  /**
   * Show context menu for audio/MIDI clips
   * Currently supports: Rename
   */
  async showClipContextMenu(clip, audioTrack) {
    const { Menu, MenuItem } = window.__TAURI__.menu
    const { PhysicalPosition } = window.__TAURI__.dpi

    const items = []

    // Rename option
    items.push(await MenuItem.new({
      text: 'Rename',
      action: async () => {
        const newName = prompt('Enter new name for clip:', clip.name || '')
        if (newName !== null && newName.trim() !== '') {
          clip.name = newName.trim()
          console.log(`Renamed clip to "${clip.name}"`)
          if (this.requestRedraw) this.requestRedraw()
        }
      }
    }))

    const menu = await Menu.new({ items })

    // Show menu at mouse position
    const clientX = this.lastEvent?.clientX || 0
    const clientY = this.lastEvent?.clientY || 0
    const position = new PhysicalPosition(clientX, clientY)
    await menu.popup(position)
  }

  /**
   * Copy selected keyframes to clipboard (Phase 6)
   */
  copySelectedKeyframes() {
    if (this.selectedKeyframes.size === 0) {
      return false // No keyframes to copy
    }

    // Find the earliest time among selected keyframes (this will be the reference point)
    let minTime = Infinity
    for (let keyframe of this.selectedKeyframes) {
      minTime = Math.min(minTime, keyframe.time)
    }

    // Build clipboard data with relative times
    const clipboardData = []

    // We need to find which curves these keyframes belong to
    // Iterate through all tracks to find curves containing selected keyframes
    for (let track of this.trackHierarchy.tracks) {
      if (track.type !== 'object' && track.type !== 'shape') continue

      const obj = track.object
      let animationData = null

      // Find animation data
      if (track.type === 'object') {
        for (let layer of this.context.activeObject.allLayers) {
          if (layer.children && layer.children.includes(obj)) {
            animationData = layer.animationData
            break
          }
        }
      } else if (track.type === 'shape') {
        const findShapeLayer = (searchObj) => {
          for (let layer of searchObj.children) {
            if (layer.shapes && layer.shapes.includes(obj)) {
              animationData = layer.animationData
              return true
            }
            if (layer.children) {
              for (let child of layer.children) {
                if (findShapeLayer(child)) return true
              }
            }
          }
          return false
        }
        findShapeLayer(this.context.activeObject)
      }

      if (!animationData) continue

      // Check all curves
      for (let curveName in animationData.curves) {
        const curve = animationData.curves[curveName]
        const prefix = track.type === 'object' ? `child.${obj.idx}.` : `shape.${obj.shapeId}.`

        if (!curveName.startsWith(prefix)) continue

        // Check which keyframes in this curve are selected
        for (let keyframe of curve.keyframes) {
          if (this.selectedKeyframes.has(keyframe)) {
            // Store keyframe data with relative time
            clipboardData.push({
              curve: curve,
              curveName: curveName,
              keyframeData: {
                time: keyframe.time - minTime,  // Relative time
                value: keyframe.value,
                interpolation: keyframe.interpolation,
                easeIn: keyframe.easeIn ? { ...keyframe.easeIn } : undefined,
                easeOut: keyframe.easeOut ? { ...keyframe.easeOut } : undefined
              }
            })
          }
        }
      }
    }

    this.keyframeClipboard = {
      keyframes: clipboardData,
      baseTime: minTime
    }

    console.log(`Copied ${clipboardData.length} keyframe(s) to clipboard`)
    return true // Successfully copied keyframes
  }

  /**
   * Paste keyframes from clipboard (Phase 6)
   */
  pasteKeyframes() {
    if (!this.keyframeClipboard || this.keyframeClipboard.keyframes.length === 0) {
      return false // No keyframes in clipboard
    }

    // Paste at current playhead time
    const pasteTime = this.timelineState.currentTime

    // Clear current selection
    this.selectedKeyframes.clear()

    // Paste each keyframe
    for (let clipboardItem of this.keyframeClipboard.keyframes) {
      const curve = clipboardItem.curve
      const kfData = clipboardItem.keyframeData

      // Calculate absolute time for pasted keyframe
      const absoluteTime = pasteTime + kfData.time

      // Create new keyframe
      const newKeyframe = {
        time: absoluteTime,
        value: kfData.value,
        interpolation: kfData.interpolation || 'linear',
        easeIn: kfData.easeIn ? { ...kfData.easeIn } : { x: 0.42, y: 0 },
        easeOut: kfData.easeOut ? { ...kfData.easeOut } : { x: 0.58, y: 1 },
        idx: this.generateUUID()
      }

      // Add to curve
      curve.addKeyframe(newKeyframe)

      // Select the newly pasted keyframe
      this.selectedKeyframes.add(newKeyframe)
    }

    console.log(`Pasted ${this.keyframeClipboard.keyframes.length} keyframe(s) at time ${pasteTime}`)

    // Trigger redraws
    if (this.context.updateUI) {
      this.context.updateUI()
    }
    if (this.requestRedraw) this.requestRedraw()

    return true // Successfully pasted keyframes
  }

  // Zoom controls (can be called from keyboard shortcuts)
  zoomIn() {
    this.timelineState.zoomIn()
  }

  zoomOut() {
    this.timelineState.zoomOut()
  }

  // Toggle time format
  toggleTimeFormat() {
    if (this.timelineState.timeFormat === 'frames') {
      this.timelineState.timeFormat = 'seconds'
    } else if (this.timelineState.timeFormat === 'seconds') {
      this.timelineState.timeFormat = 'measures'
    } else {
      this.timelineState.timeFormat = 'frames'
    }
  }

  // Fetch automation name from backend and cache it
  async fetchAutomationName(trackId, nodeId) {
    const cacheKey = `${trackId}:${nodeId}`

    // Return cached value if available
    if (this.automationNameCache.has(cacheKey)) {
      return this.automationNameCache.get(cacheKey)
    }

    try {
      const name = await invoke('automation_get_name', {
        trackId: trackId,
        nodeId: nodeId
      })

      // Cache the result
      if (name && name !== '') {
        this.automationNameCache.set(cacheKey, name)
        return name
      }
    } catch (err) {
      console.error(`Failed to fetch automation name for node ${nodeId}:`, err)
    }

    // Fallback to node ID if fetch fails or returns empty
    return `${nodeId}`
  }

  // Get automation name synchronously from cache, trigger fetch if not cached
  getAutomationName(trackId, nodeId) {
    const cacheKey = `${trackId}:${nodeId}`

    if (this.automationNameCache.has(cacheKey)) {
      return this.automationNameCache.get(cacheKey)
    }

    // Trigger async fetch in background
    this.fetchAutomationName(trackId, nodeId).then(() => {
      // Redraw when name arrives
      if (this.context.timelineWidget?.requestRedraw) {
        this.context.timelineWidget.requestRedraw()
      }
    })

    // Return node ID as placeholder while fetching
    return `${nodeId}`
  }
}

/**
 * VirtualPiano - Interactive piano keyboard for MIDI input
 * Displays a piano keyboard that users can click/play
 * Can be connected to MIDI tracks in the DAW backend
 */
class VirtualPiano extends Widget {
  constructor() {
    super(0, 0);

    // Piano configuration - width scales based on height
    this.whiteKeyAspectRatio = 6.0; // White key height:width ratio (taller keys)
    this.blackKeyWidthRatio = 0.6; // Black key width as ratio of white key width
    this.blackKeyHeightRatio = 0.62; // Black key height as ratio of white key height

    // State
    this.pressedKeys = new Set(); // Currently pressed MIDI note numbers (user input)
    this.playingNotes = new Set(); // Currently playing notes (from MIDI playback)
    this.hoveredKey = null; // Currently hovered key
    this.visibleStartNote = 48; // C3 - will be adjusted based on pane width
    this.visibleEndNote = 72; // C5 - will be adjusted based on pane width

    // Keyboard control state
    this.octaveOffset = 0; // Octave transpose (-2 to +2)
    this.velocity = 100; // Default velocity (0-127)
    this.sustainActive = false; // Sustain pedal (Tab key)
    this.activeKeyPresses = new Map(); // Map of keyboard key -> MIDI note that's currently playing
    this.sustainedNotes = new Set(); // Notes being held by sustain

    // MIDI note mapping (white keys in an octave: C, D, E, F, G, A, B)
    this.whiteKeysInOctave = [0, 2, 4, 5, 7, 9, 11]; // Semitones from C
    // Black keys indexed by white key position (after which white key the black key appears)
    // Position 0 (after C), 1 (after D), null (no black after E), 3 (after F), 4 (after G), 5 (after A), null (no black after B)
    this.blackKeysInOctave = [1, 3, null, 6, 8, 10, null]; // Actual semitone values

    // Keyboard bindings matching piano layout (QWERTY)
    // TODO: Auto-detect keyboard layout and generate mapping dynamically
    // Black keys: W E (one group) T Y U (other group) O P (next group)
    // White keys: A S D F G H J K L ; '
    this.keyboardMap = {
      'a': 60, // C4
      'w': 61, // C#4
      's': 62, // D4
      'e': 63, // D#4
      'd': 64, // E4
      'f': 65, // F4
      't': 66, // F#4
      'g': 67, // G4
      'y': 68, // G#4
      'h': 69, // A4
      'u': 70, // A#4
      'j': 71, // B4
      'k': 72, // C5
      'o': 73, // C#5
      'l': 74, // D5
      'p': 75, // D#5
      ';': 76, // E5
      "'": 77, // F5
    };

    // Reverse mapping for displaying keyboard keys on piano keys
    this.noteToKeyMap = {};
    for (const [key, note] of Object.entries(this.keyboardMap)) {
      this.noteToKeyMap[note] = key.toUpperCase();
    }

    // Setup keyboard event listeners
    this.setupKeyboardListeners();
  }

  /**
   * Setup keyboard event listeners for computer keyboard input
   */
  setupKeyboardListeners() {
    window.addEventListener('keydown', (e) => {
      if (e.repeat) return; // Ignore key repeats

      const key = e.key.toLowerCase();

      // Handle sustain (Tab key)
      if (key === 'tab') {
        this.sustainActive = true;
        e.preventDefault();
        return;
      }

      // Handle control keys (Z, X for octave, C, V for velocity)
      if (key === 'z') {
        this.octaveOffset = Math.max(-2, this.octaveOffset - 1);
        // Trigger a redraw to update the visible piano range
        if (window.context && window.context.pianoRedraw) {
          window.context.pianoRedraw();
        }
        e.preventDefault();
        return;
      }
      if (key === 'x') {
        this.octaveOffset = Math.min(2, this.octaveOffset + 1);
        // Trigger a redraw to update the visible piano range
        if (window.context && window.context.pianoRedraw) {
          window.context.pianoRedraw();
        }
        e.preventDefault();
        return;
      }
      if (key === 'c') {
        this.velocity = Math.max(1, this.velocity - 10);
        e.preventDefault();
        return;
      }
      if (key === 'v') {
        this.velocity = Math.min(127, this.velocity + 10);
        e.preventDefault();
        return;
      }

      // Handle piano keys
      const baseNote = this.keyboardMap[key];
      if (baseNote !== undefined) {
        // Check if this key is already pressed (prevents duplicate note-ons from OS key repeat quirks)
        if (this.activeKeyPresses.has(key)) {
          e.preventDefault();
          return;
        }

        // Note: octave offset is applied by shifting the visible piano range
        // so we play the base note directly
        const note = baseNote + (this.octaveOffset * 12);
        // Clamp to valid MIDI range (0-127)
        if (note >= 0 && note <= 127) {
          // Track which key is playing which note
          this.activeKeyPresses.set(key, note);
          this.noteOn(note, this.velocity);
          e.preventDefault();
        }
      }
    });

    window.addEventListener('keyup', (e) => {
      const key = e.key.toLowerCase();

      // Handle sustain release
      if (key === 'tab') {
        this.sustainActive = false;
        // Release only the sustained notes that aren't currently being held by a key
        const currentlyPlayingNotes = new Set(this.activeKeyPresses.values());
        for (const note of this.sustainedNotes) {
          if (!currentlyPlayingNotes.has(note)) {
            this.noteOff(note);
          }
        }
        this.sustainedNotes.clear();
        e.preventDefault();
        return;
      }

      // Ignore control keys on keyup
      if (['z', 'x', 'c', 'v'].includes(key)) {
        return;
      }

      // Look up which note this key was playing
      const transposedNote = this.activeKeyPresses.get(key);
      if (transposedNote !== undefined) {
        this.activeKeyPresses.delete(key);

        // If sustain is active, add to sustained notes instead of releasing
        if (this.sustainActive) {
          this.sustainedNotes.add(transposedNote);
        } else {
          this.noteOff(transposedNote);
        }
        e.preventDefault();
      }
    });
  }

  /**
   * Convert MIDI note number to note info
   */
  getMidiNoteInfo(midiNote) {
    const octave = Math.floor(midiNote / 12) - 1;
    const semitone = midiNote % 12;
    const isBlack = [1, 3, 6, 8, 10].includes(semitone);
    const noteNames = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
    return {
      octave,
      semitone,
      isBlack,
      name: noteNames[semitone] + octave
    };
  }

  /**
   * Calculate key position and dimensions for a given MIDI note
   * @param {number} midiNote - MIDI note number
   * @param {number} whiteKeyHeight - Height of white keys (full pane height)
   * @param {number} whiteKeyWidth - Width of white keys (calculated from height)
   * @param {number} offsetX - Horizontal offset for centering
   */
  getKeyGeometry(midiNote, whiteKeyHeight, whiteKeyWidth, offsetX = 0) {
    const info = this.getMidiNoteInfo(midiNote);
    const blackKeyWidth = whiteKeyWidth * this.blackKeyWidthRatio;
    const blackKeyHeight = whiteKeyHeight * this.blackKeyHeightRatio;

    // Count how many white keys are between visibleStartNote and this note
    let whiteKeysBefore = 0;
    for (let n = this.visibleStartNote; n < midiNote; n++) {
      const nInfo = this.getMidiNoteInfo(n);
      if (!nInfo.isBlack) {
        whiteKeysBefore++;
      }
    }

    if (info.isBlack) {
      // Black key positioning - place it at the right edge of the preceding white key
      // whiteKeysBefore is the number of white keys to the left, so multiply by width
      // and subtract half the black key width to center it at the gap
      const x = offsetX + whiteKeysBefore * whiteKeyWidth - blackKeyWidth / 2;

      return {
        x,
        y: 0,
        width: blackKeyWidth,
        height: blackKeyHeight,
        isBlack: true
      };
    } else {
      // White key positioning - just use the count
      const x = offsetX + whiteKeysBefore * whiteKeyWidth;

      return {
        x,
        y: 0,
        width: whiteKeyWidth,
        height: whiteKeyHeight,
        isBlack: false
      };
    }
  }

  /**
   * Calculate visible range and offset based on pane width and height
   */
  calculateVisibleRange(width, height) {
    // Calculate white key width based on height to maintain aspect ratio
    const whiteKeyWidth = height / this.whiteKeyAspectRatio;

    // Calculate how many white keys can fit in the pane (ceiling to fill space)
    const whiteKeysFit = Math.ceil(width / whiteKeyWidth);

    // Keyboard-mapped range is C4 (60) to C5 (72), shifted by octave offset
    // This contains 8 white keys: C, D, E, F, G, A, B, C
    const keyboardCenter = 60 + (this.octaveOffset * 12); // C4 + octave shift
    const keyboardWhiteKeys = 8;

    if (whiteKeysFit <= keyboardWhiteKeys) {
      // Not enough space to show all keyboard keys, just center what we have
      this.visibleStartNote = keyboardCenter;
      this.visibleEndNote = keyboardCenter + 12; // One octave up
      const totalWhiteKeyWidth = keyboardWhiteKeys * whiteKeyWidth;
      const offsetX = (width - totalWhiteKeyWidth) / 2;
      return { offsetX, whiteKeyWidth };
    }

    // Calculate how many extra white keys we have space for
    const extraWhiteKeys = whiteKeysFit - keyboardWhiteKeys;
    const leftExtra = Math.floor(extraWhiteKeys / 2);
    const rightExtra = extraWhiteKeys - leftExtra;

    // Start from shifted keyboard center and go back leftExtra white keys
    let startNote = keyboardCenter;
    let leftCount = 0;
    while (leftCount < leftExtra && startNote > 0) {
      startNote--;
      const info = this.getMidiNoteInfo(startNote);
      if (!info.isBlack) {
        leftCount++;
      }
    }

    // Now count forward exactly whiteKeysFit white keys from startNote
    let endNote = startNote - 1; // Start one before so the first increment includes startNote
    let whiteKeyCount = 0;

    while (whiteKeyCount < whiteKeysFit && endNote < 127) {
      endNote++;
      const info = this.getMidiNoteInfo(endNote);
      if (!info.isBlack) {
        whiteKeyCount++;
      }
    }

    this.visibleStartNote = startNote;
    this.visibleEndNote = endNote;

    // No offset - keys start from left edge and fill to the right
    return { offsetX: 0, whiteKeyWidth };
  }

  /**
   * Find which MIDI note is at the given x, y position
   */
  findKeyAtPosition(x, y, height, whiteKeyWidth, offsetX) {
    // Check black keys first (they're on top)
    for (let note = this.visibleStartNote; note <= this.visibleEndNote; note++) {
      const info = this.getMidiNoteInfo(note);
      if (!info.isBlack) continue;

      const geom = this.getKeyGeometry(note, height, whiteKeyWidth, offsetX);
      if (x >= geom.x && x < geom.x + geom.width &&
          y >= geom.y && y < geom.y + geom.height) {
        return note;
      }
    }

    // Then check white keys
    for (let note = this.visibleStartNote; note <= this.visibleEndNote; note++) {
      const info = this.getMidiNoteInfo(note);
      if (info.isBlack) continue;

      const geom = this.getKeyGeometry(note, height, whiteKeyWidth, offsetX);
      if (x >= geom.x && x < geom.x + geom.width &&
          y >= geom.y && y < geom.y + geom.height) {
        return note;
      }
    }

    return null;
  }

  /**
   * Set which notes are currently playing (from MIDI playback)
   */
  setPlayingNotes(notes) {
    this.playingNotes = new Set(notes);
  }

  /**
   * Trigger a note on event
   */
  noteOn(midiNote, velocity = 100) {
    this.pressedKeys.add(midiNote);

    console.log(`Note ON: ${this.getMidiNoteInfo(midiNote).name} (${midiNote}) velocity: ${velocity}`);

    // Send to backend - use selected track or recording track
    let trackId = 0; // Default to first track
    if (typeof context !== 'undefined') {
      // If recording, use the recording track
      if (context.isRecording && context.recordingTrackId !== null) {
        trackId = context.recordingTrackId;
      }
      // Otherwise use the selected track
      else if (context.activeObject && context.activeObject.activeLayer && context.activeObject.activeLayer.audioTrackId !== null) {
        trackId = context.activeObject.activeLayer.audioTrackId;
      }
    }

    invoke('audio_send_midi_note_on', { trackId: trackId, note: midiNote, velocity }).catch(error => {
      console.error('Failed to send MIDI note on:', error);
    });

    // Request redraw to show the pressed key
    if (typeof context !== 'undefined' && context.pianoRedraw) {
      context.pianoRedraw();
    }
  }

  /**
   * Trigger a note off event
   */
  noteOff(midiNote) {
    this.pressedKeys.delete(midiNote);

    console.log(`Note OFF: ${this.getMidiNoteInfo(midiNote).name} (${midiNote})`);

    // Send to backend - use selected track or recording track
    let trackId = 0; // Default to first track
    if (typeof context !== 'undefined') {
      // If recording, use the recording track
      if (context.isRecording && context.recordingTrackId !== null) {
        trackId = context.recordingTrackId;
      }
      // Otherwise use the selected track
      else if (context.activeObject && context.activeObject.activeLayer && context.activeObject.activeLayer.audioTrackId !== null) {
        trackId = context.activeObject.activeLayer.audioTrackId;
      }
    }

    invoke('audio_send_midi_note_off', { trackId: trackId, note: midiNote }).catch(error => {
      console.error('Failed to send MIDI note off:', error);
    });

    // Request redraw to show the released key
    if (typeof context !== 'undefined' && context.pianoRedraw) {
      context.pianoRedraw();
    }
  }

  hitTest(x, y) {
    // Will be calculated in draw() based on pane width/height
    return true; // Accept all events, let findKeyAtPosition handle precision
  }

  mousedown(x, y, width, height) {
    const { offsetX, whiteKeyWidth } = this.calculateVisibleRange(width, height);
    const key = this.findKeyAtPosition(x, y, height, whiteKeyWidth, offsetX);
    if (key !== null) {
      this.noteOn(key, this.velocity);
    }
  }

  mousemove(x, y, width, height) {
    const { offsetX, whiteKeyWidth } = this.calculateVisibleRange(width, height);
    this.hoveredKey = this.findKeyAtPosition(x, y, height, whiteKeyWidth, offsetX);
  }

  mouseup(x, y, width, height) {
    // Release all pressed keys on mouse up
    for (const key of this.pressedKeys) {
      this.noteOff(key);
    }
  }

  draw(ctx, width, height) {
    ctx.save();

    // Background
    ctx.fillStyle = backgroundColor;
    ctx.fillRect(0, 0, width, height);

    // Calculate visible range and offset
    const { offsetX, whiteKeyWidth } = this.calculateVisibleRange(width, height);

    // Draw white keys first
    for (let note = this.visibleStartNote; note <= this.visibleEndNote; note++) {
      const info = this.getMidiNoteInfo(note);
      if (info.isBlack) continue;

      const geom = this.getKeyGeometry(note, height, whiteKeyWidth, offsetX);

      // Key color
      const isPressed = this.pressedKeys.has(note);
      const isPlaying = this.playingNotes.has(note);
      const isHovered = this.hoveredKey === note;

      if (isPressed) {
        ctx.fillStyle = highlight; // User pressed key
      } else if (isPlaying) {
        ctx.fillStyle = '#c8e6c9'; // Light green for MIDI playback
      } else if (isHovered) {
        ctx.fillStyle = '#f0f0f0';
      } else {
        ctx.fillStyle = '#ffffff';
      }

      // Draw white key with rounded corners at the bottom
      const radius = 3;
      ctx.beginPath();
      ctx.moveTo(geom.x, geom.y);
      ctx.lineTo(geom.x + geom.width, geom.y);
      ctx.lineTo(geom.x + geom.width, geom.y + geom.height - radius);
      ctx.arcTo(geom.x + geom.width, geom.y + geom.height, geom.x + geom.width - radius, geom.y + geom.height, radius);
      ctx.lineTo(geom.x + radius, geom.y + geom.height);
      ctx.arcTo(geom.x, geom.y + geom.height, geom.x, geom.y + geom.height - radius, radius);
      ctx.lineTo(geom.x, geom.y);
      ctx.closePath();
      ctx.fill();

      // Key border
      ctx.strokeStyle = shadow;
      ctx.lineWidth = 1;
      ctx.stroke();

      // Keyboard mapping label (if exists)
      // Subtract octave offset to get the base note for label lookup
      const baseNote = note - (this.octaveOffset * 12);
      const keyLabel = this.noteToKeyMap[baseNote];
      if (keyLabel) {
        ctx.fillStyle = isPressed ? '#000000' : '#333333';
        ctx.font = 'bold 16px sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(keyLabel, geom.x + geom.width / 2, geom.y + geom.height - 30);
      }

      // Note name at bottom of white keys
      if (info.semitone === 0) { // Only show octave number on C notes
        ctx.fillStyle = labelColor;
        ctx.font = '10px sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(info.name, geom.x + geom.width / 2, geom.y + geom.height - 5);
      }
    }

    // Draw black keys on top
    for (let note = this.visibleStartNote; note <= this.visibleEndNote; note++) {
      const info = this.getMidiNoteInfo(note);
      if (!info.isBlack) continue;

      const geom = this.getKeyGeometry(note, height, whiteKeyWidth, offsetX);

      // Key color
      const isPressed = this.pressedKeys.has(note);
      const isPlaying = this.playingNotes.has(note);
      const isHovered = this.hoveredKey === note;

      if (isPressed) {
        ctx.fillStyle = '#4a4a4a'; // User pressed black key
      } else if (isPlaying) {
        ctx.fillStyle = '#66bb6a'; // Darker green for MIDI playback on black keys
      } else if (isHovered) {
        ctx.fillStyle = '#2a2a2a';
      } else {
        ctx.fillStyle = '#000000';
      }

      // Draw black key with rounded corners at the bottom
      const blackRadius = 2;
      ctx.beginPath();
      ctx.moveTo(geom.x, geom.y);
      ctx.lineTo(geom.x + geom.width, geom.y);
      ctx.lineTo(geom.x + geom.width, geom.y + geom.height - blackRadius);
      ctx.arcTo(geom.x + geom.width, geom.y + geom.height, geom.x + geom.width - blackRadius, geom.y + geom.height, blackRadius);
      ctx.lineTo(geom.x + blackRadius, geom.y + geom.height);
      ctx.arcTo(geom.x, geom.y + geom.height, geom.x, geom.y + geom.height - blackRadius, blackRadius);
      ctx.lineTo(geom.x, geom.y);
      ctx.closePath();
      ctx.fill();

      // Highlight on top edge
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(geom.x, geom.y);
      ctx.lineTo(geom.x + geom.width, geom.y);
      ctx.stroke();

      // Keyboard mapping label (if exists)
      // Subtract octave offset to get the base note for label lookup
      const baseNote = note - (this.octaveOffset * 12);
      const keyLabel = this.noteToKeyMap[baseNote];
      if (keyLabel) {
        ctx.fillStyle = isPressed ? '#ffffff' : 'rgba(255, 255, 255, 0.7)';
        ctx.font = 'bold 14px sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(keyLabel, geom.x + geom.width / 2, geom.y + geom.height - 20);
      }
    }

    ctx.restore();
  }
}

/**
 * Piano Roll Editor
 * MIDI note editor with piano keyboard on left and grid on right
 */
class PianoRollEditor extends Widget {
  constructor(width, height, x, y) {
    super(x, y)
    this.width = width
    this.height = height

    // Display settings
    this.keyboardWidth = 60  // Width of piano keyboard on left
    this.noteHeight = 16  // Height of each note row
    this.pixelsPerSecond = 100  // Horizontal zoom
    this.minNote = 21  // A0
    this.maxNote = 108  // C8
    this.totalNotes = this.maxNote - this.minNote + 1

    // Scroll state
    this.scrollX = 0
    this.scrollY = 0
    this.initialScrollSet = false  // Track if we've set initial scroll position

    // Interaction state
    this.selectedNotes = new Set()  // Set of note indices
    this.selectedClipId = null  // Currently selected clip ID for editing
    this.dragMode = null  // null, 'move', 'resize', 'create', 'select'
    this.dragStartX = 0
    this.dragStartY = 0
    this.creatingNote = null  // Temporary note being created
    this.selectionRect = null  // Rectangle for multi-select {startX, startY, endX, endY}
    this.isDragging = false

    // Note preview playback state
    this.playingNote = null  // Currently playing note number
    this.playingNoteMaxDuration = null  // Max duration in seconds
    this.playingNoteStartTime = null  // Timestamp when note started playing

    // Auto-scroll state
    this.autoScrollEnabled = true  // Auto-scroll to follow playhead during playback
    this.lastPlayheadTime = 0  // Track last playhead position

    // Properties panel state
    this.propertyInputs = {}  // Will hold references to input elements

    // Start timer to check for note duration expiry
    this.checkNoteDurationTimer = setInterval(() => this.checkNoteDuration(), 50)
  }

  // Get the dimensions of the piano roll grid area (excluding keyboard)
  // Note: Properties panel is outside the canvas now, so we don't subtract it here
  getGridBounds() {
    return {
      left: this.keyboardWidth,
      top: 0,
      width: this.width - this.keyboardWidth,
      height: this.height
    }
  }

  checkNoteDuration() {
    if (this.playingNote !== null && this.playingNoteMaxDuration !== null && this.playingNoteStartTime !== null) {
      const elapsed = (Date.now() - this.playingNoteStartTime) / 1000
      if (elapsed >= this.playingNoteMaxDuration) {
        // Stop the note
        const clipData = this.getSelectedClip()
        if (clipData) {
          invoke('audio_send_midi_note_off', {
            trackId: clipData.trackId,
            note: this.playingNote
          })
          this.playingNote = null
          this.playingNoteMaxDuration = null
          this.playingNoteStartTime = null
        }
      }
    }
  }

  // Get all MIDI clips and the selected clip from the first MIDI track
  getMidiClipsData() {
    if (typeof context === 'undefined' || !context.activeObject || !context.activeObject.audioTracks) {
      return null
    }

    // Find the first MIDI track
    for (const track of context.activeObject.audioTracks) {
      if (track.type === 'midi' && track.clips && track.clips.length > 0) {
        // If no clip is selected, default to the first clip
        if (this.selectedClipId === null && track.clips.length > 0) {
          this.selectedClipId = track.clips[0].clipId
        }

        // Find the selected clip
        let selectedClip = track.clips.find(c => c.clipId === this.selectedClipId)

        // If selected clip not found (maybe deleted), select first clip
        if (!selectedClip && track.clips.length > 0) {
          selectedClip = track.clips[0]
          this.selectedClipId = selectedClip.clipId
        }

        return {
          allClips: track.clips,
          selectedClip: selectedClip,
          trackId: track.audioTrackId
        }
      }
    }
    return null
  }

  // Get the currently selected MIDI clip (for backward compatibility)
  getSelectedClip() {
    const data = this.getMidiClipsData()
    if (!data || !data.selectedClip) return null
    return { clip: data.selectedClip, trackId: data.trackId }
  }

  hitTest(x, y) {
    return x >= 0 && x <= this.width && y >= 0 && y <= this.height
  }

  // Convert screen coordinates to note/time
  screenToNote(y) {
    const gridY = y + this.scrollY
    const noteIndex = Math.floor(gridY / this.noteHeight)
    return this.maxNote - noteIndex  // Invert (higher notes at top)
  }

  screenToTime(x) {
    const gridX = x - this.keyboardWidth + this.scrollX
    return gridX / this.pixelsPerSecond
  }

  // Convert note/time to screen coordinates
  noteToScreenY(note) {
    const noteIndex = this.maxNote - note
    return noteIndex * this.noteHeight - this.scrollY
  }

  timeToScreenX(time) {
    return time * this.pixelsPerSecond - this.scrollX + this.keyboardWidth
  }

  // Find which clip contains the given time
  findClipAtTime(time) {
    const clipsData = this.getMidiClipsData()
    if (!clipsData || !clipsData.allClips) return null

    for (const clip of clipsData.allClips) {
      const clipStart = clip.startTime || 0
      const clipEnd = clipStart + (clip.duration || 0)
      if (time >= clipStart && time <= clipEnd) {
        return clip
      }
    }
    return null
  }

  // Find note at screen position (only searches selected clip)
  findNoteAtPosition(x, y) {
    const clipData = this.getSelectedClip()
    if (!clipData || !clipData.clip.notes) {
      return -1
    }

    const note = this.screenToNote(y)
    const time = this.screenToTime(x)
    const clipStartTime = clipData.clip.startTime || 0
    const clipLocalTime = time - clipStartTime

    // Search in reverse order so we find top-most notes first
    for (let i = clipData.clip.notes.length - 1; i >= 0; i--) {
      const n = clipData.clip.notes[i]
      const noteMatches = Math.round(n.note) === Math.round(note)
      const timeInRange = clipLocalTime >= n.start_time && clipLocalTime <= (n.start_time + n.duration)

      if (noteMatches && timeInRange) {
        return i
      }
    }

    return -1
  }

  // Check if clicking on the right edge resize handle
  isOnResizeHandle(x, noteIndex) {
    const clipData = this.getSelectedClip()
    if (!clipData || noteIndex < 0 || noteIndex >= clipData.clip.notes.length) {
      return false
    }

    const note = clipData.clip.notes[noteIndex]
    const clipStartTime = clipData.clip.startTime || 0
    const globalEndTime = clipStartTime + note.start_time + note.duration
    const noteEndX = this.timeToScreenX(globalEndTime)

    // Consider clicking within 8 pixels of the right edge as resize
    return Math.abs(x - noteEndX) < 8
  }

  mousedown(x, y) {
    this._globalEvents.add("mousemove")
    this._globalEvents.add("mouseup")

    this.isDragging = true
    this.dragStartX = x
    this.dragStartY = y

    // Check if clicking on keyboard or grid
    if (x < this.keyboardWidth) {
      // Clicking on keyboard - could preview note
      return
    }

    const note = this.screenToNote(y)
    const time = this.screenToTime(x)

    // Check if clicking on a different clip and switch to it
    const clickedClip = this.findClipAtTime(time)
    if (clickedClip && clickedClip.clipId !== this.selectedClipId) {
      this.selectedClipId = clickedClip.clipId
      this.selectedNotes.clear()
      // Redraw to show the new selection
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw()
      }
      // Don't start dragging/editing on the same click that switches clips
      this.isDragging = false
      this._globalEvents.delete("mousemove")
      this._globalEvents.delete("mouseup")
      return
    }

    // Check if clicking on an existing note
    const noteIndex = this.findNoteAtPosition(x, y)

    if (noteIndex >= 0) {
      // Clicking on an existing note
      const clipData = this.getSelectedClip()
      if (this.isOnResizeHandle(x, noteIndex)) {
        // Start resizing
        this.dragMode = 'resize'
        this.resizingNoteIndex = noteIndex
        this.selectedNotes.clear()
        this.selectedNotes.add(noteIndex)
      } else {
        // Start moving
        this.dragMode = 'move'
        this.movingStartTime = time
        this.movingStartNote = note

        // Select this note (or add to selection with Ctrl/Cmd)
        if (!this.selectedNotes.has(noteIndex)) {
          this.selectedNotes.clear()
          this.selectedNotes.add(noteIndex)
        }

        // Play preview of the note
        if (clipData && clipData.clip.notes[noteIndex]) {
          const clickedNote = clipData.clip.notes[noteIndex]
          this.playingNote = clickedNote.note
          this.playingNoteMaxDuration = clickedNote.duration
          this.playingNoteStartTime = Date.now()

          invoke('audio_send_midi_note_on', {
            trackId: clipData.trackId,
            note: clickedNote.note,
            velocity: clickedNote.velocity
          })
        }
      }
    } else {
      // Clicking on empty space
      const isShiftHeld = this.lastClickEvent?.shiftKey || false

      if (isShiftHeld) {
        // Shift+click: Start creating a new note
        this.dragMode = 'create'
        this.selectedNotes.clear()

        // Create a temporary note for preview (store in clip-local time)
        const clipData = this.getSelectedClip()
        const clipStartTime = clipData?.clip?.startTime || 0
        const clipLocalTime = time - clipStartTime

        const newNoteValue = Math.round(note)
        this.creatingNote = {
          note: newNoteValue,
          start_time: clipLocalTime,
          duration: 0.1, // Minimum duration
          velocity: 100
        }

        // Play preview of the new note
        if (clipData) {
          this.playingNote = newNoteValue
          this.playingNoteMaxDuration = null // No max duration for creating notes
          this.playingNoteStartTime = Date.now()

          invoke('audio_send_midi_note_on', {
            trackId: clipData.trackId,
            note: newNoteValue,
            velocity: 100
          })
        }
      } else {
        // Regular click: Start selection rectangle
        this.dragMode = 'select'
        this.selectedNotes.clear()
        this.selectionRect = {
          startX: x,
          startY: y,
          endX: x,
          endY: y
        }
      }
    }
  }

  mousemove(x, y) {
    // Update cursor based on hover position even when not dragging
    if (!this.isDragging && x >= this.keyboardWidth) {
      const noteIndex = this.findNoteAtPosition(x, y)
      if (noteIndex >= 0 && this.isOnResizeHandle(x, noteIndex)) {
        this.cursor = 'ew-resize'
      } else {
        this.cursor = 'default'
      }
    }

    if (!this.isDragging) return

    const clipData = this.getSelectedClip()
    if (!clipData) return

    if (this.dragMode === 'create') {
      // Extend the note being created
      if (this.creatingNote) {
        const currentTime = this.screenToTime(x)
        const clipStartTime = clipData.clip.startTime || 0
        const clipLocalTime = currentTime - clipStartTime
        const duration = Math.max(0.1, clipLocalTime - this.creatingNote.start_time)
        this.creatingNote.duration = duration
      }
    } else if (this.dragMode === 'move') {
      // Move selected notes
      const currentTime = this.screenToTime(x)
      const currentNote = this.screenToNote(y)

      const deltaTime = currentTime - this.movingStartTime
      const deltaNote = Math.round(currentNote - this.movingStartNote)

      // Check if pitch changed
      if (deltaNote !== 0) {
        const firstSelectedIndex = Array.from(this.selectedNotes)[0]
        if (firstSelectedIndex >= 0 && firstSelectedIndex < clipData.clip.notes.length) {
          const movedNote = clipData.clip.notes[firstSelectedIndex]
          const newPitch = Math.max(0, Math.min(127, movedNote.note + deltaNote))

          // Stop old note if one is playing
          if (this.playingNote !== null) {
            invoke('audio_send_midi_note_off', {
              trackId: clipData.trackId,
              note: this.playingNote
            })
          }

          // Update playing note to new pitch
          this.playingNote = newPitch
          this.playingNoteMaxDuration = movedNote.duration
          this.playingNoteStartTime = Date.now()

          // Play new note at new pitch
          invoke('audio_send_midi_note_on', {
            trackId: clipData.trackId,
            note: newPitch,
            velocity: movedNote.velocity
          })
        }
      }

      // Update positions of all selected notes
      for (const noteIndex of this.selectedNotes) {
        if (noteIndex >= 0 && noteIndex < clipData.clip.notes.length) {
          const note = clipData.clip.notes[noteIndex]
          note.start_time = Math.max(0, note.start_time + deltaTime)
          note.note = Math.max(0, Math.min(127, note.note + deltaNote))
        }
      }

      // Update drag start positions for next move
      this.movingStartTime = currentTime
      this.movingStartNote = currentNote

      // Trigger timeline redraw to show updated notes
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw()
      }
    } else if (this.dragMode === 'resize') {
      // Resize the selected note
      if (this.resizingNoteIndex >= 0 && this.resizingNoteIndex < clipData.clip.notes.length) {
        const note = clipData.clip.notes[this.resizingNoteIndex]
        const currentTime = this.screenToTime(x)
        const clipStartTime = clipData.clip.startTime || 0
        const clipLocalTime = currentTime - clipStartTime
        const newDuration = Math.max(0.1, clipLocalTime - note.start_time)
        note.duration = newDuration

        // Trigger timeline redraw to show updated notes
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw()
        }
      }
    } else if (this.dragMode === 'select') {
      // Update selection rectangle
      if (this.selectionRect) {
        this.selectionRect.endX = x
        this.selectionRect.endY = y

        // Update selected notes based on rectangle
        this.updateSelectionFromRect(clipData)
      }
    }
  }

  mouseup(x, y) {
    this._globalEvents.delete("mousemove")
    this._globalEvents.delete("mouseup")

    const clipData = this.getSelectedClip()

    // Check if this was a simple click (not a drag) on empty space
    if (this.dragMode === 'select' && this.dragStartX !== undefined && this.dragStartY !== undefined) {
      const dragDistance = Math.sqrt(
        Math.pow(x - this.dragStartX, 2) + Math.pow(y - this.dragStartY, 2)
      )

      // If drag distance is minimal (< 5 pixels), treat it as a click to reposition playhead
      if (dragDistance < 5) {
        const time = this.screenToTime(x)

        // Set playhead position
        if (context.activeObject) {
          context.activeObject.currentTime = time

          // Request redraws to show the new playhead position
          if (context.timelineWidget) {
            context.timelineWidget.requestRedraw()
          }
          if (context.pianoRollRedraw) {
            context.pianoRollRedraw()
          }
        }
      }
    }

    // Stop playing note
    if (this.playingNote !== null && clipData) {
      invoke('audio_send_midi_note_off', {
        trackId: clipData.trackId,
        note: this.playingNote
      })
      this.playingNote = null
      this.playingNoteMaxDuration = null
      this.playingNoteStartTime = null
    }

    // If we were creating a note, add it to the clip
    if (this.dragMode === 'create' && this.creatingNote && clipData) {
      if (!clipData.clip.notes) {
        clipData.clip.notes = []
      }

      // Binary search to find insertion position to maintain sorted order
      const newNote = { ...this.creatingNote }
      let left = 0
      let right = clipData.clip.notes.length
      while (left < right) {
        const mid = Math.floor((left + right) / 2)
        if (clipData.clip.notes[mid].start_time < newNote.start_time) {
          left = mid + 1
        } else {
          right = mid
        }
      }
      clipData.clip.notes.splice(left, 0, newNote)

      // Trigger timeline redraw to show new note
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw()
      }

      // Sync to backend
      this.syncNotesToBackend(clipData)
    }

    // If we moved or resized notes, sync to backend
    if ((this.dragMode === 'move' || this.dragMode === 'resize') && clipData) {
      if (context.timelineWidget) {
        context.timelineWidget.requestRedraw()
      }

      // Sync to backend
      this.syncNotesToBackend(clipData)
    }

    this.isDragging = false
    this.dragMode = null
    this.creatingNote = null
    this.selectionRect = null
    this.resizingNoteIndex = -1
  }

  wheel(e) {
    // Support horizontal scrolling from trackpad (deltaX) or Shift+scroll (deltaY)
    if (e.deltaX !== 0) {
      // Trackpad horizontal scroll
      this.scrollX += e.deltaX
    } else if (e.shiftKey) {
      // Shift+wheel for horizontal scroll
      this.scrollX += e.deltaY
    } else {
      // Normal vertical scroll
      this.scrollY += e.deltaY
    }

    this.scrollX = Math.max(0, this.scrollX)
    this.scrollY = Math.max(0, this.scrollY)

    // Disable auto-scroll when user manually scrolls
    this.autoScrollEnabled = false
  }

  keydown(e) {
    // Handle delete/backspace to delete selected notes
    if (e.key === 'Delete' || e.key === 'Backspace') {
      if (this.selectedNotes.size > 0) {
        const clipData = this.getSelectedClip()
        if (clipData && clipData.clip && clipData.clip.notes) {
          // Convert set to sorted array in reverse order to avoid index shifting
          const indicesToDelete = Array.from(this.selectedNotes).sort((a, b) => b - a)

          for (const index of indicesToDelete) {
            if (index >= 0 && index < clipData.clip.notes.length) {
              clipData.clip.notes.splice(index, 1)
            }
          }

          // Clear selection
          this.selectedNotes.clear()

          // Sync to backend
          this.syncNotesToBackend(clipData)

          // Trigger redraws
          if (context.timelineWidget) {
            context.timelineWidget.requestRedraw()
          }
          if (context.pianoRollRedraw) {
            context.pianoRollRedraw()
          }
        }
        e.preventDefault()
      }
    }
  }

  updateSelectionFromRect(clipData) {
    if (!clipData || !clipData.clip || !clipData.clip.notes || !this.selectionRect) {
      return
    }

    const clipStartTime = clipData.clip.startTime || 0
    this.selectedNotes.clear()

    // Get rectangle bounds
    const minX = Math.min(this.selectionRect.startX, this.selectionRect.endX)
    const maxX = Math.max(this.selectionRect.startX, this.selectionRect.endX)
    const minY = Math.min(this.selectionRect.startY, this.selectionRect.endY)
    const maxY = Math.max(this.selectionRect.startY, this.selectionRect.endY)

    // Convert to time/note coordinates
    const minTime = this.screenToTime(minX)
    const maxTime = this.screenToTime(maxX)
    const minNote = this.screenToNote(maxY)  // Note: Y is inverted
    const maxNote = this.screenToNote(minY)

    // Check each note
    for (let i = 0; i < clipData.clip.notes.length; i++) {
      const note = clipData.clip.notes[i]
      const noteGlobalStart = clipStartTime + note.start_time
      const noteGlobalEnd = noteGlobalStart + note.duration

      // Check if note overlaps with selection rectangle
      const timeOverlaps = noteGlobalEnd >= minTime && noteGlobalStart <= maxTime
      const noteOverlaps = note.note >= minNote && note.note <= maxNote

      if (timeOverlaps && noteOverlaps) {
        this.selectedNotes.add(i)
      }
    }
  }

  syncNotesToBackend(clipData) {
    // Convert notes to backend format: (start_time, note, velocity, duration)
    const notes = clipData.clip.notes.map(n => [
      n.start_time,
      n.note,
      n.velocity,
      n.duration
    ])

    // Send to backend
    invoke('audio_update_midi_clip_notes', {
      trackId: clipData.trackId,
      clipId: clipData.clip.clipId,
      notes: notes
    }).catch(err => {
      console.error('Failed to update MIDI notes:', err)
    })
  }

  draw(ctx) {
    // Update dimensions
    // (width/height will be set by parent container)

    // Set initial scroll position to center on G4 (MIDI note 67) on first draw
    if (!this.initialScrollSet && this.height > 0) {
      const g4Index = this.maxNote - 67  // G4 is MIDI note 67
      const g4Y = g4Index * this.noteHeight
      // Center G4 in the viewport
      this.scrollY = g4Y - (this.height / 2)
      this.initialScrollSet = true
    }

    // Auto-scroll to follow playhead during playback
    if (this.autoScrollEnabled && context.activeObject && this.width > 0) {
      const playheadTime = context.activeObject.currentTime || 0

      // Check if playhead is moving forward (playing)
      if (playheadTime > this.lastPlayheadTime) {
        // Center playhead in viewport
        const gridWidth = this.width - this.keyboardWidth
        const playheadScreenX = playheadTime * this.pixelsPerSecond
        const targetScrollX = playheadScreenX - (gridWidth / 2)

        this.scrollX = Math.max(0, targetScrollX)
      }

      this.lastPlayheadTime = playheadTime
    }

    // Clear
    ctx.fillStyle = backgroundColor
    ctx.fillRect(0, 0, this.width, this.height)

    // Draw piano keyboard
    this.drawKeyboard(ctx, this.width, this.height)

    // Draw grid
    this.drawGrid(ctx, this.width, this.height)

    // Draw clip boundaries
    const clipsData = this.getMidiClipsData()
    if (clipsData && clipsData.allClips) {
      this.drawClipBoundaries(ctx, this.width, this.height, clipsData.allClips)
    }

    // Draw notes for all clips in the track
    if (clipsData && clipsData.allClips) {
      // Draw non-selected clips first (at lower opacity)
      for (const clip of clipsData.allClips) {
        if (clip.clipId !== this.selectedClipId && clip.notes) {
          this.drawNotes(ctx, this.width, this.height, clip, 0.3)
        }
      }

      // Draw selected clip on top (at full opacity)
      if (clipsData.selectedClip && clipsData.selectedClip.notes) {
        this.drawNotes(ctx, this.width, this.height, clipsData.selectedClip, 1.0)
      }
    }

    // Draw playhead
    this.drawPlayhead(ctx, this.width, this.height)

    // Draw selection rectangle
    if (this.selectionRect) {
      this.drawSelectionRect(ctx, this.width, this.height)
    }

    // Update HTML properties panel
    this.updatePropertiesPanel()
  }

  drawSelectionRect(ctx, width, height) {
    if (!this.selectionRect) return

    const gridLeft = this.keyboardWidth
    const minX = Math.max(gridLeft, Math.min(this.selectionRect.startX, this.selectionRect.endX))
    const maxX = Math.min(width, Math.max(this.selectionRect.startX, this.selectionRect.endX))
    const minY = Math.max(0, Math.min(this.selectionRect.startY, this.selectionRect.endY))
    const maxY = Math.min(height, Math.max(this.selectionRect.startY, this.selectionRect.endY))

    ctx.save()

    // Draw filled rectangle with transparency
    ctx.fillStyle = 'rgba(100, 150, 255, 0.2)'
    ctx.fillRect(minX, minY, maxX - minX, maxY - minY)

    // Draw border
    ctx.strokeStyle = 'rgba(100, 150, 255, 0.6)'
    ctx.lineWidth = 1
    ctx.strokeRect(minX, minY, maxX - minX, maxY - minY)

    ctx.restore()
  }

  updatePropertiesPanel() {
    // Update the HTML properties panel with current selection
    if (!this.propertiesPanel) return

    const clipData = this.getSelectedClip()
    const properties = this.getSelectedNoteProperties(clipData)

    // Update pitch (display-only)
    this.propertiesPanel.pitch.textContent = properties.pitch || '-'

    // Update velocity
    if (properties.velocity !== null) {
      this.propertiesPanel.velocity.input.value = properties.velocity
      this.propertiesPanel.velocity.slider.value = properties.velocity
    } else {
      this.propertiesPanel.velocity.input.value = ''
      this.propertiesPanel.velocity.slider.value = 64 // Default middle value
    }

    // Update modulation
    if (properties.modulation !== null) {
      this.propertiesPanel.modulation.input.value = properties.modulation
      this.propertiesPanel.modulation.slider.value = properties.modulation
    } else {
      this.propertiesPanel.modulation.input.value = ''
      this.propertiesPanel.modulation.slider.value = 0
    }
  }

  getSelectedNoteProperties(clipData) {
    if (!clipData || !clipData.clip || !clipData.clip.notes || this.selectedNotes.size === 0) {
      return { pitch: null, velocity: null, modulation: null }
    }

    const selectedIndices = Array.from(this.selectedNotes)
    const notes = selectedIndices.map(i => clipData.clip.notes[i]).filter(n => n)

    if (notes.length === 0) {
      return { pitch: null, velocity: null, modulation: null }
    }

    // Check if all selected notes have the same values
    const firstNote = notes[0]
    const allSamePitch = notes.every(n => n.note === firstNote.note)
    const allSameVelocity = notes.every(n => n.velocity === firstNote.velocity)
    const allSameModulation = notes.every(n => (n.modulation || 0) === (firstNote.modulation || 0))

    // Convert MIDI note number to name
    const noteName = allSamePitch ? this.midiNoteToName(firstNote.note) : null

    return {
      pitch: noteName,
      velocity: allSameVelocity ? firstNote.velocity : null,
      modulation: allSameModulation ? (firstNote.modulation || 0) : null
    }
  }

  midiNoteToName(midiNote) {
    const noteNames = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
    const octave = Math.floor(midiNote / 12) - 1
    const noteName = noteNames[midiNote % 12]
    return `${noteName}${octave} (${midiNote})`
  }

  drawKeyboard(ctx, width, height) {
    const keyboardWidth = this.keyboardWidth

    // Draw keyboard background
    ctx.fillStyle = shade
    ctx.fillRect(0, 0, keyboardWidth, height)

    // Draw keys
    for (let note = this.minNote; note <= this.maxNote; note++) {
      const y = this.noteToScreenY(note)

      if (y < 0 || y > height) continue

      const isBlackKey = [1, 3, 6, 8, 10].includes(note % 12)

      ctx.fillStyle = isBlackKey ? '#333' : '#fff'
      ctx.fillRect(5, y, keyboardWidth - 10, this.noteHeight - 1)

      // Draw note label for C notes
      if (note % 12 === 0) {
        ctx.fillStyle = '#999'
        ctx.font = '10px sans-serif'
        ctx.textAlign = 'right'
        ctx.textBaseline = 'middle'
        ctx.fillText(`C${Math.floor(note / 12) - 1}`, keyboardWidth - 15, y + this.noteHeight / 2)
      }
    }
  }

  drawGrid(ctx, width, height) {
    const gridBounds = this.getGridBounds()
    const gridLeft = gridBounds.left
    const gridWidth = gridBounds.width
    const gridHeight = gridBounds.height

    ctx.save()
    ctx.beginPath()
    ctx.rect(gridLeft, 0, gridWidth, gridHeight)
    ctx.clip()

    // Draw background
    ctx.fillStyle = backgroundColor
    ctx.fillRect(gridLeft, 0, gridWidth, gridHeight)

    // Draw horizontal lines (note separators)
    ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)'
    ctx.lineWidth = 1

    for (let note = this.minNote; note <= this.maxNote; note++) {
      const y = this.noteToScreenY(note)

      if (y < 0 || y > height) continue

      // Highlight C notes
      if (note % 12 === 0) {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.3)'
      } else {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)'
      }

      ctx.beginPath()
      ctx.moveTo(gridLeft, y)
      ctx.lineTo(width, y)
      ctx.stroke()
    }

    // Draw vertical lines (time grid)
    const beatInterval = 0.5  // Half second intervals
    const startTime = Math.floor(this.scrollX / this.pixelsPerSecond / beatInterval) * beatInterval
    const endTime = (this.scrollX + gridWidth) / this.pixelsPerSecond

    for (let time = startTime; time <= endTime; time += beatInterval) {
      const x = this.timeToScreenX(time)

      if (x < gridLeft || x > width) continue

      // Every second is brighter
      if (Math.abs(time % 1.0) < 0.01) {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.3)'
      } else {
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)'
      }

      ctx.beginPath()
      ctx.moveTo(x, 0)
      ctx.lineTo(x, height)
      ctx.stroke()
    }

    ctx.restore()
  }

  drawClipBoundaries(ctx, width, height, clips) {
    const gridLeft = this.keyboardWidth

    ctx.save()
    ctx.beginPath()
    ctx.rect(gridLeft, 0, width - gridLeft, height)
    ctx.clip()

    // Draw background highlight for selected clip
    const selectedClip = clips.find(c => c.clipId === this.selectedClipId)
    if (selectedClip) {
      const clipStart = selectedClip.startTime || 0
      const clipEnd = clipStart + (selectedClip.duration || 0)
      const startX = Math.max(gridLeft, this.timeToScreenX(clipStart))
      const endX = Math.min(width, this.timeToScreenX(clipEnd))

      if (endX > startX) {
        ctx.fillStyle = 'rgba(111, 220, 111, 0.05)'  // Very subtle green tint
        ctx.fillRect(startX, 0, endX - startX, height)
      }
    }

    // Draw start and end lines for each clip
    for (const clip of clips) {
      const clipStart = clip.startTime || 0
      const clipEnd = clipStart + (clip.duration || 0)
      const isSelected = clip.clipId === this.selectedClipId

      // Use brighter green for selected clip, dimmer for others
      const color = isSelected ? 'rgba(111, 220, 111, 0.5)' : 'rgba(111, 220, 111, 0.2)'
      const lineWidth = isSelected ? 2 : 1

      ctx.strokeStyle = color
      ctx.lineWidth = lineWidth

      // Draw clip start line
      const startX = this.timeToScreenX(clipStart)
      if (startX >= gridLeft && startX <= width) {
        ctx.beginPath()
        ctx.moveTo(startX, 0)
        ctx.lineTo(startX, height)
        ctx.stroke()
      }

      // Draw clip end line
      const endX = this.timeToScreenX(clipEnd)
      if (endX >= gridLeft && endX <= width) {
        ctx.beginPath()
        ctx.moveTo(endX, 0)
        ctx.lineTo(endX, height)
        ctx.stroke()
      }
    }

    ctx.restore()
  }

  drawNotes(ctx, width, height, clip, opacity = 1.0) {
    const gridLeft = this.keyboardWidth
    const clipStartTime = clip.startTime || 0
    const isSelectedClip = clip.clipId === this.selectedClipId

    ctx.save()
    ctx.globalAlpha = opacity
    ctx.beginPath()
    ctx.rect(gridLeft, 0, width - gridLeft, height)
    ctx.clip()

    // Draw existing notes at their global timeline position
    for (let i = 0; i < clip.notes.length; i++) {
      const note = clip.notes[i]

      // Convert note time to global timeline time
      const globalTime = clipStartTime + note.start_time
      const x = this.timeToScreenX(globalTime)
      const y = this.noteToScreenY(note.note)
      const noteWidth = note.duration * this.pixelsPerSecond
      const noteHeight = this.noteHeight - 2

      // Skip if off-screen
      if (x + noteWidth < gridLeft || x > width || y + noteHeight < 0 || y > height) {
        continue
      }

      // Calculate brightness based on velocity (1-127)
      // Map velocity to brightness range: 0.35 (min) to 1.0 (max)
      const velocity = note.velocity || 100
      const brightness = 0.35 + (velocity / 127) * 0.65

      // Highlight selected notes (only for selected clip)
      if (isSelectedClip && this.selectedNotes.has(i)) {
        // Selected note: brighter green with velocity-based brightness
        const r = Math.round(143 * brightness)
        const g = Math.round(252 * brightness)
        const b = Math.round(143 * brightness)
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`
      } else {
        // Normal note: velocity-based brightness
        const r = Math.round(111 * brightness)
        const g = Math.round(220 * brightness)
        const b = Math.round(111 * brightness)
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`
      }

      ctx.fillRect(x, y, noteWidth, noteHeight)

      // Draw border
      ctx.strokeStyle = 'rgba(0, 0, 0, 0.3)'
      ctx.strokeRect(x, y, noteWidth, noteHeight)
    }

    // Draw note being created (only for selected clip)
    if (this.creatingNote && isSelectedClip) {
      // Note being created is in clip-local time, convert to global
      const globalTime = clipStartTime + this.creatingNote.start_time
      const x = this.timeToScreenX(globalTime)
      const y = this.noteToScreenY(this.creatingNote.note)
      const noteWidth = this.creatingNote.duration * this.pixelsPerSecond
      const noteHeight = this.noteHeight - 2

      // Draw with a slightly transparent color to indicate it's being created
      ctx.fillStyle = 'rgba(111, 220, 111, 0.7)'
      ctx.fillRect(x, y, noteWidth, noteHeight)

      ctx.strokeStyle = 'rgba(0, 0, 0, 0.5)'
      ctx.setLineDash([4, 4])
      ctx.strokeRect(x, y, noteWidth, noteHeight)
      ctx.setLineDash([])
    }

    ctx.restore()
  }

  drawPlayhead(ctx, width, height) {
    // Get current playhead time from context
    if (typeof context === 'undefined' || !context.activeObject) {
      return
    }

    const playheadTime = context.activeObject.currentTime || 0
    const gridLeft = this.keyboardWidth

    // Convert time to screen X position
    const playheadX = this.timeToScreenX(playheadTime)

    // Only draw if playhead is visible
    if (playheadX < gridLeft || playheadX > width) {
      return
    }

    ctx.save()
    ctx.beginPath()
    ctx.rect(gridLeft, 0, width - gridLeft, height)
    ctx.clip()

    // Draw playhead line
    ctx.strokeStyle = 'rgba(255, 100, 100, 0.8)'
    ctx.lineWidth = 2
    ctx.beginPath()
    ctx.moveTo(playheadX, 0)
    ctx.lineTo(playheadX, height)
    ctx.stroke()

    ctx.restore()
  }
}

export {
  SCROLL,
  Widget,
  HueSelectionBar,
  SaturationValueSelectionGradient,
  AlphaSelectionBar,
  ColorWidget,
  ColorSelectorWidget,
  HBox, VBox,
  ScrollableWindow,
  ScrollableWindowHeaders,
  TimelineWindow,
  TimelineWindowV2,
  VirtualPiano,
  PianoRollEditor
};
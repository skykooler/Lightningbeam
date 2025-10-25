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
    this.timelineState = new TimelineState(context.config?.framerate || 24)

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

    // Draw snapping checkbox in ruler header area (Phase 5)
    this.drawSnappingCheckbox(ctx)

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
   * Draw snapping checkbox in ruler header area (Phase 5)
   */
  drawSnappingCheckbox(ctx) {
    const checkboxSize = 14
    const checkboxX = 10
    const checkboxY = (this.ruler.height - checkboxSize) / 2

    // Draw checkbox border
    ctx.strokeStyle = foregroundColor
    ctx.lineWidth = 1
    ctx.strokeRect(checkboxX, checkboxY, checkboxSize, checkboxSize)

    // Fill if snapping is enabled
    if (this.timelineState.snapToFrames) {
      ctx.fillStyle = foregroundColor
      ctx.fillRect(checkboxX + 2, checkboxY + 2, checkboxSize - 4, checkboxSize - 4)
    }

    // Draw label
    ctx.fillStyle = labelColor
    ctx.font = '11px sans-serif'
    ctx.textAlign = 'left'
    ctx.textBaseline = 'middle'
    ctx.fillText('Snap', checkboxX + checkboxSize + 6, this.ruler.height / 2)
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

      // Draw toggle buttons for object/shape/audio tracks (Phase 3)
      if (track.type === 'object' || track.type === 'shape' || track.type === 'audio') {
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

        // Segment visibility button (only for object/shape tracks, not audio)
        if (track.type !== 'audio') {
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

              // Draw parameter name (extract last part after last dot)
              ctx.fillStyle = isHidden ? foregroundColor : labelColor
              const paramName = curve.parameter.split('.').pop()
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

      // Draw track background (same color for all tracks)
      ctx.fillStyle = shade
      ctx.fillRect(0, y, trackAreaWidth, trackHeight)

      // Draw interval markings
      const visibleStartTime = this.timelineState.viewportStartTime
      const visibleEndTime = visibleStartTime + (trackAreaWidth / this.timelineState.pixelsPerSecond)

      if (this.timelineState.timeFormat === 'frames') {
        // Frames mode: mark every frame edge, with every 5th frame shaded
        const frameDuration = 1 / this.timelineState.framerate
        const startFrame = Math.floor(visibleStartTime / frameDuration)
        const endFrame = Math.ceil(visibleEndTime / frameDuration)

        for (let frame = startFrame; frame <= endFrame; frame++) {
          const time = frame * frameDuration
          const x = this.timelineState.timeToPixel(time)
          const nextX = this.timelineState.timeToPixel((frame + 1) * frameDuration)

          if (x >= 0 && x <= trackAreaWidth) {
            if (frame % 5 === 0) {
              // Every 5th frame: shade the entire frame width
              ctx.fillStyle = shadow
              ctx.fillRect(x, y, nextX - x, trackHeight)
            } else {
              // Regular frame: draw edge line
              ctx.strokeStyle = shadow
              ctx.lineWidth = 1
              ctx.beginPath()
              ctx.moveTo(x, y)
              ctx.lineTo(x, y + trackHeight)
              ctx.stroke()
            }
          }
        }
      } else {
        // Seconds mode: mark every second edge
        const startSecond = Math.floor(visibleStartTime)
        const endSecond = Math.ceil(visibleEndTime)

        ctx.strokeStyle = shadow
        ctx.lineWidth = 1

        for (let second = startSecond; second <= endSecond; second++) {
          const x = this.timelineState.timeToPixel(second)

          if (x >= 0 && x <= trackAreaWidth) {
            ctx.beginPath()
            ctx.moveTo(x, y)
            ctx.lineTo(x, y + trackHeight)
            ctx.stroke()
          }
        }
      }

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

              // Calculate visible time range within the clip
              const clipEndX = startX + clipWidth
              const visibleStartTime = this.timelineState.pixelToTime(Math.max(startX, 0)) - clip.startTime
              const visibleEndTime = this.timelineState.pixelToTime(Math.min(clipEndX, this.width)) - clip.startTime

              // Binary search to find first visible note
              let firstVisibleIdx = 0
              let left = 0
              let right = clip.notes.length - 1
              while (left <= right) {
                const mid = Math.floor((left + right) / 2)
                const noteEndTime = clip.notes[mid].start_time + clip.notes[mid].duration

                if (noteEndTime < visibleStartTime) {
                  left = mid + 1
                  firstVisibleIdx = left
                } else {
                  right = mid - 1
                }
              }

              // Draw visible notes only
              ctx.fillStyle = '#6fdc6f'  // Bright green for note bars

              for (let i = firstVisibleIdx; i < clip.notes.length; i++) {
                const note = clip.notes[i]

                // Exit early if note starts after visible range
                if (note.start_time > visibleEndTime) {
                  break
                }

                // Calculate note position (pitch mod 12 for chromatic representation)
                const pitchClass = note.note % 12
                // Invert Y so higher pitches appear at top
                const noteY = y + 5 + ((11 - pitchClass) * noteHeight)

                // Calculate note timing on timeline
                const noteStartX = this.timelineState.timeToPixel(clip.startTime + note.start_time)
                const noteEndX = this.timelineState.timeToPixel(clip.startTime + note.start_time + note.duration)

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

              // Calculate how many pixels each waveform peak represents
              const pixelsPerPeak = clipWidth / waveformData.length

              // Calculate the range of visible peaks
              const firstVisiblePeak = Math.max(0, Math.floor((visibleStart - startX) / pixelsPerPeak))
              const lastVisiblePeak = Math.min(waveformData.length - 1, Math.ceil((visibleEnd - startX) / pixelsPerPeak))

              // Draw waveform as a filled path
              ctx.beginPath()

              // Trace along the max values (left to right)
              for (let i = firstVisiblePeak; i <= lastVisiblePeak; i++) {
                const peakX = startX + (i * pixelsPerPeak)
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
                const peakX = startX + (i * pixelsPerPeak)
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

      // Only draw curves for objects, shapes, and audio tracks
      if (track.type !== 'object' && track.type !== 'shape' && track.type !== 'audio') continue

      const obj = track.object

      // Skip if curves are hidden
      if (obj.curvesMode === 'segment') continue

      const y = this.trackHierarchy.getTrackY(i)

      // Find the layer containing this object/shape to get AnimationData
      let animationData = null
      if (track.type === 'audio') {
        // For audio tracks, animation data is directly on the track object
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

        // Filter to only curves for this specific object/shape/audio
        if (track.type === 'audio') {
          // Audio tracks: include all curves (they're prefixed with 'track.' or 'clip.')
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
    // Check if clicking on snapping checkbox (Phase 5)
    if (y <= this.ruler.height && x < this.trackHeaderWidth) {
      const checkboxSize = 14
      const checkboxX = 10
      const checkboxY = (this.ruler.height - checkboxSize) / 2

      if (x >= checkboxX && x <= checkboxX + checkboxSize &&
          y >= checkboxY && y <= checkboxY + checkboxSize) {
        // Toggle snapping
        this.timelineState.snapToFrames = !this.timelineState.snapToFrames
        console.log('Snapping', this.timelineState.snapToFrames ? 'enabled' : 'disabled')
        if (this.requestRedraw) this.requestRedraw()
        return true
      }
    }

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
        if (track.type === 'object' || track.type === 'shape') {
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

        // Check if clicking on audio clip to start dragging
        const audioClipInfo = this.getAudioClipAtPoint(track, adjustedX, adjustedY)
        if (audioClipInfo) {
          // Select the track
          this.selectTrack(track)

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
          audioTrack: audioTrack
        }
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

      // Clean up dragging state
      this.draggingAudioClip = null
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
    // Check if right-clicking in timeline area with curves
    const trackY = y - this.ruler.height
    if (trackY >= 0 && x >= this.trackHeaderWidth) {
      const adjustedY = trackY - this.trackScrollOffset
      const adjustedX = x - this.trackHeaderWidth
      const track = this.trackHierarchy.getTrackAtY(adjustedY)

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
    } else {
      this.timelineState.timeFormat = 'frames'
    }
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

    // MIDI note mapping (white keys in an octave: C, D, E, F, G, A, B)
    this.whiteKeysInOctave = [0, 2, 4, 5, 7, 9, 11]; // Semitones from C
    // Black keys indexed by white key position (after which white key the black key appears)
    // Position 0 (after C), 1 (after D), null (no black after E), 3 (after F), 4 (after G), 5 (after A), null (no black after B)
    this.blackKeysInOctave = [1, 3, null, 6, 8, 10, null]; // Actual semitone values

    // Keyboard bindings matching piano layout
    // Black keys: W E (one group) T Y U (other group)
    // White keys: A S D F G H J K
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
      const midiNote = this.keyboardMap[e.key.toLowerCase()];
      if (midiNote !== undefined) {
        this.noteOn(midiNote, 100); // Default velocity 100
        e.preventDefault();
      }
    });

    window.addEventListener('keyup', (e) => {
      const midiNote = this.keyboardMap[e.key.toLowerCase()];
      if (midiNote !== undefined) {
        this.noteOff(midiNote);
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

    // Keyboard-mapped range is C4 (60) to C5 (72)
    // This contains 8 white keys: C, D, E, F, G, A, B, C
    const keyboardCenter = 60; // C4
    const keyboardWhiteKeys = 8;

    if (whiteKeysFit <= keyboardWhiteKeys) {
      // Not enough space to show all keyboard keys, just center what we have
      this.visibleStartNote = 60; // C4
      this.visibleEndNote = 72; // C5
      const totalWhiteKeyWidth = keyboardWhiteKeys * whiteKeyWidth;
      const offsetX = (width - totalWhiteKeyWidth) / 2;
      return { offsetX, whiteKeyWidth };
    }

    // Calculate how many extra white keys we have space for
    const extraWhiteKeys = whiteKeysFit - keyboardWhiteKeys;
    const leftExtra = Math.floor(extraWhiteKeys / 2);
    const rightExtra = extraWhiteKeys - leftExtra;

    // Start from C4 and go back leftExtra white keys
    let startNote = 60; // C4
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

    // Send to backend - use track ID 0 (first MIDI track)
    // TODO: Make this configurable to select which track to send to
    invoke('audio_send_midi_note_on', { trackId: 0, note: midiNote, velocity }).catch(error => {
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

    // Send to backend - use track ID 0 (first MIDI track)
    invoke('audio_send_midi_note_off', { trackId: 0, note: midiNote }).catch(error => {
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
      this.noteOn(key, 100);
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
      const keyLabel = this.noteToKeyMap[note];
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
      const keyLabel = this.noteToKeyMap[note];
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
    this.dragMode = null  // null, 'move', 'resize-left', 'resize-right', 'create'
    this.dragStartX = 0
    this.dragStartY = 0
    this.creatingNote = null  // Temporary note being created
    this.isDragging = false

    // Note preview playback state
    this.playingNote = null  // Currently playing note number
    this.playingNoteMaxDuration = null  // Max duration in seconds
    this.playingNoteStartTime = null  // Timestamp when note started playing

    // Auto-scroll state
    this.autoScrollEnabled = true  // Auto-scroll to follow playhead during playback
    this.lastPlayheadTime = 0  // Track last playhead position

    // Start timer to check for note duration expiry
    this.checkNoteDurationTimer = setInterval(() => this.checkNoteDuration(), 50)
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

  // Get the currently selected MIDI clip from context
  getSelectedClip() {
    if (typeof context === 'undefined' || !context.activeObject || !context.activeObject.audioTracks) {
      return null
    }

    // Find the first MIDI track with a selected clip
    for (const track of context.activeObject.audioTracks) {
      if (track.type === 'midi' && track.clips && track.clips.length > 0) {
        // For now, just return the first clip on the first MIDI track
        // TODO: Add proper clip selection mechanism
        return { clip: track.clips[0], trackId: track.audioTrackId }
      }
    }
    return null
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

  // Find note at screen position
  findNoteAtPosition(x, y) {
    const clipData = this.getSelectedClip()
    if (!clipData || !clipData.clip.notes) {
      return -1
    }

    const note = this.screenToNote(y)
    const time = this.screenToTime(x)

    // Search in reverse order so we find top-most notes first
    for (let i = clipData.clip.notes.length - 1; i >= 0; i--) {
      const n = clipData.clip.notes[i]
      const noteMatches = Math.round(n.note) === Math.round(note)
      const timeInRange = time >= n.start_time && time <= (n.start_time + n.duration)

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
    const noteEndX = this.timeToScreenX(note.start_time + note.duration)

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
      // Clicking on empty space - start creating a new note
      this.dragMode = 'create'
      this.selectedNotes.clear()

      // Create a temporary note for preview
      const newNoteValue = Math.round(note)
      this.creatingNote = {
        note: newNoteValue,
        start_time: time,
        duration: 0.1, // Minimum duration
        velocity: 100
      }

      // Play preview of the new note
      const clipData = this.getSelectedClip()
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
        const duration = Math.max(0.1, currentTime - this.creatingNote.start_time)
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
        const newDuration = Math.max(0.1, currentTime - note.start_time)
        note.duration = newDuration

        // Trigger timeline redraw to show updated notes
        if (context.timelineWidget) {
          context.timelineWidget.requestRedraw()
        }
      }
    }
  }

  mouseup(x, y) {
    this._globalEvents.delete("mousemove")
    this._globalEvents.delete("mouseup")

    const clipData = this.getSelectedClip()

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

    // Draw notes if we have a selected clip
    const selected = this.getSelectedClip()
    if (selected && selected.clip && selected.clip.notes) {
      this.drawNotes(ctx, this.width, this.height, selected.clip)
    }

    // Draw playhead
    this.drawPlayhead(ctx, this.width, this.height)
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
    const gridLeft = this.keyboardWidth
    const gridWidth = width - gridLeft

    ctx.save()
    ctx.beginPath()
    ctx.rect(gridLeft, 0, gridWidth, height)
    ctx.clip()

    // Draw background
    ctx.fillStyle = backgroundColor
    ctx.fillRect(gridLeft, 0, gridWidth, height)

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

  drawNotes(ctx, width, height, clip) {
    const gridLeft = this.keyboardWidth

    ctx.save()
    ctx.beginPath()
    ctx.rect(gridLeft, 0, width - gridLeft, height)
    ctx.clip()

    // Draw existing notes
    ctx.fillStyle = '#6fdc6f'

    for (let i = 0; i < clip.notes.length; i++) {
      const note = clip.notes[i]

      const x = this.timeToScreenX(note.start_time)
      const y = this.noteToScreenY(note.note)
      const noteWidth = note.duration * this.pixelsPerSecond
      const noteHeight = this.noteHeight - 2

      // Skip if off-screen
      if (x + noteWidth < gridLeft || x > width || y + noteHeight < 0 || y > height) {
        continue
      }

      // Highlight selected notes
      if (this.selectedNotes.has(i)) {
        ctx.fillStyle = '#8ffc8f'
      } else {
        ctx.fillStyle = '#6fdc6f'
      }

      ctx.fillRect(x, y, noteWidth, noteHeight)

      // Draw border
      ctx.strokeStyle = 'rgba(0, 0, 0, 0.3)'
      ctx.strokeRect(x, y, noteWidth, noteHeight)
    }

    // Draw note being created
    if (this.creatingNote) {
      const x = this.timeToScreenX(this.creatingNote.start_time)
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
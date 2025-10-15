import { backgroundColor, foregroundColor, frameWidth, highlight, layerHeight, shade, shadow, labelColor } from "./styles.js";
import { clamp, drawBorderedRect, drawCheckerboardBackground, hslToRgb, hsvToRgb, rgbToHex } from "./utils.js"
import { TimelineState, TimeRuler, TrackHierarchy } from "./timeline.js"

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
      // } else if (layer instanceof AudioLayer) {
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
  }

  draw(ctx) {
    ctx.save()

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
        ctx.fillStyle = i % 2 === 0 ? backgroundColor : shade
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

      // Draw track name
      ctx.fillStyle = labelColor
      ctx.font = '12px sans-serif'
      ctx.textAlign = 'left'
      ctx.textBaseline = 'middle'
      ctx.fillText(track.name, indent + 20, y + this.trackHierarchy.trackHeight / 2)

      // Draw type indicator
      ctx.fillStyle = foregroundColor
      ctx.font = '10px sans-serif'
      const typeText = track.type === 'layer' ? '[L]' : track.type === 'object' ? '[G]' : '[S]'
      ctx.fillText(typeText, indent + 20 + ctx.measureText(track.name).width + 8, y + this.trackHierarchy.trackHeight / 2)

      // Draw toggle buttons for object/shape tracks (Phase 3)
      if (track.type === 'object' || track.type === 'shape') {
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
        const curveSymbol = track.object.curvesMode === 'expanded' ? '~' :
                           track.object.curvesMode === 'minimized' ? '≈' : '-'
        ctx.fillText(curveSymbol, buttonX + buttonSize / 2, buttonY + buttonSize / 2)

        // Segment visibility button
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

      // Draw track background (alternating colors only, no selection highlight)
      ctx.fillStyle = i % 2 === 0 ? backgroundColor : shade
      ctx.fillRect(0, y, trackAreaWidth, trackHeight)

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

        // Get the exists curve for this shape
        const existsCurveKey = `shape.${shape.idx}.exists`
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

      // Only draw curves for objects and shapes
      if (track.type !== 'object' && track.type !== 'shape') continue

      const obj = track.object

      // Skip if curves are hidden
      if (obj.curvesMode === 'hidden') continue

      const y = this.trackHierarchy.getTrackY(i)

      // Find the layer containing this object/shape to get AnimationData
      let animationData = null
      if (track.type === 'object') {
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

      // Get all curves for this object/shape
      const curves = []
      for (let curveName in animationData.curves) {
        const curve = animationData.curves[curveName]

        // Filter to only curves for this specific object/shape
        if (track.type === 'object' && curveName.startsWith(`child.${obj.idx}.`)) {
          curves.push(curve)
        } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.idx}.`)) {
          curves.push(curve)
        }
      }

      if (curves.length === 0) continue

      // Draw based on curves mode
      if (obj.curvesMode === 'minimized') {
        this.drawMinimizedCurves(ctx, curves, y)
      } else if (obj.curvesMode === 'expanded') {
        this.drawExpandedCurves(ctx, curves, y)
      }
    }

    ctx.restore()
  }

  /**
   * Draw minimized curves (keyframe dots only)
   */
  drawMinimizedCurves(ctx, curves, trackY) {
    const dotRadius = 3
    const rowHeight = 15  // Height per curve in minimized mode
    const startY = trackY + 10  // Start below segment area

    for (let i = 0; i < curves.length; i++) {
      const curve = curves[i]
      const curveY = startY + (i * rowHeight)

      // Draw keyframe dots
      for (let keyframe of curve.keyframes) {
        const x = this.timelineState.timeToPixel(keyframe.time)

        ctx.fillStyle = curve.displayColor
        ctx.beginPath()
        ctx.arc(x, curveY, dotRadius, 0, 2 * Math.PI)
        ctx.fill()

        // Draw outline for visibility
        ctx.strokeStyle = shadow
        ctx.lineWidth = 1
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
            // Calculate control points for Bezier curve
            const cpOffset = (x2 - x1) / 3  // Control points at 1/3 and 2/3 of time range

            const cp1x = x1 + cpOffset
            const cp1y = y1 + (kf1.outTangent || 0) * cpOffset
            const cp2x = x2 - cpOffset
            const cp2y = y2 - (kf2.inTangent || 0) * cpOffset

            ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, x2, y2)
            ctx.stroke()

            // Draw tangent handles for bezier mode only
            ctx.strokeStyle = curve.displayColor + '80'  // Semi-transparent
            ctx.lineWidth = 1

            // Out tangent handle
            ctx.beginPath()
            ctx.moveTo(x1, y1)
            ctx.lineTo(cp1x, cp1y)
            ctx.stroke()

            // In tangent handle
            ctx.beginPath()
            ctx.moveTo(x2, y2)
            ctx.lineTo(cp2x, cp2y)
            ctx.stroke()

            // Reset for next curve segment
            ctx.strokeStyle = curve.displayColor
            ctx.lineWidth = 2
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
            // Cycle through curves modes: hidden -> minimized -> expanded -> hidden
            if (track.object.curvesMode === 'hidden') {
              track.object.curvesMode = 'minimized'
            } else if (track.object.curvesMode === 'minimized') {
              track.object.curvesMode = 'expanded'
            } else {
              track.object.curvesMode = 'hidden'
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
        // Phase 5: Check if clicking on expanded curves
        if ((track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'expanded') {
          const curveClickResult = this.handleCurveClick(track, adjustedX, adjustedY)
          if (curveClickResult) {
            return true
          }
        }

        // Check if clicking on segment
        if (this.isPointInSegment(track, adjustedX, adjustedY)) {
          this.selectTrack(track)
          if (this.requestRedraw) this.requestRedraw()
          return true
        }
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
      } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.idx}.`)) {
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
    // Find the closest curve to the click position
    let targetCurve = curves[0]
    let minDistance = Infinity

    for (let curve of curves) {
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

      const existsCurveKey = `shape.${shape.idx}.exists`
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
   * Check if a track is currently selected
   */
  isTrackSelected(track) {
    if (track.type === 'layer') {
      return this.context.activeLayer === track.object
    } else if (track.type === 'shape') {
      return this.context.shapeselection?.includes(track.object)
    } else if (track.type === 'object') {
      return this.context.selection?.includes(track.object)
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
      // Find the index of this layer in the activeObject
      const layerIndex = this.context.activeObject.children.indexOf(track.object)
      if (layerIndex !== -1) {
        this.context.activeObject.currentLayer = layerIndex
      }
      // Clear selections when selecting layer
      this.context.selection = []
      this.context.shapeselection = []
    } else if (track.type === 'shape') {
      // Find the layer this shape belongs to and select it
      for (let i = 0; i < this.context.activeObject.allLayers.length; i++) {
        const layer = this.context.activeObject.allLayers[i]
        if (layer.shapes && layer.shapes.includes(track.object)) {
          // Find index in children array
          const layerIndex = this.context.activeObject.children.indexOf(layer)
          if (layerIndex !== -1) {
            this.context.activeObject.currentLayer = layerIndex
          }
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
    // Update hover state for keyframe tooltips (even when not dragging)
    // Clear hover if mouse is outside timeline curve areas
    let foundHover = false

    if (!this.draggingKeyframe && !this.draggingPlayhead) {
      const trackY = y - this.ruler.height
      if (trackY >= 0 && x >= this.trackHeaderWidth) {
        const adjustedY = trackY - this.trackScrollOffset
        const adjustedX = x - this.trackHeaderWidth
        const track = this.trackHierarchy.getTrackAtY(adjustedY)

        if (track && (track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'expanded') {
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
                } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.idx}.`)) {
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
        selectedKeyframe.value = selectedKeyframe.initialDragValue + valueDelta
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

    return false
  }

  /**
   * Handle right-click context menu (Phase 5)
   * Deletes keyframe if right-clicking on one
   */
  contextmenu(x, y) {
    // Check if right-clicking in timeline area with curves
    const trackY = y - this.ruler.height
    if (trackY >= 0 && x >= this.trackHeaderWidth) {
      const adjustedY = trackY - this.trackScrollOffset
      const adjustedX = x - this.trackHeaderWidth
      const track = this.trackHierarchy.getTrackAtY(adjustedY)

      if (track && (track.type === 'object' || track.type === 'shape') && track.object.curvesMode === 'expanded') {
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
            } else if (track.type === 'shape' && curveName.startsWith(`shape.${obj.idx}.`)) {
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
          for (let curve of curves) {
            for (let i = 0; i < curve.keyframes.length; i++) {
              const keyframe = curve.keyframes[i]
              const kfX = this.timelineState.timeToPixel(keyframe.time)
              const kfY = startY + curveHeight - padding - ((keyframe.value - minValue) / (maxValue - minValue) * (curveHeight - 2 * padding))
              const distance = Math.sqrt((adjustedX - kfX) ** 2 + (adjustedY - kfY) ** 2)

              if (distance < 8) {
                // Check if this keyframe is in the current selection
                const isInSelection = this.selectedKeyframes.has(keyframe)

                // Determine what to delete
                // If there are multiple selected keyframes (regardless of which one we clicked),
                // show the confirmation menu
                if (this.selectedKeyframes.size > 1) {
                  // Delete all selected keyframes
                  const keyframesToDelete = Array.from(this.selectedKeyframes)
                  this.showDeleteKeyframesMenu(keyframesToDelete, curves)
                  return true
                }

                // Single keyframe deletion - check if it's the last one in its curve
                if (curve.keyframes.length <= 1) {
                  console.log(`Cannot delete last keyframe in curve ${curve.parameter}`)
                  return true  // Still return true to indicate event was handled
                }

                // Delete single keyframe
                console.log(`Deleting keyframe at time ${keyframe.time} from curve ${curve.parameter}`)
                curve.keyframes.splice(i, 1)

                // Remove from selection if it was selected
                this.selectedKeyframes.delete(keyframe)

                if (this.requestRedraw) this.requestRedraw()
                return true
              }
            }
          }
        }
      }
    }

    return false
  }

  /**
   * Show Tauri context menu for deleting multiple selected keyframes (Phase 5)
   */
  async showDeleteKeyframesMenu(keyframesToDelete, curves) {
    const { Menu, MenuItem } = window.__TAURI__.menu
    const { PhysicalPosition, LogicalPosition } = window.__TAURI__.dpi

    // Build menu with delete option
    const items = [
      await MenuItem.new({
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
      })
    ]

    const menu = await Menu.new({ items })

    // Show menu at mouse position (using lastEvent for clientX/clientY)
    const clientX = this.lastEvent?.clientX || 0
    const clientY = this.lastEvent?.clientY || 0
    const position = new PhysicalPosition(clientX, clientY)
    console.log(position)
    // await menu.popup({ at: position })
    await menu.popup(position)
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
  TimelineWindowV2
};
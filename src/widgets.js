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
      "dblclick"
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

    // Create shared timeline state (24 fps default)
    this.timelineState = new TimelineState(24)

    // Create time ruler widget
    this.ruler = new TimeRuler(this.timelineState)

    // Create track hierarchy manager
    this.trackHierarchy = new TrackHierarchy()

    // Track if we're dragging playhead
    this.draggingPlayhead = false

    // Vertical scroll offset for track hierarchy
    this.trackScrollOffset = 0
  }

  draw(ctx) {
    ctx.save()

    // Draw background
    ctx.fillStyle = backgroundColor
    ctx.fillRect(0, 0, this.width, this.height)

    // Draw time ruler at top
    this.ruler.draw(ctx, this.width)

    // Phase 2: Build and draw track hierarchy
    if (this.context.activeObject) {
      this.trackHierarchy.buildTracks(this.context.activeObject)
      this.drawTracks(ctx)
    }

    // TODO Phase 3: Draw segments
    // TODO Phase 4: Draw minimized curves

    ctx.restore()
  }

  /**
   * Draw track hierarchy (Phase 2)
   */
  drawTracks(ctx) {
    ctx.save()
    ctx.translate(0, this.ruler.height)  // Start below ruler

    // Clip to available track area
    const trackAreaHeight = this.height - this.ruler.height
    ctx.beginPath()
    ctx.rect(0, 0, this.width, trackAreaHeight)
    ctx.clip()

    // Apply vertical scroll offset
    ctx.translate(0, this.trackScrollOffset)

    const indentSize = 20  // Pixels per indent level

    for (let i = 0; i < this.trackHierarchy.tracks.length; i++) {
      const track = this.trackHierarchy.tracks[i]
      const y = i * this.trackHierarchy.trackHeight

      // Check if this track is selected
      const isSelected = this.isTrackSelected(track)

      // Draw track background (alternating colors or selected highlight)
      if (isSelected) {
        ctx.fillStyle = highlight  // Highlighted color for selected track
      } else {
        ctx.fillStyle = i % 2 === 0 ? backgroundColor : shade
      }
      ctx.fillRect(0, y, this.width, this.trackHierarchy.trackHeight)

      // Draw track border
      ctx.strokeStyle = shadow
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, y + this.trackHierarchy.trackHeight)
      ctx.lineTo(this.width, y + this.trackHierarchy.trackHeight)
      ctx.stroke()

      // Calculate indent
      const indent = track.indent * indentSize

      // Draw expand/collapse indicator for layers and objects with children
      if (track.type === 'layer' || (track.type === 'object' && track.object.children && track.object.children.length > 0)) {
        const triangleX = indent + 8
        const triangleY = y + this.trackHierarchy.trackHeight / 2

        ctx.fillStyle = foregroundColor
        ctx.beginPath()
        if (track.collapsed) {
          // Collapsed: right-pointing triangle ▶
          ctx.moveTo(triangleX, triangleY - 4)
          ctx.lineTo(triangleX + 6, triangleY)
          ctx.lineTo(triangleX, triangleY + 4)
        } else {
          // Expanded: down-pointing triangle ▼
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
    }

    ctx.restore()
  }

  mousedown(x, y) {
    // Check if clicking in ruler area
    if (y <= this.ruler.height) {
      // Let the ruler handle the mousedown (for playhead dragging)
      const hitPlayhead = this.ruler.mousedown(x, y);
      if (hitPlayhead) {
        this.draggingPlayhead = true
        this._globalEvents.add("mousemove")
        this._globalEvents.add("mouseup")
        return true
      }
    }

    // Check if clicking in track area
    const trackY = y - this.ruler.height
    if (trackY >= 0) {
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

        // Clicking elsewhere on track selects it
        this.selectTrack(track)
        if (this.requestRedraw) this.requestRedraw()
        return true
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
    if (this.draggingPlayhead) {
      // Let the ruler handle the mousemove
      this.ruler.mousemove(x, y)

      // Sync GraphicsObject currentTime with timeline playhead
      if (this.context.activeObject) {
        this.context.activeObject.currentTime = this.timelineState.currentTime
      }
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
    return false
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
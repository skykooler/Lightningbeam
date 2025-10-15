// Timeline V2 - New timeline implementation for AnimationData curve-based system

import { backgroundColor, foregroundColor, shadow, labelColor, scrubberColor } from "./styles.js"

/**
 * TimelineState - Global state for timeline display and interaction
 */
class TimelineState {
  constructor(framerate = 24) {
    // Time format settings
    this.timeFormat = 'frames'  // 'frames' | 'seconds' | 'measures'
    this.framerate = framerate

    // Zoom and viewport
    this.pixelsPerSecond = 100  // Zoom level - how many pixels per second of animation
    this.viewportStartTime = 0  // Horizontal scroll position (in seconds)

    // Playhead
    this.currentTime = 0  // Current time (in seconds)

    // Ruler settings
    this.rulerHeight = 30  // Height of time ruler in pixels
  }

  /**
   * Convert time (seconds) to pixel position
   */
  timeToPixel(time) {
    return (time - this.viewportStartTime) * this.pixelsPerSecond
  }

  /**
   * Convert pixel position to time (seconds)
   */
  pixelToTime(pixel) {
    return (pixel / this.pixelsPerSecond) + this.viewportStartTime
  }

  /**
   * Convert time (seconds) to frame number
   */
  timeToFrame(time) {
    return Math.floor(time * this.framerate)
  }

  /**
   * Convert frame number to time (seconds)
   */
  frameToTime(frame) {
    return frame / this.framerate
  }

  /**
   * Calculate appropriate ruler interval based on zoom level
   * Returns interval in seconds that gives ~50-100px spacing
   */
  getRulerInterval() {
    const targetPixelSpacing = 75  // Target pixels between major ticks
    const timeSpacing = targetPixelSpacing / this.pixelsPerSecond  // In seconds

    // Standard interval options (in seconds)
    const intervals = [
      0.01, 0.02, 0.05,  // 10ms, 20ms, 50ms
      0.1, 0.2, 0.5,     // 100ms, 200ms, 500ms
      1, 2, 5,           // 1s, 2s, 5s
      10, 20, 30, 60,    // 10s, 20s, 30s, 1min
      120, 300, 600      // 2min, 5min, 10min
    ]

    // Find closest interval
    let bestInterval = intervals[0]
    let bestDiff = Math.abs(timeSpacing - bestInterval)

    for (let interval of intervals) {
      const diff = Math.abs(timeSpacing - interval)
      if (diff < bestDiff) {
        bestDiff = diff
        bestInterval = interval
      }
    }

    return bestInterval
  }

  /**
   * Calculate appropriate ruler interval for frame mode
   * Returns interval in frames that gives ~50-100px spacing
   */
  getRulerIntervalFrames() {
    const targetPixelSpacing = 75
    const pixelsPerFrame = this.pixelsPerSecond / this.framerate
    const frameSpacing = targetPixelSpacing / pixelsPerFrame

    // Standard frame intervals
    const intervals = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000]

    // Find closest interval
    let bestInterval = intervals[0]
    let bestDiff = Math.abs(frameSpacing - bestInterval)

    for (let interval of intervals) {
      const diff = Math.abs(frameSpacing - interval)
      if (diff < bestDiff) {
        bestDiff = diff
        bestInterval = interval
      }
    }

    return bestInterval
  }

  /**
   * Format time for display based on current format setting
   */
  formatTime(time) {
    if (this.timeFormat === 'frames') {
      return `${this.timeToFrame(time)}`
    } else if (this.timeFormat === 'seconds') {
      const minutes = Math.floor(time / 60)
      const seconds = Math.floor(time % 60)
      const ms = Math.floor((time % 1) * 10)

      if (minutes > 0) {
        return `${minutes}:${seconds.toString().padStart(2, '0')}`
      } else {
        return `${seconds}.${ms}s`
      }
    }
    // measures format - TODO when DAW features added
    return `${time.toFixed(2)}`
  }

  /**
   * Zoom in (increase pixelsPerSecond)
   */
  zoomIn(factor = 1.5) {
    this.pixelsPerSecond *= factor
    // Clamp to reasonable range
    this.pixelsPerSecond = Math.min(this.pixelsPerSecond, 10000)  // Max zoom
  }

  /**
   * Zoom out (decrease pixelsPerSecond)
   */
  zoomOut(factor = 1.5) {
    this.pixelsPerSecond /= factor
    // Clamp to reasonable range
    this.pixelsPerSecond = Math.max(this.pixelsPerSecond, 10)  // Min zoom
  }
}

/**
 * TimeRuler - Widget for displaying time ruler with adaptive intervals
 */
class TimeRuler {
  constructor(timelineState) {
    this.state = timelineState
    this.height = timelineState.rulerHeight
  }

  /**
   * Draw the time ruler
   */
  draw(ctx, width) {
    ctx.save()

    // Background
    ctx.fillStyle = backgroundColor
    ctx.fillRect(0, 0, width, this.height)

    // Determine interval based on current zoom and format
    let interval, isFrameMode
    if (this.state.timeFormat === 'frames') {
      interval = this.state.getRulerIntervalFrames()  // In frames
      isFrameMode = true
    } else {
      interval = this.state.getRulerInterval()  // In seconds
      isFrameMode = false
    }

    // Calculate visible time range
    const startTime = this.state.viewportStartTime
    const endTime = this.state.pixelToTime(width)

    // Draw tick marks and labels
    if (isFrameMode) {
      this.drawFrameTicks(ctx, width, interval, startTime, endTime)
    } else {
      this.drawSecondTicks(ctx, width, interval, startTime, endTime)
    }

    // Draw playhead (current time indicator)
    this.drawPlayhead(ctx, width)

    ctx.restore()
  }

  /**
   * Draw tick marks for frame mode
   */
  drawFrameTicks(ctx, width, interval, startTime, endTime) {
    const startFrame = Math.floor(this.state.timeToFrame(startTime) / interval) * interval
    const endFrame = Math.ceil(this.state.timeToFrame(endTime) / interval) * interval

    ctx.fillStyle = labelColor
    ctx.font = '11px sans-serif'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'top'

    for (let frame = startFrame; frame <= endFrame; frame += interval) {
      const time = this.state.frameToTime(frame)
      const x = this.state.timeToPixel(time)

      if (x < 0 || x > width) continue

      // Major tick
      ctx.strokeStyle = foregroundColor
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(x, this.height - 10)
      ctx.lineTo(x, this.height)
      ctx.stroke()

      // Label
      ctx.fillText(frame.toString(), x, 2)

      // Minor ticks (subdivisions)
      const minorInterval = interval / 5
      if (minorInterval >= 1) {
        for (let i = 1; i < 5; i++) {
          const minorFrame = frame + (minorInterval * i)
          const minorTime = this.state.frameToTime(minorFrame)
          const minorX = this.state.timeToPixel(minorTime)

          if (minorX < 0 || minorX > width) continue

          ctx.strokeStyle = shadow
          ctx.beginPath()
          ctx.moveTo(minorX, this.height - 5)
          ctx.lineTo(minorX, this.height)
          ctx.stroke()
        }
      }
    }
  }

  /**
   * Draw tick marks for second mode
   */
  drawSecondTicks(ctx, width, interval, startTime, endTime) {
    const startTick = Math.floor(startTime / interval) * interval
    const endTick = Math.ceil(endTime / interval) * interval

    ctx.fillStyle = labelColor
    ctx.font = '11px sans-serif'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'top'

    for (let time = startTick; time <= endTick; time += interval) {
      const x = this.state.timeToPixel(time)

      if (x < 0 || x > width) continue

      // Major tick
      ctx.strokeStyle = foregroundColor
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(x, this.height - 10)
      ctx.lineTo(x, this.height)
      ctx.stroke()

      // Label
      ctx.fillText(this.state.formatTime(time), x, 2)

      // Minor ticks (subdivisions)
      const minorInterval = interval / 5
      for (let i = 1; i < 5; i++) {
        const minorTime = time + (minorInterval * i)
        const minorX = this.state.timeToPixel(minorTime)

        if (minorX < 0 || minorX > width) continue

        ctx.strokeStyle = shadow
        ctx.beginPath()
        ctx.moveTo(minorX, this.height - 5)
        ctx.lineTo(minorX, this.height)
        ctx.stroke()
      }
    }
  }

  /**
   * Draw playhead (current time indicator)
   */
  drawPlayhead(ctx, width) {
    const x = this.state.timeToPixel(this.state.currentTime)

    // Only draw if playhead is visible
    if (x < 0 || x > width) return

    ctx.strokeStyle = scrubberColor
    ctx.lineWidth = 2
    ctx.beginPath()
    ctx.moveTo(x, 0)
    ctx.lineTo(x, this.height)
    ctx.stroke()

    // Playhead handle (triangle at top)
    ctx.fillStyle = scrubberColor
    ctx.beginPath()
    ctx.moveTo(x, 0)
    ctx.lineTo(x - 6, 8)
    ctx.lineTo(x + 6, 8)
    ctx.closePath()
    ctx.fill()
  }

  /**
   * Hit test for playhead dragging (no longer used, kept for potential future use)
   */
  hitTestPlayhead(x, y) {
    const playheadX = this.state.timeToPixel(this.state.currentTime)
    const distance = Math.abs(x - playheadX)

    // 10px tolerance for hitting playhead
    return distance < 10 && y >= 0 && y <= this.height
  }

  /**
   * Handle mouse down - start dragging playhead
   */
  mousedown(x, y) {
    // Clicking anywhere in the ruler moves the playhead there
    this.state.currentTime = this.state.pixelToTime(x)
    this.state.currentTime = Math.max(0, this.state.currentTime)
    this.draggingPlayhead = true
    return true
  }

  /**
   * Handle mouse move - drag playhead
   */
  mousemove(x, y) {
    if (this.draggingPlayhead) {
      const newTime = this.state.pixelToTime(x);
      this.state.currentTime = Math.max(0, newTime);
      return true
    }
    return false
  }

  /**
   * Handle mouse up - stop dragging
   */
  mouseup(x, y) {
    if (this.draggingPlayhead) {
      this.draggingPlayhead = false
      return true
    }
    return false
  }
}

/**
 * TrackHierarchy - Builds and manages hierarchical track structure from GraphicsObject
 * Phase 2: Track hierarchy display
 */
class TrackHierarchy {
  constructor() {
    this.tracks = []  // Flat list of tracks for rendering
    this.trackHeight = 30  // Default track height in pixels
  }

  /**
   * Build track list from GraphicsObject layers
   * Creates a flattened list of tracks for rendering, maintaining hierarchy info
   */
  buildTracks(graphicsObject) {
    this.tracks = []

    if (!graphicsObject || !graphicsObject.children) {
      return
    }

    // Iterate through layers (GraphicsObject.children are Layers)
    for (let layer of graphicsObject.children) {
      // Add layer track
      const layerTrack = {
        type: 'layer',
        object: layer,
        name: layer.name || 'Layer',
        indent: 0,
        collapsed: layer.collapsed || false,
        visible: layer.visible !== false
      }
      this.tracks.push(layerTrack)

      // If layer is not collapsed, add its children
      if (!layerTrack.collapsed) {
        // Add child GraphicsObjects (nested groups)
        if (layer.children) {
          for (let child of layer.children) {
            this.addObjectTrack(child, 1)
          }
        }

        // Add shapes
        if (layer.shapes) {
          for (let shape of layer.shapes) {
            this.addShapeTrack(shape, 1)
          }
        }
      }
    }
  }

  /**
   * Recursively add object track and its children
   */
  addObjectTrack(obj, indent) {
    const track = {
      type: 'object',
      object: obj,
      name: obj.name || obj.idx,
      indent: indent,
      collapsed: obj.trackCollapsed || false
    }
    this.tracks.push(track)

    // If object is not collapsed, add its children
    if (!track.collapsed && obj.children) {
      for (let layer of obj.children) {
        // Nested object's layers
        const nestedLayerTrack = {
          type: 'layer',
          object: layer,
          name: layer.name || 'Layer',
          indent: indent + 1,
          collapsed: layer.collapsed || false,
          visible: layer.visible !== false
        }
        this.tracks.push(nestedLayerTrack)

        if (!nestedLayerTrack.collapsed) {
          // Add nested layer's children
          if (layer.children) {
            for (let child of layer.children) {
              this.addObjectTrack(child, indent + 2)
            }
          }
          if (layer.shapes) {
            for (let shape of layer.shapes) {
              this.addShapeTrack(shape, indent + 2)
            }
          }
        }
      }
    }
  }

  /**
   * Add shape track
   */
  addShapeTrack(shape, indent) {
    const track = {
      type: 'shape',
      object: shape,
      name: shape.constructor.name || 'Shape',
      indent: indent
    }
    this.tracks.push(track)
  }

  /**
   * Calculate total height needed for all tracks
   */
  getTotalHeight() {
    return this.tracks.length * this.trackHeight
  }

  /**
   * Get track at a given Y position
   */
  getTrackAtY(y) {
    const trackIndex = Math.floor(y / this.trackHeight)
    if (trackIndex >= 0 && trackIndex < this.tracks.length) {
      return this.tracks[trackIndex]
    }
    return null
  }
}

export { TimelineState, TimeRuler, TrackHierarchy }

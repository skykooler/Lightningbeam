// Timeline V2 - New timeline implementation for AnimationData curve-based system

import { backgroundColor, foregroundColor, shadow, labelColor, scrubberColor } from "./styles.js"

/**
 * TimelineState - Global state for timeline display and interaction
 */
class TimelineState {
  constructor(framerate = 24, bpm = 120, timeSignature = { numerator: 4, denominator: 4 }) {
    // Time format settings
    this.timeFormat = 'frames'  // 'frames' | 'seconds' | 'measures'
    this.framerate = framerate
    this.bpm = bpm  // Beats per minute for measures mode
    this.timeSignature = timeSignature  // Time signature for measures mode (e.g., {numerator: 4, denominator: 4} or {numerator: 6, denominator: 8})

    // Zoom and viewport
    this.pixelsPerSecond = 100  // Zoom level - how many pixels per second of animation
    this.viewportStartTime = 0  // Horizontal scroll position (in seconds)

    // Playhead
    this.currentTime = 0  // Current time (in seconds)

    // Ruler settings
    this.rulerHeight = 30  // Height of time ruler in pixels

    // Snapping (Phase 5)
    this.snapToFrames = true  // Whether to snap keyframes to frame boundaries (default: on)
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
   * Convert time (seconds) to measure position
   * Returns {measure, beat, tick} where tick is subdivision of beat (0-999)
   */
  timeToMeasure(time) {
    const beatsPerSecond = this.bpm / 60
    const totalBeats = time * beatsPerSecond
    const beatsPerMeasure = this.timeSignature.numerator
    const measure = Math.floor(totalBeats / beatsPerMeasure) + 1  // Measures are 1-indexed
    const beat = Math.floor(totalBeats % beatsPerMeasure) + 1  // Beats are 1-indexed
    const tick = Math.floor((totalBeats % 1) * 1000)  // Ticks are 0-999
    return { measure, beat, tick }
  }

  /**
   * Convert measure position to time (seconds)
   */
  measureToTime(measure, beat = 1, tick = 0) {
    const beatsPerMeasure = this.timeSignature.numerator
    const totalBeats = (measure - 1) * beatsPerMeasure + (beat - 1) + (tick / 1000)
    const beatsPerSecond = this.bpm / 60
    return totalBeats / beatsPerSecond
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
   * Calculate appropriate ruler interval for measures mode
   * Returns interval in beats that gives ~50-100px spacing
   */
  getRulerIntervalBeats() {
    const targetPixelSpacing = 75
    const beatsPerSecond = this.bpm / 60
    const pixelsPerBeat = this.pixelsPerSecond / beatsPerSecond
    const beatSpacing = targetPixelSpacing / pixelsPerBeat

    const beatsPerMeasure = this.timeSignature.numerator
    // Standard beat intervals: 1 beat, 2 beats, 1 measure, 2 measures, 4 measures, etc.
    const intervals = [1, 2, beatsPerMeasure, beatsPerMeasure * 2, beatsPerMeasure * 4, beatsPerMeasure * 8, beatsPerMeasure * 16]

    // Find closest interval
    let bestInterval = intervals[0]
    let bestDiff = Math.abs(beatSpacing - bestInterval)

    for (let interval of intervals) {
      const diff = Math.abs(beatSpacing - interval)
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
    } else if (this.timeFormat === 'measures') {
      const { measure, beat } = this.timeToMeasure(time)
      return `${measure}.${beat}`
    }
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

  /**
   * Snap time to nearest frame boundary (Phase 5)
   */
  snapTime(time) {
    if (!this.snapToFrames) {
      return time
    }
    const frame = Math.round(time * this.framerate)
    return frame / this.framerate
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

    // Calculate visible time range
    const startTime = this.state.viewportStartTime
    const endTime = this.state.pixelToTime(width)

    // Draw tick marks and labels based on format
    if (this.state.timeFormat === 'frames') {
      const interval = this.state.getRulerIntervalFrames()  // In frames
      this.drawFrameTicks(ctx, width, interval, startTime, endTime)
    } else if (this.state.timeFormat === 'measures') {
      const interval = this.state.getRulerIntervalBeats()  // In beats
      this.drawMeasureTicks(ctx, width, interval, startTime, endTime)
    } else {
      const interval = this.state.getRulerInterval()  // In seconds
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
   * Draw tick marks for measures mode
   */
  drawMeasureTicks(ctx, width, interval, startTime, endTime) {
    const beatsPerSecond = this.state.bpm / 60
    const beatsPerMeasure = this.state.timeSignature.numerator

    // Always draw individual beats, regardless of interval
    const startBeat = Math.floor(startTime * beatsPerSecond)
    const endBeat = Math.ceil(endTime * beatsPerSecond)

    ctx.fillStyle = labelColor
    ctx.font = '11px sans-serif'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'top'

    // Draw all beats
    for (let beat = startBeat; beat <= endBeat; beat++) {
      const time = beat / beatsPerSecond
      const x = this.state.timeToPixel(time)

      if (x < 0 || x > width) continue

      // Determine position within the measure
      const beatInMeasure = beat % beatsPerMeasure
      const isMeasureBoundary = beatInMeasure === 0
      const isEvenBeatInMeasure = (beatInMeasure % 2) === 0

      // Determine tick style based on position
      let opacity, tickHeight
      if (isMeasureBoundary) {
        // Measure boundary: full opacity, tallest
        opacity = 1.0
        tickHeight = 12
      } else if (isEvenBeatInMeasure) {
        // Even beat within measure: half opacity, medium height
        opacity = 0.5
        tickHeight = 8
      } else {
        // Odd beat within measure: quarter opacity, shortest
        opacity = 0.25
        tickHeight = 5
      }

      // Draw tick with appropriate opacity
      ctx.save()
      ctx.globalAlpha = opacity
      ctx.strokeStyle = foregroundColor
      ctx.lineWidth = isMeasureBoundary ? 2 : 1
      ctx.beginPath()
      ctx.moveTo(x, this.height - tickHeight)
      ctx.lineTo(x, this.height)
      ctx.stroke()
      ctx.restore()

      // Determine if we're zoomed in enough to show individual beat labels
      const pixelsPerBeat = this.state.pixelsPerSecond / beatsPerSecond
      const beatFadeThreshold = 100  // Full opacity at 100px per beat
      const beatFadeStart = 60       // Start fading in at 60px per beat

      // Calculate fade opacity for beat labels (0 to 1)
      const beatLabelOpacity = Math.max(0, Math.min(1, (pixelsPerBeat - beatFadeStart) / (beatFadeThreshold - beatFadeStart)))

      // Calculate spacing-based fade for measure labels when zoomed out
      const pixelsPerMeasure = pixelsPerBeat * beatsPerMeasure

      // Determine which measures to show based on spacing
      const { measure: measureNumber } = this.state.timeToMeasure(time)
      let showThisMeasure = false
      let measureLabelOpacity = 1

      const isEvery16th = (measureNumber - 1) % 16 === 0
      const isEvery4th = (measureNumber - 1) % 4 === 0

      if (isEvery16th) {
        // Always show every 16th measure when very zoomed out
        showThisMeasure = true
        if (pixelsPerMeasure < 20) {
          // Fade in from 10-20px
          measureLabelOpacity = Math.max(0, Math.min(1, (pixelsPerMeasure - 10) / 10))
        } else {
          measureLabelOpacity = 1
        }
      } else if (isEvery4th && pixelsPerMeasure >= 20) {
        // Show every 4th measure when zoomed out but not too far
        showThisMeasure = true
        if (pixelsPerMeasure < 30) {
          // Fade in from 20-30px
          measureLabelOpacity = Math.max(0, Math.min(1, (pixelsPerMeasure - 20) / 10))
        } else {
          measureLabelOpacity = 1
        }
      } else if (pixelsPerMeasure >= 80) {
        // Show all measures when zoomed in enough
        showThisMeasure = true
        if (pixelsPerMeasure < 100) {
          // Fade in from 80-100px
          measureLabelOpacity = Math.max(0, Math.min(1, (pixelsPerMeasure - 80) / 20))
        } else {
          measureLabelOpacity = 1
        }
      }

      // Label logic
      if (isMeasureBoundary && showThisMeasure) {
        // Measure boundaries: show just the measure number with fade
        const { measure } = this.state.timeToMeasure(time)
        ctx.save()
        ctx.globalAlpha = measureLabelOpacity
        ctx.fillText(measure.toString(), x, 2)
        ctx.restore()
      } else if (beatLabelOpacity > 0) {
        // Zoomed in: show measure.beat for all beats with fade
        ctx.save()
        ctx.globalAlpha = beatLabelOpacity
        ctx.fillText(this.state.formatTime(time), x, 2)
        ctx.restore()
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
      // Determine layer type - check if it's a VideoLayer
      const layerType = layer.type === 'video' ? 'video' : 'layer'

      // Add layer track
      const layerTrack = {
        type: layerType,
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

        // Add shapes (grouped by shapeId for shape tweening)
        if (layer.shapes) {
          // Group shapes by shapeId
          const shapesByShapeId = new Map();
          for (let shape of layer.shapes) {
            if (!shapesByShapeId.has(shape.shapeId)) {
              shapesByShapeId.set(shape.shapeId, []);
            }
            shapesByShapeId.get(shape.shapeId).push(shape);
          }

          // Add one track per unique shapeId
          for (let [shapeId, shapes] of shapesByShapeId) {
            // Use the first shape as the representative for the track
            this.addShapeTrack(shapes[0], 1, shapeId, shapes)
          }
        }
      }
    }

    // Add audio tracks (after visual layers)
    if (graphicsObject.audioTracks) {
      for (let audioTrack of graphicsObject.audioTracks) {
        const audioTrackItem = {
          type: 'audio',
          object: audioTrack,
          name: audioTrack.name || 'Audio',
          indent: 0,
          collapsed: audioTrack.collapsed || false,
          visible: audioTrack.audible !== false
        }
        this.tracks.push(audioTrackItem)
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
            // Group shapes by shapeId
            const shapesByShapeId = new Map();
            for (let shape of layer.shapes) {
              if (!shapesByShapeId.has(shape.shapeId)) {
                shapesByShapeId.set(shape.shapeId, []);
              }
              shapesByShapeId.get(shape.shapeId).push(shape);
            }

            // Add one track per unique shapeId
            for (let [shapeId, shapes] of shapesByShapeId) {
              this.addShapeTrack(shapes[0], indent + 2, shapeId, shapes)
            }
          }
        }
      }
    }
  }

  /**
   * Add shape track (grouped by shapeId for shape tweening)
   */
  addShapeTrack(shape, indent, shapeId, shapes) {
    const track = {
      type: 'shape',
      object: shape,  // Representative shape for display
      shapeId: shapeId,  // The shared shapeId
      shapes: shapes,  // All shape versions with this shapeId
      name: shape.constructor.name || 'Shape',
      indent: indent
    }
    this.tracks.push(track)
  }

  /**
   * Calculate height for a specific track based on its curves mode (Phase 4)
   */
  getTrackHeight(track) {
    const baseHeight = this.trackHeight

    // Only objects, shapes, and audio tracks can have curves
    if (track.type !== 'object' && track.type !== 'shape' && track.type !== 'audio') {
      return baseHeight
    }

    const obj = track.object

    // Calculate additional height needed for curves
    if (obj.curvesMode === 'keyframe') {
      // Phase 6: Minimized mode should be compact - no extra height
      // Keyframes are overlaid on the segment bar
      return baseHeight
    } else if (obj.curvesMode === 'curve') {
      // Use the object's curvesHeight property
      return baseHeight + (obj.curvesHeight || 150) + 10  // +10 for padding
    }

    return baseHeight
  }

  /**
   * Calculate total height needed for all tracks
   */
  getTotalHeight() {
    let totalHeight = 0
    for (let track of this.tracks) {
      totalHeight += this.getTrackHeight(track)
    }
    return totalHeight
  }

  /**
   * Get track at a given Y position
   */
  getTrackAtY(y) {
    let currentY = 0
    for (let i = 0; i < this.tracks.length; i++) {
      const track = this.tracks[i]
      const trackHeight = this.getTrackHeight(track)

      if (y >= currentY && y < currentY + trackHeight) {
        return track
      }

      currentY += trackHeight
    }
    return null
  }

  /**
   * Get Y position for a specific track index (Phase 4)
   */
  getTrackY(trackIndex) {
    let y = 0
    for (let i = 0; i < trackIndex && i < this.tracks.length; i++) {
      y += this.getTrackHeight(this.tracks[i])
    }
    return y
  }
}

export { TimelineState, TimeRuler, TrackHierarchy }

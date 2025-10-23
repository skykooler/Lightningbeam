// Shape models: BaseShape, TempShape, Shape

import { context, pointerList } from '../state.js';
import { Bezier } from '../bezier.js';
import { Quadtree } from '../quadtree.js';

// Helper function for UUID generation
function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

// Forward declarations for dependencies that will be injected
let growBoundingBox = null;
let lerp = null;
let lerpColor = null;
let uuidToColor = null;
let simplifyPolyline = null;
let fitCurve = null;
let createMissingTexturePattern = null;
let debugQuadtree = null;
let d3 = null;

// Initialize function to be called from main.js
export function initializeShapeDependencies(deps) {
  growBoundingBox = deps.growBoundingBox;
  lerp = deps.lerp;
  lerpColor = deps.lerpColor;
  uuidToColor = deps.uuidToColor;
  simplifyPolyline = deps.simplifyPolyline;
  fitCurve = deps.fitCurve;
  createMissingTexturePattern = deps.createMissingTexturePattern;
  debugQuadtree = deps.debugQuadtree;
  d3 = deps.d3;
}

class BaseShape {
  constructor(startx, starty) {
    this.startx = startx;
    this.starty = starty;
    this.curves = [];
    this.regions = [];
    this.boundingBox = {
      x: { min: startx, max: starty },
      y: { min: starty, max: starty },
    };
  }
  recalculateBoundingBox() {
    this.boundingBox = undefined;
    for (let curve of this.curves) {
      if (!this.boundingBox) {
        this.boundingBox = curve.bbox();
      }
      growBoundingBox(this.boundingBox, curve.bbox());
    }
  }
  draw(context) {
    let ctx = context.ctx;
    ctx.lineWidth = this.lineWidth;
    ctx.lineCap = "round";

    // Create a repeating pattern for indicating selected shapes
    if (!this.patternCanvas) {
      this.patternCanvas = document.createElement('canvas');
      this.patternCanvas.width = 2;
      this.patternCanvas.height = 2;
      let patternCtx = this.patternCanvas.getContext('2d');
      // Draw the pattern:
      // black,       transparent,
      // transparent, white
      patternCtx.fillStyle = 'black';
      patternCtx.fillRect(0, 0, 1, 1);
      patternCtx.clearRect(1, 0, 1, 1);
      patternCtx.clearRect(0, 1, 1, 1);
      patternCtx.fillStyle = 'white';
      patternCtx.fillRect(1, 1, 1, 1);
    }
    let pattern = ctx.createPattern(this.patternCanvas, 'repeat'); // repeat the pattern across the canvas

    if (this.filled) {
      ctx.beginPath();
      if (this.fillImage && this.fillImage instanceof Element) {
        let pat;
        if (this.fillImage instanceof Element ||
          Object.keys(this.fillImage).length !== 0) {
          pat = ctx.createPattern(this.fillImage, "no-repeat");
        } else {
          pat = createMissingTexturePattern(ctx)
        }
        ctx.fillStyle = pat;
      } else {
        ctx.fillStyle = this.fillStyle;
      }
      if (context.debugColor) {
        ctx.fillStyle = context.debugColor;
      }
      if (this.curves.length > 0) {
        ctx.moveTo(this.curves[0].points[0].x, this.curves[0].points[0].y);
        for (let curve of this.curves) {
          ctx.bezierCurveTo(
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
            curve.points[3].x,
            curve.points[3].y,
          );
        }
      }
      ctx.fill();
      if (context.selected) {
        ctx.fillStyle = pattern
        ctx.fill()
      }
    }
    function drawCurve(curve, selected) {
      ctx.strokeStyle = curve.color;
      ctx.beginPath();
      ctx.moveTo(curve.points[0].x, curve.points[0].y);
      ctx.bezierCurveTo(
        curve.points[1].x,
        curve.points[1].y,
        curve.points[2].x,
        curve.points[2].y,
        curve.points[3].x,
        curve.points[3].y,
      );
      ctx.stroke();
      if (selected) {
        ctx.strokeStyle = pattern
        ctx.stroke()
      }
    }
    if (this.stroked && !context.debugColor) {
      for (let curve of this.curves) {
        drawCurve(curve, context.selected)

        // // Debug, show curve control points
        // ctx.beginPath()
        // ctx.arc(curve.points[1].x,curve.points[1].y, 5, 0, 2*Math.PI)
        // ctx.arc(curve.points[2].x,curve.points[2].y, 5, 0, 2*Math.PI)
        // ctx.arc(curve.points[3].x,curve.points[3].y, 5, 0, 2*Math.PI)
        // ctx.fill()
      }
    }
    if (context.activeCurve && this==context.activeCurve.shape) {
      drawCurve(context.activeCurve.current, true)
    }
    if (context.activeVertex && this==context.activeVertex.shape) {
      const curves = {
        ...context.activeVertex.current.startCurves,
        ...context.activeVertex.current.endCurves
      }
      for (let i in curves) {
        let curve = curves[i]
        drawCurve(curve, true)
      }
      ctx.fillStyle = "#000000aa";
        ctx.beginPath();
        let vertexSize = 15 / context.zoomLevel;
        ctx.rect(
          context.activeVertex.current.point.x - vertexSize / 2,
          context.activeVertex.current.point.y - vertexSize / 2,
          vertexSize,
          vertexSize,
        );
        ctx.fill();
    }
    // Debug, show quadtree
    if (debugQuadtree && this.quadtree && !context.debugColor) {
      this.quadtree.draw(ctx);
    }
  }
  lerpShape(shape2, t) {
    if (this.curves.length == 0) return this;
    let path1 = [
      {
        type: "M",
        x: this.curves[0].points[0].x,
        y: this.curves[0].points[0].y,
      },
    ];
    for (let curve of this.curves) {
      path1.push({
        type: "C",
        x1: curve.points[1].x,
        y1: curve.points[1].y,
        x2: curve.points[2].x,
        y2: curve.points[2].y,
        x: curve.points[3].x,
        y: curve.points[3].y,
      });
    }
    let path2 = [];
    if (shape2.curves.length > 0) {
      path2.push({
        type: "M",
        x: shape2.curves[0].points[0].x,
        y: shape2.curves[0].points[0].y,
      });
      for (let curve of shape2.curves) {
        path2.push({
          type: "C",
          x1: curve.points[1].x,
          y1: curve.points[1].y,
          x2: curve.points[2].x,
          y2: curve.points[2].y,
          x: curve.points[3].x,
          y: curve.points[3].y,
        });
      }
    }
    const interpolator = d3.interpolatePathCommands(path1, path2);
    let current = interpolator(t);
    let curves = [];
    let start = current.shift();
    let { x, y } = start;
    let bezier;
    for (let curve of current) {
      bezier = new Bezier(
        x,
        y,
        curve.x1,
        curve.y1,
        curve.x2,
        curve.y2,
        curve.x,
        curve.y,
      )
      bezier.color = lerpColor(this.strokeStyle, shape2.strokeStyle)
      curves.push(bezier);
      x = curve.x;
      y = curve.y;
    }
    let lineWidth = lerp(this.lineWidth, shape2.lineWidth, t);
    let strokeStyle = lerpColor(
      this.strokeStyle,
      shape2.strokeStyle,
      t,
    );
    let fillStyle;
    if (!this.fillImage) {
      fillStyle = lerpColor(this.fillStyle, shape2.fillStyle, t);
    }
    return new TempShape(
      start.x,
      start.y,
      curves,
      lineWidth,
      this.stroked,
      this.filled,
      strokeStyle,
      fillStyle,
    )
  }
}

class TempShape extends BaseShape {
  constructor(
    startx,
    starty,
    curves,
    lineWidth,
    stroked,
    filled,
    strokeStyle,
    fillStyle,
  ) {
    super(startx, starty);
    this.curves = curves;
    this.lineWidth = lineWidth;
    this.stroked = stroked;
    this.filled = filled;
    this.strokeStyle = strokeStyle;
    this.fillStyle = fillStyle;
    this.inProgress = false;
    this.recalculateBoundingBox();
  }
}

class Shape extends BaseShape {
  constructor(startx, starty, context, parent, uuid = undefined, shapeId = undefined) {
    super(startx, starty);
    this.parent = parent; // Reference to parent Layer (required)
    this.vertices = [];
    this.triangles = [];
    this.fillStyle = context.fillStyle;
    this.fillImage = context.fillImage;
    this.strokeStyle = context.strokeStyle;
    this.lineWidth = context.lineWidth;
    this.filled = context.fillShape;
    this.stroked = context.strokeShape;
    this.quadtree = new Quadtree(
      { x: { min: 0, max: 500 }, y: { min: 0, max: 500 } },
      4,
    );
    if (!uuid) {
      this.idx = uuidv4();
    } else {
      this.idx = uuid;
    }
    if (!shapeId) {
      this.shapeId = uuidv4();
    } else {
      this.shapeId = shapeId;
    }
    this.shapeIndex = 0;  // Default shape version index for tweening
    pointerList[this.idx] = this;
    this.regionIdx = 0;
    this.inProgress = true;

    // Timeline display settings (Phase 3)
    this.showSegment = true  // Show segment bar in timeline
    this.curvesMode = 'hidden'  // 'hidden' | 'minimized' | 'expanded'
    this.curvesHeight = 150  // Height in pixels when curves are expanded
  }
  static fromJSON(json, parent) {
    let fillImage = undefined;
    if (json.fillImage && Object.keys(json.fillImage).length !== 0) {
      let img = new Image();
      img.src = json.fillImage.src
      fillImage = img
    } else {
      fillImage = {}
    }
    const shape = new Shape(
      json.startx,
      json.starty,
      {
        fillStyle: json.fillStyle,
        fillImage: fillImage,
        strokeStyle: json.strokeStyle,
        lineWidth: json.lineWidth,
        fillShape: json.filled,
        strokeShape: json.stroked,
      },
      parent,
      json.idx,
      json.shapeId,
    );
    for (let curve of json.curves) {
      shape.addCurve(Bezier.fromJSON(curve));
    }
    for (let region of json.regions) {
      const curves = [];
      for (let curve of region.curves) {
        curves.push(Bezier.fromJSON(curve));
      }
      shape.regions.push({
        idx: region.idx,
        curves: curves,
        fillStyle: region.fillStyle,
        filled: region.filled,
      });
    }
    // Load shapeIndex if present (for shape tweening)
    if (json.shapeIndex !== undefined) {
      shape.shapeIndex = json.shapeIndex;
    }
    return shape;
  }
  toJSON(randomizeUuid = false) {
    const json = {};
    json.type = "Shape";
    json.startx = this.startx;
    json.starty = this.starty;
    json.fillStyle = this.fillStyle;
    if (this.fillImage instanceof Element) {
      json.fillImage = {
        src: this.fillImage.src
      }
    }
    json.strokeStyle = this.fillStyle;
    json.lineWidth = this.lineWidth;
    json.filled = this.filled;
    json.stroked = this.stroked;
    if (randomizeUuid) {
      json.idx = uuidv4();
    } else {
      json.idx = this.idx;
    }
    json.shapeId = this.shapeId;
    json.shapeIndex = this.shapeIndex;  // For shape tweening
    json.curves = [];
    for (let curve of this.curves) {
      json.curves.push(curve.toJSON(randomizeUuid));
    }
    json.regions = [];
    for (let region of this.regions) {
      const curves = [];
      for (let curve of region.curves) {
        curves.push(curve.toJSON(randomizeUuid));
      }
      json.regions.push({
        idx: region.idx,
        curves: curves,
        fillStyle: region.fillStyle,
        filled: region.filled,
      });
    }
    return json;
  }
  get segmentColor() {
    return uuidToColor(this.idx);
  }
  addCurve(curve) {
    if (curve.color == undefined) {
      curve.color = context.strokeStyle;
    }
    this.curves.push(curve);
    this.quadtree.insert(curve, this.curves.length - 1);
    growBoundingBox(this.boundingBox, curve.bbox());
  }
  addLine(x, y) {
    let lastpoint;
    if (this.curves.length) {
      lastpoint = this.curves[this.curves.length - 1].points[3];
    } else {
      lastpoint = { x: this.startx, y: this.starty };
    }
    let midpoint = { x: (x + lastpoint.x) / 2, y: (y + lastpoint.y) / 2 };
    let curve = new Bezier(
      lastpoint.x,
      lastpoint.y,
      midpoint.x,
      midpoint.y,
      midpoint.x,
      midpoint.y,
      x,
      y,
    );
    curve.color = context.strokeStyle;
    this.quadtree.insert(curve, this.curves.length - 1);
    this.curves.push(curve);
  }
  bbox() {
    return this.boundingBox;
  }
  clear() {
    this.curves = [];
    this.quadtree.clear();
  }
  copy(idx) {
    let newShape = new Shape(
      this.startx,
      this.starty,
      {},
      this.parent,
      idx.slice(0, 8) + this.idx.slice(8),
      this.shapeId,
    );
    newShape.startx = this.startx;
    newShape.starty = this.starty;
    for (let curve of this.curves) {
      let newCurve = new Bezier(
        curve.points[0].x,
        curve.points[0].y,
        curve.points[1].x,
        curve.points[1].y,
        curve.points[2].x,
        curve.points[2].y,
        curve.points[3].x,
        curve.points[3].y,
      );
      newCurve.color = curve.color;
      newShape.addCurve(newCurve);
    }
    // TODO
    // for (let vertex of this.vertices) {

    // }
    newShape.updateVertices();
    newShape.fillStyle = this.fillStyle;
    if (this.fillImage instanceof Element) {
      newShape.fillImage = this.fillImage.cloneNode(true)
    } else {
      newShape.fillImage = this.fillImage;
    }
    newShape.strokeStyle = this.strokeStyle;
    newShape.lineWidth = this.lineWidth;
    newShape.filled = this.filled;
    newShape.stroked = this.stroked;

    return newShape;
  }
  fromPoints(points, error = 30) {
    console.log(error);
    this.curves = [];
    let curves = fitCurve.fitCurve(points, error);
    for (let curve of curves) {
      let bezier = new Bezier(
        curve[0][0],
        curve[0][1],
        curve[1][0],
        curve[1][1],
        curve[2][0],
        curve[2][1],
        curve[3][0],
        curve[3][1],
      );
      this.curves.push(bezier);
      this.quadtree.insert(bezier, this.curves.length - 1);
    }
    return this;
  }
  simplify(mode = "corners") {
    this.quadtree.clear();
    this.inProgress = false;
    // Mode can be corners, smooth or auto
    if (mode == "corners") {
      let points = [{ x: this.startx, y: this.starty }];
      for (let curve of this.curves) {
        points.push(curve.points[3]);
      }
      // points = points.concat(this.curves)
      let newpoints = simplifyPolyline(points, 10, false);
      this.curves = [];
      let lastpoint = newpoints.shift();
      let midpoint;
      for (let point of newpoints) {
        midpoint = {
          x: (lastpoint.x + point.x) / 2,
          y: (lastpoint.y + point.y) / 2,
        };
        let bezier = new Bezier(
          lastpoint.x,
          lastpoint.y,
          midpoint.x,
          midpoint.y,
          midpoint.x,
          midpoint.y,
          point.x,
          point.y,
        );
        this.curves.push(bezier);
        this.quadtree.insert(bezier, this.curves.length - 1);
        lastpoint = point;
      }
    } else if (mode == "smooth") {
      let error = 30;
      let points = [[this.startx, this.starty]];
      for (let curve of this.curves) {
        points.push([curve.points[3].x, curve.points[3].y]);
      }
      this.fromPoints(points, error);
    } else if (mode == "verbatim") {
      // Just keep existing shape
    }
    let epsilon = 0.01;
    let newCurves = [];
    let intersectMap = {};
    for (let i = 0; i < this.curves.length - 1; i++) {
      // for (let j=i+1; j<this.curves.length; j++) {
      for (let j of this.quadtree.query(this.curves[i].bbox())) {
        if (i >= j) continue;
        let intersects = this.curves[i].intersects(this.curves[j]);
        if (intersects.length) {
          intersectMap[i] ||= [];
          intersectMap[j] ||= [];
          for (let intersect of intersects) {
            let [t1, t2] = intersect.split("/");
            intersectMap[i].push(parseFloat(t1));
            intersectMap[j].push(parseFloat(t2));
          }
        }
      }
    }
    for (let lst in intersectMap) {
      for (let i = 1; i < intersectMap[lst].length; i++) {
        if (
          Math.abs(intersectMap[lst][i] - intersectMap[lst][i - 1]) < epsilon
        ) {
          intersectMap[lst].splice(i, 1);
          i--;
        }
      }
    }
    for (let i = this.curves.length - 1; i >= 0; i--) {
      if (i in intersectMap) {
        intersectMap[i].sort().reverse();
        let remainingFraction = 1;
        let remainingCurve = this.curves[i];
        for (let t of intersectMap[i]) {
          let split = remainingCurve.split(t / remainingFraction);
          remainingFraction = t;
          newCurves.push(split.right);
          remainingCurve = split.left;
        }
        newCurves.push(remainingCurve);
      } else {
        newCurves.push(this.curves[i]);
      }
    }
    for (let curve of newCurves) {
      curve.color = context.strokeStyle;
    }
    newCurves.reverse();
    this.curves = newCurves;
  }
  update() {
    this.recalculateBoundingBox();
    this.updateVertices();
    if (this.curves.length) {
      this.startx = this.curves[0].points[0].x;
      this.starty = this.curves[0].points[0].y;
    }
    return [this];
  }
  getClockwiseCurves(point, otherPoints) {
    // Returns array of {x, y, idx, angle}

    let points = [];
    for (let point of otherPoints) {
      points.push({ ...this.vertices[point].point, idx: point });
    }
    // Add an angle property to each point using tan(angle) = y/x
    const angles = points.map(({ x, y, idx }) => {
      return {
        x,
        y,
        idx,
        angle: (Math.atan2(y - point.y, x - point.x) * 180) / Math.PI,
      };
    });
    // Sort your points by angle
    const pointsSorted = angles.sort((a, b) => a.angle - b.angle);
    return pointsSorted;
  }
  translate(x, y) {
    this.quadtree.clear()
    let j=0;
    for (let curve of this.curves) {
      for (let i in curve.points) {
        const point = curve.points[i];
        curve.points[i] = { x: point.x + x, y: point.y + y };
      }
      this.quadtree.insert(curve, j)
      j++;
    }
    this.update();
  }
  updateVertices() {
    this.vertices = [];
    let utils = Bezier.getUtils();
    let epsilon = 1.5; // big epsilon whoa
    let tooClose;
    let i = 0;

    let region = {
      idx: `${this.idx}-r${this.regionIdx++}`,
      curves: [],
      fillStyle: context.fillStyle,
      filled: context.fillShape,
    };
    pointerList[region.idx] = region;
    this.regions = [region];
    for (let curve of this.curves) {
      this.regions[0].curves.push(curve);
    }
    if (this.regions[0].curves.length) {
      if (
        utils.dist(
          this.regions[0].curves[0].points[0],
          this.regions[0].curves[this.regions[0].curves.length - 1].points[3],
        ) < epsilon
      ) {
        this.regions[0].filled = true;
      }
    }

    // Generate vertices
    for (let curve of this.curves) {
      for (let index of [0, 3]) {
        tooClose = false;
        for (let vertex of this.vertices) {
          if (utils.dist(curve.points[index], vertex.point) < epsilon) {
            tooClose = true;
            vertex[["startCurves", , , "endCurves"][index]][i] = curve;
            break;
          }
        }
        if (!tooClose) {
          if (index == 0) {
            this.vertices.push({
              point: curve.points[index],
              startCurves: { [i]: curve },
              endCurves: {},
            });
          } else {
            this.vertices.push({
              point: curve.points[index],
              startCurves: {},
              endCurves: { [i]: curve },
            });
          }
        }
      }
      i++;
    }

    let shapes = [this];
    this.vertices.forEach((vertex, i) => {
      for (let i = 0; i < Math.min(10, this.regions.length); i++) {
        let region = this.regions[i];
        let regionVertexCurves = [];
        let vertexCurves = { ...vertex.startCurves, ...vertex.endCurves };
        if (Object.keys(vertexCurves).length == 1) {
          // endpoint
          continue;
        } else if (Object.keys(vertexCurves).length == 2) {
          // path vertex, don't need to do anything
          continue;
        } else if (Object.keys(vertexCurves).length == 3) {
          // T junction. Region doesn't change but might need to update curves?
          // Skip for now.
          continue;
        } else if (Object.keys(vertexCurves).length == 4) {
          // Intersection, split region in 2
          for (let i in vertexCurves) {
            let curve = vertexCurves[i];
            if (region.curves.includes(curve)) {
              regionVertexCurves.push(curve);
            }
          }
          let start = region.curves.indexOf(regionVertexCurves[1]);
          let end = region.curves.indexOf(regionVertexCurves[3]);
          if (end > start) {
            let newRegion = {
              idx: `${this.idx}-r${this.regionIdx++}`, // TODO: generate this deterministically so that undo/redo works
              curves: region.curves.splice(start, end - start),
              fillStyle: region.fillStyle,
              filled: true,
            };
            pointerList[newRegion.idx] = newRegion;
            this.regions.push(newRegion);
          }
        } else {
          // not sure how to handle vertices with more than 4 curves
          console.log(
            `Unexpected vertex with ${Object.keys(vertexCurves).length} curves!`,
          );
        }
      }
    });
  }
}

export { BaseShape, TempShape, Shape };

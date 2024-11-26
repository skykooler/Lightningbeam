const { invoke } = window.__TAURI__.core;
import * as fitCurve from '/fit-curve.js';
import { Bezier } from "/bezier.js";
import { Quadtree } from './quadtree.js';
const { writeTextFile: writeTextFile, readTextFile: readTextFile }=  window.__TAURI__.fs;
const { open: openFileDialog, save: saveFileDialog, message: messageDialog } = window.__TAURI__.dialog;
const { documentDir, join } = window.__TAURI__.path;

let simplifyPolyline = simplify

let greetInputEl;
let greetMsgEl;
let rootPane;

let canvases = [];

let mode = "draw"

let minSegmentSize = 5;
let maxSmoothAngle = 0.6;

let undoStack = [];
let redoStack = [];

let layoutElements = []

let minFileVersion = "1.0"
let maxFileVersion = "2.0"


let tools = {
  select: {
    icon: "/assets/select.svg",
    properties: {}

  },
  transform: {
    icon: "/assets/transform.svg",
    properties: {}

  },
  draw: {
    icon: "/assets/draw.svg",
    properties: {
      "lineWidth": {
        type: "number",
        label: "Line Width"
      },
      "simplifyMode": {
        type: "enum",
        options: ["corners", "smooth"], // "auto"],
        label: "Line Mode"
      },
      "fillShape": {
        type: "boolean",
        label: "Fill Shape"
      }
    }
  },
  rectangle: {
    icon: "/assets/rectangle.svg",
    properties: {}
  },
  ellipse: {
    icon: "assets/ellipse.svg",
    properties: {}
  },
  paint_bucket: {
    icon: "/assets/paint_bucket.svg",
    properties: {}
  }
}

let mouseEvent;

let context = {
  mouseDown: false,
  swatches: [
    "#000000",
    "#FFFFFF",
    "#FF0000",
    "#FFFF00",
    "#00FF00",
    "#00FFFF",
    "#0000FF",
    "#FF00FF",
  ],
  lineWidth: 5,
  simplifyMode: "smooth",
  fillShape: true,
  strokeShape: true,
  dragging: false,
  selectionRect: undefined,
  selection: [],
}

let config = {
  shortcuts: {
    playAnimation: " ",
    // undo: "<ctrl>+z"
    undo: "z",
    redo: "Z",
    save: "s",
    open: "o"
  }
}

// Pointers to all objects
let pointerList = {}
// Keeping track of initial values of variables when we edit them continuously
let startProps = {}

let actions = {
  addShape: {
    create: (parent, shape) => {
      redoStack.length = 0; // Clear redo stack
      let serializableCurves = []
      for (let curve of shape.curves) {
        serializableCurves.push({ points: curve.points, color: curve.color })
      }
      let action = {
        parent: parent.idx,
        curves: serializableCurves,
        startx: shape.startx,
        starty: shape.starty,
        uuid: uuidv4()
      }
      undoStack.push({name: "addShape", action: action})
      actions.addShape.execute(action)
    },
    execute: (action) => {
      let object = pointerList[action.parent]
      let curvesList = action.curves
      let shape = new Shape(action.startx, action.starty, context, action.uuid)
      for (let curve of curvesList) {
        shape.addCurve(new Bezier(
          curve.points[0].x, curve.points[0].y,
          curve.points[1].x, curve.points[1].y,
          curve.points[2].x, curve.points[2].y,
          curve.points[3].x, curve.points[3].y
        ).setColor(curve.color))
      }
      shape.update()
      object.addShape(shape)
    },
    rollback: (action) => {
      let object = pointerList[action.parent]
      let shape = pointerList[action.uuid]
      object.removeShape(shape)
      delete pointerList[action.uuid]
    }
  },
  editShape: {
    create: (shape, newCurves) => {
      redoStack.length = 0; // Clear redo stack
      let serializableNewCurves = []
      for (let curve of newCurves) {
        serializableNewCurves.push({ points: curve.points, color: curve.color })
      }
      let serializableOldCurves = []
      for (let curve of shape.curves) {
        serializableOldCurves.push({ points: curve.points })
      }
      let action = {
        shape: shape.idx,
        oldCurves: serializableOldCurves,
        newCurves: serializableNewCurves
      }
      undoStack.push({name: "editShape", action: action})
      actions.editShape.execute(action)

    },
    execute: (action) => {
      let shape = pointerList[action.shape]
      let curvesList = action.newCurves
      shape.curves = []
      for (let curve of curvesList) {
        shape.addCurve(new Bezier(
          curve.points[0].x, curve.points[0].y,
          curve.points[1].x, curve.points[1].y,
          curve.points[2].x, curve.points[2].y,
          curve.points[3].x, curve.points[3].y
        ).setColor(curve.color))
      }
      shape.update()
    },
    rollback: (action) => {
      let shape = pointerList[action.shape]
      let curvesList = action.oldCurves
      shape.curves = []
      for (let curve of curvesList) {
        shape.addCurve(new Bezier(
          curve.points[0].x, curve.points[0].y,
          curve.points[1].x, curve.points[1].y,
          curve.points[2].x, curve.points[2].y,
          curve.points[3].x, curve.points[3].y
        ).setColor(curve.color))
      }
      shape.update()
    }
  },
  colorRegion: {
    create: (region, color) => {
      redoStack.length = 0; // Clear redo stack
      let action = {
        region: region.idx,
        oldColor: region.fillStyle,
        newColor: color
      }
      undoStack.push({name: "colorRegion", action: action})
      actions.colorRegion.execute(action)
    },
    execute: (action) => {
      let region = pointerList[action.region]
      region.fillStyle = action.newColor
    },
    rollback: (action) => {
      let region = pointerList[action.region]
      region.fillStyle = action.oldColor
    }
  },
  addImageObject: {
    create: (x, y, img, parent) => {
      redoStack.length = 0; // Clear redo stack
      let action = {
        shapeUuid: uuidv4(),
        objectUuid: uuidv4(),
        x: x,
        y: y,
        width: img.width,
        height: img.height,
        ix: img.ix,
        img: img.idx,
        parent: parent.idx
      }
      undoStack.push({name: "addImageObject", action: action})
      actions.addImageObject.execute(action)
    },
    execute: (action) => {
      let imageObject = new GraphicsObject(action.objectUuid)
      let img = pointerList[action.img] 
      let ct = {
        ...context,
        fillImage: img,
        strokeShape: false,
      }
      let imageShape = new Shape(0, 0, ct, action.shapeUuid)
      imageShape.addLine(action.width, 0)
      imageShape.addLine(action.width, action.height)
      imageShape.addLine(0, action.height)
      imageShape.addLine(0, 0)
      imageShape.update()
      imageObject.addShape(imageShape)
      let parent = pointerList[action.parent]
      parent.addObject(
        imageObject,
        action.x-action.width/2 + (20*action.ix),
        action.y-action.height/2 + (20*action.ix)
      )
    },
    rollback: (action) => {
      let shape = pointerList[action.shapeUuid]
      let object = pointerList[action.objectUuid]
      let parent = pointerList[action.parent]
      object.removeShape(shape)
      delete pointerList[action.shapeUuid]
      parent.removeChild(object)
      delete pointerList[action.objectUuid]
      let selectIndex = context.selection.indexOf(object)
      if (selectIndex >= 0) {
        context.selection.splice(selectIndex, 1)
      }
    }
  },
  editFrame: {
    create: (frame) => {
      redoStack.length = 0; // Clear redo stack
      let action = {
        newState: structuredClone(frame.keys),
        oldState: startProps[frame.idx],
        frame: frame.idx
      }
      undoStack.push({name: "editFrame", action: action})
      actions.editFrame.execute(action)
    },
    execute: (action) => {
      let frame = pointerList[action.frame]
      frame.keys = structuredClone(action.newState)
    },
    rollback: (action) => {
      let frame = pointerList[action.frame]
      frame.keys = structuredClone(action.oldState)
    }
  },
}

function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, c =>
    (+c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> +c / 4).toString(16)
  );
}
function vectorDist(a, b) {
  return Math.sqrt((a.x-b.x)*(a.x-b.x) + (a.y-b.y)*(a.y-b.y))
}

function getMousePos(canvas, evt) {
  var rect = canvas.getBoundingClientRect();
  return {
    x: evt.clientX - rect.left,
    y: evt.clientY - rect.top
  };
}

function getProperty(context, path) {
  let pointer = context;
  let pathComponents = path.split('.')
  for (let component of pathComponents) {
    pointer = pointer[component]
  }
  return pointer
}

function setProperty(context, path, value) {
  let pointer = context;
  let pathComponents = path.split('.')
  let finalComponent = pathComponents.pop()
  for (let component of pathComponents) {
    pointer = pointer[component]
  }
  pointer[finalComponent] = value
}

function selectCurve(context, mouse) {
  let mouseTolerance = 15;
  let closestDist = mouseTolerance;
  let closestCurve = undefined
  let closestShape = undefined
  for (let shape of context.activeObject.currentFrame.shapes) {
    if (mouse.x > shape.boundingBox.x.min - mouseTolerance &&
        mouse.x < shape.boundingBox.x.max + mouseTolerance &&
        mouse.y > shape.boundingBox.y.min - mouseTolerance &&
        mouse.y < shape.boundingBox.y.max + mouseTolerance) {
      for (let curve of shape.curves) {
        let dist = vectorDist(mouse, curve.project(mouse))
        if (dist <= closestDist ) {
          closestDist = dist
          closestCurve = curve
          closestShape = shape
        }
      }
      }
    }
      if (closestCurve) {
        return {curve:closestCurve, shape:closestShape}
      } else {
        return undefined
  }
}
function selectVertex(context, mouse) {
  let mouseTolerance = 15;
  let closestDist = mouseTolerance;
  let closestVertex = undefined
  let closestShape = undefined
  for (let shape of context.activeObject.currentFrame.shapes) {
    if (mouse.x > shape.boundingBox.x.min - mouseTolerance &&
        mouse.x < shape.boundingBox.x.max + mouseTolerance &&
        mouse.y > shape.boundingBox.y.min - mouseTolerance &&
        mouse.y < shape.boundingBox.y.max + mouseTolerance) {
      for (let vertex of shape.vertices) {
        let dist = vectorDist(mouse, vertex.point)
        if (dist <= closestDist ) {
          closestDist = dist
          closestVertex = vertex
          closestShape = shape
        }
      }
      }
    }
      if (closestVertex) {
        return {vertex:closestVertex, shape:closestShape}
      } else {
        return undefined
  }
}

function moldCurve(curve, mouse, oldmouse) {
  let diff = {x: mouse.x - oldmouse.x, y: mouse.y - oldmouse.y}
  let p = curve.project(mouse)
  let min_influence = 0.1
  const CP1 = {
    x: curve.points[1].x + diff.x*(1-p.t)*2,
    y: curve.points[1].y + diff.y*(1-p.t)*2
  }
  const CP2 = {
    x: curve.points[2].x + diff.x*(p.t)*2,
    y: curve.points[2].y + diff.y*(p.t)*2
  }
  return new Bezier(curve.points[0], CP1, CP2, curve.points[3])
  // return curve
}

function moldCurveMath(curve, mouse) {
  let interpolated = true

  let p = curve.project({x: mouse.x, y: mouse.y})

  let t1 = p.t;
  let struts = curve.getStrutPoints(t1);
  let m = {
      t: p.t,
      B: p,
      e1: struts[7],
      e2: struts[8]
  };
  m.d1 = { x: m.e1.x - m.B.x, y: m.e1.y - m.B.y};
  m.d2 = { x: m.e2.x - m.B.x, y: m.e2.y - m.B.y};

  const S = curve.points[0],
        E = curve.points[curve.order],
        {B, t, e1, e2} = m,
        org = curve.getABC(t, B),
        nB = mouse,
        d1 = { x: e1.x - B.x, y: e1.y - B.y },
        d2 = { x: e2.x - B.x, y: e2.y - B.y },
        ne1 = { x: nB.x + d1.x, y: nB.y + d1.y },
        ne2 = { x: nB.x + d2.x, y: nB.y + d2.y },
        {A, C} = curve.getABC(t, nB),
        // The cubic case requires us to derive two control points,
        // which we'll do in a separate function to keep the code
        // at least somewhat manageable.
        {v1, v2, C1, C2} = deriveControlPoints(S, A, E, ne1, ne2, t);

  // if (interpolated) {
    // For the last example, we need to show what the "ideal" curve
    // looks like, in addition to the one we actually get when we
    // rely on the B we picked with the `t` value and e1/e2 points
    // that point B had...
    const ideal = getIdealisedCurve(S, nB, E);
    let idealCurve = new Bezier(ideal.S, ideal.C1, ideal.C2, ideal.E);
  // }
  let molded = new Bezier(S,C1,C2,E);

  let falloff = 100

  let d = Bezier.getUtils().dist(ideal.B, p);
  let t2 = Math.min(falloff, d) / falloff;
  let iC1 = {
      x: (1-t2) * molded.points[1].x + t2 * idealCurve.points[1].x,
      y: (1-t2) * molded.points[1].y + t2 * idealCurve.points[1].y
  };
  let iC2 = {
      x: (1-t2) * molded.points[2].x + t2 * idealCurve.points[2].x,
      y: (1-t2) * molded.points[2].y + t2 * idealCurve.points[2].y
  };
  let interpolatedCurve = new Bezier(molded.points[0], iC1, iC2, molded.points[3]);

  return interpolatedCurve
}

function deriveControlPoints(S, A, E, e1, e2, t) {
  // Deriving the control points is effectively "doing what
  // we talk about in the section", in code:

  const v1 = {
      x: A.x - (A.x - e1.x)/(1-t),
      y: A.y - (A.y - e1.y)/(1-t)
  };
  const v2 = {
      x: A.x - (A.x - e2.x)/t,
      y: A.y - (A.y - e2.y)/t
  };

  const C1 = {
      x: S.x + (v1.x - S.x) / t,
      y: S.y + (v1.y - S.y) / t
  };
  const C2 = {
      x: E.x + (v2.x - E.x) / (1-t),
      y: E.y + (v2.y - E.y) / (1-t)
  };

  return {v1, v2, C1, C2};
}

function getIdealisedCurve(p1, p2, p3) {
  // This "reruns" the curve composition, but with a `t` value
  // that is unrelated to the actual point B we picked, instead
  // using whatever the appropriate `t` value would be if we were
  // trying to fit a circular arc, as per earlier in the section.
  const utils = Bezier.getUtils()
  const c = utils.getccenter(p1, p2, p3),
        d1 = utils.dist(p1, p2),
        d2 = utils.dist(p3, p2),
        t = d1 / (d1 + d2),
        { A, B, C, S, E } = Bezier.getABC(3, p1, p2, p3, t),
        angle = (Math.atan2(E.y-S.y, E.x-S.x) - Math.atan2(B.y-S.y, B.x-S.x) + utils.TAU) % utils.TAU,
        bc = (angle < 0 || angle > utils.PI ? -1 : 1) * utils.dist(S, E)/3,
        de1 = t * bc,
        de2 = (1-t) * bc,
        tangent = [
          { x: B.x - 10 * (B.y-c.y), y: B.y + 10 * (B.x-c.x) },
          { x: B.x + 10 * (B.y-c.y), y: B.y - 10 * (B.x-c.x) }
        ],
        tlength = utils.dist(tangent[0], tangent[1]),
        dx = (tangent[1].x - tangent[0].x)/tlength,
        dy = (tangent[1].y - tangent[0].y)/tlength,
        e1 = { x: B.x + de1 * dx, y: B.y + de1 * dy},
        e2 = { x: B.x - de2 * dx, y: B.y - de2 * dy },
        {v1, v2, C1, C2} = deriveControlPoints(S, A, E, e1, e2, t);

  return {A,B,C,S,E,e1,e2,v1,v2,C1,C2};
}

function growBoundingBox(bboxa, bboxb) {
  bboxa.x.min = Math.min(bboxa.x.min, bboxb.x.min)
  bboxa.y.min = Math.min(bboxa.y.min, bboxb.y.min)
  bboxa.x.max = Math.max(bboxa.x.max, bboxb.x.max)
  bboxa.y.max = Math.max(bboxa.y.max, bboxb.y.max)
}

function regionToBbox(region) {
  return {
    x: {min: Math.min(region.x1, region.x2), max: Math.max(region.x1, region.x2)},
    y: {min: Math.min(region.y1, region.y2), max: Math.max(region.y1, region.y2)}
  }
}

function hitTest(candidate, object) {
  let bbox = object.bbox()
  if (candidate.x.min) {
    // We're checking a bounding box
    if (candidate.x.min < bbox.x.max + object.x && candidate.x.max > bbox.x.min + object.x &&
      candidate.y.min < bbox.y.max + object.y && candidate.y.max > bbox.y.min + object.y) {
        return true;
    } else {
      return false;
    }
  } else {
    // We're checking a point
    if (candidate.x > bbox.x.min + object.x &&
      candidate.x < bbox.x.max + object.x &&
      candidate.y > bbox.y.min + object.y &&
      candidate.y < bbox.y.max + object.y) {
        return true;
    } else {
      return false
    }
  }
}

function undo() {
  let action = undoStack.pop()
  if (action) {
    actions[action.name].rollback(action.action)
    redoStack.push(action)
    updateUI()
  } else {
    console.log("No actions to undo")
  }
}

function redo() {
  let action = redoStack.pop()
  if (action) {
    actions[action.name].execute(action.action)
    undoStack.push(action)
    updateUI()
  } else {
    console.log("No actions to redo")
  }
}


class Frame {
  constructor(uuid) {
    this.keys = {}
    this.shapes = []
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    pointerList[this.idx] = this
  }
  saveState() {
    startProps[this.idx] = structuredClone(this.keys)
  }
}

class Layer {
  constructor(uuid) {
    this.frames = [new Frame()]
    this.children = []
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    pointerList[this.idx] = this
  }
}

class Shape {
  constructor(startx, starty, context, uuid=undefined) {
    this.startx = startx;
    this.starty = starty;
    this.curves = [];
    this.vertices = [];
    this.triangles = [];
    this.regions = [];
    this.fillStyle = context.fillStyle;
    this.fillImage = context.fillImage;
    this.strokeStyle = context.strokeStyle;
    this.lineWidth = context.lineWidth
    this.filled = context.fillShape;
    this.stroked = context.strokeShape;
    this.boundingBox = {
      x: {min: startx, max: starty},
      y: {min: starty, max: starty}
    }
    this.quadtree = new Quadtree({x: {min: 0, max: 500}, y: {min: 0, max: 500}}, 4)
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    pointerList[this.idx] = this
  }
  addCurve(curve) {
    this.curves.push(curve)
    this.quadtree.insert(curve, this.curves.length - 1)
    growBoundingBox(this.boundingBox, curve.bbox())
  }
  addLine(x, y) {
    let lastpoint;
    if (this.curves.length) {
      lastpoint = this.curves[this.curves.length - 1].points[3]
    } else {
      lastpoint = {x: this.startx, y: this.starty}
    }
    let midpoint = {x: (x + lastpoint.x) / 2, y: (y + lastpoint.y) / 2}
    let curve = new Bezier(lastpoint.x, lastpoint.y,
                           midpoint.x, midpoint.y,
                           midpoint.x, midpoint.y,
                           x, y)
    curve.color = context.strokeStyle
    this.curves.push(curve)
  }
  clear() {
    this.curves = []
  }
  recalculateBoundingBox() {
    for (let curve of this.curves) {
      growBoundingBox(this.boundingBox, curve.bbox())
    }
  }
  simplify(mode="corners") {
    this.quadtree.clear()
    // Mode can be corners, smooth or auto
    if (mode=="corners") {
      let points = [{x: this.startx, y: this.starty}]
      for (let curve of this.curves) {
        points.push(curve.points[3])
      }
      // points = points.concat(this.curves)
      let newpoints = simplifyPolyline(points, 10, false)
      this.curves = []
      let lastpoint = newpoints.shift()
      let midpoint
      for (let point of newpoints) {
        midpoint = {x: (lastpoint.x+point.x)/2, y: (lastpoint.y+point.y)/2}
        let bezier = new Bezier(lastpoint.x, lastpoint.y,
                                midpoint.x, midpoint.y,
                                midpoint.x,midpoint.y,
                                point.x,point.y)
        this.curves.push(bezier)
        this.quadtree.insert(bezier, this.curves.length - 1)
        lastpoint = point
      }
    } else if (mode=="smooth") {
      let error = 30;
      let points = [[this.startx, this.starty]]
      for (let curve of this.curves) {
        points.push([curve.points[3].x, curve.points[3].y])
      }
      this.curves = []
      let curves = fitCurve.fitCurve(points, error)
      for (let curve of curves) {
        let bezier = new Bezier(curve[0][0], curve[0][1],
                                curve[1][0],curve[1][1],
                                curve[2][0], curve[2][1],
                                curve[3][0], curve[3][1])
        this.curves.push(bezier)
        this.quadtree.insert(bezier, this.curves.length - 1)

      }
    }
    let epsilon = 0.01
    let newCurves = []
    let intersectMap = {}
    for (let i=0; i<this.curves.length-1; i++) {
      console.log(this.quadtree.query(this.curves[i].bbox()))
      // for (let j=i+1; j<this.curves.length; j++) {
      for (let j of this.quadtree.query(this.curves[i].bbox())) {
        if (i == j) continue;
        let intersects = this.curves[i].intersects(this.curves[j])
        if (intersects.length) {
          intersectMap[i] ||= []
          intersectMap[j] ||= []
          for(let intersect of intersects) {
            let [t1, t2] = intersect.split("/")
            intersectMap[i].push(parseFloat(t1))
            intersectMap[j].push(parseFloat(t2))
          }
        }
      }
    }
    for (let lst in intersectMap) {
      for (let i=1; i<intersectMap[lst].length; i++) {
        if (Math.abs(intersectMap[lst][i] - intersectMap[lst][i-1]) < epsilon) {
          intersectMap[lst].splice(i,1)
          i--
        }
      }
    }
    for (let i=this.curves.length-1; i>=0; i--) {
      if (i in intersectMap) {
        intersectMap[i].sort().reverse()
        let remainingFraction = 1
        let remainingCurve = this.curves[i]
        for (let t of intersectMap[i]) {
          let split = remainingCurve.split(t / remainingFraction)
          remainingFraction = t
          newCurves.push(split.right)
          remainingCurve = split.left
        }
        newCurves.push(remainingCurve)

      } else {
        newCurves.push(this.curves[i])
      }
    }
    for (let curve of newCurves) {
      curve.color = context.strokeStyle
    }
    newCurves.reverse()
    this.curves = newCurves
  }
  update() {
    this.recalculateBoundingBox()
    this.updateVertices()
    if (this.curves.length) {
      this.startx = this.curves[0].points[0].x
      this.starty = this.curves[0].points[0].y
    }
  }
  getClockwiseCurves(point, otherPoints) {
    // Returns array of {x, y, idx, angle}

    let points = []
    for (let point of otherPoints) {
      points.push({...this.vertices[point].point, idx: point})
    }
    // Add an angle property to each point using tan(angle) = y/x
    const angles = points.map(({ x, y, idx }) => {
      return { x, y, idx, angle: Math.atan2(y - point.y, x - point.x) * 180 / Math.PI };
    });
    // Sort your points by angle
    const pointsSorted = angles.sort((a, b) => a.angle - b.angle);
    return pointsSorted
  }
  updateVertices() {
    this.vertices = []
    let utils = Bezier.getUtils()
    let epsilon = 1.5 // big epsilon whoa
    let tooClose;
    let i = 0;


    let region = {idx: uuidv4(), curves: [], fillStyle: undefined, filled: false}
    pointerList[region.idx] = region
    this.regions = [region]
    for (let curve of this.curves) {
      this.regions[0].curves.push(curve)
    }
    if (this.regions[0].curves.length) {
      if (utils.dist(
        this.regions[0].curves[0].points[0],
        this.regions[0].curves[this.regions[0].curves.length - 1].points[3]
      ) < epsilon) {
        this.regions[0].filled = true
      }
    }

    // Generate vertices
    for (let curve of this.curves) {
      for (let index of [0, 3]) {
        tooClose = false
        for (let vertex of this.vertices) {
          if (utils.dist(curve.points[index], vertex.point) < epsilon){
            tooClose = true;
            vertex[["startCurves",,,"endCurves"][index]][i] = curve
            break
          }
        }
        if (!tooClose) {
          if (index==0) {
            this.vertices.push({
              point:curve.points[index],
              startCurves: {[i]:curve},
              endCurves: {}
            })
          } else {
            this.vertices.push({
              point:curve.points[index],
              startCurves: {},
              endCurves: {[i]:curve}
            })
          }
        }
      }
      i++;
    }

    this.vertices.forEach((vertex, i) => {
      console.log(i)
      for (let i=0; i<Math.min(10,this.regions.length); i++) {
        let region = this.regions[i]
        let regionVertexCurves = []
        let vertexCurves = {...vertex.startCurves, ...vertex.endCurves}
        if (Object.keys(vertexCurves).length==1) {
          // endpoint
          continue;
        } else if (Object.keys(vertexCurves).length==2) {
          // path vertex, don't need to do anything
          continue;
        } else if (Object.keys(vertexCurves).length==3) {
          // T junction. Region doesn't change but might need to update curves?
          // Skip for now.
          continue;
        } else if (Object.keys(vertexCurves).length==4) {
          // Intersection, split region in 2
          for (let i in vertexCurves) {
            let curve = vertexCurves[i]
            if (region.curves.includes(curve)) {
              regionVertexCurves.push(curve)
            }
          }
          let start = region.curves.indexOf(regionVertexCurves[1])
          let end = region.curves.indexOf(regionVertexCurves[3])
          if (end > start) {
            let newRegion = {
              idx: uuidv4(), // TODO: generate this deterministically so that undo/redo works
              curves: region.curves.splice(start, end - start),
              fillStyle: region.fillStyle,
              filled: true
            }
            pointerList[newRegion.idx] = newRegion
            this.regions.push(newRegion)  
          }
        } else {
          // not sure how to handle vertices with more than 4 curves
          console.log(`Unexpected vertex with ${Object.keys(vertexCurves).length} curves!`)
        }
      }
    })
  }
  draw(context) {
    let ctx = context.ctx;
    ctx.lineWidth = this.lineWidth
    ctx.lineCap = "round"
    for (let region of this.regions) {
      // if (region.filled) continue;
      if (region.fillStyle && region.filled) {
        // ctx.fillStyle = region.fill
        if (region.fillImage) {
          let pat = ctx.createPattern(region.fillImage, "no-repeat")
          ctx.fillStyle = pat
        } else {
          ctx.fillStyle = region.fillStyle
        }
        ctx.beginPath()
        for (let curve of region.curves) {
          ctx.lineTo(curve.points[0].x, curve.points[0].y)
          ctx.bezierCurveTo(curve.points[1].x, curve.points[1].y,
                            curve.points[2].x, curve.points[2].y,
                            curve.points[3].x, curve.points[3].y)
        }
        ctx.fill()
      }
    }
    for (let curve of this.curves) {
      ctx.strokeStyle = curve.color
      ctx.beginPath()
      ctx.moveTo(curve.points[0].x, curve.points[0].y)
      ctx.bezierCurveTo(curve.points[1].x, curve.points[1].y,
                        curve.points[2].x, curve.points[2].y,
                        curve.points[3].x, curve.points[3].y)
      ctx.stroke()

      // Debug, show curve endpoints
      // ctx.beginPath()
      // ctx.arc(curve.points[3].x,curve.points[3].y, 3, 0, 2*Math.PI)
      // ctx.fill()
    }
    // Debug, show quadtree
    // this.quadtree.draw(ctx)

  }
}

class GraphicsObject {
  constructor(uuid) {
    this.x = 0;
    this.y = 0;
    this.rotation = 0; // in radians
    this.scale = 1;
    if (!uuid) {
      this.idx = uuidv4()
    } else {
      this.idx = uuid
    }
    pointerList[this.idx] = this

    this.currentFrameNum = 0;
    this.currentLayer = 0;
    this.layers = [new Layer()]
    // this.children = []

    this.shapes = []
  }
  get activeLayer() {
    return this.layers[this.currentLayer]
  }
  get children() {
    return this.layers[this.currentLayer].children
  }
  get currentFrame() {
    return this.layers[this.currentLayer].frames[this.currentFrameNum]
  }
  get maxFrame() {
    let maxFrames = []
    for (let layer of this.layers) {
      maxFrames.push(layer.frames.length)
    }
    return Math.max(maxFrames)
  }
  bbox() {
    let bbox;
    if (this.currentFrame.shapes.length > 0) {
      bbox = this.currentFrame.shapes[0].boundingBox
      for (let shape of this.currentFrame.shapes) {
        growBoundingBox(bbox, shape.boundingBox)
      }
    }
    if (this.children.length > 0) {
      if (!bbox) {
        bbox = this.children[0].bbox()
      }
      for (let child of this.children) {
        growBoundingBox(bbox, child.bbox())
      }
    }
    return bbox
  }
  draw(context) {
    let ctx = context.ctx;
    ctx.translate(this.x, this.y)
    ctx.rotate(this.rotation)
    if (this.currentFrameNum>=this.maxFrame) {
      this.currentFrameNum = 0;
    }
    for (let shape of this.currentFrame.shapes) {
      shape.draw(context)
    }
    for (let child of this.children) {
      let idx = child.idx
      if (idx in this.currentFrame.keys) {
        child.x = this.currentFrame.keys[idx].x;
        child.y = this.currentFrame.keys[idx].y;
        child.rotation = this.currentFrame.keys[idx].rotation;
        child.scale = this.currentFrame.keys[idx].scale;
        ctx.save()
        child.draw(context)
        ctx.restore()
      }
    }
    if (this == context.activeObject) {
      if (context.activeCurve) {
        ctx.strokeStyle = "magenta"
        ctx.beginPath()
        ctx.moveTo(context.activeCurve.current.points[0].x, context.activeCurve.current.points[0].y)
        ctx.bezierCurveTo(context.activeCurve.current.points[1].x, context.activeCurve.current.points[1].y,
                          context.activeCurve.current.points[2].x, context.activeCurve.current.points[2].y,
                          context.activeCurve.current.points[3].x, context.activeCurve.current.points[3].y
        )
        ctx.stroke()
      }
      if (context.activeVertex) {
        ctx.save()
        ctx.strokeStyle = "#00ffff"
        let curves = {...context.activeVertex.current.startCurves,
          ...context.activeVertex.current.endCurves
        }
        // I don't understand why I can't use a for...of loop here
        for (let idx in curves) {
          let curve = curves[idx]
          ctx.beginPath()
          ctx.moveTo(curve.points[0].x, curve.points[0].y)
          ctx.bezierCurveTo(
            curve.points[1].x,curve.points[1].y,
            curve.points[2].x,curve.points[2].y,
            curve.points[3].x,curve.points[3].y
          )
          ctx.stroke()
        }
        ctx.fillStyle = "black"
        ctx.beginPath()
        let vertexSize = 15
        ctx.rect(context.activeVertex.current.point.x - vertexSize/2,
          context.activeVertex.current.point.y - vertexSize/2, vertexSize, vertexSize
        )
        ctx.fill()
        ctx.restore()
      }
      for (let item of context.selection) {
        ctx.save()
        ctx.strokeStyle = "#00ffff"
        ctx.lineWidth = 1;
        ctx.translate(item.x, item.y)
        ctx.beginPath()
        let bbox = item.bbox()
        ctx.rect(bbox.x.min, bbox.y.min, bbox.x.max, bbox.y.max)
        ctx.stroke()
        ctx.restore()
      }
      if (context.selectionRect) {
        ctx.save()
        ctx.strokeStyle = "#00ffff"
        ctx.lineWidth = 1;
        ctx.beginPath()
        ctx.rect(
          context.selectionRect.x1, context.selectionRect.y1,
          context.selectionRect.x2 - context.selectionRect.x1,
          context.selectionRect.y2 - context.selectionRect.y1
        )
        ctx.stroke()
        ctx.restore()
      }
    }
  }
  addShape(shape) {
    this.currentFrame.shapes.push(shape)
  }
  addObject(object, x=0, y=0) {
    this.children.push(object)
    let idx = object.idx
    this.currentFrame.keys[idx] = {
      x: x,
      y: y,
      rotation: 0,
      scale: 1,
    }
  }
  removeShape(shape) {
    for (let layer of this.layers) {
      for (let frame of layer.frames) {
        let shapeIndex = frame.shapes.indexOf(shape)
        if (shapeIndex >= 0) {
          frame.shapes.splice(shapeIndex, 1)
        }
      }
    }
  }
  removeChild(childObject) {
    let idx = childObject.idx
    for (let layer of this.layers) {
      for (let frame of layer.frames) {
        delete frame[idx]
      }
    }
    this.children.splice(this.children.indexOf(childObject), 1)
  }
  saveState() {
    startProps[this.idx] = {
      x: this.x,
      y: this.y,
      rotation: this.rotation,
      scale: this.scale
    }
  }
}

let root = new GraphicsObject("root");
context.activeObject = root

async function greet() {
  // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
  greetMsgEl.textContent = await invoke("greet", { name: greetInputEl.value });

}

window.addEventListener("DOMContentLoaded", () => {
  rootPane = document.querySelector("#root")
  rootPane.appendChild(createPane(toolbar()))
  rootPane.addEventListener("mousemove", (e) => {
    mouseEvent = e;
  })
  let [_toolbar, panel] = splitPane(rootPane, 10, true, createPane(timeline()))
  let [stageAndTimeline, _infopanel] = splitPane(panel, 70, false, createPane(infopanel()))
  let [_timeline, _stage] = splitPane(stageAndTimeline, 30, false, createPane(stage()))
});

window.addEventListener("resize", () => {
  updateAll()
})

window.addEventListener("keypress", (e) => {
  // let shortcuts = {}
  // for (let shortcut of config.shortcuts) {
    // shortcut = shortcut.split("+")
    // TODO
  // }
  console.log(e)
  if (e.key == config.shortcuts.playAnimation) {
    console.log("Spacebar pressed")
  } else if (e.key == config.shortcuts.undo && e.ctrlKey == true) {
    undo()
  } else if (e.key == config.shortcuts.redo && e.ctrlKey == true) {
    redo()
  } else if (e.key == config.shortcuts.save && e.ctrlKey == true) {
    save()
  } else if (e.key == config.shortcuts.open && e.ctrlKey == true) {
    open()
  }
})

async function save() { 
  const path = await saveFileDialog({
    filters: [
      {
        name: 'Lightningbeam files (.beam)',
        extensions: ['beam'],
      },
    ],
    defaultPath: await join(await documentDir(), "untitled.beam")
  });
  try {
    const fileData = {
      version: "1.0",
      actions: undoStack
    }
    const contents = JSON.stringify(fileData   );
    await writeTextFile(path, contents)
    console.log(`${path} saved successfully!`);
  } catch (error) {
    console.error("Error saving text file:", error);
  }
}

async function open() {
  console.log("gonna open")
  const path = await openFileDialog({
    multiple: false,
    directory: false,
    filters: [
      {
        name: 'Lightningbeam files (.beam)',
        extensions: ['beam'],
      },
    ],
    defaultPath: await documentDir(),
  });
  if (path) {
    try {
      const contents = await readTextFile(path)
      let file = JSON.parse(contents)
      if (file.version == undefined) {
        await messageDialog("Could not read file version!", { title: "Load error", kind: 'error' })
        return
      }
      if (file.version >= minFileVersion) {
        if (file.version < maxFileVersion) {
          root = new GraphicsObject("root");
          context.activeObject = root
          if (file.actions == undefined) {
            await messageDialog("File has no content!", {title: "Parse error", kind: 'error'})
            return
          }
          for (let action of file.actions) {
            if (!(action.name in actions)) {
              await messageDialog(`Invalid action ${action.name}. File may be corrupt.`, { title: "Error", kind: 'error'})
              return
            }
            actions[action.name].execute(action.action)
            undoStack.push(action)
          }
          updateUI()
        } else {
          await messageDialog(`File ${path} was created in a newer version of Lightningbeam and cannot be opened in this version.`, { title: 'File version mismatch', kind: 'error' });
        }
      } else {
        await messageDialog(`File ${path} is too old to be opened in this version of Lightningbeam.`, { title: 'File version mismatch', kind: 'error' });
      }
    } catch (e) {
      console.log(e )
      if (e instanceof SyntaxError) {
        await messageDialog(`Could not parse ${path}, ${e.message}`, { title: 'Error', kind: 'error' })
      } else if (e.startsWith("failed to read file as text")) {
        await messageDialog(`Could not parse ${path}, is it actually a Lightningbeam file?`, { title: 'Error', kind: 'error' })
      }
    }
  }
}

function stage() {
  let stage = document.createElement("canvas")
  let scroller = document.createElement("div")
  stage.className = "stage"
  stage.width = 1500
  stage.height = 1000
  scroller.className = "scroll"
  stage.addEventListener("drop", (e) => {
    e.preventDefault()
    let mouse = getMousePos(stage, e)
    const imageTypes = ['image/png', 'image/gif', 'image/avif', 'image/jpeg',
       'image/svg+xml', 'image/webp'
    ];
    if (e.dataTransfer.items) {
      let i = 0
      for (let item of e.dataTransfer.items) {
        if (item.kind == "file") {
          let file = item.getAsFile()
          if (imageTypes.includes(file.type)) {
            let img = new Image()
            img.src = window.URL.createObjectURL(file)
            img.ix = i
            img.idx = uuidv4()
            pointerList[img.idx] = img
            img.onload = function() {
              actions.addImageObject.create(
                mouse.x, mouse.y, img, context.activeObject)
              updateUI()
            }
          }
          i++;
        }
      }
    } else {
    }
  })
  stage.addEventListener("dragover", (e) => {
    e.preventDefault()
  })
  canvases.push(stage)
  scroller.appendChild(stage)
  stage.addEventListener("mousedown", (e) => {
    let mouse = getMousePos(stage, e)
    switch (mode) {
      case "rectangle":
      case "draw":
        context.mouseDown = true
        context.activeShape = new Shape(mouse.x, mouse.y, context, true, true)
        context.lastMouse = mouse
        break;
      case "select":
        let selection = selectVertex(context, mouse)
        if (selection) {
          context.dragging = true
          context.activeCurve = undefined
          context.activeVertex = {
            current: {
              point: {x: selection.vertex.point.x, y: selection.vertex.point.y},
              startCurves: structuredClone(selection.vertex.startCurves),
              endCurves: structuredClone(selection.vertex.endCurves),
            },
            initial: selection.vertex,
            shape: selection.shape,
            startmouse: {x: mouse.x, y: mouse.y}
          }
          console.log("gonna move this")
        } else {
          selection = selectCurve(context, mouse)
          if (selection) {
            context.dragging = true
            context.activeVertex = undefined
            context.activeCurve = {
              initial: selection.curve,
              current: new Bezier(selection.curve.points).setColor(selection.curve.color),
              shape: selection.shape,
              startmouse: {x: mouse.x, y: mouse.y}
            }
            console.log("gonna move this")
          } else {
            let selected = false
            let child;
            if (context.selection.length) {
              for (child of context.selection) {
                if (hitTest(mouse, child)) {
                  context.dragging = true
                  context.lastMouse = mouse
                  context.activeObject.currentFrame.saveState()
                  break
                }
              }
            }
            if (!context.dragging) {
              // Have to iterate in reverse order to grab the frontmost object when two overlap
              for (let i=context.activeObject.children.length-1; i>=0; i--) {
                child = context.activeObject.children[i]
                // let bbox = child.bbox()
                if (hitTest(mouse, child)) {
                    if (context.selection.indexOf(child) != -1) {
                      // dragging = true
                    }
                    child.saveState()
                    context.selection = [child]
                    context.dragging = true
                    selected = true
                    context.activeObject.currentFrame.saveState()
                    break
                }
              }
              if (!selected) {
                context.selection = []
                context.selectionRect = {x1: mouse.x, x2: mouse.x, y1: mouse.y, y2:mouse.y}
              }
            }
          }
        }
        break;
      case "paint_bucket":
        let line = {p1: mouse, p2: {x: mouse.x + 3000, y: mouse.y}}
        for (let shape of context.activeObject.currentFrame.shapes) {
          for (let region of shape.regions) {
            let intersect_count = 0;
            for (let curve of region.curves) {
              intersect_count += curve.intersects(line).length
            }
            console.log(region)
            console.log(intersect_count)
            if (intersect_count%2==1) {
              // region.fillStyle = context.fillStyle
              actions.colorRegion.create(region, context.fillStyle)
            }
          }
        }
        break;
      default:
        break;
    }
    context.lastMouse = mouse
    updateUI()
  })
  stage.addEventListener("mouseup", (e) => {
    context.mouseDown = false
    context.dragging = false
    context.selectionRect = undefined
    let mouse = getMousePos(stage, e)
    switch (mode) {
      case "draw":
        if (context.activeShape) {
          context.activeShape.addLine(mouse.x, mouse.y)
          context.activeShape.simplify(context.simplifyMode)
          actions.addShape.create(context.activeObject, context.activeShape)
          context.activeShape = undefined
        }
        break;
      case "rectangle":
        actions.addShape.create(context.activeObject, context.activeShape)
        context.activeShape = undefined
        break;
      case "select":
        if (context.activeVertex) {
          let newCurves = []
          for (let i in context.activeVertex.shape.curves) {
            if (i in context.activeVertex.current.startCurves) {
              newCurves.push(context.activeVertex.current.startCurves[i])
            } else if (i in context.activeVertex.current.endCurves) {
              newCurves.push(context.activeVertex.current.endCurves[i])
            } else {
              newCurves.push(context.activeVertex.shape.curves[i])
            }
          }
          actions.editShape.create(context.activeVertex.shape, newCurves)
        } else if (context.activeCurve) {
          let newCurves = []
          for (let curve of context.activeCurve.shape.curves) {
            if (curve == context.activeCurve.initial) {
              newCurves.push(context.activeCurve.current)
            } else {
              newCurves.push(curve)
            }
          }
          actions.editShape.create(context.activeCurve.shape, newCurves)
        } else if (context.selection.length) {
          actions.editFrame.create(context.activeObject.currentFrame)
        }
        break;
      default:
        break;
    }
    context.lastMouse = mouse
    context.activeCurve = undefined
    updateUI()
  })
  stage.addEventListener("mousemove", (e) => {
    let mouse = getMousePos(stage, e)
    switch (mode) {
      case "draw":
        context.activeCurve = undefined
        if (context.activeShape) {
          if (vectorDist(mouse, context.lastMouse) > minSegmentSize) {
            context.activeShape.addLine(mouse.x, mouse.y)
            context.lastMouse = mouse
          }
        }
        break;
      case "rectangle":
        context.activeCurve = undefined
        if (context.activeShape) {
          context.activeShape.clear()
          context.activeShape.addLine(mouse.x, context.activeShape.starty)
          context.activeShape.addLine(mouse.x, mouse.y)
          context.activeShape.addLine(context.activeShape.startx, mouse.y)
          context.activeShape.addLine(context.activeShape.startx, context.activeShape.starty)
          context.activeShape.update()
        }
        break;
      case "select":
        if (context.dragging) {
          if (context.activeVertex) {
            let vert = context.activeVertex
            let mouseDelta = {x: mouse.x - vert.startmouse.x, y: mouse.y - vert.startmouse.y}
            vert.current.point.x = vert.initial.point.x + mouseDelta.x
            vert.current.point.y = vert.initial.point.y + mouseDelta.y
            for (let i in vert.current.startCurves) {
              let curve = vert.current.startCurves[i]
              let oldCurve = vert.initial.startCurves[i]
              curve.points[0] = vert.current.point
              curve.points[1] = {
                x: oldCurve.points[1].x + mouseDelta.x,
                y: oldCurve.points[1].y + mouseDelta.y
              }
            }
            for (let i in vert.current.endCurves) {
              let curve = vert.current.endCurves[i]
              let oldCurve = vert.initial.endCurves[i]
              curve.points[3] = {x:vert.current.point.x, y:vert.current.point.y}
              curve.points[2] = {
                x: oldCurve.points[2].x + mouseDelta.x,
                y: oldCurve.points[2].y + mouseDelta.y
              }
            }
          } else if (context.activeCurve) {
            context.activeCurve.current.points = moldCurve(
              context.activeCurve.initial, mouse, context.activeCurve.startmouse
            ).points 
          } else {
            for (let child of context.selection) {
              context.activeObject.currentFrame.keys[child.idx].x += (mouse.x - context.lastMouse.x)
              context.activeObject.currentFrame.keys[child.idx] .y += (mouse.y - context.lastMouse.y)
            }
          }
        } else if (context.selectionRect) {
          context.selectionRect.x2 = mouse.x
          context.selectionRect.y2 = mouse.y
          context.selection = []
          for (let child of context.activeObject.children) {
            if (hitTest(regionToBbox(context.selectionRect), child)) {
              context.selection.push(child)
            }
          }
        } else {
          let selection = selectVertex(context, mouse)
          if (selection) {
            context.activeCurve = undefined
            context.activeVertex = {
              current: selection.vertex,
              initial: {
                point: {x: selection.vertex.point.x, y: selection.vertex.point.y}, 
                startCurves: structuredClone(selection.vertex.startCurves),
                endCurves: structuredClone(selection.vertex.endCurves),
              },
              shape: selection.shape,
              startmouse: {x: mouse.x, y: mouse.y}
            }
          } else {
            context.activeVertex = undefined
            selection = selectCurve(context, mouse)
            if (selection) {
              context.activeCurve = {
                current: selection.curve, 
                initial: new Bezier(selection.curve.points).setColor(selection.curve.color),
                shape: selection.shape,
                startmouse: mouse
              }
            } else {
              context.activeCurve = undefined
            }
          }
        }
        context.lastMouse = mouse
        break;
      default:
        break;
    }
    updateUI()
  })
  return scroller
}

function toolbar() {
  let tools_scroller = document.createElement("div")
  tools_scroller.className = "toolbar"
  for (let tool in tools) {
    let toolbtn = document.createElement("button")
    toolbtn.className = "toolbtn"
    let icon = document.createElement("img")
    icon.className = "icon"
    icon.src = tools[tool].icon
    toolbtn.appendChild(icon)
    tools_scroller.appendChild(toolbtn)
    toolbtn.addEventListener("click", () => {
      mode = tool
      console.log(tool)
    })
  }
  let tools_break = document.createElement("div")
  tools_break.className = "horiz_break"
  tools_scroller.appendChild(tools_break)
  let fillColor = document.createElement("input")
  let strokeColor = document.createElement("input")
  fillColor.className = "color-field"
  strokeColor.className = "color-field"
  fillColor.value = "#ffffff"
  strokeColor.value = "#000000"
  context.fillStyle = fillColor.value
  context.strokeStyle = strokeColor.value
  fillColor.addEventListener('click', e => {
    Coloris({
      el: ".color-field",
      selectInput: true,
      focusInput: true,
      theme: 'default',
      swatches: context.swatches,
      defaultColor: '#ffffff',
      onChange: (color) => {
        context.fillStyle = color;
      }
    })
  })
  strokeColor.addEventListener('click', e => {
    Coloris({
      el: ".color-field",
      selectInput: true,
      focusInput: true,
      theme: 'default',
      swatches: context.swatches,
      defaultColor: '#000000',
      onChange: (color) => {
        context.strokeStyle = color;
      }
    })
  })
  // Fill and stroke colors use the same set of swatches
  fillColor.addEventListener("change", e => {
    context.swatches.unshift(fillColor.value)
    if (context.swatches.length>12) context.swatches.pop();
  })
  strokeColor.addEventListener("change", e => {
    context.swatches.unshift(strokeColor.value)
    if (context.swatches.length>12) context.swatches.pop();
  })
  tools_scroller.appendChild(fillColor)
  tools_scroller.appendChild(strokeColor)
  return tools_scroller
}

function timeline() {
  let container = document.createElement("div")
  let layerspanel = document.createElement("div")
  let framescontainer = document.createElement("div")
  container.classList.add("horizontal-grid")
  container.classList.add("layers-container")
  layerspanel.className = "layers"
  framescontainer.className = "frames-container"
  container.appendChild(layerspanel)
  container.appendChild(framescontainer)
  layoutElements.push(container)
  container.setAttribute("lb-percent", 20)

  return container
}

function infopanel() {
  let panel = document.createElement("div")
  panel.className = "infopanel"
  let input;
  let label;
  let span;
  // for (let i=0; i<10; i++) {
  for (let property in tools[mode].properties) {
    let prop = tools[mode].properties[property]
    label = document.createElement("label")
    label.className = "infopanel-field"
    span = document.createElement("span")
    span.className = "infopanel-label"
    span.innerText = prop.label
    switch (prop.type) {
      case "number":
        input = document.createElement("input")
        input.className = "infopanel-input"
        input.type = "number"
        input.value = getProperty(context, property)   
        break;
      case "enum":
        input = document.createElement("select")
        input.className = "infopanel-input"
        let optionEl;
        for (let option of prop.options) {
          optionEl = document.createElement("option")
          optionEl.value = option
          optionEl.innerText = option
          input.appendChild(optionEl)
        }
        input.value = getProperty(context, property)
        break;
      case "boolean":
        input = document.createElement("input")
        input.className = "infopanel-input"
        input.type = "checkbox"
        input.checked = getProperty(context, property)
        break;
    }
    input.addEventListener("input", (e) => {
      switch (prop.type) {
        case "number":
          if (!isNaN(e.target.value) && e.target.value > 0) {
            setProperty(context, property, e.target.value)
          }
          break;
        case "enum":
          if (prop.options.indexOf(e.target.value) >= 0) {
            setProperty(context, property, e.target.value)
          }
          break;
        case "boolean":
          setProperty(context, property, e.target.checked)
      }

    })
    label.appendChild(span)
    label.appendChild(input)
    panel.appendChild(label)
  }
  return panel
}

function createPane(content=undefined) {
  let div = document.createElement("div")
  let header = document.createElement("div")
  if (!content) {
    content = stage() // TODO: change based on type
  }
  header.className = "header"

  let button = document.createElement("button")
  header.appendChild(button)
  let icon = document.createElement("img")
  icon.className="icon"
  icon.src = "/assets/stage.svg"
  button.appendChild(icon)

  div.className = "vertical-grid"
  header.style.height = "calc( 2 * var(--lineheight))"
  content.style.height = "calc( 100% - 2 * var(--lineheight) )"
  div.appendChild(header)
  div.appendChild(content)
  return div
}

function splitPane(div, percent, horiz, newPane=undefined) {
  let content = div.firstElementChild
  let div1 = document.createElement("div")
  let div2 = document.createElement("div")

  div1.className = "panecontainer"
  div2.className = "panecontainer"

  div1.appendChild(content)
  if (newPane) {
    div2.appendChild(newPane)
  } else {
    div2.appendChild(createPane())
  }
  div.appendChild(div1)
  div.appendChild(div2)

  if (horiz) {
    div.className = "horizontal-grid"
  } else {
    div.className = "vertical-grid"
  }
  div.setAttribute("lb-percent", percent) // TODO: better attribute name
  Coloris({el: ".color-field"})
  updateAll()
  updateUI()
  updateLayers()
  return [div1, div2]
}

function updateAll() {
  updateLayout(rootPane)
  for (let element of layoutElements) {
    updateLayout(element)
  }
}

function updateLayout(element) {
  let rect = element.getBoundingClientRect()
  let percent = element.getAttribute("lb-percent")
  percent ||= 50
  let children = element.children
  if (children.length != 2) return;
  if (element.classList.contains("horizontal-grid")) {
    children[0].style.width = `${rect.width * percent / 100}px`
    children[1].style.width = `${rect.width * (100 - percent) / 100}px`
    children[0].style.height = `${rect.height}px`
    children[1].style.height = `${rect.height}px`
  } else if (element.classList.contains("vertical-grid")) {
    children[0].style.height = `${rect.height * percent / 100}px`
    children[1].style.height = `${rect.height * (100 - percent) / 100}px`
    children[0].style.width = `${rect.width}px`
    children[1].style.width = `${rect.width}px`
  }
  if (children[0].getAttribute("lb-percent")) {
    updateLayout(children[0])
  }
  if (children[1].getAttribute("lb-percent")) {
    updateLayout(children[1])
  }
}

function updateUI() {
  for (let canvas of canvases) {
    let ctx = canvas.getContext("2d")
    ctx.reset();
    ctx.fillStyle = "white"
    ctx.fillRect(0,0,canvas.width,canvas.height)

    context.ctx = ctx;
    root.draw(context)
    if (context.activeShape) {
      context.activeShape.draw(context)
    }

  }
}

function updateLayers() {
  console.log(document.querySelectorAll(".layers-container"))
  for (let container of document.querySelectorAll(".layers-container")) {
    let layerspanel = container.querySelectorAll(".layers")[0]
    let framescontainer = container.querySelectorAll(".frames-container")[0]
    layerspanel.textContent = ""
    framescontainer.textContent = ""
    for (let layer of context.activeObject.layers) {
    // for (let i=0; i<5; i++) {
      let layerHeader = document.createElement("div")
      layerHeader.className = "layer-header"
      layerspanel.appendChild(layerHeader)
      let layerTrack = document.createElement("div")
      layerTrack.className = "layer-track"
      framescontainer.appendChild(layerTrack)
      for (let frame of layer.frames) {
      // for (let j=0; j<5-i; j++) {
        let frameEl = document.createElement("div")
        frameEl.className = "frame"
        layerTrack.appendChild(frameEl)
      }
    }
  }
}
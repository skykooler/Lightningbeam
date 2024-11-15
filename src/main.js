const { invoke } = window.__TAURI__.core;
import * as fitCurve from '/fit-curve.js';

let greetInputEl;
let greetMsgEl;
let rootPane;

let canvases = [];

let mode = "draw"

let minSegmentSize = 5;
let maxSmoothAngle = 0.2;

let tools = {
  select: {
    icon: "/assets/select.svg",

  },
  transform: {
    icon: "/assets/transform.svg",

  },
  draw: {
    icon: "/assets/draw.svg"
  },
  rectangle: {
    icon: "/assets/rectangle.svg"
  },
  polygon: {
    icon: "assets/polygon.svg"
  }
}

let mouseEvent;
console.log(fitCurve)

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
  ]
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

class Curve {
  constructor(cp1x, cp1y, cp2x, cp2y, x, y) {
    this.cp1x = cp1x;
    this.cp1y = cp1y;
    this.cp2x = cp2x;
    this.cp2y = cp2y;
    this.x = x;
    this.y = y;
  }
}

class Frame {
  constructor() {
    this.keys = {}
  }
}

class Shape {
  constructor(startx, starty, fillStyle, strokeStyle, filled=true, stroked=true) {
    this.startx = startx;
    this.starty = starty;
    this.curves = [];
    this.fillStyle = fillStyle;
    this.strokeStyle = strokeStyle;
    this.filled = filled;
    this.stroked = stroked;
  }
  addCurve(curve) {
    this.curves.push(curve)
  }
  simplify(mode="smooth") {
    // Mode can be corners, smooth or auto
    // let maxIndex;
    // for (let j=0; j<5; j++) {
    //   if (this.curves.length < 3) return;
    //   maxIndex = this.curves.length-1;
    //   for (let i=1; i<maxIndex; i++) {
    //     let P1 = this.curves[i]
    //     let P2 = this.curves[i-1]
    //     let P3 = this.curves[i+1]
    //     let angle = Math.atan2(P3.y - P1.y, P3.x - P1.x) -
    //             Math.atan2(P2.y - P1.y, P2.x - P1.x);
    //     angle = Math.PI - Math.abs(angle)
    //     if (Math.abs(angle) < maxSmoothAngle) {
    //       if (mode=="corners") {
    //         this.curves.splice(i,1)
    //       } else if (mode=="smooth") {
    //         P3.cp1x = P1.x;
    //         P3.cp1y = P1.y;
    //         P3.cp2x = P1.x;
    //         P3.cp2y = P1.y;
    //         this.curves.splice(i,1)
    //       }
    //       i;
    //       maxIndex--;
    //       console.log(angle)
    //     }
    //   }

    // }
    let error = 30;
    let points = [[this.startx, this.starty]]
    for (let curve of this.curves) {
      points.push([curve.x, curve.y])
    }
    this.curves = []
    let curves = fitCurve.fitCurve(points, error)
    for (let curve of curves) {
      this.curves.push(new Curve(curve[1][0],curve[1][1],curve[2][0], curve[2][1], curve[3][0], curve[3][1]))
    }
  }
}

class GraphicsObject {
  constructor() {
    this.x = 0;
    this.y = 0;
    this.rotation = 0;
    this.scale = 1;
    this.idx = uuidv4()

    this.frames = [new Frame()]
    this.currentFrame = 0;
    this.children = []

    this.shapes = []
  }
  draw(context) {
    let ctx = context.ctx;
    if (this.currentFrame>=this.frames.length) {
      this.currentFrame = 0;
    }
    for (let child of this.children) {
      let idx = child.idx
      child.x = this.frames[this.currentFrame][idx].x;
      child.y = this.frames[this.currentFrame][idx].y;
      child.rotation = this.frames[this.currentFrame][idx].rotation;
      child.scale = this.frames[this.currentFrame][idx].scale;
      child.draw(context)
    }
    for (let shape of this.shapes) {
      ctx.beginPath()
      ctx.moveTo(shape.startx, shape.starty)
      for (let curve of shape.curves) {
        ctx.bezierCurveTo(curve.cp1x, curve.cp1y, curve.cp2x, curve.cp2y, curve.x, curve.y)

        // Debug, show curve endpoints
        // ctx.beginPath()
        // ctx.arc(curve.x,curve.y, 3, 0, 2*Math.PI)
        // ctx.fill()
      }
      if (shape.filled) {
        ctx.fillStyle = shape.fillStyle
        ctx.fill()
      }
      if (shape.stroked) {
        ctx.strokeStyle = shape.strokeStyle
        ctx.stroke()
      }
    }
  }
  addShape(shape) {
    this.shapes.push(shape)
  }
}

let root = new GraphicsObject();
// let shp = new Shape(100,100,'blue', 'black')
// shp.addCurve(new Curve(150,150,150,150,200,100))
// root.addShape(shp)
context.activeObject = root

async function greet() {
  // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
  greetMsgEl.textContent = await invoke("greet", { name: greetInputEl.value });

  // splitPane(rootPane, 50, true)
}

window.addEventListener("DOMContentLoaded", () => {
  // greetInputEl = document.querySelector("#greet-input");
  // greetMsgEl = document.querySelector("#greet-msg");
  rootPane = document.querySelector("#root")
  rootPane.appendChild(toolbar())
  rootPane.addEventListener("mousemove", (e) => {
    mouseEvent = e;
  })
  // document.querySelector("#greet-form").addEventListener("submit", (e) => {
  //   e.preventDefault();
  //   greet();
  // });
  splitPane(rootPane, 10, true)
});

function stage() {
  let stage = document.createElement("canvas")
  let scroller = document.createElement("div")
  stage.className = "stage"
  stage.width = 1500
  stage.height = 1000
  scroller.className = "scroll"
  canvases.push(stage)
  scroller.appendChild(stage)
  stage.addEventListener("mousedown", (e) => {
    let mouse = getMousePos(stage, e)
    switch (mode) {
      case "draw":
        context.mouseDown = true
        context.activeShape = new Shape(mouse.x, mouse.y, context.fillStyle, context.strokeStyle, true, true)
        context.activeObject.addShape(context.activeShape)
        context.lastMouse = mouse
        console.log(stage)
        break;
    
      default:
        break;
    }
    context.lastMouse = mouse
    updateUI()
  })
  stage.addEventListener("mouseup", (e) => {
    context.mouseDown = false
    let mouse = getMousePos(stage, e)
    switch (mode) {
      case "draw":
        if (context.activeShape) {
          let midpoint = {x: (mouse.x+context.lastMouse.x)/2, y: (mouse.y+context.lastMouse.y)/2}
          context.activeShape.addCurve(new Curve(midpoint.x, midpoint.y, midpoint.x, midpoint.y, mouse.x, mouse.y))
          context.activeShape.simplify()
          context.activeShape = undefined
        }
        break;
    
      default:
        break;
    }
    context.lastMouse = mouse
    updateUI()
  })
  stage.addEventListener("mousemove", (e) => {
    let mouse = getMousePos(stage, e)
    switch (mode) {
      case "draw":
        if (context.activeShape) {
          if (vectorDist(mouse, context.lastMouse) > minSegmentSize) {
            let midpoint = {x: (mouse.x+context.lastMouse.x)/2, y: (mouse.y+context.lastMouse.y)/2}
            context.activeShape.addCurve(new Curve(midpoint.x, midpoint.y, midpoint.x, midpoint.y, mouse.x, mouse.y))
            context.lastMouse = mouse
          }
        }
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

function createPane() {
  let div = document.createElement("div")
  let header = document.createElement("div")
  let content = stage() // TODO: change based on type
  header.className = "header"

  let button = document.createElement("button")
  header.appendChild(button)
  let icon = document.createElement("img")
  icon.className="icon"
  icon.src = "/assets/stage.svg"
  button.appendChild(icon)


  // div.style.display = "grid";
  // div.style.gridTemplateColumns = `var(--lineheight) 1fr`
  // div.style.gridTemplateRows = "1fr"
  // header.style.gridArea = "1 / 1 / 2 / 2"
  // content.style.gridArea = "1 / 2 / 2 / 3"

  div.classList = ["vertical-grid", "pane"]
  header.style.flex = "0 0 var(--lineheight)"
  content.style.flex = "1 1 100%"
  div.appendChild(header)
  div.appendChild(content)
  return div
}

function splitPane(div, percent, horiz) {
  let content = div.firstElementChild
  let div1 = document.createElement("div")
  let div2 = document.createElement("div")

  div1.className = "panecontainer"
  div2.className = "panecontainer"

  div1.appendChild(content)
  div2.appendChild(createPane())
  div.appendChild(div1)
  div.appendChild(div2)

  // div.style.display = "grid";
  // if (horiz) {
  //   div.classList.add("horizontal-grid")
  //   div.style.gridTemplateColumns = `${percent}% 1fr`
  //   div1.style.gridArea = "1 / 1 / 2 / 2"
  //   div2.style.gridArea = "1 / 2 / 2 / 3"
  // } else {
  //   div.classList.add("vertical-grid")
  //   div.style.gridTemplateRows = `${percent}% 1fr`
  //   div1.style.gridArea = "1 / 1 / 2 / 2"
  //   div2.style.gridArea = "2 / 1 / 3 / 2"
  // }
  if (horiz) {
    div.className = "horizontal-grid"
  } else {
    div.className = "verical-grid"
  }
  div1.style.flex = `0 0 ${percent}%`
  div2.style.flex = `1 1 auto`
  Coloris({el: ".color-field"})
  updateUI()
}



function updateUI() {
  for (let canvas of canvases) {
    let ctx = canvas.getContext("2d")
    ctx.fillStyle = "white"
    ctx.fillRect(0,0,canvas.width,canvas.height)
    ctx.fillStyle = "green"
    // ctx.fillRect(0,0,200,200)

    context.ctx = ctx;
    root.draw(context)

    // let mouse;
    // if (mouseEvent) {
    //   mouse = getMousePos(canvas, mouseEvent);
    // } else {
    //   mouse = {x: 0, y: 0}
    // }
    // ctx.fillRect(mouse.x, mouse.y, 50,50)
  }
  // requestAnimationFrame(updateUI)
}
// const { invoke } = window.__TAURI__.core;

let greetInputEl;
let greetMsgEl;
let rootPane;

let canvases = [];

let mode = "draw"

let tools = {
  select: {
    icon: "/assets/select.png",

  },
  draw: {
    icon: "/assets/pen.png"
  },
  rectangle: {
    icon: "/assets/rectangle.png"
  },
  polygon: {
    icon: "assets/polygon.png"
  }
}

let mouseEvent;

let context = {}

function uuidv4() {
  return "10000000-1000-4000-8000-100000000000".replace(/[018]/g, c =>
    (+c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> +c / 4).toString(16)
  );
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
let shp = new Shape(100,100,'blue', 'black')
shp.curves.push(new Curve(150,150,150,150,200,100))
root.addShape(shp)

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

    let mouse;
    if (mouseEvent) {
      mouse = getMousePos(canvas, mouseEvent);
    } else {
      mouse = {x: 0, y: 0}
    }
    ctx.fillRect(mouse.x, mouse.y, 50,50)
  }
  requestAnimationFrame(updateUI)
}
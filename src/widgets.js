import { clamp, drawCheckerboardBackground, hslToRgb, hsvToRgb, rgbToHex } from "./utils.js"

class Widget {
    constructor(x, y) {
        this._globalEvents = new Set()
        this.x = x
        this.y = y
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
    draw(ctx) {
        for (let child of this.children) {
            const transform = ctx.getTransform()
            ctx.translate(child.x, child.y)
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

export {
    Widget,
    HueSelectionBar,
    SaturationValueSelectionGradient,
    AlphaSelectionBar,
    ColorWidget,
    ColorSelectorWidget
};
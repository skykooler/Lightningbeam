<html>
<head>
<style type="text/css">
canvas { border: 2px solid #000; position:absolute; top:0;left:0; 
visibility: hidden; }
</style>
</head>
<body>
<canvas id="canvas1" width=500 height=500></canvas>
<canvas id="canvas2" width=500 height=500></canvas>
<script>
//--------------------------  BEGIN JAVASCRIPT  --------------------------------\\

var fps = 50
var cr;
var canvas;

function _timerBase () {
	/* This provides the 'tick' by which all animations are run.
	Playing animations should have their ._draw() method added here;
	to stop them, call remove() on it. */
	this.funcs = {}
	this.add = function (item) {
		this.funcs[item]=item;
	}
	this.remove = function (item) {
		delete this.funcs[item];
	}
	this.iterate = function() {
		canvas = Buffers[DrawingBuffer];

		if (canvas.getContext) {
			cr = canvas.getContext("2d");
			cr.clearRect(0, 0, canvas.width, canvas.height);
			cr.beginPath()

			DrawingBuffer=1-DrawingBuffer;
			//canvas = Buffers[DrawingBuffer];
			draw()
			for (i in this.funcs){
				this.funcs[i]._draw()
			}
			Buffers[1-DrawingBuffer].style.visibility='hidden';
			Buffers[DrawingBuffer].style.visibility='visible';
		}
	}
	
	setInterval('Timer.iterate()', 1000/fps)
}

var Timer = new _timerBase()

function Frame () {
	this.actions = ''
	this.run_script = function() {
		eval(this.actions)
	}
}

function MovieClip() {
	/* From the ActionScript reference:
	
	You do not use a constructor method to create a movie clip. You can choose from among
	three methods to create movie clip instances:

	The attachMovie() method allows you to create a movie clip instance based on a movie 
	clip symbol that exists in the library.
	The createEmptyMovieClip() method allows you to create an empty movie clip instance as
	a child based on another movie clip.
	The duplicateMovieClip() method allows you to create a movie clip instance based on 
	another movie clip.
	*/
	this._frames = [new Frame()]
	this._currentframe = 1;
	this._draw = function (sttc) {
		for (i in this) {
			if (this._frames[this._currentframe-1][i]) {
				this[i]._draw(this._frames[this._currentframe-1][i]);
			}
		}
		if (!sttc) {
			this._frames[this._currentframe-1].run_script()
			this._currentframe++;
			if (this._currentframe>this._frames.length) {
				this._currentframe = 1;
			}
		}
	}
	this.play = function () {
		Timer.add(this)
	}
	this.stop = function () {
		Timer.remove(this)
	}
}

function Shape() {
	// Not part of the ActionScript spec, but necessary.
	this._shapedata = []
	this.fill = "#000000"
	this._draw = function (frame) {
		this._x = frame._x
		this._y = frame._y
		this._xscale = frame._xscale
		this._yscale = frame._yscale
		this._rotation = frame._rotation
		cr.save()
		cr.translate(this._x,this._y)
		cr.rotate(this._rotation*Math.PI/180)
		cr.scale(this._xscale*1.0, this._yscale*1.0)
		cr.fillStyle = this.fill;
		for (i in this._shapedata) {
			if (this._shapedata[i][0]=="M") {
				cr.moveTo(this._shapedata[i][1],this._shapedata[i][2])
			} else if (this._shapedata[i][0]=="L") {
				cr.lineTo(this._shapedata[i][1],this._shapedata[i][2])
			} else if (this._shapedata[i][0]=="C") {
				cr.bezierCurveTo(this._shapedata[i][1],this._shapedata[i][2],this._shapedata[i][3],this._shapedata[i][4],this._shapedata[i][5],this._shapedata[i][6])
			}
		}
		if (self.filled) {
			cr.stroke()
			cr.fill()
		} else {
			cr.stroke()
		}
		cr.restore()
	}
}

var Stage = {
	
}

var root = {}

/*if (canvas.getContext) {
	cr = canvas.getContext("2d");
}*/

var Buffers = [document.getElementById("canvas1"), document.getElementById("canvas2")]
var DrawingBuffer = 0

function draw() {
	
	if (canvas.getContext) {
		cr = canvas.getContext("2d");
		
		
		for (i in root) {
			if (root[i]._draw) {
				//root[i]._draw(true)
			}
		}
	}
}

function play() {
}
	

var a = new Shape()
a._shapedata = [["M",0,0],["L",400,0],["L",400,200],["L",0,200],["L",0,0]]
var b = new MovieClip()
b.a = a
b._frames[0].a = {}
b._frames[0].a._x = 100
b._frames[0].a._y = 20
b._frames[0].actions = 'this.a._x = this.a._x + 1'
root.b = b
b._frames[1] = new Frame()
b._frames[1].a = {}
b._frames[1].a._x = 50
b._frames[1].a._y = 40

setTimeout('b.play()',2000)

//-------------------  END OF JAVASCRIPT ------------------------\\
</script>
</body>
</html>

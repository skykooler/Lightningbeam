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
var _processingobj

var appendError = function(str){
   throw new Error("DEBUG: "+str)
}

function log(str){
   setTimeout("appendError('"+str+"')", 1)
}

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

function ave(x, y, fac) {
	//Weighted average. 
	//fac is the weight - 0.5 gives a standard average
	return y - fac*(y-x)
}

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
	this._playing = true;
	Timer.add(this)
	this._draw = function (sttc) {
		_processingobj = this
		for (var i in this) {
			if (this._frames[this._currentframe-1]==undefined) {
				for (var j=0; j<this._currentframe-1; j++) {
					if (this._frames[j]) {
						last = j
					}
				}
				for (var j=this._frames.length; j>this._currentframe-1; j--) {
					if (this._frames[j]) {
						next = j
					}
				}
				if (this._frames[last][i]) {
					this[i]._draw(this._frames[last][i],this._frames[next][i],(this._currentframe-last)/(next-last));
				}
			}
			else {
				if (this._frames[this._currentframe-1][i]) {
					this[i]._draw(this._frames[this._currentframe-1][i]);
				}
			}
		}
		if (this._frames[this._currentframe-1]) {
			if (this._playing) {
				this._frames[this._currentframe-1].run_script()
			}
			if (this._playing) {
				this._currentframe++;
				if (this._currentframe>this._frames.length) {
					this._currentframe = 1;
				}
			}
		} else {
			if (this._playing) {
				this._currentframe++;
				if (this._currentframe>this._frames.length) {
					this._currentframe = 1;
				}
			}	
		}
		this._previousframe = this._currentframe
	}
	this.play = function () {
		this._playing = true
	}
	this.stop = function () {
		//Timer.remove(this)
		this._playing = false
	}
}

function Shape() {
	// Not part of the ActionScript spec, but necessary.
	this._shapedata = []
	this.fill = "#000000"
	this._draw = function (frame,frame2,r) {
		if (!frame2) {
			this._x = frame._x
			this._y = frame._y
			this._xscale = frame._xscale
			this._yscale = frame._yscale
			this._rotation = frame._rotation
		} else {
			this._x = ave(frame2._x, frame._x, r)
			this._y = ave(frame2._y, frame._y, r)
			this._xscale = ave(frame2._xscale, frame._xscale, r)
			this._yscale = ave(frame2._yscale, frame._yscale, r)
			this._rotation = ave(frame2._rotation ,frame._rotation, r)
		}
		//log(this._x)
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

/* ------  ACTIONSCRIPT CLASSES  -------- */

function play() {
}
	
function stop() {
	_processingobj.stop();
}
	
/*--------------------------------------- */

var a = new Shape()
a._shapedata = [["M",0,0],["L",400,0],["L",400,200],["L",0,200],["L",0,0]]
var b = new MovieClip()
b.a = a
b._frames[0].a = {}
b._frames[0].a._x = 100
b._frames[0].a._y = 20
b._frames[0].actions = 'this.a._x = this.a._x + 1'
root.b = b
b._frames[50] = new Frame()
b._frames[50].a = {}
b._frames[50].a._x = 50
b._frames[50].a._y = 40
b._frames[100] = new Frame()
b._frames[100].a = {}
b._frames[100].a._x = 75
b._frames[100].actions = 'stop();'
b._frames[100].a._y = 120
b._frames[150] = new Frame()
b._frames[150].a = {}
b._frames[150].a._x = 100
b._frames[150].a._y = 20

setTimeout('b.play()',20)

//-------------------  END OF JAVASCRIPT ------------------------\\
</script>
</body>
</html>

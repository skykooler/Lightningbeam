//--------------------------  BEGIN JAVASCRIPT  --------------------------------\\

var fps = 50
//var fps = 10;
var cr;
var canvas;
var _processingobj;
var _lastmouse = [0,0];
var _global = {};

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
			_root._draw(_rootFrame)
			if (!(_lastmouse[0]==_root._xmouse&&_lastmouse[1]==_root._ymouse)) {
				// Mouse moved
				_root._onMouseMove()
			}
			for (i in this.funcs){
				this.funcs[i].onEnterFrame();
			}
			Buffers[1-DrawingBuffer].style.visibility='hidden';
			Buffers[DrawingBuffer].style.visibility='visible';
			_lastmouse=[_root._xmouse,_root._ymouse]
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


function getObjectClass(obj) {
	/* Returns the class name of the argument or undefined if
	   it's not a valid JavaScript object.
	*/
    if (obj && obj.constructor && obj.constructor.toString) {
        var arr = obj.constructor.toString().match(
            /function\s*(\w+)/);

        if (arr && arr.length == 2) {
            return arr[1];
        }
    }

    return undefined;
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
	this._layers = [new Layer(this)]
	this._currentframe = 1;
	this._playing = true;
	this._x = 0;
	this._y = 0;
	this._xscale = 1;
	this._yscale = 1;
	this._rotation = 0;
	Timer.add(this)
	
	
	
	///////////////////             TODO: RECAST THIS. ROOT AS MOVIECLIP. DRAW THROUGH HIEREARCHY
	
	
	this._draw = function (frame,frame2,r) {
		_processingobj = this
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
		/*for (var i in this) {
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
		}*/
		for (i in this._layers) {
			this._layers[i]._draw(this._currentframe,this)
		}
		/*if (this._frames[this._currentframe-1]) {
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
		}*/
		if (this._playing) {
			var lessthan=false
			for (var i=0; i<this._layers.length; i++) {
				if (this._layers[i]._frames.length>this._currentframe) {
					lessthan=true;
					this._currentframe++;
					break;
				}
			}
			if (!lessthan){
				this._currentframe = 1;
			}
		}
		cr.restore()
		this._previousframe = this._currentframe
		if (!frame2) {
			frame._x = this._x
			frame._y = this._y
			frame._xscale = this._xscale
			frame._yscale = this._yscale
			frame._rotation = this._rotation
		}
	}
	this.play = function () {
		this._playing = true
	}
	this.stop = function () {
		//Timer.remove(this)
		this._playing = false
	}
										// Implemented?
	this.onData = function () {				//No
	}
	this.onDragOut = function () {			//No
	}
	this.onDragOver = function () {			//No
	}
	this.onEnterFrame = function () {		//Yes
	}
	this.onKeyDown = function () {			//No
	}
	this.onKeyUp = function () {			//No
	}
	this.onKillFocus = function () {		//No
	}
	this.onLoad = function () {				//No
	}
	this._onMouseDown = function () {
		for (var i in this) {
			if (getObjectClass(this[i])=='MovieClip') {
				// TODO: Add bounds checking.
				this[i]._onMouseDown();
			}
		this.onMouseDown()
		}
	}
	this.onMouseDown = function () {		//No
	}
	this._onMouseMove = function () {
		for (var i in this) {
			if (getObjectClass(this[i])=='MovieClip') {
				// TODO: Add bounds checking.
				this[i]._onMouseMove();
			}
		this.onMouseMove()
		}
	}
	this.onMouseMove = function () {		//No
	}
	this.onMouseUp = function () {			//No
	}
	this.onPress = function () {			//No
	}
	this.onRelease = function () {			//No
	}
	this.onReleaseOutside = function () {	//No
	}
	this.onRollOut = function () {			//No
	}
	this.onRollOver = function () {			//No
	}
	this.onSetFocus = function () {			//No
	}
	this.onUnload = function () {			//No
	}
}

function Layer (parent) {
	this._frames = [new Frame()]
	this._parent = parent;
	this._draw = function (currentframe) {
		_processingobj = this
		cr.save()
		for (var i in this._parent) {
			if (this._frames[currentframe-1]==undefined) {
				for (var j=0; j<currentframe-1; j++) {
					if (this._frames[j]) {
						last = j
					}
				}
				for (var j=this._frames.length; j>currentframe-1; j--) {
					if (this._frames[j]) {
						next = j
					}
				}
				if (this._frames[last][i]) {
					this._parent[i]._draw(this._frames[last][i],this._frames[next][i],(currentframe-last)/(next-last));
				}
			}
			else {
				if (this._frames[currentframe-1][i]) {
					this._parent[i]._draw(this._frames[currentframe-1][i]);
				}
			}
		}
		if (this._frames[currentframe-1]) {
			if (this._parent._playing) {
				this._frames[currentframe-1].run_script()
			}
		}
		cr.restore()
	}
	this.stop = function () {
		this._parent.stop()
	}
	this.play = function () {
		this._parent.play()
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

var _rootFrame = new Frame()
var _root = new MovieClip()

_rootFrame._root = {}
_rootFrame._root._x = 50
_rootFrame._root._y = 40

/*if (canvas.getContext) {
	cr = canvas.getContext("2d");
}*/

var Buffers = [document.getElementById("canvas1"), document.getElementById("canvas2")]
var DrawingBuffer = 0

function draw() {
	
	if (canvas.getContext) {
		cr = canvas.getContext("2d");
		
		
		for (i in _root) {
			if (_root[i]._draw) {
				//_root[i]._draw(true)
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

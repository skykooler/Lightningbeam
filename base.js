//--------------------------  BEGIN JAVASCRIPT  --------------------------------\\

//var fps = 50
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

function trace(str) {
	//Placeholder
	log(str);
}

function _timerBase () {
	/* This provides the 'tick' by which all animations are run.
	Playing animations should have their ._draw() method added here;
	to stop them, call remove() on it. */
	this.funcs = {}
	this.add = function (item) {
		this.funcs[item._id]=item;
	}
	this.remove = function (item) {
		delete this.funcs[item._id];
	}
	this.nextime = new Date().getTime()
	this.iterate = function() {
		canvas = Buffers[DrawingBuffer];

		if (canvas.getContext) {
			cr = canvas.getContext("2d");
			cr.clearRect(0, 0, canvas.width, canvas.height);
			cr.beginPath()

			DrawingBuffer=1-DrawingBuffer;
			//canvas = Buffers[DrawingBuffer];
			for (i in this.funcs){
				if (!this.funcs[i]._loaded) {
					this.funcs[i].onLoad();
					this.funcs[i]._loaded = true;
				}
			}
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
			this.nextime = this.nextime+1000/fps
			setTimeout('Timer.iterate()', this.nextime-new Date().getTime())
		}
	}
	
	setTimeout('Timer.iterate()', 1000/fps)
	
	//setInterval('Timer.iterate()', 1000/fps)
}

function _eventBase () {
	this.funcs = {}
	this.add = function (item) {
		this.funcs[item._id]=item;
	}
	this.remove = function (item) {
		delete this.funcs[item._id];
	}
	this.doEvent = function (event) {
		for (i in this.funcs) {
			this.funcs[i][event]();
		}
	}
}

var Timer = new _timerBase()

var Event = new _eventBase()

function ave(x, y, fac) {
	//Weighted average. 
	//fac is the weight - 0.5 gives a standard average
	return y - fac*(y-x)
}

function decimalToHex(d, padding) {
    var hex = Number(d).toString(16);
    padding = typeof (padding) === "undefined" || padding === null ? padding = 2 : padding;

    while (hex.length < padding) {
        hex = "0" + hex;
    }

    return hex;
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
	this._visible = true;
	this._id = "MC"+Math.round(Math.random()*1000000000000)
	this._loaded = false
	Timer.add(this)
	Event.add(this)
	
	
	
	///////////////////             TODO: RECAST THIS. ROOT AS MOVIECLIP. DRAW THROUGH HIEREARCHY
	
	
	this._draw = function (frame,frame2,r) {
		_processingobj = this
		if (this._visible) {
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
			if (getObjectClass(this._layers[i])=="Layer"){
				this._layers[i]._draw(this._currentframe,this)
			}
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
	this.gotoAndPlay = function (frame) {
		this._playing = true;
		this._currentframe = frame;
	}
	this.gotoAndStop = function (frame) {
		this._playing = false;
		this._currentframe = frame;
	}
	this.prevFrame = function () {
		this.gotoAndStop(this._previousframe)
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
	this.fill = "#123456"
	this.line = "#FEDCBAFF".substr(0,7)
	this._draw = function (frame,frame2,r) {
		if (!frame2) {
			this._x = frame._x
			this._y = frame._y
			this._xscale = frame._xscale
			this._yscale = frame._yscale
			this._rotation = frame._rotation
			if (frame.fill) {
				this.filr = parseInt(parseInt(frame.fill.replace("#",""),16)/65536)
				this.filg = parseInt(parseInt(frame.fill.replace("#",""),16)/256)%256
				this.filb = parseInt(parseInt(frame.fill.replace("#",""),16))%256
				this.linr = parseInt(parseInt(frame.line.replace("#",""),16)/65536)
				this.ling = parseInt(parseInt(frame.line.replace("#",""),16)/256)%256
				this.linb = parseInt(parseInt(frame.line.replace("#",""),16))%256
				this.fill = "#"+decimalToHex(this.filr,2)+decimalToHex(this.filg,2)+decimalToHex(this.filb,2)
				this.line = "#"+decimalToHex(this.linr,2)+decimalToHex(this.ling,2)+decimalToHex(this.linb,2)
			}
		} else {
			this._x = ave(frame2._x, frame._x, r)
			this._y = ave(frame2._y, frame._y, r)
			this._xscale = ave(frame2._xscale, frame._xscale, r)
			this._yscale = ave(frame2._yscale, frame._yscale, r)
			this._rotation = ave(frame2._rotation ,frame._rotation, r)
			if (frame2.fill) {
				this.filr2 = parseInt(parseInt(frame2.fill.replace("#",""),16)/65536)
				this.filg2 = parseInt(parseInt(frame2.fill.replace("#",""),16)/256)%256
				this.filb2 = parseInt(parseInt(frame2.fill.replace("#",""),16))%256
				this.filra = parseInt(parseInt(frame.fill.replace("#",""),16)/65536)
				this.filga = parseInt(parseInt(frame.fill.replace("#",""),16)/256)%256
				this.filba = parseInt(parseInt(frame.fill.replace("#",""),16))%256
				this.filr = parseInt(ave(this.filr2, this.filra, r))
				this.filg = parseInt(ave(this.filg2, this.filga, r))
				this.filb = parseInt(ave(this.filb2, this.filba, r))
				this.fill = "#"+decimalToHex(this.filr,2)+decimalToHex(this.filg,2)+decimalToHex(this.filb,2)
			}
			if (frame2.line) {
				this.linr2 = parseInt(parseInt(frame2.line.replace("#",""),16)/65536)
				this.ling2 = parseInt(parseInt(frame2.line.replace("#",""),16)/256)%256
				this.linb2 = parseInt(parseInt(frame2.line.replace("#",""),16))%256
				this.linra = parseInt(parseInt(frame.line.replace("#",""),16)/65536)
				this.linga = parseInt(parseInt(frame.line.replace("#",""),16)/256)%256
				this.linba = parseInt(parseInt(frame.line.replace("#",""),16))%256
				this.linr = parseInt(ave(this.linr2, this.linra, r))
				this.ling = parseInt(ave(this.ling2, this.linga, r))
				this.linb = parseInt(ave(this.linb2, this.linba, r))
				this.line = "#"+decimalToHex(this.linr,2)+decimalToHex(this.ling,2)+decimalToHex(this.linb,2)
			}
		}
		//log(this._x)
		cr.save()
		cr.translate(this._x,this._y)
		cr.rotate(this._rotation*Math.PI/180)
		cr.scale(this._xscale*1.0, this._yscale*1.0)
		cr.fillStyle = this.fill.substr(0,7);
		cr.strokeStyle = this.line.substr(0,7);
		for (i in this._shapedata) {
			if (this._shapedata[i][0]=="M") {
				cr.moveTo(this._shapedata[i][1],this._shapedata[i][2])
			} else if (this._shapedata[i][0]=="L") {
				cr.lineTo(this._shapedata[i][1],this._shapedata[i][2])
			} else if (this._shapedata[i][0]=="C") {
				cr.bezierCurveTo(this._shapedata[i][1],this._shapedata[i][2],this._shapedata[i][3],this._shapedata[i][4],this._shapedata[i][5],this._shapedata[i][6])
			}
		}
		if (this.filled) {
			cr.stroke()
			cr.fill()
		} else {
			cr.stroke()
		}
		cr.restore()
		cr.beginPath()
	}
}

function TextField() {
	/*From the ActionScript reference:
	
	To create a text field dynamically, you do not use the new operator.
	Instead, you use MovieClip.createTextField(). The default size for a
	text field is 100 x 100 pixels. 
	 
	*/
	this._x = 0;
	this._y = 0;
	this.textHeight = 100;
	this.textWidth = 100;
	this.text = "";
	this.textColor = "#000000"
	this.borderColor = "#000000"
	this.backgroundColor = "#FFFFFF"
	this.border = false
	this.hwaccel = true		// Use the browser function for drawing text (faster)
	this._documentObject = document.createElement('div')
	document.getElementById('events').appendChild(this._documentObject)
	this._documentObject.style.zIndex = 10
	this._documentObject.style.position = 'absolute'
	//this._documentObject.style.color = 'rgba(255,255,255,0)'
	this._documentObject.innerHTML = this.text
	this._textFormat = new TextFormat()
	this._textFormat.size = 12
	this._draw = function(frame,frame2,r) {
		this._documentObject.innerHTML = this.text;
		this._width = this._documentObject.clientWidth
		this._height = this._documentObject.clientHeight
		this._documentObject.style.fontSize=this._textFormat.size+"px"
		if (!frame2) {
			this._x = frame._x
			this._y = frame._y
			this._xscale = frame._xscale
			this._yscale = frame._yscale
			this._rotation = frame._rotation
			if (frame.textColor) {
				this.tcolr = parseInt(parseInt(frame.textColor.replace("#",""),16)/65536)
				this.tcolg = parseInt(parseInt(frame.textColor.replace("#",""),16)/256)%256
				this.tcolb = parseInt(parseInt(frame.textColor.replace("#",""),16))%256
				this.bcolr = parseInt(parseInt(frame.borderColor.replace("#",""),16)/65536)
				this.bcolg = parseInt(parseInt(frame.borderColor.replace("#",""),16)/256)%256
				this.bcolb = parseInt(parseInt(frame.borderColor.replace("#",""),16))%256
				this.textColor = "#"+decimalToHex(this.tcolr,2)+decimalToHex(this.tcolg,2)+decimalToHex(this.tcolb,2)
				this.borderColor = "#"+decimalToHex(this.bcolr,2)+decimalToHex(this.bcolg,2)+decimalToHex(this.bcolb,2)
			}
		} 
		else {
			this._x = ave(frame2._x, frame._x, r)
			this._y = ave(frame2._y, frame._y, r)
			this._xscale = ave(frame2._xscale, frame._xscale, r)
			this._yscale = ave(frame2._yscale, frame._yscale, r)
			this._rotation = ave(frame2._rotation ,frame._rotation, r)
			if (frame2.textColor) {
				this.tcolr2 = parseInt(parseInt(frame2.textColor.replace("#",""),16)/65536)
				this.tcolg2 = parseInt(parseInt(frame2.textColor.replace("#",""),16)/256)%256
				this.tcolb2 = parseInt(parseInt(frame2.textColor.replace("#",""),16))%256
				this.tcolra = parseInt(parseInt(frame.textColor.replace("#",""),16)/65536)
				this.tcolga = parseInt(parseInt(frame.textColor.replace("#",""),16)/256)%256
				this.tcolba = parseInt(parseInt(frame.textColor.replace("#",""),16))%256
				this.tcolr = parseInt(ave(this.tcolr2, this.tcolra, r))
				this.tcolg = parseInt(ave(this.tcolg2, this.tcolga, r))
				this.tcolb = parseInt(ave(this.tcolb2, this.tcolba, r))
				this.textColor = "#"+decimalToHex(this.tcolr,2)+decimalToHex(this.tcolg,2)+decimalToHex(this.tcolb,2)
			}
			if (frame2.borderColor) {
				this.bcolr2 = parseInt(parseInt(frame2.line.replace("#",""),16)/65536)
				this.bcolg2 = parseInt(parseInt(frame2.line.replace("#",""),16)/256)%256
				this.bcolb2 = parseInt(parseInt(frame2.line.replace("#",""),16))%256
				this.bcolra = parseInt(parseInt(frame.line.replace("#",""),16)/65536)
				this.bcolga = parseInt(parseInt(frame.line.replace("#",""),16)/256)%256
				this.bcolba = parseInt(parseInt(frame.line.replace("#",""),16))%256
				this.bcolr = parseInt(ave(this.bcolr2, this.bcolra, r))
				this.bcolg = parseInt(ave(this.bcolg2, this.bcolga, r))
				this.bcolb = parseInt(ave(this.bcolb2, this.bcolba, r))
				this.borderColor = "#"+decimalToHex(this.bcolr,2)+decimalToHex(this.bcolg,2)+decimalToHex(this.bcolb,2)
			}
		}
		if (!this.hwaccel) {
			this._documentObject.style.color = 'rgba(255,255,255,0)'
			cr.save()
			cr.translate(this._x,this._y)
			cr.rotate(this._rotation*Math.PI/180)
			cr.scale(this._xscale*1.0, this._yscale*1.0)
			cr.fillStyle = this.textColor.substr(0,7);
			cr.strokeStyle = this.borderColor.substr(0,7);
			cr.textBaseline = 'top'
			if (this._textFormat.font) {
				if (this._textFormat.size){
					cr.font = this._textFormat.size+"pt "+this._textFormat.font;
					this._documentObject.style.font = this._textFormat.size+"pt "+this._textFormat.font;
				} else {
					cr.font = "12pt "+this._textFormat.font;
					this._documentObject.style.font = "12pt "+this._textFormat.font;
				}
			} else if (this._textFormat.size){
				cr.font = this._textFormat.size+"pt Times New Roman"
				this._documentObject.style.font = this._textFormat.size+"pt Times New Roman"
			} else {
				cr.font = "12pt Times New Roman"
				this._documentObject.style.font = "12pt Times New Roman"
			}
			cr.fillText(this.text, 0, 0)
			if (this.border) {
				cr.beginPath()
				cr.moveTo(0,0)
				cr.lineTo(this._width,0)
				cr.lineTo(this._width,this._height)
				cr.lineTo(0,this._height)
				cr.lineTo(0,0)
				cr.stroke()
			}
			cr.restore()
			cr.beginPath()
		}
		else {
			if (this._textFormat.font) {
				if (this._textFormat.size){
					cr.font = this._textFormat.size+"pt "+this._textFormat.font;
					this._documentObject.style.font = this._textFormat.size+"pt "+this._textFormat.font;
				} else {
					cr.font = "12pt "+this._textFormat.font;
					this._documentObject.style.font = "12pt "+this._textFormat.font;
				}
			} else if (this._textFormat.size){
				cr.font = this._textFormat.size+"pt Times New Roman"
				this._documentObject.style.font = this._textFormat.size+"pt Times New Roman"
			} else {
				cr.font = "12pt Times New Roman"
				this._documentObject.style.font = "12pt Times New Roman"
			}
		}
		this._documentObject.style.left = this._x
		this._documentObject.style.top = this._y
		
		
	}
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

Object.prototype.addProperty = function ( name, getter, setter) {
	this.__defineGetter__(name,getter)
	if (setter) {
		this.__defineSetter__(name,setter)
	}
}

Object.prototype.isPropertyEnumerable = function (name) {
	return this.propertyIsEnumerable(name);
}

Object.defineProperty( Object.prototype, "addProperty", {enumerable: false});
Object.defineProperty( Object.prototype, "isPropertyEnumerable", {enumerable: false});

function Point (x, y) {
	this.x = x
	this.y = y
	this.getlength = function () {
		return Math.sqrt(this.x*this.x + this.y*this.y)
	}
	this.addProperty('length',this.getlength)
	this.add = function (v) {
		return Point (this.x+v.x, this.y+v.y)
	}
	this.clone = function () {
		return Point (this.x, this.y)
	}
	this.equals = function (toCompare) {
		return (this.x==toCompare.x && this.y==toCompare.y)
	}
	this.normalize = function (length) {
		x = this.x/((this.length*1.0))*length
		y = this.y/((this.length*1.0))*length
		this.x = x
		this.y = y
	}
	this.offset = function (dx, dy) {
		this.x += dx
		this.y += dy
	}
	this.subtract = function (v) {
		return Point(this.x-v.x, this.y-v.y)
	}
	this.toString = function () {
		return "(x="+this.x+", y="+this.y+")"
	}
}
Point.distance = function (pt1, pt2) {
	return Math.sqrt((pt2.x-pt1.x)*(pt2.x-pt1.x) + (pt2.y-pt1.y)*(pt2.y-pt1.y))
}
Point.interpolate = function (pt1, pt2, f) {
	return Point(ave (pt1.x, pt2.x, f), ave (pt1.y, pt2.y, f) )
}
Point.polar = function (len, angle) {
	return Point(len*Math.cos(angle), len*Math.sin(angle))
}

function Rectangle (x, y, width, height) {
	this.x = x
	this.y = y
	this.width = width
	this.height = height
	this.getbottom = function () {
		return this.y+this.height;
	}
	this.getbottomRight = function () {
		return Point(this.x + this.width, this.y + this.height)
	}
	this.getsize = function () {
		return Point(this.width, this.height)
	}
	this.getleft = function () {
		return this.x
	}
	this.getright = function () {
		return this.x + this.width
	}
	this.gettop = function () {
		return this.y
	}
	this.gettopLeft = function () {
		return Point(this.x, this.y)
	}
	this.addProperty('bottom',this.getbottom);
	this.addProperty('bottomRight',this.getbottomRight);
	this.addProperty('size',this.getsize);
	this.addProperty('left',this.getleft);
	this.addProperty('right',this.getright);
	this.addProperty('top',this.gettop);
	this.addProperty('topLeft',this.gettopLeft);
	this.clone = function () {
		return Rectangle(this.x, this.y, this.width, this.height);
	}
	this.contains = function (x, y) {
		return ((x>this.x && x<this.right) && y>(this.y && y<this.bottom))
	}
	this.containsPoint = function (pt) {
		return ((pt.x>this.x && pt.x<this.right) && (pt.y>this.y && pt.y<this.bottom))
	}
	this.containsRectangle = function (rect) {
		return ((rect.x>this.x && rect.right<this.right) && (rect.y>this.y && rect.bottom<this.bottom))
	}
	this.equals = function (toCompare) {
		return ((toCompare.x==this.x && toCompare.y==this.y) && (toCompare.width==this.width && toCompare.height==this.height))
	}
	this.inflate = function (dx, dy) {
		this.x -= dx;
		this.width += 2 * dx;
		this.y -= dy;
		this.height += 2 * dy;
	}
	this.inflatePoint = function (pt) {
		this.x -= pt.x;
		this.width += 2 * pt.x;
		this.y -= pt.y;
		this.height += 2 * pt.y;
	}
	this.intersection = function (toIntersect) {
		x = Math.max(this.x, toIntersect.x);
		y = Math.max(this.y, toIntersect.y);
		right = Math.min(this.right, toIntersect.right)
		bottom = Math.min(this.bottom, toIntersect.bottom)
		if (right>x && bottom>y) {
			return Rectangle(x, y, right-x, bottom-y)
		} else {
			return Rectangle (0, 0, 0, 0)
		}
	}
	this.intersects = function (toIntersect) {
		x = Math.max(this.x, toIntersect.x);
		y = Math.max(this.y, toIntersect.y);
		right = Math.min(this.right, toIntersect.right)
		bottom = Math.min(this.bottom, toIntersect.bottom)
		if (right>x && bottom>y) {
			return true
		} else {
			return false
		}
	}
	this.isEmpty = function () {
		if (this.width<=0) {
			return true
		} else if (this.height<=0) {
			return true
		} else {
			return false
		}
	}
	this.offset = function (dx, dy) {
		this.x += dx;
		this.y += dy;
	}
	this.offsetPoint = function (pt) {
		this.x += pt.x;
		this.y += pt.y;
	}
	this.setEmpty = function () {
		this.x = 0;
		this.y = 0;
		this.width = 0;
		this.height = 0;
	}
	this.toString = function () {
		return "(x="+this.x+", y="+this.y+", w="+this.width+", h="+this.height+")"
	}
	this.union = function (toUnion) {
		x = Math.min(this.x, toUnion.x);
		y = Math.min(this.y, toUnion.y);
		right = Math.max(this.right, toUnion.right)
		bottom = Math.max(this.bottom, toUnion.bottom)
		return Rectangle(x, y, right-x, bottom-y)
	}
}

var Stage = function () {}
function getheight () {
	return canvas.height
}
function getwidth () {
	return canvas.width
}
Stage.addProperty('height',getheight)
Stage.addProperty('width', getwidth)
//TODO: Various Stage methods

var Key = {}
Key.BACKSPACE = 8
Key.CAPSLOCK = 20
Key.CONTROL = 17
Key.DELETEKEY = 46
Key.DOWN = 40
Key.END = 35
Key.ENTER = 13
Key.ESCAPE = 27
Key.HOME = 36
Key.INSERT = 45
Key.LEFT = 37
Key.PGDN = 34
Key.PGUP = 33
Key.RIGHT = 39
Key.SHIFT = 16
Key.SPACE = 32
Key.TAB = 9
Key.UP = 38
Key.mask = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]
Key.press = function (key) {
	this.keyCode = key.keyCode;
	this.ascii = key.charCode;
	this.mask[key.keyCode] = 1;
}
Key.release = function (key) {
	this.mask[key.keyCode] = 0;
}
Key.isDown = function (code) {
	return (this.mask[code]==1)
}
Key.getCode = function() {
	return this.keyCode;
}
Key.getAscii = function() {
	return this.ascii;
}
Key.addListener = function(listener) {
	Event.add(listener)
}
Key.removeListener = function(listener) {
	Event.remove(listener)
}

function TextFormat () {
	this.align="left"	//Yes, I know it's supposed to be 'undefined', but that defaults to 'left'
	this.blockIndent=0
	this.bold = false
	this.bullet = null
	this.color = 0x000000	//hmm...
	this.font = null
	this.indent = 0
	this.italic = false
	this.kerning = false // And I doubt I will implement it since even Adobe doesn't on OSX...
	this.leading = 0 // TODO: research this in CSS
	this.leftMargin = 0
	this.letterSpacing = 0
	this.rightMargin = 0
	this.size = null // Default value is null? WTF?
	this.tabStops = new Array()
	this.target = "_self" //ActionScript docs specify no default value - find out what it is
	this.underline = false
	this.url = null
}

function SharedObject () {
	this.data = {}
	this.flush = function () {
		localStorage.setItem(this._name, this.data)
	}
	this.clear = function () {
		localStorage.removeItem(this._name)
		for (i in this) {
			this[i] = undefined
		}
	}
	this.getSize = function () {
		//This may not be byte-exact, but it should be close enough.
		return JSON.stringify(this.data).length
	}
	this.setFps = function () {
		//TODO: first understand this. Then, implement it!
	}
	Object.defineProperty(this, 'flush', {enumerable:false})
	Object.defineProperty(this, 'clear', {enumerable:false})
	Object.defineProperty(this, '_name', {enumerable:false})
	Object.defineProperty(this, 'getSize', {enumerable:false})
	Object.defineProperty(this, 'setFps', {enumerable:false})
}
SharedObject.list = {}
for (var i in localStorage) {
	SharedObject.list[i] = new SharedObject()
	SharedObject.list[i]._name = i
	SharedObject.list[i].data = localStorage[i]
}
//TODO: Remote shared objects
SharedObject.getLocal = function (name, localPath, secure) {
	if (name in SharedObject.list) {
		return SharedObject.list[name]
	}
	else {
		var so = new SharedObject()
		so._name = name
		SharedObject.list[name] = so
		return so
	}
	//TODO: localPath should allow changing domain access; secure should force HTTPS.
}

//TODO: ContextMenu



	
/*--------------------------------------- */

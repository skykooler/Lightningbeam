<html>
<head>
</head>
<body>
<canvas id="canvas" width=500 height=500></canvas>
<script>
//--------------------------  BEGIN JAVASCRIPT  --------------------------------\\


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
	this._frames = [[]]
	this._currentframe = 1;
	this._draw = function () {
		for (i in this) {
			if (this._frames[this._currentframe-1][i]) {
				this[i]._draw(this._frames[this._currentframe-1][i]);
			}
		}
	}
	this.play = function () {
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
var cr

function draw() {
	var canvas = document.getElementById("canvas");

	if (canvas.getContext) {
		cr = canvas.getContext("2d");
		
		cr.strokeSyle = "#0000FF"
		
		for (i in root) {
			if (root[i]._draw) {
				root[i]._draw()
			}
		}
	}
}

function play() {

	

var a = new Shape()
a._shapedata = [["M",0,0],["L",400,0],["L",400,200],["L",0,200],["L",0,0]]
var b = new MovieClip()
b.a = a
b._frames[0].a = {}
b._frames[0].a._x = 100
b._frames[0].a._y = 20
b._frames[0].a._xscale = 1
b._frames[0].a._xscale = 1
b._frames[0].a._rotation = 0
root.b = b
b._frames[1].a = {}
b._frames[1].a._x = 50
b._frames[1].a._y = 40
b._frames[1].a._xscale = 1
b._frames[1].a._xscale = 1
b._frames[1].a._rotation = 0
play()

//-------------------  END OF JAVASCRIPT ------------------------\\
</script>
</body>
</html>
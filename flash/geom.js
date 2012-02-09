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

function radianToDegree(angle) { return angle * (180.0 / Math.PI); }
function degreeToRadian(angle) { return Math.PI * angle / 180.0; }

function Matrix(a, b, c, d, tx, ty) {
	this.elements = [a||1, c||0, tx||0, 
					 b||0, d||1, ty||0];

	this.__defineGetter__("a", function() { return this.elements[0]; });  
	this.__defineSetter__("a", function(n) { this.elements[0]=n; });  
	this.__defineGetter__("b", function() { return this.elements[3]; });  
	this.__defineSetter__("b", function(n) { this.elements[3]=n; });
	this.__defineGetter__("c", function() { return this.elements[1]; });  
	this.__defineSetter__("c", function(n) { this.elements[1]=n; });
	this.__defineGetter__("d", function() { return this.elements[4]; });  
	this.__defineSetter__("d", function(n) { this.elements[4]=n; });
	this.__defineGetter__("tx", function() { return this.elements[2]; });  
	this.__defineSetter__("tx", function(n) { this.elements[2]=n; });
	this.__defineGetter__("ty", function() { return this.elements[5]; });  
	this.__defineSetter__("ty", function(n) { this.elements[5]=n; });
	
	this.clone = function() {	
	};
	
	this.concat = function(m) {	
	};
	
	this.identity = function() {
		this.elements = [1, 0, 0, 1, 0, 0];
	};
	
	this.scale = function(sx, sy) {
		if (sx && !sy) {
			sy = sx;
		}
		if (sx && sy) {
			this.elements[0] *= sx;
			this.elements[1] *= sy;
			this.elements[3] *= sx;
			this.elements[4] *= sy;
		}
	};
	
	this.translate = function(dx, dy) {
		this.elements[2] = dx * this.elements[0] + dy * this.elements[1] + this.elements[2];
		this.elements[5] = dx * this.elements[3] + dy * this.elements[4] + this.elements[5];
	};
	
	this.angle = 0; // faster but dumber method
	
	this.rotate = function(angle) {
		this.angle += angle;
		
		r = radianToDegree(angle);
		c = Math.cos(angle);
		s = Math.sin(angle);
		
		temp1 = this.elements[0];
		temp2 = this.elements[1];
		this.elements[0] =  c * temp1 + s * temp2;
		this.elements[1] = -s * temp1 + c * temp2;
		
		temp1 = this.elements[3];
		temp2 = this.elements[4];
		this.elements[3] =  c * temp1 + s * temp2;
		this.elements[4] = -s * temp1 + c * temp2;
		
	};
	
	
}

function ColorTransform(redMultiplier, greenMultiplier, blueMultiplier, alphaMultiplier, redOffset, greenOffset, blueOffset, alphaOffset) {
	this.redMultiplier=redMultiplier==undefined?1:redMultiplier;
	this.greenMultiplier=greenMultiplier==undefined?1:greenMultiplier;
	this.blueMultiplier=blueMultiplier==undefined?1:blueMultiplier;
	this.alphaMultiplier=alphaMultiplier==undefined?1:alphaMultiplier;
	this.redOffset=redOffset || 0;
	this.greenOffset=greenOffset || 0;
	this.blueOffset=blueOffset || 0;
	this.alphaOffset=alphaOffset || 0;
	this.concat = function (second) {
		this.redMultiplier=this.redMultiplier*second.redMultiplier;
		this.greenMultiplier=this.greenMultiplier*second.greenMultiplier;
		this.blueMultiplier=this.blueMultiplier*second.blueMultiplier;
		this.alphaMultiplier=this.alphaMultiplier*second.alphaMultiplier;
		this.redOffset=this.redOffset+second.redOffset;
		this.greenOffset=this.redOffset+second.greenOffset;
		this.blueOffset=this.redOffset+second.blueOffset;
		this.alphaOffset=this.redOffset+second.alphaOffset;
	}
	this.toString = function () {
		return "(redMultiplier="+this.redMultiplier+", greenMultiplier="+this.greenMultiplier+
				", blueMultiplier="+this.blueMultiplier+", alphaMultiplier="+this.alphaMultiplier+
				", redOffset="+this.redOffset+", greenOffset="+this.greenOffset+
				", blueOffset="+this.blueOffset+", alphaOffset="+this.alphaOffset+")"
	}
}
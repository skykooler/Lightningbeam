function BitmapFilter () {
	this.clone=function() {
		return new BitmapFilter();
	}
}

function ColorMatrixFilter(matrix) {
	this.matrix=matrix || [
							1, 0, 0, 0, 0,
							0, 1, 0, 0, 0,
							0, 0, 1, 0, 0,
							0, 0, 0, 1, 0
						  ];
	
	this.run=function(sourceRect, image, copy) {
		var numPixel=image.length/4;
		var m=this.matrix;
		
		for(var i=0;i<numPixel;i++) {
			var r=i*4;
			var g=r+1;
			var b=r+2;
			var a=r+3;
			
			var oR=image[r];
			var oG=image[g];
			var oB=image[b];
			var oA=image[a];
			
			image[r] = (m[0]  * oR) + (m[1]  * oG) + (m[2]  * oB) + (m[3]  * oA) + m[4];
	 		image[g] = (m[5]  * oR) + (m[6]  * oG) + (m[7]  * oB) + (m[8]  * oA) + m[9];
	 		image[b] = (m[10] * oR) + (m[11] * oG) + (m[12] * oB) + (m[13] * oA) + m[14];
	 		image[a] = (m[15] * oR) + (m[16] * oG) + (m[17] * oB) + (m[18] * oA) + m[19];
		}
	}
	
	this.clone=function() {
		return new ColorMatrixFilter(this.matrix);
	}
}

ColorMatrixFilter.inherits(BitmapFilter)

function BlurFilter (blurX, blurY, quality) {
	this.blurX = blurX
	this.blurY = blurY
	this.quality = quality
	this.run = function(sourceRect, image, copy) {
		var numPixel=image.length/4;
		
		for(var i=0;i<numPixel;i++) {
			var r=i*4;
			var g=r+1;
			var b=r+2;
			var a=r+3;
			
			var oR=image[r];
			var oG=image[g];
			var oB=image[b];
			var oA=image[a];
			
			image[r] = (m[0]  * oR) + (m[1]  * oG) + (m[2]  * oB) + (m[3]  * oA) + m[4];
	 		image[g] = (m[5]  * oR) + (m[6]  * oG) + (m[7]  * oB) + (m[8]  * oA) + m[9];
	 		image[b] = (m[10] * oR) + (m[11] * oG) + (m[12] * oB) + (m[13] * oA) + m[14];
	 		image[a] = (m[15] * oR) + (m[16] * oG) + (m[17] * oB) + (m[18] * oA) + m[19];
		}
		radius = (blurX+blurY)/2
		if ( isNaN(radius) || radius < 1 ) return;
		radius |= 0;
	
		var x, y, i, p, yp, yi, yw, r_sum, g_sum, b_sum, a_sum, 
		r_out_sum, g_out_sum, b_out_sum, a_out_sum,
		r_in_sum, g_in_sum, b_in_sum, a_in_sum, 
		pr, pg, pb, pa, rbs;
				
		var width = sourceRect.width
		var height = sourceRect.height
		var div = radius + radius + 1;
		var w4 = width << 2;
		var widthMinus1  = width - 1;
		var heightMinus1 = height - 1;
		var radiusPlus1  = radius + 1;
		var sumFactor = radiusPlus1 * ( radiusPlus1 + 1 ) / 2;
	
		var stackStart = new BlurStack();
		var stack = stackStart;
		for ( i = 1; i < div; i++ ) {
			stack = stack.next = new BlurStack();
			if ( i == radiusPlus1 ) var stackEnd = stack;
		}
		stack.next = stackStart;
		var stackIn = null;
		var stackOut = null;
	
		yw = yi = 0;
	
		var mul_sum = mul_table[radius];
		var shg_sum = shg_table[radius];
	
		for ( y = 0; y < height; y++ ) {
			r_in_sum = g_in_sum = b_in_sum = a_in_sum = r_sum = g_sum = b_sum = a_sum = 0;
			r_out_sum = radiusPlus1 * ( pr = image[yi] );
			g_out_sum = radiusPlus1 * ( pg = image[yi+1] );
			b_out_sum = radiusPlus1 * ( pb = image[yi+2] );
			a_out_sum = radiusPlus1 * ( pa = image[yi+3] );
			r_sum += sumFactor * pr;
			g_sum += sumFactor * pg;
			b_sum += sumFactor * pb;
			a_sum += sumFactor * pa;
		
			stack = stackStart;
			for( i = 0; i < radiusPlus1; i++ ) {
				stack.r = pr;
				stack.g = pg;
				stack.b = pb;
				stack.a = pa;
				stack = stack.next;
			}
		
			for( i = 1; i < radiusPlus1; i++ ) {
				p = yi + (( widthMinus1 < i ? widthMinus1 : i ) << 2 );
				r_sum += ( stack.r = ( pr = image[p])) * ( rbs = radiusPlus1 - i );
				g_sum += ( stack.g = ( pg = image[p+1])) * rbs;
				b_sum += ( stack.b = ( pb = image[p+2])) * rbs;
				a_sum += ( stack.a = ( pa = image[p+3])) * rbs;
				r_in_sum += pr;
				g_in_sum += pg;
				b_in_sum += pb;
				a_in_sum += pa;
			
				stack = stack.next;
			}
		
		
		stackIn = stackStart;
		stackOut = stackEnd;
		for ( x = 0; x < width; x++ ) {
			image[yi+3] = pa = (a_sum * mul_sum) >> shg_sum;
			if ( pa != 0 ) {
				pa = 255 / pa;
				image[yi]   = ((r_sum * mul_sum) >> shg_sum) * pa;
				image[yi+1] = ((g_sum * mul_sum) >> shg_sum) * pa;
				image[yi+2] = ((b_sum * mul_sum) >> shg_sum) * pa;
			} else {
				image[yi] = image[yi+1] = image[yi+2] = 0;
			}
			
			r_sum -= r_out_sum;
			g_sum -= g_out_sum;
			b_sum -= b_out_sum;
			a_sum -= a_out_sum;
			
			r_out_sum -= stackIn.r;
			g_out_sum -= stackIn.g;
			b_out_sum -= stackIn.b;
			a_out_sum -= stackIn.a;
			
			p =  ( yw + ( ( p = x + radius + 1 ) < widthMinus1 ? p : widthMinus1 ) ) << 2;
			
			r_in_sum += ( stackIn.r = image[p]);
			g_in_sum += ( stackIn.g = image[p+1]);
			b_in_sum += ( stackIn.b = image[p+2]);
			a_in_sum += ( stackIn.a = image[p+3]);
			
			r_sum += r_in_sum;
			g_sum += g_in_sum;
			b_sum += b_in_sum;
			a_sum += a_in_sum;
			
			stackIn = stackIn.next;
			
			r_out_sum += ( pr = stackOut.r );
			g_out_sum += ( pg = stackOut.g );
			b_out_sum += ( pb = stackOut.b );
			a_out_sum += ( pa = stackOut.a );
			
			r_in_sum -= pr;
			g_in_sum -= pg;
			b_in_sum -= pb;
			a_in_sum -= pa;
			
			stackOut = stackOut.next;

			yi += 4;
		}
		yw += width;
	}

	
		for ( x = 0; x < width; x++ ) {
			g_in_sum = b_in_sum = a_in_sum = r_in_sum = g_sum = b_sum = a_sum = r_sum = 0;
			
			yi = x << 2;
			r_out_sum = radiusPlus1 * ( pr = image[yi]);
			g_out_sum = radiusPlus1 * ( pg = image[yi+1]);
			b_out_sum = radiusPlus1 * ( pb = image[yi+2]);
			a_out_sum = radiusPlus1 * ( pa = image[yi+3]);
			
			r_sum += sumFactor * pr;
			g_sum += sumFactor * pg;
			b_sum += sumFactor * pb;
			a_sum += sumFactor * pa;
			
			stack = stackStart;
			
			for( i = 0; i < radiusPlus1; i++ ) {
				stack.r = pr;
				stack.g = pg;
				stack.b = pb;
				stack.a = pa;
				stack = stack.next;
			}
			
			yp = width;
			
			for( i = 1; i <= radius; i++ ) {
				yi = ( yp + x ) << 2;
				
				r_sum += ( stack.r = ( pr = image[yi])) * ( rbs = radiusPlus1 - i );
				g_sum += ( stack.g = ( pg = image[yi+1])) * rbs;
				b_sum += ( stack.b = ( pb = image[yi+2])) * rbs;
				a_sum += ( stack.a = ( pa = image[yi+3])) * rbs;
			   
				r_in_sum += pr;
				g_in_sum += pg;
				b_in_sum += pb;
				a_in_sum += pa;
				
				stack = stack.next;
			
				if( i < heightMinus1 )
				{
					yp += width;
				}
			}
			
			yi = x;
			stackIn = stackStart;
			stackOut = stackEnd;
			for ( y = 0; y < height; y++ )
			{
				p = yi << 2;
				image[p+3] = pa = (a_sum * mul_sum) >> shg_sum;
				if ( pa > 0 )
				{
					pa = 255 / pa;
					image[p]   = ((r_sum * mul_sum) >> shg_sum ) * pa;
					image[p+1] = ((g_sum * mul_sum) >> shg_sum ) * pa;
					image[p+2] = ((b_sum * mul_sum) >> shg_sum ) * pa;
				} else {
					image[p] = image[p+1] = image[p+2] = 0;
				}
				
				r_sum -= r_out_sum;
				g_sum -= g_out_sum;
				b_sum -= b_out_sum;
				a_sum -= a_out_sum;
			   
				r_out_sum -= stackIn.r;
				g_out_sum -= stackIn.g;
				b_out_sum -= stackIn.b;
				a_out_sum -= stackIn.a;
				
				p = ( x + (( ( p = y + radiusPlus1) < heightMinus1 ? p : heightMinus1 ) * width )) << 2;
				
				r_sum += ( r_in_sum += ( stackIn.r = image[p]));
				g_sum += ( g_in_sum += ( stackIn.g = image[p+1]));
				b_sum += ( b_in_sum += ( stackIn.b = image[p+2]));
				a_sum += ( a_in_sum += ( stackIn.a = image[p+3]));
			   
				stackIn = stackIn.next;
				
				r_out_sum += ( pr = stackOut.r );
				g_out_sum += ( pg = stackOut.g );
				b_out_sum += ( pb = stackOut.b );
				a_out_sum += ( pa = stackOut.a );
				
				r_in_sum -= pr;
				g_in_sum -= pg;
				b_in_sum -= pb;
				a_in_sum -= pa;
				
				stackOut = stackOut.next;
				
				yi += width;
			}
		}
		
		//context.putImageData( imageData, top_x, top_y );
			
		function BlurStack() {
			this.r = 0;
			this.g = 0;
			this.b = 0;
			this.a = 0;
			this.next = null;
		}
	}
	this.clone = function () {
		return new BlurFilter(this.blurX, this.blurY, this.quality)
	}
}

BlurFilter.inherits(BitmapFilter)
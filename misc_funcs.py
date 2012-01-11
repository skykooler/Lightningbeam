#! /usr/bin/env python
# -*- coding:utf-8 -*-
# © 2012 Skyler Lehmkuhl
# Released under the GPLv3. For more information, see gpl.txt.

import svlgui
from threading import Event, Thread
import math

def select_any(self):
	svlgui.MODE = " "
	svlgui.set_cursor("arrow", stage)
def resize_any(self):
	svlgui.MODE = "s"
	svlgui.set_cursor("arrow", stage)
def lasso(self):
	svlgui.MODE = "l"
	svlgui.set_cursor("lasso", stage)
def text(self):
	svlgui.MODE = "t"
	svlgui.set_cursor("text", stage)
def rectangle(self):
	svlgui.MODE = "r"
	svlgui.set_cursor("crosshair", stage)
def ellipse(self):
	svlgui.MODE = "e"
	svlgui.set_cursor("crosshair", stage)
def curve(self):
	svlgui.MODE = "c"
	svlgui.set_cursor("curve", stage)
def paintbrush(self):
	svlgui.MODE = "p"
	svlgui.set_cursor("paintbrush", stage)
def pen(self):
	svlgui.MODE = "n"
	svlgui.set_cursor("pen", stage)
def paint_bucket(self):
	svlgui.MODE = "b"
	svlgui.set_cursor("bucket", stage)
	
def box(x, y, width, height, fill):
	global objects
	box = svlgui.Shape(x, y)
	box.shapedata = [["M",0,0],["L",width,0],["L",width,height],["L",0,height],["L",0,0]]
	box.fillcolor = svlgui.Color(fill)
	box.linecolor = svlgui.Color("#cccccc")
	box.filled = True
	return box


def lastval(arr,index):
	for i in reversed(arr[:index]):
		if i:
			return i
	




def catmullRom2bezier( points ) :
	#crp = points.split(/[,\s]/);
	crp = points
	d = [["M",points[0][1],points[0][2]]];
	'''
	i = 0
	iLen = len(crp); 
	while iLen - 2 > i:
		i+=2
		p = [];
		if 0 == i:
			p.append( [crp[i],   crp[i+1]);
			p.append( [crp[i],   crp[i+1]);
			p.append( [crp[i+2], crp[i+3]);
			p.append( [crp[i+4], crp[i+5]);
		elif iLen - 4 == i:
			p.append( [crp[i-2], crp[i-1]]);
			p.append( [crp[i],   crp[i+1]]);
			p.append( [crp[i+2], crp[i+3]]);
			p.append( [crp[i+2], crp[i+3]]);
		} else {
			p.append( [crp[i-2], crp[i-1]]);
			p.append( [crp[i],   crp[i+1]]);
			p.append( [crp[i+2], crp[i+3]]);
			p.append( [crp[i+4], crp[i+5]]);
		}
		'''
	for i in range(2,len(crp)-2):
		p = [ [crp[i-1][1],crp[i-1][2]], [crp[i][1],crp[i][2]], [crp[i+1][1],crp[i+1][2]], [crp[i+2][1],crp[i+2][2]] ]

		# Catmull-Rom to Cubic Bezier conversion matrix 
		#    0       1       0       0
		#  -1/6      1      1/6      0
		#    0      1/6      1     -1/6
		#    0       0       1       0

		bp = []
		bp.append( [p[1][0],  p[1][1]] );
		bp.append( [(-p[0][0]+6*p[1][0]+ p[2][0])/6, ((-p[0][1]+6*p[1][1]+p[2][1])/6)]);
		bp.append( [(p[1][0]+6*p[2][0]-p[3][0])/6, ((p[1][1]+6*p[2][1]-p[3][1])/6)]);
		bp.append( [ p[2][0],  p[2][1] ] );

		
		d.append( ["C", bp[1][0], bp[1][1], bp[2][0], bp[2][1], bp[3][0], bp[3][1]]);
	
	return d;
	
def simplify_shape(shape,mode,iterations):
	if mode in ("straight","smooth"):
		for i in xrange(iterations):
			for j in reversed(range(len(shape))):
				if j>0 and j<len(shape)-1:
					pax=shape[j-1][1];
					pay=shape[j-1][2];
					pbx=shape[j][1];
					pby=shape[j][2];
					pcx=shape[j+1][1];
					pcy=shape[j+1][2];
					abx=pax-pbx;
					aby=pay-pby;
					#____________calculate ab,bc,ca, Angles A, B, c _________________________
					ab=math.sqrt(abx*abx+aby*aby);
					bcx=pbx-pcx;
					bcy=pby-pcy;
					bc=math.sqrt(bcx*bcx+bcy*bcy);
					cax=pcx-pax;
					cay=pcy-pay;
					ca=math.sqrt(cax*cax+cay*cay);
					cosB=-(ca*ca-bc*bc-ab*ab)/(2*bc*ab);
					try:
						acosB=math.acos(cosB)*180/math.pi;
					except ValueError:
						acosB=0
					if acosB>(165-500/(ab+bc)):	# at least 15 degrees away from straight angle
						del shape[j]
		if mode=="smooth":
			shape = catmullRom2bezier([shape[0]]*2+shape+[shape[-1]])
			print shape
							
	return shape#+nshape
	
# Timer module - not mine

# Copyright (c) 2009 Geoffrey Foster
# 
# Permission is hereby granted, free of charge, to any person
# obtaining a copy of this software and associated documentation
# files (the "Software"), to deal in the Software without
# restriction, including without limitation the rights to use,
# copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the
# Software is furnished to do so, subject to the following
# conditions:
# 
# The above copyright notice and this permission notice shall be
# included in all copies or substantial portions of the Software.
# 
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
# EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
# OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
# NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
# HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
# WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
# FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
# OTHER DEALINGS IN THE SOFTWARE.

class RepeatTimer(Thread):
    def __init__(self, interval, function, iterations=0, args=[], kwargs={}):
        Thread.__init__(self)
        self.interval = interval
        self.function = function
        self.iterations = iterations
        self.args = args
        self.kwargs = kwargs
        self.finished = Event()
 
    def run(self):
        count = 0
        while not self.finished.isSet() and (self.iterations <= 0 or count < self.iterations):
			try:
				self.finished.wait(self.interval)
				if not self.finished.isSet():
					#print self.function
					self.function(*self.args, **self.kwargs)
					count += 1 
			except Exception:
				self.cancel()
 
    def cancel(self):
        self.finished.set()

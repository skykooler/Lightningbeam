#! /usr/bin/env python
# -*- coding:utf-8 -*-
# © 2012 Skyler Lehmkuhl
# Released under the GPLv3. For more information, see gpl.txt.

import svlgui
from threading import Event, Thread
from itertools import tee, izip
import math
import subprocess 
import re
import os
import sys

def select_any(self):
	svlgui.MODE = " "
	svlgui.set_cursor("arrow", stage)
	update_tooloptions()
def resize_any(self):
	svlgui.MODE = "s"
	svlgui.set_cursor("arrow", stage)
	update_tooloptions()
def lasso(self):
	svlgui.MODE = "l"
	svlgui.set_cursor("lasso", stage)
	update_tooloptions()
def text(self):
	svlgui.MODE = "t"
	svlgui.set_cursor("text", stage)
	update_tooloptions()
def rectangle(self):
	svlgui.MODE = "r"
	svlgui.set_cursor("crosshair", stage)
	update_tooloptions()
def ellipse(self):
	svlgui.MODE = "e"
	svlgui.set_cursor("crosshair", stage)
	update_tooloptions()
def curve(self):
	svlgui.MODE = "c"
	svlgui.set_cursor("curve", stage)
	update_tooloptions()
def paintbrush(self):
	svlgui.MODE = "p"
	svlgui.set_cursor("paintbrush", stage)
	update_tooloptions()
def pen(self):
	svlgui.MODE = "n"
	svlgui.set_cursor("pen", stage)
	update_tooloptions()
def paint_bucket(self):
	svlgui.MODE = "b"
	svlgui.set_cursor("bucket", stage)
	update_tooloptions()
	
	
def update_tooloptions():
	for i in svlgui.TOOLOPTIONS:
			if svlgui.MODE==svlgui.TOOLOPTIONS[i]:
				i.setvisible(True)
			else:
				i.setvisible(False)
	
def ave(x, y, fac):
	"""Weighted average. 
	fac is the weight - 0.5 gives a standard average"""
	return y - fac*(y-x)

	
def box(x, y, width, height, fill):
	global objects
	box = svlgui.Shape(x, y)
	box.shapedata = [["M",0,0],["L",width,0],["L",width,height],["L",0,height],["L",0,0]]
	box.fillcolor = svlgui.Color(fill)
	box.linecolor = svlgui.Color("#cccccc")
	box.filled = True
	return box


def process_exists(proc_name):
    ps = subprocess.Popen("ps ax -o pid= -o args= ", shell=True, stdout=subprocess.PIPE)
    ps_pid = ps.pid
    output = ps.stdout.read()
    ps.stdout.close()
    ps.wait()

    for line in output.split("\n"):
        res = re.findall("(\d+) (.*)", line)
        if res:
            pid = int(res[0][0])
            if proc_name in res[0][1] and pid != os.getpid() and pid != ps_pid:
                return True
    return False

def lastval(arr,index):
	for i in reversed(arr[:index]):
		if i:
			return i
	

def angle_to_point(point1, point2):
	deltaX = point2.x-point1.x
	deltaY = point2.y-point1.y
	angleInDegrees = math.atan2(-deltaY, deltaX) * 180 / math.pi
	if angleInDegrees<0: angleInDegrees = 360+angleInDegrees
	return angleInDegrees

def sqr(x) :
	return x * x
def dist2(v, w):
	return sqr(v.x - w.x) + sqr(v.y - w.y)
def distToSegmentSquared(p, v, w):
	l2 = dist2(v, w)
	if l2 == 0:
		return dist2(p, v)
	t = ((p.x - v.x) * (w.x - v.x) + (p.y - v.y) * (w.y - v.y)) / l2
	if t < 0:
		return dist2(p, v)
	if t > 1:
		return dist2(p, w)
	return dist2(p, svlgui.Point(x=(v.x+t*(w.x-v.x)), y=(v.y+t*(w.y-v.y))))

def distToSegment(p, v, w):
	return math.sqrt(distToSegmentSquared(p, v, w))

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
					try:
						cosB=-(ca*ca-bc*bc-ab*ab)/(2*bc*ab);
					except ZeroDivisionError:
						cosB=-1 # Two of the points overlap; the best thing to do is delete one.
					try:
						acosB=math.acos(cosB)*180/math.pi;
					except ValueError:
						acosB=0
					try:
						if acosB>(165-500/(ab+bc)):	# at least 15 degrees away from straight angle
							del shape[j]
					except ZeroDivisionError:
						# Either both points overlap or one is imaginary. Kudos to you if you manage
						# to create a point with an imaginary coordinate in Lightningbeam.
						del shape[j]	
		if mode=="smooth":
			shape = catmullRom2bezier([shape[0]]*2+shape+[shape[-1]])
							
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


def pairwise(iterable):
    "s -> (s0,s1), (s1,s2), (s2, s3), ..."
    a, b = tee(iterable)
    next(b, None)
    return izip(a, b)

def hittest(linelist,x,y):
	hits = False
	def IsOnLeft(a, b, c):
		return Area2(a, b, c) > 0
	def IsOnRight(a, b, c):
		return Area2(a, b, c) < 0
	def IsCollinear(a, b, c):
		return Area2(a, b, c) == 0
	# calculates the triangle's size (formed by the "anchor" segment and additional point)
	def Area2(a, b, c):
		return (b[0]-a[0])*(c[1]-a[1])-(c[0]-a[0])*(b[1]-a[1])
	def intersects(a,b,c,d):
		return not (IsOnLeft(a,b,c) != IsOnRight(a,b,d))
	def ccw(a,b,c):
		return (c[1]-a[1])*(b[0]-a[0]) > (b[1]-a[1])*(c[0]-a[0])
	def intersect(a,b,c,d):
		return ccw(a,c,d) != ccw(b,c,d) and ccw(a,b,c) != ccw(a,b,d)
	for i in xrange(len(linelist)):
		hits = hits != intersect([linelist[i-1].endpoint1.x,linelist[i-1].endpoint1.y],
								 [linelist[i].endpoint1.x,linelist[i].endpoint1.y],[x,y],[x,sys.maxint])
	print hits, x, y
	return hits


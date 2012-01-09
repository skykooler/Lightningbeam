import svlgui
from threading import Event, Thread

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
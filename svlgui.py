#! /usr/bin/python

import os
import sys
import math
import random
import colors
import platform
import re

'''
#	Tool mode. Keyboard shortcut is the same key. Modes are:
#	" ": selection
#	"l": lasso tool
#	"s": scale/rotate tool
#	"t": text tool
#	"r": rectangle tool
#	"e": ellipse tool
#	"c": curve tool
#	"p": paintbrush tool
#	"n": pen tool
#	"b": paint bucket tool
'''
MODE="e"
SITER=0

#Currentframe - the frame selected on the timeline. Not necessarily the frame being shown.
CURRENTFRAME=0

#Object which has the keyboard focus.
FOCUS = None


class Color (object):
	def __init__(self, val):
		if type(val)==type([]):
			self.type = "RGB"
			self.val = val
		elif type(val)==type(""):
			if val.startswith("#"):
				self.type = "RGB"
				self.val = hex2rgb(val)
			else:
				self.type = "Image"
				self.val = val
	def _getcairo(self):
		if self.type=="RGB":
			return cairo.SolidPattern(*self.val)
		elif self.type=="Image":
			surface = cairo.ImageSurface.create_from_png(self.val)
			pat = cairo.SurfacePattern(surface)
			return pat
	def _getpygui(self):
		if self.type=="RGB":
			return Colors.rgb(*self.val)
	def _getrgb(self):
		if self.type=="RGB":
			return rgb2hex(*self.val)
		else:
			print "Error: Trying to get RGB from image!"
			return None
	def _setrgb(self, rgb):
		self.type = "RGB"
		if (val)==type([]):
			self.val = rgb
		elif type(val)==type(""):
			self.val = hex2rgb(val)
	cairo = property(_getcairo)
	pygui = property(_getpygui)
	rgb = property(_getrgb, _setrgb)
def rgb2hex(r, g, b, a=1):
	r=hex(int(r*255)).split("x")[1].zfill(2)
	g=hex(int(g*255)).split("x")[1].zfill(2)
	b=hex(int(b*255)).split("x")[1].zfill(2)
	a=hex(int(a*255)).split("x")[1].zfill(2)
	return "#"+r+g+b+a
def hex2rgb(hex):
	a=hex[1]
	b=hex[2]
	c=hex[3]
	if len(hex)==7:
		d=hex[4]
		e=hex[5]
		f=hex[6]
		ab = a+b
		cd = c+d
		ef = e+f
		return (int(ab, 16)/256.0, int(cd, 16)/256.0, int(ef, 16)/256.0)
	elif len(hex)==9:
		d=hex[4]
		e=hex[5]
		f=hex[6]
		g=hex[7]
		h=hex[8]
		ab = a+b
		cd = c+d
		ef = e+f
		gh = g+h
		return (int(ab, 16)/256.0, int(cd, 16)/256.0, int(ef, 16)/256.0, int(gh, 16)/256.0)
		
	else:
		return (int(a, 16)/16.0, int(b, 16)/16.0, int(c, 16)/16.0)




LINECOLOR = Color("#990099")
FILLCOLOR = Color("#000000")

#Magic. Detect platform and select appropriate toolkit. To be used throughout code.
if sys.platform=="linux2":
	id = platform.machine()
	if id.startswith('arm'):
		PLATFORM = 'linuxARM'
	elif (not id) or (not re.match('(x|i[3-6])86$', id) is None):
		PLATFORM = 'linux32'
	elif id.lower() in ('x86_64', "amd64"):
		PLATFORM = 'linux64'
	elif "ppc" in plid.lower():
		PLATFORM = "linuxPPC"
	else:
		PLATFORM = "error"
	import gtk
	import cairo
	import gobject
	import Image
	import time
	import misc_funcs
	#SYSTEM="gtk"
	###   TESTING - gtk should be Linux platform, at least for now  ####
	#'''
	import pickle
	import GUI		# Using PyGUI. Experimental.
	from GUI import Window as OSXWindow, Button as OSXButton, Image as OSXImage
	from GUI import Frame as OSXFrame, Color as OSXColor, Grid as OSXGrid
	from GUI import Column, Row, ScrollableView, TextEditor, Colors
	from GUI import StdCursors, Alerts, FileDialogs, Font
	from GUI.StdMenus import basic_menus, file_cmds, print_cmds
	from GUI.Files import FileType
	from GUI.Geometry import offset_rect, rect_sized
	#app = GUI.application()
	SYSTEM="osx"
	'''
	SYSTEM="html"
	ids = {}
	jsdefs = []
	jsfunctions = ""'''
	sep = "/"
elif sys.platform=="win32":
	PLATFORM="win32"
	import pickle
	import misc_funcs
	import GUI		# Using PyGUI. Experimental.
	from GUI import Window as OSXWindow, Button as OSXButton, Image as OSXImage
	from GUI import Frame as OSXFrame, Color as OSXColor, Grid as OSXGrid
	from GUI import Column, Row, ScrollableView, TextEditor, Colors
	from GUI import StdCursors, Alerts, FileDialogs, Font
	from GUI.StdMenus import basic_menus, file_cmds, print_cmds
	from GUI.Files import FileType
	from GUI.Geometry import offset_rect, rect_sized
	SYSTEM="osx"
	sep = "\\"
elif sys.platform=="linux-armv6l":
	import android
	droid = android.Android()
	SYSTEM="android"
	tb = ""
	sep = "/"
	print str(sys.platform)
elif sys.platform=="darwin":
	PLATFORM="osx"
	import pickle
	import misc_funcs
	import GUI		# Using PyGUI. Experimental.
	from GUI import Window as OSXWindow, Button as OSXButton, Image as OSXImage
	from GUI import Frame as OSXFrame, Color as OSXColor, Grid as OSXGrid
	from GUI import Column, Row, ScrollableView, TextEditor, Colors
	from GUI import StdCursors, Alerts, FileDialogs, Font
	from GUI.StdMenus import basic_menus, file_cmds, print_cmds
	from GUI.Files import FileType
	from GUI.Geometry import offset_rect, rect_sized
	#app = GUI.application()
	SYSTEM="osx"
	sep = "/"
	
__windowlist__=[]

if SYSTEM=="osx":
	class Lightningbeam(GUI.Application):
		def __init__(self):
			GUI.Application.__init__(self)
			self.file_type = FileType(name = "Untitled Document", suffix = "changethis", 
				mac_creator = "BLBE", mac_type = "BLOB"), # These are optional)
		def setup_menus(self, m):
			m.quit_cmd.enabled = 1
			m.save_cmd.enabled = 1
			m.open_cmd.enabled = 1
			m.run_file.enabled = 1
			m.create_sc.enabled = 1
			m.add_keyframe.enabled = 1
			m.add_layer.enabled = 1
			m.delete_layer.enabled = 1
			m.bring_forward.enabled = 1
			m.bring_to_front.enabled = 1
			m.send_backward.enabled = 1
			m.send_to_back.enabled = 1
        
        #def create_sc(self):
		#	pass
		#def run_file(self):
		#	pass
	class LightningbeamWindow(OSXWindow):
		def __init__(self,*args,**kwargs):
			OSXWindow.__init__(self,*args,**kwargs)
		#def save_cmd(widget=None):
		#	print "to save"
		def key_down(self, event):
			if FOCUS:
				FOCUS.key_down(event)
		def key_up(self, event):
			if FOCUS:
				FOCUS.key_up(event)
			
			
	app = Lightningbeam()
elif SYSTEM=="html":
	app = ""
	

class htmlobj:
	"""
	HTML Object class. Only should be used when SYSTEM is "html".
	Constructor: htmlobj (tag, [data])
		tag is the name of the element in question. For example, to
			create a <div> element, tag would be "div".
		data is a dictionary containing attributes of the tag. For
			example: htmlobj("div", {"width":120, "id":"div10}) creates:
			<div id=div10 width=120>
		style is a dictionary of css style attributes to be applied. For
			example: htmlobj("div", {}, {"float":"left","width":"200"})
			creates: <div style='float:left; width:200;'>
	To access the HTML representation of an instance, call str() or the 
		built-in html() method.
	"""
	def __init__(self,tag,data={},style={}):
		self.tag = tag
		self.data = data
		self.style = style
		self.contents = []
	def __str__(self):
		retval = "<"+self.tag
		for i in self.data:
			retval+=" "+i+"="+str(self.data[i])
		if self.style:
			retval+=" style='"
			for i in self.style:
				retval+=i+":"+str(self.style[i])+";"
			retval+="'"
		retval+=">"+"".join((str(i) for i in self.contents))+"</"+self.tag+">\n"
		return retval
	def add(self, item):
		self.contents.append(item)
	def html(self):
		return str(self)

class Window:
	def __init__(self, title=""):
		__windowlist__.append(self)
		if SYSTEM=="gtk":
			self.window = gtk.Window()
			self.vbox = gtk.VBox()
			self.window.add(self.vbox)
			self.window.show_all()
			self.window.connect("destroy",self.destroy)
		elif SYSTEM=="osx":
			self.window = LightningbeamWindow(width=1024,height=500)
			#components = [i._int() for i in args]
			#self.vbox = GUI.Column(components, equalize="w", expand=0)
			#self.window.place(self.vbox, left = 0, top = 0, right = 0, bottom = 0, sticky = 'nsew')
			self.window.show()
		elif SYSTEM=="html":
			self.window = htmlobj("div")

	def add(self, obj,expand=False):
		objint = obj._int()		#Internal representation
		if SYSTEM=="gtk":
			self.vbox.pack_start(objint, expand, True, 0)
			self.window.show_all()
		elif SYSTEM=="osx":
			self.window.place(objint, left=0, top=0, right=0, bottom=0, sticky="nsew")
		elif SYSTEM=="html":
			objint.data["width"] = "100%"
			objint.data["height"] = "100%"
			self.window.add(objint)
	def destroy(self,data=None):
		__windowlist__.remove(self)
		if __windowlist__==[]:
			if SYSTEM=="gtk":
				gtk.main_quit()
			elif SYSTEM=="osx":
				pass
	def maximize(self):
		if SYSTEM=="gtk":
			self.window.maximize()
	def set_title(self, title):
		if SYSTEM=="gtk":
			self.window.set_title(title)
		elif SYSTEM=="osx":
			self.window.title = title
		elif SYSTEM=="html":
			jscommunicate("document.title = "+title)
			
# Widget meta-class - to prevent code duplication
# I don't seem to have any code in here. :(
class Widget(object):
	def __init__(self):
		pass
	
class Menu(Widget):
	def __init__(self, top, menuitems):
		if SYSTEM=="gtk":
			if top:
				self.mb = gtk.MenuBar()
			else:
				self.mb = gtk.Menu()
			def build_menu(j, parent):
				for i in j:
					if type(i)==type(""):
						#lambda is an anonymous function name, I'll use 'kappa' for an anonymous variable
						kappa = gtk.MenuItem(i)
					elif type(i)==type([]):
						kappa = gtk.MenuItem(i[0])
						kappabeta = gtk.Menu()	#Same idea. Kappa is the menu item, kappabeta is the menu.
						build_menu(i[1:],kappabeta)
						kappa.set_submenu(kappabeta)
					parent.append(kappa)
			build_menu(menuitems,self.mb)
		elif SYSTEM=="android":
			for i in menuitems:
				if i[0]=="File":
					droid.addOptionsMenuItem(i[0], "javaevent", "pass")
				elif i[0]=="Edit":
					droid.addOptionsMenuItem(i[0], "javaevent", "quit()")
				elif i[0]=="Help":
					droid.addOptionsMenuItem(i[0], "pythonevent", "pass")
				else:
					droid.addOptionsMenuItem(i[0], "pythonevent", "quit()")
		elif SYSTEM=="osx":
			if top:
				global menus
				self.mb = GUI.MenuList()
				tool_menu = GUI.Menu("Tools", [("Execute", "test_cmd")])
				menus = basic_menus(exclude = print_cmds)
				menus.append(tool_menu)
		elif SYSTEM=="html":
			pass
			# I need to figure out how the menus work here.
	def _int(self):		# Returns internal GUI-specific item 
		return self.mb

#def menufuncs(menu,j):
def menufuncs(j):
	if SYSTEM=="gtk":
		def connect_menu(j,parent):
			def dummifunc():
				pass
			agr = gtk.AccelGroup()
			menu._int().get_toplevel().add_accel_group(agr)
			for i in xrange(len(j)):
				if type(j[i])==type(dummifunc):
					parent.get_children()[i].connect("activate",j[i])
				elif type(j[i])==type(()):
					parent.get_children()[i].connect("activate",j[i][0])
					key, mod = gtk.accelerator_parse(j[i][1])
					parent.get_children()[i].add_accelerator("activate", agr, key, mod, gtk.ACCEL_VISIBLE)
				elif type(j[i])==type([]):
					connect_menu(j[i],parent.get_children()[i].get_submenu())
		connect_menu(j,menu._int())
	elif SYSTEM=="osx":
		global menus
		global app
		def test():
			print 95
		for i in j:
			if not i[0] in ["File", "Edit", "Help"]:
				'''for n in i:
					if type(n) == type(()):
						print n[1].__name__
						if n[1].__name__=="run_file":
							app.run_file = test'''
				[setattr(app,k[1].__name__, k[1]) for k in i if type(k)==type(())]
				menu = GUI.Menu(i[0],[(k[0],k[1].__name__) for k in i if type(k)==type(())])
				#menu = GUI.Menu("Test", [("Run", 'run_file')])
				menus.append(menu)
			else:
				cmds={"Save":"save_cmd", "Open":"open_cmd"}
				[setattr(app,cmds[k[0]],k[1]) for k in i if (k[0] in cmds)]
			
class VBox(Widget):
	def __init__(self,width=False,height=False,*args):
		if SYSTEM=="gtk":
			self.vbox=gtk.VBox()
			if width and height:
				self.vbox.set_size_request(width,height)
			[self.add(*i) for i in args]
		elif SYSTEM=="osx":
			seq = [i[0] for i in args]		# TODO: load elements on load please
			self.vbox=GUI.Column(seq)
		elif SYSTEM=="html":
			self.vbox = htmlobj("table")
	def _int(self):		# Returns internal GUI-specific item 
		return self.vbox
	def add(self,obj,expand=False,fill=True):
		objint = obj._int()
		if SYSTEM=="gtk":
			self.vbox.pack_start(objint,expand,fill,0)
			self.vbox.show_all()
		elif SYSTEM=="osx":
			self.vbox.add(objint)
		elif SYSTEM=="html":
			if expand:
				objint.data["height"]="100%"
			tr = htmlobj("tr")
			td = htmlobj("td")
			td.add(objint)
			tr.add(td)
			self.vbox.add(tr)
			
class HBox(Widget):
	def __init__(self,width=False,height=False,*args):
		if SYSTEM=="gtk":
			self.hbox=gtk.HBox()
			if width and height:
				self.hbox.set_size_request(width,height)
			[self.add(*i) for i in args]
		elif SYSTEM=="osx":
			seq = [i[0] for i in args]		# TODO: load elements on load please
			self.hbox=GUI.Row(seq)
		elif SYSTEM=="html":
			self.hbox = htmlobj("table")
			self.tr = htmlobj("tr")
			self.hbox.add(self.tr)
	def _int(self):		# Returns internal GUI-specific item 
		return self.hbox
	def add(self,obj,expand=False,fill=False):
		objint = obj._int()
		if SYSTEM=="gtk":
			self.hbox.pack_start(objint,expand,fill,0)
			self.hbox.show_all()
		elif SYSTEM=="html":
			if expand:
				objint.data["width"]="100%"
			td = htmlobj("td")
			td.add(objint)
			self.tr.add(td)

class Grid(Widget):
	def __init__(self,*args):
		if SYSTEM=="osx":
			self.buttons = args
			self.grid = GUI.Grid([[j._int() for j in i] for i in args],row_spacing=2,column_spacing=2,
								align="c",equalize="wh")
		elif SYSTEM=="html":
			self.buttons = args
			self.grid = htmlobj("table")
			for i in args:
				tr = htmlobj("tr")
				self.grid.add(tr)
				for j in i:
					td = htmlobj("td")
					td.add(j._int())
					tr.add(td)
	def _int(self):
		return self.grid
class Button(Widget):
	def __init__(self,text=""):
		if SYSTEM=="gtk":
			self.button=gtk.Button()
			self.button.connect("clicked", self._onPress)
		elif SYSTEM=="osx":
			self.button = GUI.Button(title=text)
			self.button.action = (self._onPress, self.button)
		elif SYSTEM=="html":
			global ids
			while True:
				tid = id(self)
				if not tid in ids:
					ids[tid]=self
					self.tid = tid
					break
			#self.button = htmlobj("button",{"onmousedown":"pythoncommu\
#nicate('ids["+self.tid+"]._onPress('+event.pageX+','+event.pageY+')')"})
			self.button = htmlobj("button",{"onmousedown":"pythoncommun\
icate('ids["+str(self.tid)+"]._onPress(ids["+str(self.tid)+"])')"})
	def _int(self):
		return self.button
	def set_image(self, img):
		if SYSTEM=="gtk":
			image=gtk.Image()
			image.set_from_file(img)
			self.button.add(image)
		elif SYSTEM=="osx":
			self.button.title = img.split("/")[-1].split(".")[0]
	def set_content(self, content):
		if SYSTEM=="gtk":
			self.button.add(content._int())
		elif SYSTEM=="html":
			self.button.add(content._int())
	def _onPress(self, widget):
		self.onPress(self)
	def onPress(self, self1):
		pass

class ButtonBox(Widget):
	# This class appears to be platform-independent. Nice!
	def __init__(self,rows,columns):
		self.buttons=[]
		self.hboxes=[]
		self.vbox=VBox()
		for i in range(rows):
			self.hboxes.append(HBox())
			self.vbox.add(self.hboxes[-1])
			self.buttons.append([])
			for j in range(columns):
				self.buttons[-1].append(Button())
				self.hboxes[-1].add(self.buttons[-1][-1])
	def _int(self):
		return self.vbox._int()
	def add(self, obj):
		self.vbox.add(obj)
		
class ScrolledWindow(Widget):
	#sch controls the horizontal scrollbar, scv controls the vertical one
	def __init__(self,sch=True,scv=True):
		if SYSTEM=="gtk":
			self.sw = gtk.ScrolledWindow()
			self.sw.set_policy(gtk.POLICY_ALWAYS if sch else gtk.POLICY_AUTOMATIC, gtk.POLICY_ALWAYS if scv else gtk.POLICY_AUTOMATIC)
	def _int(self):
		return self.sw
	def add(self,obj):
		objint = obj._int()
		self.sw.add_with_viewport(objint)

class Frame(Widget):
	# PyGUI, HTML only right now
	def __init__(self):
		if SYSTEM=="osx":
			self.frame = GUI.Frame()
		elif SYSTEM=="html":
			self.frame = htmlobj("div")
	def _int(self):
		return self.frame
	def layout_self(self, *args):
		if SYSTEM=="osx":
			for i in args:
				self.frame.place(i[0]._int(),left=i[1],right=i[2],top=i[3],bottom=i[4],sticky=i[5], scrolling=i[6])
		elif SYSTEM=="html":
			for i in args:
				i[0]._int().style["position"]="absolute"
				if i[1]:
					i[0]._int().style["left"]=i[1]
				if i[2]:
					i[0]._int().style["right"]=i[2]
				if i[3]:
					i[0]._int().style["top"]=i[3]
				if i[4]:
					i[0]._int().style["bottom"]=i[4]
				if "h" in i[6]:
					i[0]._int().style["overflow-x"]="scroll"
				else:
					i[0]._int().style["overflow-x"]="hidden"
				if "v" in i[6]:
					i[0]._int().style["overflow-y"]="scroll"
				else:
					i[0]._int().style["overflow-y"]="hidden"
				self.frame.add(i[0]._int())
class Canvas(Widget):
	def __init__(self,width=False,height=False):
		self.objs=[]
		if SYSTEM=="gtk":
			self.canvas = gtk.DrawingArea()
			self.canvas.add_events(gtk.gdk.EXPOSURE_MASK
									| gtk.gdk.LEAVE_NOTIFY_MASK
									| gtk.gdk.BUTTON_PRESS_MASK
									| gtk.gdk.BUTTON_RELEASE_MASK
									| gtk.gdk.KEY_PRESS_MASK
									| gtk.gdk.POINTER_MOTION_MASK
									| gtk.gdk.POINTER_MOTION_HINT_MASK)
			if width and height:
				self.canvas.set_size_request(width,height)
			def onMouseDown(canvas, event):
				for i in self.objs:
					i._onMouseDown(event.x, event.y)
				self.expose_event(self.canvas, "expose-event", self.objs)
			def onMouseUp(canvas, event):
				for i in self.objs:
					i._onMouseUp(event.x, event.y)
				self.expose_event(self.canvas, "expose-event", self.objs)
			def onMouseMove(canvas, event):
				for i in self.objs:
					i._onMouseMove(event.x, event.y)
				self.expose_event(self.canvas, "expose-event", self.objs)
			self.canvas.connect("expose-event", self.expose_event, self.objs)
			self.canvas.connect("button-press-event", onMouseDown)
			self.canvas.connect("button-release-event", onMouseUp)
			self.canvas.connect("motion_notify_event", onMouseMove)
		elif SYSTEM=="osx":
			class OSXCanvas (ScrollableView):
				def draw(self, canvas, update_rect):
					canvas.erase_rect(update_rect)
					for i in self.objs:
						i.draw(canvas)

				def mouse_down(self, event):
					x, y = event.position
					for i in self.objs:
						i._onMouseDown(x, y)
					self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
					
				def mouse_drag(self, event):
					x, y = event.position
					for i in self.objs:
						i._onMouseDrag(x, y)
					self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
					
				def mouse_move(self, event):
					x, y = event.position
					for i in self.objs:
						i._onMouseMove(x, y)
					self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
					
				def mouse_up(self, event):
					x, y = event.position
					for i in self.objs:
						i._onMouseUp(x, y)
					self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
					
				def key_down(self, event):
					keydict = {127:"backspace",63272:"delete",63232:"up_arrow",63233:"down_arrow",
									63235:"right_arrow",63234:"left_arrow",13:"enter",9:"tab",
									63236:"F1",63237:"F2",63238:"F3",63239:"F4",63240:"F5",
									63241:"F6",63242:"F7",63243:"F8",}
					if not event.unichars=='':
						if ord(event.unichars) in keydict:
							key = keydict[ord(event.unichars)]
						else:
							key = event.unichars
					else:
						key = event.key.upper()
					for i in self.objs:
						i._onKeyDown(key)
					self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
				
				def key_up(self, event):
					pass
			self.canvas = OSXCanvas(extent = (width, height), scrolling = 'hv')
			self.canvas.objs = self.objs
		elif SYSTEM=="html":
			global ids
			while True:
				tid = id(self)
				ids[tid]=self
				self.tid = tid
				break
			self.canvas = htmlobj("canvas",{"id":"canvas"+str(self.tid)})
			jsdefine("drawcanvas","(tid)",'''
			var canvas = document.getElementById("canvas"+tid.toString());
			var ctx = canvas.getContext("2d")
			ctx.clearRect(0, 0, canvas.width, canvas.height)
			for (i in cchildren[tid]) {
				i.draw(ctx);
			}''')
			jscommunicate("cchildren["+str(self.tid)+"]="+str(self.objs))
	def _int(self):
		return self.canvas
	def expose_event(self, canvas, event, objs):
		x,y,w,h = canvas.allocation
		surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, w,h)
		cr = cairo.Context(surface)
		cra = canvas.window.cairo_create()
		cr.set_source_rgb(0.5, 0.5, 0.5)
		cr.paint()
		for i in objs:
			i.draw(cr)
		cra.set_source_surface(surface)
		cra.paint()
	def draw(self):
		if SYSTEM=="gtk":
			self.expose_event(self.canvas, "draw_event", self.objs)
		elif SYSTEM in ["osx", "android"]:
			self.canvas.invalidate_rect((0,0,self.canvas.extent[0],self.canvas.extent[1]))
		elif SYSTEM=="html":
			jscommunicate("drawcanvas("+self.tid+")")
	def add(self, obj, x, y):
		obj.x = x
		obj.y = y
		self.objs.append(obj)
		if SYSTEM=="html":
			jscommunicate("cchildren["+str(self.tid)+"]="+str(self.objs))
	def delete(self, obj):
		self.objs.remove(obj)
		del obj
		self.draw()
		if SYSTEM=="html":
			jscommunicate("cchildren["+str(self.tid)+"]="+str(self.objs))
	def key_down(self, event):
		if SYSTEM=="osx":
			self.canvas.key_down(event)
	def key_up(self, event):
		if SYSTEM=="osx":
			self.canvas.key_up(event)

class TextView(Widget):
	def _gettext(self):
		if SYSTEM=="osx":
			return self.box.text
	def _settext(self, text):
		if SYSTEM=="osx":
			self.box.text = text
	text = property(_gettext, _settext)
	def __init__(self,editable=True,width=False,height=False):
		if SYSTEM=="gtk":
			self.sw=ScrolledWindow()
			self.box=gtk.TextView()
			if width and height:
				self.sw._int().set_size_request(width,height)
			self.box.set_cursor_visible(editable)
			self.sw._int().add_with_viewport(self.box)
			#self.sw._int().set_policy(gtk.POLICY_AUTOMATIC, gtk.POLICY_AUTOMATIC)
			def scroll(self,widget):
				self.scroll_to_mark(self.get_buffer().get_insert(), 0)
			self.box.connect("key-press-event",scroll)
		elif SYSTEM=="osx":
			self.box = GUI.TextEditor(scrolling="hv")
			self.box.font = Font("Mono", 12, [])
		elif SYSTEM=="html":
			self.box = htmlobj("textarea")
	def _int(self):
		if SYSTEM=="gtk":
			return self.sw._int()
		elif SYSTEM=="osx":
			return self.box
		elif SYSTEM=="html":
			return self.box

class Image(object):
	def __init__(self,image,x=0,y=0,animated=False,canvas=None,htiles=1,vtiles=1):
		self.x = x
		self.y = y
		self.minx = x
		self.miny = y
		self.rotation = 0
		self.xscale = 1
		self.yscale = 1
		self.filled = True
		self.linecolor = None
		self.fillcolor = None
		self.name = image
		if animated:
			self.animated = True
			self.htiles = htiles
			self.vtiles = vtiles
			self.pointer = 0
			self.canvas = canvas
			def animate(self):
				self.pointer = (self.pointer+1)%(htiles*vtiles)
				if SYSTEM in ["gtk", "osx"]:
					self.canvas._int().invalidate_rect([self.x, self.y, self.x+self.image.bounds[2]/self.htiles, self.y+self.image.bounds[3]/self.vtiles])
				else:
					jscommunicate("drawcanvas("+str(self.canvas.tid)+")")
			r = misc_funcs.RepeatTimer(0.1, animate, args=[self])
			r.daemon = True
			r.start()
		else:
			self.animated = False
		if SYSTEM=="osx":
			self.image = GUI.Image(file = image)
			if self.animated:
				self.maxx = self.x+self.image.bounds[2]/self.htiles
				self.maxy = self.y+self.image.bounds[3]/self.vtiles
			else:
				self.maxx = self.x+self.image.bounds[2]
				self.maxy = self.y+self.image.bounds[3]
		elif SYSTEM=="html":
			self.image = htmlobj("img", {"src":image})
			#TODO: ##### FIGURE OUT WIDTH, HEIGHT #####
			if self.animated:
				self.maxx = self.x#+self.image.width[2]/self.htiles
				self.maxy = self.y#+self.image.height[3]/self.vtiles
			else:
				self.maxx = self.x#+self.image.width[2]
				self.maxy = self.y#+self.image.height[3]
	def _int(self):
		return self.image
	def draw(self, cr=None, parent=None, rect=None):
		if SYSTEM=="android":
			pass
		elif SYSTEM=="osx":
			cr.gsave()
			if sep=="\\":
				# Very ugly hack for Windows. :(
				# Windows doesn't respect coordinate transformations
				# with respect to translation, so we have to do this
				# bit ourselves.
				
				# Rotation in radians
				radrot = parent.group.rotation*math.pi/180 
				# Coordinate transform: multiplication by a rotation matrix
				cr.translate(self.x*math.cos(radrot)-self.y*math.sin(radrot), self.x*math.sin(radrot)+self.y*math.cos(radrot))
			else:
				cr.translate(self.x,self.y)
			cr.rotate(self.rotation)
			cr.scale(self.xscale*1.0, self.yscale*1.0)
			if self.animated:
				src_rect = self.image.bounds
				# (i%4)%6, i/4
				src_rect = [(src_rect[2]/self.htiles)*(self.pointer%self.htiles),
							(src_rect[3]/self.vtiles)*(self.pointer/self.htiles),
							(src_rect[2]/self.htiles)*(self.pointer%self.htiles+1),
							(src_rect[3]/self.vtiles)*(self.pointer/self.htiles+1)]
				#src_rect = [16*self.pointer,0,16+16*self.pointer,32]
				#print [self.x, self.y, self.x+self.image.bounds[2]/self.htiles, self.y+self.image.bounds[3]/self.vtiles]
				dst_rect = [self.x, self.y, self.image.bounds[2]/self.htiles+self.x, self.image.bounds[3]/self.vtiles+self.y]
				self.image.draw(cr, src_rect, dst_rect)
			else:
				src_rect = self.image.bounds
				dst_rect = src_rect
				self.image.draw(cr, src_rect, dst_rect)
			cr.grestore()
		elif SYSTEM=="html":
			cr.save()
			pass

class Shape (object):
	def __init__(self,x=0,y=0,rotation=0,fillcolor=None,linecolor=None):
		global SITER
		self.x=x
		self.y=y
		self.rotation=rotation
		self.xscale = 1
		self.yscale = 1
		self.linecolor = linecolor if linecolor else LINECOLOR
		self.fillcolor = fillcolor if fillcolor else FILLCOLOR
		self.shapedata=[]
		self.filled=False
		self.type="Shape"
		####################-----TEMPORARY-----#########################
		self.name = "s"+str(int(random.random()*10000))+str(SITER)
		SITER+=1
		################################################################
	def draw(self,cr=None,parent=None,rect=None):
		if SYSTEM=="gtk":
			cr.save()
			cr.translate(self.x,self.y)
			cr.rotate(self.rotation*math.pi/180)
			cr.scale(self.xscale*1.0, self.yscale*1.0)
			cr.set_source(self.linecolor.cairo)
			for i in self.shapedata:
				if i[0]=="M":
					cr.move_to(i[1],i[2])
				elif i[0]=="L":
					cr.line_to(i[1],i[2])
				elif i[0]=="C":
					cr.curve_to(i[1],i[2],i[3],i[4],i[5],i[6])
			if self.filled:
				cr.stroke_preserve()
				cr.set_source(self.fillcolor.cairo)
				cr.fill()
			else:
				cr.stroke()
			cr.restore()
		elif SYSTEM=="android":
			global tb
			tb+="cr.save()\n"
			tb+="cr.translate("+str(self.x)+","+str(self.y)+")\n"
			tb+="cr.rotate("+str(self.rotation*math.pi/180)+")\n"
			tb+="cr.scale("+str(self.xscale)+"*1.0, "+str(self.yscale)+"*1.0)\n"
			if type(self.fill)==type([]):
				tb+="cr.fillStyle = \""+rgb2hex(self.fill[0],self.fill[1],self.fill[2])+"\"\n"
			for i in self.shapedata:
				if i[0]=="M":
					tb+="cr.moveTo("+str(i[1])+","+str(i[2])+")\n"
				elif i[0]=="L":
					tb+="cr.lineTo("+str(i[1])+","+str(i[2])+")\n"
				elif i[0]=="C":
					tb+="cr.bezierCurveTo("+str(i[1])+","+str(i[2])+","+str(i[3])+","+str(i[4])+","+str(i[5])+","+str(i[6])+")\n"
			if self.filled:
				tb+="cr.stroke()\n"
				tb+="cr.fill()\n"
			else:
				tb+="cr.stroke()\n"
			tb+="cr.restore()\n"
		elif SYSTEM=="osx":
			cr.gsave()
			if sep=="\\":
				# Very ugly hack for Windows. :(
				# Windows doesn't respect coordinate transformations
				# with respect to translation, so we have to do this
				# bit ourselves.
				
				# Rotation in radians
				radrot = parent.group.rotation*math.pi/180 
				# Coordinate transform: multiplication by a rotation matrix
				cr.translate(self.x*math.cos(radrot)-self.y*math.sin(radrot), self.x*math.sin(radrot)+self.y*math.cos(radrot))
			else:
				cr.translate(self.x,self.y)
			cr.rotate(self.rotation)
			cr.scale(self.xscale*1.0, self.yscale*1.0)
			cr.newpath()
			cr.pencolor = self.linecolor.pygui
			cr.fillcolor = self.fillcolor.pygui
			for i in self.shapedata:
				if i[0]=="M":
					point = (i[1], i[2])
					cr.moveto(point[0],point[1])
				elif i[0]=="L":
					point = (i[1], i[2])
					cr.lineto(point[0],point[1])
				elif i[0]=="C":
					pointa = (i[1], i[2])
					pointb = (i[3], i[4])
					pointc = (i[5], i[6])
					### Mac OSX needs custom PyGUI for this to work ###
					cr.curveto((pointa[0],pointa[1]),(pointb[0],pointb[1]),(pointc[0],pointc[1]))
			if self.filled:
				cr.closepath()
				cr.fill_stroke()
			else:
				cr.stroke()
			cr.grestore()
		elif SYSTEM=="html":
			tb = ""
			tb+="cr.save()\n"
			tb+="cr.translate("+str(self.x)+","+str(self.y)+")\n"
			tb+="cr.rotate("+str(self.rotation*math.pi/180)+")\n"
			tb+="cr.scale("+str(self.xscale)+"*1.0, "+str(self.yscale)+"*1.0)\n"
			if type(self.fill)==type([]):
				tb+="cr.fillStyle = \""+rgb2hex(self.fill[0],self.fill[1],self.fill[2])+"\"\n"
			for i in self.shapedata:
				if i[0]=="M":
					tb+="cr.moveTo("+str(i[1])+","+str(i[2])+")\n"
				elif i[0]=="L":
					tb+="cr.lineTo("+str(i[1])+","+str(i[2])+")\n"
				elif i[0]=="C":
					tb+="cr.bezierCurveTo("+str(i[1])+","+str(i[2])+","+str(i[3])+","+str(i[4])+","+str(i[5])+","+str(i[6])+")\n"
			if self.filled:
				tb+="cr.stroke()\n"
				tb+="cr.fill()\n"
			else:
				tb+="cr.stroke()\n"
			tb+="cr.restore()\n"
			jscommunicate(tb)
	def line(self,x,y,x1=False,y1=False):
		pass
	def curve(self,x,y,x1,y1,x2=False,y2=False):
		pass
	def edit(self, arguments):
		pass #no idea how this will work yet
	def move(self, x, y):
		pass
	def scale(self, width, height):
		try:
			xfactor = width/self.maxx
			yfactor = height/maxy
			def scale_section(section):
				try:
					if section[0] in ["M", "L"]:
						section[1]*=xfactor
						section[2]*=yfactor
					elif section[0]=="C":
						section[1]*=xfactor
						section[2]*=yfactor
						section[3]*=xfactor
						section[4]*=yfactor
						section[5]*=xfactor
						section[6]*=yfactor
				except ZeroDivisionError:
					print "Divided by zero while scaling a tiny segment. Continuing happily."
			result = [scale_section(i) for i in self.shapedata]
		except ZeroDivisionError:
			print "Divided by zero! Universe collapsing."
	def hitTest(self,x,y):
		hits = False
		# points "a" and "b" forms the anchored segment.
		# point "c" is the evaluated point
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
		for i in xrange(len(self.shapedata)):
			hits = hits != intersect(self.shapedata[i-1][1:3],self.shapedata[i][1:3],[x,y],[x,sys.maxint])
		return hits
	def localtransform(self, x, y, parent):
		x,y = parent.localtransform(x,y)
		nx = x*math.cos(-self.rotation)-y*math.sin(-self.rotation)+self.x
		ny = x*math.sin(-self.rotation)+y*math.cos(-self.rotation)+self.y
		return nx, ny
	def revlocaltransform(self, x, y, parent):
		x,y = parent.revlocaltransform(x,y)
		radrot = self.rotation*math.pi/180
		nx = x*math.cos(radrot)-y*math.sin(radrot)+self.x
		ny = x*math.sin(radrot)+y*math.cos(radrot)+self.y
		return nx, ny
	def getminx(self):
		return min([i[1] for i in self.shapedata])
	def getminy(self):
		return min([i[2] for i in self.shapedata])
	def getmaxx(self):
		return max([i[1] for i in self.shapedata])
	def getmaxy(self):
		return max([i[2] for i in self.shapedata])
	def onMouseDown(self, self1, x, y):
		pass
	def onMouseDrag(self, self1, x, y):
		pass
	def onMouseUp(self, self1, x, y):
		pass
	def onMouseMove(self, self1, x, y):
		pass
	def onKeyDown(self, self1, key):
		pass
	def onKeyUp(self, self1, key):
		pass
	minx = property(getminx)
	miny = property(getminy)
	maxx = property(getmaxx)
	maxy = property(getmaxy)

class framewrapper (object):
			#Wraps object per-frame. Allows for changes in position, rotation, scale.
			def __init__(self, obj, x, y, rot, scalex, scaley, parent=None):
				self.obj = obj
				self.x = obj.x = x
				self.y = obj.y = y
				self.rot = obj.rot = rot
				self.scalex = obj.scalex = scalex
				self.scaley = obj.scaley = scaley
				self.level = False # don't try to descend into a framewrapper
				self.type = obj.__class__.__name__
				self.filled = obj.filled
				self.linecolor = obj.linecolor
				self.fillcolor = obj.fillcolor
				self.name = obj.name
				self.parent = parent
			def draw(self,cr,transform):
				pass
				self.update()
				self.obj.draw(cr,transform)
			def update(self):
				self.obj.x = self.x
				self.obj.y = self.y
				self.obj.rot = self.rot
				self.obj.scalex = self.scalex
				self.obj.scaley = self.scaley
				self.obj.filled = self.filled
				self.obj.linecolor = self.linecolor
				self.obj.fillcolor = self.fillcolor
			def _onMouseDown(self, x, y):
				self.obj.onMouseDown(self,x, y)
			def _onMouseUp(self, x, y):
				self.obj.onMouseUp(self,x, y)
			def _onMouseMove(self, x, y):
				self.obj.onMouseMove(self, x, y)
			def _onMouseDrag(self, x, y):
				self.obj.onMouseDrag(self, x, y)
			def _onKeyDown(self, key):
				self.obj.onKeyDown(self, key)
			def _onKeyUp(self, key):
				self.obj.onKeyUp(self, key)
			def getminx(self):
				return self.obj.minx+self.x
			def getminy(self):
				return self.obj.miny+self.y
			def getmaxx(self):
				return self.obj.maxx
			def getmaxy(self):
				return self.obj.maxy
			minx = property(getminx)
			miny = property(getminy)
			maxx = property(getmaxx)
			maxy = property(getmaxy)
			def hitTest(self, x, y):
				x,y = self.transformcoords(x,y)
				return self.obj.hitTest(x, y)
			def transformcoords(self,x,y):
				x = x-self.x
				y = y-self.y
				return x,y
		

class frame:
		def __init__(self,parent,duplicate=None):
			self.objs = []
			self.currentselect=None
			self.type="Group"
			self.parent = parent
		def add(self, obj, x, y, rot=0, scalex=0, scaley=0):
			self.objs.append(framewrapper(obj, x, y, rot, scalex, scaley, self.objs))
		def play(self, group, cr, currentselect,transform,rect):
			if SYSTEM=="gtk":
				cr.save()
				cr.translate(group.x,group.y)
				cr.rotate(group.rotation*math.pi/180)
				cr.scale(group.xscale,group.yscale)
				result = [obj.draw(cr) for obj in self.objs if ((obj.minx>=rect[0] and obj.miny>=rect[1]) or (obj.maxx<=rect[2] and obj.maxy<=rect[3]))]
				if currentselect:
					cr.set_source_rgb(0,0,1)
					cr.rectangle(currentselect.minx-1,currentselect.miny-1,currentselect.maxx+2,currentselect.maxy+2)
					cr.stroke()
				cr.restore()
			elif SYSTEM=="android":
				global tb
				tb+="cr.save()\n"
				tb+="cr.translate("+str(group.x)+","+str(group.y)+")\n"
				tb+="cr.rotate("+str(group.rotation*math.pi/180)+")\n"
				result = [obj.draw(cr) for obj in self.objs]
				if currentselect:
					tb+="cr.strokeSyle = \"#0000FF\"\n"
					tb+="cr.rect("+str(currentselect.minx-1)+","+str(currentselect.miny-1)+","+str(currentselect.maxx+2)+","+str(currentselect.maxy+2)+")\n"
					tb+="cr.stroke()\n"
				tb+="cr.restore()\n"
			elif SYSTEM=="osx":
				pass
				self.group = group
				cr.gsave()
				cr.rotate(group.rotation)
				cr.translate(group.x,group.y)
				cr.scale(group.xscale,group.yscale)
				def dodraw(obj, cr):
					obj.draw(cr, self)
				result = [dodraw(obj, cr) for obj in self.objs]
				if currentselect:
					cr.gsave()
					cr.newpath()
					cr.pencolor = Colors.rgb(0,0,1)
					cr.rect([currentselect.minx-1,currentselect.miny-1,
									currentselect.maxx+currentselect.x+2,
									currentselect.maxy+currentselect.y+2])
					cr.stroke()
					cr.grestore()
				cr.grestore()
			elif SYSTEM=="html":
				tb = ""
				tb+="cr.save()\n"
				tb+="cr.translate("+str(group.x)+","+str(group.y)+")\n"
				tb+="cr.rotate("+str(group.rotation*math.pi/180)+")\n"
				def dodraw(obj, cr):
					obj.draw(cr, self)
				result = [dodraw(obj, cr) for obj in self.objs]
				if currentselect:
					tb+="cr.strokeSyle = \"#0000FF\"\n"
					tb+="cr.rect("+str(currentselect.minx-1)+","+str(currentselect.miny-1)+","+str(currentselect.maxx+2)+","+str(currentselect.maxy+2)+")\n"
					tb+="cr.stroke()\n"
				tb+="cr.restore()\n"
				jscommunicate(tb)
		def localtransform(self,x,y):
			radrot = self.group.rotation*math.pi/180.0
			nx = x*math.cos(-radrot)-y*math.sin(-radrot)-self.group.x
			ny = x*math.sin(-radrot)+y*math.cos(-radrot)-self.group.y
			return nx, ny
		def revlocaltransform(self,x,y):
			radrot = self.group.rotation*math.pi/180.0
			nx = x*math.cos(radrot)-y*math.sin(radrot)-self.group.x
			ny = x*math.sin(radrot)+y*math.cos(radrot)-self.group.y
			return nx, ny
		def print_sc(self):
			retval = ""
			if self==self.parent.frames[0]:
				for i in self.objs:
					retval = retval+".put "+i.name+" x="+str(i.x)+" y="+str(i.y)+"\n"
			else:
				for i in self.objs:
					if not i.obj in [j.obj for j in misc_funcs.lastval(self.parent.frames,self.parent.frames.index(self)).objs]:
						retval = retval+".put "+i.name+" x="+str(i.x)+" y="+str(i.y)+"\n"
					else:
						retval = retval+".move "+i.name+" x="+str(i.x)+" y="+str(i.y)+"\n"
			return retval
	

class Layer:
	def setscale(self, scal):
		self.xscale = scal
		self.yscale = scal
	def getminx(self):
		return min([i.minx for i in self.currentFrame()])
	def getminy(self):
		return min([i.miny for i in self.currentFrame()])
	def getmaxx(self):
		return max([i.maxx for i in self.currentFrame()])
	def getmaxy(self):
		return max([i.maxy for i in self.currentFrame()])
	def onMouseDown(self, self1, x, y):
		pass
	def onMouseDrag(self, self1, x, y):
		pass
	def onMouseUp(self, self1, x, y):
		pass
	def onMouseMove(self, self1, x, y):
		pass
	def onKeyDown(self, self1, key):
		pass
	def onKeyUp(self, self1, key):
		pass
	def getcurrentselect(self):
		return self.frames[self.currentframe].currentselect
	def setcurrentselect(self, val):
		self.frames[self.currentframe].currentselect = val
	minx = property(getminx)
	miny = property(getminy)
	maxx = property(getmaxx)
	maxy = property(getmaxy)
	scale = property(fset = setscale)
	currentselect = property(getcurrentselect, setcurrentselect)
	def __init__(self, *args):
		# init is system-independent, oh joy
		self.x=0
		self.y=0
		self.rotation=0
		self.xscale = 1.0
		self.yscale = 1.0
		self.objs=[]
		self.currentframe=0
		self.activeframe=0	# Frame selected - not necessarily the frame displayed
		self.frames=[frame(self)]
		self.level = False
		self.clicked = False
		self.hidden = False
		def parse_obj(obj):
			self.objs.append(obj)
			obj.x=obj.x-self.x
			obj.y=obj.y-self.y
		[parse_obj(obj) for obj in args]
	def draw(self,cr=None,transform=None,rect=None):
		if SYSTEM=="android":
			global tb
			rc = False
			if cr:
				rc = True
				cr = None
		self.frames[self.currentframe].play(self,cr, self.currentselect, transform,rect)
		if SYSTEM=="android":
			if rc:
				droid.eventPost("javaevent", tb)
				tb = ""
	def add(self,*args):
		# system-independent
		def parse_obj(obj):
			obj.x=obj.x-self.x
			obj.y=obj.y-self.y
			self.frames[self.currentframe].add(obj, obj.x, obj.y, obj.rotation,0,0)
			self.objs.append(obj)
		[parse_obj(obj) for obj in args]
	def add_frame(self,populate):
		if self.activeframe>len(self.frames):
			lastframe = len(self.frames)
			for i in xrange((self.activeframe+1)-len(self.frames)):
				self.frames.append(None)
		if self.frames[self.activeframe]==None:
			self.frames[self.activeframe]=frame(self)
			for i in xrange(self.activeframe-1,-1,-1):
				if self.frames[i]:
					lastframe = i
					break
			else:
				lastframe = self.activeframe
		else:
			lastframe = self.activeframe
			self.activeframe+=1
			self.frames.insert(self.activeframe,frame(self))
		for i in self.frames[lastframe].objs:
			i.update()
			self.frames[self.activeframe].add(i.obj, i.x, i.y, i.rot)
		self.currentframe = self.activeframe
	def descendItem(self):
		if self.currentselect.__class__.__name__=="Group" and self.level==True:
			return self.frames[self.currentframe].currentselect.descendItem()
		else:
			return self
	def currentFrame(self):
		return self.frames[self.currentframe].objs
	def _onMouseDown(self, x, y):
		if self.level:
			if self.currentselect and self.currentselect.level:
				self.currentselect._onMouseDown(self.currentselect, x, y)
			else:
				if MODE in [" ", "s", "b"]:
					for i in reversed(self.currentFrame()):
						test = False
						if i.hitTest(x, y):
							if MODE in [" ", "s"]:
								self.currentselect = i
							i._onMouseDown(x, y)
							test=True
							break
					if not test:
						self.currentselect = None
				else:
					self.onMouseDown(self, x, y)
		else:
			self.onMouseDown(self, x, y)
	def onMouseDown(self, self1, x, y):
		pass
	def _onMouseUp(self,x,y):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onMouseUp(x, y)
		else:
			self.onMouseUp(self, x, y)
	def onMouseUp(self, self1, x, y):
		pass
	def _onMouseMove(self,x,y):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onMouseMove(x, y)
		else:
			self.onMouseMove(self, x, y)
	def onMouseMove(self, self1, x, y):
		pass
	def _onMouseDrag(self, x, y):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onMouseDrag(x, y)
		else:
			self.onMouseDrag(self, x, y)
	def onMouseDrag(self, self1, x, y):
		pass
	def _onKeyDown(self, key):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onKeyDown(key)
		else:
			self.onKeyDown(self, key)
	def onKeyDown(self, self1, key):
		pass
	def _onKeyUp(self, key):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onKeyUp(key)
		else:
			self.onKeyUp(self, key)
	def onKeyUp(self, self1, key):
		pass
	def print_sc(self,defs=True,frams=True):
		retval = ""
		if defs:
			for i in self.objs:
				if i.type=="Group":
					retval+=".sprite "+i.name+"\n"+i.print_sc
				elif i.type=="Shape":
					retval+=".outline "+i.name+"outline:\n"
					retval+=" ".join([" ".join([str(x) for x in a]) for a in i.shapedata])+"\n.end\n"
					if i.filled:
						retval+=".filled "+i.name+" outline="+i.name+"outline fill="+i.fillcolor.rgb+" color="+i.linecolor.rgb+"\n"
					else:
						retval+=".filled "+i.name+" outline="+i.name+"outline fill=#00000000 color="+i.linecolor.rgb+"\n"
		if frams:
			for i in self.frames:
				print i
				if i:
					retval+=".frame "+str(self.frames.index(i)+1)+"\n"+i.print_sc()
		return retval


class Group (object):
	def setscale(self, scal):
		self.xscale = scal
		self.yscale = scal
	def getal(self):
		return self.layers[self._al]
	def setal(self,al):
		self._al = al
	def getlevel(self):
		return self.activelayer.level
	def setlevel(self,lev):
		self.activelayer.level = lev
	def getminx(self):
		return min([i.minx for i in self.layers])
	def getminy(self):
		return min([i.miny for i in self.layers])
	def getmaxx(self):
		return max([i.maxx for i in self.layers])
	def getmaxy(self):
		return max([i.maxy for i in self.layers])
	def onMouseDown(self, self1, x, y):
		pass
	def onMouseDrag(self, self1, x, y):
		pass
	def onMouseUp(self, self1, x, y):
		pass
	def onMouseMove(self, self1, x, y):
		pass
	def getactiveframe(self):
		return self.activelayer.activeframe
	def setactiveframe(self, frame):
		print self.activelayer.frames
		if frame<len(self.activelayer.frames) and self.activelayer.frames[frame]:
			self.activelayer.currentframe = frame
		self.activelayer.activeframe = frame
	def getcurrentframe(self):
		return self.activelayer.activeframe
	def setcurrentframe(self, frame):
		print self.activelayer.frames
		if frame<len(self.activelayer.frames) and self.activelayer.frames[frame]:
			self.activelayer.currentframe = frame
		self.activelayer.activeframe = frame
	minx = property(getminx)
	miny = property(getminy)
	maxx = property(getmaxx)
	maxy = property(getmaxy)
	activelayer = property(getal,setal)
	activeframe = property(getactiveframe, setactiveframe)
	currentframe = property(getcurrentframe, setcurrentframe)
	level = property(getlevel, setlevel)
	scale = property(fset = setscale)
	def __init__(self, *args, **kwargs):
		self.layers = [Layer(*args)]
		self._al = 0
		self.clicked = False
		self.x = 0
		self.y = 0
		self.rotation = 0
		self.xscale = 1
		self.yscale = 1
		if "onload" in kwargs:
			kwargs["onload"](self)
	def draw(self,cr=None,transform=None,rect=None):
		for i in self.layers:
			if not i.hidden:
				i.x = self.x
				i.y = self.y
				i.rotation = self.rotation
				i.xscale = self.xscale
				i.yscale = self.yscale
				i.draw(cr,rect=rect)
	def add(self, *args):
		self.activelayer.add(*args)
	def add_frame(self, populate):
		self.activelayer.add_frame(populate)
	def add_layer(self, index):
		self.layers.insert(index+1,Layer())
		self.activelayer = index+1
		self.activelayer.level = True
	def delete_layer(self, index):
		del self.layers[index]
		while self._al>=0:
			try:
				dum = self.activelayer
				break
			except IndexError:
				self._al-=1
		if self._al<0:
			self.add_layer(-1)
	def descendItem(self):
		if self.activelayer.currentselect.__class__.__name__=="Group" and self.level==True:
			return self.frames[self.currentframe].currentselect.descendItem()
		else:
			return self
		self.activelayer.descendItem()
	def currentFrame(self):
		return self.activelayer.currentFrame()
	def localtransform(self,x,y):
		radrot = self.rotation*math.pi/180.0
		nx = x*math.cos(-radrot)-y*math.sin(-radrot)-self.x
		ny = x*math.sin(-radrot)+y*math.cos(-radrot)-self.y
		return nx, ny
	def onLoad(self, self1):
		pass
	def _onMouseDown(self, x, y):
		x, y = self.localtransform(x, y)
		if self.level:
			if self.activelayer.currentselect and self.activelayer.currentselect.level:
				self.activelayer.currentselect._onMouseDown(self.activelayer.currentselect, x, y)
			else:
				if MODE in [" ", "s", "b"]:
					for i in reversed(self.currentFrame()):
						test = False
						if i.hitTest(x, y):
							if MODE in [" ", "s"]:
								self.activelayer.currentselect = i
								test=True
							i._onMouseDown(x, y)
							break
					if not test:
						self.activelayer.currentselect = None
				else:
					self.onMouseDown(self, x, y)
		else:
			self.onMouseDown(self, x, y)
	def onMouseDown(self, self1, x, y):
		pass 
	def _onMouseUp(self,x,y):
		x, y = self.localtransform(x, y)
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onMouseUp(x, y)
		else:
			self.onMouseUp(self, x, y)
	def onMouseUp(self, self1, x, y):
		pass
	def _onMouseMove(self,x,y):
		x, y = self.localtransform(x, y)
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onMouseMove(x, y)
		else:
			self.onMouseMove(self, x, y)
	def onMouseMove(self, self1, x, y):
		pass
	def _onMouseDrag(self, x, y):
		x, y = self.localtransform(x, y)
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onMouseDrag(x, y)
		else:
			self.onMouseDrag(self, x, y)
	def onMouseDrag(self, self1, x, y):
		pass
	def _onKeyDown(self, key):
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onKeyDown(key)
		else:
			self.onKeyDown(self, key)
	def onKeyDown(self, self1, key):
		pass
	def _onKeyUp(self, key):
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onKeyUp(key)
		else:
			self.onKeyUp(self, key)
	def onKeyUp(self, self1, key):
		pass
	def maxframe(self):
		frame = 0
		for i in self.layers:
			frame = max(frame, len(i.frames))
		return frame
	def print_sc(self):
		retval = ""
		for i in self.layers:
			retval+=i.print_sc(True, False)
		for i in xrange(self.maxframe()):
			for j in self.layers:
				if j.frames[i]:
					retval+=".frame "+str(i+1)+"\n"
					retval+=j.frames[i].print_sc()
		return retval

def set_cursor(curs, widget=None):
	if SYSTEM == "osx":
		cursdict = {"arrow":StdCursors.arrow, "ibeam":StdCursors.ibeam, 
			"crosshair":StdCursors.crosshair, "fist":StdCursors.fist,
			"hand":StdCursors.hand, "finger":StdCursors.finger, "invisible":StdCursors.invisible}
		if curs in cursdict:
			if widget:
				widget._int().cursor = cursdict[curs]
			else:
				app.cursor = cursdict[curs]
		else:
			print "Sorry, I don't have that cursor."

def alert(text,critical=False):
	'''Launches an alert window with a given text.
	If critical is True, closing the alert terminates SWIFT.'''
	if SYSTEM=="gtk":
		def abutton_press_event(widget, event):
			#Close when "Ok" is pressed
			alert.destroy()
		def on_destroy(event):
			if critical:
				#if this is a critical alert, such as when another instrance of SWIFT is already running
				gtk.main_quit()
		alert = gtk.Window(type=gtk.WINDOW_TOPLEVEL)			# make a new window for the alert
		alert.set_position(gtk.WIN_POS_CENTER)					# put it in the center of the screen
		alert.set_type_hint(gtk.gdk.WINDOW_TYPE_HINT_DIALOG)	# tell the WM that it is a dialog
		alert.set_destroy_with_parent(True)						# if someone closes SWIFT, we want the alert to close too
		alert.set_modal(True)									# alert should block input to SWIFT until acknowledged
		alert.set_title("Alert")								# call it "Alert"
		alert.set_size_request(250, 100)						# make it 250x100
		avbox = vbox = gtk.VBox(False, 0)						# create a vertical box container
		alert.add(avbox)										# add said container to the alert
		alabel=gtk.Label(text)									# make a label to hold the text
		alabel.set_size_request(240,-1)							# make it slightly narrower than the parent window
		alabel.set_line_wrap(True)								# we want the text to word wrap
		alabel.set_justify(gtk.JUSTIFY_CENTER)					# center the text
		avbox.pack_start(alabel, False, True, 0)				# put it at the start of the vbox
		alabel.show()											# make it visible
		abutton = gtk.Button("OK")								# new button with text "OK"
		abutton.set_use_stock(True)								# it is a stock button provided by GTK
		abutton.set_size_request(50, -1)						# we don't want it as wide as the whole alert
		abutton.set_events(gtk.gdk.ALL_EVENTS_MASK);			# capture clicks, keyboard, anything
		abutton.connect("button_press_event", abutton_press_event)	# but only listen to the clicks
		alert.connect("destroy", on_destroy)					# close alert when alert is closed
		abutton.show()											# make button visible
		ahbox = gtk.HBox(False, 10)								# make a new hbox
		ahbox.pack_start(abutton, True, False, 0)				# put the button in it, but don't make it expand
		avbox.pack_start(ahbox, True, False, 0)					# put the hbox in the vbox
		ahbox.show()											# make it visible
		avbox.show()											# make the vbox visible
		alert.show()											# make the alert itself visible
	elif SYSTEM=="osx":
		#Much simpler. :)
		if critical:
			Alerts.stop_alert(text)			# stop_alert is a critical error alert
			sys.exit(0)						# Exit to OS. Sometimes we can't recover an error.
		else:
			Alerts.note_alert(text)			# note_alert is a general alert, i.e. "I'm a computer."
	elif SYSTEM=="html":
		jscommunicate("alert("+text+")")
		if critical:
			# reloading the page is equivalent to force-quitting, right?
			jscommunicate("window.location.reload()")

def file_dialog(mode="open",default=None,types=None,multiple=False):
	if SYSTEM=="osx":
		if mode=="open":
			if multiple:
				return FileDialogs.request_old_files(default_dir=default,file_types=types)
			else:
				return FileDialogs.request_old_file(default_dir=default,file_types=types)
		elif mode=="save":
			return FileDialogs.request_new_file(default_dir=default,file_type=types)

def execute(command):
	os.system(command.replace("/",sep))

class OverlayWindow:
	if SYSTEM=="gtk":
		def expose (widget, event, startime=time.time()):
			cr = widget.window.cairo_create()
			# Sets the operator to clear which deletes everything below where an object is drawn
			cr.set_operator(cairo.OPERATOR_CLEAR)
			# Makes the mask fill the entire window
			cr.rectangle(0.0, 0.0, *widget.get_size())
			# Deletes everything in the window (since the compositing operator is clear and mask fills the entire window
			cr.fill()
			# Set the compositing operator back to the default
			cr.set_operator(cairo.OPERATOR_OVER)
			
			widget.present()
		
			# Clear background to transparent black
			cr.set_source_rgba(0.0,0.0,0.0,2.0-(time.time()-startime))
			cr.paint()
			image = "media/logo-transparent.png"
			surface = cairo.ImageSurface.create_from_png(image)
			pat = cairo.SurfacePattern(surface)
			cr.set_source(pat)
			cr.paint_with_alpha(2.0-(time.time()-startime))
			#return True
		
		
		win = gtk.Window()
		win.set_decorated(False)
		win.set_modal(True)
		
		# Makes the window paintable, so we can draw directly on it
		win.set_app_paintable(True)
		win.set_size_request(512, 512)
		win.set_position(gtk.WIN_POS_CENTER)
		
		# This sets the windows colormap, so it supports transparency.
		# This will only work if the wm support alpha channel
		screen = win.get_screen()
		rgba = screen.get_rgba_colormap()
		win.set_colormap(rgba)
		win.connect('expose-event', expose)
		win.show()
		win.present()
		gobject.timeout_add(50, expose, win, 'fade-event', time.time())
		gobject.timeout_add(2000, win.destroy)

		
class ColorSelectionWindow:
	def __init__(self,var,dispgrp, dcanv):
		if SYSTEM=="gtk":
			win = gtk.Window()
			win.set_size_request(320,208)
			win.set_decorated(False)
			darea = gtk.DrawingArea()
			darea.set_size_request(320,208)
			win.add(darea)
			win.show_all()
			win.add_events(gtk.gdk.EXPOSURE_MASK
									| gtk.gdk.LEAVE_NOTIFY_MASK
									| gtk.gdk.BUTTON_PRESS_MASK
									| gtk.gdk.BUTTON_RELEASE_MASK
									| gtk.gdk.KEY_PRESS_MASK
									| gtk.gdk.POINTER_MOTION_MASK
									| gtk.gdk.POINTER_MOTION_HINT_MASK)
			def expose_event(widget, event):
				x,y,w,h = widget.allocation
				surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, w,h)
				cr = cairo.Context(surface)
				cra = widget.window.cairo_create()
				cr.set_source_rgb(0.5, 0.5, 0.5)
				cr.paint()
				for i in xrange(21):
					for j in xrange(len(colors.colorArray(i))):
						cr.rectangle(i*16,j*16,15,15)
						r,g,b = colors.colorArray(i)[j]
						cr.set_source_rgb(r,g,b)
						cr.fill()
				cra.set_source_surface(surface)
				cra.paint()
			def close(widget, event, dispshape, dcanv):
				global LINECOLOR,FILLCOLOR
				if var=="line":
					LINECOLOR = Color(colors.colorArray(int(event.x)/16)[int(event.y)/16])
				elif var=="fill":
					FILLCOLOR = Color(colors.colorArray(int(event.x)/16)[int(event.y)/16])
				dispgrp.frames[0].objs[2].fillcolor = LINECOLOR if var=="line" else FILLCOLOR
				dcanv.draw()
				widget.destroy()
			darea.connect("expose-event",expose_event)
			win.connect("button-press-event", close, dispgrp, dcanv)
			win.set_modal(True)
			win.present()

def main():
	#Executes the main loop for whatever GUI is running
	if SYSTEM=="gtk":
		gtk.main()
	elif SYSTEM=="osx":
		global app
		app.menus = menus
		app.run()
	elif SYSTEM=="html":
		print __windowlist__[0].window
		pass

def quit():
	#Self-descriptive
	if SYSTEM=="gtk":
		gtk.main_quit()
	elif SYSTEM=="android":
		sys.exit(0)
		
def jscommunicate(string):
	pass
def jsdefine(func, args, body):
	global jsdefs, jsfunctions
	if not func in jsdefs:
		jsfunctions = jsfunctions+"function "+func+args+" {\n"+body+"\n};\n"
		jsdefs.append(func)

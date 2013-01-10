#! /usr/bin/python
# -*- coding:utf-8 -*-
# © 2012 Skyler Lehmkuhl
# Released under the GPLv3. For more information, see gpl.txt.

import os
import sys
import math
import random
import colors
import platform
import re
import shutil

import traceback
try:
	from PIL import Image as PILimage
	GLEnablable = True
except ImportError:
	GLEnablable = False

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
MODE=" "

#Painbrush mode
PMODE = "Draw straight"

SITER=0

#Currentframe - the frame selected on the timeline. Not necessarily the frame being shown.
CURRENTFRAME=0

#Framerate - speed at which animation is played back
FRAMERATE=50

#Width and height are the width and height of the document
WIDTH, HEIGHT = 500, 500

#Whether we are using OpenGL for rendering. Currently we need PIL for this.
if GLEnablable:
	try:
		import OpenGL
		USING_GL = True
	except ImportError:
		USING_GL = False

# Disable OpenGL. It's not ready for prime time.
USING_GL = False

#Object which has the keyboard focus.
FOCUS = None

#Options for export
EXPORT_OPTS = {"swf":False,"html5":False,"basehtml":False,"fallback":False,"pack":False}

#Editing - whether the user is editing text
EDITING = True

CURRENTTEXT = None

#Scaling - whether the user is resizing an object
SCALING = False

#Library. Contatins all objects whether displayed or not.
Library = []


class Color (object):
	def __getstate__(self):
		dict = self.__dict__.copy()
		print dict
		dict['image'] = None
		return dict
	def __init__(self, val):
		if type(val)==type([]):
			self.type = "RGB"
			self.val = val
		elif isinstance(val, basestring):
			if val.startswith("#"):
				self.type = "RGB"
				self.val = hex2rgb(val)
			else:
				global Library
				Library.append(self)
				self.type = "Image"
				self.val = val
				self.set_image(val)
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
		elif self.type=="Image":
			a = Colors.rgb(0,0,0,1,image=True,im=self.image)
			return a
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
	def set_image(self, path):
		if SYSTEM=="osx":
			self.image = GUI.Image(file=path)
	def print_sc(self):
		retval = ".png "+self.val.split('/')[-1].replace(' ','_').replace('.','_')+" \""+self.val+"\"\n"
		return retval
	def print_html(self):
		shutil.copy(self.val, os.getenv('HOME')+"/"+self.val.split('/')[-1])
		retval = "var "+self.val.split('/')[-1].replace(' ','_').replace('.','_')+" = new Image();\n"
		retval = retval+self.val.split('/')[-1].replace(' ','_').replace('.','_')+".src = \""+self.val.split("/")[-1]+"\";\n"
		return retval
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
FILLCOLOR = Color("#00FF00")
TEXTCOLOR = Color("#000000")

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
	import tarfile
	import tempfile
	import GUI		# Using PyGUI. Experimental.
	from GUI import Window as OSXWindow, Button as OSXButton, Image as OSXImage
	from GUI import Frame as OSXFrame, Color as OSXColor, Grid as OSXGrid, CheckBox as OSXCheckBox
	from GUI import Label as OSXLabel, RadioGroup as OSXRadioGroup, RadioButton as OSXRadioButton
	from GUI import Column, Row, ScrollableView, TextEditor, Colors, ModalDialog
	from GUI import StdCursors, Alerts, FileDialogs, Font, TextField, Slider
	from GUI.StdMenus import basic_menus, file_cmds, print_cmds
	from GUI.StdButtons import DefaultButton, CancelButton
	from GUI.Files import FileType
	if USING_GL:
		from OpenGL.GL import *
		from OpenGL.GLU import *
		from GUI import GL
		try:
			from PIL import Image as PILImage
		except ImportError as err:
			import Image as PILImage
	from GUI.Geometry import offset_rect, rect_sized
	
	#If we can import this, we are in the install directory. Mangle media paths accordingly.
	try:
		from distpath import media_path
	except:
		media_path = ""
	#app = GUI.application()
	SYSTEM="osx"
	TEMPDIR = "/tmp"
	FONT = u'Times New Roman'
	'''
	SYSTEM="html"
	ids = {}
	jsdefs = []
	jsfunctions = ""'''
	sep = "/"
elif sys.platform=="win32":
	PLATFORM="win32"
	import pickle
	import tarfile
	import tempfile
	import misc_funcs
	import GUI		# Using PyGUI. Experimental.
	from GUI import Window as OSXWindow, Button as OSXButton, Image as OSXImage
	from GUI import Frame as OSXFrame, Color as OSXColor, Grid as OSXGrid, CheckBox as OSXCheckBox
	from GUI import Label as OSXLabel, RadioGroup as OSXRadioGroup, RadioButton as OSXRadioButton
	from GUI import Column, Row, ScrollableView, TextEditor, Colors, ModalDialog
	from GUI import StdCursors, Alerts, FileDialogs, Font, TextField, Slider
	from GUI.StdMenus import basic_menus, file_cmds, print_cmds
	from GUI.StdButtons import DefaultButton, CancelButton
	from GUI.Files import FileType
	from GUI.Geometry import offset_rect, rect_sized
	if USING_GL:
		from OpenGL.GL import *
		from OpenGL.GLU import *
		from GUI import GL
		try:
			from PIL import Image as PILImage
		except ImportError as err:
			import Image as PILImage
	media_path = ""
	SYSTEM="osx"
	TEMPDIR="C:\\Windows\\Temp"
	sep = "\\"
elif sys.platform=="linux-armv6l":
	import android
	import tarfile
	import tempfile
	droid = android.Android()
	SYSTEM="android"
	TEMPDIR="/tmp"		# TODO:FIXTHIS
	media_path = ""
	tb = ""
	sep = "/"
	print str(sys.platform)
elif sys.platform=="darwin":
	PLATFORM="osx"
	import pickle
	import misc_funcs
	import tarfile
	import tempfile
	#'''
	import GUI		# Using PyGUI. Experimental.
	from GUI import Window as OSXWindow, Button as OSXButton, Image as OSXImage
	from GUI import Frame as OSXFrame, Color as OSXColor, Grid as OSXGrid, CheckBox as OSXCheckBox
	from GUI import Label as OSXLabel, RadioGroup as OSXRadioGroup, RadioButton as OSXRadioButton
	from GUI import Column, Row, ScrollableView, TextEditor, Colors, ModalDialog
	from GUI import StdCursors, Alerts, FileDialogs, Font, TextField, Slider
	from GUI.StdMenus import basic_menus, file_cmds, print_cmds
	from GUI.StdButtons import DefaultButton, CancelButton
	from GUI.Files import FileType
	from GUI.Geometry import offset_rect, rect_sized
	if USING_GL:
		from OpenGL.GL import *
		from OpenGL.GLU import *
		from GUI import GL
		try:
			from PIL import Image as PILImage
		except ImportError as err:
			import Image as PILImage
	SYSTEM="osx"
	'''
	import pyglet	# Using Pyglet. Even more experimental. As in doesn't work yet.
	SYSTEM="pyglet"
	'''  # comment these out to use pyglet
	import Cocoa
	SYSTEM_FONTS = list(Cocoa.NSFontManager.sharedFontManager().availableFontFamilies())
	FONT_PATH = "/Library/Fonts/"
	FONT = u'Times New Roman'
	media_path = ""
	#app = GUI.application()
	SYSTEM="osx"
	TEMPDIR="/tmp"
	sep = "/"
	
if SYSTEM=="osx":
	from codeeditor import CodeEditor

FILE = tarfile.open(name=TEMPDIR+"/Untitled",mode="w:gz")
FILE.close()

#Used for storing images, sounds, etc.
SECURETEMPDIR = tempfile.mkdtemp()
	
__windowlist__=[]

if SYSTEM=="osx":
	class Lightningbeam(GUI.Application):
		def __init__(self):
			GUI.Application.__init__(self)
			self.file_type = FileType(name = "Untitled Document", suffix = "beam", 
				mac_creator = "LNBM", mac_type = "BEAM"), # These are optional)
		def setup_menus(self, m):
			m.about_cmd.enabled = 1
			m.quit_cmd.enabled = 1
			m.save_cmd.enabled = 1
			m.save_as_cmd.enabled = 1
			m.open_cmd.enabled = 1
			m.undo_cmd.enabled = 1
			m.redo_cmd.enabled = 1
			m.run_file.enabled = 1
			m.run_html.enabled = 1
			m.create_sc.enabled = 1
			m.add_keyframe.enabled = 1
			m.add_layer.enabled = 1
			m.delete_layer.enabled = 1
			m.bring_forward.enabled = 1
			m.bring_to_front.enabled = 1
			m.send_backward.enabled = 1
			m.send_to_back.enabled = 1
			m.import_to_stage.enabled = 1
			m.import_to_library.enabled = 1
			m.convert_to_symbol.enabled = 1
			m.preferences_cmd.enabled = 1
		
		#def create_sc(self):
		#	pass
		#def run_file(self):
		#	pass
	class LightningbeamWindow(OSXWindow):
		def __init__(self,*args,**kwargs):
			OSXWindow.__init__(self,*args,**kwargs)
		#def save_cmd(widget=None):
		#	print "to save"
		#def key_down(self, event):
		#	if FOCUS:
		#		FOCUS.key_down(event)
		#def key_up(self, event):
		#	if FOCUS:
		#		FOCUS.key_up(event)
			
			
	app = Lightningbeam()
elif SYSTEM=="html":
	app = ""

	
class ObjectDeletedError:
	def __str__(self):
		return "Object deleted!"


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
	def __init__(self, title="", closable=True):
		__windowlist__.append(self)
		if SYSTEM=="gtk":
			self.window = gtk.Window()
			self.vbox = gtk.VBox()
			self.window.add(self.vbox)
			self.window.show_all()
			self.window.connect("destroy",self.destroy)
		elif SYSTEM=="osx":
			self.window = LightningbeamWindow(width=1024,height=500, closable=closable)
			if not title=="":
				self.window.title = title
			#components = [i._int() for i in args]
			#self.vbox = GUI.Column(components, equalize="w", expand=0)
			#self.window.place(self.vbox, left = 0, top = 0, right = 0, bottom = 0, sticky = 'nsew')
			self.window.show()
		elif SYSTEM=="html":
			self.window = htmlobj("div")
		elif SYSTEM=="pyglet":
			self.window = pyglet.window.Window(resizable=True)

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
		elif SYSTEM=="pyglet":
			pass	# how to do?
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
		elif SYSTEM=="pyglet":
			self.window.maximize()
	def set_title(self, title):
		if SYSTEM=="gtk":
			self.window.set_title(title)
		elif SYSTEM=="osx":
			self.window.title = title
		elif SYSTEM=="html":
			jscommunicate("document.title = "+title)
		elif SYSTEM=="pyglet":
			self.window.set_caption(title)
	def resize(self,x,y):
		if SYSTEM=="osx":
			self.window.resize(width=x,height=y)
		elif SYSTEM=="pyglet":
			self.window.set_size(width=x, height=y)
			
# Widget meta-class - to prevent code duplication
# I don't seem to have any code in here. :(
# Now used as generic wrapper class
class Widget(object):
	def __init__(self,obj):
		self.obj = obj
	def _int(self):
		return self.obj
	
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
		elif SYSTEM=="pyglet":
			pass
			#Ditto.
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
				menu = GUI.Menu(i[0],[((k[0]+k[2],k[1].__name__) if (len(k)==3 and "/" in k[2]) else (k[0],k[1].__name__)) for k in i if type(k)==type(())])
				#menu = GUI.Menu("Test", [("Run", 'run_file')])
				menus.append(menu)
			else:
				cmds={"Save":"save_cmd", "Save As":"save_as_cmd", "Open":"open_cmd","About Lightningbeam...":"about_cmd",\
					"Preferences":"preferences_cmd", "Undo":"undo_cmd", "Redo":"redo_cmd"}
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
		elif SYSTEM=="pyglet":
			self.vbox=None	#TODO
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
		elif SYSTEM=="pyglet":
			self.hbox=None	#TODO
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
class Label(Widget):
	def _gettext(self):
		if SYSTEM=="osx":
			return self.label.text
		elif SYSTEM=="pyglet":
			return self.label.text
	def _settext(self,text):
		if SYSTEM=="osx":
			self.label.text=text
		elif SYSTEM=="pyglet":
			self.label.text=text
	text = property(_gettext,_settext)
	def __init__(self, text=""):
		if SYSTEM=="osx":
			self.label = OSXLabel()
			self.label.text = text
		elif SYSTEM=="pyglet":
			self.label = pyglet.text.Label(text)
	def _int(self):
		if SYSTEM=="osx":
			return self.label
	def disable(self):
		if SYSTEM=="osx":
			self.label.enabled = False
		elif SYSTEM=="pyglet":
			self.label.color = (0, 0, 0, 100)
	def enable(self):
		if SYSTEM=="osx":
			self.label.enabled = True
		elif SYSTEM=="pyglet":
			self.label.color = (0, 0, 0, 255)
class RadioGroup(Widget):
	def __getitem__(self,num):
		return self.buttons[num]
	def __init__(self,*args):
		self.buttons = []
		for i in args:
			self.buttons.append(RadioButton(i))
		self.group = OSXRadioGroup([j._int() for j in self.buttons])
		self.group.value = args[0]
		self.group.action = self._action
	def _int(self):
		return self.group
	def _getvalue(self):
		return self.group.value
	def _setvalue(self,value):
		self.group.value = value
	value = property(_getvalue, _setvalue)
	def _action(self):
		self.action(self)
	def action(self,self1=None):
		pass

class RadioButton(Widget):
	def __init__(self,title):
		if SYSTEM=="osx":
			self.button = OSXRadioButton(title=title,value=title)
	def _int(self):
		return self.button
			
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
		elif SYSTEM=="pyglet":
			self.button = None	#TODO
	def _int(self):
		return self.button
	def set_text(self, text):
		if SYSTEM=="osx":
			self.button.title = text
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
	if SYSTEM=="pyglet":
		scroll_imgs =  [pyglet.image.load("Themes/Default/gtk-2.0/Scrollbar/horizontal_trough.png"),
						pyglet.image.load("Themes/Default/gtk-2.0/Scrollbar/scrollbar_horizontal.png"),
						pyglet.image.load("Themes/Default/gtk-2.0/Scrollbar/scrollbar_horizontal_prelight.png"),
						pyglet.image.load("Themes/Default/gtk-2.0/Scrollbar/vertical_trough.png"),
						pyglet.image.load("Themes/Default/gtk-2.0/Scrollbar/scrollbar_vertical.png"),
						pyglet.image.load("Themes/Default/gtk-2.0/Scrollbar/scrollbar_vertical_prelight.png")]
	#sch controls the horizontal scrollbar, scv controls the vertical one
	def __init__(self,sch=True,scv=True):
		if SYSTEM=="gtk":
			self.sw = gtk.ScrolledWindow()
			self.sw.set_policy(gtk.POLICY_ALWAYS if sch else gtk.POLICY_AUTOMATIC, gtk.POLICY_ALWAYS if scv else gtk.POLICY_AUTOMATIC)
		elif SYSTEM=="pyglet":
			self.x = 0
			self.y = 0
			self.clickedhoriz = False
			self.clickedvert = False
			self.xoffset = 0	# Offset from mouse click point
			self.yoffset = 0	#
			self.hx = 0		# Scroll distance
			self.vy = 0
			self.horiztrough = pyglet.sprite.Sprite(self.scroll_imgs[0])
			self.vertrough = pyglet.sprite.Sprite(self.scroll_imgs[3])
			self.horizbar = pyglet.sprite.Sprite(self.scroll_imgs[1])
			self.vertbar = pyglet.sprite.Sprite(self.scroll_imgs[4])
			pass 	# TODO: Content.					
	def _int(self):
		return self.sw
	def add(self,obj):
		objint = obj._int()
		self.sw.add_with_viewport(objint)
	def draw(self):
		#Pyglet-specific.
		if not SYSTEM=="pyglet":
			print "Called from wrong GUI!"
			return
		self.horiztrough.set_position(self.x, self.y)
		self.horiztrough.draw()
		if self.clickedhoriz:
			self.clickedvert = False # we should never be dragging two scrollbars at the same time!
			self.horizbar.image = self.scroll_imgs[2]
			self.horizbar.set_postion(self.x+self.hx, self.y)
			

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
			self.width = self.frame.width
			self.height = self.frame.height
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
	def setvisible(self,visible):
		if SYSTEM=="osx":
			if visible:
				self.frame.height = self.height
				# Setting the height to 0 doesn't work on Linux, so we hack around it
				if PLATFORM.startswith("linux"):
					self.frame._gtk_inner_widget.set_property('visible', True)
			else:
				self.frame.height = 0
				if PLATFORM.startswith("linux"):
					self.frame._gtk_inner_widget.set_property('visible', False)
			print "visible:",visible

class Scale(Widget):
	def __init__(self,min,val,max):
		if SYSTEM=="osx":
			self.scale = Slider('h')
			self.scale.min_value = min
			self.scale.max_value = max
			self.scale.value = val
	def _int(self):
		return self.scale
	def set_action(self,action):
		if SYSTEM=="osx":
			self.scale.action = action
	def getval(self):
		return self.scale.value
	def setval(self, val):
		self.scale.value = val
	value = property(getval, setval)
	
class CheckBox(Widget):
	def __init__(self,text):
		if SYSTEM=="osx":
			self.box = OSXCheckBox(text)
			self.box.action = self._action
		elif SYSTEM=="pyglet":
			self.checked = False
	def _int(self):
		return self.box
	def _action(self):
		self.action()
	def action(self):
		pass
	def get_value(self):
		return self.box.value
	def set_value(self, value):
		self.box.value = value
	value = property(get_value, set_value)
		
class _CR(object):
	"""Internal use only. This is a class that only exists for GLViews 
	to pass window dimensions on to their children."""
	def __init__(self):
		self.x = 0
		self.y = 0
		self.stack = []
	def save(self):
		glPushAttrib(GL_ALL_ATTRIB_BITS);
		glPushMatrix()
		self.stack.append((self.x, self.y))
	def restore(self):
		self.x, self.y = self.stack.pop()
		glPopMatrix()
		glPopAttrib()
	def translate(self, x, y):
		glTranslatef(x, y, 0);
	def rotate(self, r):
		pass
	def scale(self, sx, sy):
		pass
	def drawCurve(self, points, precision=20):
		npoints = []
		ave = misc_funcs.ave
		s = range(0, len(points)-2, 2)
		for i in s:
			for j in range(precision):
				k=1.0*(precision-j)
				x=ave(	ave(ave(self.x, 
								points[i][0], 
								k/precision), 
							ave(points[i][0], 
								points[i+1][0], 
								k/precision), 
							k/precision),
						ave(ave(points[i][0],
								points[i+1][0],
								k/precision), 
							ave(points[i+1][0],
								points[i+2][0], 
								k/precision), 
							k/precision),
						k/precision)
							
				
				y=ave(	ave(ave(self.y, 
								points[i][1], 
								k/precision), 
							ave(points[i][1], 
								points[i+1][1], 
								k/precision), 
							k/precision),
						ave(ave(points[i][1],
								points[i+1][1],
								k/precision), 
							ave(points[i+1][1],
								points[i+2][1], 
								k/precision), 
							k/precision),
						k/precision)
				npoints.append((x, y))
		glVertex2f(self.x, self.y)
		glVertex2f(npoints[0][0], npoints[0][1])
		for i in range(len(npoints)-1):
			#drawLine(gc, drawable, npoints[i][0], npoints[i][1], npoints[i+1][0], npoints[i+1][1])
			# print npoints[i][0],npoints[i][1],npoints[i+1][0],npoints[i+1][1]
			glVertex2f(npoints[i][0], npoints[i][1])
			glVertex2f(npoints[i+1][0], npoints[i+1][1])
		glVertex2f(npoints[-1][0],npoints[-1][1])
		glVertex2f(*points[2])

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
			if USING_GL:
				class OSXCanvas(GL.GLView):
					def init_context(self):
						glClearColor(0.75,0.75,0.75,0.0)
					def init_projection(self):
						glViewport(0, 0, width, height);
						glEnable( GL_TEXTURE_2D );
						glEnable (GL_BLEND);
						#glDisable( GL_LIGHTING) 
						glBlendFunc (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
					def render(self):
						glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
						glLoadIdentity();
						gluOrtho2D(0, width, 0, height); #TODO: width, height
						glMatrixMode(GL_MODELVIEW);
						cr = _CR()
						cr.width = self.width
						cr.height = self.height
						for i in self.objs:
							i.draw(cr)

					def mouse_down(self, event):
						self.become_target()
						x, y = event.position
						try:
							for i in self.objs:
								i._onMouseDown(x, y, button={"left":1,"right":2,"middle":3}[event.button], clicks=event.num_clicks)
						except ObjectDeletedError:
							return
						self.update()
					def mouse_drag(self, event):
						x, y = event.position
						for i in self.objs:
							i._onMouseDrag(x, y, button={"left":1,"right":2,"middle":3}[event.button])
						self.update()
						
					def mouse_move(self, event):
						global MOUSE_X, MOUSE_Y
						MOUSE_X, MOUSE_Y = event.position
						x, y = event.position
						for i in self.objs:
							i._onMouseMove(x, y, button={"left":1,"right":2,"middle":3}[event.button])
						self.update()
						
					def mouse_up(self, event):
						x, y = event.position
						for i in self.objs:
							i._onMouseUp(x, y, button={"left":1,"right":2,"middle":3}[event.button])
						self.update()
						
					def key_down(self, event):
						keydict = {127:"backspace",63272:"delete",63232:"up_arrow",63233:"down_arrow",
										63235:"right_arrow",63234:"left_arrow",13:"enter",9:"tab",
										63236:"F1",63237:"F2",63238:"F3",63239:"F4",63240:"F5",
										63241:"F6",63242:"F7",63243:"F8",27:"escape"}
						if not event.unichars=='':
							if ord(event.unichars) in keydict:
								key = keydict[ord(event.unichars)]
							else:
								key = event.unichars
						else:
							key = event.key.upper()
						for i in self.objs:
							i._onKeyDown(key)
						self.update()
					
					def key_up(self, event):
						pass

				self.canvas = OSXCanvas()
				self.canvas.update()
			else:
				class OSXCanvas (ScrollableView):
					def draw(self, canvas, update_rect):
						canvas.backcolor = Color("#888888").pygui
						canvas.erase_rect(update_rect)
						canvas.fillcolor = Color("#ffffff").pygui
						canvas.fill_rect((0,0,WIDTH,HEIGHT))
						for i in self.objs:
							try:
								i.draw(canvas)
							except:
								traceback.print_exc()

					def mouse_down(self, event):
						self.become_target()
						x, y = event.position
						try:
							try:
								for i in self.objs:
									i._onMouseDown(x, y, button={"left":1,"right":2,"middle":3}[event.button], clicks=event.num_clicks)
							except ObjectDeletedError:
								return
						except:
							traceback.print_exc()
						self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
						
					def mouse_drag(self, event):
						x, y = event.position
						for i in self.objs:
							try:
								i._onMouseDrag(x, y, button={"left":1,"right":2,"middle":3}[event.button])
							except:
								traceback.print_exc()
						self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
						
					def mouse_move(self, event):
						x, y = event.position
						for i in self.objs:
							i._onMouseMove(x, y)
						self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
						
					def mouse_up(self, event):
						x, y = event.position
						for i in self.objs:
							i._onMouseUp(x, y, button={"left":1,"right":2,"middle":3}[event.button], clicks=event.num_clicks)
						self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
						
					def key_down(self, event):
						keydict = {127:"backspace",63272:"delete",63232:"up_arrow",63233:"down_arrow",
										63235:"right_arrow",63234:"left_arrow",13:"enter",9:"tab",
										63236:"F1",63237:"F2",63238:"F3",63239:"F4",63240:"F5",
										63241:"F6",63242:"F7",63243:"F8",27:"escape"}
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
			class OSXTextEditor(GUI.TextEditor):
				
				def mouse_down(self, event):
					self.become_target()
					GUI.TextEditor.mouse_down(self, event)
			# self.box = OSXTextEditor(scrolling="hv")
			self.box = CodeEditor()
			self.box.font = Font("Courier", 12, [])
			if width and height:
				self.box.size = (width, height)
		elif SYSTEM=="html":
			self.box = htmlobj("textarea")
	def _int(self):
		if SYSTEM=="gtk":
			return self.sw._int()
		elif SYSTEM=="osx":
			return self.box
		elif SYSTEM=="html":
			return self.box
	def scroll_bottom(self):
		if SYSTEM=="osx":
			self.scroll_page_down();

class TextEntry(Widget):
	def __init__(self,text="",password=False):
		if SYSTEM=="osx":
			self.entry = TextField(text=text,multiline=False,password=password)
	def _int(self):
		if SYSTEM=="osx":
			return self.entry
	def disable(self):
		self.entry.enabled = False
	def enable(self):
		self.entry.enabled = True
	def set_action(self,action):
		if SYSTEM=="osx":
			self.entry.enter_action = action
	def get_text(self):
		return self.entry.text
	def set_text(self, text):
		self.entry.text = text
	text = property(get_text, set_text)
		
class Image(object):
	def __getstate__(self):
		dict = self.__dict__.copy()
		print dict
		dict['image'] = None
		dict['pilimage'] = None
		return dict
	def __init__(self,image,x=0,y=0,animated=False,canvas=None,htiles=1,vtiles=1,skipl=False):
		if not skipl:
			global Library
			Library.append(self)
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
		self.name = image.split(sep)[-1]
		self.iname = None
		self.path = image
		self.type="Image"
		if USING_GL:
			self.pilimage = PILimage.open(image)
			# This is an OpenGL texture ID.
			self.gltexture = self.loadGLImage(file = image)
			
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
		self.shapedata = [['M',0,0],['L',self.maxx,0],['L',self.maxx,self.maxy],['L',0,self.maxy],['L',0,0]]
	def _int(self):
		return self.image
	def draw(self, cr=None, parent=None, rect=None):
		if SYSTEM=="android":
			pass
		elif SYSTEM=="osx":
			if USING_GL:
				pass
				glEnable(GL_TEXTURE_2D);
				glColor3f(0.5,0.5,0.5);
				#self.gltexture.bind()
				#self.gltexture.gl_tex_image_2d(self.image, with_mipmaps=True)
				
				glBindTexture(GL_TEXTURE_2D, self.gltexture)
				
				if self.animated:
					src_rect = [(1.0/self.htiles)*(self.pointer%self.htiles),
								(1.0/self.vtiles)*(self.pointer/self.htiles),
								(1.0/self.htiles)*(self.pointer%self.htiles+1),
								(1.0/self.vtiles)*(self.pointer/self.htiles+1)]
					width = self.image.bounds[2]/self.htiles
					height = self.image.bounds[3]/self.vtiles
				else:
					src_rect = [0.0, 0.0, 1.0, 1.0]
					width, height = self.image.bounds
				glBegin(GL_QUADS);
				glTexCoord2f(0.0, 0.0);
				#glTexCoord2f(src_rect[0], src_rect[1]);
				glVertex2f( self.x, cr.height-(self.y));
				glTexCoord2f(1.0, 0.0);
				#glTexCoord2f(src_rect[2], src_rect[1]);
				glVertex2f(width+self.x, cr.height-(self.y));
				glTexCoord2f(1.0, 1.0);
				#glTexCoord2f(src_rect[2], src_rect[3]);
				glVertex2f( width+self.x, cr.height-(height+self.y));
				glTexCoord2f(0.0, 1.0);
				#glTexCoord2f(src_rect[0], src_rect[3]);
				glVertex2f( self.x, cr.height-(height+self.y));
				# print src_rect
				glEnd();
			else:
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
					dst_rect = [self.x,self.y,self.x+src_rect[2],self.y+src_rect[3]]
					self.image.draw(cr, src_rect, dst_rect)
				cr.grestore()
		elif SYSTEM=="html":
			cr.save()
			pass
	def set_image(self,img):
		if SYSTEM=="osx":
			self.image = GUI.Image(file = img)
			if USING_GL:
				self.gltexture = self.loadGLImage(file = img)
	def loadGLImage(self, file = None ):
		"""Load an image file as a 2D texture using PIL"""
		if not file:
			file = self.path
		im = PILImage.open(file)
		try:
			ix, iy, image = im.size[0], im.size[1], im.tostring("raw", "RGBA", 0, -1)
		except SystemError:
			ix, iy, image = im.size[0], im.size[1], im.tostring("raw", "RGBX", 0, -1)
		ID = glGenTextures(1)
		glBindTexture(GL_TEXTURE_2D, ID)
		glPixelStorei(GL_UNPACK_ALIGNMENT,1)
		glTexImage2D(
			GL_TEXTURE_2D, 0, 3, ix, iy, 0,
			GL_RGBA, GL_UNSIGNED_BYTE, image
		)
		return ID
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
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseMove(self, self1, x, y, button=1, clicks=1):
		pass
	def onKeyDown(self, self1, key):
		pass
	def onKeyUp(self, self1, key):
		pass
	def print_sc(self):
		return ".png "+self.name+" \""+self.path+"\"\n"
	
class Shape (object):
	def __init__(self,x=0,y=0,rotation=0,fillcolor=None,linecolor=None):
		global SITER
		global Library
		Library.append(self)
		self.x=x
		self.y=y
		self.rotation=rotation
		self.xscale = 1
		self.yscale = 1
		self.linecolor = linecolor if linecolor else LINECOLOR
		self.fillcolor = fillcolor if fillcolor else FILLCOLOR
		self.linewidth = 2
		self.shapedata=[]
		self.filled=False
		self.type="Shape"
		self.iname = None
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
			cr.set_line_width(max(self.linewidth,1))
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
			tb+="cr.lineWidth = "+str(max(self.linewidth,1))+"\n"
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

			if USING_GL:
				cr.save()
				cr.translate(self.x, cr.height-self.y)
				# cr.translate(MOUSE_X, MOUSE_Y)
				cr.rotate(self.rotation)
				cr.scale(self.xscale*1.0, self.yscale*1.0)
				
				#pencolor, fillcolor, pensize
				#Temporary.
				glColor3f(1.0,0.0,0.0)
				
				glBegin(GL_LINES)
				for i in self.shapedata:
					if i[0]=="M":
						point = (i[1], i[2])
						#glVertex2f(point[0], cr.height-point[1])
						cr.x, cr.y = point
					elif i[0]=="L":
						point = (i[1], i[2])
						#glVertex2f(point[0], cr.height-point[1])
						glVertex2f(cr.x, -cr.y)
						glVertex2f(point[0], -point[1])
						cr.x, cr.y = point
					elif i[0]=="C":
						pointa = (i[1], -i[2])
						pointb = (i[3], -i[4])
						pointc = (i[5], -i[6])
						#TODO: curve
						#glVertex2f(pointc[0], -pointc[1])
						#glVertex2f(pointc[0], -pointc[1])
						cr.drawCurve([ pointa, pointb, pointc])
						cr.x, cr.y = pointc
				glEnd()
				
				cr.restore()
			else:
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
					pass
					# cr.translate(self.x,self.y)
					cr.translate(self.x/(self.xscale*1.0),self.y/(self.yscale*1.0))
				cr.rotate(self.rotation)
				cr.scale(self.xscale*1.0, self.yscale*1.0)
				cr.newpath()
				cr.pencolor = self.linecolor.pygui
				cr.fillcolor = self.fillcolor.pygui
				cr.pensize = max(self.linewidth,1)
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
			tb+="cr.lineWidth = "+str(max(self.linewidth,1))+"\n"
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
			yfactor = height/self.maxy
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
		print hits, x, y
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
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseMove(self, self1, x, y, button=1, clicks=1):
		pass
	def onKeyDown(self, self1, key):
		pass
	def onKeyUp(self, self1, key):
		pass
	minx = property(getminx)
	miny = property(getminy)
	maxx = property(getmaxx)
	maxy = property(getmaxy)
	def print_sc(self):
		retval = ""
		retval+=".outline "+self.name+"outline:\n"
		retval+=" ".join([" ".join([str(x) for x in a]) for a in self.shapedata])+"\n.end\n"
		if self.filled:
			if self.fillcolor.type=="Image":
				retval+=".filled "+self.name+" outline="+self.name+"outline fill="+self.fillcolor.val.split('/')[-1].replace(' ','_').replace('.','_')+" color="+self.linecolor.rgb+" line="+str(self.linewidth)+"\n"
			else:
				retval+=".filled "+self.name+" outline="+self.name+"outline fill="+self.fillcolor.rgb+" color="+self.linecolor.rgb+" line="+str(self.linewidth)+"\n"
		else:
			retval+=".filled "+self.name+" outline="+self.name+"outline fill=#00000000 color="+self.linecolor.rgb+" line="+str(self.linewidth)+"\n"
		return retval
	def print_html(self):
		retval = "var "+self.name+" = new Shape();\n"+self.name+"._shapedata = "+str(self.shapedata)+";\n"
		if self.fillcolor.type=="Image":
			retval += self.name+".fill = "+self.fillcolor.val.split('/')[-1].replace(' ','_').replace('.','_')+";\n"+self.name+".line = \""+self.linecolor.rgb+"\";\n"
		else:
			retval += self.name+".fill = \""+self.fillcolor.rgb+"\";\n"+self.name+".line = \""+self.linecolor.rgb+"\";\n"
		retval += self.name+".filled = "+str(self.filled).lower()+";\n"
		return retval

class Text (object):
	def __init__(self,text="",x=0,y=0):
		global SITER
		global Library
		Library.append(self)
		self.text = text
		self.x = x
		self.y = y
		self.rotation = 0
		self.xscale = 1
		self.yscale = 1
		self.fill = TEXTCOLOR
		self.font = Font(FONT,16)
		self.dynamic = False
		self.variable = None
		self.password = False
		self.wordwrap = False
		self.multiline = False
		self.html = False
		self.editable = False
		self.selectable = True
		self.border = False
		# self.width = self.font.width(self.text)
		self.height = self.font.height
		self.iname = None
		self.hwaccel = False
		self.type="Text"
		self.name = "t"+str(int(random.random()*10000))+str(SITER)
		self.editing = False
		self.cursorpos = len(self.text)
		SITER+=1
	def draw(self,cr=None,parent=None,rect=None):
		if SYSTEM=="osx":
			if USING_GL:
				pass
			else:
				cr.font = self.font
				cr.textcolor = self.fill.pygui
				cr.gsave()
				#cr.moveto(self.x,self.y)
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
				if self.editing:
					w = self.font.width(self.text)
					d = self.font.descent
					h = self.font.height
					lines = self.text.count('\n')
					cr.newpath()
					cr.moveto(0,d+h*lines)
					cr.lineto(w,d+h*lines)
					cr.lineto(w,-h)
					cr.lineto(0,-h)
					cr.lineto(0,d+h*lines)
					cr.pencolor = Color([0,0,0]).pygui
					cr.fillcolor = Color([1,1,1]).pygui
					cr.fill_stroke()
					cr.fill()
					if '\n' in self.text[:self.cursorpos]:
						cw = self.font.width(self.text[self.text.rindex('\n',0,self.cursorpos):self.cursorpos])
					else:
						cw = self.font.width(self.text[:self.cursorpos])
					cr.newpath()
					elines = self.text[:self.cursorpos].count('\n')
					cr.moveto(cw,d+h*elines)
					cr.lineto(cw,-h+h*elines)
					cr.stroke()
				cr.newpath()
				cr.moveto(0,0)
				cr.show_text(self.text)
				cr.grestore()
	def hitTest(self, x, y):
		self.height = self.font.height
		if 0<x<self.width and -self.height<y<0:
			return True
	def getwidth(self):
		return self.font.width(self.text)
	def getminx(self):
		return 0
	def getminy(self):
		return -self.height
	def getmaxx(self):
		return self.width
	def getmaxy(self):
		return 0
	def getsize(self):
		return self.font.size
	def setsize(self,size):
		self.font = Font(self.font.family,size,self.font.style)
		self.height = self.font.height
	minx = property(getminx)
	miny = property(getminy)
	maxx = property(getmaxx)
	maxy = property(getmaxy)
	size = property(getsize,setsize)
	width= property(getwidth)
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseDrag(self, self1, x, y, button=1):
		pass
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseMove(self, self1, x, y, button=1):
		pass
	def onKeyDown(self, self1, key):
		if key == "\b":
			self.text = self.text[:-1]
		elif key == "enter":
			self.text = self.text[:self.cursorpos]+"\n"+self.text[self.cursorpos:]
			self.height = self.font.height*(self.text.count('\n')+1)
			self.cursorpos += 1
		elif key == "backspace":
			if self.cursorpos>0:
				self.text = self.text[:self.cursorpos-1]+self.text[self.cursorpos:]
				self.cursorpos -= 1
		elif key == "delete":
			if self.cursorpos<len(self.text):
				self.text = self.text[:self.cursorpos]+self.text[self.cursorpos+1:]
		elif key == "left_arrow":
			if self.cursorpos>0:
				self.cursorpos -= 1
		elif key == "right_arrow":
			if self.cursorpos<len(self.text):
				self.cursorpos += 1
		elif key == "up_arrow":
			if '\n' in self.text[:self.cursorpos]:
				lpos = self.text[:self.cursorpos].rindex('\n', 0, self.cursorpos)
				if '\n' in self.text[:lpos]:
					llpos = self.text[:lpos].rindex('\n',0,lpos)
				else:
					llpos = -1 # to account for no \n preceding it
				self.cursorpos = min(self.cursorpos-lpos+llpos,lpos)
		elif key == "down_arrow":
			if '\n' in self.text[self.cursorpos:]:
				if '\n' in self.text[:self.cursorpos]:
					lpos = self.text[:self.cursorpos].rindex('\n', 0, self.cursorpos)
				else:
					lpos = -1
				npos = self.text[self.cursorpos:].index('\n')
				if '\n' in self.text[self.cursorpos+npos+1:]:
					nnpos = self.text[self.cursorpos+npos+1:].index('\n')
				else:
					nnpos = len(self.text[self.cursorpos:])
				self.cursorpos = min(self.cursorpos+npos+self.cursorpos-lpos,self.cursorpos+npos+nnpos+1)
		else:
			self.text=self.text[:self.cursorpos]+str(key)+self.text[self.cursorpos:]
			self.cursorpos += 1
		if not key=="enter":
			if len(undo_stack)>0:
				if isinstance(undo_stack[-1], maybe):
					if undo_stack[-1].edit.obj==self:
						undo_stack[-1].edit.to_attrs={"text":self.text, "cursorpos":self.cursorpos}
					else:
						undo_stack.append(maybe("text", self, {"text":self.text, "cursorpos":self.cursorpos}))
				else:
					undo_stack.append(maybe("text", self, {"text":self.text, "cursorpos":self.cursorpos}))
			else:
				undo_stack.append(maybe("text", self, {"text":self.text, "cursorpos":self.cursorpos}))
		else:
			undo_stack[-1] = undo_stack[-1].complete({"text":self.text, "cursorpos":self.cursorpos})
			clear(redo_stack)
		pass
	def onKeyUp(self, self1, key):
		pass
	def print_sc(self):
		retval = ".font "+''.join(self.font.family.split(' '))+self.name+" filename=\""\
			+FONT_PATH+self.font.family+".ttf\"\n"
		if self.dynamic:
			retval+=".edittext "+self.name+" width="+str(self.width)+" height="+str(self.height)\
				+" font="+''.join(self.font.family.split(' '))+self.name+" text=\""+self.text\
				+"\" color="+self.fill.rgb+" size="+str(self.font.size)+"pt"
			if self.variable:
				retval+=" variable="+self.variable
			if self.password:
				retval+=" password"
			if self.wordwrap:
				retval+=" wordwrap"
			if self.multiline:
				retval+=" multiline"
			if self.html:
				retval+=" html"
			if self.border:
				retval+=" border"
			if self.editable:
				retval+="\n"
			if not self.selectable:
				retval+=" noselect"
			else:
				retval+=" readonly\n"
		else:
			retval+=".text "+self.name+" font="+''.join(self.font.family.split(' '))+self.name\
			+" text=\""+self.text+"\" color="+self.fill.rgb+" size="+str(self.font.size)\
			+"pt\n"
		return retval
	def print_html(self):
		retval = "var "+self.name+" = new TextField();\n"+self.name+".text = \""+self.text\
			+"\";\n"+self.name+".hwaccel = "+str(self.hwaccel).lower()+"\n"
		return retval

class Sound:
	"""Class for storing sounds in."""
	def __init__(self, data, name, path, info):
		global Library
		Library.append(self)
		self.data = data
		self.name = name.replace(" ", "_")
		self.x = 0
		self.y = 0
		self.rotation = 0
		self.xscale = 0
		self.yscale = 0
		self.path = path
		self.iname = None
		reading_comments_flag = False
		other = ''
		for l in info.splitlines():
			if( not l.strip() ):
				continue
			if( reading_comments_flag and l.strip() ):
				if( comments ):
					comments += '\n'
				comments += l
			else:
				if( l.startswith('Input File') ):
					input_file = l.split(':',1)[1].strip()[1:-1]
				elif( l.startswith('Channels') ):
					num_channels = int(l.split(':',1)[1].strip())
				elif( l.startswith('Sample Rate') ):
					sample_rate = int(l.split(':',1)[1].strip())
				elif( l.startswith('Precision') ):
					bits_per_sample = int(l.split(':',1)[1].strip()[0:-4])
				elif( l.startswith('Duration') ):
					tmp = l.split(':',1)[1].strip()
					tmp = tmp.split('=',1)
					duration_time = tmp[0]
					duration_samples = int(tmp[1].split(None,1)[0])
				elif( l.startswith('Sample Encoding') ):
					encoding = l.split(':',1)[1].strip()
				elif( l.startswith('Comments') ):
					comments = ''
					reading_comments_flag = True
				else:
					if( other ):
						other += '\n'+l
					else:
						other = l
					print >>sys.stderr, "Unhandled:",l
		self.sample_rate = int(sample_rate)
		self.duration_samples = int(duration_samples)
		self.duration_time = int(duration_time.split(':')[0])*3600+int(duration_time.split(':')[1])*60+float(duration_time.split(':')[2])
	def draw(self, cr, transform):
		pass
	def draw_frame(self, cr, transform):
		if SYSTEM=="osx":
			cr.newpath()
			# cr.moveto(0,16)
			chunk_size = int(self.duration_samples/(self.duration_time*FRAMERATE*16))
			print chunk_size
			print self.duration_samples/chunk_size
			for i in xrange(self.duration_samples/chunk_size):
				j = abs(NP.amax(self.data[i*chunk_size:(i+1)*chunk_size])/65536.0)
				cr.moveto(i,16-j*16)
				cr.lineto(i,16+j*16)
			# cr.lineto(self.duration_time*16*FRAMERATE,16)
			cr.stroke()
	def hitTest(self, x, y):
		return False
	def print_sc(self):
		retval = ".sound "+self.name+" \""+self.path+"\"\n"
		return retval
	def print_html(self):
		retval = "var "+self.name.replace(".","_")+" = new Sound();\n"+self.name.replace(".","_")+"._sound = new Audio('"+self.path.split("/")[-1]+"');\n"
		retval = retval + self.name.replace(".","_")+"._sound.load();\n"+self.name.replace(".","_")+".duration = "+self.name.replace(".","_")+"._sound.duration\n"
		return retval

class framewrapper (object):
	#def __getstate__(self):
	#	dict = self.__dict__.copy()
	#	dict['parent'] = None
	#	return dict
	#Wraps object per-frame. Allows for changes in position, rotation, scale.
	def __init__(self, obj, x, y, rot, scalex, scaley, parent=None):
		self.obj = obj
		self.x = obj.x = x
		self.y = obj.y = y
		self.rot = obj.rot = rot
		self.scalex = self.xscale = obj.scalex = scalex
		self.scaley = self.yscale = obj.scaley = scaley
		self.level = False # don't try to descend into a framewrapper
		self.type = obj.__class__.__name__
		if obj.__class__.__name__=="Shape":
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
				self.obj.xscale = self.xscale
				self.obj.yscale = self.yscale
				if self.type=="Shape":
					self.obj.filled = self.filled
					self.obj.linecolor = self.linecolor
					self.obj.fillcolor = self.fillcolor
	def _onMouseDown(self, x, y, button=1, clicks=1):
				self.obj.onMouseDown(self,x, y, button, clicks)
	def _onMouseUp(self, x, y, button=1, clicks=1):
				self.obj.onMouseUp(self,x, y, button)
	def _onMouseMove(self, x, y, button=1):
				self.obj.onMouseMove(self, x, y, button)
	def _onMouseDrag(self, x, y, button=1):
				self.obj.onMouseDrag(self, x, y, button)
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
	def __reduce__(self):
		badvars = (self.parent,self.group)
		self.parent = None
		ret = pickle.dumps(self)
		self.parent, self.group = badvars
		return ret
	def __init__(self,parent,duplicate=None):
		self.objs = []
		self.currentselect=None
		self.type="Group"
		self.parent = parent
		self.actions = ''
	def add(self, obj, x, y, rot=0, scalex=1, scaley=1):
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
				self.group = group
				if USING_GL:
					#cr.gsave()
					#cr.rotate(group.rotation)
					#cr.translate(group.x,group.y)
					#cr.scale(group.xscale,group.yscale)
					
					def dodraw(obj, cr):
						obj.draw(cr, self)
					result = [dodraw(obj, cr) for obj in self.objs]
					if currentselect:
						cr.save()
						
						glColor3f(0,0,1)
						
						glBegin(GL_LINES)
						glVertex2f(currentselect.minx-1,cr.height-(currentselect.miny-1))
						glVertex2f(currentselect.maxx+currentselect.x+2, cr.height-(currentselect.miny-1))
						glVertex2f(currentselect.maxx+currentselect.x+2, cr.height-(currentselect.miny-1))
						glVertex2f(currentselect.maxx+currentselect.x+2, cr.height-(currentselect.maxy+currentselect.y+2))
						glVertex2f(currentselect.maxx+currentselect.x+2, cr.height-(currentselect.maxy+currentselect.y+2))
						glVertex2f(currentselect.minx-1,cr.height-(currentselect.maxy+currentselect.y+2))
						glVertex2f(currentselect.minx-1,cr.height-(currentselect.maxy+currentselect.y+2))
						glVertex2f(currentselect.minx-1,cr.height-(currentselect.miny-1))
						glEnd()
						
						cr.restore()
						print "selected", currentselect
					# cr.restore()
				else:
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
						if MODE=="s":
							cr.newpath()
							cr.pencolor = Colors.rgb(1,1,1)
							cr.fillcolor = Colors.rgb(0,0,0)
							cr.rect([currentselect.minx-5,currentselect.miny-5,
									 currentselect.minx+5,currentselect.miny+5])
							cr.rect([currentselect.maxx+currentselect.x-5,currentselect.miny-5,
									 currentselect.maxx+currentselect.x+5,currentselect.miny+5])
							cr.rect([currentselect.maxx+currentselect.x-5,currentselect.maxy+currentselect.y-5,
									 currentselect.maxx+currentselect.x+5,currentselect.maxy+currentselect.y+5])
							cr.rect([currentselect.minx-5,currentselect.maxy+currentselect.y-5,
									 currentselect.minx+5,currentselect.maxy+currentselect.y+5])
							cr.fill_stroke()
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
					if isinstance(i.obj, Sound):
						retval = retval+".play "+i.obj.name+"\n"
					elif i.obj.iname:
						retval = retval+".put "+i.obj.iname+"="+i.name+" x="+str(i.x)+" y="+str(i.y)+" scalex="+str(i.xscale*100)+" scaley="+str(i.yscale*100)+"\n"
					else:
						retval = retval+".put "+i.name+" x="+str(i.x)+" y="+str(i.y)+" scalex="+str(i.xscale*100)+"% scaley="+str(i.yscale*100)+"%\n"
			else:
				for i in self.objs:
					if isinstance(i.obj, Sound):
							retval = retval+".play "+i.obj.name+"\n"
					elif not i.obj in [j.obj for j in misc_funcs.lastval(self.parent.frames,self.parent.frames.index(self)).objs]:
						if not hasattr(i.obj, "iname"):
							i.obj.iname = None
						if i.obj.iname:
							retval = retval+".put "+i.obj.iname+"="+i.name+" x="+str(i.x)+" y="+str(i.y)+" scalex="+str(i.xscale*100)+"% scaley="+str(i.yscale*100)+"%\n"
						else:
							retval = retval+".put "+i.name+" x="+str(i.x)+" y="+str(i.y)+"scalex="+str(i.xscale*100)+"% scaley="+str(i.yscale*100)+"%\n"
					else:
						retval = retval+".move "+i.name+" x="+str(i.x)+" y="+str(i.y)+"\n"
						retval = retval+".change "+i.name+" scalex="+str(i.xscale*100)+"% scaley="+str(i.yscale*100)+"%\n"
			if not self.actions.strip()=='':
				retval = retval + ".action:\n"+self.actions+"\n.end\n"
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
		return max([i.maxx+i.x for i in self.currentFrame()])
	def getmaxy(self):
		return max([i.maxy+i.y for i in self.currentFrame()])
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseMove(self, self1, x, y, button=1):
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
			self.frames[self.currentframe].add(obj, obj.x, obj.y, obj.rotation,1,1)
			self.objs.append(obj)
		[parse_obj(obj) for obj in args]
	def delete(self,*args):
		for i in args:
			print "#>>",i
			for j in self.frames[self.currentframe].objs:
				if j == i:
					del self.currentFrame()[self.currentFrame().index(j)]
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
	def _onMouseDown(self, x, y, button=1, clicks=1):
		if self.level:
			if self.currentselect and self.currentselect.level:
				self.currentselect._onMouseDown(self.currentselect, x, y, button=button, clicks=clicks)
			else:
				if MODE in [" ", "s", "b"]:
					if self.currentselect and MODE=="s":
						if self.currentselect.minx-5<x<self.currentselect.minx+5:
							print "hey!"
					else:
						for i in reversed(self.currentFrame()):
							test = False
							if i.hitTest(x, y):
								if MODE in [" ", "s"]:
									self.currentselect = i
								i._onMouseDown(x, y, button=button, clicks=clicks)
								test=True
								break
						if not test:
							self.currentselect = None
				else:
					self.onMouseDown(self, x, y, button=button, clicks=clicks)
		else:
			self.onMouseDown(self, x, y, button=button, clicks=clicks)
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass
	def _onMouseUp(self,x,y, button=1, clicks=1):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onMouseUp(x, y, button=1)
		else:
			self.onMouseUp(self, x, y, button=button)
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def _onMouseMove(self,x,y, button=1):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onMouseMove(x, y, button=button)
		else:
			self.onMouseMove(self, x, y)
	def onMouseMove(self, self1, x, y, button=1):
		pass
	def _onMouseDrag(self, x, y, button=1):
		if self.level and MODE in [" ", "s"]:
			if self.currentselect:
				self.currentselect._onMouseDrag(x, y, button=button)
		else:
			self.onMouseDrag(self, x, y, button=button)
	def onMouseDrag(self, self1, x, y, button=1):
		pass
	def _onKeyDown(self, key):
		if self.level and MODE in [" ", "s", "t"]:
			if self.currentselect:
				self.currentselect._onKeyDown(key)
		else:
			self.onKeyDown(self, key)
	def onKeyDown(self, self1, key):
		pass
	def _onKeyUp(self, key):
		if self.level and MODE in [" ", "s", "t"]:
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
					retval+=".sprite "+i.name+"\n"+i.print_sc()+".end\n"
				elif i.type=="Shape":
					retval+=".outline "+i.name+"outline:\n"
					retval+=" ".join([" ".join([str(x) for x in a]) for a in i.shapedata])+"\n.end\n"
					if i.filled:
						retval+=".filled "+i.name+" outline="+i.name+"outline fill="+i.fillcolor.rgb+" color="+i.linecolor.rgb+"\n"
					else:
						retval+=".filled "+i.name+" outline="+i.name+"outline fill=#00000000 color="+i.linecolor.rgb+"\n"
				elif i.type=="Image":
					retval+=".png "+i.name+" \""+i.path+"\"\n"
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
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseMove(self, self1, x, y, button=1, clicks=1):
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
	def getobjs(self):
		return [item for sublist in self.layers for item in sublist.objs]
	minx = property(getminx)
	miny = property(getminy)
	maxx = property(getmaxx)
	maxy = property(getmaxy)
	activelayer = property(getal,setal)
	activeframe = property(getactiveframe, setactiveframe)
	currentframe = property(getcurrentframe, setcurrentframe)
	level = property(getlevel, setlevel)
	scale = property(fset = setscale)
	objs = property(getobjs)
	def __init__(self, *args, **kwargs):
		if not 'skipl' in kwargs:
			global Library
			Library.append(self)
		self.layers = [Layer(*args)]
		self._al = 0
		self.clicked = False
		self.x = 0
		self.y = 0
		self.rotation = 0
		self.xscale = 1
		self.yscale = 1
		self.type = "Group"
		self.startx = 0
		self.starty = 0
		self.cx = 0
		self.cy = 0
		self.dragging = False
		self.selecting = False
		self.tempgroup = None
		self.is_mc = False
		self.name = "g"+str(int(random.random()*10000))+str(SITER)
		if "onload" in kwargs:
			kwargs["onload"](self)
	def draw(self,cr=None,transform=None,rect=None):
		if self.is_mc and self.level:
			cr.fillcolor = Color([1,1,1,0.5]).pygui
			cr.fill_rect([0,0,1000,1000])
		for i in self.layers:
			if not i.hidden:
				i.x = self.x
				i.y = self.y
				i.rotation = self.rotation
				i.xscale = self.xscale
				i.yscale = self.yscale
				i.draw(cr,rect=rect)
		if self.dragging and self.selecting and MODE in (" ", "s"):
			if SYSTEM=="osx":
				cr.newpath()
				cr.pencolor = Color([0,0,1]).pygui
				cr.stroke_rect([sorted([self.startx,self.cx])[0], sorted([self.starty,self.cy])[0], \
								sorted([self.startx,self.cx])[1], sorted([self.starty,self.cy])[1]])
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
	def _onMouseDown(self, x, y, button=1, clicks=1):
		x, y = self.localtransform(x, y)
		if self.level:
			if self.activelayer.currentselect and self.activelayer.currentselect.level:
				self.activelayer.currentselect._onMouseDown(self.activelayer.currentselect, x, y)
			else:
				if MODE in [" ", "s", "b"]:
					if CURRENTTEXT:
						CURRENTTEXT.editing = False
					if self.activelayer.currentselect and MODE=="s":
						global SCALING
						if (self.activelayer.currentselect.minx-5<x<self.activelayer.currentselect.minx+5 and \
								self.activelayer.currentselect.miny-5<y<self.activelayer.currentselect.miny+5) or \
						(self.activelayer.currentselect.minx-5<x<self.activelayer.currentselect.minx+5 and \
								self.activelayer.currentselect.miny+self.activelayer.currentselect.maxy-5<y<self.activelayer.currentselect.miny+self.activelayer.currentselect.maxy+5) or \
						(self.activelayer.currentselect.minx+self.activelayer.currentselect.maxx-5<x<self.activelayer.currentselect.minx+self.activelayer.currentselect.maxx+5 and \
								self.activelayer.currentselect.miny+self.activelayer.currentselect.maxy-5<y<self.activelayer.currentselect.miny+self.activelayer.currentselect.maxy+5) or \
						(self.activelayer.currentselect.minx+self.activelayer.currentselect.maxx-5<x<self.activelayer.currentselect.minx+self.activelayer.currentselect.maxx+5 and \
								self.activelayer.currentselect.miny-5<y<self.activelayer.currentselect.miny+5):
							SCALING = True
							self.activelayer.currentselect._onMouseDown(x, y, button=button, clicks=clicks)
					else:
						test = False
						for i in reversed(self.currentFrame()):
							if i.hitTest(x, y):
								if MODE in [" ", "s"]:
									self.activelayer.currentselect = i
									test=True
								print 'onmousedowning'
								i._onMouseDown(x, y, button=button, clicks=clicks)
								break
						if not test:
							if self.tempgroup:
								del self.currentFrame()[[i.obj for i in self.currentFrame()].index(self.tempgroup)]
								[self.currentFrame().append(i) for i in self.tempgroup.split()]
								self.tempgroup = None
							self.activelayer.currentselect = None
							self.startx, self.starty = x, y
							self.selecting = True
				else:
					self.onMouseDown(self, x, y, button=button, clicks=clicks)
		else:
			print "HEYY"
			self.onMouseDown(self, x, y, button=button, clicks=clicks)
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		pass 
	def _onMouseUp(self,x,y, button=1, clicks=1):
		global SCALING
		SCALING = False
		self.dragging = False
		self.selecting = False
		x, y = self.localtransform(x, y)
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onMouseUp(x, y, button=button, clicks=clicks)
			elif abs(self.startx-x)>4 or abs(self.starty-y)>4:
				objs = []
				for i in reversed(self.currentFrame()):
					if self.startx<i.x+i.minx<x or self.startx<i.x+i.maxx<x:
						if self.starty<i.y+i.miny<y or self.starty<i.y+i.maxy<y:
							objs.append(i)
							del self.currentFrame()[self.currentFrame().index(i)]
				if objs:
					tgroup = TemporaryGroup(skipl=True)
					[tgroup.add(i.obj) for i in reversed(objs)]
					self.add(tgroup)
					self.activelayer.currentselect = tgroup
					self.tempgroup = tgroup
					print [i.obj for i in self.currentFrame()]
		else:
			self.onMouseUp(self, x, y, button=button, clicks=clicks)
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	def _onMouseMove(self,x,y,button=1,clicks=1):
		x, y = self.localtransform(x, y)
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onMouseMove(x, y, button=button)
		else:
			self.onMouseMove(self, x, y, button=button)
	def onMouseMove(self, self1, x, y, button=1, clicks=1):
		pass
	def _onMouseDrag(self, x, y, button=1, clicks=1):
		x, y = self.localtransform(x, y)
		self.cx, self.cy = x, y
		self.dragging = True
		if self.activelayer.level and MODE in [" ", "s"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onMouseDrag(x, y, button=button)
		else:
			self.onMouseDrag(self, x, y, button=button)
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		pass
	def _onKeyDown(self, key):
		if self.activelayer.level and MODE in [" ", "s", "t"]:
			if self.activelayer.currentselect:
				self.activelayer.currentselect._onKeyDown(key)
		else:
			self.onKeyDown(self, key)
	def onKeyDown(self, self1, key):
		pass
	def _onKeyUp(self, key):
		if self.activelayer.level and MODE in [" ", "s", "t"]:
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
	def hitTest(self,x,y):
		for i in self.layers:
			for j in i.frames[0].objs:
				if j.hitTest(x, y):
					return True
		return False
	def print_sc(self):
		retval = ""
		#for i in self.layers:
		#	retval+=i.print_sc(True, False)
		if not self.name=="_root":
			retval+=".sprite "+self.name+"\n"
		for i in xrange(self.maxframe()):
			for j in self.layers:
				if j.frames[i]:
					retval+=".frame "+str(i+1)+"\n"
					retval+=j.frames[i].print_sc()
		if not self.name=="_root":
			retval+=".end\n"
		return retval
	def print_html(self):
		retval = ""
		if not self.name=="_root":
			retval = retval + "var "+self.name+" = new MovieClip();\n"
		for i in self.layers:
			pass
			#retval+=i.print_html(True,False)
		'''for i in xrange(self.maxframe()):
			for j in self.layers:
				if j.frames[i]:
					retval+=".frame "+str(i+1)+"\n"
					retval+=j.frames[i].print_html()'''
		print self.objs
		for i in self.objs:
			retval += self.name+"."+i.name.replace(".","_")+" = "+i.name.replace(".","_")+";\n"
		for i in range(len(self.layers)):
			for j in xrange(self.maxframe()):
				if self.layers[i].frames[j]:
					retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"] = new Frame ();\n"
					for k in self.layers[i].frames[j].objs:
						# if isinstance(k.obj, Sound):
						# 	retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.obj.name.replace(".","_")+".start();\n"
						# 	# retval += self.name+"."+k.obj.name.replace(".","_")+".start();\n"
						# else:
							retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.name.replace('.',"_")+" = {};\n"
							retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.name.replace('.',"_")+"._x = "+str(k.x)+";\n"
							retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.name.replace('.',"_")+"._y = "+str(k.y)+";\n"
							retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.name.replace('.',"_")+"._rotation = "+str(k.rot)+";\n"
							retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.name.replace('.',"_")+"._xscale = "+str(k.xscale)+";\n"
							retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"]."+k.name.replace('.',"_")+"._yscale = "+str(k.yscale)+";\n"
					retval += self.name+"._layers["+str(i)+"]._frames["+str(j)+"].actions = \""+self.layers[i].frames[j].actions.replace("\n"," ").replace("\\","\\\\").replace("\"","\\\"")+"\"\n"
		return retval

class TemporaryGroup(Group):
	"""Created when selecting multiple items, for ease of use."""
	def __init__(self, *args, **kwargs):
		super(TemporaryGroup, self).__init__(*args, **kwargs)
	# def draw(self, cr=None, transform=None, rect=None):
	# 	super(TemporaryGroup, self).draw(cr, transform, rect)
	# 	print self.x, self.activelayer.x
	# 	pass
	def split(self):
		for i in self.currentFrame():
			i.x = i.x+self.x
			i.y = i.y+self.y
		return self.currentFrame()
		pass
	def onMouseDown(self, self1, x, y, button=1, clicks=1):
		print "Hihihihihi"
		if self1.hitTest(x, y):
			self1.clicked = True
			self1.initx,self1.inity = x-self1.x, y-self1.y
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		if MODE==" ":
			self1.x = x-self1.initx
			self1.y = y-self1.inity
		elif MODE=="s":
			if SCALING:
				# Not working yet.
				if self1.initx>self1.maxx/2:
					self1.xscale = (x-self1.x)/self1.maxx
				else:
					self1.xscale = (2*self1.maxx+self1.x-(x-self1.initx)-x)/self1.maxx
					self1.x = x
				if self1.inity>self1.maxy/2:
					self1.yscale = (y-self1.y)/self1.maxy
				else:
					# 3 times?? Why??
					self1.yscale = (3*self1.maxy+self1.y-(y-self1.inity)-y)/self1.maxy
					self1.y = y
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		self.clicked = False
	def onKeyDown(self, self1, key):
		if key in ("delete", "backspace"):
			del self1.parent[self1.parent.index(self1)] # Need to clean up deletion
		elif key in [" ", "s", "r", "e", "b", "p"]:
			svlgui.MODE=key
			svlgui.set_cursor({" ":"arrow","s":"arrow","r":"crosshair","e":"crosshair",
					"b":"arrow","p":"arrow"}[key], MainWindow.stage)
			misc_funcs.update_tooloptions()
		elif key=="F6":
			add_keyframe()
		elif key=="F8":
			convert_to_symbol()
		elif key=="left_arrow":
			self1.x-=1
		elif key=="right_arrow":
			self1.x+=1
		elif key=="up_arrow":
			self1.y-=1
		elif key=="down_arrow":
			self1.y+=1
		

def set_cursor(curs, widget=None):
	if SYSTEM == "osx":
		cursdict = {"arrow":StdCursors.arrow, "ibeam":StdCursors.ibeam, 
			"crosshair":StdCursors.crosshair, "fist":StdCursors.fist,
			"hand":StdCursors.hand, "finger":StdCursors.finger, 
			"invisible":StdCursors.invisible, "text":StdCursors.ibeam}
		if curs in cursdict:
			if widget:
				widget._int().cursor = cursdict[curs]
			else:
				app.cursor = cursdict[curs]
		else:
			print "Sorry, I don't have that cursor."

def alert(text,critical=False,confirm=False,async=False):
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
			if confirm:
				return Alerts.confirm(text)
			else:
				if async:
					a = ModalDialog()
					a.place(OSXLabel(text=text),left=0,top=0,right=0,bottom=0,sticky="nesw")
					a.show()
					a.present()
					return a
				else:
					Alerts.note_alert(text)			# note_alert is a general alert, i.e. "I'm a computer."
	elif SYSTEM=="html":
		jscommunicate("alert("+text+")")
		if critical:
			# reloading the page is equivalent to force-quitting, right?
			jscommunicate("window.location.reload()")

def file_dialog(mode="open",default=None,types=None,multiple=False,name=None):
	if SYSTEM=="osx":
		if types:
			ntypes = []
			mactypes = {"txt":'TEXT', "pdf":'PDF ',"mov":'MooV',"mpg":'MPG ',"mp2":'MPG2',
		"mp4":'M4V ',"m4v":'M4V ',"mp3":'Mp3 ',"gif":'GIFf',"png":'PNGf',"jpg":'JPEG',
		"bmp":'BMPf',"tiff":'TIFF',"psd":'8BPS',"mid":'Midi',"rtf":'RTF ',"wav":'WAVE',
		"aif":'AIFF',"ttf":'tfil',"swf":'SWFL'}
			for i in types:
				ntypes.append(GUI.Files.FileType())
				ntypes[-1].suffix = i
				if i in mactypes:
					ntypes[-1].mac_type=mactypes[i]
			types = ntypes
		if mode=="open":
			if multiple:
				return FileDialogs.request_old_files(default_dir=default,file_types=types)
			else:
				return FileDialogs.request_old_file(default_dir=default,file_types=types)
		elif mode=="save":
			return FileDialogs.request_new_file(default_name=name,default_dir=default,file_type=types)

def execute(command):
	rv = os.system(command.replace("/",sep))
	if PLATFORM == "osx":
		if rv==0:
			return True
		else:
			return False

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
			image = media_path+"media/logo-transparent.png"
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
	def __init__(self,var,dispgrp=None, dcanv=None):
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
		elif SYSTEM=="osx":
			win = ModalDialog(width=336,height=208,resizable=False)
			def onClickRectFill(self,x,y,button=None,clicks=None):
				global FILLCOLOR
				FILLCOLOR = Color(colors.colorArray(int(x/16))[int(y/16)])
				if root.descendItem().activelayer.currentselect:
					if not (root.descendItem().activelayer.currentselect.fillcolor.val == FILLCOLOR.val and root.descendItem().activelayer.currentselect.filled==True):
						undo_stack.append(edit("fill", root.descendItem().activelayer.currentselect, \
							{"filled":root.descendItem().activelayer.currentselect.filled, \
									"fillcolor":root.descendItem().activelayer.currentselect.fillcolor}, 
							{"filled":True, "fillcolor":svlgui.FILLCOLOR}))
						clear(redo_stack)
					root.descendItem().activelayer.currentselect.fillcolor = FILLCOLOR
					root.descendItem().activelayer.currentselect.filled = True
					root.descendItem().activelayer.currentselect.update()
				self.window.dismiss()
				raise ObjectDeletedError
			def onClickRectLine(self,x,y,button=None,clicks=None):
				global LINECOLOR
				LINECOLOR = Color(colors.colorArray(int(x/16))[int(y/16)])
				if root.descendItem().activelayer.currentselect:
					root.descendItem().activelayer.currentselect.linecolor = LINECOLOR
					root.descendItem().activelayer.currentselect.update()
				self.window.dismiss()
				raise ObjectDeletedError
			canvas = Canvas(336,208)
			canvas._int().scrolling = ''
			group = Group(skipl=True)
			def dummy(*args):
				pass
			group._onMouseMove = dummy
			canvas.add(group,0,0)
			im = Image(media_path+"media/colors.png",skipl=True)
			group.add(im)
			group.window = win
			group.canvas = canvas
			if var=="fill":
				group.onMouseDown = onClickRectFill
			if var=="line":
				group.onMouseDown = onClickRectLine
			win.place(canvas._int(),left=0,top=0,right=0,bottom=0,sticky="news",scrolling="")
			win.present()
			
class PreferencesWindow:
	def __init__(self):
		if SYSTEM=="osx":
			win = ModalDialog(closable=True,width=500,height=500)
			self.win.title = "Preferences"
			frame = Frame()
			win.place(frame._int(), left=0, top=0, right=0, bottom=0, sticky="nsew")
			label = Label("Path to Flash Debugger: ")
			frame.layout_self([label,0,None,0,None,"nw",""])
			win.present()

class SizeWindow:
	def __init__(self):
		if SYSTEM=="osx":
			self.width = WIDTH
			self.height = HEIGHT
			self.win = ModalDialog(closable=True,width=160,height=70)
			self.win.title = "Dimensions"
			frame = Frame()
			self.win.place(frame._int(), left=0, top=0, right=0, bottom=0, sticky="nsew")
			wlabel = Label("Width: ")
			hlabel = Label("Height: ")
			self.wentry = TextEntry(str(WIDTH))
			self.hentry = TextEntry(str(HEIGHT))
			b1 = DefaultButton()
			b1.action = self.set_size
			b2 = CancelButton()
			b2.action = self.restore_size
			frame.layout_self(	[wlabel,0,None,0,None,"nw",""],
								[self.wentry,wlabel._int(),None,0,None,"nw",""],
								[hlabel,0,None,self.wentry._int(),None,"nw",""],
								[self.hentry,hlabel._int(),None,self.wentry._int(),None,"nw",""],
								[Widget(b2),0,None,None,0,'nw',''],
								[Widget(b1),None,0,None,0,'nw',''])
			self.win.present()
	def set_size(self):
		global WIDTH, HEIGHT
		WIDTH = int(self.wentry.text)
		HEIGHT = int(self.hentry.text)
		self.win.ok()
	def restore_size(self):
		global WIDTH, HEIGHT
		WIDTH, HEIGHT = self.width, self.height
		self.win.cancel()
		
class PublishSettingsWindow:
	def __init__(self):
		if SYSTEM=="osx":
			self.win = ModalDialog(closable=True,width=400,height=300)
			self.win.title = "Publish Settings"
			frame = Frame()
			self.win.place(frame._int(), left=0, top=0, right=0, bottom=0, sticky="nsew")
			plabel = Label("Settings-publish")
			elabel = Label("Export: ")
			self.c1 = OSXCheckBox("SWF")
			self.c2 = OSXCheckBox("HTML5")
			self.c3 = OSXCheckBox("Base HTML file")
			self.c3.action = self.deactivate4
			self.c4 = OSXCheckBox("Setup fallback content")
			self.c4.action = self.activate3
			swlabel = Label("SWF:")
			htlabel = Label("HTML5:")
			self.impack = OSXCheckBox("Pack Images (Not implemented yet!)")
			self.impack.action = self.activate3
			b1 = DefaultButton()
			b1.action = self.confirm
			b2 = CancelButton()
			frame.layout_self(	[plabel,5,None,5,None,"nw",""],
								[elabel,5,None,plabel._int(),None,"nw",""],
								[Widget(self.c1),16,None,elabel._int(),None,"nw",""],
								[Widget(self.c2),self.c1+16,None,elabel._int(),None,"nw",""],
								[Widget(self.c3),self.c2+16,None,elabel._int(),None,"nw",""],
								[Widget(self.c4),self.c2+32,None,self.c3,None,"nw",""],
								[swlabel, 5, None, self.c4, None, "nw", ""],
								[htlabel, 5, None, swlabel._int(), None, "nw", ""],
								[Widget(self.impack), 16, None, htlabel._int(), None, "nw", ""],
								[Widget(b2),5,None,None,-5,'nw',''],
								[Widget(b1),None,-5,None,-5,'nw',''])
			self.win.present()
	def activate2(self):
		self.c2.set_value(self.c2.value or self.impack.value)
	def activate3(self):
		self.c3.set_value(self.c3.value or self.c4.value)
	def deactivate4(self):
		if self.c3.value==False:
			self.c4.set_value(False)
	def confirm(self):
		global EXPORT_OPTIONS
		EXPORT_OPTIONS = {"swf":self.c1.value, "html5":self.c2.value, "basehtml":self.c3.value,
							"fallback":self.c4.value,"pack":self.impack.value}
		self.win.ok()

class ConvertToSymbolWindow:
	def __init__(self,root,onMouseDown):
		self.root = root
		self.onMouseDown = onMouseDown
		if SYSTEM=="osx":
			self.win = ModalDialog(closable=True,width=400,height=150)
			self.win.title = "Convert to symbol"
			frame = Frame()
			self.win.place(frame._int(), left=0, top=0, right=0, bottom=0, sticky="nsew")
			nlabel = Label("Name: ")
			self.ntry = TextEntry("Symbol 1")	#TODO: dynamically generate this
			self.ntry.set_action(self.confirm)
			tlabel = Label("Type: ")
			tgroup = RadioGroup("Movie Clip", "Button", "Group")
			b1 = DefaultButton()
			b1.action = self.confirm
			b2 = CancelButton()
			frame.layout_self(	[nlabel,5,None,5,None,"nw",""],
								[self.ntry, nlabel._int()+5,-5,5,None,'new','h'],
								[tlabel,5,None,self.ntry._int(),None,'nw',''],
								[tgroup[0],32,None,tlabel._int(),None,'nw',''],
								[tgroup[1],32,None,tgroup[0]._int(),None,'nw',''],
								[tgroup[2],32,None,tgroup[1]._int(),None,'nw',''],
								[Widget(b2),5,None,None,-5,'nw',''],
								[Widget(b1),None,-5,None,-5,'nw',''])
			self.win.present()
	def settype(self,tgroup):
		self.type = tgroup.value
	def confirm(self):
		symbol = Group()
		symbol.add(self.root.descendItem().activelayer.currentselect.obj)
		symbol.name = self.ntry.text
		symbol.is_mc = True
		self.root.descendItem().activelayer.delete(self.root.descendItem().activelayer.currentselect)
		print self.root.descendItem().activelayer.currentFrame()
		self.root.descendItem().activelayer.add(symbol)
		symbol.onMouseDown = self.onMouseDown
		self.win.ok()

class FramesCanvas(Canvas):
	def __init__(self,w,h):
		Canvas.__init__(self,w,h)
		self.pointer = 1
		self.x = None
		if SYSTEM == 'osx':
			self.canvas.draw = self._draw
			self.canvas.mouse_down = self.mouse_down
			self.canvas.mouse_drag = self.mouse_drag
			self.canvas.mouse_up = self.mouse_up
			self.ackfr = GUI.Image(file = media_path+"media/keyframe_active.png")
			self.inackfr = GUI.Image(file = media_path+"media/keyframe_inactive.png")
			self.acfr = GUI.Image(file = media_path+"media/frame_active_tween.png")
			self.inacfr = GUI.Image(file = media_path+"media/frame_inactive_tween.png")
	def _draw(self,cr,update_rect):
		try:
			for k in xrange(len(self.root.descendItem().layers)):
				FRAMES = self.root.descendItem().layers[k].frames
				for i in xrange(len(FRAMES)):
					cr.gsave()
					#cr.translate(i*16,k*32)
					if FRAMES[i]:
						if self.root.descendItem().currentframe == i:
							src_rect = self.ackfr.bounds
							src_rect = [0,0,(16)*(self.pointer%17),32]
							dst_rect = [i*16, k*32, 16+i*16, 32+k*32]
							# print dst_rect
							self.ackfr.draw(cr, src_rect, dst_rect)
						else:
							src_rect = self.inackfr.bounds
							dst_rect = [i*16, k*32, 16+i*16, 32+k*32]
							self.inackfr.draw(cr, src_rect, dst_rect)
					else:
						if self.root.descendItem() == i:
							src_rect = self.acfr.bounds
							dst_rect = [i*16, k*32, 16+i*16, 32+k*32]
							self.acfr.draw(cr, src_rect, dst_rect)
						else:
							src_rect = self.inacfr.bounds
							dst_rect = [i*16, k*32, 16+i*16, 32+k*32]
							self.inacfr.draw(cr, src_rect, dst_rect)
					cr.grestore()
				for i in xrange(len(FRAMES)):
					if FRAMES[i]:
						try:
							cr.gsave()
							cr.translate(i*16,0)
							sounds = [i.obj for i in FRAMES[i].objs if isinstance(i.obj, Sound)]
							[i.draw_frame(cr, None) for i in sounds]
							cr.grestore()
						except:
							traceback.print_exc()
				# print max(len(FRAMES),int(update_rect[0]/16-1)),int(update_rect[2]/16+1)
				for i in xrange(max(len(FRAMES),int(update_rect[0]/16-1)),int(update_rect[2]/16+1)):
					cr.newpath()
					cr.rect([i*16,k*32,i*16+16,k*32+32])
					if self.root.descendItem().activeframe==i:
						cr.fillcolor = Color([0.5,0.5,0.5]).pygui
						cr.fill()
					elif i%5==0:
						cr.fillcolor = Color([0.8,0.8,0.8]).pygui
						cr.fill()
						# print i
					else:
						cr.fillcolor = Color([1.0,1.0,1.0]).pygui
						cr.fill()
						cr.newpath()
						cr.fillcolor = Color([0.1,0.1,0.1]).pygui
						cr.rect([i*16+15,k*32,i*16+16,k*32+32])
						cr.fill()
			if self.x:
				src_rect = [0,0,16,32]
				dst_rect = [self.x-8, 0, self.x+8, 32]
				self.ackfr.draw(cr,src_rect,dst_rect)
		except:
			traceback.print_exc()
	def mouse_down(self, event):
		x, y = event.position
		clicks = event.num_clicks
		self.onMouseDown(self,x, y, clicks)
		self.canvas.invalidate_rect([0,0,self.canvas.extent[0],self.canvas.extent[1]])
	def mouse_drag(self, event):
		x, y = event.position
		self.onMouseDrag(self,x, y)
		self.canvas.invalidate_rect([0,0,self.canvas.extent[0],self.canvas.extent[1]])
	def mouse_up(self, event):
		x, y = event.position
		self.onMouseUp(self,x, y)
		self.canvas.invalidate_rect([0,0,self.canvas.extent[0],self.canvas.extent[1]])
	def onMouseDown(self,self1,x, y, button=1, clicks=1):
		pass
	def onMouseDrag(self, self1, x, y, button=1, clicks=1):
		pass
	def onMouseUp(self, self1, x, y, button=1, clicks=1):
		pass
	
	
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
	elif SYSTEM=="pyglet":
		pyglet.app.run()

def quit():
	#Self-descriptive
	FILE.close()
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

#! /usr/bin/python
# -*- coding:utf-8 -*-
# © 2012 Skyler Lehmkuhl
# Released under the GPLv3. For more information, see gpl.txt.

from __future__ import with_statement

import os, shutil, tarfile, tempfile, StringIO, urllib, subprocess, sys


# Workaround for broken menubar under Ubuntu
os.putenv("UBUNTU_MENUPROXY", "0")

#Uncomment to build on OS X
#import objc, AppKit, cPickle

#Uncomment to build on Windows
# import ctypes, ctypes.wintypes, win32print

#SVLGUI - my custom GUI wrapper to abstract the GUI
import svlgui

#swift_window - builds the application windows
import lightningbeam_windows

#pickle - used to save and open files
import pickle

#webbrowser - used to launch HTML5
import webbrowser

#misc_funcs - miscelleneous functions in a separate file so as not to clutter things up too much
import misc_funcs

#If we can import this, we are in the install directory. Mangle media paths accordingly.
try:
	from distpath import media_path
except:
	media_path = ""

#specify the current version and what version files it can still open
LIGHTNINGBEAM_VERSION = "1.0-alpha1"
LIGHTNINGBEAM_COMPAT = ["1.0-alpha1"]

#Global variables. Used to keep stuff together.
global root
global layers
global undo_stack
global redo_stack
undo_stack = []
redo_stack = []
global preferences
preferences={}

def clear(arr):
	arr.__delslice__(0,len(arr))

def update_date():
	return "Tue, January 10, 2012"

class edit:
	def __init__(self, type, obj, from_attrs, to_attrs):
		self.type = type
		self.obj = obj
		self.from_attrs = from_attrs
		self.to_attrs = to_attrs

class maybe:
	def __init__(self, type, obj, from_attrs):
		self.edit = edit(type, obj, from_attrs, from_attrs)
		self.type = type
	def complete(self, to_attrs):
		self.edit.to_attrs = to_attrs
		return self.edit

svlgui.undo_stack = undo_stack
svlgui.edit = edit
svlgui.maybe = maybe
svlgui.clear = clear

def onLoadFrames(self):
	'''for i in range(2000):
		if i%5==0:
			j = box(i*16,0,16,32,svlgui.Color([0.5,0.5,0.5]))
			self.add(j)
		else:
			j = box(i*16,0,16,32,svlgui.Color([1,1,1]))
			self.add(j)'''
def onClickFrame(self, x, y,button=1,clicks=1):
	self.clicks = clicks
	root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions = MainWindow.scriptwindow.text
	root.descendItem().activeframe = int(x/16)
	print ">>>>>> ", x, y
	MainWindow.stage.draw()
	MainWindow.scriptwindow.text = str(root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions)
def onKeyDownFrame(self, key):
	root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions = MainWindow.scriptwindow.text
	if key in [" ", "s", "r", "e", "b"]:
		svlgui.MODE=key
		svlgui.set_cursor({" ":"arrow","s":"arrow","r":"crosshair","e":"crosshair",
				"b":"arrow"}[key], MainWindow.stage)
		misc_funcs.update_tooloptions()
	elif key=="F6":
		add_keyframe()
	elif key=="F8":
		convert_to_symbol()
	MainWindow.scriptwindow.text = root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions
def onMouseDownGroup(self, x, y,button=1,clicks=1):
	self.activelayer.frames[self.activelayer.currentframe].actions = MainWindow.scriptwindow.text
	if svlgui.MODE in [" ", "s"]:
		if self.hitTest(x, y):
			self.clicked = True
		
	elif svlgui.MODE in ["r", "e", "p"]:
		if svlgui.MODE=="r":
			# 'c' stands for 'current'
			self.cshape = box(x, y, 0, 0)
		elif svlgui.MODE=="e":
			self.cshape = ellipse(x, y, 0, 0)
		elif svlgui.MODE=="p":
			for i in self.lines:
				if abs(x-i.endpoint1.x)<10 and abs(y-i.endpoint1.y)<10:
					x, y = i.endpoint1.x, i.endpoint1.y
					break
				elif abs(x-i.endpoint2.x)<10 and abs(i.endpoint2.y)<10:
					x, y = i.endpoint2.x, i.endpoint2.y
					break
			self.cshape = shape(x, y)
			'''self.cshape = svlgui.Line(svlgui.Point(x, y),svlgui.Point(x,y))
			for i in self.lines:
				if abs(self.cshape.endpoint1.x-i.endpoint1.x)<10 and abs(self.cshape.endpoint1.y-i.endpoint1.y)<10:
					self.cshape.connection1 = i.endpoint1
					self.cshape.connection1.lines.add(self.cshape)
					break
				elif abs(self.cshape.endpoint1.x-i.endpoint2.x)<10 and abs(self.cshape.endpoint1.y-i.endpoint2.y)<10:
					self.cshape.connection1 = i.endpoint2
					self.cshape.connection1.lines.add(self.cshape)
					break
			self.lines.append(self.cshape)
			return'''
		#self.cshape.rotation = 5
		self.cshape.initx,self.cshape.inity = x, y
		self.add(self.cshape)
		self.cshape.onMouseDown = onMouseDownObj
		self.cshape.onMouseMove = onMouseMoveObj
		self.cshape.onMouseDrag = onMouseDragObj
		self.cshape.onMouseUp = onMouseUpObj
		self.cshape.onKeyDown = onKeyDownObj
		undo_stack.append(maybe("add_object", self, {"frame":self.activelayer.currentframe, "layer":self.activelayer}))
		self.clicked = True
		MainWindow.scriptwindow.text = self.activelayer.frames[self.activelayer.currentframe].actions
	elif svlgui.MODE in ["t"]:
		self.ctext = svlgui.Text("Mimimi",x,y)
		self.ctext.editing = True
		svlgui.CURRENTTEXT = self.ctext
		self.ctext.onMouseDown = onMouseDownText
		self.ctext.onMouseDrag = onMouseDragText
		self.ctext.onMouseUp = onMouseUpText
		self.add(self.ctext)
		self.ctext = None
		undo_stack.append(edit("add_object", self, {"frame":self.activelayer.currentframe, "layer":self.activelayer}, \
												   {"frame":self.activelayer.currentframe, "layer":self.activelayer, \
													"obj":self.activelayer.frames[self.activelayer.currentframe].objs[-1]}))
		self.activelayer.currentselect = self.activelayer.frames[self.activelayer.currentframe].objs[-1]
	MainWindow.docbox.setvisible(True)
	MainWindow.textbox.setvisible(False)

def onMouseDownObj(self, x, y,button=1,clicks=1):
	MainWindow.scriptwindow.text = root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions
	self.clicked = True
	self.initx,self.inity = x-self.x, y-self.y
	if svlgui.MODE == " ":
		undo_stack.append(maybe("move", self, {"x":self.x, "y":self.y}))
	elif svlgui.MODE == "s":
		undo_stack.append(maybe("scale", self, {"x":self.x, "y":self.y, "xscale":self.xscale, "yscale":self.yscale}))
	elif svlgui.MODE == "b":
		if not (self.fillcolor.val == svlgui.FILLCOLOR.val and self.filled==True):
			undo_stack.append(edit("fill", self, {"filled":self.filled, "fillcolor":self.fillcolor}, {"filled":True, "fillcolor":svlgui.FILLCOLOR}))
			clear(redo_stack)
		self.filled = True
		self.fillcolor = svlgui.FILLCOLOR
def onMouseDownText(self,x,y,button=1,clicks=1):
	MainWindow.scriptwindow.text = root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions
	self.clicked = True
	self.initx, self.inity = x-self.x, y-self.y
	MainWindow.docbox.setvisible(False)
	MainWindow.textbox.setvisible(True)
	svlgui.CURRENTTEXT = self.obj
	if clicks>1:
		self.obj.editing = True
def onMouseDownFrame(self, x, y,button=1,clicks=1):
	pass
def onMouseDownMC(self, x, y, button=1, clicks=1):
	if clicks==2:
		self.obj.level = True
def onMouseUpGroup(self, x, y,button=1,clicks=1):
	self.clicked = False
	if svlgui.MODE in ["r", "e"]:
		self.cshape = None
		cobj = self.activelayer.frames[self.activelayer.currentframe].objs[-1]
		if isinstance(undo_stack[-1], maybe):
			if undo_stack[-1].edit.obj==self:
				if undo_stack[-1].type=="add_object":
					undo_stack[-1] = undo_stack[-1].complete({"obj":cobj, "frame":self.activelayer.currentframe, "layer":self.activelayer})
					clear(redo_stack)
	elif svlgui.MODE=="p":
		prelen = len(self.cshape.shapedata)
		self.cshape.shapedata = misc_funcs.simplify_shape(self.cshape.shapedata, svlgui.PMODE.split()[-1],1)
		postlen = len(self.cshape.shapedata)
		print str((prelen-postlen)*100/prelen)+"% reduction: started at "+str(prelen)+" vertices, ended at "+str(postlen)+" vertices"
		'''for i in self.lines:
			if abs(self.cshape.endpoint2.x-i.endpoint1.x)<10 and abs(self.cshape.endpoint2.y-i.endpoint1.y)<10:
				self.cshape.connection2 = i.endpoint1
				self.cshape.connection2.lines.add(self.cshape)
				break
			elif abs(self.cshape.endpoint2.x-i.endpoint2.x)<10 and abs(self.cshape.endpoint2.y-i.endpoint2.y)<10:
				self.cshape.connection2 = i.endpoint2
				self.cshape.connection2.lines.add(self.cshape)
				break'''
		self.cshape = None
		MainWindow.stage.draw()
def onMouseUpObj(self, x, y,button=1,clicks=1):
	self.clicked = False
	if isinstance(undo_stack[-1], maybe):
		if undo_stack[-1].edit.obj==self:
			if undo_stack[-1].type=="move":
				if abs(self.x-undo_stack[-1].edit.from_attrs["x"])>0 or abs(self.y-undo_stack[-1].edit.from_attrs["y"])>0:
					undo_stack[-1] = undo_stack[-1].complete({"x":self.x, "y":self.y})
					clear(redo_stack)
				else:
					del undo_stack[-1]
			elif undo_stack[-1].type=="scale":
				if abs(self.x-undo_stack[-1].edit.from_attrs["x"])>0 or abs(self.y-undo_stack[-1].edit.from_attrs["y"])>0 \
						or abs(self.xscale-undo_stack[-1].edit.from_attrs["xscale"])>0 or abs(self.yscale-undo_stack[-1].edit.from_attrs["yscale"])>0:
					undo_stack[-1] = undo_stack[-1].complete({"x":self.x, "y":self.y, "xscale":self.xscale, "yscale":self.yscale})
					clear(redo_stack)
				else:
					del undo_stack[-1]
def onMouseUpText(self, x, y,button=1,clicks=1):
	self.clicked = False
def onMouseUpFrame(self, x, y, button=1, clicks=1):
	self.x = None
	if root.descendItem().activeframe==root.descendItem().activelayer.currentframe:
		index = int(x/16)
		if index>len(root.descendItem().activelayer.frames):
			[root.descendItem().activelayer.frames.append(None) for i in xrange(len(root.descendItem().activelayer.frames),index+1)]
		if index>root.descendItem().activeframe:
			print "bigger"
			root.descendItem().activelayer.frames.insert(index, root.descendItem().activelayer.frames.pop(root.descendItem().activeframe))
		else:
			root.descendItem().activelayer.frames.insert(index, root.descendItem().activelayer.frames.pop(root.descendItem().activeframe))			
			if not any(root.activelayer.frames[index+1:]):
				root.descendItem().activelayer.frames = root.descendItem().activelayer.frames[:index+1]
		root.descendItem().currentframe = index
		print root.descendItem().activelayer.frames
		
def onMouseMoveGroup(self, x, y,button=1):
	pass
	#This is for testing rotation. Comment out before any commit!
	#root.rotation+=0.01
def onMouseMoveObj(self, x, y,button=1):
	pass
def onMouseDragGroup(self, x, y,button=1,clicks=1):
	if svlgui.MODE in [" ", "s"]:
		self.x = x
		self.y = y
	elif svlgui.MODE == "r":
		sd = self.cshape.shapedata
		x=x-self.cshape.initx
		y=y-self.cshape.inity
		self.cshape.shapedata = [sd[0],["L",x,sd[0][2]],["L",x,y],["L",sd[0][1],y],sd[4]]
	elif svlgui.MODE == "e":
		sd = self.cshape.shapedata
		x=x-self.cshape.initx
		y=y-self.cshape.inity
		self.cshape.shapedata = [["M",x/2,0],["C",4*x/5,0,x,y/5,x,y/2],["C",x,4*y/5,4*x/5,y,x/2,y],["C",x/5,y,0,4*y/5,0,y/2],["C",0,y/5,x/5,0,x/2,0]]
	elif svlgui.MODE == "p":
		self.cshape.shapedata.append(["L",x-self.cshape.initx,y-self.cshape.inity])
		# self.cshape.endpoint2.x = x
		# self.cshape.endpoint2.y = y
def onMouseDragObj(self, x, y,button=1,clicks=1):
	if svlgui.MODE==" ":
		self.x = x-self.initx
		self.y = y-self.inity
	elif svlgui.MODE=="s":
		if svlgui.SCALING:
			# self.xscale = (x-(self.maxx/2.0+self.minx))/(self.maxx/2.0)
			# self.yscale = (y-(self.maxy/2.0+self.miny))/(self.maxy/2.0)
			if self.initx>self.maxx/2:
				self.xscale = (x-self.x)/self.maxx
			else:
				# I don't understand why I need 2*self.maxx instead of just maxx, but it works.
				self.xscale = (2*self.maxx+self.x-(x-self.initx)-x)/self.maxx
				self.x = x
			if self.inity>self.maxy/2:
				self.yscale = (y-self.y)/self.maxy
			else:
				# 3 times?? Why??
				self.yscale = (3*self.maxy+self.y-(y-self.inity)-y)/self.maxy
				self.y = y


			print self.initx

			# self.xscale = ((self.maxx/2.0+self.minx)-x)/((self.maxx/2.0+self.minx)-self.initx)
			# self.yscale = ((self.maxy/2.0+self.miny)-y)/((self.maxy/2.0+self.miny)-self.inity)

def onMouseDragText(self, x, y,button=1,clicks=1):
	self.x = x-self.initx
	self.y = y-self.inity
def onMouseDragFrame(self, x, y, button=1, clicks=1):
	if root.descendItem().activeframe==root.descendItem().activelayer.currentframe:
		self.x = x
def onKeyDownGroup(self, key):
	if not svlgui.EDITING:
		if key in [" ", "s", "r", "e", "b", "p"]:
			svlgui.MODE=key
			svlgui.set_cursor({" ":"arrow","s":"arrow","r":"crosshair","e":"crosshair",
					"b":"arrow","p":"arrow"}[key], MainWindow.stage)
			misc_funcs.update_tooloptions()
		elif key=="F6":
			add_keyframe()
		elif key=="F8":
			convert_to_symbol()
	else:
		if not key=="escape":
			pass
		else:
			svlgui.EDITING=False
def onKeyDownObj(self, key):
	if key in ("delete", "backspace"):
		del self.parent[self.parent.index(self)] # Need to clean up deletion
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
		self.x-=1
	elif key=="right_arrow":
		self.x+=1
	elif key=="up_arrow":
		self.y-=1
	elif key=="down_arrow":
		self.y+=1
	
def create_sc(root):
	#retval = ".flash bbox="+str(svlgui.WIDTH)+"x"+str(svlgui.HEIGHT)+" background=#ffffff \
#fps="+str(svlgui.FRAMERATE)+"\n"+root.print_sc()+".end"
	print svlgui.Library
	retval = ".flash bbox="+str(svlgui.WIDTH)+"x"+str(svlgui.HEIGHT)+" background=#ffffff \
fps="+str(svlgui.FRAMERATE)+"\n"+"".join([i.print_sc() for i in svlgui.Library])+root.print_sc()+".end"
	return retval
def run_file(self=None):
	global root
	print "RUNNING"
	print svlgui.FILE.name
	root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions = MainWindow.scriptwindow.text
	# open(os.getenv('HOME')+"/test.sc", "w").write(create_sc(root))
	open(svlgui.FILE.name+".sc", "w").write(create_sc(root))
	# svlgui.execute("swfc/swfc_"+svlgui.PLATFORM+" "+os.getenv('HOME')+"/test.sc -o "+os.getenv('HOME')+"/test.swf")
	# x = os.system("swfc/swfc_"+svlgui.PLATFORM+" "+os.getenv('HOME')+"/test.sc -o "+os.getenv('HOME')+"/test.swf")
	x = os.system("swfc"+svlgui.sep+"swfc_"+svlgui.PLATFORM+" "+svlgui.FILE.name+".sc -o "+svlgui.FILE.name+".swf")
	if sys.version_info < (2, 6):
		if x==5:	# which is the error value returned when linking libjpeg fails
			if svlgui.alert("You appear to be missing libjpeg. Install it?", confirm=True):
				os.system("""osascript -e 'do shell script "mkdir -p /usr/local/lib; cp swfc/libjpeg.8.dylib /usr/local/lib" with administrator privileges'""")
				# x = os.system("swfc/swfc_"+svlgui.PLATFORM+" "+os.getenv('HOME')+"/test.sc -o "+os.getenv('HOME')+"/test.swf")
				x = os.system("swfc/swfc_"+svlgui.PLATFORM+" "+svlgui.FILE.name+".sc -o "+svlgui.FILE.name+".swf")
				if x==5:
					svlgui.alert("Sorry, something has gone terribly wrong.")
			else:
				return
	#TODO: Make this cross-platform compatible
	if svlgui.PLATFORM=="win32":
		# Untested.
		logloc = os.path.expanduser("~")+"\\AppData\\Roaming\\Macromedia\\Flash Player\\Logs\\flashlog.txt"
		if not os.path.exists(os.path.expanduser("~")+"\\flashplayerdebugger.exe"):
			result = svlgui.alert("You do not have a Flash debugger installed. Install one?", confirm=True)
			if not result:
				svlgui.alert("Aborting.")
				return
			else:
				svlgui.alert("The Flash Debugger will download when you click Ok.\nThis may take some time.")
				urllib.urlretrieve("http://download.macromedia.com/pub/flashplayer/updaters/11/flashplayer_11_sa_debug.exe", os.path.expanduser("~")+"\\flashplayerdebugger.exe")
		if not os.path.exists(os.path.expanduser('~')+'\\mm.cfg'):
			with open(os.path.expanduser('~')+"/mm.cfg", "w") as mm:
				mm.write("ErrorReportingEnable=1\nTraceOutputFileEnable=1")
	elif "linux" in svlgui.PLATFORM:
		logloc = os.getenv('HOME')+"/.macromedia/Flash_Player/Logs/flashlog.txt"
		if not os.path.exists('/usr/bin/flashplayerdebugger'):
			if os.system('which gnash')==0:
				if not 'use_gnash' in preferences:
					if svlgui.alert("You have GNASH installed. Do you wish to use it instead of Adobe's flash player?", confirm=True):
						use_gnash = True
						return
			else:
				result = svlgui.alert("You do not have a Flash debugger installed. Install one?", confirm=True)
				if not result:
					svlgui.alert("Aborting.")
					return
				else:
					svlgui.alert("The Flash Debugger will download when you click Ok.\nThis may take some time.")
					if not "PPC" in svlgui.PLATFORM:
						urllib.urlretrieve("http://fpdownload.macromedia.com/pub/flashplayer/updaters/11/flashplayer_11_sa_debug.i386.tar.gz", "fp.tar.gz")
						os.system("tar -zxf fp.tar.gz")
						os.system("gksudo mv flashplayerdebugger /usr/bin/")
		if not os.path.exists(os.getenv('HOME')+"/mm.cfg"):
			# By default, the Flash debugger on Linux does not log traces.
			# So, we create a configuration file to tell it to do so if  the user hasn't already.
			with open(os.getenv('HOME')+"/mm.cfg", "w") as mm:
				mm.write("ErrorReportingEnable=1\nTraceOutputFileEnable=1")
	elif svlgui.PLATFORM=="osx":
		logloc = os.getenv('HOME')+"/Library/Preferences/Macromedia/Flash Player/Logs/flashlog.txt"
		if not os.path.exists('/Applications/Flash Player Debugger.app'):
			# check for Flash Player
			result = svlgui.alert("You do not have a Flash debugger installed. Install one?", confirm=True)
			if not result:
				svlgui.alert("Aborting.")
				return
			else:
				svlgui.alert("The Flash Debugger will download when you click Ok.\nThis may take some time.")
				if sys.version_info < (2, 6):
					# Newer flash players won't run on Leopard
					urllib.urlretrieve("http://download.macromedia.com/pub/flashplayer/updaters/10/flashplayer_10_sa_debug.app.zip", "fp.app.zip")
				else:
					urllib.urlretrieve("http://fpdownload.macromedia.com/pub/flashplayer/updaters/11/flashplayer_11_sa_debug.app.zip", "fp.app.zip")
				# Unzip the file. Apparently ditto is better for OSX apps than unzip.
				os.system('ditto -V -x -k --sequesterRsrc --rsrc fp.app.zip .')
				shutil.move('Flash Player Debugger.app', '/Applications/Flash Player Debugger.app')
				# Generally it is not recognized until it is opened
				os.system('open -a "/Applications/Flash Player Debugger.app"')
				# Set Flash Player Debugger as the default app for .swf files
				os.system('defaults write com.apple.LaunchServices LSHandlers -array-add "<dict><key>LSHandlerContentTag</key><string>swf</string><key>LSHandlerContentTagClass</key><string>public.filename-extension</string><key>LSHandlerRoleAll</key><string>com.macromedia.flash player debugger.app</string></dict>"')
				os.system("/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister -kill -r -domain local -domain system -domain user")
				svlgui.alert("Downloaded!")
		# mm.cfg is the debugger's configuration file - we're telling it to log output
		if not os.path.exists(os.getenv('HOME')+'/mm.cfg'):
			with open(os.getenv('HOME')+'/mm.cfg', "w") as mm:
				mm.write("ErrorReportingEnable=1\nTraceOutputFileEnable=1")
			if not os.path.exists(os.getenv('HOME')+"/Library/Preferences/Macromedia/Flash Player/Logs"):
				os.mkdir(os.getenv('HOME')+"/Library/Preferences/Macromedia/Flash Player/Logs")
			with open(logloc, "w") as f:
				f.write("")
	try:
		logfile.close()
	except:
		pass
	logfile = open(logloc,"w")
	logfile.write("")
	logfile.close()
	outputwin = svlgui.Window("Output")
	outputwin.resize(200,500)
	outputtext = svlgui.TextView(False)
	outputwin.add(outputtext)
	logfile = open(logloc, "r")
	def updatetrace(outputtext):
		try:
			# print logfile.readline()
			outputtext.text+=logfile.readline()
			outputtext.scroll_bottom()		# this doesn't work
		except:
			pass
		return True
	if hasattr(svlgui, 'gobject'):
		svlgui.gobject.timeout_add(20, updatetrace, outputtext)
	else:
		r = misc_funcs.RepeatTimer(0.02, updatetrace, args=[outputtext])
		r.daemon = True
		r.start()
	if svlgui.PLATFORM=="osx":
		osx_flash_player_loc = "/Applications/Flash\ Player\ Debugger.app"
		# success = svlgui.execute("open -a "+osx_flash_player_loc+" "+os.getenv('HOME')+"/test.swf")
		success = svlgui.execute("open -a "+osx_flash_player_loc+" "+svlgui.FILE.name+".swf")
		if not success:
			svlgui.alert("Oops! Didn't work. I probably couldn't find your Flash debugger!")
	elif svlgui.PLATFORM=='win32':
		win_flash_player_loc = os.path.expanduser("~")+"\\flashplayerdebugger.exe"
		# svlgui.execute('start '+win_flash_player_loc+" test.swf")
		svlgui.execute('start '+win_flash_player_loc+" "+svlgui.FILE.name+".swf")
	elif svlgui.PLATFORM.startswith('linux'):
		linux_flash_player_loc = "/usr/bin/flashplayerdebugger"
		# if not svlgui.execute(linux_flash_player_loc+" "+os.getenv('HOME')+"/test.swf &"):
		if not svlgui.execute(linux_flash_player_loc+" "+svlgui.FILE.name+".swf &"):
			if '64' in svlgui.PLATFORM:
				svlgui.alert("Flash debugger failed to launch! Try installing ia32-libs.")
			elif '32' in svlgui.PLATFORM:
				svlgui.alert("Flash debugger failed to launch! No idea why.")
def create_html5(root):
	retval = "<head>\n\
<style type=\"text/css\">\n\
canvas { \n\
border: none; position:absolute; top:0;left:0;\n\
visibility: hidden; }\n\
</style>\n\
</head>\n\
<body>\n\
<div id='events'>\n\
<canvas id=\"canvas1\" width="+str(svlgui.WIDTH)+" height="+str(svlgui.HEIGHT)+" ></canvas>\n\
<canvas id=\"canvas2\" width="+str(svlgui.WIDTH)+" height="+str(svlgui.HEIGHT)+"></canvas>\n\
</div>\n\
<script>\n\
//Setup\nvar fps = "+str(svlgui.FRAMERATE)+";\n</script>\n\
<script src=\"base.js\">\n\
</script>\n\
<script>\n"+"".join([i.print_html() for i in svlgui.Library])+root.print_html()+"\n\
document.onmousemove = function(e){_root._xmouse=e.pageX;_root._ymouse=e.pageY}\n\
document.onmousedown = function(e){Event.doEvent(\"onMouseDown\")}\n\
document.onkeydown = function(e){Key.press(e);Event.doEvent(\"onKeyDown\")}\n\
document.onkeyup = function(e){Key.release(e);Event.doEvent(\"onKeyUp\")}\n\
</script>\n</body>\n</html>"
	return retval
def run_html(self=None):
	global root
	print "RUNNING"
	root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions = MainWindow.scriptwindow.text
	open(os.getenv('HOME')+"/test.html", "w").write(create_html5(root))
	try:
		shutil.copyfile("base.js",os.getenv('HOME')+"/base.js")
	except IOError:
		svlgui.alert("Couldn't copy base.js to "+os.getenv('HOME')+"/base.js!")
	webbrowser.open("file://"+os.getenv('HOME')+"/test.html")

		

def box(x, y, width, height, fill=None):
	global objects
	box = svlgui.Shape(x, y)
	box.shapedata = [["M",0,0],["L",width,0],["L",width,height],["L",0,height],["L",0,0]]
	box.onMouseDown = onMouseDownObj
	box.onMouseUp = onMouseUpObj
	if fill:
		box.fillcolor = fill
		box.linecolor = svlgui.Color([0,0,0,0])
		box.filled = True
	return box
def ellipse(x, y, width, height, fill=svlgui.FILLCOLOR):
	global objects
	ellipse = svlgui.Shape(x, y)
	ellipse.shapedata = [["M",width/2,0],["C",4*width/5,0,width,height/5,width,height/2], ["C",width,4*height/5,4*width/5,height,width/2,height], ["C",width/5,height,0,4*height/5,0,height/2], ["C",0,height/5,width/5,0,width/2,0]]

	# must figure out shapedata...
	return ellipse
def shape(x, y, fill=None):
	shape = svlgui.Shape(x,y)
	shape.shapedata = [["M",0,0]]
	return shape

root = svlgui.Group(skipl=True)
root.name = "_root"
root.level = True
root.onMouseDown = onMouseDownGroup
root.onMouseUp = onMouseUpGroup
root.onMouseMove = onMouseMoveGroup
root.onMouseDrag = onMouseDragGroup
root.onKeyDown = onKeyDownGroup

svlgui.root = root

e=ellipse(100,100,100,50,None)
e.onMouseDown = onMouseDownObj
e.onMouseMove = onMouseMoveObj
e.onMouseDrag = onMouseDragObj
e.onMouseUp = onMouseUpObj
e.onKeyDown = onKeyDownObj
root.add(e)




if svlgui.SYSTEM == "gtk":
	overlaywindow = svlgui.OverlayWindow()
	MainWindow = lightningbeam_windows.MainWindow()
elif svlgui.SYSTEM=="osx":
	MainWindow = lightningbeam_windows.MainWindowOSX()
elif svlgui.SYSTEM=="kivy":
	MainWindow = lightningbeam_windows.MainWindowKivy()
elif svlgui.SYSTEM=="html":
	MainWindow = lightningbeam_windows.MainWindowHTML()
elif svlgui.SYSTEM=="pyglet":
	MainWindow = lightningbeam_windows.MainWindowOSX()
elif svlgui.SYSTEM=="android":
	MainWindow = lightningbeam_windows.MainWindowAndroid()
MainWindow.stage.add(root, 0,0)
svlgui.FOCUS = MainWindow.stage
layers = svlgui.Group(skipl=True)
b = svlgui.Image(media_path+"media/object_active.png",0,0,True,MainWindow.layerbox,16,1,True)
layers.add(b)
MainWindow.layerbox.add(layers,0,0)

#frames = svlgui.Group(onload=onLoadFrames,skipl=True)
#b = svlgui.Image("media/keyframe_active.png",0,0,True,MainWindow.timelinebox,16,1,True)
#frames.add(b)
#frames.onMouseDown = onClickFrame
#frames.onKeyDown = onKeyDownFrame
#MainWindow.timelinebox.add(frames,0,0)
MainWindow.timelinebox.root = root
MainWindow.timelinebox.onMouseDown = onClickFrame
MainWindow.timelinebox.onMouseDrag = onMouseDragFrame
MainWindow.timelinebox.onMouseUp = onMouseUpFrame


def new_file(widget=None):
	global root
	MainWindow.stage.delete(root)
	root = svlgui.Group()
	root.level = True
	root.onMouseDown = onMouseDownGroup
	root.onMouseUp = onMouseUpGroup
	root.onMouseMove = onMouseMoveGroup
	MainWindow.stage.add(root,0,0)
def open_file(widget=None):
	global root
	MainWindow.stage.delete(root)
	shutil.rmtree(svlgui.SECURETEMPDIR)
	try:
		thetarfile = tarfile.open(fileobj=svlgui.file_dialog("open").open("rb"),mode="r:gz")
	except AttributeError:
		# User cancelled
		return
	basefile = thetarfile.extractfile("basefile")
	root, svlgui.Library = pickle.load(basefile)
	svlgui.SECURETEMPDIR = tempfile.mkdtemp()
	thetarfile.extractall(path=svlgui.SECURETEMPDIR)
	for i in svlgui.Library:
		if i.type=="Image":
			if not hasattr(i,'path'):
				i.val = svlgui.SECURETEMPDIR+"/"+i.val.split(os.sep)[-1]
				i.set_image(i.val)
			else:
				i.path = svlgui.SECURETEMPDIR+"/"+i.path.split(os.sep)[-1]
				i.set_image(i.path)
			if not hasattr(i, 'iname'):
				i.iname = None
	MainWindow.stage.add(root, 0, 0)
	MainWindow.stage.draw()
	MainWindow.timelinebox.root = root
	MainWindow.timelinebox.draw()
	thetarfile.close()
def open_sc_file(widget=None):
	pass
def save_file(widget=None):
	data = pickle.dumps((root,svlgui.Library))
	tarinfo = tarfile.TarInfo('basefile')
	tarinfo.size = len(data)
	if svlgui.FILE.name.startswith(svlgui.TEMPDIR):
		try:
			thetarfile = tarfile.open(fileobj=svlgui.file_dialog("save", name="untitled.beam").open('wb'),mode="w:gz")
		except AttributeError:
			# User cancelled
			return
	else:
		thetarfile = tarfile.open(svlgui.FILE.name,mode="w:gz")
	thetarfile.addfile(tarinfo, StringIO.StringIO(data))
	#Save the path so we can come back here
	lastpath = os.path.abspath(".")
	for i in svlgui.Library:
		if i.type=="Image":
			if not hasattr(i, 'path'):
				try:
					os.chdir(os.sep.join(i.val.split(os.sep)[:-1]) or i.origpath)
					i.val = i.val.split(os.sep)[-1]
					thetarfile.add(i.val.split(os.sep)[-1])
				except OSError:
					tmpdir = tempfile.mkdtemp()
					os.chdir(tmpdir)
					i.pilimage.save(i.val)
					thetarfile.add(i.val)
					os.remove(i.val)
			else:
				print "i.path: ",i.path
				try:
					os.chdir(os.sep.join(i.path.split(os.sep)[:-1]) or i.origpath)
					i.path = i.path.split(os.sep)[-1]
					thetarfile.add(i.path.split(os.sep)[-1])
				except OSError:
					tmpdir = tempfile.mkdtemp()
					os.chdir(tmpdir)
					i.pilimage.save(i.path)
					thetarfile.add(i.path)
					os.remove(i.path)
			os.chdir(lastpath)
	thetarfile.close()
	svlgui.FILE = thetarfile
	#thetarfile.close()
def save_file_as(widget=None):
	print "HI"
	data = pickle.dumps((root,svlgui.Library))
	tarinfo = tarfile.TarInfo('basefile')
	tarinfo.size = len(data)
	try:
		thetarfile = tarfile.open(fileobj=svlgui.file_dialog("save", name="untitled.beam").open('wb'),mode="w:gz")
	except AttributeError:
		# User cancelled
		return
	thetarfile.addfile(tarinfo, StringIO.StringIO(data))
	#Save the path so we can come back here
	lastpath = os.path.abspath(".")
	for i in svlgui.Library:
		if i.type=="Image":
			print "i.path: ",i.path
			try:
				os.chdir(os.sep.join(i.path.split(os.sep)[:-1]) or i.origpath)
				i.path = i.path.split(os.sep)[-1]
				thetarfile.add(i.path.split(os.sep)[-1])
			except OSError:
				tmpdir = tempfile.mkdtemp()
				os.chdir(tmpdir)
				i.pilimage.save(i.path)
				thetarfile.add(i.path)
				os.remove(i.path)
			os.chdir(lastpath)
	thetarfile.close()
	svlgui.FILE = thetarfile
	pass
def import_to_stage(widget=None):
	try:
		thefile = svlgui.file_dialog("open",None,["jpg","png","bmp","wav","mp3","ogg"]).path
	except AttributeError:
		# User cancelled
		return
	for i in ("jpg","png","bmp"):
		if thefile.endswith(i):
			# im = svlgui.Image(thefile)
			if svlgui.PLATFORM=="osx":
				# sips is OSX's built-in image manipulation tool
				os.system("sips -s format png "+thefile+" --out "+svlgui.SECURETEMPDIR+"/"+".".join(thefile.split("/")[-1].split(".")[:-1])+".png")
			elif "linux" in svlgui.PLATFORM:
				os.system("convert "+thefile+" "+svlgui.SECURETEMPDIR+"/"+".".join(thefile.split("/")[-1].split(".")[:-1])+".png")
			elif svlgui.PLATFORM=="win32":
				from PIL import Image
				im = Image.open(thefile)
				# Use alpha channel if available (e.g. GIF)
				transparency = im.info["transparency"] if 'transparency' in im.info else None
				if transparency:
					im.save(svlgui.SECURETEMPDIR+"\\"+".".join(thefile.split("\\")[-1].split(".")[:-1])+".png", transparency=transparency)
				else:
					im.save(svlgui.SECURETEMPDIR+"\\"+".".join(thefile.split("\\")[-1].split(".")[:-1])+".png")
			thefile = svlgui.SECURETEMPDIR+svlgui.sep+".".join(thefile.split(svlgui.sep)[-1].split(".")[:-1])+".png"
			im = box(100,100,200,200,svlgui.Color(thefile))
			im.onMouseDown = onMouseDownObj
			im.onMouseMove = onMouseMoveObj
			im.onMouseDrag = onMouseDragObj
			im.onMouseUp = onMouseUpObj
			im.onKeyDown = onKeyDownObj
			root.descendItem().add(im)
			break
	else:
		for i in ("wav", "mp3", "ogg"):
			if thefile.endswith(i):
				if i in ("mp3", "ogg"):
					if svlgui.PLATFORM=="win32":
						if os.system("C:\\Progra~2\\MPlayer\\mplayer.exe")!=0:
							if os.system("C:\\Progra~1\\MPlayer\\mplayer.exe")!=0:
								result = svlgui.alert("To import mp3 and ogg files you must install mplayer. This will take about 3 MB of space. Install?", confirm=True)
								if not result:
									return
								urllib.urlretrieve("http://mplayer.kenkon.net/files/mplayer-win32-mingw-dev-svn20061001.exe", "mplayerdl.exe")
								os.system("mplayerdl.exe")
								mpath = "C:\\Progra~2\\MPlayer\\mplayer.exe"
							else:
								mpath = "C:\\Progra~1\\MPlayer\\mplayer.exe"
						else:
							mpath = "C:\\Progra~2\\MPlayer\\mplayer.exe"
					else:
						mpath = "mplayer"
					theorigfile = thefile
					thefile = svlgui.SECURETEMPDIR+svlgui.sep+thefile.split(svlgui.sep)[-1].replace(" ","_").replace("-","_").split(".")[0]+'.wav'
					print mpath+' -srate 48000 -vo null -vc null -ao pcm:fast:file="'+thefile+'" "'+theorigfile+'"'
					if svlgui.PLATFORM=="win32":
						os.system(mpath+' -srate 48000 -vo null -vc null -ao pcm:fast:file=\\"'+thefile+'\\" "'+theorigfile+'"')
					else:
						os.system(mpath+' -srate 48000 -vo null -vc null -ao pcm:fast:file="'+thefile+'" "'+theorigfile+'"')
				if svlgui.PLATFORM=="osx":
					if not os.path.exists('sox/sox'):
						try:
							import numpy as NP
							result = svlgui.alert("To import sound you must install SoX. This will take about 1 MB of space. Install?", confirm=True)
							if not result:
								return
							urllib.urlretrieve('http://downloads.sourceforge.net/project/sox/sox/14.4.0/sox-14.4.0-macosx.zip?r=&ts=1357270265&use_mirror=iweb', 'sox.zip')
							os.system('ditto -V -x -k --sequesterRsrc --rsrc sox.zip .')
							os.system('mv sox-14.4.0 sox')
						except:
							result = svlgui.alert("To import sound you must install NumPy and SoX. This will take about 10 MB of space. Install?", confirm=True)
							if not result:
								return
							os.system("""osascript -e 'do shell script "easy_install numpy" with administrator privileges'""")
							import numpy as NP
							urllib.urlretrieve('http://downloads.sourceforge.net/project/sox/sox/14.4.0/sox-14.4.0-macosx.zip?r=&ts=1357270265&use_mirror=iweb', 'sox.zip')
							os.system('ditto -V -x -k --sequesterRsrc --rsrc sox.zip .')
							os.system('mv sox-14.4.0 sox')
					else:
						try:
							import numpy as NP
						except:
							result = svlgui.alert("To import sound you must install NumPy. This will take about 9 MB of space. Install?", confirm=True)
							if not result:
								return
							os.system("""osascript -e 'do shell script "easy_install numpy" with administrator privileges'""")
							import numpy as NP
					SOX_EXEC = 'sox/sox'
				elif svlgui.PLATFORM=="win32":
					SOX_EXEC = "C:\\Progra~2\\sox-14~1\\sox.exe"
					if not os.path.exists(SOX_EXEC):
						if os.path.exists("C:\\Progra~1\\sox-14~1\\sox.exe"):
							SOX_EXEC = "C:\\Progra~1\\sox-14~1\\sox.exe"
						else:
							urllib.urlretrieve("http://downloads.sourceforge.net/project/sox/sox/14.4.0/sox-14.4.0-win32.exe?r=&ts=1358725954&use_mirror=iweb", 'sox.exe')
							os.system("sox.exe")
					import numpy as NP
				elif "linux" in svlgui.PLATFORM:
					import numpy as NP
					SOX_EXEC = "sox"
				svlgui.NP = NP
				num_channels = 1
				out_byps = 2 # Bytes per sample you want, must be 1, 2, 4, or 8
				cmd = [SOX_EXEC,
					thefile,              # input filename
					'-t','raw',            # output file type raw
					'-e','signed-integer', # output encode as signed ints
					'-L',                  # output little endin
					'-b',str(out_byps*8),  # output bytes per sample
					'-']                   # output to stdout]
				data = NP.fromstring(subprocess.check_output(cmd),'<i%d'%(out_byps))
				data = data.reshape(len(data)/num_channels, num_channels)
				info = subprocess.check_output([SOX_EXEC,'--i',thefile])
				if i=="mp3":
					sound = svlgui.Sound(data, name=thefile.split(svlgui.sep)[-1], path=theorigfile, info=info, type=i)
				else:
					sound = svlgui.Sound(data, name=thefile.split(svlgui.sep)[-1], path=thefile, info=info, type=i)
				root.descendItem().add(sound)
				break

	MainWindow.stage.draw()
def import_to_library(widget=None):
	pass

def quit(widget):
	svlgui.quit()
	
def undo(widget=None):
	if len(undo_stack)>0:
		if isinstance(undo_stack[-1], edit):
			e = undo_stack.pop()
			print e.from_attrs
			print e.to_attrs
			if e.type=="move":
				e.obj.x = e.from_attrs["x"]
				e.obj.y = e.from_attrs["y"]
			elif e.type=="scale":
				e.obj.x = e.from_attrs["x"]
				e.obj.y = e.from_attrs["y"]
				e.obj.xscale = e.from_attrs["xscale"]
				e.obj.yscale = e.from_attrs["yscale"]
			elif e.type=="fill":
				e.obj.filled = e.from_attrs["filled"]
				e.obj.fillcolor = e.from_attrs["fillcolor"]
			elif e.type=="add_object":
				if e.from_attrs["layer"].currentselect==e.to_attrs["obj"]:
					e.from_attrs["layer"].currentselect = None
				del e.from_attrs["layer"].frames[e.from_attrs["frame"]].objs[e.from_attrs["layer"].frames[e.from_attrs["frame"]].objs.index(e.to_attrs["obj"])]
			elif e.type=="text":
				e.obj.text = e.from_attrs["text"]
				e.obj.cursorpos = e.from_attrs["cursorpos"]
			redo_stack.append(e)
		MainWindow.stage.draw()

def redo(widget=None):
	if len(redo_stack)>0:
		if isinstance(redo_stack[-1], edit):
			e = redo_stack.pop()
			print e.from_attrs
			print e.to_attrs
			if e.type=="move":
				e.obj.x = e.to_attrs["x"]
				e.obj.y = e.to_attrs["y"]
			elif e.type=="scale":
				e.obj.x = e.to_attrs["x"]
				e.obj.y = e.to_attrs["y"]
				e.obj.xscale = e.to_attrs["xscale"]
				e.obj.yscale = e.to_attrs["yscale"]
			elif e.type=="fill":
				e.obj.filled = e.to_attrs["filled"]
				e.obj.fillcolor = e.to_attrs["fillcolor"]
			elif e.type=="add_object":
				e.to_attrs["layer"].frames[e.from_attrs["frame"]].objs.append(e.to_attrs["obj"])
			elif e.type=="text":
				e.obj.text = e.to_attrs["text"]
				e.obj.cursorpos = e.to_attrs["cursorpos"]
			undo_stack.append(e)
		MainWindow.stage.draw()
	
def cut(widget=None):
	if MainWindow.scriptwindow.is_focused():
		clip = MainWindow.scriptwindow.text[MainWindow.scriptwindow.selection[0]:MainWindow.scriptwindow.selection[1]]
		MainWindow.scriptwindow.insert("")
		svlgui.app.set_clipboard(clip)

def copy(widget=None):
	if MainWindow.scriptwindow.is_focused():
		clip = MainWindow.scriptwindow.text[MainWindow.scriptwindow.selection[0]:MainWindow.scriptwindow.selection[1]]
		svlgui.app.set_clipboard(clip)
def paste(widget=None):
	clip = svlgui.app.get_clipboard() if svlgui.app.query_clipboard() else None
	if clip:
		print clip
		if MainWindow.stage.is_focused():
			ctext = svlgui.Text(clip,200,100)
			ctext.editing = False
			# svlgui.CURRENTTEXT = self.ctext
			ctext.onMouseDown = onMouseDownText
			ctext.onMouseDrag = onMouseDragText
			ctext.onMouseUp = onMouseUpText
			self = root.descendItem()
			self.add(ctext)
			# self.add(self.ctext)
			# self.ctext = None
			undo_stack.append(edit("add_object", self, {"frame":self.activelayer.currentframe, "layer":self.activelayer}, \
													   {"frame":self.activelayer.currentframe, "layer":self.activelayer, \
														"obj":self.activelayer.frames[self.activelayer.currentframe].objs[-1]}))
			self.activelayer.currentselect = self.activelayer.frames[self.activelayer.currentframe].objs[-1]
			MainWindow.stage.draw()
		elif MainWindow.scriptwindow.is_focused():
			MainWindow.scriptwindow.insert(clip)

def add_keyframe(widget=None):
	print "af> ", root.descendItem().activeframe
	root.descendItem().add_frame(True)
	MainWindow.timelinebox.draw()
def add_layer(widget=None):
	root.descendItem().add_layer(root.descendItem()._al)
	layers.add(svlgui.Image(media_path+"media/object_active.png",0,root.descendItem().layers.index(root.descendItem().activelayer)*32,True,MainWindow.layerbox,16,1,True))
	print root.descendItem().layers.index(root.descendItem().activelayer)*32
	#MainWindow.layerbox.draw()
def delete_layer(widget=None):
	root.descendItem().delete_layer(root.descendItem()._al)
	#layers.delete(box(0,(root.layers.index(root.activelayer))*32,128,32,svlgui.Color("media/object_inactive.png")))
	MainWindow.timelineref.draw()

def send_to_back(widget=None):
	rac = root.descendItem().activelayer.currentFrame()
	index = rac.index(root.descendItem().activelayer.currentselect)
	if index>0:
		a = rac[:index]
		b = rac[index+1:]
		del rac[:index]
		del rac[1:]
		[rac.append(i) for i in a]
		[rac.append(i) for i in b]
	MainWindow.stage.draw()
	
def send_backward(widget=None):
	rac = root.descendItem().activelayer.currentFrame()
	index = rac.index(root.descendItem().activelayer.currentselect)
	if index>0:
		rac[index-1], rac[index] = rac[index], rac[index-1]
	MainWindow.stage.draw()
	
def bring_forward(widget=None):
	rac = root.descendItem().activelayer.currentFrame()
	index = rac.index(root.descendItem().activelayer.currentselect)
	if index+1<len(rac):
		rac[index+1], rac[index] = rac[index], rac[index+1]
	MainWindow.stage.draw()
	
def bring_to_front(widget=None):
	rac = root.descendItem().activelayer.currentFrame()
	index = rac.index(root.descendItem().activelayer.currentselect)
	if index<len(rac):
		a = rac[index]
		del rac[index]
		rac.append(a)
	MainWindow.stage.draw()
	
def convert_to_symbol(widget=None):
	if not root.descendItem().activelayer.currentselect:
		svlgui.alert("No object selected!")
		return
	else:
		svlgui.ConvertToSymbolWindow(root, onMouseDownMC)
		MainWindow.stage.draw()
		
def export_svg(widget=None):
	pass

def export_swf(widget=None):
	global root
	root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions = MainWindow.scriptwindow.text
	
	f = svlgui.file_dialog("save", name="untitled.swf")
	if not f:
		return
	open(f.path+".sc",'w').write(create_sc(root))
	# svlgui.execute("swfc/swfc_"+svlgui.PLATFORM+" "+os.getenv('HOME')+"/test.sc -o "+os.getenv('HOME')+"/test.swf")
	x = os.system("swfc"+svlgui.sep+"swfc_"+svlgui.PLATFORM+" "+f.path+".sc -o "+f.path)
	if sys.version_info < (2, 6):
		if x==5:	# which is the error value returned when linking libjpeg fails
			if svlgui.alert("You appear to be missing libjpeg. Install it?", confirm=True):
				os.system("""osascript -e 'do shell script "mkdir -p /usr/local/lib; cp swfc/libjpeg.8.dylib /usr/local/lib" with administrator privileges'""")
				x = os.system("swfc/swfc_"+svlgui.PLATFORM+" "+f.path+".sc -o "+f.path)
				if x==5:
					svlgui.alert("Sorry, something has gone terribly wrong.")
			else:
				return

def about(widget=None):
	svlgui.alert("Lightningbeam v1.0-alpha1\nLast Updated: "+update_date()+
	"\nCreated by: Skyler Lehmkuhl\nBased on SWIFT")
	
def preferences(widget=None):
	prefwin = svlgui.PreferencesWindow()


svlgui.menufuncs([["File",
						("New...", new_file,"<Control>N"),
						("Open", open_file,"<Control>O"),
						("Open .sc", open_sc_file),
						("Save",save_file,"<Control>S"),
						("Save As", save_file_as,"/^s"),
						"Publish",
						("Quit",quit,"<Control>Q")],
					["Edit",
						("Undo", undo, "/z"),
						("Redo", redo, "/^z"),
						("Cut", cut, "/x"),
						("Copy", copy, "/c"),
						("Paste", paste, "/v"),
						"Delete",
						("Preferences",preferences,"")],
					["Timeline",
						("Add Keyframe",add_keyframe,"F6"),
						"Add Blank Keyframe",
						("Add Layer",add_layer,"<Shift><Control>N"),
						("Delete Layer",delete_layer,"<Shift><Control>Delete")],
					["Import",
						("Import to Stage",import_to_stage,"/I"),
						("Import to Library",import_to_library)],
					["Export",
						("Export .swf",export_swf,""),
						"Export HTML5",
						"Export Native Application",
						"Export .sc",
						("Export SVG",export_svg,""),
						"Export Image",
						"Export Video",
						"Export .pdf",
						"Export Animated GIF"],
					["Tools",
						("Execute",run_file,"/\r"),
						("Execute as HTML5",run_html,"/\\")],
					["Modify",
						"Document",
						("Convert to Symbol",convert_to_symbol,"F8"),
						("Send to Back",send_to_back,"<Shift><Control>Down"),
						("Send Backwards",send_backward,"<Control>Down"),
						("Bring Forwards",bring_forward,"<Control>Up"),
						("Bring to Front",bring_to_front,"<Shift><Control>Up")], 
					["Help",
						"Lightningbeam Help",
						"Actionscript Reference",
						("About Lightningbeam...",about)]])


#open("/home/skyler/Desktop/test.sc","w").write(create_sc(root))

if not svlgui.SYSTEM=="android":
	svlgui.main()
else:
	import os
	svlgui.droid.webViewShow('{0}/lightningbeam_ui.html'.format(str(os.curdir)))
	while True:
		result = svlgui.droid.eventWaitFor('pythonevent').result
		svlgui.droid.eventClearBuffer()
		print result
		exec result["data"]

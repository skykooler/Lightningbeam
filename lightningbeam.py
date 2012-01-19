#! /usr/bin/python
# -*- coding:utf-8 -*-
# © 2012 Skyler Lehmkuhl
# Released under the GPLv3. For more information, see gpl.txt.

import os, shutil, tarfile, tempfile, StringIO

#Uncomment to build on OS X
#import objc, AppKit, cPickle

#Uncomment to build on Windows
#import ctypes, ctypes.wintypes, win32print

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

#specify the current version and what version files it can still open
LIGHTNINGBEAM_VERSION = "1.0-alpha1"
LIGHTNINGBEAM_COMPAT = ["1.0-alpha1"]

#Global variables. Used to keep stuff together.
global root
global layers

def update_date():
	return "Tue, January 10, 2012"



def onLoadFrames(self):
	'''for i in range(2000):
		if i%5==0:
			j = box(i*16,0,16,32,svlgui.Color([0.5,0.5,0.5]))
			self.add(j)
		else:
			j = box(i*16,0,16,32,svlgui.Color([1,1,1]))
			self.add(j)'''
def onClickFrame(self, x, y):
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
def onMouseDownGroup(self, x, y):
	self.activelayer.frames[self.activelayer.currentframe].actions = MainWindow.scriptwindow.text
	if svlgui.MODE in [" ", "s"]:
		if self.hitTest(x, y):
			self.clicked = True
	elif svlgui.MODE in ["r", "e", "p"]:
		if svlgui.MODE=="r":
			#I can't remember what the 'c' stands for...
			self.cshape = box(x, y, 0, 0)
		elif svlgui.MODE=="e":
			self.cshape = ellipse(x, y, 0, 0)
		elif svlgui.MODE=="p":
			self.cshape = shape(x, y)
		#self.cshape.rotation = 5
		self.cshape.initx,self.cshape.inity = x, y
		self.add(self.cshape)
		self.cshape.onMouseDown = onMouseDownObj
		self.cshape.onMouseMove = onMouseMoveObj
		self.cshape.onMouseDrag = onMouseDragObj
		self.cshape.onMouseUp = onMouseUpObj
		self.cshape.onKeyDown = onKeyDownObj
		self.clicked = True
		MainWindow.scriptwindow.text = self.activelayer.frames[self.activelayer.currentframe].actions
	elif svlgui.MODE in ["t"]:
		self.ctext = svlgui.Text("Mimimi",x,y)
		self.ctext.onMouseDown = onMouseDownText
		self.ctext.onMouseDrag = onMouseDragText
		self.ctext.onMouseUp = onMouseUpText
		self.add(self.ctext)
		self.ctext = None
	MainWindow.docbox.setvisible(True)
	MainWindow.textbox.setvisible(False)

def onMouseDownObj(self, x, y):
	MainWindow.scriptwindow.text = root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions
	self.clicked = True
	self.initx,self.inity = x-self.x, y-self.y
	if svlgui.MODE == "b":
		self.filled = True
		self.fillcolor = svlgui.FILLCOLOR
def onMouseDownText(self,x,y):
	MainWindow.scriptwindow.text = root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions
	self.clicked = True
	self.initx, self.inity = x-self.x, y-self.y
	MainWindow.docbox.setvisible(False)
	MainWindow.textbox.setvisible(True)
	svlgui.CURRENTTEXT = self.obj
	print "Height", MainWindow.textbox.height
def onMouseDownFrame(self, x, y):
	pass
def onMouseUpGroup(self, x, y):
	self.clicked = False
	if svlgui.MODE in ["r", "e"]:
		self.cshape = None
	elif svlgui.MODE=="p":
		print len(self.cshape.shapedata)
		self.cshape.shapedata = misc_funcs.simplify_shape(self.cshape.shapedata, svlgui.PMODE.split()[-1],1)
		print len(self.cshape.shapedata)
		self.cshape = None
		MainWindow.stage.draw()
def onMouseUpObj(self, x, y):
	self.clicked = False
def onMouseUpText(self, x, y):
	self.clicked = False
def onMouseMoveGroup(self, x, y):
	pass
	#This is for testing rotation. Comment out before any commit!
	#root.rotation+=0.01
def onMouseMoveObj(self, x, y):
	pass
def onMouseDragGroup(self, x, y):
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
def onMouseDragObj(self, x, y):
	self.x = x-self.initx
	self.y = y-self.inity
def onMouseDragText(self, x, y):
	self.x = x-self.initx
	self.y = y-self.inity

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
	
def create_sc(root):
	#retval = ".flash bbox="+str(svlgui.WIDTH)+"x"+str(svlgui.HEIGHT)+" background=#ffffff \
#fps="+str(svlgui.FRAMERATE)+"\n"+root.print_sc()+".end"
	retval = ".flash bbox="+str(svlgui.WIDTH)+"x"+str(svlgui.HEIGHT)+" background=#ffffff \
fps="+str(svlgui.FRAMERATE)+"\n"+"".join([i.print_sc() for i in svlgui.Library])+root.print_sc()+".end"
	return retval
def run_file(self=None):
	global root
	print "RUNNING"
	root.descendItem().activelayer.frames[root.descendItem().activelayer.currentframe].actions = MainWindow.scriptwindow.text
	open(os.getenv('HOME')+"/test.sc", "w").write(create_sc(root))
	svlgui.execute("swfc/swfc_"+svlgui.PLATFORM+" "+os.getenv('HOME')+"/test.sc -o "+os.getenv('HOME')+"/test.swf")
	#TODO: Make this cross-platform compatible
	logloc = os.getenv('HOME')+"/Library/Preferences/Macromedia/Flash Player/Logs/flashlog.txt"
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
			outputtext.text+=logfile.readline()
			outputtext.scroll_bottom()		# this doesn't work
		except:
			pass
	r = misc_funcs.RepeatTimer(0.02, updatetrace, args=[outputtext])
	print dir(outputwin.window)
	r.daemon = True
	r.start()
	if svlgui.PLATFORM=="osx":
		osx_flash_player_loc = "/Applications/Flash\ Player\ Debugger.app"
		success = svlgui.execute("open -a "+osx_flash_player_loc+" "+os.getenv('HOME')+"/test.swf")
		if not success:
			svlgui.alert("Oops! Didn't work. I probably couldn't find your Flash debugger!")
	elif svlgui.PLATFORM=='win32':
		win_flash_player_loc = ""
		svlgui.execute('start '+win_flash_player_loc+" test.swf")
	elif svlgui.PLATFORM.startswith('linux'):
		linux_flash_player_loc = ""
		svlgui.execute('xdg-open '+linux_flash_player_loc+" test.swf")
def create_html5(root):
	retval = "<head>\n\
<style type=\"text/css\">\n\
canvas { \n\
border: none; position:absolute; top:0;left:0;\n\
visibility: hidden; }\n\
</style>\n\
</head>\n\
<body>\n\
<canvas id=\"canvas1\" width="+str(svlgui.WIDTH)+" height="+str(svlgui.HEIGHT)+" ></canvas>\n\
<canvas id=\"canvas2\" width="+str(svlgui.WIDTH)+" height="+str(svlgui.HEIGHT)+"></canvas>\n\
<script>\n\
//Setup\nvar fps = "+str(svlgui.FRAMERATE)+";\n</script>\n\
<script src=\"base.js\">\n\
</script>\n\
<script>"+"".join([i.print_html() for i in svlgui.Library])+root.print_html()+"\n\
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
'''
e=ellipse(100,100,10,10,None)
e.onMouseDown = onMouseDownObj
e.onMouseMove = onMouseMoveObj
e.onMouseDrag = onMouseDragObj
e.onMouseUp = onMouseUpObj
e.onKeyDown = onKeyDownObj
root.add(e)'''




if svlgui.SYSTEM == "gtk":
	overlaywindow = svlgui.OverlayWindow()
	MainWindow = lightningbeam_windows.MainWindow()
elif svlgui.SYSTEM=="osx":
	MainWindow = lightningbeam_windows.MainWindowOSX()
elif svlgui.SYSTEM=="html":
	MainWindow = lightningbeam_windows.MainWindowHTML()
elif svlgui.SYSTEM=="android":
	MainWindow = lightningbeam_windows.MainWindowAndroid()
MainWindow.stage.add(root, 0,0)
svlgui.FOCUS = MainWindow.stage
layers = svlgui.Group(skipl=True)
b = svlgui.Image("media/object_active.png",0,0,True,MainWindow.layerbox,16,1,True)
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
	thetarfile = tarfile.open(fileobj=svlgui.file_dialog("open").open("rb"),mode="r:gz")
	basefile = thetarfile.extractfile("basefile")
	root, svlgui.Library = pickle.load(basefile)
	svlgui.SECURETEMPDIR = tempfile.mkdtemp()
	thetarfile.extractall(path=svlgui.SECURETEMPDIR)
	for i in svlgui.Library:
		if i.type=="Image":
			i.path = svlgui.SECURETEMPDIR+"/"+i.path.split(os.sep)[-1]
			i.set_image(i.path)
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
		thetarfile = tarfile.open(fileobj=svlgui.file_dialog("save").open('wb'),mode="w:gz")
		print thetarfile.name
	else:
		thetarfile = tarfile.open(svlgui.FILE.name,mode="w:gz")
	thetarfile.addfile(tarinfo, StringIO.StringIO(data))
	#Save the path so we can come back here
	lastpath = os.path.abspath(".")
	for i in svlgui.Library:
		if i.type=="Image":
			os.chdir(os.sep.join(i.path.split(os.sep)[:-1]))
			i.path = i.path.split(os.sep)[-1]
			thetarfile.add(i.path.split(os.sep)[-1])
			os.chdir(lastpath)
	thetarfile.close()
	svlgui.FILE = thetarfile
	#thetarfile.close()
def save_file_as(widget=None):
	pass
def import_to_stage(widget=None):
	thefile = svlgui.file_dialog("open",None,["jpg","png","bmp"]).path
	im = svlgui.Image(thefile)
	im.onMouseDown = onMouseDownObj
	im.onMouseMove = onMouseMoveObj
	im.onMouseDrag = onMouseDragObj
	im.onMouseUp = onMouseUpObj
	im.onKeyDown = onKeyDownObj
	root.descendItem().add(im)
	MainWindow.stage.draw()
def import_to_library(widget=None):
	pass

def quit(widget):
	svlgui.quit()
	
	
def add_keyframe(widget=None):
	print "af> ", root.descendItem().activeframe
	root.descendItem().add_frame(True)
	MainWindow.timelinebox.draw()
def add_layer(widget=None):
	root.descendItem().add_layer(root.descendItem()._al)
	layers.add(svlgui.Image("media/object_active.png",0,root.descendItem().layers.index(root.descendItem().activelayer)*32,True,MainWindow.layerbox,16,1,True))
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
		svlgui.ConvertToSymbolWindow(root)
		MainWindow.stage.draw()
		
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
						("Save As", save_file_as,"<Shift><Control>S"),
						"Publish",
						("Quit",quit,"<Control>Q")],
					["Edit",
						"Undo",
						"Redo",
						"Cut",
						"Copy",
						"Paste",
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
						"Export .swf",
						"Export HTML5",
						"Export Native Application",
						"Export .sc",
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

#! /usr/bin/python

import svlgui
import misc_funcs
from misc_funcs import *

class MainWindow:
	def __init__(self):
		self.window = svlgui.Window("Lightningbeam")
		self.window.maximize()
		self.menu = svlgui.Menu(1, [[	"File",
									"New",
									"Open",
									"Open .sc",
									"Save",
									"Save As...",
									["Import...",
										"Import to Stage",
										"Import to Library"],
									["Export...",
										"Export .swf",
										"Export HTML5",
										"Export Native Application",
										"Export .sc",
										"Export Image",
										"Export Video",
										"Export .pdf",
										"Export Animated GIF"],
									"Publish",
									"Quit"],
								["Edit",
									"Undo",
									"Redo",
									"Cut",
									"Copy",
									"Paste",
									"Delete",
									"Preferences"],
								["Timeline",
									"Add Keyframe",
									"Add Blank Keyframe",
									"Add Layer",
									"Delete Current Layer"],
								["Tools",
									"Execute"],
								["Modify",
									"Document",
									"Convert to Symbol",
									"Send to Back",
									"Send Backwards",
									"Bring Forwards",
									"Bring to Front"], 
								["Help",
									"Lightningbeam Help",
									"Actionscript Reference",
									"About Lightningbeam"]])
		

		#self.window.add(self.menu)
		self.hbox1 = svlgui.HBox()
		self.buttonbox = svlgui.ButtonBox(6,2)
		self.buttonbox.buttons[0][0].set_image("media/left_ptr.png")
		self.buttonbox.buttons[0][1].set_image("media/lasso.png")
		self.buttonbox.buttons[1][0].set_image("media/resize.png")
		self.buttonbox.buttons[1][1].set_image("media/text.png")
		self.buttonbox.buttons[2][0].set_image("media/rectangle.png")
		self.buttonbox.buttons[2][1].set_image("media/ellipse.png")
		self.buttonbox.buttons[3][0].set_image("media/curve.png")
		self.buttonbox.buttons[3][1].set_image("media/paintbrush.png")
		self.buttonbox.buttons[4][0].set_image("media/pen.png")
		self.buttonbox.buttons[4][1].set_image("media/paintbucket.png")
		self.buttonbox.buttons[0][0].onPress = select_any
		self.buttonbox.buttons[0][1].onPress = lasso
		self.buttonbox.buttons[1][0].onPress = resize_any
		self.buttonbox.buttons[1][1].onPress = text
		self.buttonbox.buttons[2][0].onPress = rectangle
		self.buttonbox.buttons[2][1].onPress = ellipse
		self.buttonbox.buttons[3][0].onPress = curve
		self.buttonbox.buttons[3][1].onPress = paintbrush
		self.buttonbox.buttons[4][0].onPress = pen
		self.buttonbox.buttons[4][1].onPress = paint_bucket
		self.linebutton = svlgui.Button()
		self.fillbutton= svlgui.Button()
		self.linecanvas = svlgui.Canvas(60, 30)
		self.fillcanvas = svlgui.Canvas(60, 30)
		self.linebutton.set_content(self.linecanvas)
		self.fillbutton.set_content(self.fillcanvas)
		linegroup = svlgui.Layer()
		linegroup.add(box(0,0,100,30,"#cccccc"))
		linegroup.add(box(0,0,30,30,"media/curve.png"))
		lbox = box(35,0,65,30,svlgui.LINECOLOR.rgb)
		linegroup.add(lbox)
		self.linecanvas.add(linegroup,0,0)
		fillgroup = svlgui.Layer()
		fillgroup.add(box(0,0,100,30,"#cccccc"))
		fillgroup.add(box(0,0,30,30,"media/paintbucket.png"))
		fbox = box(35,0,65,30,svlgui.FILLCOLOR.rgb)
		fillgroup.add(fbox)
		self.fillcanvas.add(fillgroup,0,0)
		self.linebutton.onPress = lambda self1: svlgui.ColorSelectionWindow("line",linegroup,self.linecanvas)
		self.fillbutton.onPress = lambda self1: svlgui.ColorSelectionWindow("fill",fillgroup,self.fillcanvas)
		self.buttonbox.add(self.linebutton)
		self.buttonbox.add(self.fillbutton)
		self.hbox1.add(self.buttonbox)
		self.vbox1 = svlgui.VBox(700,-1)
		self.hbox1.add(self.vbox1)
		self.stage = svlgui.Canvas(800,600)
		self.timeline = svlgui.Canvas(2048,100)
		self.timelineref = svlgui.Canvas(128,100)
		self.timelinehbox = svlgui.HBox()
		self.stagesw = svlgui.ScrolledWindow()
		self.timelinesw = svlgui.ScrolledWindow()
		#self.stagesw.add(self.stage)
		#self.timelinesw.add(self.timeline)
		self.timelinehbox.add(self.timelineref)
		self.timelinehbox.add(self.timeline,True,True)
		self.vbox1.add(self.timelinehbox)
		self.vbox1.add(self.stage, True)
		self.vbox2 = svlgui.VBox(-1,100)
		self.actions = svlgui.TextView(True,200,200)
		self.vbox2.add(self.actions)
		self.hbox1.add(self.vbox2)
		self.s1 = svlgui.Shape()
		self.s1.shapedata=[["m",0,0],["l",200,0],["l",200,300],["l",0,300],["l",200,400],["l",0,400]]
		self.s1.filled=True
		group = svlgui.Group(self.s1)
		#self.stage.add(group,23,42)
		#self.stage.add(self.s1,0,0)
		self.window.add(self.hbox1, True)
	

class MainWindowAndroid:
	def __init__(self):
		class stagewrapper:
			def add(self, obj, x, y):
				pass
		self.stage = stagewrapper()
		self.menu = svlgui.Menu(1, [[	"File",
									"New",
									"Open",
									"Open .sc",
									"Save",
									"Save As...",
									["Import...",
										"Import to Stage",
										"Import to Library"],
									["Export...",
										"Export .swf",
										"Export HTML5",
										"Export .sc",
										"Export Image",
										"Export Video",
										"Export .pdf",
										"Export Animated GIF"],
									"Publish",
									"Quit"],
								["Edit",
									"Undo",
									"Redo",
									"Cut",
									"Copy",
									"Paste",
									"Delete",
									"Preferences"],
								["Timeline",
									"Add Keyframe",
									"Add Blank Keyframe"],
								["Tools",
									"Execute"],
								["Modify",
									"Document",
									"Convert to Symbol",
									"Send to Back",
									"Send Backwards",
									"Bring Forwards",
									"Bring to Front"], 
								["Help",
									"Lightningbeam Help",
									"Actionscript Reference",
									"About Lightningbeam"]])
									
class MainWindowOSX:
	def __init__(self):
		self.window = svlgui.Window("Lightningbeam")
		self.menu = svlgui.Menu(True, None)
		self.stage = svlgui.Canvas(800,600)
		misc_funcs.stage = self.stage
		self.layerbox = svlgui.Canvas(128,320)
		self.timelinebox = svlgui.Canvas(2000,320)
		self.frame = svlgui.Frame()
		self.toolbox = svlgui.Grid([svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")])
		self.toolbox.buttons[0][0].set_image("media/left_ptr.png")
		self.toolbox.buttons[0][1].set_image("media/lasso.png")
		self.toolbox.buttons[1][0].set_image("media/resize.png")
		self.toolbox.buttons[1][1].set_image("media/text.png")
		self.toolbox.buttons[2][0].set_image("media/rectangle.png")
		self.toolbox.buttons[2][1].set_image("media/ellipse.png")
		self.toolbox.buttons[3][0].set_image("media/curve.png")
		self.toolbox.buttons[3][1].set_image("media/paintbrush.png")
		self.toolbox.buttons[4][0].set_image("media/pen.png")
		self.toolbox.buttons[4][1].set_image("media/paintbucket.png")
		self.toolbox.buttons[0][0].onPress = select_any
		self.toolbox.buttons[0][1].onPress = lasso
		self.toolbox.buttons[1][0].onPress = resize_any
		self.toolbox.buttons[1][1].onPress = text
		self.toolbox.buttons[2][0].onPress = rectangle
		self.toolbox.buttons[2][1].onPress = ellipse
		self.toolbox.buttons[3][0].onPress = curve
		self.toolbox.buttons[3][1].onPress = paintbrush
		self.toolbox.buttons[4][0].onPress = pen
		self.toolbox.buttons[4][1].onPress = paint_bucket
		self.frame.layout_self(	[self.toolbox,0,None,0,0,"nws",""],
								[self.timelinebox,self.toolbox._int()+148,0,0,None,"new","hv"],
								[self.layerbox,self.toolbox._int(),self.toolbox._int().width+150,0,None,"n","v"],
								[self.stage,self.toolbox._int(),0,self.timelinebox._int()+2,0,"nsew", "hv"])
		self.window.add(self.frame)

# use mainwindowosx, this is just to comment things out
class MainWindowHTML:
	def __init__(self):
		self.window = svlgui.Window("Lightningbeam")
		self.menu = svlgui.Menu(True, None)
		self.stage = svlgui.Canvas(800,600)
		self.layerbox = svlgui.Canvas(128,320)
		self.timelinebox = svlgui.Canvas(2000,320)
		self.frame = svlgui.Frame()
		self.toolbox = svlgui.Grid([svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")],
									[svlgui.Button("------"),svlgui.Button("------")])
		self.toolbox.buttons[0][0].set_image("media/left_ptr.png")
		self.toolbox.buttons[0][1].set_image("media/lasso.png")
		self.toolbox.buttons[1][0].set_image("media/resize.png")
		self.toolbox.buttons[1][1].set_image("media/text.png")
		self.toolbox.buttons[2][0].set_image("media/rectangle.png")
		self.toolbox.buttons[2][1].set_image("media/ellipse.png")
		self.toolbox.buttons[3][0].set_image("media/curve.png")
		self.toolbox.buttons[3][1].set_image("media/paintbrush.png")
		self.toolbox.buttons[4][0].set_image("media/pen.png")
		self.toolbox.buttons[4][1].set_image("media/paintbucket.png")
		self.toolbox.buttons[0][0].onPress = select_any
		self.toolbox.buttons[0][1].onPress = lasso
		self.toolbox.buttons[1][0].onPress = resize_any
		self.toolbox.buttons[1][1].onPress = text
		self.toolbox.buttons[2][0].onPress = rectangle
		self.toolbox.buttons[2][1].onPress = ellipse
		self.toolbox.buttons[3][0].onPress = curve
		self.toolbox.buttons[3][1].onPress = paintbrush
		self.toolbox.buttons[4][0].onPress = pen
		self.toolbox.buttons[4][1].onPress = paint_bucket
		self.frame.layout_self(	[self.toolbox,0,None,0,0,"nws",""],
								[self.timelinebox,148,0,0,None,"new","hv"],
								[self.layerbox,140,150,0,None,"n","v"],
								[self.stage,140,0,2,0,"nsew", "hv"])
		self.window.add(self.frame)
	


if __name__=="__main__":
	a = MainWindow()

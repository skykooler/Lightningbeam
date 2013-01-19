#! /usr/bin/python
# -*- coding:utf-8 -*-
# © 2012 Skyler Lehmkuhl
# Released under the GPLv3. For more information, see gpl.txt.

import svlgui
import math
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
		self.stage = svlgui.Canvas(1200,1100)
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
		try:
			import gtk
			ubuntu = True
		except:
			ubuntu = False
		self.window = svlgui.Window("Lightningbeam")
		self.menu = svlgui.Menu(True, None)
		self.stage = svlgui.Canvas(1200,1100)
		misc_funcs.stage = self.stage
		self.layerbox = svlgui.Canvas(128,320)
		self.timelinebox = svlgui.FramesCanvas(2000,320)
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
		self.toolbox.buttons[5][0].set_image("media/line_color.png")	# TODO: make these canvases
		self.toolbox.buttons[5][1].set_image("media/fill_color.png")
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
		self.toolbox.buttons[5][0].onPress = lambda self1: svlgui.ColorSelectionWindow("line")#,linegroup)#,self.linecanvas)
		self.toolbox.buttons[5][1].onPress = lambda self1: svlgui.ColorSelectionWindow("fill")#,linegroup)#,self.fillcanvas)
		self.toolbox.buttons[0][1]._int().enabled = False
		# self.toolbox.buttons[1][0]._int().enabled = False
		self.toolbox.buttons[3][0]._int().enabled = False
		self.toolbox.buttons[4][0]._int().enabled = False
		self.scriptwindow = svlgui.TextView(code=True)
		self.paintgroup = svlgui.RadioGroup("Draw straight", "Draw smooth", "Draw as inked")
		def setmode(self):
			svlgui.PMODE = self.value
		self.paintgroup.action = setmode
		self.paintbox = svlgui.Frame()
		self.pboptions = svlgui.Label("Paintbrush Options")
		self.paintbox.layout_self([self.pboptions,0,0,0,None,"news",""],
								[self.paintgroup[0],0,0,self.pboptions._int(),None,"new",""],
								[self.paintgroup[1],0,0,self.paintgroup[0]._int(),None,"new",""],
								[self.paintgroup[2],0,0,self.paintgroup[1]._int(),None,"new",""])
								#)
		svlgui.TOOLOPTIONS = {self.paintbox:"p"}
		for i in svlgui.TOOLOPTIONS:
			if svlgui.MODE==svlgui.TOOLOPTIONS[i]:
				i.setvisible(True)
			else:
				i.setvisible(False)
		
		self.docbox = svlgui.Frame()
		self.sizelabel = svlgui.Label("Size: ")
		self.sizebutton = svlgui.Button(" 500 x 500 pixels ")
		def setSize(self):
			w1 = svlgui.SizeWindow()
			self.set_text(" "+str(svlgui.WIDTH)+" x "+str(svlgui.HEIGHT)+" pixels ")
		self.sizebutton.onPress = setSize
		self.publishlabel = svlgui.Label("Publish: ")
		self.publishbutton = svlgui.Button(" Settings... ")
		def publishSettings(self):
			w1 = svlgui.PublishSettingsWindow()
		self.publishbutton.onPress = publishSettings
		self.frameratelabel = svlgui.Label("Framerate: ")
		self.frameratentry = svlgui.TextEntry("50")
		self.frameratentry.set_action(self.set_framerate)
		self.docbox.layout_self( [self.sizelabel,10,None,5,None,"nw", ""],
								[self.sizebutton,self.sizelabel._int(),None,5,None,"nw", ""],
								[self.publishlabel,10,None,self.sizebutton._int(),None,"nw", ""],
								[self.publishbutton,self.publishlabel._int(),None,self.sizebutton._int(),None,"nw", ""],
								[self.frameratelabel,10,None,self.publishbutton._int(),None,"nw", ""],
								[self.frameratentry,self.frameratelabel._int(),None,self.publishbutton._int(),None,"nw", ""])
		self.textbox = svlgui.Frame()
		self.tgroup = svlgui.RadioGroup("Static text", "Dynamic text", "Input text")
		def setmode(self):
			if self.value=="Static text":
				svlgui.CURRENTTEXT.dynamic = False
				self.textvarentry.text = ""
				self.textvarentry.disable()
				self.tinstancename.disable()
				self.tinstancename.text = "<Instance Name>"
				self.tinstancename._int().color = svlgui.Color("#AAAAAA").pygui
			elif self.value=="Input text":
				svlgui.CURRENTTEXT.dynamic = True
				svlgui.CURRENTTEXT.editable = False
				self.textvarentry.enable()
				self.tinstancename.enable()
			else:
				svlgui.CURRENTTEXT.dynamic = True
				svlgui.CURRENTTEXT.editable = True
				self.textvarentry.enable()
				self.tinstancename.enable()
		self.tgroup.action = setmode
		self.tfontlabel =  svlgui.Label("Font:")
		self.tfontbutton = svlgui.Button("Times New Roman")
		self.mlgroup = svlgui.RadioGroup("Single line","Multiline","Multiline no wrap")
		self.fontsizelabel = svlgui.Label("Size:")
		self.fontsizentry = svlgui.TextEntry("16.0")
		self.fontsizentry.set_action(self.editFontSizeText)
		self.fontsizescale = svlgui.Scale(1,4,20)
		self.fontsizescale.set_action(self.editFontSizeScale)
		self.textvarlabel = svlgui.Label("Var:")
		self.textvarentry = svlgui.TextEntry("                          ")
		self.textvarentry.set_action(self.setFontVar)
		self.textvarentry.disable()
		self.tgroup.textvarentry = self.textvarentry
		self.tinstancename = svlgui.TextEntry("<Instance Name>")
		self.tgroup.tinstancename = self.tinstancename
		self.tinstancename.original_color = self.tinstancename._int().color
		self.tinstancename._int().color = svlgui.Color("#aaaaaa").pygui
		self.tinstancename._int().mouse_down = self.darkentinstance
		self.tinstancename.set_action(self.setFontInstanceName)
		self.tinstancename.disable()
		self.thwaccel = svlgui.CheckBox("Draw on top (improves performance under HTML5)")
		self.thwaccel.action = self.setFontHWAccel
		self.textbox.layout_self([self.tgroup[0],10,None,5,None,"nw",""],
								[self.tgroup[1],10,None,self.tgroup[0]._int(),None,"nw",""],
								[self.tgroup[2],10,None,self.tgroup[1]._int(),None,"nw",""],
								[self.tinstancename,10,None,self.tgroup[2]._int(),None,"nw",""],
								[self.tfontlabel,self.tinstancename._int(),None,5,None,"nw",""],
								[self.tfontbutton,self.tfontlabel._int(),None,5,None,"nw",""],
								[self.mlgroup[0],self.tinstancename._int(),None,self.tfontbutton._int(),None,"nw",""],
								[self.mlgroup[1],self.tinstancename._int(),None,self.mlgroup[0]._int(),None,"nw",""],
								[self.mlgroup[2],self.tinstancename._int(),None,self.mlgroup[1]._int(),None,"nw",""],
								[self.fontsizelabel,self.tfontbutton._int(),None,5,None,"nw",""],
								[self.fontsizentry,self.fontsizelabel._int(),None,5,None,"nw",""],
								[self.fontsizescale,self.fontsizentry._int(),None,5,None,"nw",""],
								[self.textvarlabel,self.tfontbutton._int(),None,self.fontsizentry._int()+3,None,"nw",""],
								[self.textvarentry,self.textvarlabel._int(),None,self.fontsizentry._int()+3,None,"nw",""],
								[self.thwaccel,self.tfontbutton._int(),None,self.textvarlabel._int()+3,None,"nw",""])
		self.textvarentry.text=""
		if ubuntu:
			self.frame.layout_self(	[self.toolbox,0,None,0,None,"nw",""],
								#[self.timelinebox,self.toolbox._int()+148,-500,0,None,"new","hv"],
								[self.timelinebox,self.toolbox._int()+148,-500,0,100,"new","hv"],
								[self.layerbox,self.toolbox._int(),self.toolbox._int().width+150,0,100,"n","v"],
								[self.docbox,self.toolbox._int(),0,-200,0,"wse", ""],
								[self.textbox,self.toolbox._int(),0,-200,0,"wse", ""],
								[self.scriptwindow,self.timelinebox._int(),0,0,self.docbox._int(),"nse", "hv"],
								[self.stage,self.toolbox._int(),self.scriptwindow._int(),self.timelinebox._int()+2,self.docbox._int(),"nsew", "hv"],
								[self.paintbox,0,self.stage._int(),self.toolbox._int(),None,"nw","v"] )
		else:
			self.frame.layout_self(	[self.toolbox,0,None,0,None,"nw",""],
								[self.timelinebox,self.toolbox._int()+148,-500,0,None,"new","hv"],
								[self.layerbox,self.toolbox._int(),self.toolbox._int().width+150,0,None,"n","v"],
								[self.docbox,self.toolbox._int(),0,-200,0,"wse", ""],
								[self.textbox,self.toolbox._int(),0,-200,0,"wse", ""],
								[self.scriptwindow,self.timelinebox._int(),0,0,self.docbox._int(),"nse", "hv"],
								[self.stage,self.toolbox._int(),self.scriptwindow._int(),self.timelinebox._int()+2,self.docbox._int(),"nsew", "hv"],
								[self.paintbox,0,self.stage._int(),self.toolbox._int(),None,"nw","v"] )
								#[self.stage,self.paintbox._int(),self.scriptwindow._int(),self.timelinebox._int()+2,0,"nsew", "hv"] )
		self.textbox.setvisible(False)
		self.window.add(self.frame)
		if svlgui.SYSTEM=="osx":
			self.stage._int().become_target();
	
	def set_framerate(self):
		svlgui.FRAMERATE=int(self.frameratentry.text)
		if svlgui.SYSTEM=="osx":
			self.stage._int().become_target();
	def editFontSizeScale(self):
		self.fontsizentry.text = str(int(self.fontsizescale.value**2)*1.0)
		svlgui.CURRENTTEXT.size = int(self.fontsizescale.value**2)*1.0
		self.stage.draw()
	def editFontSizeText(self):
		self.fontsizescale.value = math.sqrt(float(self.fontsizentry.text))
		if svlgui.SYSTEM=="osx":
			self.stage._int().become_target();
		svlgui.CURRENTTEXT.size = int(self.fontsizescale.value**2)*1.0
	def setFontVar(self):
		if self.tgroup.value=="Static text":
			self.tgroup.value="Dynamic text"
		svlgui.CURRENTTEXT.variable = self.textvarentry.text
		if svlgui.SYSTEM=="osx":
			self.stage._int().become_target();
	def setFontInstanceName(self):
		if not self.tinstancename.text.strip() == "":
			svlgui.CURRENTTEXT.iname = self.tinstancename.text
			self.stage._int().become_target();
		else:
			self.tinstancename.text = "<Instance Name>"
			self.tinstancename._int().color = svlgui.Color("#AAAAAA").pygui
			self.stage._int().become_target()
	def darkentinstance(self,*args):
		self.tinstancename._int().color = self.tinstancename.original_color
		if self.tinstancename.text == "<Instance Name>":
			self.tinstancename.text = ""
	def setFontHWAccel(self):
		svlgui.CURRENTTEXT.hwaccel = self.thwaccel.value

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
		self.scriptwindow = svlgui.TextView()
		self.paintgroup = svlgui.RadioGroup("Draw straight", "Draw smooth", "Draw as inked")
		self.paintbox = svlgui.VBox([[svlgui.Label("Paintbrush Options")._int()],[self.paintgroup]])
		self.frame.layout_self(	[self.toolbox,0,None,0,None,"nw",""],
								[self.paintbox,0,None,self.toolbox._int(),0,"nws","v"],
								[self.timelinebox,self.toolbox._int()+148,-500,0,None,"new","hv"],
								[self.layerbox,self.toolbox._int(),self.toolbox._int().width+150,0,None,"n","v"],
								[self.scriptwindow,self.timelinebox._int(),0,0,0,"nse", "hv"],
								[self.stage,self.toolbox._int(),self.scriptwindow._int(),self.timelinebox._int()+2,0,"nsew", "hv"] )
		self.window.add(self.frame)
	


if __name__=="__main__":
	a = MainWindow()

#!/usr/bin/python
# -*- coding: utf-8 -*-

from GUI import Application, ScrollableView, Window, Font, Colors
from pygments import highlight
from pygments.lexers import ActionScriptLexer
from pygments.formatter import Formatter
from pygments.filters import NameHighlightFilter
from pygments.token import Token

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

class EverythingHighlightFilter(NameHighlightFilter):
	def __init__(self, names, tokentype, names2, tokentype2, names3, tokentype3):
		NameHighlightFilter.__init__(self, names=names, tokentype=tokentype)
		self.names2 = names2
		self.tokentype2 = tokentype2
		self.names3 = names3
		self.tokentype3 = tokentype3
	def filter(self, lexer, stream):
	    define = False
	    for ttype, value in stream:
	    	if value in self.names:
	    		ttype = self.tokentype
	    	elif value in self.names2:
	    		ttype = self.tokentype2
	    	elif value in self.names3:
	    		ttype = self.tokentype3
	        yield ttype, value

class PyGUIFormatter(Formatter):
	def __init__(self, **options):
		Formatter.__init__(self, **options)
		self.styles = {}
		for token, style in self.style:
			self.styles[token] = {}
			if style['color']:
				self.styles[token]['color'] = Colors.rgb(*hex2rgb(style['color']))

	def format(self, tokensource, outfile):
		for ttype, value in tokensource:
			self.lines = 0
			self.tabs = 0
			self.cr.gsave()
			if 'color' in self.styles[ttype]:
				self.cr.textcolor = self.styles[ttype]['color']
			else:
				if ttype==Token.Text:
					self.lines+=value.count('\n')
					self.tabs+=value.count('\t')
			self.cr.show_text(value)
			self.cr.stroke()
			self.cr.grestore()
			if self.lines:
				self.cr.rmoveto(-self.cr.current_point[0],self.lines*self.height)
			if self.tabs:
				self.cr.rmoveto(self.tabwidth*self.tabs, 0)
class CodeEditor(ScrollableView):
	def __init__(self):
		ScrollableView.__init__(self)
		self.text = "var a = {b:5, c:[3, 'df \\'']};\n_xscale\nif (this.hitTest(_root._xmouse, root._ymouse, false)) {\n\n\ttrace('hi');\n}"
		self.font = Font('Courier', 16)
		self.selecting = False
		self.lexer = ActionScriptLexer()
		self.cursorpos = 0
		self.scursorpos = 0
		self.formatter = PyGUIFormatter()
		# self.filter = NameHighlightFilter(
		self.filter = EverythingHighlightFilter(
		    names=['trace'],
		    tokentype=Token.Keyword,
		    names2=['_root', '_global'],
		    tokentype2=Token.Name.Builtin,
		    names3=['_alpha', 'blendMode', 'cacheAsBitmap', '_currentframe', '_droptarget', 'enabled', 'filters',
		    		'focusEnabled', '_focusRect', 'forceSmoothing', '_framesloaded', '_height', '_highquality',
		    		'hitArea', '_lockroot', 'menu', 'opaqueBackground', '_parent', '_quality', '_rotation', 'scale9Grid',
		    		'scrollRect', '_soundbuftime', 'tabChildren', 'tabEnabled', 'tabIndex', '_target', 'totalframes',
		    		'trackAsMenu', 'transform', '_url', 'useHandCursor', '_visible', '_width', '_x', '_xmouse',
		    		'_xscale', '_y', '_ymouse', '_yscale'],
		    tokentype3=Token.Name.Variable.Class
		)
		self.extent = (self.width, self.height)
		# Not working - no idea why - disabled to add speed
		self.lexer.add_filter(self.filter)
	def container_resized(self, event):
		self.extent = (self.width, self.height)
		ScrollableView.container_resized(self, event)
	def draw(self, cr, update_rect):
		cr.fillcolor = Colors.rgb(1,1,1)
		cr.fill_rect(update_rect)
		d = self.font.descent
		h = self.font.height
		w = self.font.width(' ')
		cr.font = self.font
		if '\n' in self.text[:self.scursorpos]:
			scw = self.font.width(self.text[self.text.rindex('\n',0,self.scursorpos):self.scursorpos])
		else:
			scw = self.font.width(self.text[:self.scursorpos])
		if '\n' in self.text[:self.cursorpos]:
			cw = self.font.width(self.text[self.text.rindex('\n',0,self.cursorpos):self.cursorpos])
		else:
			cw = self.font.width(self.text[:self.cursorpos])
		selines = self.text[:self.scursorpos].count('\n')+1
		elines = self.text[:self.cursorpos].count('\n')+1
		if self.selecting:
			cr.fillcolor = Colors.rgb(0.5,0.75,1)
			cr.moveto(scw,d+h*selines)
			cr.lineto(scw,-h+h*selines)
			if selines!=elines:
				cr.lineto(self.extent[0],-h+h*selines)
				cr.lineto(self.extent[0],-h+h*elines)
			cr.lineto(cw,-h+h*elines)
			cr.lineto(cw,d+h*elines)
			if selines != elines:
				cr.lineto(0,d+h*elines)
				cr.lineto(0,d+h*selines)
			cr.fill()
			# cr.fill_rect([scw,d+h*elines,cw,-h+h*elines])
			cr.newpath()
		cr.moveto(0,self.font.height)
		self.formatter.cr = cr
		self.formatter.height = self.font.height
		self.formatter.tabwidth = self.font.width('\t')
		highlight(self.text, self.lexer, self.formatter)
		cr.newpath()
		cr.moveto(cw,d+h*elines)
		cr.lineto(cw,-h+h*elines)
		cr.stroke()
		# cr.show_text(self.text)
		# cr.stroke()
	def mouse_down(self, event):
		self.become_target()
		x, y = event.position
		self.selecting = False
		# self.cursorpos = self.text.replace('\n',' ',int(y/self.font.height)-1).find('\n')
		# if self.cursorpos==-1:
		# 	self.cursorpos = len(self.text)-1
		# self.cursorpos+=1
		# self.cursorpos = min(self.cursorpos+int(x/self.font.width(' ')), max(self.text.find('\n',self.cursorpos+1),0) or len(self.text))
		lns = self.text.splitlines()
		self.cursorpos = sum([len(i) for i in lns[:int(y/self.font.height)]])
		try:
			self.cursorpos += min(int(x/self.font.width(' '))+1,lns[int(y/self.font.height)])
		except:
			pass
		self.scursorpos = self.cursorpos
		if int(y/self.font.height):
			self.cursorpos+=1
		self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
	def mouse_drag(self, event):
		x, y = event.position
		self.selecting = True
		lns = self.text.splitlines()
		self.cursorpos = sum([len(i) for i in lns[:int(y/self.font.height)]])
		try:
			self.cursorpos += min(int(x/self.font.width(' '))+1,lns[int(y/self.font.height)])
		except:
			pass
		if int(y/self.font.height):
			self.cursorpos+=1
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
				print 'uni', key
		else:
			key = event.key#.upper()
			print key
		if key == "\b":
			if self.cursorpos>0:
				self.text = self.text[:self.cursorpos-1]+self.text[self.cursorpos:]
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
		self.invalidate_rect([0,0,self.extent[0],self.extent[1]])
class test(Application):
	def __init__(self):
		Application.__init__(self)
		self.make_window()
	def make_window(self):
		win = Window(size = (400,400))
		tex = CodeEditor()
		win.place(tex, left=0, top=0, right=0, bottom=0, sticky='nesw')
		win.show()

if __name__=="__main__":
	test().run()
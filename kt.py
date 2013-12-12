from random import random
from kivy.app import App
from kivy.lang import Builder
from kivy.uix.widget import Widget
from kivy.uix.codeinput import CodeInput
from kivy.uix.tabbedpanel import TabbedPanel
from kivy.uix.button import Button
from kivy.graphics import Color, Ellipse, Line

Builder.load_file("lightningbeam.kv")

class LightningbeamPanel(TabbedPanel):

    pass

class KivyCanvas(Widget):
	def on_touch_down(self, touch):
		print touch.button

class LightningbeamApp(App):

    def build(self):
        return LightningbeamPanel()

if __name__ == '__main__':
    LightningbeamApp().run()
from random import random
from kivy.app import App
from kivy.lang import Builder
from kivy.uix.widget import Widget
from kivy.uix.tabbedpanel import TabbedPanel
from kivy.uix.button import Button
from kivy.graphics import Color, Ellipse, Line

Builder.load_file("lightningbeam.kv")

class Lightningbeam(TabbedPanel):

    pass


class MyPaintApp(App):

    def build(self):
        return Lightningbeam()


if __name__ == '__main__':
    MyPaintApp().run()
from GUI import Font, Window, TextField, Button, application
from testing import say

fancy = Font("Times", 24, ['italic'])

win_num = 0


class TestWindow(Window):
    
    def do_default_action(self):
        say("Default")
    
    def do_cancel_action(self):
        say("Cancel")
    

def show_text(win):
    fields = [win.tf1, win.tf2, win.tf3]
    n = None
    t = application().target
    for i, f in enumerate(fields):
        say("Field %d:" % (i + 1), repr(f.text))
        if f is t:
            n = i + 1
    if n:
        say("Focus: Field %d: Selection = %r" % (n, t.selection))
    else:
        say("No focus")

def select_text(win):
    win.tf2.selection = (7, 11)

def make_window():
    global win_num
    win_num += 1
    win = TestWindow(size = (260, 200), title = "Text fields %d" % (win_num))	
    win.tf1 = TextField(position = (20, 20), width = 200)
    #say("Field 1 Height =", win.tf1.height) ###
    win.tf2 = TextField(position = (20, win.tf1.bottom + 10), width = 200,
        text = "Spam\nGlorious Spam", multiline = True, lines = 2)
    #say("Field 2 Height =", win.tf2.height) ###
    win.tf3 = TextField(position = (20, win.tf2.bottom + 10), width = 200, font = fancy)
    show_but = Button("Show", position = (20, win.tf3.bottom + 20),	
        action = (show_text, win))
    sel_but = Button("Select",
        position = (show_but.right + 5, win.tf3.bottom + 20),
        action = (select_text, win))
    new_but = Button("New",
        position = (sel_but.right + 5, win.tf3.bottom + 20),
        action = make_window)
    win.add(win.tf1)
    win.add(win.tf2)
    win.add(win.tf3)
    win.add(show_but)
    win.add(sel_but)
    win.add(new_but)
    win.height = show_but.bottom + 20
    win.tf1.become_target()
    win.show()
    return win

win = make_window()

instructions = """
There should be a window containing 3 text fields:

1. A single-line text field
2. A 2-line text field with some initial text
3. A single-line field with a large italic font

A. Field 1 should have the initial keyboard focus.

B. Field 2 should allow multi-line editing, the others should not.

C. Tabbing between all fields should work.

D. Cut, Copy, Paste, Clear, Select All commands and their keyboard equivalents
should work. Their menu items should be enabled or disabled as appropriate.

E. Pressing the Enter key on the numeric keypad should print "Default", and
pressing Escape should print "Cancel". In single-line fields, the Return key
on the main keyboard should also print "Default".

F. The Show button should report the contents of each text field and the
selection range of the field having the keyboard focus.

G. The Sel button should select characters 5 to 11 of the second text
field and focus that field. Check that type characters are entered into
the field afterwards.

H. Use New button to create an additional window and ensure that switching
focus between windows and cut/copy/paste between windows works correctly.
"""

say(instructions)

def sigterm(*a):
    raise Exception("SIGTERM")

import signal
signal.signal(signal.SIGTERM, sigterm)

application().run()

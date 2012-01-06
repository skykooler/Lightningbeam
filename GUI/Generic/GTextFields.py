#
#   Python GUI - Text fields - Generic
#

from GUI.Properties import overridable_property
from GUI.Actions import ActionBase, action_property
from GUI import application
from GUI import Control
from GUI import EditCmdHandler

class TextField(Control, ActionBase, EditCmdHandler):
    """A control for entering and editing small amounts of text."""
    
    text = overridable_property('text')
    selection = overridable_property('selection', "Range of text selected.")
    multiline = overridable_property('multiline', "Multiple text lines allowed.")
    password = overridable_property('password', "Display characters obfuscated.")
    enter_action = action_property('enter_action', "Action to be performed "
        "when the Return or Enter key is pressed.")
    escape_action = action_property('escape_action', "Action to be performed "
        "when the Escape key is pressed.")
    
    _may_be_password = True

    #_tabbable = True
    _default_tab_stop = True
    _user_tab_stop_override = False
    _enter_action = 'do_default_action'
    _escape_action = 'do_cancel_action'
    
    _intercept_tab_key = True
    
    def __init__(self, **kwds):
        self._multiline = kwds.pop('multiline')
        Control.__init__(self, **kwds)
    
    def get_multiline(self):
        return self._multiline
    
    def key_down(self, event):
        #print "GTextField.key_down for", self ###
        c = event.char
        if c == '\r':
            if event.key == 'enter' or not self._multiline:
                self.do_enter_action()
                return
        if c == '\x1b':
            self.do_escape_action()
            return
        if c == '\t':
            if self._intercept_tab_key:
                self.pass_event_to_next_handler(event)
                return
        Control.key_down(self, event)		
    
    def setup_menus(self, m):
        Control.setup_menus(self, m)
        EditCmdHandler.setup_menus(self, m)

    def do_enter_action(self):
        self.do_named_action('enter_action')

    def do_escape_action(self):
        self.do_named_action('escape_action')

    def get_text_length(self):
        #  Implementations can override this if they have a more
        #  efficient way of getting the text length.
        return len(self.text)
    
    def get_value(self):
        return self.text
    
    def set_value(self, x):
        self.text = x


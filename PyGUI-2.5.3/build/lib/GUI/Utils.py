#------------------------------------------------------------------------------
#
#		Python GUI - Utilities - PyObjC
#
#------------------------------------------------------------------------------

from math import ceil
from inspect import getmro
from Foundation import NSObject
from AppKit import NSView
from GUI import Event, Globals

def NSMultiClass(name, bases, dic):
    #  Workaround for PyObjC classes not supporting
    #  multiple inheritance properly. Note: MRO is
    #  right to left across the bases.
    main = bases[0]
    slots = list(dic.get('__slots__', ()))
    dic2 = {}
    for mix in bases[1:]:
        for cls in getmro(mix)[::-1]:
            slots2 = cls.__dict__.get('__slots__')
            if slots2:
                for slot in slots2:
                    if slot not in slots:
                        slots.append(slot)
            dic2.update(cls.__dict__)
    dic2.update(dic)
    if slots:
        dic2['__slots__'] = slots
    cls = type(main)(name, (main,), dic2)
    return cls

#------------------------------------------------------------------------------

class PyGUI_Flipped_NSView(NSView):
    #  An NSView with a flipped coordinate system.
    
    def isFlipped(self):
        return True

#------------------------------------------------------------------------------

class PyGUI_NSActionTarget(NSObject):
    #  A shared instance of this class is used as the target of
    #  all action messages from the NSViews of Components. It
    #  performs the action by calling the similarly-named method of
    #  the corresponding Component.
    
    def doAction_(self, ns_sender):
        self.call_method('do_action', ns_sender)
    
    def call_method(self, method_name, ns_sender):
        component = Globals._ns_view_to_component.get(ns_sender)
        if component:
            getattr(component, method_name)()

_ns_action_target = PyGUI_NSActionTarget.alloc().init()

def ns_set_action(ns_control, method_name):
    #  Arrange for the 'action' message of the NSControl to
    #  invoke the indicated method of its associated Component.
    ns_control.setAction_(method_name)
    ns_control.setTarget_(_ns_action_target)

#------------------------------------------------------------------------------

class PyGUI_NS_EventHandler:
    #  Methods to be mixed in with NSView subclasses that are
    #  to relay mouse and keyboard events to a Component.
    #
    #  pygui_component   Component

    def mouseDown_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def mouseUp_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def mouseDragged_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def rightMouseDown_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def rightMouseUp_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def rightMouseDragged_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def otherMouseDown_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def otherMouseUp_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def otherMouseDragged_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def mouseMoved_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def mouseEntered_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def mouseExited_(self, ns_event):
        self._ns_mouse_event(ns_event)

    def keyDown_(self, ns_event):
        #print "PyGUI_NS_EventHandler.keyDown_:", repr(ns_event.characters()), \
        #	"for", object.__repr__(self) ###
        self._ns_other_event(ns_event)

    def keyUp_(self, ns_event):
        #print "PyGUI_NS_EventHandler.keyUp_ for", self ###
        self._ns_other_event(ns_event)

    def _ns_mouse_event(self, ns_event):
        #print "PyGUI_NS_EventHandler._ns_mouse_event" ###
        event = self._ns_mouse_event_to_event(ns_event)
        #print "...sending to", self.pygui_component ###
        component = self.pygui_component
        if component:
            component.handle_event_here(event)
    
    def _ns_mouse_event_to_event(self, ns_event):
        event = Event(ns_event)
        event.position = tuple(self._ns_event_position(ns_event))
        return event

    def _ns_event_position(self, ns_event):
        #print "PyGUI_NS_EventHandler._ns_event_position:", self ###
        #print "...mro =", self.__class__.__mro__ ###
        ns_win_pos = ns_event.locationInWindow()
        return self.convertPoint_fromView_(ns_win_pos, None)

    def _ns_other_event(self, ns_event):
        #print "PyGUI_NS_EventHandler._ns_other_event for", self ###
        event = Event(ns_event)
        component = self.pygui_component
        if component:
            #print "...passing", event.kind, "to", component ###
            component.handle_event(event)

    def acceptsFirstResponder(self):
        ###return True
        return self.pygui_component._ns_accept_first_responder

#	def canBecomeKeyView(self):
#		return self.pygui_component._tab_stop

#------------------------------------------------------------------------------

class PyGUI_NS_ViewBase(PyGUI_NS_EventHandler):
    #  Methods to be mixed in with PyGUI_NSView classes.
    #
    #  pygui_component   ViewBase
    
    __slots__ = ['tracking_rect']
    
#	tracking_rect = None
    
    def becomeFirstResponder(self):
        self.pygui_component.targeted()
        return True
    
    def resignFirstResponder(self):
        self.pygui_component.untargeted()
        return True

    def resetCursorRects(self):
        #print "PyGUI_NS_ViewBase: resetCursorRects" ###
        self.removeCursorRects()
        self.tracking_rect = self.addTrackingRect_owner_userData_assumeInside_(
            self.visibleRect(), self, 0, False)
        self.pygui_component._ns_reset_cursor_rects()

    def removeCursorRects(self):
        #print "PyGUI_NS_ViewBase: removeCursorRects" ###
        tag = getattr(self, 'tracking_rect', None)
        if tag:
            self.removeTrackingRect_(tag)
            self.tracking_rect = None

#------------------------------------------------------------------------------

def ns_size_to_fit(ns_control, padding = (0, 0), height = None):
    # Set size of control to fit its contents, plus the given padding.
    # Height may be overridden, because some controls don't seem to
    # calculate it properly.
    # Auto sizing can result in fractional sizes, which seems to cause
    # problems when NS autoresizing occurs later. So we round the size up
    # to whole numbers of pixels.
    ns_control.sizeToFit()
    w, h = ns_control.frame().size
    pw, ph = padding
    ns_control.setFrameSize_((ceil(w + pw), ceil((height or h) + ph)))

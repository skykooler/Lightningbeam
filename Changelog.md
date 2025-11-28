# 0.8.1-alpha:
Changes:
- Rewrite timeline UI
- Add start screen
- Move audio engine to backend
- Add node editor for audio synthesis
- Add factory presets for instruments
- Add MIDI input support
- Add BPM handling and time signature
- Add metronome
- Add preset layouts for different tasks
- Add video import
- Add animation curves for object properties

# 0.7.14-alpha:
Changes:
- Moving frames can now be undone
- A wait cursor is shown during file loading

Bugfixes:
- Fix clicking on layers bug
- Fix "frame deleting" issue when clicking on frames in a scrolled timeline

# 0.7.13-alpha:
Changes:
- changed file MIME type from text/plain to application/lightningbeam to prevent editor woes on Linux

Bugfixes:
- Port several live fixes to version control
- Fix opening files on macOS
- Improve rendering speed by 10x or more when multiple layers are present

# 0.7.12-alpha:
New features:
- Add "New Window" command
- Enable files to be opened with Lightningbeam

Bugfixes:
- Fix error when an object is deleted from a frame
- Fix parent references being lost
- Fix objects not showing up when imported multiple times

# 0.7.11-alpha:
Bugfixes:
- Fix duplicate objects showing up after grouping
- Fix being unable to scroll audio layers into view

# 0.7.10-alpha:
New features:
- Add proper save/export dialog for web UI

Changes:
- When trying to play an animation and the scrubber is on the last frame, the animation will play from the beginning
- Lightningbeam now uses pointer events instead of mouse events for input, so it can be used with styluses and touchscreens

Bugfixes:
- Fix outlines losing their colors
- Fix audio not opening properly
- Fix delete not working for groups
- Fix undoing group sending shapes to 0,0

# 0.7.9-alpha:
New features:
- MP4 export is now faster and full resolution
- Added WebM export

Changes:
- Files saved in Lightningbeam 0.7.7 or later are now opened by directly parsing the file structure, bypassing the need to replay every action

Bugfixes:
- Fix frame number after exporting video

# 0.7.8-alpha:
Bugfixes:
- Fix mp4 export on macOS
- Fix animations in imported clips not playing grouped object movements correctly

# 0.7.7-alpha:
Bugfixes:
- Fix pasting multiple times
- Hack around broken files

# 0.7.6-alpha:
Bugfixes:
- Fix errors when images are not present in a saved file
- Save images properly

# 0.7.5-alpha:
Bugfixes:
- Fix errors when files refer to now nonexistant frames

# 0.7.4-alpha:
Bugfixes:
- Fix timeline collapse on imported objects

# 0.7.3-alpha:
Bugfixes:
- Fix some files not importing properly

# 0.7.2-alpha:
New features:
- mp4 export (unreliable)
- Added "Recenter View" menu option

Bugfixes:
- Fix layer visibility toggle not functioning
- Fix some files not opening properly

# 0.7.1-alpha:
New features:
- Added "Duplicate keyframe" menu option

Bugfixes:
- Fix importing from file

# 0.7.0-alpha:
New features:
- Keyframes can now have both motion and shape tweens on the same frame

Changes:
- Tweens are now indicated with colored lines
- Tweens are now attached to keyframes rather than the frames in between them

Bugfixes:
- Fix paint bucket coordinates being incorrect inside of movie clips
- Fix paint bucket not working for large shapes and shapes whose internal coordinates crossed 0,0
- Fixed dragging frames breaking tweens
- Fixed logs being inaccessible on macOS
- Fixed right-click causing a menu with "Reload" to appear which would reset the application

# 0.6.18-alpha:
New features:
- Errors and debug messages are now logged to a file

Bugfixes:
- Fix mouse clicks going to wrong locations in color picker and outliner when zoomed

# 0.6.17-alpha:
New features:
- Clicking on an object in the outliner will select it

Bugfixes:
- Fix color picker being unresponsive when color is black
- Fix paintbucket not working in transformed shapes
- Fixed selecting shapes rendering incorrectly
- Fix errors in goToDrame
- Fix being unable to select imported objects
- Fix being unable to open files in some directories
- Fix grouped groups not being copy-pastable

# 0.6.16-alpha:
Bugfixes:
- Fix importing animations not functioning

# 0.6.15-alpha:
Changes:
- Lightningbeam can now open/save files in the Desktop and Downloads folders as well as Documents

Bugfixes:
- Fix old files not importing animations correctly
- Fix app freezing when encountering errors
- Fix clicking on timeline selecting incorrect frame when zoomed

# 0.6.14-alpha:
Changes:
- Make vertex handles semitransparent and always the same visual size

Bugfixes:
- Fix grouped objects losing position on save/load
- Fix copy-pasted objects not being editable

# 0.6.13-alpha:
New features:
- Rotate functionality of transform tool is now working

Bugfixes:
- Fix grouped objects always having a position of 0,0


# 0.6.12-alpha:
Changes:
- Rendering the canvas is better optimized

Bugfixes:
- Prevent double-triggering the keyboard shortcuts on macOS
- Fix line widths not getting saved

# 0.6.11-alpha:
Changes:
- Curve editing is now more predictable

New features:
- "Outliner" pane shows all objects
- Objects can now be imported from .beam files

Bugfixes:
- Fix stuttering during playback

# 0.6.10-alpha:
Changes:
- Selecting and deselecting are now undo-able

New features:
- Layers now have a mute button
- A web version of lightningbeam is available

Bugfixes:
- Fix audio layers not showing up

# 0.6.9-alpha:
Changes:

New features:
- Delete frame is now functional
- Very early support for moving frames. Do not use with motion or shape tweens yet!

Bugfixes:
- Fix motion tween being incorrect after adding a keyframe in the middle of it
- Fix deleted frames still being visible
- Fix timeline playing for too long after removing or moving frames

# 0.6.8-alpha:
Changes:
- Improve stage rendering

New features:
- Add "verbatim" mode to shape drawing
- Add keyboard shortcut for "Add Layer"

Bugfixes:
- Fixed severe bug where all shapes end up on first frame after loading a saved file
- Fixed clicking on frames not updating the stage
- Fixed "Are you sure you want to quit?" message showing up even if the file had just been saved
- Fixed layers in clips preventing playback
- Fixed default filename not getting reset after creating a new file
- Fixed various tools getting confused if the mouse let go of the button outside the window
- Fixed undoing an add layer keeping the removed layer active

# 0.6.7-alpha:
Changes:
- Default configuration is saved between app launches

New features:
- Added "recent files" list on startup
- Added keyboard shortcuts to menus
- Panes can now be split in two

Bugfixes:
- Fixed layer visibility icons not rendering
- New file dialog now closes when opening a file
- Fixed resize cursor showing up between pane header and content

# 0.6.6-alpha:
Changes:
- Rename "Active Object" to "Context"
- Objects display their first frame when not editing them

New features:
- Added scrubber to timeline

Bugfixes:
- Fixed timeline not rendering frame backgrounds when scrolled
- Layers were in reverse order
- Fixed delete keyboard shortcut being triggered when typing in a text box

# 0.6.4-alpha:
Changes:
- "Save As" dialog will use the existing filename as a default

New features:
- Added option to play objects from specific frames
- Added automatic builds for Linux, macOS and Windows

Bugfixes:
- Fixed performance issues with drawing ellipses and rectangles
- Fixed mouse coordinates being incorrect inside a moved object

# 0.6.3-alpha:
Changes:
- "Reset Zoom" renamed to "Actual Size"
- "Fill shape" now defaults to off

New features:
- Paintbucket can now be used on un-filled shapes
- Layers can be hidden
- New color picker
- Navigation breadcrumbs

Bugfixes:
- Audio layers had no names
- Deleting a layer didn't rerender immediately
- Active layer was hard to see
- New layers were not active by default
- "Play" menu item did nothing
- Objects with multiple layers had incorrect bounding boxes

# 0.6.2-alpha:
New features:
- Delete objects and shapes
- Zoom in and out
- Import audio (mp3 only for now)
- Multiple layers
- Edit timelines of groups/objects
- Add "revert" menu option

Bugfixes:
- Timeline did not refresh when creating a new file
- Layer names did not display properly
- Fixed copy and paste breaking saved files
- "Line width" input field was not rendering properly on macOS

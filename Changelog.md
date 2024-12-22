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

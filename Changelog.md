# 1.0.10-alpha:
Changes:
- Cycle recording: highlight a range on the timeline ruler (the cycle strip, shown when looping is armed) and the transport loops it. Recording round the loop turns each pass into a separate take you can choose between afterwards, and recording again over the same region adds more takes rather than stacking a second clip on top. Starting playback from outside the region jumps to its start
- Takes: a "Take N/M" badge on the clip picks which take plays. Splitting a clip lets each half play a different take, so you can comp a part together from the best bits of several passes. Right-click a clip to delete the take that's playing or to delete all the unused ones; double-click a take in the list to rename it
- MIDI cycle recording can either merge every pass into one clip (the default — earlier passes play back as you record, so you can layer a hi-hat over a kick) or keep each pass as a separate take, like audio does. Set it in Preferences → Audio
- Painting: every colour-using tool (brush, paint bucket, eyedropper) now shares one Foreground/Background swatch row, and the eyedropper has an explicit toggle for which of the two it fills
- Painting: the effect brushes (dodge/burn, sponge, blur, smudge, clone stamp, healing brush, pattern stamp) were stuck on a single fixed brush shape. They now each have their own size, strength, hardness and spacing, and can use any brush from the library
- Tablet: stylus barrel buttons now work, with actions you can bind in Preferences (Pan, Eyedropper, or Eraser). Middle-mouse panning added too
- Timeline: raster and text layers get an opacity slider; each layer row now shows only the controls that mean anything for it (volume for audio, opacity for raster/text, both for vector and video), and layers gain a visibility (eye) toggle
- The stage gains undo/redo buttons, sized for touch and tablet use

Bugfixes:
- Splitting an audio clip made it play back at the wrong length (a clip split at 1 second cut off after half a second at 120 BPM)
- Trimmed MIDI clips were the wrong length on the timeline at any tempo other than 60 BPM
- The paint bucket always filled with the background colour, whatever the brush was set to
- Where an eyedropper sample landed (foreground or background) depended on which colour picker you had opened last
- Stylus barrel buttons were silently ignored on Wayland
- The opacity slider on a raster layer actually changed its volume, which means nothing on a raster layer
- Tool cursors lagged behind the pointer while drawing; they're now drawn by the system rather than into the canvas
- Colour picker popups wouldn't close when you dragged on the stage, so drawing left one hanging open, and clicking inside a picker's own hue slider closed it
- The toolbar could not be scrolled when the pane was too short to show every tool

# 1.0.9-alpha:
Bugfixes:
- Fix audio recording placement: a second recording landed at the wrong spot (and clicking/dragging clips was similarly off) at any tempo other than 60 BPM — recordings now start exactly at the playhead at any tempo
- While recording a second audio clip, the live bar showed as a zero-length clip until you stopped; it now grows as you record
- MIDI clips were drawn with the wrong length (their end grew too fast) at tempos other than 120 BPM
- Recording a MIDI clip didn't mark the project as having unsaved changes, so closing or starting a new file didn't prompt to save
- Audio and MIDI recording can now be undone (and redone)

# 1.0.8-alpha:
Changes:
- Mobile/touch UI (experimental, testing only — not built or packaged for mobile yet; enabled on desktop with the LB_MOBILE_UI environment variable): work-in-progress phone-friendly interface with a vertical sliding-window pane stack you drag to reveal panes, a new-file intent picker, a selection inspector sheet, a keyboard-primary music surface, a Focus/Patch node editor, long-press context menus, a command palette, and landscape/orientation support
- Text layers: add and edit text with a chosen font; non-bundled fonts are embedded in the project so it renders on machines that lack them
- Animated GIF export (parallel palette encoding)
- Audio tag metadata (title, artist, album, genre, year, track, comment) is written into exports — ID3v2 for MP3, iTunes/MP4 atoms for AAC, Vorbis comments for FLAC, RIFF INFO for WAV — with sensible defaults (year, artist/album remembered between exports)
- Lossy WebP image export, so the quality control now actually applies
- SVG export now includes text layers, as real font-independent glyph outlines
- Crash recovery: the editor autosaves your work to a recovery file in the background and offers to restore it after an unclean shutdown
- Prompt to save unsaved changes before starting a new file, opening another, or quitting
- Faster saves on painting projects: unchanged raster frames are no longer re-encoded every save

Bugfixes:
- FLAC export previously wrote a WAV file with a .flac extension; it now encodes real (compressed) FLAC
- ProRes 422 export always failed; it now encodes 10-bit 4:2:2 correctly
- Fix VP8 video export with audio (muxed into WebM instead of an incompatible container)
- The WebP "Quality" slider had no effect
- Starting a new file now fully resets the audio engine, so instruments and voices from the previous project no longer linger
- Fix oscillator/synth phase drift over long playback

# 1.0.7-alpha:
Changes:
- HDR video support: PQ/HLG/BT.2020 video is now read correctly (decoded to scene-linear), with a per-document output mode (clip vs highlight rolloff) and 10-bit HDR export (HEVC Main10, PQ or HLG)
- Hardware-accelerated video decode (VAAPI) for both playback and export, including a GPU NV12 preview path; the editor now runs on a shared VAAPI-capable GPU device so decode → composite → encode stay GPU-resident
- SVG import and export for vector layers: export the current frame to .svg, and import .svg as a new vector layer (Ctrl+I)
- Export fit modes: choose Stretch, Letterbox, or Crop when the export resolution's aspect ratio differs from the document (video and image export)
- Videos imported directly and dragged in from the asset library now use the same placement, so they no longer end up with different aspect ratios
- Resizing the document now leaves raster layers untouched; the Info Panel shows the active raster layer's size and a "Layer to document size" button (scale or expand/crop)
- The active raster layer now shows a dashed outline on the canvas
- H.264 export gained a color-range option (Limited/TV or Full/PC)
- Hide/Show Layer now works

Bugfixes:
- Fix sped-up and jerky 4K video playback (frame-index frame cache + request-based seeking)
- Fix washed-out HDR/10-bit video (propagate stream color tags to hardware frames, P010 import)
- Fix the final frame(s) of a clip occasionally failing to render at the end of the stream
- Fix the CPU export color path producing shifted colors on unusual resolutions (BT.709 + honor the chosen range)
- Fix fills occasionally vanishing after a paint-bucket or lasso cut
- Fix silent gaps in exported audio
- Fix black video thumbnails and decoder thrashing
- Fix file-descriptor and GPU-memory leaks on hardware-import error paths
- SVG export now omits hidden layers; SVG import respects gradient opacity
- Fix the macOS/Windows build (hardware export is Linux-only)

# 1.0.6-alpha:
Changes:
- Hardware-accelerated H.264 video export: each frame is rendered and encoded on the GPU (zero-copy VAAPI), roughly 2x faster, with automatic fallback to software encoding when hardware acceleration isn't available (Linux, Intel/AMD only for now)
- Video export now runs on a background thread, so the UI stays responsive during export and edits made while exporting no longer affect the output
- Grouped and nested video clips now composite on the GPU path
- Video is now packed into and streamed from the .beam project container

Bugfixes:
- Fix an export hang when a video's audio track is shorter than the video
- Fix a sample key-range overlap bug in instruments

# 1.0.5-alpha:
Changes:
- Add shape tweens (morph vector geometry between keyframes)
- Add motion tweens for groups and movie clips
- Group geometry and Convert to Movie Clip now work on DCEL vector shapes
- Region/lasso select now cuts the shape and feeds the normal selection, so Group, Convert, Delete and Properties all work from a lasso (hold shift to add to the selection)
- Clip instances now draw on top of a layer's loose shapes
- Add onion skinning for raster and vector layers, with tinted ghosts and settings in the Info Panel
- Images can now fill vector shapes (None / Solid / Gradient / Image fill types)
- Imported images can be placed on the canvas
- Add a raster keyframe timeline UI with explicit keyframe creation; click a keyframe diamond to snap the playhead to it
- Stream audio, video and images to and from the project file instead of holding them in memory, supporting arbitrarily long media
- Persist (and resume) waveforms and video thumbnails in the project file
- Use low-res proxies for fast cold scrubbing of raster frames
- Bound memory use for raster pixels, GPU textures, video frames and decoded images on large projects
- Video export is roughly 4x faster
- Downmix surround video audio to stereo

Bugfixes:
- Fix video export resolution scaling and a post-export UI hang
- Fix gamma handling and improve brush canvas performance
- Fix a save crash on projects with zero or sparse audio
- Fix raster strokes vanishing when committed
- Fix image fill mapping (anchor to the fill's bounding box)
- Fix video thumbnail strip bugs

# 1.0.4-alpha:
Changes:
- Beats are now the canonical time representation (replacing seconds)
- Tempo can now be non-constant (variable BPM)
- All events now have time references in seconds, measures/beats, and frames
- Add piano roll note snapping
- Snap to beats in measures mode
- Add velocity and modulation editing
- Add pitch bend support
- Add automation inputs for audio graphs
- Add automatable volume and pan controls to default instruments
- Add count-in and metronome
- Add drawing tablet input support
- Set default timeline mode based on activity
- Tweaked automation lane appearance
- Double CPU rendering performance by switching to tiny-skia

Bugfixes:
- Fix MIDI track recording previews
- Fix timeline elements not updating on BPM changes

# 1.0.3-alpha:
Changes:
- Add gradient support to vector graphics
- Add "frames" timeline mode
- Reduce CPU usage at idle
- Allow group tracks' audio node graphs to be edited

Bugfixes:
- Support Vello CPU fallback on systems with older GPUs

# 1.0.2-alpha:
Changes:
- All vector shapes on a layer go into a unified shape rather than separate shapes
- Keyboard shortcuts are now user-configurable
- Added webcam support in video editor
- Background can now be transparent
- Video thumbnails are now displayed on the clip
- Virtual keyboard, piano roll and node editor now have a quick switcher
- Add electric guitar preset
- Layers can now be grouped
- Layers can be reordered by dragging
- Added VU meters to audio layers and mix
- Added raster image editing
- Added brush, airbrush, dodge/burn, sponge, pattern stamp, healing brush, clone stamp, blur/sharpen, magic wand and quick select tools
- Added support for MyPaint .myb brushes
- UI now uses CSS styling to support future user styles
- Added image export

Bugfixes:
- Toolbar now only shows tools that can be used on the current layer
- Fix NAM model loading
- Fix menu width and mouse following
- Export dialog now remembers the previous export filename

# 1.0.1-alpha:
Changes:
- Added real-time amp simulation via NAM
- Added beat mode to the timeline
- Changed shape drawing from making separate shapes to making shapes in the layer using a DCEL graph
- Licensed under GPLv3
- Added snapping for vector editing
- Added organ instrument and vibrato node

Bugfixes:
- Fix preset loading not updating node graph editor
- Fix stroke intersections not splitting strokes
- Fix paint bucket fill not attaching to existing strokes

# 1.0.0-alpha:
Changes:
- New native GUI built with egui + wgpu (replaces Tauri/web frontend)
- GPU-accelerated canvas with vello rendering
- MIDI input and node-based audio graph improvements
- Factory instrument presets
- Video import and high performance playback

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

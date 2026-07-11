# Lightningbeam TODO

## Known Issues (Rust)

### Animation: Tweens are broken — LOW PRIORITY
- Shape/vector interpolation between keyframes, and the `tween_after` behavior on
  keyframes, don't work correctly in the current app. Needs investigation + fix.
  Not urgent — revisit later.

## Backlog / Feature ideas

### Animation curve enhancements
- [ ] Extrapolation modes, separate for start vs end: hold (default), extend, repeat, decay
- [ ] Position / scale / rotation animation curves for shapes
- [ ] Shape morphing / tweening between keyframes

### Keyframing behavior
- [ ] User preference for keyframing when editing objects:
  - Auto-keyframe (current default): create/update keyframe at current time
  - Edit previous (Flash-style): update most recent keyframe before current time
  - Ephemeral (Blender-style): changes don't persist without manual keyframe
  - Optional modifier key (e.g. Shift) to toggle modes

### Shape ordering
- [ ] Bring Forward / Send Backward / Bring to Front / Send to Back menu options

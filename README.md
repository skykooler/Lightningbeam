# Lightningbeam

An open-source vector animation tool built with Tauri. A spiritual successor to Macromedia Flash 8 / Adobe Animate.

[![Version](https://img.shields.io/badge/version-0.7.14--alpha-orange)](https://github.com/skykooler/Lightningbeam/releases)

## Overview

Lightningbeam is a cross-platform vector animation application for creating keyframe-based animations and interactive content. Originally started in 2010 as an open-source alternative to Adobe Flash, the project has been rewritten using modern web technologies (JavaScript/Canvas) with a Tauri-based native desktop application wrapper.

## Current Status

**⚠️ Alpha Software**: Lightningbeam is in active development and not yet feature-complete. The codebase is currently undergoing significant refactoring, particularly the timeline system which is being migrated from frame-based to curve-based animation.

Current branch `new_timeline` implements a major timeline redesign inspired by GarageBand, featuring hierarchical tracks and animation curve visualization.

## Features

Current functionality includes:

- **Vector Drawing Tools**: Pen, brush, line, rectangle, ellipse, polygon tools
- **Keyframe Animation**: Timeline-based animation with interpolation
- **Shape Tweening**: Morph between different vector shapes
- **Motion Tweening**: Smooth object movement with curve-based interpolation
- **Layer System**: Multiple layers with visibility controls
- **Hierarchical Objects**: Group objects and edit nested timelines
- **Audio Support**: Import MP3 audio files
- **Video Export**: Export animations as MP4 or WebM
- **Transform Tools**: Move, rotate, and scale objects
- **Color Tools**: Color picker, paint bucket with flood fill
- **Undo/Redo**: Full history management
- **Copy/Paste**: Duplicate objects and keyframes

## Installation

### Pre-built Releases

Download the latest release for your platform from the [Releases page](https://github.com/skykooler/Lightningbeam/releases).

Supported platforms:
- Linux (AppImage, .deb, .rpm)
- macOS
- Windows
- Web (limited functionality)

### Building from Source

**Prerequisites:**
- [pnpm](https://pnpm.io/) package manager
- [Rust](https://rustup.rs/) toolchain (installed automatically by Tauri)
- Platform-specific dependencies for Tauri (see [Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites))

**Build steps:**

```bash
# Clone the repository
git clone https://github.com/skykooler/Lightningbeam.git
cd Lightningbeam

# Install dependencies
pnpm install

# Run in development mode
pnpm tauri dev

# Build for production
pnpm tauri build
```

**Note for Linux users:** `pnpm tauri dev` works on any distribution, but `pnpm tauri build` currently only works on Ubuntu due to limitations in Tauri's AppImage generation. If you're on a non-Ubuntu distro, you can build in an Ubuntu container/VM or use the development mode instead.

## Quick Start

1. Launch Lightningbeam
2. Create a new file (File → New)
3. Select a drawing tool from the toolbar
4. Draw shapes on the canvas
5. Create keyframes on the timeline to animate objects
6. Use motion or shape tweens to interpolate between keyframes
7. Export your animation (File → Export → Video)

## File Format

Lightningbeam uses the `.beam` file extension. Files are stored in JSON format and contain all project data including vector shapes, keyframes, layers, and animation curves.

**Note**: The file format specification is not yet documented and may change during development.

## Known Limitations

### Platform-Specific Issues

- **Linux**: Pinch-to-zoom gestures zoom the entire window instead of individual canvases. This is a [known Tauri/GTK WebView limitation](https://github.com/tauri-apps/tauri/discussions/3843) with no current workaround.
- **macOS**: Limited testing; some platform-specific bugs may exist.
- **Windows**: Minimal testing; application has been confirmed to run but may have undiscovered issues.

### Web Version Limitations

The web version has several limitations compared to desktop:
- Restricted file system access
- Keyboard shortcut conflicts with browser
- Higher audio latency
- No native file association
- Memory limitations with video export

### General Limitations

- The current timeline system is being replaced; legacy frame-based features may be unstable
- Many features and optimizations are still in development
- Performance benchmarking has not been completed
- File format may change between versions

## Contributing

Contributions are currently limited while the codebase undergoes restructuring. Once the timeline refactor is complete and the code is better organized, the project will be more open to external contributions.

If you encounter bugs or have feature requests, please open an issue on GitHub.

## Credits

Lightningbeam is built with:
- [Tauri](https://tauri.app/) - Desktop application framework
- [FFmpeg](https://ffmpeg.org/) - Video encoding/decoding
- Various JavaScript libraries for drawing, compression, and utilities

## License

**License not yet determined.** The author is considering the MIT License for maximum simplicity and adoption. Contributors should await license clarification before submitting code.

---

**Repository**: https://github.com/skykooler/Lightningbeam
**Version**: 0.7.14-alpha
**Status**: Active Development

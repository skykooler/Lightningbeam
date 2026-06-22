# Contributing to Lightningbeam

Thank you for your interest in contributing to Lightningbeam! This document provides guidelines and instructions for setting up your development environment and contributing to the project.

## Table of Contents

- [Development Setup](#development-setup)
- [Building the Project](#building-the-project)
- [Project Structure](#project-structure)
- [Making Changes](#making-changes)
- [Code Style](#code-style)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Getting Help](#getting-help)

## Development Setup

### Prerequisites

- **Rust**: Install via [rustup](https://rustup.rs/) (stable toolchain)
- **System dependencies** (Linux):
  - ALSA development files: `libasound2-dev`
  - For Ubuntu/Debian: `sudo apt install libasound2-dev pkg-config`
  - For Arch/Manjaro: `sudo pacman -S alsa-lib`
- **FFmpeg**: Required for video encoding/decoding
  - Ubuntu/Debian: `sudo apt install ffmpeg libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev pkg-config clang`
  - Arch/Manjaro: `sudo pacman -S ffmpeg`

### Clone and Build

```bash
# Clone the repository (GitHub)
git clone https://github.com/skykooler/lightningbeam.git
# Or from Gitea
git clone https://git.skyler.io/skyler/lightningbeam.git

cd lightningbeam

# Build the Rust UI editor (current focus)
cd lightningbeam-ui
cargo build

# Run the editor
cargo run
```

**Note**: The project is hosted on both GitHub and Gitea (git.skyler.io). You can use either for cloning and submitting pull requests.

## Building the Project

### Workspace Structure

The project consists of multiple Rust workspaces:

1. **lightningbeam-ui** (current focus) - Pure Rust UI application
   - `lightningbeam-editor/` - Main editor application
   - `lightningbeam-core/` - Core data models and business logic

2. **daw-backend** - Audio engine (standalone crate)

3. **Root workspace** (legacy) - Contains Tauri backend and benchmarks

### Build Commands

```bash
# Build the editor (from lightningbeam-ui/)
cargo build

# Build with optimizations (faster runtime)
cargo build --release

# Check just the audio backend
cargo check -p daw-backend

# Build the audio backend separately
cd ../daw-backend
cargo build
```

### Debug Builds and Audio Performance

The audio engine runs on a real-time thread with strict timing constraints (~5.8ms at 44.1kHz). To maintain performance in debug builds, the audio backend is compiled with optimizations even in debug mode:

```toml
# In lightningbeam-ui/Cargo.toml
[profile.dev.package.daw-backend]
opt-level = 2
```

This is already configured—no action needed.

### Debug Flags

Enable audio diagnostics with:
```bash
DAW_AUDIO_DEBUG=1 cargo run
```

This prints timing information, buffer sizes, and overrun warnings to help debug audio issues.

## Project Structure

```
lightningbeam-2/
├── lightningbeam-ui/           # Rust UI workspace (current)
│   ├── lightningbeam-editor/   # Main application
│   │   └── src/
│   │       ├── main.rs         # Entry point
│   │       ├── panes/          # UI panes (stage, timeline, etc.)
│   │       └── tools/          # Drawing and editing tools
│   └── lightningbeam-core/     # Core data model
│       └── src/
│           ├── document.rs     # Document structure
│           ├── clip.rs         # Clips and instances
│           ├── action.rs       # Undo/redo system
│           └── tools.rs        # Tool system
├── daw-backend/                # Audio engine
│   └── src/
│       ├── lib.rs              # Audio system setup
│       ├── audio/
│       │   ├── engine.rs       # Audio callback
│       │   ├── track.rs        # Track management
│       │   └── project.rs      # Project state
│       └── effects/            # Audio effects
└── src/                        # Legacy JavaScript frontend (browser-only)
```

## Making Changes

### Branching Strategy

- `main` - Main development branch
- Feature branches - Create from `main` for new features

### Before You Start

1. Check existing issues or create a new one to discuss your change
2. Make sure you're on the latest `main` branch:
   ```bash
   git checkout main
   git pull origin main
   ```
3. Create a feature branch:
   ```bash
   git checkout -b feature/your-feature-name
   ```

## Code Style

### Rust Style

- Follow standard Rust formatting: `cargo fmt`
- Check for common issues: `cargo clippy`
- Use meaningful variable names
- Add comments for non-obvious code
- Keep functions focused and reasonably sized

### Key Patterns

#### Pane ID Salting
When implementing new panes, **always salt egui IDs** with the node path to avoid collisions when users add multiple instances of the same pane:

```rust
ui.horizontal(|ui| {
    ui.label("My Widget");
}).id.with(&node_path); // Salt with node path
```

#### Splitting Borrows with `std::mem::take`
When you need to split borrows from a struct, use `std::mem::take`:

```rust
let mut clips = std::mem::take(&mut self.clips);
// Now you can borrow other fields while processing clips
```

#### Two-Phase Dispatch
Panes register handlers during render, execution happens after:

```rust
// During render
shared_state.register_action(Box::new(MyAction { ... }));

// After all panes rendered
for action in shared_state.pending_actions.drain(..) {
    action.execute(&mut document);
}
```

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Test specific package
cargo test -p lightningbeam-core
cargo test -p daw-backend

# Run with output
cargo test -- --nocapture
```

### Audio Testing

Test audio functionality:
```bash
# Run with audio debug output
DAW_AUDIO_DEBUG=1 cargo run

# Check for audio dropouts or timing issues in the console output
```

## Submitting Changes

### Before Submitting

1. **Format your code**: `cargo fmt --all`
2. **Run clippy**: `cargo clippy --all-targets --all-features`
3. **Run tests**: `cargo test --all`
4. **Test manually**: Build and run the application to verify your changes work
5. **Write clear commit messages**: Describe what and why, not just what

### Commit Message Format

```
Short summary (50 chars or less)

More detailed explanation if needed. Wrap at 72 characters.
Explain the problem this commit solves and why you chose
this solution.

- Bullet points are fine
- Use present tense: "Add feature" not "Added feature"
```

### Pull Request Process

1. Push your branch to GitHub or Gitea
2. Open a pull request against `main` branch
   - GitHub: https://github.com/skykooler/lightningbeam
   - Gitea: https://git.skyler.io/skyler/lightningbeam
3. Provide a clear description of:
   - What problem does this solve?
   - How does it work?
   - Any testing you've done
   - Screenshots/videos if applicable (especially for UI changes)
4. Address review feedback
5. Once approved, a maintainer will merge your PR

### PR Checklist

- [ ] Code follows project style (`cargo fmt`, `cargo clippy`)
- [ ] Tests pass (`cargo test`)
- [ ] New code has appropriate tests (if applicable)
- [ ] Documentation updated (if needed)
- [ ] Commit messages are clear
- [ ] PR description explains the change

## Getting Help

- **Issues**: Check issues on [GitHub](https://github.com/skykooler/lightningbeam/issues) or [Gitea](https://git.skyler.io/skyler/lightningbeam/issues) for existing discussions
- **Documentation**: See `ARCHITECTURE.md` and `docs/` folder for technical details
- **Questions**: Open a discussion or issue with the "question" label on either platform

## Additional Resources

- [ARCHITECTURE.md](ARCHITECTURE.md) - System architecture overview
- [docs/AUDIO_SYSTEM.md](docs/AUDIO_SYSTEM.md) - Audio engine details
- [docs/UI_SYSTEM.md](docs/UI_SYSTEM.md) - UI and pane system

## License

By contributing, you agree that your contributions will be licensed under the same license as the project.

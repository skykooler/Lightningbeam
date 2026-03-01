# Building Lightningbeam

This guide provides detailed instructions for building Lightningbeam on different platforms, including dependency installation, troubleshooting, and advanced build configurations.

## Table of Contents

- [Quick Start](#quick-start)
- [Platform-Specific Instructions](#platform-specific-instructions)
- [Dependencies](#dependencies)
- [Build Configurations](#build-configurations)
- [Troubleshooting](#troubleshooting)
- [Development Builds](#development-builds)

## Quick Start

```bash
# Clone the repository
git clone https://github.com/skykooler/lightningbeam.git
cd lightningbeam

# Initialize submodules (including nested ones required by nam-ffi)
git submodule update --init --recursive

cd lightningbeam-ui

# Build and run
cargo build
cargo run
```

## Platform-Specific Instructions

### Linux

#### Ubuntu/Debian

**Important**: Lightningbeam requires FFmpeg 8, which may not be in the default repositories.

```bash
# Install basic dependencies
sudo apt update
sudo apt install -y \
    build-essential \
    pkg-config \
    libasound2-dev \
    clang \
    libclang-dev

# Install FFmpeg 8 from PPA (Ubuntu)
sudo add-apt-repository ppa:ubuntuhandbook1/ffmpeg7
sudo apt update
sudo apt install -y \
    ffmpeg \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libswresample-dev

# Verify FFmpeg version (should be 8.x)
ffmpeg -version

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cd lightningbeam-ui
cargo build --release
```

**Note**: If the PPA doesn't provide FFmpeg 8, you may need to compile FFmpeg from source or find an alternative PPA. See [FFmpeg Issues](#ffmpeg-issues) for details.

#### Arch Linux/Manjaro

```bash
# Install system dependencies
sudo pacman -S --needed \
    base-devel \
    rust \
    alsa-lib \
    ffmpeg \
    clang

# Build
cd lightningbeam-ui
cargo build --release
```

#### Fedora/RHEL

```bash
# Install system dependencies
sudo dnf install -y \
    gcc \
    gcc-c++ \
    make \
    pkg-config \
    alsa-lib-devel \
    ffmpeg \
    ffmpeg-devel \
    clang \
    clang-devel

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cd lightningbeam-ui
cargo build --release
```

### macOS

```bash
# Install Homebrew if needed
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install dependencies
brew install rust ffmpeg pkg-config

# Build
cd lightningbeam-ui
cargo build --release
```

**Note**: macOS uses CoreAudio for audio I/O (via cpal), so no additional audio libraries are needed.

### Windows

#### Using Visual Studio

1. Install [Visual Studio 2022](https://visualstudio.microsoft.com/) with "Desktop development with C++" workload
2. Install [Rust](https://rustup.rs/)
3. Install [FFmpeg](https://ffmpeg.org/download.html#build-windows):
   - Download a shared build from https://www.gyan.dev/ffmpeg/builds/
   - Extract to `C:\ffmpeg`
   - Add `C:\ffmpeg\bin` to PATH
   - Set environment variables:
     ```cmd
     set FFMPEG_DIR=C:\ffmpeg
     set PKG_CONFIG_PATH=C:\ffmpeg\lib\pkgconfig
     ```

4. Build:
   ```cmd
   cd lightningbeam-ui
   cargo build --release
   ```

#### Using MSYS2/MinGW

```bash
# In MSYS2 shell
pacman -S mingw-w64-x86_64-rust \
          mingw-w64-x86_64-ffmpeg \
          mingw-w64-x86_64-pkg-config

cd lightningbeam-ui
cargo build --release
```

**Note**: Windows uses WASAPI for audio I/O (via cpal), which is built into Windows.

## Dependencies

### Required Dependencies

#### Rust Toolchain
- **Version**: Stable (1.70+)
- **Install**: https://rustup.rs/
- **Components**: Default installation includes everything needed

#### Audio I/O (ALSA on Linux)
- **Ubuntu/Debian**: `libasound2-dev`
- **Arch**: `alsa-lib`
- **Fedora**: `alsa-lib-devel`
- **macOS**: CoreAudio (built-in)
- **Windows**: WASAPI (built-in)

#### FFmpeg
**Version Required**: FFmpeg 8.x

Required for video encoding/decoding. Note that many distribution repositories may have older versions.

- **Ubuntu/Debian**: Use PPA for FFmpeg 8 (see [Ubuntu/Debian instructions](#ubuntudebian))
- **Arch**: `ffmpeg` (usually up-to-date)
- **Fedora**: `ffmpeg ffmpeg-devel` (check version with `ffmpeg -version`)
- **macOS**: `brew install ffmpeg` (Homebrew usually has latest)
- **Windows**: Download FFmpeg 8 from https://ffmpeg.org/download.html

#### Build Tools
- **Linux**: `build-essential` (Ubuntu), `base-devel` (Arch)
- **macOS**: Xcode Command Line Tools (`xcode-select --install`)
- **Windows**: Visual Studio with C++ tools or MinGW

#### pkg-config
Required for finding system libraries.

- **Linux**: Usually included with build tools
- **macOS**: `brew install pkg-config`
- **Windows**: Included with MSYS2/MinGW, or use vcpkg

### Optional Dependencies

#### GPU Drivers
Vello requires a GPU with Vulkan (Linux/Windows) or Metal (macOS) support:

- **Linux Vulkan**:
  - NVIDIA: Install proprietary drivers
  - AMD: `mesa-vulkan-drivers` (Ubuntu) or `vulkan-radeon` (Arch)
  - Intel: `mesa-vulkan-drivers` (Ubuntu) or `vulkan-intel` (Arch)

- **macOS Metal**: Built-in (macOS 10.13+)

- **Windows Vulkan**:
  - Usually included with GPU drivers
  - Manual install: https://vulkan.lunarg.com/

## Build Configurations

### Release Build (Optimized)

```bash
cargo build --release
```

- Optimizations: Level 3
- LTO: Enabled
- Debug info: None
- Build time: Slower (~5-10 minutes)
- Runtime: Fast

Binary location: `target/release/lightningbeam-editor`

### Debug Build (Default)

```bash
cargo build
```

- Optimizations: Level 1 (Level 2 for audio code)
- LTO: Disabled
- Debug info: Full
- Build time: Faster (~2-5 minutes)
- Runtime: Slower (but audio is still optimized)

Binary location: `target/debug/lightningbeam-editor`

**Note**: Audio code is always compiled with `opt-level = 2` even in debug builds to meet real-time deadlines. This is configured in `lightningbeam-ui/Cargo.toml`:

```toml
[profile.dev.package.daw-backend]
opt-level = 2
```

### Check Without Building

Quickly check for compilation errors without producing binaries:

```bash
cargo check
```

Useful for rapid feedback during development.

### Build Specific Package

```bash
# Check only the audio backend
cargo check -p daw-backend

# Build only the core library
cargo build -p lightningbeam-core
```

## Troubleshooting

### Submodule / CMake Issues

#### "does not contain a CMakeLists.txt file" (RTNeural or math_approx)

**Cause**: The `vendor/NeuralAudio` submodule has its own nested submodules (`deps/RTNeural`, `deps/math_approx`) that weren't initialized. A plain `git submodule update --init` only initializes top-level submodules.

**Solution**: Use `--recursive` to initialize all nested submodules:
```bash
git submodule update --init --recursive
```

Or, if the top-level submodule is already checked out:
```bash
cd vendor/NeuralAudio
git submodule update --init
```

### Audio Issues

#### "ALSA lib cannot find card" or similar errors

**Solution**: Install ALSA development files:
```bash
# Ubuntu/Debian
sudo apt install libasound2-dev

# Arch
sudo pacman -S alsa-lib
```

#### Audio dropouts or crackling

**Symptoms**: Console shows "Audio overrun" or timing warnings.

**Solutions**:
1. Increase buffer size in `daw-backend/src/lib.rs` (default: 256 frames)
2. Enable audio debug logging:
   ```bash
   DAW_AUDIO_DEBUG=1 cargo run
   ```
3. Make sure audio code is optimized (check `Cargo.toml` profile settings)
4. Close other audio applications

#### "PulseAudio" or "JACK" errors in container

**Note**: This is expected in containerized environments without audio support. These errors don't occur on native systems.

### FFmpeg Issues

#### "Could not find FFmpeg libraries" or linking errors

**Version Check First**:
```bash
ffmpeg -version
# Should show version 8.x
```

**Linux**:
```bash
# Ubuntu/Debian - requires FFmpeg 8 from PPA
sudo add-apt-repository ppa:ubuntuhandbook1/ffmpeg7
sudo apt update
sudo apt install libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev

# Arch (usually has latest)
sudo pacman -S ffmpeg

# Check installation
pkg-config --modversion libavcodec
# Should show 61.x or higher (FFmpeg 8)
```

If the PPA doesn't work or doesn't have FFmpeg 8, you may need to compile from source:
```bash
# Download and compile FFmpeg 8
wget https://ffmpeg.org/releases/ffmpeg-8.0.tar.xz
tar xf ffmpeg-8.0.tar.xz
cd ffmpeg-8.0
./configure --enable-shared --disable-static
make -j$(nproc)
sudo make install
sudo ldconfig
```

**macOS**:
```bash
brew install ffmpeg
export PKG_CONFIG_PATH="/opt/homebrew/opt/ffmpeg/lib/pkgconfig:$PKG_CONFIG_PATH"
```

**Windows**:
Set environment variables:
```cmd
set FFMPEG_DIR=C:\path\to\ffmpeg
set PKG_CONFIG_PATH=C:\path\to\ffmpeg\lib\pkgconfig
```

#### "Unsupported codec" or video not playing

Make sure FFmpeg was compiled with the necessary codecs:
```bash
ffmpeg -codecs | grep h264  # Check for H.264
ffmpeg -codecs | grep vp9   # Check for VP9
```

### GPU/Rendering Issues

#### Black screen or no rendering

**Check GPU support**:
```bash
# Linux - check Vulkan
vulkaninfo | grep deviceName

# macOS - Metal is built-in on 10.13+
system_profiler SPDisplaysDataType
```

**Solutions**:
1. Update GPU drivers
2. Install Vulkan runtime (Linux)
3. Check console for wgpu errors

#### "No suitable GPU adapter found"

This usually means missing Vulkan/Metal support.

**Linux**: Install Vulkan drivers (see [Optional Dependencies](#optional-dependencies))

**macOS**: Requires macOS 10.13+ (Metal support)

**Windows**: Update GPU drivers

### Build Performance

#### Slow compilation times

**Solutions**:
1. Use `cargo check` instead of `cargo build` during development
2. Enable incremental compilation (enabled by default)
3. Use `mold` linker (Linux):
   ```bash
   # Install mold
   sudo apt install mold  # Ubuntu 22.04+

   # Use mold
   mold -run cargo build
   ```
4. Increase parallel jobs:
   ```bash
   cargo build -j 8  # Use 8 parallel jobs
   ```

#### Out of memory during compilation

**Solution**: Reduce parallel jobs:
```bash
cargo build -j 2  # Use only 2 parallel jobs
```

### Linker Errors

#### "undefined reference to..." or "cannot find -l..."

**Cause**: Missing system libraries.

**Solution**: Install all dependencies listed in [Platform-Specific Instructions](#platform-specific-instructions).

#### Windows: "LNK1181: cannot open input file"

**Cause**: FFmpeg libraries not found.

**Solution**:
1. Download FFmpeg shared build
2. Set `FFMPEG_DIR` environment variable
3. Add FFmpeg bin directory to PATH

## Development Builds

### Enable Audio Debug Logging

```bash
DAW_AUDIO_DEBUG=1 cargo run
```

Output includes:
- Buffer sizes
- Average/worst-case processing times
- Audio overruns/underruns
- Playhead position updates

### Disable Optimizations for Specific Crates

Edit `lightningbeam-ui/Cargo.toml`:

```toml
[profile.dev.package.my-crate]
opt-level = 0  # No optimizations
```

**Warning**: Do not disable optimizations for `daw-backend` or audio-related crates, as this will cause audio dropouts.

### Build with Specific Features

```bash
# Build with all features
cargo build --all-features

# Build with no default features
cargo build --no-default-features
```

### Clean Build

Remove all build artifacts and start fresh:

```bash
cargo clean
cargo build
```

Useful when dependencies change or build cache becomes corrupted.

### Cross-Compilation

Cross-compiling is not currently documented but should be possible using `cross`:

```bash
cargo install cross
cross build --target x86_64-unknown-linux-gnu
```

See [cross documentation](https://github.com/cross-rs/cross) for details.

## Running Tests

```bash
# Run all tests
cargo test

# Run tests for specific package
cargo test -p lightningbeam-core

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_name
```

## Building Documentation

Generate and open Rust API documentation:

```bash
cargo doc --open
```

This generates HTML documentation from code comments and opens it in your browser.

## Next Steps

After building successfully:

- See [CONTRIBUTING.md](../CONTRIBUTING.md) for development workflow
- See [ARCHITECTURE.md](../ARCHITECTURE.md) for system architecture
- See [docs/AUDIO_SYSTEM.md](AUDIO_SYSTEM.md) for audio engine details
- See [docs/UI_SYSTEM.md](UI_SYSTEM.md) for UI development

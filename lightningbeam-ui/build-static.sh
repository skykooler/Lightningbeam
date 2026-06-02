#!/bin/bash
# Build script for static FFmpeg linking

set -e

# Point pkg-config to our static FFmpeg build
export PKG_CONFIG_PATH="/opt/ffmpeg-static/lib/pkgconfig:${PKG_CONFIG_PATH}"

# Tell pkg-config to use static linking
export PKG_CONFIG_ALL_STATIC=1

# Force static linking of codec libraries (and link required C++ and NUMA libraries)
export RUSTFLAGS="-C prefer-dynamic=no -C link-arg=-L/usr/lib/x86_64-linux-gnu -C link-arg=-Wl,-Bstatic -C link-arg=-lx264 -C link-arg=-lx265 -C link-arg=-lvpx -C link-arg=-lmp3lame -C link-arg=-Wl,-Bdynamic -C link-arg=-lstdc++ -C link-arg=-lnuma"

# Build with static features
echo "Building with static FFmpeg from /opt/ffmpeg-static..."
echo "PKG_CONFIG_PATH=$PKG_CONFIG_PATH"
echo "PKG_CONFIG_ALL_STATIC=$PKG_CONFIG_ALL_STATIC"

cargo build --release

echo ""
echo "Build complete! Binary at: target/release/lightningbeam-editor"
echo ""
echo "To verify static linking, run:"
echo "  ldd target/release/lightningbeam-editor | grep -E '(ffmpeg|avcodec|avformat|x264|x265|vpx)'"
echo "(Should show no ffmpeg or codec libraries if fully static)"

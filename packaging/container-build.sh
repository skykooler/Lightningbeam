#!/bin/bash
# Build Lightningbeam packages inside a minimal container
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE_NAME="lightningbeam-build"

# Detect container runtime
if command -v podman &>/dev/null; then
    RUNTIME=podman
elif command -v docker &>/dev/null; then
    RUNTIME=docker
else
    echo "Error: Neither podman nor docker found"
    exit 1
fi

echo "==> Using $RUNTIME"

# Build the container image (cached after first run)
echo "==> Building container image..."
$RUNTIME build -t "$IMAGE_NAME" -f "$SCRIPT_DIR/Containerfile" "$SCRIPT_DIR"

# Default: build all formats
TARGETS="${@:-deb rpm appimage}"

echo "==> Building packages: $TARGETS"
echo "    (To stop: $RUNTIME kill lb-build)"

mkdir -p "$SCRIPT_DIR/output"

# Remove stale container if it exists
$RUNTIME rm -f lb-build 2>/dev/null || true

$RUNTIME run --rm --init \
    --name lb-build \
    -v "$REPO_ROOT:/build/lightningbeam:ro" \
    -v "$SCRIPT_DIR/output:/output" \
    -v lb-cargo-registry:/root/.cargo/registry \
    -v lb-cargo-git:/root/.cargo/git \
    -v lb-target:/build/src/lightningbeam-ui/target \
    -v lb-egui-fork:/build/egui-fork \
    "$IMAGE_NAME" \
    bash -c "
set -e

# Clone egui fork if not already present
if [ ! -d /build/egui-fork/crates ]; then
    echo '==> Cloning egui fork...'
    rm -rf /build/egui-fork/*
    git clone --depth 1 -b ibus-wayland-fix https://git.skyler.io/skyler/egui.git /tmp/egui-clone
    mv /tmp/egui-clone/* /tmp/egui-clone/.* /build/egui-fork/ 2>/dev/null || true
    rm -rf /tmp/egui-clone
fi

# Sync source (fast — skips target/ and .git/)
echo '==> Syncing source...'
mkdir -p /build/src
rsync -a --delete \
    --exclude='/target' \
    --exclude='/lightningbeam-ui/target' \
    --exclude='/.git' \
    /build/lightningbeam/ /build/src/

cd /build/src/lightningbeam-ui

# Build FFmpeg from source and link statically (no .so bundling needed)
sed -i 's/ffmpeg-next = { version = \"8.0\", features = \[\"static\"\] }/ffmpeg-next = { version = \"8.0\", features = [\"build\", \"static\"] }/' lightningbeam-editor/Cargo.toml
export FFMPEG_STATIC=1

# Stage factory presets for packaging
echo '==> Staging factory presets...'
mkdir -p lightningbeam-editor/assets/presets
cp -r ../src/assets/instruments/* lightningbeam-editor/assets/presets/
# Remove empty category dirs and README
find lightningbeam-editor/assets/presets -maxdepth 1 -type d -empty -delete
rm -f lightningbeam-editor/assets/presets/README.md

# Add preset entries to cargo-generate-rpm config
find lightningbeam-editor/assets/presets -type f | sort | while read -r f; do
    rel=\"\${f#lightningbeam-editor/}\"
    dest=\"/usr/share/lightningbeam-editor/presets/\${f#lightningbeam-editor/assets/presets/}\"
    printf '\\n[[package.metadata.generate-rpm.assets]]\\nsource = \"%s\"\\ndest = \"%s\"\\nmode = \"644\"\\n' \"\$rel\" \"\$dest\" >> lightningbeam-editor/Cargo.toml
done

# Build release binary
echo '==> Building release binary...'
cargo build --release --bin lightningbeam-editor

for target in $TARGETS; do
    case \"\$target\" in
        deb)
            echo '==> Building .deb package...'
            cargo deb -p lightningbeam-editor --no-build --no-strip

            # Add factory presets to .deb (cargo-deb doesn't handle recursive dirs well)
            DEB=\$(ls target/debian/*.deb | head -1)
            WORK=\$(mktemp -d)
            dpkg-deb -R \"\$DEB\" \"\$WORK\"
            mkdir -p \"\$WORK/usr/share/lightningbeam-editor/presets\"
            cp -r lightningbeam-editor/assets/presets/* \"\$WORK/usr/share/lightningbeam-editor/presets/\"
            dpkg-deb -b \"\$WORK\" \"\$DEB\"
            rm -rf \"\$WORK\"

            cp target/debian/*.deb /output/
            echo '    .deb done'
            ;;
        rpm)
            echo '==> Building .rpm package...'
            cargo generate-rpm -p lightningbeam-editor
            cp target/generate-rpm/*.rpm /output/
            echo '    .rpm done'
            ;;
        appimage)
            echo '==> Building AppImage...'
            APPDIR=/tmp/AppDir
            ASSETS=lightningbeam-editor/assets
            rm -rf \"\$APPDIR\"
            mkdir -p \"\$APPDIR/usr/bin\"
            mkdir -p \"\$APPDIR/usr/share/applications\"
            mkdir -p \"\$APPDIR/usr/share/metainfo\"
            mkdir -p \"\$APPDIR/usr/share/icons/hicolor/32x32/apps\"
            mkdir -p \"\$APPDIR/usr/share/icons/hicolor/128x128/apps\"
            mkdir -p \"\$APPDIR/usr/share/icons/hicolor/256x256/apps\"

            cp target/release/lightningbeam-editor \"\$APPDIR/usr/bin/\"

            # Factory presets (next to binary for AppImage detection)
            mkdir -p \"\$APPDIR/usr/bin/presets\"
            cp -r lightningbeam-editor/assets/presets/* \"\$APPDIR/usr/bin/presets/\"
            cp \"\$ASSETS/com.lightningbeam.editor.desktop\" \"\$APPDIR/usr/share/applications/\"
            cp \"\$ASSETS/com.lightningbeam.editor.appdata.xml\" \"\$APPDIR/usr/share/metainfo/\"
            cp \"\$ASSETS/icons/32x32.png\" \"\$APPDIR/usr/share/icons/hicolor/32x32/apps/lightningbeam-editor.png\"
            cp \"\$ASSETS/icons/128x128.png\" \"\$APPDIR/usr/share/icons/hicolor/128x128/apps/lightningbeam-editor.png\"
            cp \"\$ASSETS/icons/256x256.png\" \"\$APPDIR/usr/share/icons/hicolor/256x256/apps/lightningbeam-editor.png\"

            ln -sf usr/share/icons/hicolor/256x256/apps/lightningbeam-editor.png \"\$APPDIR/lightningbeam-editor.png\"
            ln -sf usr/share/applications/com.lightningbeam.editor.desktop \"\$APPDIR/lightningbeam-editor.desktop\"

            cat > \"\$APPDIR/AppRun\" << 'APPRUN'
#!/bin/bash
SELF=\$(readlink -f \"\$0\")
HERE=\${SELF%/*}
export XDG_DATA_DIRS=\"\${HERE}/usr/share:\${XDG_DATA_DIRS:-/usr/local/share:/usr/share}\"
exec \"\${HERE}/usr/bin/lightningbeam-editor\" \"\$@\"
APPRUN
            chmod +x \"\$APPDIR/AppRun\"

            # Build squashfs from AppDir
            rm -f /tmp/appimage.squashfs
            mksquashfs \"\$APPDIR\" /tmp/appimage.squashfs \
                -root-owned -noappend -no-exports -no-xattrs \
                -comp gzip -b 131072

            # Verify squashfs is valid
            unsquashfs -s /tmp/appimage.squashfs

            # Concatenate runtime + squashfs = AppImage
            cat /opt/appimage-runtime /tmp/appimage.squashfs \
                > /output/Lightningbeam_Editor-x86_64.AppImage
            chmod +x /output/Lightningbeam_Editor-x86_64.AppImage
            rm -f /tmp/appimage.squashfs
            echo '    AppImage done'
            ;;
    esac
done

echo '==> All done!'
ls -lh /output/
"

.PHONY: all deb rpm appimage pkgbuild setup-icons clean

SCRIPT_DIR := $(dir $(lastword $(MAKEFILE_LIST)))
REPO_ROOT := $(realpath $(SCRIPT_DIR)/..)
EDITOR := $(REPO_ROOT)/lightningbeam-ui/lightningbeam-editor
OUTPUT := $(SCRIPT_DIR)/output

all: deb rpm appimage

deb rpm appimage:
	bash $(SCRIPT_DIR)/container-build.sh $@

pkgbuild: setup-icons
	@echo "==> Building Arch package..."
	cd $(SCRIPT_DIR)/arch && makepkg -sf
	@mkdir -p $(OUTPUT)
	cp $(SCRIPT_DIR)/arch/*.pkg.tar.zst $(OUTPUT)/ 2>/dev/null || true
	@echo "==> Done: $(OUTPUT)/*.pkg.tar.zst"

setup-icons:
	@echo "==> Setting up icons..."
	@mkdir -p $(EDITOR)/assets/icons
	@cp -u $(REPO_ROOT)/src-tauri/icons/32x32.png $(EDITOR)/assets/icons/ 2>/dev/null || true
	@cp -u $(REPO_ROOT)/src-tauri/icons/128x128.png $(EDITOR)/assets/icons/ 2>/dev/null || true
	@cp -u $(REPO_ROOT)/src-tauri/icons/icon.png $(EDITOR)/assets/icons/256x256.png 2>/dev/null || true
	@echo "    Icons ready"

clean:
	rm -rf $(OUTPUT)
	rm -f $(SCRIPT_DIR)/arch/*.pkg.tar.zst 2>/dev/null || true

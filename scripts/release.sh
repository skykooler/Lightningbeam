#!/usr/bin/env bash
set -euo pipefail

CARGO_TOML="$(dirname "$0")/../lightningbeam-ui/lightningbeam-editor/Cargo.toml"
CHANGELOG="$(dirname "$0")/../Changelog.md"

# Read current version
current=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)"/\1/')
echo "Current version: $current"

# Extract numeric prefix (e.g. 1.0.1 from 1.0.1-alpha)
base=${current%%-*}
suffix=${current#"$base"}

# Split into major.minor.patch
IFS='.' read -r major minor patch <<< "$base"

# Bump patch
new_patch=$((patch + 1))
new_version="${major}.${minor}.${new_patch}${suffix}"

# Check if version was already bumped this session (uncommitted change to Cargo.toml)
if git -C "$(dirname "$CARGO_TOML")" diff --name-only HEAD -- "$(basename "$CARGO_TOML")" | grep -q .; then
    echo "Cargo.toml already modified — skipping version bump (staying at $current)"
    new_version="$current"
else
    echo "Bumping to: $new_version"
    sed -i "0,/^version = \"$current\"/s//version = \"$new_version\"/" "$CARGO_TOML"
fi

# Edit changelog
vim "$CHANGELOG"

# Commit and push
git add "$CARGO_TOML" "$CHANGELOG"
git commit -m "Release v${new_version}"
# Push to the 'all' remote so the release branch lands on both GitHub and Gitea.
# CI (GitHub Actions) still triggers via the GitHub pushurl.
git push --force all "$(git branch --show-current):release"

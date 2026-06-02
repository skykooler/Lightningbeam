#!/bin/bash

set -e

if [ -z "$1" ]; then
  echo "Usage: ./create_release.sh <version>"
  exit 1
fi

VERSION=$1
RELEASE_BRANCH="release"
MAIN_BRANCH=$(git rev-parse --abbrev-ref HEAD)
CARGO_TOML="lightningbeam-ui/lightningbeam-editor/Cargo.toml"

echo "Updating version to $VERSION in $CARGO_TOML..."
sed -i "0,/^version = .*/s/^version = .*/version = \"$VERSION\"/" "$CARGO_TOML"

echo "Committing to $MAIN_BRANCH..."
git add "$CARGO_TOML"
git commit -m "Bump version to $VERSION"

echo "Checking out the release branch..."
git checkout $RELEASE_BRANCH

echo "Merging $MAIN_BRANCH into $RELEASE_BRANCH..."
git merge $MAIN_BRANCH --no-ff -m "Release $VERSION"

echo "Pushing $RELEASE_BRANCH..."
git push origin $RELEASE_BRANCH

git checkout $MAIN_BRANCH

echo "Release $VERSION created and pushed successfully!"

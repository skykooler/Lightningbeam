#!/bin/bash

# Ensure the script stops on error
set -e

# Check if a version argument was passed
if [ -z "$1" ]; then
  echo "Usage: ./create-release.sh <version>"
  exit 1
fi

VERSION=$1
RELEASE_BRANCH="release"
MAIN_BRANCH="main"
CONFIG_FILE="src-tauri/tauri.conf.json"

echo "Updating version to $VERSION in $CONFIG_FILE..."
jq --arg version "$VERSION" '.version = $version' $CONFIG_FILE > tmp.json && mv tmp.json $CONFIG_FILE

echo "Committing to main..."
git add $CONFIG_FILE
git commit -m "Bump version to $VERSION"

echo "Checking out the release branch..."
git checkout $RELEASE_BRANCH

echo "Merging the main branch into $RELEASE_BRANCH..."
git merge $MAIN_BRANCH --no-ff -m "Release $VERSION"

echo "Pushing changes to the release branch..."
git push origin $RELEASE_BRANCH

git checkout $MAIN_BRANCH

echo "Release $VERSION created and pushed successfully!"

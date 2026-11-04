#!/bin/sh
# Copies the synced String Catalog over the checked-in one. Run with
# `bazel run --config=xcode //apps/lifr/app:update_strings`, then review
# the diff: new strings need translating, and stale ones can be removed.
set -eu
dest="$BUILD_WORKSPACE_DIRECTORY/apps/lifr/app/Resources/Localizable.xcstrings"
cp "$1" "$dest"
chmod 644 "$dest"
echo "Updated $dest"

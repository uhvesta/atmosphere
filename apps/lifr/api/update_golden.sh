#!/bin/sh
# Copies freshly generated golden files over the checked-in ones. Run with
# `bazel run //apps/lifr/api:update_golden`, then review the diff.
set -eu
dest="$BUILD_WORKSPACE_DIRECTORY/apps/lifr/api/Tests/golden"
for file in "$@"; do
  cp "$file" "$dest/$(basename "$file")"
  chmod 644 "$dest/$(basename "$file")"
done
echo "Updated $dest"

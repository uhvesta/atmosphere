#!/bin/sh
set -eu

usage() {
    echo "Usage: $0 [--verify] <dir>" >&2
    echo "  --verify <dir>  Fail if any packaged JSON hook config contains permissionRequest or PermissionRequest." >&2
    echo "  <dir>           Strip permissionRequest / PermissionRequest keys from JSON hook configs in <dir>." >&2
    exit 2
}

mode="strip"
if [ "$#" -eq 0 ]; then
    usage
fi

if [ "$1" = "--verify" ]; then
    mode="verify"
    shift
fi

if [ "$#" -ne 1 ]; then
    usage
fi

target_dir="$1"
if [ ! -d "$target_dir" ]; then
    echo "ERROR: directory does not exist: $target_dir" >&2
    exit 1
fi

if [ "$mode" = "verify" ]; then
    found=0
    for file in $(find "$target_dir" -type f -name '*.json' | sort); do
        if grep -Eq '"(permissionRequest|PermissionRequest)"' "$file"; then
            echo "ERROR: forbidden generic permission hook found in $file (remove permissionRequest / PermissionRequest)" >&2
            found=1
        fi
    done
    if [ "$found" -ne 0 ]; then
        exit 1
    fi
    exit 0
fi

python3 - "$target_dir" <<'PY'
import json
import os
import sys

root = sys.argv[1]
FORBIDDEN = ("permissionRequest", "PermissionRequest")

for dirpath, _, filenames in os.walk(root):
    for filename in sorted(filenames):
        if not filename.endswith(".json"):
            continue
        path = os.path.join(dirpath, filename)
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except Exception:
            continue
        if not isinstance(data, dict):
            continue
        changed = False
        hooks = data.get("hooks")
        if isinstance(hooks, dict):
            for key in FORBIDDEN:
                if key in hooks:
                    del hooks[key]
                    changed = True
        for key in FORBIDDEN:
            if key in data:
                del data[key]
                changed = True
        if changed:
            with open(path, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2)
                f.write("\n")
PY

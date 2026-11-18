#!/bin/sh
set -eu
prototype_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
workspace_dir=$(CDPATH= cd -- "$prototype_dir/../../.." && pwd)
cd "$workspace_dir"
.tools/bin/bazel build -c opt //apps/humata2/ui-prototype:desktop
desktop_binary="$workspace_dir/bazel-bin/apps/humata2/ui-prototype/desktop"
python3 - "$prototype_dir" "$desktop_binary" <<'PY'
from pathlib import Path
import plistlib
import sys

prototype, desktop_binary = map(Path, sys.argv[1:])
app = prototype / 'Humata UI Prototype.app'
binary = app / 'Contents/MacOS/Humata UI Prototype'
binary.parent.mkdir(parents=True, exist_ok=True)
if binary.is_symlink():
    binary.unlink()
binary.symlink_to(desktop_binary)
(app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
    'CFBundleExecutable': 'Humata UI Prototype',
    'CFBundleIdentifier': 'local.humata2.ui-prototype',
    'CFBundleName': 'Humata UI Prototype',
    'CFBundlePackageType': 'APPL',
    'NSHighResolutionCapable': True,
}))
print(app)
PY
exec open -n "$prototype_dir/Humata UI Prototype.app"

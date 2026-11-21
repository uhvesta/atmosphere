#!/bin/sh
set -eu

bundle_dir="$1"
[ -d "$bundle_dir" ] || {
  echo "browser_extension bundle directory missing: $bundle_dir" >&2
  exit 1
}

python3 - "$bundle_dir" <<'PY'
import json
import os
import sys

bundle_dir = sys.argv[1]

forbidden_js_patterns = [
    "executeScript",
    "tabs.update",
    "performClick",
    "performType",
    "performSubmit",
    "renderHighlightGuide",
    'case "click"',
    'case "type"',
    'case "navigate"',
    'case "submit"',
    'case "highlight_guide"',
    ".click(",
    "requestSubmit",
]

for family in ("chromium", "firefox"):
    fam_dir = os.path.join(bundle_dir, family)
    manifest_path = os.path.join(fam_dir, "manifest.json")
    assert os.path.isfile(manifest_path), f"Missing {manifest_path}"

    with open(manifest_path, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    assert manifest.get("manifest_version") == 3, f"{family}: expected MV3"
    perms = manifest.get("permissions", [])
    assert "scripting" not in perms, f"{family}: scripting permission must not be present"
    for required in ("nativeMessaging", "tabs", "activeTab"):
        assert required in perms, f"{family}: missing required permission {required}"

    bg = manifest.get("background", {})
    if family == "chromium":
        bg_paths = [bg["service_worker"]]
    else:
        bg_paths = list(bg["scripts"])
    assert bg_paths == ["shared/background.js"], f"{family}: unexpected background paths {bg_paths}"

    content_scripts = manifest.get("content_scripts", [])
    assert len(content_scripts) == 1, f"{family}: expected 1 content script entry"
    cs_paths = content_scripts[0].get("js", [])
    assert cs_paths == ["shared/content.js"], f"{family}: unexpected content_scripts js {cs_paths}"

    for rel_path in bg_paths + cs_paths:
        full_path = os.path.join(fam_dir, rel_path)
        assert os.path.isfile(full_path), f"{family}: declared script missing at {full_path}"
        with open(full_path, "r", encoding="utf-8") as sf:
            src = sf.read()
        assert src.strip(), f"{family}/{rel_path} must not be empty"
        for pat in forbidden_js_patterns:
            assert pat not in src, f"{family}/{rel_path} contains forbidden execute/mutate pattern: {pat}"

    bg_full = os.path.join(fam_dir, "shared/background.js")
    with open(bg_full, "r", encoding="utf-8") as bf:
        bg_src = bf.read()
    for required_bg in ('case "list_tabs"', 'case "read_tab"', 'case "screenshot"', "captureVisibleTab"):
        assert required_bg in bg_src, f"{family}/shared/background.js missing required read-only handler: {required_bg}"

print("browser_extension bundle verified: strictly read-only and self-contained for chromium & firefox.")
PY

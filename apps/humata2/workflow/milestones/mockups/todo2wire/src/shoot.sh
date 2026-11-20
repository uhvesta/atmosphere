#!/bin/sh
# usage: shoot.sh; runs t.py and screenshots its frames (headless, no window)
cd "$(dirname "$0")" && python3 t.py && for h in t*-*.html; do f=${h%.html}
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --window-size=1600,950 --screenshot="$PWD/../$f.png" "file://$PWD/$h" >/dev/null 2>&1; echo $f; done

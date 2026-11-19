#!/bin/sh
# usage: shoot.sh <batch number>; runs b<N>.py and screenshots its frames
cd "$(dirname "$0")" && python3 b$1.py && for h in b$1-*.html; do f=${h%.html}
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --window-size=1600,950 --screenshot="$PWD/../$f.png" "file://$PWD/$h" >/dev/null 2>&1; echo $f; done

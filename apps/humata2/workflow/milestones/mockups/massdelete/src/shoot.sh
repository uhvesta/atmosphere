#!/bin/sh
# usage: shoot.sh <batch number>; runs m<N>.py and screenshots its frames.
# Pages are 1280x760, shot at 1.25x: each PNG is 1600x950.
cd "$(dirname "$0")" && python3 m$1.py && for h in m$1-*.html; do f=${h%.html}
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1.25 --window-size=1280,760 --screenshot="$PWD/../$f.png" "file://$PWD/$h" >/dev/null 2>&1; echo $f; done

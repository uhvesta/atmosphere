#!/bin/sh
# usage: shoot.sh; runs im.py and screenshots its frames at 1600x950 in parallel (headless)
set -e
cd "$(dirname "$0")"
python3 im.py
for h in im-*.html; do
  f=${h%.html}
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    --headless=new --disable-gpu --hide-scrollbars \
    --force-device-scale-factor=1 --window-size=1600,950 \
    --screenshot="$PWD/../$f.png" "file://$PWD/$h" >/dev/null 2>&1 &
done
wait
for h in im-*.html; do echo "${h%.html}"; done

#!/bin/sh
set -eu
: "${TERMINAL_LAB_SSH_CONFIG:?Set TERMINAL_LAB_SSH_CONFIG to the existing h2-terminal-c1 SSH config}"
/usr/bin/ssh -F "$TERMINAL_LAB_SSH_CONFIG" -o BatchMode=yes -o ConnectTimeout=10 \
  lima-h2-terminal-c1 sh -s <<'SH'
set -eu
uname -sm
chromium --version
printf '\nInstalled graphics packages:\n'
dpkg-query -W -f='${Package} ${Version}\n' chromium chromium-common mesa-vulkan-drivers libegl-mesa0 libgl1-mesa-dri libgles2 2>/dev/null || true
printf '\nChromium library directory:\n'
ls -1 /usr/lib/chromium
printf '\nSoftware GL/Vulkan libraries and ICDs:\n'
if command -v rg >/dev/null 2>&1; then
  rg --files /usr/lib/chromium /usr/lib/aarch64-linux-gnu /usr/share/vulkan/icd.d | rg '(swiftshader|libEGL|libGLES|libGLX|swrast|lvp|icd.*json)' || true
else
  find /usr/lib/chromium /usr/lib/aarch64-linux-gnu /usr/share/vulkan/icd.d -maxdepth 3 \
    \( -iname '*swiftshader*' -o -iname '*libEGL*' -o -iname '*libGLES*' -o -iname '*libGLX*' -o -iname '*swrast*' -o -iname '*lvp*' -o -iname '*icd*json' \) -print
fi
printf '\nInstalled capability utilities:\n'
for utility in eglinfo glxinfo vulkaninfo; do command -v "$utility" || true; done
SH

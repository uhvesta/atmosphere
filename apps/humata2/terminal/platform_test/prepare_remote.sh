#!/bin/sh
set -eu
if [ "$(uname -s)" != Linux ]; then
    echo 'The selected proof machine must run Linux.' >&2
    exit 1
fi
printf 'Runtime machine: '
uname -sm
awk '/MemTotal:|MemAvailable:|SwapTotal:/ { print }' /proc/meminfo
df -Pm /var/tmp
set --
if ! command -v chromium >/dev/null 2>&1; then set -- "$@" chromium; fi
if ! command -v node >/dev/null 2>&1; then
    set -- "$@" nodejs
elif node -e 'if(typeof WebSocket!=="function")process.exit(1)' 2>/dev/null; then
    echo 'Existing Node provides built-in WebSocket; no Node download is needed.'
elif node --experimental-websocket -e 'if(typeof WebSocket!=="function")process.exit(1)' 2>/dev/null; then
    echo 'Existing Node provides WebSocket with --experimental-websocket; no Node download is needed.'
else
    echo 'Existing Node lacks built-in WebSocket; checking the Debian nodejs package.'
    set -- "$@" nodejs
fi
case "$(dpkg-query -W -f='${Status}' mesa-vulkan-drivers 2>/dev/null || true)" in
    'install ok installed') ;;
    *) set -- "$@" mesa-vulkan-drivers ;;
esac
if [ "$#" -gt 0 ]; then
    available=$(df -Pm /var/tmp | awk 'NR==2 { print $4 }')
    if [ "$available" -lt 700 ]; then
        echo 'The owned VM needs at least 700 MiB free for Chromium, Node and the software Vulkan runtime. Free only disposable proof artifacts before retrying.' >&2
        exit 1
    fi
    sudo -n env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=60 update
    sudo -n env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=60 \
        install -y --no-install-recommends "$@"
fi
node --version
chromium --version
if node -e 'if(typeof WebSocket!=="function")process.exit(1)' 2>/dev/null; then
    echo 'Node WebSocket: available without flags'
elif node --experimental-websocket -e 'if(typeof WebSocket!=="function")process.exit(1)' 2>/dev/null; then
    echo 'Node WebSocket: available with --experimental-websocket'
else
    echo 'The installed Node runtime has no built-in WebSocket. Install Node 20.10+ (with --experimental-websocket) or Node 22+ in this VM, then retry; no npm package is needed.' >&2
    exit 1
fi
echo 'Runtime preparation complete; VM processes and live terminal sessions were left running.'

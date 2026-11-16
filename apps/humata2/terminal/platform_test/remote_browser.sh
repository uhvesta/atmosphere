#!/bin/sh
set -eu
umask 077
root=$1
cd "$root"
mkdir -p output
mkdir -p "$root/tmp" "$root/cache" "$root/config" "$root/runtime"
export TMPDIR="$root/tmp"
export XDG_CACHE_HOME="$root/cache"
export XDG_CONFIG_HOME="$root/config"
export XDG_RUNTIME_DIR="$root/runtime"
if ! command -v node >/dev/null 2>&1 || ! command -v chromium >/dev/null 2>&1; then
  echo 'Run platform_test:prepare_linux_browser_test first to install the isolated VM browser runtime.' >&2
  exit 1
fi
node_flag=
if ! node -e 'if(typeof WebSocket!=="function")process.exit(1)' 2>/dev/null; then
  if node --experimental-websocket -e 'if(typeof WebSocket!=="function")process.exit(1)' 2>/dev/null; then
    node_flag=--experimental-websocket
  else
    echo 'Node needs built-in WebSocket support; run the runtime preparation target before retrying.' >&2
    exit 1
  fi
fi
export TERMINAL_VIEWER_URL_FILE="$root/viewer.url"
export TERMINAL_EXPECT_SESSION_OS
TERMINAL_EXPECT_SESSION_OS=$(cat expected-os.json)
export TERMINAL_BROWSER_INPUT=1
export TERMINAL_BROWSER_MIRROR
TERMINAL_BROWSER_MIRROR=$(cat mirror)
case "$TERMINAL_BROWSER_MIRROR" in 0 | 1) ;; *)
  echo 'Mirror acceptance must be 0 or 1.' >&2
  exit 1
  ;;
esac
TERMINAL_BROWSER_TAB=$(sed -n '1p' checks)
TERMINAL_BROWSER_STYLE=$(sed -n '2p' checks)
TERMINAL_BROWSER_CLIPBOARD=$(sed -n '3p' checks)
for check in "$TERMINAL_BROWSER_TAB" "$TERMINAL_BROWSER_STYLE" "$TERMINAL_BROWSER_CLIPBOARD"; do
  case "$check" in 0 | 1) ;; *)
    echo 'Optional browser checks must be 0 or 1.' >&2
    exit 1
    ;;
  esac
done
export TERMINAL_BROWSER_TAB TERMINAL_BROWSER_STYLE TERMINAL_BROWSER_CLIPBOARD
export TERMINAL_BROWSER_ISOLATED_CLIPBOARD=1
export TERMINAL_BROWSER=/usr/bin/chromium
export TEST_UNDECLARED_OUTPUTS_DIR="$root/output"
export TERMINAL_BROWSER_FLAGS
TERMINAL_BROWSER_FLAGS=$(cat browser-flags.json)
if [ -z "$TERMINAL_BROWSER_FLAGS" ]; then
  TERMINAL_BROWSER_FLAGS='["--no-sandbox","--no-zygote","--renderer-process-limit=1","--use-gl=angle","--use-angle=swiftshader-webgl","--enable-unsafe-swiftshader","--disable-features=Vulkan"]'
fi
node -e 'try{const f=require("node:fs"),u=new URL(f.readFileSync(process.env.TERMINAL_VIEWER_URL_FILE,"utf8").trim());if(u.protocol!=="http:"||u.hostname!=="127.0.0.1"||u.port!=="8741")throw Error();const values=Object.values(JSON.parse(process.env.TERMINAL_EXPECT_SESSION_OS));if(!values.includes("Darwin")||!values.includes("Linux"))throw Error();}catch{console.error("Use a valid private localhost:8741 viewer URL and configure both Darwin/Linux session expectations; credentials were not logged.");process.exit(1);}'
uname -sm >output/linux-platform.txt
node --version >>output/linux-platform.txt
chromium --version >>output/linux-platform.txt
awk '/MemTotal:|MemAvailable:|SwapTotal:/ { print }' /proc/meminfo >>output/linux-platform.txt
group=
cleanup() {
  if [ -n "$group" ]; then
    /bin/kill -TERM -- "-$group" 2>/dev/null || true
    sleep 0.2
    /bin/kill -KILL -- "-$group" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
# Only the proof's process group is terminated on timeout/cleanup.
setsid sh -c 'printf "500\n" > /proc/self/oom_score_adj; exec "$@"' sh \
  timeout --signal=TERM --kill-after=5s 300s node ${node_flag:+"$node_flag"} browser_smoke.mjs \
  >output/browser-run.log 2>&1 &
group=$!
status=0
wait "$group" || status=$?
cleanup
group=
node sanitize.mjs "$root/viewer.url" "$root/output"
cat output/linux-platform.txt
cat output/browser-run.log
exit "$status"

#!/bin/sh
# Builds Humata for this Mac, installs it in ~/Applications and opens it.
# Installs the pinned Bazelisk first if it isn't there. Needs Xcode.
#
#   ./humata.sh               build, install and open
#   ./humata.sh --no-open     build and install only
set -eu

root=$(CDPATH='' cd -P "$(dirname "$0")" && pwd)
cd "$root"

open_after=1
for arg in "$@"; do
  case "$arg" in
    --no-open) open_after=0 ;;
    *)
      echo "Unknown option $arg; the only option is --no-open." >&2
      exit 2
      ;;
  esac
done

if [ "$(uname -s)" != Darwin ]; then
  echo "Humata is a Mac app; build it on a Mac." >&2
  exit 1
fi
if ! xcode-select -p >/dev/null 2>&1; then
  echo "Xcode isn't set up. Install Xcode, then run: sudo xcode-select -s /Applications/Xcode.app" >&2
  exit 1
fi

sh tools/bootstrap/bazelisk.sh
bazel="$root/.tools/bin/bazel"

"$bazel" build --config=xcode //apps/humata/app:macos

out=$("$bazel" cquery --config=xcode --output=files //apps/humata/app:macos 2>/dev/null | head -n 1)
if [ -z "$out" ] || [ ! -e "$out" ]; then
  echo "The build finished but its output wasn't found; run: bazel cquery --config=xcode --output=files //apps/humata/app:macos" >&2
  exit 1
fi

dest="$HOME/Applications"
app="$dest/Humata.app"
mkdir -p "$dest"

# Quit the running copy so the new one replaces it; humatad keeps running
# when Humata is installed as a login item, and sessions survive either way.
if pgrep -xq Humata; then
  osascript -e 'quit app "Humata"' >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -xq Humata || break
    sleep 0.5
  done
fi

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
case "$out" in
  *.zip) ditto -x -k "$out" "$staging" ;;
  *.app) ditto "$out" "$staging/Humata.app" ;;
  *)
    echo "Didn't expect the build to make $out; expected a .app or .zip." >&2
    exit 1
    ;;
esac
if [ ! -d "$staging/Humata.app" ]; then
  echo "$out has no Humata.app in it." >&2
  exit 1
fi

rm -rf "$app"
ditto "$staging/Humata.app" "$app"
# Bazel's outputs are read-only; make the copy writable so the next run can replace it.
chmod -R u+w "$app"
echo "Installed $app"

if [ "$open_after" = 1 ]; then
  open "$app"
fi

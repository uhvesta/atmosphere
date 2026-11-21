#!/bin/sh
# Incrementally fastbuilds Humata 2 via tools/bq/bq, signs it locally with a
# Developer ID Application certificate (no notarization, no GitHub upload),
# installs it to /Applications/Humata 2.app, and relaunches it.
#
# Run from anywhere in the repository:
#   apps/humata2/tools/release/dev-install.sh [options]
set -eu

fail() {
  echo "dev-install: $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: apps/humata2/tools/release/dev-install.sh [options]

Incrementally fastbuild //apps/humata2/app:app with Bazel (via tools/bq/bq),
Developer-ID sign locally, install to /Applications/Humata 2.app, and relaunch.
No notarization and no GitHub upload are performed.

Options:
  -h, --help             Show this help message and exit
  --no-launch            Install without quitting or relaunching the app
  --dest <path>          Install destination (default: /Applications/Humata 2.app)
  --identity <name>      Code signing identity (default: $HUMATA2_SIGNING_IDENTITY
                         or first "Developer ID Application:" in Keychain)
  --watch                Rebuild, sign, install, and relaunch on changes under
                         apps/humata2 (uses fswatch if available, else polling)

Environment variables:
  HUMATA2_SIGNING_IDENTITY  Default signing identity if --identity is omitted
  HUMATA2_SECURITY_BIN      Path to security binary (default: security)
  DEV_INSTALL_POLL_SECS     Polling interval in seconds for --watch fallback (default: 2)
  DEV_INSTALL_WATCH_MAX     Max watch rebuild cycles after initial build (default: unlimited)
EOF
}

dest="/Applications/Humata 2.app"
no_launch=0
watch=0
identity="${HUMATA2_SIGNING_IDENTITY:-}"
security_bin="${HUMATA2_SECURITY_BIN:-security}"
poll_secs="${DEV_INSTALL_POLL_SECS:-2}"
watch_max="${DEV_INSTALL_WATCH_MAX:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --no-launch)
      no_launch=1
      shift
      ;;
    --watch)
      watch=1
      shift
      ;;
    --dest)
      [ $# -ge 2 ] || fail "--dest requires a path argument"
      dest="$2"
      shift 2
      ;;
    --dest=*)
      dest="${1#--dest=}"
      [ -n "$dest" ] || fail "--dest requires a non-empty path"
      shift
      ;;
    --identity)
      [ $# -ge 2 ] || fail "--identity requires a signing identity argument"
      identity="$2"
      shift 2
      ;;
    --identity=*)
      identity="${1#--identity=}"
      [ -n "$identity" ] || fail "--identity requires a non-empty signing identity"
      shift
      ;;
    *)
      fail "unknown option: $1 (run with --help for usage)"
      ;;
  esac
done

script_dir=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/../../../.." && pwd)
cd "$root"

target="//apps/humata2/app:app"

# Fail fast before building if no Developer ID Application identity is available.
if [ -z "$identity" ]; then
  identity=$("$security_bin" find-identity -v -p codesigning 2>/dev/null \
    | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' \
    | head -n 1 || true)
fi
[ -n "$identity" ] ||
  fail "no Developer ID Application certificate found in the Keychain. Create one in Xcode > Settings > Accounts > Manage Certificates, or pass --identity / set HUMATA2_SIGNING_IDENTITY. An Apple Development certificate won't do: the daemon accepts only a Developer ID app."

[ -x "$root/.tools/bin/bazel" ] || sh tools/bootstrap/bazelisk.sh
bazel="$root/.tools/bin/bazel"

sign() {
  codesign --force --timestamp --options runtime --sign "$identity" "$@"
}

current_scratch=""
cleanup() {
  if [ -n "$current_scratch" ] && [ -d "$current_scratch" ]; then
    rm -rf "$current_scratch"
  fi
}
trap cleanup EXIT

run_once() {
  echo "dev-install: building $target (fastbuild via tools/bq/bq)..."
  tools/bq/bq --build --fast "$target"

  out=$("$bazel" cquery --config=fast --output=files "$target" 2>/dev/null | head -n 1 || true)
  if [ -z "$out" ] || [ ! -d "$out" ]; then
    out="$root/bazel-bin/apps/humata2/app/Humata 2.app"
  fi
  [ -d "$out" ] || fail "build finished but .app bundle was not found at \"$out\""

  current_scratch=$(mktemp -d)
  chmod 700 "$current_scratch"
  app="$current_scratch/Humata 2.app"
  ditto "$out" "$app"
  chmod -R u+w "$app"

  bundle_id=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$app/Contents/Info.plist")
  case "$bundle_id" in
    com.example.*)
      fail "the app was built as $bundle_id (placeholder). Set LIFR_TEAM_ID and LIFR_BUNDLE_PREFIX in this checkout's user.bazelrc (see apps/lifr/identity.bzl) and rebuild."
      ;;
  esac

  identity_defs=$(ls "$("$bazel" info output_base 2>/dev/null)"/external/*lifr_identity*/defs.bzl 2>/dev/null | head -n 1 || true)
  team=$(sed -n 's/^TEAM_ID = "\([A-Z0-9]*\)"$/\1/p' "${identity_defs:-/dev/null}")
  [ -n "$team" ] ||
    fail "couldn't find the team ID the daemon was built with. Set LIFR_TEAM_ID in this checkout's user.bazelrc (see apps/lifr/identity.bzl) and rebuild."

  echo "dev-install: signing with ${identity}..."
  for helper in "$app/Contents/Helpers/"*; do
    [ -e "$helper" ] || continue
    sign "$helper"
  done
  for appex in "$app/Contents/Extensions/"*.appex; do
    [ -d "$appex" ] || continue
    appex_ent="$current_scratch/$(basename "$appex").entitlements"
    if [ -f "apps/humata2/app/fskit/HumataFS.entitlements" ]; then
      sed "s/@APP_GROUP@/$team.$bundle_id/g" "apps/humata2/app/fskit/HumataFS.entitlements" >"$appex_ent"
      for bin in "$appex/Contents/MacOS/"*; do
        [ -f "$bin" ] && sign --entitlements "$appex_ent" "$bin"
      done
      sign --entitlements "$appex_ent" "$appex"
    else
      sign "$appex"
    fi
  done

  manifest="$app/Contents/Resources/manifest.json"
  if [ -f "$manifest" ]; then
    for helper in "$app/Contents/Helpers/"*; do
      [ -e "$helper" ] || continue
      name=$(basename "$helper")
      key="platforms.darwin-arm64.$name.sha256"
      plutil -extract "$key" raw "$manifest" >/dev/null 2>&1 || continue
      plutil -replace "$key" -string "$(shasum -a 256 "$helper" | cut -d' ' -f1)" "$manifest"
    done
    for helper in "$app/Contents/Helpers/"*; do
      [ -e "$helper" ] || continue
      name=$(basename "$helper")
      listed=$(plutil -extract "platforms.darwin-arm64.$name.sha256" raw "$manifest" 2>/dev/null) || continue
      [ "$listed" = "$(shasum -a 256 "$helper" | cut -d' ' -f1)" ] ||
        fail "the manifest's checksum for $name doesn't match the signed file."
    done
  fi

  sign "$app"
  codesign --verify --deep --strict "$app" ||
    fail "the app's signature failed verification after signing."

  requirement="anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13] and certificate leaf[subject.OU] = \"$team\" and identifier \"$bundle_id\""
  codesign --verify -R="$requirement" "$app" ||
    fail "the signed app doesn't meet the daemon's requirement ($bundle_id, Developer ID of team $team). Sign with a Developer ID Application certificate of team $team, or set LIFR_TEAM_ID to the certificate's team."

  if [ "$no_launch" -eq 0 ]; then
    osascript -e 'tell application "Humata 2" to quit' >/dev/null 2>&1 || true
    sleep 1
  fi

  if [ -e "$dest" ]; then
    rm -rf "$dest" || fail "couldn't remove existing \"$dest\"; quit Humata 2 and try again."
  fi
  mkdir -p "$(dirname "$dest")"
  ditto "$app" "$dest" || fail "couldn't copy the app to \"$dest\"; check permissions."

  installed=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$dest/Contents/Info.plist" 2>/dev/null || echo "dev")
  rm -rf "$current_scratch"
  current_scratch=""

  echo "dev-install: installed Humata 2 ($installed) to $dest"
  if [ "$no_launch" -eq 0 ]; then
    open "$dest"
    echo "dev-install: relaunched $dest"
  fi
}

watch_fingerprint() {
  find apps/humata2 -type f -exec stat -f '%m %z %N' {} + 2>/dev/null \
    | sort \
    | shasum -a 256 \
    | cut -d' ' -f1
}

run_once

if [ "$watch" -eq 1 ]; then
  cycles=0
  if command -v fswatch >/dev/null 2>&1; then
    echo "dev-install: watching apps/humata2 with fswatch..."
    while :; do
      if [ -n "$watch_max" ] && [ "$cycles" -ge "$watch_max" ]; then
        echo "dev-install: reached DEV_INSTALL_WATCH_MAX=$watch_max, exiting watch mode."
        break
      fi
      fswatch -1 -r apps/humata2 >/dev/null
      cycles=$((cycles + 1))
      echo "dev-install: change detected (cycle $cycles), rebuilding..."
      run_once || echo "dev-install: rebuild failed; waiting for next change..." >&2
    done
  else
    echo "dev-install: watching apps/humata2 by polling every ${poll_secs}s..."
    last_fp=$(watch_fingerprint)
    while :; do
      if [ -n "$watch_max" ] && [ "$cycles" -ge "$watch_max" ]; then
        echo "dev-install: reached DEV_INSTALL_WATCH_MAX=$watch_max, exiting watch mode."
        break
      fi
      sleep "$poll_secs"
      next_fp=$(watch_fingerprint)
      if [ "$next_fp" != "$last_fp" ]; then
        last_fp="$next_fp"
        cycles=$((cycles + 1))
        echo "dev-install: change detected (cycle $cycles), rebuilding..."
        run_once || echo "dev-install: rebuild failed; waiting for next change..." >&2
      fi
    done
  fi
fi

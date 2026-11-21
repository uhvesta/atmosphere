#!/bin/sh
# Verifies that HumataFS.appex builds as a valid, signed macOS arm64 FSKit
# extension with the expected Info.plist attributes and entitlements.
set -eu

appex_in="$1"
[ -d "$appex_in" ] || { echo "missing appex dir: $appex_in" >&2; exit 1; }
tmp=$(mktemp -d "${TEST_TMPDIR:-/tmp}/fskit-appex-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
appex="$tmp/HumataFS.appex"
cp -RL "$appex_in" "$appex"

[ -x "$appex/Contents/MacOS/HumataFS" ] || { echo "missing Mach-O executable" >&2; exit 1; }
[ -f "$appex/Contents/Info.plist" ] || { echo "missing Info.plist" >&2; exit 1; }

/usr/bin/plutil -lint "$appex/Contents/Info.plist"

ep=$(/usr/libexec/PlistBuddy -c "Print :EXAppExtensionAttributes:EXExtensionPointIdentifier" "$appex/Contents/Info.plist")
[ "$ep" = "com.apple.fskit.fsmodule" ] || { echo "unexpected EXExtensionPointIdentifier: $ep" >&2; exit 1; }

short=$(/usr/libexec/PlistBuddy -c "Print :EXAppExtensionAttributes:FSShortName" "$appex/Contents/Info.plist")
[ "$short" = "humatafs" ] || { echo "unexpected FSShortName: $short" >&2; exit 1; }

scheme=$(/usr/libexec/PlistBuddy -c "Print :EXAppExtensionAttributes:FSSupportedSchemes:0" "$appex/Contents/Info.plist")
[ "$scheme" = "humatafs" ] || { echo "unexpected FSSupportedSchemes[0]: $scheme" >&2; exit 1; }

/usr/bin/codesign --verify --deep --strict "$appex"

ent=$(/usr/bin/codesign --display --entitlements :- "$appex" 2>/dev/null)
printf '%s' "$ent" | grep -q "com.apple.developer.fskit.fsmodule" || {
  echo "missing com.apple.developer.fskit.fsmodule entitlement" >&2
  exit 1
}
printf '%s' "$ent" | grep -q "com.apple.security.application-groups" || {
  echo "missing com.apple.security.application-groups entitlement" >&2
  exit 1
}

#!/bin/sh
# Releases Humata 2 on GitHub. It builds the app from the commit checked out
# and signs the app and every Mac binary in it with the Developer ID
# certificate and the hardened runtime. That signature is what the release
# daemon checks before it takes the app as its user (D135).
# It then notarizes the app with the App Store Connect key that the
# TestFlight uploads keep in the Keychain (apps/lifr/tools/testflight/setup.sh),
# staples the ticket and uploads a draft release. check.sh downloads the draft
# as people would get it, and the release is published only if that passes.
#
# Run from anywhere in the repository:
#   apps/humata2/tools/release/release.sh 0.1.0
# Then install the published release into /Applications with install.sh.
#
# GitHub access comes from the GitHub command line (`gh auth login`). The key
# only ever travels on a pipe. The exception is while notarytool reads it from
# a file in a folder only you can open, which is deleted right after the upload.
set -eu
fail() {
  echo "release: $*" >&2
  exit 1
}
[ $# -eq 1 ] || fail "usage: apps/humata2/tools/release/release.sh <version>, such as 0.1.0"
version=$1
printf '%s' "$version" | grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+' ||
  fail "\"$version\" isn't a version: give three numbers, such as 0.1.0."
root=$(git rev-parse --show-toplevel)
cd "$root"
here=apps/humata2/tools/release
tag="humata2-v$version"
service=lifr.app-store-connect
target=//apps/humata2/app

[ -z "$(git status --porcelain --untracked-files=no)" ] ||
  fail "commit your changes first: a release is built from a commit, so the tag says what's in it."
commit=$(git rev-parse HEAD)
[ -n "$(git branch -r --contains "$commit" 2>/dev/null)" ] ||
  fail "push $commit first: the release's tag points at it, so GitHub needs to have it."
command -v gh >/dev/null 2>&1 || fail "the GitHub command line isn't installed; install it with \`brew install gh\`, then run \`gh auth login\`."
gh auth status --hostname github.com >/dev/null 2>&1 ||
  fail "the GitHub command line isn't signed in, or its login expired; run \`gh auth login\` and try again."
if gh release view "$tag" >/dev/null 2>&1; then
  fail "there's already a release $tag; give a new version. Deleting that one instead would break the links people have to it."
fi
identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -n 1)
[ -n "$identity" ] ||
  fail "there's no Developer ID Application certificate in the Keychain. Make one in Xcode › Settings › Accounts › Manage Certificates. An Apple Development certificate won't do: Gatekeeper accepts only Developer ID outside the App Store, and the daemon accepts only a Developer ID app."
security find-generic-password -s "$service" >/dev/null 2>&1 ||
  fail "there's no App Store Connect key in the Keychain to notarize with. Store one first: bazel run //apps/lifr/tools/testflight:setup -- <.p8> <Key ID> <Issuer ID>"

sh tools/bootstrap/bazelisk.sh
bazel="$root/.tools/bin/bazel"
echo "Building Humata 2 $version from ${commit}…"
"$bazel" build --compilation_mode=opt "$target"
out=$("$bazel" cquery --compilation_mode=opt --output=files "$target" 2>/dev/null | head -n 1)
[ -n "$out" ] && [ -d "$out" ] || fail "the build finished but its output wasn't found; run: bazel cquery --compilation_mode=opt --output=files $target"

scratch=$(mktemp -d)
chmod 700 "$scratch"
trap 'rm -rf "$scratch"' EXIT
app="$scratch/Humata 2.app"
ditto "$out" "$app"
chmod -R u+w "$app"
bundle_id=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$app/Contents/Info.plist")
case "$bundle_id" in
  com.example.*)
    fail "the app was built as $bundle_id, the placeholder used when no identity is set. Set LIFR_TEAM_ID and LIFR_BUNDLE_PREFIX in this checkout's user.bazelrc (see apps/lifr/identity.bzl), then release again. A placeholder ID would change once fixed, and the daemon would stop taking the app as its user."
    ;;
esac
# The team the daemon was built to accept (HUMATA2_TEAM_ID, from @lifr_identity).
identity_defs=$(ls "$("$bazel" info output_base 2>/dev/null)"/external/*lifr_identity*/defs.bzl 2>/dev/null | head -n 1)
team=$(sed -n 's/^TEAM_ID = "\([A-Z0-9]*\)"$/\1/p' "${identity_defs:-/dev/null}")
[ -n "$team" ] ||
  fail "couldn't find the team ID the daemon was built with. Set LIFR_TEAM_ID in this checkout's user.bazelrc (see apps/lifr/identity.bzl) and release again: a daemon built without a team accepts no app, so this build would lock everyone out."
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$(git rev-list --count HEAD)" "$app/Contents/Info.plist"

# Inside out: the helpers first, then the app. No entitlements: the daemon
# refuses an app that holds any that allow code injection.
echo "Signing with ${identity}…"
sign() {
  codesign --force --timestamp --options runtime --sign "$identity" "$@"
}
for helper in "$app/Contents/Helpers/"*; do
  sign "$helper"
done
for appex in "$app/Contents/Extensions/"*.appex; do
  [ -d "$appex" ] || continue
  appex_ent="$scratch/$(basename "$appex").entitlements"
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
# Signing rewrote the helpers, so the release manifest's Mac checksums, taken
# at build time, no longer match them. The daemon checks those checksums
# before it installs its tools, so put the signed files' ones in before the
# app's signature seals the manifest.
manifest="$app/Contents/Resources/manifest.json"
if [ -f "$manifest" ]; then
  for helper in "$app/Contents/Helpers/"*; do
    name=$(basename "$helper")
    key="platforms.darwin-arm64.$name.sha256"
    plutil -extract "$key" raw "$manifest" >/dev/null 2>&1 || continue
    plutil -replace "$key" -string "$(shasum -a 256 "$helper" | cut -d' ' -f1)" "$manifest"
  done
  for helper in "$app/Contents/Helpers/"*; do
    name=$(basename "$helper")
    listed=$(plutil -extract "platforms.darwin-arm64.$name.sha256" raw "$manifest" 2>/dev/null) || continue
    [ "$listed" = "$(shasum -a 256 "$helper" | cut -d' ' -f1)" ] ||
      fail "the manifest's checksum for $name doesn't match the signed file, so the daemon would refuse to install it. Nothing was uploaded; look at how manifest.json names the Helpers."
  done
fi
sign "$app"
codesign --verify --deep --strict "$app" || fail "the app's signature doesn't verify after signing; see codesign's message above."
# The daemon's own requirement (daemon/app_peer/src/sys.rs), with the team it
# was built with: a signature that misses it would ship an app the daemon refuses.
requirement="anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13] and certificate leaf[subject.OU] = \"$team\" and identifier \"$bundle_id\""
codesign --verify -R="$requirement" "$app" ||
  fail "the signed app doesn't meet the daemon's requirement ($bundle_id, Developer ID of team $team), so the daemon wouldn't take it as its user. Sign with a Developer ID Application certificate of team $team, or set LIFR_TEAM_ID to the certificate's team and release again."

echo "Notarizing (this takes a few minutes)…"
key="$scratch/key"
mkdir "$key"
chmod 700 "$key"
security find-generic-password -s "$service" -w | base64 -d | tail -n +3 >"$key/key.p8"
key_id=$(security find-generic-password -s "$service" -w | base64 -d | sed -n 1p)
issuer=$(security find-generic-password -s "$service" -w | base64 -d | sed -n 2p)
ditto -c -k --keepParent "$app" "$scratch/notarize.zip"
xcrun notarytool submit "$scratch/notarize.zip" --key "$key/key.p8" --key-id "$key_id" --issuer "$issuer" \
  --wait --output-format plist >"$scratch/notarized.plist" || true
submission=$(plutil -extract id raw "$scratch/notarized.plist" 2>/dev/null || true)
status=$(plutil -extract status raw "$scratch/notarized.plist" 2>/dev/null || true)
if [ "$status" != Accepted ]; then
  if [ -n "$submission" ]; then
    xcrun notarytool log "$submission" --key "$key/key.p8" --key-id "$key_id" --issuer "$issuer" >&2 || true
  fi
  rm -rf "$key"
  fail "Apple didn't notarize the app (status: ${status:-no reply}). Its log, above, names each file it refused and why."
fi
rm -rf "$key"
xcrun stapler staple "$app"

assets="$scratch/assets"
mkdir "$assets"
(cd "$scratch" && ditto -c -k --keepParent "Humata 2.app" "$assets/Humata2-$version.zip")
for name in humata humatad2 humata2_holder stack humata2_fusefs; do
  cp "$app/Contents/Helpers/$name" "$assets/$name-darwin-arm64"
done
(cd "$assets" && shasum -a 256 -- * >SHA256SUMS)

echo "Uploading a draft release ${tag}…"
notes="Humata 2 $version, built from $commit. Signed with Developer ID and notarized by Apple.

- Humata2-$version.zip: the Mac app (Apple silicon), with humatad2, humata2_holder, humata, stack and humata2_fusefs inside. Move Humata 2.app into Applications and open it; it installs its tools into ~/.humata2 and starts the daemon as a login agent.
- humata-darwin-arm64, humatad2-darwin-arm64, humata2_holder-darwin-arm64, stack-darwin-arm64, humata2_fusefs-darwin-arm64: the command line, the daemon, the terminal holder, the stacked-PR command line and the FUSEFS helper on their own, the same files as in the app.
- SHA256SUMS: \`shasum -a 256 -c SHA256SUMS\` checks the downloads."
gh release create "$tag" "$assets"/* --draft --target "$commit" --title "Humata 2 $version" --notes "$notes"

echo "Checking the draft as people will download it…"
if sh "$here/check.sh" "$tag"; then
  gh release edit "$tag" --draft=false
  echo "Published $tag. Install it with: sh $here/install.sh $tag"
else
  fail "the check failed, so $tag stays a draft. Fix what it says, then delete the draft (gh release delete $tag) and release again; nobody else could see it."
fi

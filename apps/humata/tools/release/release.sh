#!/bin/sh
# Releases Humata on GitHub: builds it from the commit checked out, signs
# the app and every Mac binary in it with the Developer ID certificate and
# the hardened runtime, notarizes it with the App Store Connect key the
# TestFlight uploads keep in the Keychain (see
# apps/lifr/tools/testflight/setup.sh), staples the ticket and uploads a
# draft release. check.sh then downloads the draft as people would get it
# and runs it; only if that passes is the release published.
#
# Run from anywhere in the repository:
#   apps/humata/tools/release/release.sh 0.1.0
#
# GitHub access is the GitHub command line's (`gh auth login`). The key is
# only ever on a pipe, except while notarytool reads it from a file in a
# folder only you can open, deleted as soon as the upload is done.
set -eu
fail() {
  echo "release: $*" >&2
  exit 1
}
[ $# -eq 1 ] || fail "usage: apps/humata/tools/release/release.sh <version>, such as 0.1.0"
version=$1
printf '%s' "$version" | grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+' ||
  fail "\"$version\" isn't a version: give three numbers, such as 0.1.0."
root=$(git rev-parse --show-toplevel)
cd "$root"
here=apps/humata/tools/release
tag="humata-v$version"
service=lifr.app-store-connect

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
  fail "there's no Developer ID Application certificate in the Keychain. Make one in Xcode › Settings › Accounts › Manage Certificates; an Apple Development certificate won't do, since Gatekeeper only accepts Developer ID outside the App Store."
security find-generic-password -s "$service" >/dev/null 2>&1 ||
  fail "there's no App Store Connect key in the Keychain to notarize with. Store one first: bazel run //apps/lifr/tools/testflight:setup -- <.p8> <Key ID> <Issuer ID>"

sh tools/bootstrap/bazelisk.sh
bazel="$root/.tools/bin/bazel"
echo "Building Humata $version from ${commit}…"
"$bazel" build --config=xcode --//apps/humata/daemon/cli:admin=false //apps/humata/app:macos
out=$("$bazel" cquery --config=xcode --//apps/humata/daemon/cli:admin=false --output=files //apps/humata/app:macos 2>/dev/null | head -n 1)
[ -n "$out" ] && [ -e "$out" ] || fail "the build finished but its output wasn't found; run: bazel cquery --config=xcode --output=files //apps/humata/app:macos"

scratch=$(mktemp -d)
chmod 700 "$scratch"
trap 'rm -rf "$scratch"' EXIT
case "$out" in
  *.zip) ditto -x -k "$out" "$scratch" ;;
  *.app) ditto "$out" "$scratch/Humata.app" ;;
  *) fail "didn't expect the build to make $out; expected a .app or a .zip." ;;
esac
app="$scratch/Humata.app"
[ -d "$app" ] || fail "$out has no Humata.app in it."
chmod -R u+w "$app"
bundle_id=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$app/Contents/Info.plist")
case "$bundle_id" in
  com.example.*)
    fail "the app was built as $bundle_id, the placeholder used when no identity is set. Set LIFR_TEAM_ID and LIFR_BUNDLE_PREFIX in this checkout's user.bazelrc (see apps/lifr/identity.bzl), then release again; a placeholder ID would change once fixed, and people's settings and login item would be left behind." ;;
esac
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$(git rev-list --count HEAD)" "$app/Contents/Info.plist"

# Inside out: everything an Apple tool counts as code, then the app. The
# Mac builds of humatad under Resources/builds are code too, and notarization
# refuses anything in the app that isn't signed this way.
echo "Signing with ${identity}…"
sign() {
  codesign --force --timestamp --options runtime --sign "$identity" "$@"
}
find "$app/Contents" -type f ! -path "*/MacOS/*" | while IFS= read -r file; do
  if file -b "$file" | grep -q '^Mach-O'; then sign "$file"; fi
done
# Keeps any entitlements the build gave the app.
sign --preserve-metadata=entitlements "$app"
codesign --verify --deep --strict "$app" || fail "the app's signature doesn't verify after signing; see codesign's message above."

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
ditto -c -k --keepParent "$app" "$assets/Humata-$version.zip"
cp "$app/Contents/Helpers/humata" "$assets/humata-darwin-arm64"
cp "$app/Contents/Helpers/humatad" "$assets/humatad-darwin-arm64"
cp "$app/Contents/Resources/builds/humatad-linux-aarch64" "$assets/humatad-linux-aarch64"
cp "$app/Contents/Resources/builds/humatad-linux-x86_64" "$assets/humatad-linux-x86_64"
(cd "$assets" && shasum -a 256 -- * >SHA256SUMS)

echo "Uploading a draft release ${tag}…"
notes="Humata $version, built from $commit. Signed with Developer ID and notarized by Apple.

- Humata-$version.zip: the Mac app (Apple silicon), with humata and humatad inside.
- humata-darwin-arm64, humatad-darwin-arm64: the command line and the daemon on their own.
- humatad-linux-aarch64, humatad-linux-x86_64: the daemon for Linux hosts. The app installs these itself; they're here to check or install by hand.
- SHA256SUMS: \`shasum -a 256 -c SHA256SUMS\` checks the downloads."
gh release create "$tag" "$assets"/* --draft --target "$commit" --title "Humata $version" --notes "$notes"

echo "Checking the draft as people will download it…"
if sh "$here/check.sh" "$tag"; then
  gh release edit "$tag" --draft=false
  echo "Published $tag."
else
  fail "the check failed, so $tag stays a draft. Fix what it says, then delete the draft (gh release delete $tag) and release again; nobody else could see it."
fi

#!/bin/sh
# Checks a Humata 2 release as people get it. It downloads the release into an
# empty folder and marks each file as downloaded from the internet
# (quarantined, as a browser does). It then checks that:
# - every file matches SHA256SUMS;
# - Gatekeeper accepts the app and the Mac binaries as notarized Developer ID
#   code, and the app's ticket is stapled;
# - the app has the hardened runtime and no entitlements, which the daemon
#   requires (D135);
# - the app starts and says the release's version (`--version`, which
#   installs nothing);
# - the binaries on their own are the ones in the app, and they run.
# Opens no windows.
#
#   apps/humata2/tools/release/check.sh humata2-v0.1.0
#
# Works on drafts too, for whoever can see them.
set -eu
fail() {
  echo "check: $*" >&2
  exit 1
}
[ $# -eq 1 ] || fail "usage: apps/humata2/tools/release/check.sh <tag>, such as humata2-v0.1.0"
tag=$1
command -v gh >/dev/null 2>&1 || fail "the GitHub command line isn't installed; install it with \`brew install gh\`, then run \`gh auth login\`."

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
downloads="$scratch/downloads"
mkdir "$downloads"
gh release download "$tag" --dir "$downloads" ||
  fail "couldn't download $tag; check the tag with \`gh release list\` and that \`gh auth status\` is signed in."
cd "$downloads"

echo "Checking the files against SHA256SUMS…"
[ -f SHA256SUMS ] || fail "$tag has no SHA256SUMS, so its files can't be checked."
shasum -a 256 -c SHA256SUMS || fail "a file doesn't match SHA256SUMS: the upload is damaged or was replaced. Don't use it; release again."

quarantine="0081;$(printf '%x' "$(date +%s)");Safari;"
for asset in *; do xattr -w com.apple.quarantine "$quarantine" "$asset"; done
zip=$(ls Humata2-*.zip 2>/dev/null | head -n 1)
[ -n "$zip" ] || fail "$tag has no Humata2-<version>.zip."
mkdir "$scratch/app"
ditto -x -k "$zip" "$scratch/app"
app="$scratch/app/Humata 2.app"
[ -d "$app" ] || fail "$zip has no \"Humata 2.app\" in it."
# Unzipping in Finder carries the mark onto everything inside.
xattr -r -w com.apple.quarantine "$quarantine" "$app"

echo "Checking the app…"
codesign --verify --deep --strict "$app" || fail "the app's signature doesn't verify: it was changed after signing."
xcrun stapler validate "$app" >/dev/null ||
  fail "the app has no stapled ticket, so a Mac that's offline the first time it opens it would refuse it."
spctl --assess --type execute -vv "$app" 2>&1 | grep -q 'source=Notarized Developer ID' ||
  fail "Gatekeeper doesn't accept the app as notarized Developer ID code: $(spctl --assess --type execute -vv "$app" 2>&1 | tr '\n' ' ')"
codesign --display --verbose=2 "$app" 2>&1 | grep -q 'flags=.*runtime' ||
  fail "the app isn't signed with the hardened runtime, so the daemon would refuse it as its user. Release again; release.sh signs with --options runtime."
if codesign --display --entitlements - "$app" 2>/dev/null | grep -q '<key>'; then
  fail "the app holds entitlements, and the daemon refuses an app that holds any that allow code injection. Release again without them."
fi

# `--version` loads the app as Gatekeeper and the hardened runtime let it,
# then prints the bundle's version and stops: it installs nothing and opens
# no window.
echo "Starting the app once…"
version=${tag#humata2-v}
probe_home="$scratch/home"
mkdir "$probe_home"
set +e
shown=$(HOME="$probe_home" "$app/Contents/MacOS/humata2" --version)
started=$?
set -e
[ "$started" -ne 137 ] ||
  fail "macOS killed the app as it started (exit 137): Gatekeeper or the hardened runtime refused its code. Run spctl --assess -vv on it and check the signing step's output, then release again."
[ "$started" -eq 0 ] ||
  fail "the signed app doesn't start (exit $started), so it would crash when opened. Unzip $zip and run \"Humata 2.app/Contents/MacOS/humata2\" --version to see why (a library refused under the hardened runtime is the usual cause), fix the build, then release again."
[ "$shown" = "Humata 2 $version" ] ||
  fail "the app says \"$shown\", not \"Humata 2 $version\": its Info.plist doesn't carry the release's version as XML. Release again; release.sh sets it before signing."
[ ! -e "$probe_home/.humata2" ] ||
  fail "\`humata2 --version\` installed tools into HOME, so opening the app to check it would change the Mac it runs on. Fix desktop/src/main.rs so --version returns before anything else, then release again."

for name in humata humatad2 humata2_holder stack; do
  binary="$name-darwin-arm64"
  echo "Checking ${binary}…"
  [ -f "$binary" ] || fail "$tag has no $binary."
  chmod +x "$binary"
  codesign --verify --strict "$binary" || fail "$binary's signature doesn't verify."
  spctl --assess --type open --context context:primary-signature -vv "$binary" 2>&1 |
    grep -q 'source=Notarized Developer ID' ||
    fail "Gatekeeper doesn't accept $binary as notarized Developer ID code."
  cmp -s "$binary" "$app/Contents/Helpers/$name" ||
    fail "$binary isn't the $name inside the app, so someone installing by hand would get another build than the app's."
done

echo "Running the downloaded binaries…"
./humatad2-darwin-arm64 --help >/dev/null || fail "the downloaded humatad2 doesn't run."
./humata-darwin-arm64 --help >/dev/null || fail "the downloaded humata doesn't run."
./stack-darwin-arm64 --help >/dev/null || fail "the downloaded stack doesn't run."
echo "$tag checks out: Gatekeeper accepts it, the app starts, and its binaries run."

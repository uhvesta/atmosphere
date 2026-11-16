#!/bin/sh
# Installs a published Humata 2 release into /Applications, as someone
# downloading it would get it. It downloads the release, checks it against
# SHA256SUMS and asks Gatekeeper whether it's notarized Developer ID code. It
# then replaces "/Applications/Humata 2.app" with it. It doesn't open the
# app: the app installs its tools into ~/.humata2 the first time you open it.
#
#   apps/humata2/tools/release/install.sh humata2-v0.1.0
set -eu
fail() {
  echo "install: $*" >&2
  exit 1
}
[ $# -eq 1 ] || fail "usage: apps/humata2/tools/release/install.sh <tag>, such as humata2-v0.1.0"
tag=$1
command -v gh >/dev/null 2>&1 || fail "the GitHub command line isn't installed; install it with \`brew install gh\`, then run \`gh auth login\`."
destination="/Applications/Humata 2.app"

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
gh release download "$tag" --dir "$scratch" --pattern 'Humata2-*.zip' --pattern SHA256SUMS ||
  fail "couldn't download $tag; check the tag with \`gh release list\` and that \`gh auth status\` is signed in."
zip=$(cd "$scratch" && ls Humata2-*.zip 2>/dev/null | head -n 1)
[ -n "$zip" ] || fail "$tag has no Humata2-<version>.zip."
(cd "$scratch" && grep " $zip\$" SHA256SUMS | shasum -a 256 -c -) ||
  fail "$zip doesn't match SHA256SUMS: the download is damaged or was replaced. Don't install it; try again, and if it still fails, release again."
mkdir "$scratch/app"
ditto -x -k "$scratch/$zip" "$scratch/app"
app="$scratch/app/Humata 2.app"
[ -d "$app" ] || fail "$zip has no \"Humata 2.app\" in it."
spctl --assess --type execute "$app" 2>/dev/null ||
  fail "Gatekeeper doesn't accept $tag's app as notarized Developer ID code, so it isn't installed. Run check.sh $tag to see why."

if [ -e "$destination" ]; then
  # A running copy keeps its files open; the new one takes over at the next launch.
  rm -rf "$destination" || fail "couldn't remove the old \"$destination\"; quit Humata 2 and try again."
fi
ditto "$app" "$destination" || fail "couldn't copy the app into /Applications; check that you can write there."
installed=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$destination/Contents/Info.plist")
echo "Installed Humata 2 $installed in $destination. Open it to install its tools and start the daemon."

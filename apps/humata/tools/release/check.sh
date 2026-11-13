#!/bin/sh
# Checks a Humata release as people get it: downloads it into an empty
# folder, marks each file as downloaded from the internet (quarantined, as a
# browser does), then checks that Gatekeeper accepts the app and the Mac
# binaries as notarized Developer ID code, the app's ticket is stapled,
# every file matches SHA256SUMS, the Linux daemons are the ones in the app,
# and the downloaded humatad runs a daemon the downloaded humata talks to.
# Opens no windows: the app itself isn't launched.
#
#   apps/humata/tools/release/check.sh humata-v0.1.0
#
# Works on drafts too, for whoever can see them.
set -eu
fail() {
  echo "check: $*" >&2
  exit 1
}
[ $# -eq 1 ] || fail "usage: apps/humata/tools/release/check.sh <tag>, such as humata-v0.1.0"
tag=$1
command -v gh >/dev/null 2>&1 || fail "the GitHub command line isn't installed; install it with \`brew install gh\`, then run \`gh auth login\`."

scratch=$(mktemp -d)
daemon=
cleanup() {
  if [ -n "$daemon" ]; then kill "$daemon" 2>/dev/null || true; fi
  rm -rf "$scratch"
}
trap cleanup EXIT
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
zip=$(ls Humata-*.zip 2>/dev/null | head -n 1)
[ -n "$zip" ] || fail "$tag has no Humata-<version>.zip."
mkdir "$scratch/app"
ditto -x -k "$zip" "$scratch/app"
app="$scratch/app/Humata.app"
[ -d "$app" ] || fail "$zip has no Humata.app in it."
# Unzipping in Finder carries the mark onto everything inside.
xattr -r -w com.apple.quarantine "$quarantine" "$app"

echo "Checking the app…"
codesign --verify --deep --strict "$app" || fail "the app's signature doesn't verify: it was changed after signing."
xcrun stapler validate "$app" >/dev/null ||
  fail "the app has no stapled ticket, so a Mac that's offline the first time it opens it would refuse it."
spctl --assess --type execute -vv "$app" 2>&1 | grep -q 'source=Notarized Developer ID' ||
  fail "Gatekeeper doesn't accept the app as notarized Developer ID code: $(spctl --assess --type execute -vv "$app" 2>&1 | tr '\n' ' ')"

for binary in humata-darwin-arm64 humatad-darwin-arm64; do
  echo "Checking ${binary}…"
  chmod +x "$binary"
  codesign --verify --strict "$binary" || fail "$binary's signature doesn't verify."
  spctl --assess --type open --context context:primary-signature -vv "$binary" 2>&1 |
    grep -q 'source=Notarized Developer ID' ||
    fail "Gatekeeper doesn't accept $binary as notarized Developer ID code."
done
for linux in aarch64 x86_64; do
  cmp -s "humatad-linux-$linux" "$app/Contents/Resources/builds/humatad-linux-$linux" ||
    fail "humatad-linux-$linux isn't the build inside the app, so a host would be given another daemon than this one."
done

echo "Running the downloaded daemon and command line…"
# Socket paths are limited to about 100 bytes.
run=$(mktemp -d /tmp/humata-check.XXXXXX)
socket="$run/d.sock"
printf '{"hosts": {}}' >"$run/hosts.json"
./humatad-darwin-arm64 daemon --state "$run/state" --cache "$run/cache" --socket "$socket" \
  --hosts "$run/hosts.json" >"$run/daemon.log" 2>&1 &
daemon=$!
tries=0
until ./humata-darwin-arm64 --socket "$socket" sessions >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ $tries -ge 100 ] || ! kill -0 "$daemon" 2>/dev/null; then
    cat "$run/daemon.log" >&2
    rm -rf "$run"
    fail "the downloaded humatad didn't start a daemon the downloaded humata could reach; its output is above."
  fi
  sleep 0.1
done
git init -q "$run/repository"
git -C "$run/repository" -c user.name=check -c user.email=check@localhost commit -q --allow-empty -m check
project=$(./humata-darwin-arm64 --socket "$socket" project add --name check --repository "$run/repository")
[ -n "$project" ] || fail "the daemon didn't add a project."
echo "Checking that humata mcp has no admin tools…"
list='{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
for humata in ./humata-darwin-arm64 "$app/Contents/Helpers/humata"; do
  tools=$(echo "$list" | "$humata" --socket "$socket" mcp)
  echo "$tools" | grep -q '"propose_task"' || fail "$humata mcp didn't list its tools: $tools"
  if echo "$tools" | grep -qE '"(apply_proposal|create_item|delete_item|approval_status)"'; then
    fail "$humata was built with admin tools (--config=humata-admin), which are for dev builds only. Don't publish this release; build it again without that config."
  fi
done
kill "$daemon"
daemon=
rm -rf "$run"
echo "$tag checks out: Gatekeeper accepts it, and its daemon and command line run."

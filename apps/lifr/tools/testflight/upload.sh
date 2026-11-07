#!/bin/sh
# Builds an iPhone app for the App Store and uploads it to TestFlight.
# Makes what's missing on the way: the bundle IDs, an Apple Distribution
# certificate in a keychain of its own, and the App Store profiles. The API
# key comes from the Keychain (see setup.sh) and is only ever on a pipe,
# except for the upload itself: altool reads it from a file, which lives in
# a folder only you can open and is deleted when this ends.
#
# Each app's tools/testflight:upload passes, in order: the team, the asc
# tool, the app's name, the --config that signs it for the App Store, its
# ios_application label, the prefix of its --embed_label, then for the app
# and each of its extensions "<bundle ID>|<name>|<profile name>|<profile
# file>", the app first.
set -eu

service=lifr.app-store-connect
profiles="$HOME/Library/MobileDevice/Provisioning Profiles"

fail() {
  echo "upload: $*" >&2
  exit 1
}

[ $# -ge 7 ] || fail "run it with bazel run --config=xcode //apps/<app>/tools/testflight:upload"
team=$1
asc="$PWD/$2"
app=$3
config=$4
target=$5
prefix=$6
shift 6
bundle=${1%%|*}
[ "$team" != none ] || fail "set LIFR_TEAM_ID and LIFR_BUNDLE_PREFIX in user.bazelrc first (see apps/lifr/identity.bzl)."
security find-generic-password -s "$service" >/dev/null 2>&1 ||
  fail "there's no App Store Connect key in the Keychain. Run setup first: bazel run //apps/lifr/tools/testflight:setup -- <.p8> <Key ID> <Issuer ID>"

key() { security find-generic-password -s "$service" -w | base64 -d; }
api() { key | "$asc" "$@"; }

scratch=$(mktemp -d)
chmod 700 "$scratch"
trap 'rm -rf "$scratch"' EXIT

echo "Registering the bundle IDs…"
for signed in "$@"; do
  rest=${signed#*|}
  api register "${signed%%|*}" "${rest%%|*}" >/dev/null
done

# Signing uses a keychain of its own, so codesign never waits on a
# Keychain prompt: its key is open to codesign from the start, and the
# keychain's random password sits in the login keychain, readable only
# by security. Passwords go to security on stdin, never on the command line.
signing="$HOME/Library/Keychains/lifr-signing.keychain-db"
signing_service=lifr.signing-keychain
signing_password() { security find-generic-password -s "$signing_service" -w; }
if [ ! -f "$signing" ]; then
  echo "Making a keychain for signing…"
  password=$(/usr/bin/openssl rand -hex 24)
  printf 'add-generic-password -U -s %s -a lifr -l "Lifr signing keychain" -T /usr/bin/security -w %s\n' \
    "$signing_service" "$password" | security -i >/dev/null
  printf 'create-keychain -p %s "%s"\n' "$password" "$signing" | security -i >/dev/null
  security set-keychain-settings "$signing"
  # Keep the keychains already searched, and add this one.
  eval "set -- $(security list-keychains -d user | tr '\n' ' ')"
  security list-keychains -d user -s "$@" "$signing"
  unset password
fi
printf 'unlock-keychain -p %s "%s"\n' "$(signing_password)" "$signing" | security -i >/dev/null ||
  fail "couldn't unlock $signing. Delete it and its \"Lifr signing keychain\" password in Keychain Access, then run this again to make new ones."

echo "Finding the Apple Distribution certificate…"
identities=$(security find-identity -v -p codesigning "$signing" | awk -v team="($team)" \
  'index($0, "\"Apple Distribution: ") && index($0, team) { print $2 }')
certificate=$(api certificates | while read -r id sha1; do
  if printf '%s\n' "$identities" | grep -qx "$sha1"; then
    echo "$id"
    break
  fi
done)
if [ -z "$certificate" ]; then
  echo "None in the signing keychain: making one."
  /usr/bin/openssl genrsa -out "$scratch/key.pem" 2048 2>/dev/null
  /usr/bin/openssl req -new -key "$scratch/key.pem" -out "$scratch/request.csr" -subj "/CN=Lifr TestFlight"
  certificate=$(api create-certificate "$scratch/request.csr" "$scratch/certificate.cer")
  /usr/bin/openssl x509 -inform der -in "$scratch/certificate.cer" -out "$scratch/certificate.pem"
  pass=$(/usr/bin/openssl rand -hex 16)
  PASS=$pass /usr/bin/openssl pkcs12 -export -inkey "$scratch/key.pem" -in "$scratch/certificate.pem" \
    -out "$scratch/identity.p12" -passout env:PASS
  printf 'import "%s" -k "%s" -f pkcs12 -P %s -T /usr/bin/codesign\n' "$scratch/identity.p12" "$signing" "$pass" |
    security -i >/dev/null
  rm -f "$scratch/key.pem" "$scratch/identity.p12"
  unset pass
  printf 'set-key-partition-list -S apple-tool:,apple:,codesign: -s -k %s "%s"\n' "$(signing_password)" "$signing" |
    security -i >/dev/null
fi

echo "Getting the App Store profiles…"
mkdir -p "$profiles"
for signed in "$@"; do
  rest=${signed#*|}
  rest=${rest#*|}
  api profile "${rest%%|*}" "${signed%%|*}" "$certificate" "$profiles/${rest#*|}.mobileprovision"
done

echo "Checking the app is in App Store Connect…"
api app "$bundle" >/dev/null

day=$(date -u +%Y%m%d)
time=$(date -u +%H%M | sed 's/^0*//; s/^$/0/')
build="$day.$time"
echo "Building $app ($build)…"
cd "$BUILD_WORKSPACE_DIRECTORY"
label="${prefix}_${day}_$time"
bazel build --config="$config" --embed_label="$label" "$target"
ipa="$PWD/$(bazel cquery --config="$config" --embed_label="$label" --output=files "$target" 2>/dev/null |
  grep '\.ipa$')"

echo "Uploading…"
mkdir -m 700 "$scratch/private_keys"
{ key | tail -n +3; } >"$scratch/private_keys/AuthKey_$(key | sed -n 1p).p8"
chmod 600 "$scratch/private_keys/"*
# altool looks for the key in ./private_keys.
# altool exits 0 even when Apple refuses the build, so read what it says.
(cd "$scratch" && xcrun altool --upload-app --type ios --file "$ipa" \
  --apiKey "$(key | sed -n 1p)" --apiIssuer "$(key | sed -n 2p)" 2>&1) | tee "$scratch/altool.log"
rm -rf "$scratch/private_keys"
if grep -q "Failed to upload" "$scratch/altool.log" || ! grep -qi "UPLOAD SUCCEEDED\|No errors uploading" "$scratch/altool.log"; then
  fail "Apple refused build $build; its reasons are above. Fix those and run this again."
fi
echo "Uploaded build $build. Waiting for Apple to process it, usually within 15 minutes…"
# Done only once TestFlight can offer the build, so a build Apple rejects
# after the upload doesn't pass for one that's out.
for _ in $(seq 1 80); do
  state=$(api build "$bundle" "$build")
  case "$state" in
    VALID)
      echo "Build $build is on TestFlight."
      exit 0
      ;;
    FAILED | INVALID)
      fail "Apple couldn't process build $build ($state). App Store Connect › TestFlight says why, and Apple emails the reasons to the account holder. Fix those and run this again."
      ;;
  esac
  sleep 30
done
fail "Build $build is still processing after 40 minutes. Check App Store Connect › TestFlight; it often finishes later, so there's no need to upload again unless it fails."

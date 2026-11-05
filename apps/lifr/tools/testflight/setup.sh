#!/bin/sh
# Stores an App Store Connect API key in the login Keychain for upload.sh:
# its Key ID, Issuer ID and .p8 in one item that only /usr/bin/security can
# read. The key goes to `security` on standard input, so it's never in a
# command line other processes can see, and never printed.
set -eu

service=lifr.app-store-connect

fail() {
  echo "setup: $*" >&2
  exit 1
}

[ $# -eq 3 ] || fail "usage: setup <path to the .p8> <Key ID> <Issuer ID>, both IDs from App Store Connect › Users and Access › Integrations"
p8=$1
key_id=$2
issuer=$3
case $p8 in
  /*) ;;
  *) p8="${BUILD_WORKING_DIRECTORY:-$PWD}/$p8" ;;
esac
[ -r "$p8" ] || fail "can't read $p8. Give the path to the .p8 you downloaded from App Store Connect."
grep -q -- "-----BEGIN PRIVATE KEY-----" "$p8" || fail "$p8 isn't an App Store Connect .p8 key."
printf '%s' "$key_id" | grep -Eqx '[A-Z0-9]{10}' ||
  fail "\"$key_id\" isn't a Key ID: it's 10 capital letters and digits."
printf '%s' "$issuer" | grep -Eqx '[0-9a-f-]{36}' ||
  fail "\"$issuer\" isn't an Issuer ID: it's the UUID shown above the keys."

# printf is a shell builtin: the payload stays in this process until
# `security` reads it from the pipe.
printf 'add-generic-password -U -s %s -a %s -l %s -T /usr/bin/security -w %s\n' \
  "$service" "$key_id" "$service" \
  "$({ printf '%s\n%s\n' "$key_id" "$issuer" && cat "$p8"; } | base64)" |
  security -i >/dev/null 2>&1 || true

if security find-generic-password -s "$service" -w 2>/dev/null | base64 -d 2>/dev/null | tail -n +3 | cmp -s - "$p8"; then
  echo "Stored the key in the login Keychain as \"$service\"."
  echo "upload doesn't need $p8 any more: once the first upload works, delete it, keeping a copy"
  echo "somewhere safe if you want one. App Store Connect won't let you download it again."
else
  fail "the key couldn't be stored in the login Keychain. Unlock the login keychain in Keychain Access and run setup again."
fi

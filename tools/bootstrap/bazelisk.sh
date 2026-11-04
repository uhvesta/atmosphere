#!/bin/sh
# Installs the Bazelisk pinned in versions.lock as .tools/bin/bazel. To update,
# edit versions.toml and run `bazel run //tools/bootstrap/lock`.
set -eu

dir=$(CDPATH='' cd -P "$(dirname "$0")" && pwd)
bin=$(CDPATH='' cd -P "$dir/../.." && pwd)/.tools/bin

case "$(uname -sm)" in
  "Darwin arm64") platform=darwin-arm64 ;;
  "Darwin x86_64") platform=darwin-x86_64 ;;
  "Linux aarch64" | "Linux arm64") platform=linux-arm64 ;;
  "Linux x86_64") platform=linux-x86_64 ;;
  *)
    echo "Bazelisk is pinned only for macOS and Linux on arm64 or x86_64, not $(uname -sm)." >&2
    exit 1
    ;;
esac

# Each line of versions.lock is: component platform sha256 url.
read -r sha256 url <<EOF
$(awk -v p="$platform" '$1 == "bazelisk" && $2 == p { print $3, $4 }' "$dir/versions.lock")
EOF
if [ -z "$url" ]; then
  echo "versions.lock has no Bazelisk for $platform; restore it from git." >&2
  exit 1
fi

checksum() {
  if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d ' ' -f 1
}

if [ -f "$bin/bazelisk" ] && [ "$(checksum "$bin/bazelisk")" = "$sha256" ]; then
  status='already installed'
else
  mkdir -p "$bin"
  curl --fail --location --silent --show-error --retry 3 --output "$bin/bazelisk.part" "$url"
  actual=$(checksum "$bin/bazelisk.part")
  if [ "$actual" != "$sha256" ]; then
    rm -f "$bin/bazelisk.part"
    cat >&2 <<EOF
SHA-256 mismatch for $url
  pinned:     $sha256
  downloaded: $actual
The download may be corrupted, or upstream may have replaced or tampered with
the file. Nothing was installed. Retry once; if it still mismatches, find out
why before re-pinning.
EOF
    exit 1
  fi
  chmod 0755 "$bin/bazelisk.part"
  mv -f "$bin/bazelisk.part" "$bin/bazelisk"
  status=installed
fi

ln -sfn bazelisk "$bin/bazel"
version=${url#*/download/}
echo "Bazelisk ${version%%/*} $status at $bin/bazelisk"

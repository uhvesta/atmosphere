#!/bin/sh
# Cross-compiles //examples/hello/<lang>:hello for every platform in
# //tools/platforms and runs each binary where it belongs: macOS arm64 on
# this machine, Linux in Lima VMs (tools/lima/*.yaml; x86_64 through
# Rosetta). Missing VMs are created and stopped VMs are started.
#
#   bazel run //tools/cross:matrix                 # build and run everything
#   bazel run //tools/cross:matrix -- --build-only # skip running
#   bazel run //tools/cross:matrix -- --lang swift --config linux_arm64
#
# Running needs macOS on Apple silicon (Lima's vz driver with Rosetta).
set -eu

usage() {
  sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

limactl_path=$1
shift

langs="c go rust swift zig"
configs="macos_arm64 linux_arm64 linux_x86_64 linux_arm64_gnu linux_x86_64_gnu"
build_only=false
only_langs=""
only_configs=""
while [ $# -gt 0 ]; do
  case "$1" in
    --build-only) build_only=true ;;
    --lang)
      only_langs="$only_langs $2"
      shift
      ;;
    --config)
      only_configs="$only_configs $2"
      shift
      ;;
    -h | --help) usage ;;
    *)
      echo "unknown argument: $1" >&2
      usage 2
      ;;
  esac
  shift
done
[ -n "$only_langs" ] && langs=$only_langs
[ -n "$only_configs" ] && configs=$only_configs

runfiles=${RUNFILES_DIR:-$0.runfiles}
limactl=$runfiles/$limactl_path
workspace=${BUILD_WORKSPACE_DIRECTORY:?run this with bazel run}
cd "$workspace"
bazel=${BAZEL:-$(command -v bazel || echo "$workspace/.tools/bin/bazel")}

# The Lima VMs mount this directory read-only.
out=/tmp/monty-oxide-cross
mkdir -p "$out"

# name:template:libc. Only glibc VMs run the *_gnu binaries.
vms="mo-ubuntu:ubuntu-24.04:gnu mo-debian:debian-13:gnu mo-alpine:alpine-3.23:musl"

supported() { # lang config
  case "$1:$2" in
    # Swift has no glibc toolchain yet (see tools/rules/swift).
    swift:*_gnu) return 1 ;;
  esac
  return 0
}

expected_arch() { # config -> regex the program must print
  case "$1" in
    *arm64*) echo 'arm64|aarch64' ;;
    *x86_64*) echo 'x86_64|amd64' ;;
  esac
}

report=$(mktemp)
trap 'rm -f "$report"' EXIT
failed=0
record() { # status lang config where detail
  printf '%-5s %-6s %-17s %-10s %s\n' "$1" "$2" "$3" "$4" "$5" >>"$report"
  case "$1" in
    PASS | SKIP) ;;
    *) failed=1 ;;
  esac
}

check() { # lang config where output
  if printf '%s' "$4" | grep -Eq "hello from .* ($(expected_arch "$2"))"; then
    record PASS "$1" "$2" "$3" "$4"
  else
    record FAIL "$1" "$2" "$3" "$4"
  fi
}

ensure_vm() { # name template
  status=$("$limactl" list --format '{{.Status}}' "$1" 2>/dev/null || true)
  case "$status" in
    Running) ;;
    "") "$limactl" start --tty=false --name="$1" "tools/lima/$2.yaml" ;;
    *) "$limactl" start --tty=false "$1" ;;
  esac
}

for config in $configs; do
  targets=""
  for lang in $langs; do
    if supported "$lang" "$config"; then
      targets="$targets //examples/hello/$lang:hello"
    fi
  done
  [ -n "$targets" ] || continue

  echo "==> building$targets for $config" >&2
  # shellcheck disable=SC2086 # targets is a list of labels
  "$bazel" build --config="$config" $targets
  mkdir -p "$out/$config"
  # cquery takes one expression: "//a:hello + //b:hello".
  "$bazel" cquery --config="$config" --output=starlark \
    --starlark:expr='str(target.label).removeprefix("@@") + " " + target.files_to_run.executable.path' \
    "$(echo "$targets" | sed 's/^ //; s/ / + /g')" 2>/dev/null |
    while read -r label path; do
      lang=${label#//examples/hello/}
      install -m 0755 "$path" "$out/$config/${lang%%:*}"
    done
done

if $build_only; then
  echo "built into $out"
  exit 0
fi

if [ "$(uname -s)-$(uname -m)" != Darwin-arm64 ]; then
  echo "running the matrix needs macOS on Apple silicon; use --build-only" >&2
  exit 1
fi

vms_started=false
for config in $configs; do
  for lang in $langs; do
    if ! supported "$lang" "$config"; then
      record SKIP "$lang" "$config" - "no toolchain"
      continue
    fi
    bin=$out/$config/$lang
    if [ "$config" = macos_arm64 ]; then
      check "$lang" "$config" host "$("$bin" 2>&1 || true)"
      continue
    fi
    if ! $vms_started; then
      for vm in $vms; do
        name=${vm%%:*}
        template=${vm#*:}
        ensure_vm "$name" "${template%:*}"
      done
      vms_started=true
    fi
    for vm in $vms; do
      name=${vm%%:*}
      case "$vm:$config" in
        *:musl:*_gnu) continue ;;
      esac
      check "$lang" "$config" "$name" "$("$limactl" shell "$name" "$bin" 2>&1 || true)"
    done
  done
done

cat "$report"
exit "$failed"

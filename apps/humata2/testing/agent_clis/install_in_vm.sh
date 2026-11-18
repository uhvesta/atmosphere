#!/usr/bin/env bash
# shellcheck shell=bash
# Copies the Linux Copilot and Codex CLIs into a Lima VM's ~/.humata2/test-bin/.
#
# Run it as `bazel run //apps/humata2/testing/agent_clis:install_in_vm --
# [VM [arm64|x64]]`. VM defaults to h2-terminal-c1 (the SSH test VM, see
# apps/humata2/terminal/platform_test/SSH_DEMO.md); arch defaults to what the
# VM reports. The first four arguments are the binaries, which the BUILD file
# passes, so a direct caller gives: copilot-arm64 copilot-x64 codex-arm64
# codex-x64 [VM [arch]].
#
# It writes only ~/.humata2/test-bin/copilot and ~/.humata2/test-bin/codex in
# the VM. There is no destination argument, and it never touches ~/.copilot,
# ~/.codex or any other config. Needs limactl on PATH (or H2_LIMACTL).
set -euo pipefail

die() {
  echo "install_in_vm: $*" >&2
  exit 2
}

[[ $# -ge 4 ]] || die "expected the four binary paths first, then an optional VM name and arch. Run it as 'bazel run //apps/humata2/testing/agent_clis:install_in_vm -- [VM [arm64|x64]]'."
copilot_arm64="$1" copilot_x64="$2" codex_arm64="$3" codex_x64="$4"
shift 4
vm="${1:-h2-terminal-c1}"
arch="${2:-}"
[[ $# -le 2 ]] || die "too many arguments. Give at most a VM name and an arch."

[[ "$vm" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || die "'$vm' is not a Lima VM name (letters, digits, '.', '_' and '-' only). Pass the name 'limactl list' shows."
limactl="${H2_LIMACTL:-$(command -v limactl || true)}"
[[ -n "$limactl" && -x "$limactl" ]] || die "limactl isn't installed or isn't on PATH. Install Lima (brew install lima), or set H2_LIMACTL to its path."

status="$("$limactl" list --format '{{.Status}}' "$vm" 2>/dev/null || true)"
[[ "$status" == "Running" ]] || die "the VM '$vm' isn't running (status: ${status:-missing}). Start it with 'limactl start $vm', or create it with ./terminal-ssh-demo.sh."

if [[ -z "$arch" ]]; then
  machine="$("$limactl" shell "$vm" -- uname -m | tr -d '\r')"
  case "$machine" in
    aarch64 | arm64) arch=arm64 ;;
    x86_64 | amd64) arch=x64 ;;
    *) die "the VM '$vm' reports the CPU '$machine', which has no pinned CLIs. Pass arm64 or x64 as the second argument if you know better." ;;
  esac
fi
case "$arch" in
  arm64) copilot="$copilot_arm64" codex="$codex_arm64" ;;
  x64) copilot="$copilot_x64" codex="$codex_x64" ;;
  *) die "unknown arch '$arch'. Use arm64 or x64." ;;
esac
for binary in "$copilot" "$codex"; do
  [[ -f "$binary" ]] || die "'$binary' isn't a file. Run through 'bazel run' so Bazel fetches the pinned CLIs first."
done

home="$("$limactl" shell "$vm" -- sh -c 'printf %s "$HOME"' | tr -d '\r')"
[[ "$home" == /* && "$home" != "/" ]] || die "could not read the VM user's home directory (got '$home'). Check 'limactl shell $vm'."
dir="$home/.humata2/test-bin"
"$limactl" shell "$vm" -- mkdir -p "$dir"

for pair in "copilot:$copilot" "codex:$codex"; do
  name="${pair%%:*}"
  source="${pair#*:}"
  case "$source" in /*) ;; *) source="$PWD/$source" ;; esac
  dest="$dir/$name"
  "$limactl" copy "$source" "$vm:$dest"
  "$limactl" shell "$vm" -- chmod 755 "$dest"
  echo "$vm:$dest"
done

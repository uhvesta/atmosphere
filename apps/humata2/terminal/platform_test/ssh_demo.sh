#!/bin/bash
# shellcheck shell=bash
set -euo pipefail
umask 077
export PATH="/opt/homebrew/bin:$PATH"
vm=h2-terminal-c1
alias=lima-h2-terminal-c1
action=${1:-status}
case "$action" in
  status | start | ssh-config) [[ $# -le 1 ]] || exit 2 ;;
  provision) [[ $# -eq 2 ]] || {
    echo 'Use ./terminal-ssh-demo.sh provision to build the matching Linux holder first.' >&2
    exit 2
  } ;;
  *)
    echo 'Usage: ./terminal-ssh-demo.sh [status|start|provision|ssh-config]' >&2
    exit 2
    ;;
esac
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo 'This dedicated Lima fixture requires an Apple Silicon Mac. Use a separately configured SSH host on other platforms.' >&2
  exit 1
fi
if ! command -v limactl >/dev/null; then
  echo 'Lima is missing. Install Lima on this Mac, then rerun ./terminal-ssh-demo.sh start.' >&2
  exit 1
fi
if ! command -v python3 >/dev/null; then
  echo 'Python 3 is needed to validate Lima metadata. Install it, then rerun this helper.' >&2
  exit 1
fi
metadata=$(mktemp /tmp/h2-ssh-demo.XXXXXX)
trap 'rm -f -- "$metadata"' EXIT
inspect() {
  if ! limactl list --json --filter '.name == "h2-terminal-c1"' >"$metadata"; then
    echo 'Could not inspect the dedicated Lima VM. Check Lima permissions and installation; no VM was changed.' >&2
    exit 1
  fi
}
field() {
  python3 -c 'import json,sys; text=sys.stdin.read().strip(); data=json.loads(text) if text else {}; print(data.get(sys.argv[1], ""))' "$1" <"$metadata"
}
validate() {
  python3 - "$metadata" <<'PY'
import json,sys
with open(sys.argv[1]) as source:
    text=source.read().strip()
if not text:
    raise SystemExit(0)
data=json.loads(text)
config=data.get('config',{})
if (data.get('name')!='h2-terminal-c1' or data.get('arch')!='aarch64'
    or data.get('vmType')!='vz' or data.get('cpus')!=1
    or data.get('memory',0)>1073741824 or data.get('disk',0)>4294967296
    or config.get('mounts',[]) or config.get('ssh',{}).get('forwardAgent',False)):
    sys.exit('The existing h2-terminal-c1 does not match the small, unmounted fixture. Preserve it; inspect its Lima configuration before using this helper.')
PY
}
inspect
validate
state=$(field status)
if [[ "$action" == start || "$action" == provision ]]; then
  if [[ -z "$state" ]]; then
    limactl start --name="$vm" --cpus=1 --memory=0.75 --disk=4 \
      --vm-type=vz --mount-none --containerd=none --tty=false template:debian
  elif [[ "$state" == Stopped ]]; then
    limactl start --tty=false "$vm"
  elif [[ "$state" != Running ]]; then
    echo "The dedicated VM is $state. Let its current operation finish, then rerun; this helper will not restart it." >&2
    exit 1
  fi
  inspect
  validate
  state=$(field status)
fi
if [[ -z "$state" ]]; then
  echo 'The dedicated SSH VM does not exist. Run ./terminal-ssh-demo.sh start.' >&2
  exit 1
fi
config=$(field sshConfigFile)
if [[ "$action" == ssh-config ]]; then
  printf '%s\n' "$config"
  exit 0
fi
if [[ "$state" != Running ]]; then
  echo "The dedicated SSH VM is $state. Run ./terminal-ssh-demo.sh start; saved processes cannot survive a VM shutdown." >&2
  exit 1
fi
ssh_options=(-F "$config" -o BatchMode=yes -o ConnectTimeout=5)
ready=false
for ((attempt = 0; attempt < 30; attempt++)); do
  if /usr/bin/ssh "${ssh_options[@]}" "$alias" /bin/true 2>/dev/null; then
    ready=true
    break
  fi
  [[ "$action" == status ]] && break
  sleep 1
done
if [[ "$ready" != true ]]; then
  echo 'The VM is running but OpenSSH is not ready. Inspect Lima diagnostics, wait, and retry; host-key and authentication checks remain enabled.' >&2
  exit 1
fi
if [[ "$action" == provision ]]; then
  holder=$2
  python3 - "$holder" <<'PY'
import sys
try:
    with open(sys.argv[1],'rb') as source: header=source.read(20)
except OSError as error:
    sys.exit(f'Could not read the built holder: {error}. Use ./terminal-ssh-demo.sh provision to rebuild it through Bazel.')
if len(header)!=20 or header[:6]!=b'\x7fELF\x02\x01' or int.from_bytes(header[18:20],'little')!=183:
    sys.exit('The holder is not an aarch64 Linux ELF executable. Use ./terminal-ssh-demo.sh provision to rebuild it through Bazel.')
PY
  remote=$(/usr/bin/ssh "${ssh_options[@]}" "$alias" mktemp /tmp/h2-terminal-holder.XXXXXX)
  if [[ ! "$remote" =~ ^/tmp/h2-terminal-holder\.[A-Za-z0-9]{6}$ ]]; then
    echo 'The remote temporary holder path was unexpected; no executable was installed.' >&2
    exit 1
  fi
  if ! limactl copy "$holder" "$vm:$remote"; then
    /usr/bin/ssh "${ssh_options[@]}" "$alias" rm -f -- "$remote" || true
    echo 'Could not copy the holder. Check free space and the SSH connection, then rerun provision; the installed holder was preserved.' >&2
    exit 1
  fi
  /usr/bin/ssh "${ssh_options[@]}" "$alias" sh -s -- "$remote" <<'SH'
set -eu
trap 'rm -f -- "$1"' EXIT
chmod 700 "$1"
mv -fT -- "$1" /tmp/h2-terminal-holder
SH
  echo 'Installed the matching holder at /tmp/h2-terminal-holder. Existing running holders were not stopped.'
fi
printf 'OpenSSH ready: %s (Linux aarch64, no host mounts)\nSSH config: %s\n' "$alias" "$config"
printf 'Connect: ssh -F %q %s\n' "$config" "$alias"

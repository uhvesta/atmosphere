#!/bin/bash
# shellcheck shell=bash
# Manages the dedicated Lima VM h2-fuse-r1 used by the FUSE resilience tests
# (//apps/humata2/desktop:live_flows_fuse_vm_test). It is separate from the
# shared h2-terminal-c1 VM and never touches any other VM.
#
# Usage: h2_fuse_r1.sh create | start | status | ssh-config | delete --yes
set -euo pipefail
umask 077

# The name is fixed on purpose: no argument or variable can point this script
# at another VM.
readonly VM=h2-fuse-r1
readonly HOST_ALIAS=lima-h2-fuse-r1
export PATH="/opt/homebrew/bin:$PATH"
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
for candidate in "$script_dir/../../../../.tools/bin" "$PWD/.tools/bin"; do
  [[ -d "$candidate" ]] && PATH="$candidate:$PATH"
done
export PATH
vm_type=${H2_FUSE_VM_TYPE:-vz}

usage() {
  echo 'Usage: h2_fuse_r1.sh create | start | status | ssh-config | delete --yes' >&2
  exit 2
}
[[ $# -ge 1 ]] || usage
action=$1
case "$action" in
  create | start | status | ssh-config) [[ $# -eq 1 ]] || usage ;;
  delete) [[ $# -le 2 ]] || usage ;;
  *) usage ;;
esac

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo 'This Lima fixture needs an Apple Silicon Mac. On other hosts, set up an arm64 Debian SSH host by hand and point FUSE_VM_SSH_CONFIG at it.' >&2
  exit 1
fi
if ! command -v limactl >/dev/null; then
  echo 'limactl was not found. Install Lima (brew install lima) or put it in .tools/bin, then rerun.' >&2
  exit 1
fi
if ! command -v python3 >/dev/null; then
  echo 'Python 3 is needed to read Lima metadata. Install it, then rerun.' >&2
  exit 1
fi

metadata=$(mktemp /tmp/h2-fuse-r1.XXXXXX)
trap 'rm -f -- "$metadata"' EXIT

inspect() {
  if ! limactl list --json --filter '.name == "h2-fuse-r1"' >"$metadata"; then
    echo 'Could not inspect the Lima VM list. Check Lima permissions and installation; no VM was changed.' >&2
    exit 1
  fi
}
field() {
  python3 -c 'import json,sys; t=sys.stdin.read().strip(); d=json.loads(t) if t else {}; print(d.get(sys.argv[1], ""))' "$1" <"$metadata"
}
config_path() { printf '%s/.lima/%s/ssh.config' "$HOME" "$VM"; }
ssh_vm() {
  /usr/bin/ssh -F "$(config_path)" -o BatchMode=yes -o ConnectTimeout=5 "$HOST_ALIAS" "$@"
}

wait_for_ssh() {
  local attempt
  for ((attempt = 0; attempt < 60; attempt++)); do
    if ssh_vm /bin/true 2>/dev/null; then return 0; fi
    sleep 1
  done
  echo "The VM is running but OpenSSH is not ready. Wait and retry; see ~/.lima/$VM/ha.stderr.log. Host-key and authentication checks stay enabled." >&2
  exit 1
}

# Runs inside the VM as root (via sudo). Idempotent.
provision_remote() {
  ssh_vm sudo -n bash -s <<'SH'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
missing=()
# git: the Add machine check refuses a machine without it.
for pkg in fuse3 git vim-tiny less e2fsprogs procps util-linux; do
  dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q 'install ok installed' || missing+=("$pkg")
done
if ((${#missing[@]})); then
  avail=$(df -Pm / | awk 'NR==2{print $4}')
  if ((avail < 300)); then
    echo "Less than 300 MiB free on the VM disk; free space before installing ${missing[*]}." >&2
    exit 1
  fi
  apt-get -o DPkg::Lock::Timeout=60 update
  apt-get -o DPkg::Lock::Timeout=60 install -y --no-install-recommends "${missing[@]}"
fi
# Keep SSH host keys stable across limactl stop --force / start (Lima's cidata
# bumps instance-id on start, which otherwise makes cloud-init regenerate them).
printf 'ssh_deletekeys: false\n' >/etc/cloud/cloud.cfg.d/99-keep-ssh-host-keys.cfg
echo fuse >/etc/modules-load.d/fuse.conf
modprobe fuse
touch /etc/fuse.conf
if ! grep -q '^user_allow_other' /etc/fuse.conf; then
  sed -i 's/^#[[:space:]]*user_allow_other/user_allow_other/' /etc/fuse.conf
  grep -q '^user_allow_other' /etc/fuse.conf || echo user_allow_other >>/etc/fuse.conf
fi
[[ -c /dev/fuse ]] || { echo '/dev/fuse is missing after loading the fuse module; the VM kernel has no FUSE support.' >&2; exit 1; }
touch /var/lib/h2-fuse-r1.provisioned-2
SH
}

print_test_command() {
  cat <<TXT
Run the FUSE VM tests from the repo root:
bazel test //apps/humata2/desktop:live_flows_fuse_vm_test --strategy=TestRunner=local --test_env=FUSE_VM_SSH_CONFIG="\$HOME/.lima/h2-fuse-r1/ssh.config" --test_env=FUSE_VM_SSH_HOST=lima-h2-fuse-r1 --test_env=TERMINAL_LAB_SSH_CONFIG="\$HOME/.lima/h2-terminal-c1/ssh.config"
TXT
}

is_provisioned() { ssh_vm test -e /var/lib/h2-fuse-r1.provisioned-2 2>/dev/null; }

start_vm() {
  inspect
  local state
  state=$(field status)
  if [[ -z "$state" ]]; then
    echo "$VM does not exist. Run: $0 create" >&2
    exit 1
  fi
  if [[ "$state" == Stopped ]]; then
    limactl start --tty=false "$VM"
  elif [[ "$state" != Running ]]; then
    echo "$VM is $state. Let its current operation finish, then rerun; this script will not restart it." >&2
    exit 1
  fi
  wait_for_ssh
}

case "$action" in
  create)
    inspect
    state=$(field status)
    if [[ -z "$state" ]]; then
      # The Debian template image is cached by Lima after the first download
      # (~/Library/Caches/lima/download); the first create needs network once.
      if ! limactl start --name="$VM" --cpus=1 --memory=1 --disk=6 \
        --vm-type="$vm_type" --mount-none --containerd=none --tty=false template:debian; then
        echo "Could not create $VM. If the Debian image could not be fetched, check the network and rerun; Lima caches it afterwards. If vz is unavailable, set H2_FUSE_VM_TYPE=qemu. A half-made VM can be removed with: $0 delete --yes" >&2
        exit 1
      fi
    elif [[ "$state" == Stopped ]]; then
      limactl start --tty=false "$VM"
    elif [[ "$state" != Running ]]; then
      echo "$VM is $state. Let its current operation finish, then rerun." >&2
      exit 1
    fi
    wait_for_ssh
    if is_provisioned; then
      echo "$VM already exists and is set up; nothing to do."
    else
      provision_remote
      echo "$VM is set up."
    fi
    print_test_command
    ;;
  start)
    start_vm
    is_provisioned || { echo "$VM is not set up yet. Run: $0 create" >&2; exit 1; }
    ssh_vm sudo -n modprobe fuse
    print_test_command
    ;;
  status)
    inspect
    state=$(field status)
    if [[ -z "$state" ]]; then
      echo "$VM: does not exist (run: $0 create)"
      exit 1
    fi
    echo "$VM: $state"
    if [[ "$state" != Running ]]; then
      echo "Run: $0 start"
      exit 1
    fi
    if ! ssh_vm /bin/true 2>/dev/null; then
      echo 'OpenSSH: not ready (wait and retry)'
      exit 1
    fi
    ssh_vm 'if [ -c /dev/fuse ]; then echo "/dev/fuse: present"; else echo "/dev/fuse: MISSING"; fi
if dpkg-query -W -f="\${Status}" fuse3 2>/dev/null | grep -q "install ok installed"; then echo "fuse3: installed"; else echo "fuse3: NOT installed"; fi
grep -q "^user_allow_other" /etc/fuse.conf 2>/dev/null && echo "user_allow_other: set" || echo "user_allow_other: NOT set"
echo "free disk: $(df -Pm / | awk "NR==2{print \$4}") MiB"'
    ;;
  ssh-config)
    inspect
    [[ -n "$(field status)" ]] || { echo "$VM does not exist. Run: $0 create" >&2; exit 1; }
    config_path
    printf '\n'
    ;;
  delete)
    if [[ "${2:-}" != --yes ]]; then
      echo "Refusing to delete $VM without --yes. Rerun: $0 delete --yes" >&2
      exit 1
    fi
    inspect
    [[ -n "$(field status)" ]] || { echo "$VM does not exist; nothing to delete."; exit 0; }
    limactl delete --force "$VM"
    echo "Deleted $VM."
    ;;
esac

#!/bin/bash
# shellcheck shell=bash
# This driver touches only a fresh throwaway VM in the test's private home.
set -euo pipefail
umask 077
export PATH="/opt/homebrew/bin:$PATH"
: "$LIMA_HOME"
vm=powerloss
vm_home=$(python3 - "$LIMA_HOME" <<'PY'
from pathlib import Path
import sys
p=Path(sys.argv[1]).resolve()
if p.parent.parent != Path('/private/tmp') or not p.parent.name.startswith('h2-vm-') or p.name!='lima':
    sys.exit('Refusing a Lima home outside the owned /private/tmp/h2-vm-*/lima fixture.')
print(p)
PY
)
export LIMA_HOME="$vm_home"
lima() { env LIMA_HOME="$vm_home" limactl "$@"; }
inspect() {
  lima list --json --filter '.name == "powerloss"' >"$vm_home/metadata.json"
  python3 - "$vm_home/metadata.json" "$vm_home" <<'PY'
import json,sys
from pathlib import Path
text=Path(sys.argv[1]).read_text().strip()
if not text: sys.exit('The owned powerloss VM does not exist.')
x=json.loads(text); c=x.get('config',{}); ssh=c.get('ssh',{})
if (x.get('name')!='powerloss' or Path(x.get('dir','')).resolve()!=Path(sys.argv[2])/'powerloss'
    or x.get('vmType')!='vz' or x.get('arch')!='aarch64' or x.get('cpus')!=1
    or x.get('memory',0)>805306368 or x.get('disk',0)>4294967296
    or c.get('mounts',[]) or ssh.get('forwardAgent',False)
    or ssh.get('loadDotSSHPubKeys',False)):
    sys.exit('Owned VM identity/configuration changed. Preserve its private home; no VM was stopped.')
PY
}
case "$1" in
  create)
    [[ $# -eq 3 ]] || exit 2
    holder=$2
    image=$3
    [[ ! -e "$vm_home" ]] || { echo 'Refusing to reuse an existing Lima home.' >&2; exit 1; }
    mkdir -m700 "$vm_home"
    python3 - "$holder" "$image" "$vm_home/config.yaml" <<'PY'
import json,sys
from pathlib import Path
with Path(sys.argv[1]).open('rb') as source: h=source.read(20)
if len(h)!=20 or h[:6]!=b'\x7fELF\x02\x01' or int.from_bytes(h[18:20],'little')!=183:
    sys.exit('Supply the already Bazel-built aarch64 Linux holder; no nested build is performed.')
image=Path(sys.argv[2]).resolve()
if not image.is_file(): sys.exit('Cached pinned Debian image is absent; prepare it explicitly before this manual test.')
config={
 'vmType':'vz','arch':'aarch64','cpus':1,'memory':'768MiB','disk':'4GiB',
 'images':[{'location':str(image),'arch':'aarch64','digest':'sha512:d8470b8c6c38fead046c794b5800a5a7b96672d5bcf543cc230ceb0c4b8ace05ed341a0c8928045422243fde26b2f1f2f58e99244c65709ffda2e3d4b674dd5a'}],
 'mounts':[],'containerd':{'system':False,'user':False},
 'ssh':{'loadDotSSHPubKeys':False,'forwardAgent':False,'overVsock':False},
 'user':{'name':'fixture','uid':2000,'home':'/home/fixture','shell':'/bin/bash'},
}
Path(sys.argv[3]).write_text(json.dumps(config))
PY
    lima create --tty=false --name="$vm" --mount-none --containerd=none "$vm_home/config.yaml"
    inspect
    lima start --tty=false "$vm"
    inspect
    lima copy "$holder" "$vm:/var/tmp/h2-vm-holder"
    lima shell "$vm" -- /bin/chmod 700 /var/tmp/h2-vm-holder
    # Deployment is setup, not the fault: make its data and directory durable.
    lima shell "$vm" -- /usr/bin/python3 -c '
import os
with open("/var/tmp/h2-vm-holder", "rb") as source:
    os.fsync(source.fileno())
fd=os.open("/var/tmp", os.O_RDONLY|os.O_DIRECTORY)
try: os.fsync(fd)
finally: os.close(fd)
'
    ;;
  metadata)
    [[ $# -eq 1 ]] || exit 2
    inspect
    cat "$vm_home/metadata.json"
    ;;
  poweroff)
    [[ $# -eq 1 ]] || exit 2
    inspect
    lima stop --force "$vm"
    inspect
    ;;
  start)
    [[ $# -eq 1 ]] || exit 2
    inspect
    lima start --tty=false "$vm"
    inspect
    ;;
  delete)
    [[ $# -eq 1 ]] || exit 2
    inspect
    lima delete --force "$vm"
    ;;
  *) echo 'Use create HOLDER CACHED_IMAGE, metadata, poweroff, start, or delete.' >&2; exit 2 ;;
esac

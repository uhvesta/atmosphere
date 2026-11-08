#!/bin/bash
# Lends an iOS test a simulator of its own from a pool of four, for the
# runner's create_simulator_action: it prints the simulator's UDID. Each
# simulator stays booted for the next test, since a new one takes minutes
# to boot for the first time, and is lent to one test at a time, since two
# would install over each other's host app. return.sh gives it back.
#
# The runner gives the device type and OS version in SIMULATOR_DEVICE_TYPE
# and SIMULATOR_OS_VERSION. A lend outlives a test that's killed, such as
# one that times out, until its runner's process is gone.
set -euo pipefail

readonly pool_size=4
readonly locks=/private/tmp/flowkit-simulators
device_type="${SIMULATOR_DEVICE_TYPE:?the runner sets the device type}"
os_version="${SIMULATOR_OS_VERSION:?the runner sets the OS version}"
# The runner, which calls this and return.sh.
runner=$PPID

mkdir -p "$locks"

# Takes slot $1 for the runner, or fails while another live runner has it.
take() {
  local lock="$locks/$device_type $os_version #$1"
  if mkdir "$lock" 2>/dev/null; then
    echo "$runner" >"$lock/runner"
    return 0
  fi
  local owner
  owner="$(cat "$lock/runner" 2>/dev/null || true)"
  # A runner that's gone, such as one killed when its test timed out.
  if [[ -n "$owner" ]] && ! kill -0 "$owner" 2>/dev/null; then
    echo "$runner" >"$lock/runner"
    # Another runner may have found it gone at the same moment.
    [[ "$(cat "$lock/runner")" == "$runner" ]]
    return
  fi
  return 1
}

slot=""
for _ in $(seq 1800); do
  for k in $(seq "$pool_size"); do
    if take "$k"; then
      slot=$k
      break 2
    fi
  done
  sleep 1
done
if [[ -z "$slot" ]]; then
  echo "error: all $pool_size $device_type simulators on iOS $os_version stayed lent for 30 minutes." \
    "If no tests are running, remove $locks and run the tests again." >&2
  exit 1
fi

name="FlowKit $device_type $os_version #$slot"
runtime="$(xcrun simctl list runtimes -j |
  /usr/bin/python3 -I -c 'import json, sys
version = sys.argv[1]
for runtime in json.load(sys.stdin)["runtimes"]:
    if runtime["platform"] == "iOS" and runtime["isAvailable"] and runtime["version"] == version:
        print(runtime["identifier"])
        break' "$os_version")"
if [[ -z "$runtime" ]]; then
  echo "error: no iOS $os_version simulator runtime is installed. Install it in Xcode's Settings › Components, then run the tests again." >&2
  rm -rf "$locks/$device_type $os_version #$slot"
  exit 1
fi

udid="$(xcrun simctl list devices -j |
  /usr/bin/python3 -I -c 'import json, sys
runtime, name = sys.argv[1:]
for device in json.load(sys.stdin)["devices"].get(runtime, []):
    if device["name"] == name:
        print(device["udid"])
        break' "$runtime" "$name")"
if [[ -z "$udid" ]]; then
  udid="$(xcrun simctl create "$name" "$device_type" "$runtime")"
  echo "Created simulator '$name' ($udid)" >&2
else
  echo "Lent simulator '$name' ($udid)" >&2
fi
# Waits for it to boot, booting it when it's shut down.
xcrun simctl bootstatus "$udid" -b >&2
echo "$udid"

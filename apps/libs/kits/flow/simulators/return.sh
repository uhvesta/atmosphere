#!/bin/bash
# shellcheck shell=bash
# Gives back the simulator lend.sh lent the runner, for the runner's
# clean_up_simulator_action. It stays booted for the next test.
set -euo pipefail

readonly locks=/private/tmp/flowkit-simulators
# The runner, which called lend.sh too.
runner=$PPID

for lock in "$locks"/*; do
  if [[ "$(cat "$lock/runner" 2>/dev/null || true)" == "$runner" ]]; then
    rm -rf "$lock"
  fi
done

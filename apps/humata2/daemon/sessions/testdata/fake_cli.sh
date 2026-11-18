#!/bin/sh
# A stand-in agent CLI for the sessions unit tests (agents.rs): prints what
# Codex prints for `--version` and what Copilot prints for
# `--no-auto-update --version`, and fails for anything else.
# A `version` file beside this script, when there is one, is what
# `--version` prints (a test changes it to upgrade the CLI).
here=$(dirname "$0")
case "$*" in
  --version) if [ -f "$here/version" ]; then cat "$here/version"; else echo 'codex-cli 0.169.0'; fi ;;
  '--no-auto-update --version') echo 'GitHub Copilot CLI 1.0.112-0.' ;;
  *) exit 1 ;;
esac

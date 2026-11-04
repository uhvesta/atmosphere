#!/bin/sh
# Runs a test binary that prints each check and exits non-zero on a failure.
set -eu
exec "$1"

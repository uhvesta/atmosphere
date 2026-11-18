# RELEASE2: The jank-review fixes are published and installed

- **Status:** blocked
- **Depends on:** VERIFY: publish the jank-review fixes
- **Unblocks:** FAST
- **Layer:** L10 · **Lane:** main

## Outcome
A new signed, notarized Humata 2 release carries everything VERIFY landed. The
same build is installed in /Applications on the owner's Mac.

## Exit criterion
The same as [RELEASE](RELEASE.md): the release exists with the zip and
SHA256SUMS, `check.sh` passes, the staple validates, and the app is installed
with `ditto` without opening it. `--version` matches.

## Steps
Follow RELEASE.md's steps with a new version number, from a clean tree with the
full gate green. Use the same credentials.

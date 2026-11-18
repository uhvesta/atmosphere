# RELEASE3: Fast mode is published and installed

- **Status:** blocked
- **Depends on:** FAST: publish fast mode
- **Unblocks:** nothing; the last milestone
- **Layer:** L12 · **Lane:** main

## Outcome
A new signed, notarized Humata 2 release carries everything FAST landed. The
same build is installed in /Applications on the owner's Mac.

## Exit criterion
The same as [RELEASE](RELEASE.md): the release exists with the zip and
SHA256SUMS, `check.sh` passes, the staple validates, and the app is installed
with `ditto` without opening it. `--version` matches. Fast mode is off by default in the release, and turning it on works in the installed app.

## Steps
Follow RELEASE.md's steps with a new version number, from a clean tree with the
full gate green (with fast mode both on and off). Use the same credentials.

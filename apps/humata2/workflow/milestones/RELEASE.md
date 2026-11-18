# RELEASE: The macOS desktop app is published and installed

- **Status:** blocked
- **Depends on:** V1: release what replaces Humata 1
- **Unblocks:** VERIFY (the final review runs on the released app)
- **Layer:** L8 · **Lane:** main

## Outcome
Anyone can download the signed, notarized Humata 2 from its GitHub release.
The same build is installed in /Applications on the owner's Mac.

## Exit criterion
- The GitHub release exists, with the zip and SHA256SUMS.
- The zip is verified as people get it: `check.sh` passes, and the
  notarization staple validates (`xcrun stapler validate`).
- `Humata 2.app` is copied into /Applications with `ditto`, without opening
  it (`install.sh`). The installed app's version (`--version`) matches the
  release.

## Steps
1. Commit and push on a clean tree; the full gate is green.
2. Build and sign with the same credentials Humata has always used: the
   Developer ID Application certificate and the App Store Connect key in the
   Keychain (`lifr.app-store-connect`). Run
   `apps/humata2/tools/release/release.sh <version>`. It builds the app with
   Bazel, signs it with the hardened runtime, notarizes, staples, and uploads
   a draft release.
3. `apps/humata2/tools/release/check.sh humata2-v<version>` downloads the
   draft, checks it, and the release is published only if it passes.
4. `apps/humata2/tools/release/install.sh humata2-v<version>` installs it into
   /Applications. Don't open the app.

## Read these (pointers, nothing else)
- Code: `apps/humata2/tools/release/{release,check,install}.sh`; Humata 1's
  `apps/humata/tools/release/{release,check}.sh` for the same credentials.
- Decisions: D135 (the daemon checks the app's Developer ID signature), D149
  (the release scripts).
- Work log: the humata2-v0.1.0 entry (2026-11-16) for the last release.

## Out of scope
- Fixing behaviour found after the release: that is VERIFY.
- Performance work: that is FAST.
- Linux, browser and other hosts: cut (D485).

## Lane (written by the manager)
- Design notes:
- Claims: <paths>
- Tasks:
  - [ ] T1 <task> · worker <name> · files … · status
- Waiting on:
- Build gates: <time> pass|fail (targets)
- Ready to commit: no | yes: <what changed, how a reviewer checks it>

# LINUX: The same app on Linux

- **Status:** blocked
- **Depends on:**
  - SAFE: the owner's rule: the Linux batch runs only after all infrastructure work
  - ASK: its boundary suites are part of the batch
  - BUG: its suites are part of the batch
- **Unblocks:** ANYHOST
- **Layer:** L5 · **Lane:** exclusive (nothing else builds)

## Outcome
Run Humata 2 on Linux and get the same screens and the same guarantees.

## Exit criterion
The Linux VM batch (2 CPU, 4 GiB, 20 GiB) is green: every non-image suite, the view-tree snapshots, a launch smoke test (D129), real-SSH machine flows, and VM hard-reset cases.
- The containment suite (SEC-6, from AWAY) passes in the VM. (D128)

## Read these (pointers, nothing else)
- SPEC: PER-18, PER-19, PER-28, TR-2. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: no Linux pixel goldens (D129). The smoke test launches the real binary under X11 and Wayland and asserts frame 01 is non-blank, bundled fonts load with no missing-glyph boxes, and it renders at scale 2. Check first whether GPUI draws in the GPU-less static-musl VM (`testing/linux/run.sh`); it may need glibc and a software renderer.
- Decisions: D18 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `testing/linux/run.sh`, `daemon/machines/providers/lima.toml`, `daemon/machines/testing`, `testing/README.md`
- Also: OVERVIEW.md › N1 item "Check the goldens in a browser"; Known gaps › "Testing infrastructure"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
New features. Fix only what the batch finds.

## Lane (written by the manager)
- Design notes (prep, 2026-11-16; no VM started):
  - One VM run, short: the Mac builds everything (`run.sh --build-only`), the VM only runs tests, two at a time, and Bazel caches passes (D401). Runbook: `testing/linux/BATCH.md`; targets: `testing/linux/batch_targets.txt` (stage A, static musl) and `batch_targets_gui.txt` (stage B, glibc).
  - GPUI on Linux needs glibc plus a sysroot (D402); `engine::headless` is macOS-only today and needs a Linux branch (gpui-pre-wgpu's CosmicTextSystem with `current_headless_renderer`, lavapipe in the VM).
  - Audit findings that are fine on Linux: socket path max (serve.rs), sw_vers/os-release (restart.rs), loginctl linger check (peer.rs), the revive watcher (non-recursive), daemon tests' `ps -o stat=` and `ps -A -ww -o …` (procps reads them the same).
  - Performance note: `session_service/pointers.rs` watches clones recursively; inotify takes one watch per folder, so a big repo can hit `max_user_watches`. It degrades quietly (the watch is skipped); fix only if the batch shows it.
- Claims: testing/linux/**, desktop/src/main.rs (data dir hunks only), desktop/BUILD.bazel (unit_test target)
- Tasks:
  - [x] T1 Runner runs real Bazel tests in the VM (env, args, runfiles, outputs, caching) · manager · testing/linux/{run.sh,in_vm.sh,in_vm_test.sh,BUILD.bazel} · script linux_b1_vm_runner.py
  - [x] T2 Desktop data dir follows XDG off a Mac (D400); app crate unit tests get a target · manager · desktop/src/main.rs, desktop/BUILD.bazel · script linux_b2_desktop_data_dir.py
  - [x] T3 Runbook and target lists · manager · testing/linux/{BATCH.md,batch_targets*.txt} · script linux_docs.py
  - [ ] T4 systemd user service for This Machine's daemon (D407) · desktop/install
  - [ ] T5 Bundled fonts and family mapping off macOS (D404) · desktop/src, ui font names (many ui files; VIEWS/TERM overlap)
  - [ ] T6 glibc sysroot for linux_*_gnu (D402) · MODULE.bazel, bazel/toolchains (SEAM, main)
  - [ ] T7 Linux `engine::headless` · engine/src/lib.rs, engine/BUILD.bazel, Cargo.toml gpui-pre-wgpu (SEAM)
  - [ ] T8 Ungate the non-image desktop suites off macOS (drop `target_compatible_with` macOS, `#![cfg(target_os = "macos")]` in tests/live_*.rs) once T6–T7 land
  - [ ] T9 Landlock containment (D405) · daemon/contain/src/linux.rs
  - [ ] T10 libghostty_vt: gnu Linux archives; its alias selects os+cpu only, so a glibc build gets the musl archive · terminal/vt (TERM)
- Waiting on: SAFE, ASK, BUG (owner's rule); TERM for the holder gaps below; main for T6/T7 SEAMs.
- Holder and terminal gaps for main/TERM (not edited by LINUX):
  - `daemon/holder/src/process.rs`: Linux `ps` truncates `comm=` to 15 bytes, so `humata2_holder_faults` reads `humata2_holder_` and is never matched as a holder. Match on `args=` (first word's basename) or read `/proc/<pid>/exe`.
  - `ps -axo …` in terminal/host/tests/hard_kill.rs, terminal/host/src/agent_resume_ssh_tests.rs, terminal/session/tests/ssh.rs: BSD spelling; `ps -A -o pid=,args=` works on both.
  - terminal/{vt,host,lab,provider,platform_test,client} have macOS-only targets; TERM to say which must run on Linux.
- Build gates: none yet (prep only)
- Ready to commit: no

# Handoff to Gemini: 2026-11-20 ~13:00

Claude Code main hands Humata 2 to a Gemini main agent running
`/dag-walk-gemini resume`.

- Read this file first, then `WORKLOG.md` › Resume here, then `CONFIG.md`.
- The goal stays the same: **ship every release listed below**, install each one, and log it.
- Keep `WORKLOG.md` › Resume here and each milestone's Lane section current as you go. They are the only memory a later session gets.
- The 2026-11-19 handoff that used to be in this file is in git history.

## Where the tree is

- **Last release:** 0.9.0, at 5e86d620. It is installed.
- **Last push:** `0385636a` on `main` and `atmosphere-khodr`. It holds all the 0.10.x lane work so far; some lanes are still open. The owner said to push existing work and put fixes on top: never rewrite or hold back commits.
- **Uncommitted since `0385636a`:** these are the FUSEFSWIRE shell fix (D916). Commit them once its tests pass.
  - `daemon/svc_stop_restart/src/{restart.rs,ports.rs}`
  - `daemon/humatad/src/wire_stop_restart.rs`
  - `DECISIONS.md` (D916)
  - `milestones/FUSEFSWIRE.md`
  - `WORKLOG.md`
- **Never commit:**
  - `apps/humata/proto/plugin/BUILD.bazel`
  - `apps/humata2/desktop/tests/__snapshots__/native/away-left-out-before.png`
  - `.claude/`, `.signals/` and `tools/skills/dag-walk-gemini/`

  During a release, set these aside with a SHA-tagged stash (`git stash push -m <tag> -- <paths>`), then apply them back by SHA.
- **A test run was in flight at handoff:** the D916 targets, then T13. Check the newest `.bq/logs/*.log` and `bazel-testlogs/apps/humata2/desktop/live_flows_fuse_vm_test/test.log` before rerunning.
- **Agents:** none are running. Every lane needs a fresh manager pointed at its milestone file. The Claude agent IDs in older notes don't carry over.

## Releases still owed, in order

Each milestone's file is `milestones/<ID>.md`. Its top half has the outcome and exit criterion; its Lane section says where work stopped and lists the exact targets.

### 0.10.0 (a minor release: full suite, launch check and slow tier)

**FUSEFSWIRE: the Linux FUSE mount**
- What's done:
  - The real mount is in: `libs/fusefs/linux/src/{mount,wire}.rs` and `daemon/fusefs_adapter/src/linux_mount.rs`.
  - The Lima T13 run went 1/6: the mount came up, but sessions had no `HUMATA_FUSE_ROOT`. The cause was that shells weren't given the variable. That is now fixed (D916, uncommitted).
- Next:
  1. Run `//apps/humata2/daemon/svc_stop_restart:svc_stop_restart_unit_test`, `//apps/humata2/daemon/humatad:{humatad2_unit_test,files_without_mount_test,fusefs_restart_test,humatad2}`.
  2. Run T13:
     ```
     tools/bq/bq //apps/humata2/desktop:live_flows_fuse_vm_test -- --strategy=TestRunner=local --test_env=FUSE_VM_SSH_CONFIG="$HOME/.lima/h2-fuse-r1/ssh.config" --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" --test_output=errors
     ```
     Look for the result in `bazel-testlogs/apps/humata2/desktop/live_flows_fuse_vm_test/test.log`.
  3. If T13 passes 6/6, run T12: `presubmit.sh --with-slow`, then `--full`, then `bq --build //apps/humata2/app:app`.
- Run the Linux cross-build **alone**, because bq merges requests and a merged run marks the Mac targets incompatible:
  `tools/bq/bq --build --config=linux_arm64 //apps/humata2/daemon/fusefs_adapter:humata2_fusefs //libs/fusefs/linux:linux`
- Known follow-ups, not blockers:
  - Files made in the mount live only in the helper's memory.
  - The serve loop is single-threaded.

**SCREENPARSE**
- Batch 1 is written but not yet built. Build it, then route the failures to its manager.
- There is an open question in its Lane: a Codex probe saw a new folder under `~/.codex/tmp/arg0-macos/`.
  - Never touch `~/.codex`.
  - Run probes only with scratch HOMEs.
  - Ask Fable whether the probe is safe to rerun.

**TODO2WIRE**
- Batches 3 and 4 are written but not yet built: svc_board automation, PR watcher and session events.
- After they build, tick T2 and T5–T8 in `TODO2.md`.

**BROWSER**
- A1 compiles. A2 (`browser_copilot` fixes) and B1 (the `daemon/mcp` browser operations) are written but not yet built.
- A one-line seam in `api/proto/BUILD.bazel` adds `browser.proto` to `inbox_proto`. Check it.
- The remote half (D891) comes after local works.

**Then, before the release:**
1. Run the full gate.
2. Check `desktop/tests/__snapshots__/native/providers-e-remove-blocked.view.txt`. Commit `0385636a` added a `Tab "Sync"` line to it, from Settings › Sync. Have Fable confirm it is intended, rather than re-recording the snapshot just to make the test pass.
3. Commit each lane's paths.
4. Release, install and log.

### 0.10.1

- **AGENTOFF:** an agent that is switched off is never picked as a default.
  - Its tests are green: `features/app:agent_off_test`, `desk`, `channel_desk`, `pr_review`, `humatad2_unit` and the clients tests.
  - Two hunks remain:
    - B15, the Settings row. It is at scratchpad `agentoff-settings-row.patch`, which may be gone; regenerate it from the AGENTOFF.md Lane. Apply it once `ui/settings` is free.
    - The `serve.rs` re-tell hunk, written exactly in AGENTOFF.md › Lane › Follow-ups. Apply it once BROWSER frees `serve.rs`.
- **D910:** the Desk starts or forks sessions in any channel, with the owner's approval. Done and green.
- **PROVIDERCLI (D906):** not started.

### 0.10.2

- **NOTIFY2 (D912–D914):** To-do asks stay out of the Inbox, notifications ask for permission when switched on, and there are two short sounds. Built.
  - Still to do:
    - the Settings "Play a sound" row, greyed while notifications are off;
    - the "Why?" on a To-do item, in `features/todo` and `ui/workspace/src/todo.rs` (wait for TODO2WIRE to free them);
    - the live check on the installed app.
  - Batch 2 touched `features/app/src/home.rs`, which was outside NOTIFY2's claims. Check that change.
- **NEWCHCLOSE, MACHINEARCHIVE and HIGHLIGHT:** each has its own milestone file.

### Later releases

- **0.10.3, SMFIX-P0 and SMFIX-P1:** from `STATE_MACHINE_BUGS.md`. It also includes the P2 change: an MCP oneOf error names the closest matching shape.
- **0.10.4, STACKGH:** includes outcome 5. In GitHub's native stacked-PR mode it writes no stack or history comments, since GitHub's own data covers that. A PR is a stack of commits, and a stacked PR is a stack of those.
- **0.10.5, FSKITSERVE:** the macOS FSKit mount (D911).
- **0.11.0, WEBTODO:** last of all.

## How to work

### Builds

- Run every build through `bash -c 'export PATH=$PWD/.tools/bin:$PATH; tools/bq/bq [--build] --jobs=6 <targets>'`, from the repo root.
- In zsh, quote targets like `"${H}:target"`.
- Logs are in `.bq/logs/`.
- Never run cargo, `bazel clean` or `bazel shutdown`.
- Repin with `CARGO_BAZEL_REPIN=true bazel mod tidy`.
- Humatad integration test targets are named `<file stem>_test`.

### Release

1. Stash the never-commit files by SHA.
2. Run `GH_TOKEN=$(gh auth token --user uhvesta) apps/humata2/tools/release/release.sh <ver>`.
3. Run `sh apps/humata2/tools/release/install.sh humata2-v<ver>`.
4. Push to both branches:
   ```
   git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push -q origin HEAD:main
   git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push -q origin HEAD:atmosphere-khodr
   ```
5. Log the release in WORKLOG.

### Tests (the owner's emphasis)

Every test must check an outcome a user or caller would notice: given these inputs, this result or this error. Tests here have often been slow or tied to the implementation in the past; don't repeat that.

**What a test checks**
- Write the test first, fail it for the right reason, then make it pass.
- Check behavior, not wording. No asserts on exact message text, call order, or that one function calls another. A test should break when behavior breaks, not when code is refactored.
- One behavior per test, named as a sentence: `a_shell_in_a_session_gets_the_mount_root`.
- No tests that restate the code, check only that something doesn't panic, or snapshot whole structs to "lock in" output.
- Don't re-record goldens just to make a test pass; Fable judges each image change against the signed-off frames.
- A test that fails often for reasons unrelated to the code under test is a bug to fix, not to rerun or mark slow.

**Keeping tests fast**
- **Use the cheapest test that proves the behavior.** Go up a level only when the behavior needs it:
  1. an in-process `rust_test` on the leaf crate;
  2. a reducer test (TestStore);
  3. a scoped daemon profile;
  4. a full daemon;
  5. a live or VM test.
- **Never wait with a fixed sleep, or a poll of 20 ms or more.** Wait on an event or channel, or poll every 2 ms against a generous deadline.
- **Use the narrowest daemon test profile** (`humatad2_test_machines`, `_power_cut`, `_review_remote`, `_away_board`). Use `:humatad2` only for true full-stack tests.
- **Share the harness:** `use humatad_test_common as common;`, never `mod common;`.
- **Fake agent scripts** set `stty -icanon 2>/dev/null`.
- **Multi-machine rigs** pass short retry knobs (`HUMATA2_PEER_RETRY_MS=20`).
- **A new test file in `daemon/humatad/tests/`** builds a whole binary, so first look for an existing file the case fits in.
- **Watch the timings.** After each gate, compare test times in the bq log. A test slower than about 10 s needs a reason in its Lane section, or a faster version.
- **Slow tier:** only true power-cut, VM and live-agent tests go in `tools/tests/slow.txt`, and each needs a note on why.

### Lanes

- Managers plan and review. Workers write code and never run git or bazel.
- Only main builds, tests and commits. Stage explicit paths only: never `add -A`, `commit -a` or a bare stash.
- Each milestone file's Lane section holds the lane's claims; respect them. Shared files take hunks only, and main logs every hand-over.

### Decisions

- Never ask the owner. Fable 5.2 decides anything you'd ask them: design, goldens, approvals, second opinions.
- Log every decision in `DECISIONS.md`. The next free number is **D917**.
- UI lanes copy Fable's signed-off frames in `milestones/mockups/` exactly.

### Hard rules

- Never modify `~/.claude`, `~/.codex`, `~/.copilot` or `~/.gemini`. Tokens go in env only.
- Never change the global git config or the gh account.
- Never end or kill sessions or holders.
- Never kill pid 41874, and never replace launchd `app.humata2.humatad2`.
- Don't reboot, resize or stop the `h2-terminal-c1` VM. Don't reboot `h2-fuse-r1`.
- Headless only.
- Never name outside projects in code, docs or commits.
- Fix every failing test or lint; never skip one as pre-existing.
- Never delete images, uncommitted code or personal documents.
- Approvals already given: copying agent logins into test VMs and isolated homes, SSH to the test VMs, and uploading releases.

### Notes for the owner

They review your work when they're back. Keep notes they can act on:
- one WORKLOG Event log line per gate, commit and release;
- each lane's Lane section is kept current;
- reports name milestones by their outcome sentence, not by ID.

# Gemini handoff: running the Humata 2 DAG as main

Owner, 2026-11-19: "Delegate as much of the rest of the work as possible to gemini." This file lets a Gemini main agent (`/dag-walk-gemini resume`, gemini-4-neon) run the loop end to end: managers, gates, commits, releases. Lane managers read `roles/GEMINI.md`, not this file.

## Read in this order
1. This file.
2. `WORKLOG.md` › Resume here, Roster, Claims (the newest lines are at the end of the Event log).
3. `RESUME.md` › Release plan: the order of work and releases.
4. `CONFIG.md`: build, test, commit, push and never-touch rules.
5. `roles/MAIN.md`; `roles/GEMINI.md` is the brief every lane manager gets.

## State at handoff (2026-11-19 ~16:10)
- **Released and installed:** humata2-v0.3.9, built from 3c4c7346 (INBOXHOOKS + BOARDSIMPLE + DIFFUI round 1). Pushed to origin/main.
- **Uncommitted in the tree, not yet built green:**
  - DIFFUI round 2 (D868 "N repos" header, drawn-order stepping, collapsed Fork dialog D873, missing-agent rows).
  - INBOXHOOKS batch 3 (session_move, inbox_explain, required reason, Home stopped group, global access page, Bulk removed).
  - BOARDSIMPLE board removal (D874: Route::Board, the features/board crate and the kanban page are gone; crates repinned).
  - MASSDELETE (D875 design, D876 daemon/CLI/access). All 16 seams are applied. T1–T9, T11 and T12 are written but not built; T10 (select mode) is partly written; T13 (frames) hasn't started. `milestones/MASSDELETE.md` › Lane › "Handoff state" has the details.
- **The tree does not compile yet.** MASSDELETE's UI and palette code refer to `state.mass_delete`, `AppAction::MassDelete` and two `commands.rs` arms that don't exist yet. **Do this first:** apply the `features/app` wiring listed in MASSDELETE.md › Handoff state:
  - the features/mass_delete dependency in Cargo.toml and BUILD, plus the root members;
  - the AppState field and the AppAction variant;
  - the `App.mass_delete` constructor, the reduce arm and the toast arm;
  - `SelectSessions => EnterEmpty(surface)` and `DeleteAllSessions => OpenAll`;
  - `dialog_holds_keys`, Escape routing, `Show(Listing)` and `SurfaceChanged`.

  Then repin and build the targets that section lists. These gpui names are unchecked: `IconName::Trash`, `ListChecks`, `Role::Button`, `border_1`.
- **Gemini lanes started via agy (brief `roles/GEMINI.md`):**
  - TODOLIVE: BOARDSIMPLE leftovers.
  - INBOXFWD: the INBOXHOOKS AskWhy host forwarding, IN-8 and the file-read preview.
  - REVIEWER: D858 ShellTab and OpenAgentTab.
  - DIFFVIEW: DV-10–DV-19 plus the GOLDENFIX follow-ups.
  - PROGRESS: the remaining tasks.

  - **PROGRESS reported (16:20):** T0–T10 are in the tree with no seams left; it fixed audit bugs. Report: `lane-reports/gemini-progress-2026-11-19.md` (targets list at the end). Tick T0–T5 and T8–T10 once the gate passes.
  - agy said `--mode auto` isn't available on this account, so the lanes ran in default mode. PROGRESS still edited files. If a lane log ends with no report, re-run it.

  - **DIFFVIEW reported (16:35):** report in `lane-reports/gemini-diffview-2026-11-19.md`. Apply its seam (the native pr_session.rs huge-file test). Re-record `diff-stress-pr-huge-file` (the PR file is now a bounded virtual box) and have it judged: should the box be borderless like frame 68, or carry a 1 px hairline?

  - **REVIEWER reported (16:45):** D858 ShellTab plus OpenAgentTab. Report in `lane-reports/gemini-reviewer-2026-11-19.md`. Its seams S1… (starting with the sessions.proto `ShellTab` message and the `Session` field) go in after the MASSDELETE wiring builds. Still open: a run against a real Copilot agent.

  - **TODOLIVE reported (16:50):** report in `lane-reports/gemini-todolive-2026-11-19.md`. Seams 1–3 are required for `ui/workspace/src/todo.rs` to compile; seam 1 is `TodoAction::StartSession` in features/todo. Seam 4 is optional. To judge: should `overlay_new_session.rs` be redrawn to frame b4-3 (with `31-new-session` re-recorded)? Should `StartNewForItem` prefill `first_message` the way `fill_do_next` does?

  - **INBOXFWD reported (17:00):** report in `lane-reports/gemini-inboxfwd-2026-11-19.md`. Seams:
    - seam 1 (`wire_board.rs`) is required, or humatad won't compile;
    - seam 2 is the IN-8 count end to end (inbox.proto, guard, wire_inbox, svc_inbox, clients, inbox_page);
    - seam 3 is optional (forwarding `ReadFileRequest.reason`);
    - seam 4 is a DECISIONS note.

    Then re-judge 206 and 209.
  - **All five lanes have reported.** The next main step: wire MASSDELETE's features/app; apply the required seams (TODOLIVE 1–3, INBOXFWD 1–2, REVIEWER S1–S7, the DIFFVIEW test); run one batched fast gate.

  Each reports files, seam lines, targets and goldens. Save every report under `lane-reports/`. Their logs were in a session scratchpad, which doesn't survive. If a lane's report is lost, read its milestone's Lane section and re-run it.
- **Not yet given out (one Gemini lane after the wiring builds):**
  - the rest of MASSDELETE T10: sidebar select mode in `ui/workspace/src/sidebar.rs`, keys, and the row context-menu item;
  - T13: frames copied exactly from `mockups/massdelete/`, with FRAMES.md;
  - the gaps: no MCP list_changed after a rule change, agents can't pass `delete_unpushed_work`, remote `forks_staying` is lost after a re-plan, and the box-id assumption in `delete_sessions_remote` is unchecked.

## Next steps (the loop)
1. **Gate:**
   - When lanes report, apply their seam lines.
   - Then run one batched build and test, in the background, at first in the fast loop: `tools/bq/bq --fast --jobs=6 <targets>`. In zsh, run it through `bash -c` so a list of targets word-splits.
   - Split the errors per lane by file claims, and send each lane's errors back to a Gemini fixer lane with the same claims.
2. **Lint:** before any commit, run the same targets without `--fast` (the default is lint).
3. **Goldens:**
   - Record them with `PATH=$PWD/.tools/bin:$PATH bazel run --jobs=6 //apps/humata2/desktop:<native|design|panels>_test_record`, then run `look_test`.
   - Every changed golden is judged by a gemini-4-neon-high judge against the signed-off frames and mockups. Copy those frames fully: a golden that drifts from a signed-off frame is rejected, not re-recorded.
   - Frames waiting to be judged:
     - INBOXHOOKS 206 and 209 (re-judge);
     - native 26 and nearby frames (footer gap change);
     - fork-dialog, plus the new fork-dialog-expanded;
     - the two-repo diff families (D868);
     - design 122;
     - the 34 native `.view.txt` that still say `board_item_move` (now `todo_close`);
     - the To-do desk hint.
   - Also: the MASSDELETE frames, and whether design frames 01, 02, 11 and 84–86 should be redrawn to match D868.
   - A Claude main that resumes later may send any of these to Fable 5.2 to re-judge. Mark each Gemini verdict "judged by Gemini" in DECISIONS so that main can find them.
4. **Commit:**
   - Stage by pathspec only (`git add -- <paths>`), then check `git diff --cached --stat`. Never `git commit -a`, never `git add -A`.
   - Never commit `tools/skills/dag-walk-gemini/`, `.claude/`, `.signals/`, `apps/humata/proto/plugin/BUILD.bazel` or `apps/humata2/desktop/tests/__snapshots__/native/away-left-out-before.png`. These are the owner's own edits; leave them in the tree.
   - **No AI attribution in commit messages**: no Co-authored-by, no session or "Generated with" trailers.
   - Subject style: `Humata 2 (minion X + Y): <user-visible outcome sentence> (Dnnn–Dmmm)`.
   - Also update `GAPS_REVIEW.md` and `OVERVIEW.md` as the lanes did.
5. **Push:** `GH_TOKEN=$(gh auth token --user uhvesta) git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push -q origin HEAD:main`. The token lives only in the environment: never write it to a file. Never change the global git config or the gh account.
6. **Release** (next patch is 0.3.10; then follow `RESUME.md`'s order up to 0.4.0 full suite, 0.4.1 AGY, 0.5.0 merged fast-mode-default release per D887, 0.5.1 NOTIFY last):
   - `release.sh` needs a clean tracked tree. Set the two foreign files aside with a tagged stash: `git stash push -m relNNN-foreign -- <the two files>`.
   - Record the SHA from `git stash list --format='%H %gs'`.
   - Run `export GH_TOKEN=$(gh auth token --user uhvesta); apps/humata2/tools/release/release.sh <ver>`.
   - Then `git stash apply <sha>`, and drop that entry by its tag. Never a bare `git stash` or `pop`: other sessions share the stash.
   - Install with `sh apps/humata2/tools/release/install.sh humata2-v<ver>`; it installs to /Applications. Never open the app.
7. Update `WORKLOG.md` (Event log line per gate, commit, release) and each milestone's status in `DAG.md`.

## Rules (the owner's, not all in AGENTS.md)
- **Builds:** Bazel only, via `tools/bq/bq`. Never cargo. Never `bazel clean` or `shutdown`. Repin with `CARGO_BAZEL_REPIN=1 bazel mod tidy`.
- **Don't ask the owner for approvals or design picks:** a gemini-4-neon-high judge decides them, and they're logged as Provisional decisions.
- **Fix every failure:** never skip a red test or lint as pre-existing.
- **Terminals:** never end a running terminal, holder or the owner's sessions. Never kill the terminal host pid 41874. Never replace launchd `app.humata2.humatad2`. Don't reboot or resize the `h2-terminal-c1` VM.
- **Headless only:** no windows over the screen.
- **Deleting:** never delete images, goldens, uncommitted code or personal documents. The orphan board goldens 21–24 stay until the owner decides.
- **Config:** never modify `~/.claude`, `~/.codex` or `~/.copilot`.
- **Outside projects:** never name or link them in code, docs or commits.
- **NOTIFY** is literally the last milestone.

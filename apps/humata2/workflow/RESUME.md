# RESUME: what happens after the SPLIT migration

Written 2026-11-18 20:40. SPLIT runs in the worktree `/Users/avestabarzegar/oss/atmosphere-split` (branch `humata2-split`, from d9a4a66d), due 22:05. Pick up here once it is merged. WORKLOG.md › Resume here still holds the detailed state; this file is the ordered list.

## 0. Land SPLIT

1. Read the coach's report and SPLIT.md › Lane in the worktree.
2. Rebase `humata2-split` onto main.
   - Main has uncommitted work that touches the split crates: the Dismiss-only notice card (`clients/src/model.rs` `Request.dismiss_only`, `clients/live/src/inbox.rs`, `features/inbox`, proto field 23), plus NATIVEFIX's fixes.
   - Commit those on main first, then rebase the split onto them.
3. Gate: `bazel build //apps/humata2/...` plus the fast tier.
   - Record the cache evidence: a 1-line edit in a fake or leaf crate reruns only its dependents.
4. Commit, push and cut a patch release (green build = patch release).
5. Remove the worktree with `git worktree remove ../atmosphere-split`, after the branch is merged.
6. Later SPLIT phases are queued, not dropped. Whatever didn't fit stays in SPLIT.md, picked up after the backlog:
   - the rest of phase 2;
   - phase 3, the `ui` split, including `panel_changes.rs` into `ui/changes`;
   - phase 4, the remaining daemon services, `daemon/sessions` and `clients/live`;
   - phase 5, the proto split.

## 1. Finish the open lanes

- **The redesigned Diff panel fits every width, with no doubled or clipped labels (GOLDENFIX).** These are Fable's rejects from 2026-11-18 21:20; the full verdict is in scratchpad/nativefix/FABLE_VERDICT.md, and copied in milestones/NATIVEFIX.md if the scratchpad is gone.
  - **Narrow footer beside the terminal (design 01/02/15/16/17/30/62/65/84/86):** drop the "1 draft comment" text and give the button `flex_shrink_0`. Edit `desktop/tests/design/session.rs:654-667` and `footer()` in `design/diff_targets.rs`. Also fix the mock's hunk header to show only the context, "impl Client {" (`design/session.rs:553`, `design/comments.rs:61`, `design/diff_targets.rs:118`).
  - **Repo row** (`ui/src/panel_changes.rs` `repo_head` ~2496): the name shrinks with an ellipsis; the chevron and totals don't shrink. **File header** (~1217): repo and folder give way, the file name keeps its room. Fixes diff-stress-long-repo-and-branch.
  - **Binary and mode-only files** say it twice. At `panel_changes.rs:1187-1200`, skip the header note when the file has no code lines.
  - **PR review footer** (68, 68c, 69): the drafts text gets `min_w_0` plus an ellipsis, so Submit review stays inside the padding.
  - **Follow-ups:**
    - The Open in editor ↗ icon is blank (`open_in_editor` ~583).
    - Long names in the file bar get an ellipsis.
    - PR rows need a space around +/−.
    - Back becomes a ghost button (DS-58).
    - Replace "No changes to show" on a listed file with honest text.
    - Cap the sent comment card at 720 pt.
    - `review_code` virtualizes long files; the pr-huge-file case takes 68 s.
  - **Then:** re-record the native frames, run `bazel run //apps/humata2/desktop:design_test_record`, and get Fable's re-check. Run design_test alone; it times out beside the other suites.

- **The UI test suite passes, and only intended screenshot changes are re-recorded (NATIVEFIX).**
  - The native_test hang (`diff_stress::a_pull_request_huge_file_stays_inside_the_panel`) is fixed.
  - Real failures are fixed in code: stack next-layer key, option-z in the comment box, Escape in diff full screen, the queue cases, session_menu.
  - `holder_swap_identity` passes; suspect TESTSPEED's `stty -icanon`.
  - Golden-only changes in native, design and panels are re-recorded in one batch. Fable judges them, and the approved images go to the owner.
- **Split `panel_changes.rs` into submodules** once NATIVEFIX lands, so three lanes stop colliding in one file.
- [x] **A notice that the agent waits in its terminal offers only Dismiss.** Landed in 0.3.6 (83/83 focused tests + unit tests pass).
  - Optional: show "dismissed" rather than "denied" in its list row.
  - Optional: a UI-level test for the button.
- **Reviews show the full name of any shortened label on hover and have measured performance budgets (REVIEWUI).**
  - Gate `native_test review_ui:: pr_session::`.
  - Run the RV-8 budgets with `-c opt --ignored review_ui::budgets::`. Fable accepts the numbers or proposes changes.
- **Slow tests run in minutes, not tens of minutes (TESTSPEED).**
  - Record before and after timings per target.
  - Move the now-fast targets out of `slow.txt`.
  - Then `power_cut_*` and `live_flows`.
- **Live terminals survive moves, renames and upgrades (LIVEHOLDER).**
  - The `move_session_freeze_test` rig.
  - The full `delete_channel_test`, in the slow tier.
- **Agents' questions and waits show up in the Inbox and go away when answered (AGENTS2).** The O1–O4 live checks need the owner at the machine. Queue them for the owner.
- **Progress and stack views match their design frames (PROGRESS, STACKVIEW).** Frames 122–124 and 119–121 go to Fable.
- After each closed lane: gate, commit, push, patch release, install.

## 2. Minor release (0.4)

Do this once every lane above is closed:
1. A full postsubmit (`presubmit.sh --full`, slow tier included).
2. Fix every failure.
3. Release 0.4 and install it.

## 3. NORMALREL: a release that works end to end with fast mode off

1. Walk every user flow in `apps/humata2/flows/` and audit it against the design goldens and DECISIONS.md.
2. Fix what is broken.
3. Run the full postsubmit.
4. Release and install. Send screenshots of the flows working.

## 4. FASTREL: a fast-mode release that matches the normal one

1. Benchmark fast mode against the NORMALREL build, for startup, diff open, scroll and turn latency.
2. Tune it.
3. Every test must pass with fast mode on.
4. Release fast mode as its own release and install it.

## Standing rules while resuming

- Bazel only, never cargo.
- Main owns git, bazel and releases.
- Agents never run git.
- One bazel per worktree.
- Headless only.
- Never end running terminals or holders.
- No AI attribution in commits.
- Never commit `.claude/` or `.nvimlog`.
- Fable decides in place of the owner, including golden picks.
- Report milestones as outcome sentences.

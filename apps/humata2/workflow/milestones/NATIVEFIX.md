# NATIVEFIX

**Outcome:** The desktop UI test suite is green apart from real golden changes, which go to main for Fable.

**Why:** Train 6 (2026-11-18 20:18) failed on native_test (timed out on `diff_stress::a_pull_request_huge_file_stays_inside_the_panel`, about 45 failed cases), design_test and panels_test (golden-only), and holder_swap_identity_test.

**Exit:** native_test, design_test, panels_test and holder_swap_identity_test pass, with every re-recorded golden listed with a reason.

## Lane

Manager: NATIVEFIX (Opus). Claims: the failing tests' code paths in `ui/src/panel_changes.rs`, `ui/src/panels.rs`, `daemon/humatad/tests/holder_swap_identity.rs`; goldens under `desktop/tests/__snapshots__/{native,design,panels}/` (re-record only).

### Log

- Release 0.3.5 finished (exit 0) before the first bazel run.
- Hang: on every hunk header in a review, `Target::anchor` scanned the whole file, so the 40,000-line PR case froze. It now scans only the hunk (`panel_changes.rs`). The case finishes in about 68 s. `review_code` still draws every line of a review file with no virtual list; that is a DIFFVIEW follow-up for main.
- Real failures fixed: Escape now leaves panel full screen through the app's `ToggleFullScreen` (`features/app/src/overlay.rs`). A click in the stack's layer list or on a code line now gives the Diff panel focus, so the layer keys (D804) and ⌥Z (DS-40) work after a click (`ui/src/panel_changes.rs`, `ui/src/document_tab.rs`).
- Tests that asserted the wrong thing: the submit-review header case now hides the Diff panel first (RV-1/DS-55); the stack layer-key case clicks Whole stack first; the widening case checks that the view starts the same share into the rewrapped top line; the PR long-path case measures the review's scrolling body, not one inline file.
- holder_swap_identity_test passed 5 of 5 in isolation. The `stty -icanon` line runs after whoami answers, so it is not the cause and stays.
- After those fixes every remaining native and panels failure was a snapshot. The view-tree diffs all come from the shipped Diff redesign (shared `#panel-header` with Full screen, the file bar with Previous/Next file, footer Push & PR… / Add repo…, Submit review… moved to the footer, new drafts wording, the Back button, the reordered shortcuts sheet), plus 15 new `diff-stress-*` frames. Re-recorded native and panels: 160 files, every PNG paired with a changed view text.
- design_test: 19 of 108 frames differ, all in the Diff panel (same redesign); 122/123/124 have no golden yet. Not recorded: they wait for Fable through main. New images are parked in the scratchpad `nativefix/design-new/`.
- Gate run: native_test, panels_test and holder_swap_identity_test pass together (native 252 s). design_test needs only the 19 golden picks. Under load native gets close to its 300 s timeout (252 s), and design_test hit it once (300 s) when run beside the other suites.
- 2026-11-19 (DIFFUI manager, GOLDENFIX) fixes for Fable's rejects:
  - The Diff panel fits every width. The file-bar and file-header names keep room and ellipsize, with the full name on hover. The repo base chip can't slide over the totals.
  - No label is said twice: a binary or mode-only file says it once, in the body. The review footer shows its drafts line only in full screen.
  - The ↗ Open in editor icon draws; ArrowUpRight was added to both icon lists.
  - Cards are capped at 720 pt (DS-15). OriginMain reads "No changes against main" (DS-32).
  - Back is a ghost in the session header and the top bar (DS-58).
  - Behaviour asserts were added in `diff_stress.rs`, `pr_session.rs` (frame 68 submit inside the panel), `diff_view.rs` (one note) and `diff_full_screen.rs` (Close in full screen).
- 2026-11-19, the owner's marks (D848):
  - Repo rows say "against X" in words.
  - Full screen reads "Exit full screen", with Close.
  - Stack rows name their repos.

# Known-gaps review log

What was done against `OVERVIEW.md › Known gaps (2026-11-15)`, in order, so a reviewer can check each change. Each entry names its commit, the gap it closes, how it was verified, and what to look at. Add an entry for every commit that works on a gap or a milestone item.

## How to review

- `git show <commit>` for each entry below; read the test it names first.
- Re-run the named Bazel targets (never cargo): `export PATH=$PWD/.tools/bin:$PATH; bazel test <target>`.
- Snapshot images live beside their tests in `__snapshots__/`; open them.

## Entries

### e4ec2d5d: the known-gaps list
- Adds the Known gaps section to OVERVIEW.md. Review that nothing reported by the agents is missing.

### a7c6cf3c: desktop panes and snapshot fixes (Desktop UI)
- `ui/src/panes.rs` draws `state.panes.tree()` as rows and columns with a header per pane (title, close button; a click focuses the pane). `ui/src/shell.rs` handles ⌘\, ⇧⌘\, ⌘W, ⌘⌥ arrows and ⌘Z for layout.
- Home's column is centred, and the Inbox badge sits off the rail icon.
- Verified: `bazel test //apps/humata2/desktop:shell_test` (new test `splitting_draws_two_panes_and_closing_one_goes_back`, re-recorded `home.png`).
- Look at: a single pane draws without chrome (no header), on purpose. Divider dragging and pane drag-and-drop are not built.

### 1818f806: markdown toolbar and Source toggle (Markdown)
- `markdown/src/view.rs`: the toggle keeps keyboard focus; the toolbar caches its selection and acts on mouse down.
- Verified: in `headless:flows`, `the_keyboard_switches_to_the_source_and_back` passes. Two flows still fail, as recorded in OVERVIEW.
- Look at: whether acting on press, not click, is acceptable UX for the selection toolbar.

### 639904a6: Copy as Markdown keeps block markup (Markdown)
- `markdown/src/view.rs` `with_block_markup`: when a selection starts at a block's first visible character, or ends at its last, the source range grows over the markup only (`# `, `> `, list markers, closing `**`).
- `testing/flow`: `Target::at_start()` aims a click or drag just inside a control's leading edge. The copy flow uses it for the heading.
- Verified: `bazel test //apps/humata2/markdown/headless:flows` (copy flow passes; 6/7), plus `markdown:markdown_test`, `headless:rendering` and `headless:snapshots`.
- Look at: the markup character set in `only_markup` is a heuristic.

### 68f47a13: left and right sides instead of the pane tree (Desktop UI)
- Owner direction 2026-11-15: no pane tree. `ui/src/panes.rs` and the split/close/focus/undo shortcuts and palette commands are removed. `ui/src/shell.rs` draws a divider after the sidebar and, when the open session has a panel, a divider and the panel on the right. Dragging sends `PaneAction::Drag`, a double-click `Reset`; ⌥⌘0 toggles the panel.
- The reverted gpui-base selection patch is recorded under Markdown in OVERVIEW.
- Verified: `bazel test //apps/humata2/desktop:shell_test` (new `a_sessions_side_panel_hides_shows_and_survives_switching_screens`; `home.view.txt` gains the sidebar's Splitter, `home.png` unchanged).
- Look at: the divider drag isn't exercised by a test; the panel's content is an empty state; `features/layout` still runs unseen.

### 62218781: delete the pane layout feature (Desktop UI)
- Owner call 2026-11-15: `features/layout` is gone, with `AppState.panes`, `AppAction::Panes`, `SavedUi.panes` and `ToastAction` (its only button was Undo reset). Toasts are text only. Repinned with `CARGO_BAZEL_REPIN=1 bazel mod tidy`.
- Verified: `bazel test //apps/humata2/desktop/... //apps/humata2/ui/...`.
- Look at: a saved `humata2-ui.json` with a `panes` key still loads (`#[serde(default)]`, unknown keys ignored).

### e90320ea: N1 native look directions
- `desktop/tests/directions.rs` draws three still mocks of the same window (not the app) for the owner to pick from: `a-light.png`, `b-dark.png`, `c-xcode.png`.
- Verified: `bazel test //apps/humata2/desktop:directions_test`.
- Look at: the mocks fake vibrancy with flat colours; the real window would use the system's sidebar material.

### 5c795563: one frame at rest, stale temps, the note told once (Daemon and persistence; D65, D71)
- `daemon/store/src/blob.rs`: blobs are `humata2_chunk` frames (`encode_framed`/`decode_framed`, zstd). The `H2BL` header has no reader. New errors `NewerFormat`, `Undecodable` and `Encode` are in `store/src/error.rs`.
- `daemon/store/src/atomic.rs` `sweep_partials`: `Store::open` removes `.partial` temps a crash left in the state folder, `blobs/` and `files/*/paste/`.
- `daemon/store/src/last_good.rs`: a restore writes the copy of the copy before moving the damaged database aside, and `finish_restore` completes a restore a crash cut short. Before this, such a crash left an empty database on the next open.
- `daemon/revive/src/agent.rs`: `Interruption.told` (schema v2) is saved before the note is typed. A redone resume skips the note when the conversation already holds it.
- Verified: `bazel test //apps/humata2/daemon/store:all //apps/humata2/daemon/revive:all` (new `a_write_killed_before_it_was_renamed_into_place_leaves_nothing_once_reopened`, `a_restore_killed_part_way_is_finished_when_the_store_opens_again`; kill and power-cut matrices gain `revive.resume.telling`/`told` and count the note).
- Look at: the weaker check when the note is skipped; the note is matched as plain text in the conversation file.

### d0be4475: design goldens replace the N1 mocks (N1; D76)
- `desktop/tests/directions*` becomes `desktop/tests/design.rs` and `design/*.rs`; target `design_test`. 33 frames in `tests/__snapshots__/design/` keep the owner's picks and drop the rest (tabs-on-top and stacked-pane switchers, the lines tree, the channel overview, Home columns, the Inbox stream).
- New: the expanded Board panel draws the kanban lanes (12); the split is two tab groups with the same tab row as a lone terminal (14); New session picks an agent TUI or a plain terminal (31); the header keeps the panel toggle.
- OVERVIEW: the goldens are a reference document and the UI target; platforms are macOS, Linux and web; the N1 picks and build items.
- Also: the persistence agent's notes (5c795563 entry and two Daemon gaps), whose code landed in 5c795563.
- Verified: `bazel test //apps/humata2/desktop:design_test` (recorded with `design_test_record`; frames viewed).
- Look at: whether Linux and web can draw the frames headless.

### c16dff55: N1 mocks for every main screen and its options
- `desktop/tests/directions.rs` and `tests/directions/*.rs` draw 37 still mocks (not the app) in looks A and B: session states and panels, three panel-switcher options, full screen and collapsed columns, two sidebar tree styles and folded channels, channel pages (two options plus Activity and Repos and machines), the Board as kanban, item drawer, table and dashboard, Home and Inbox options, Hosts, Settings and overlays. The three earlier frames are replaced.
- Verified: `bazel test //apps/humata2/desktop:directions_test` (recorded with `directions_test_record`; frames viewed).
- Look at: the Board dashboard isn't in SPEC; it needs the owner's yes and a BD entry before it's built.

### 71c9071b: the last good copy follows commits, and the restart wiring is tested (Daemon and persistence; D71)
- `daemon/store/src/store.rs`: `Store::on_commit` registers a hook called after each committed write, outside the database lock. A refused write calls nothing.
- `daemon/humatad/src/last_good.rs`: the keeper is woken by that hook instead of file events, so it works under Bazel's darwin sandbox and on any file system; the `notify` dependency is gone from humatad (repinned). A write committed while a copy is being taken now gets a copy of its own (before, it was drained and dropped).
- The rest of the wiring (`open_or_restore`, `Restored` into reconcile, `ReviveConfig.reports`, `last_start.not_brought_back`) was already in 5c795563; this adds the tests that drive it through the real daemon.
- Verified: `bazel test //apps/humata2/daemon/store:all //apps/humata2/daemon/humatad:humatad2_unit_test //apps/humata2/daemon/humatad:restart_test`. New: `each_committed_write_is_told_and_a_refused_one_is_not`, `a_copy_is_saved_once_writes_go_quiet` (un-ignored), `a_shell_that_cant_come_back_is_one_line_and_the_others_still_do`, `a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so`.
- Look at: the two restart tests pass on the code before this commit too (they cover 5c795563's wiring); the keeper tests are the ones that failed before.

### e2231201: D77 and the daemon's own log in restart reports (Daemon and persistence; D71, D77)
- D77 records the background reconcile, `ServerInfo.last_start`, the last good copy's debounce and the daemon's log; GRPC.md gains §2.2 (starting again), a §2.1 row for the debounce, and a DaemonService note. D71's "Open" no longer lists the log.
- `daemon/humatad/src/log.rs`: every stderr line (now `log::say!`) and a panic also go to `logs/humatad2.log`; each start keeps the run before's as `humatad2.previous.log` and hands it to the report. `serve.rs` binds the socket before opening the store, so a second daemon stops before touching the state and only the socket's owner rotates the log.
- `daemon/revive/src/report.rs`: `RestartReports.daemon_log`; its last 50 lines go in `daemon.log` ("(last run)") and `restart.json`'s `daemon_log`.
- Verified: `bazel test //apps/humata2/daemon/...` (new `the_restart_report_has_what_the_daemon_logged_before_it_stopped`; `a_failed_resume_is_reported_with_what_it_takes_to_diagnose_it` checks the lines).
- Look at: the log starts over past 4 MiB rather than rotating; the peer link's own `peer-link.log` isn't in the report.

### 0cc329af: saved review snapshots and one scroll frame (Review pane) [minion C]
- `review/harness/tests/large.rs` dispatches one wheel event before one draw; the old helper drew twice before the explicit third draw. The original 207-per-step failure reproduces; a bound is now checked on every frame, not just the mean.
- `review/src/model/session.rs` and `view/mod.rs` stop touching the revision when taking an empty queue. Advancing the clock alone left Saved revision 2 against current revision 3; the new snapshot regression failed before this fix. Snapshot fixtures advance 400 ms, and the text/layout/image references now show Saved. Closes the two Review pane gaps in OVERVIEW.
- Verified: `bazel test //apps/humata2/review:large //apps/humata2/review:snapshots //apps/humata2/review:flows //apps/humata2/review:model_test //apps/humata2/review:review_view_test` (all passed, in the recorded targeted runs). Recorded both snapshot suites; inspected all four image modes and the text/layout diffs.
- Look at: this fixes the measurement and save state without changing the pane's layout or list overdraw. Pixel tolerance had retained the old tiny status label, so the four PNGs were regenerated before verification.

### 1bc95455: embedded DiffView behaviour and D67 (Diff view) [minion C]
- Adds `diff/tests/view.rs` and `view_test` / `view_fast_test` in `diff/BUILD.bazel`: a Pane subscribes to the actual embedded DiffView on both engines. Tests exercise pending comparison edits, selected comment anchors, context requests/responses, replacement and empty-review counters, and Viewed folding/clearing after a content change. Ticks those OVERVIEW items; writes D67 from the existing numbers without running benches.
- Verified: `bazel test //apps/humata2/diff:view_test //apps/humata2/diff:view_fast_test` (five tests per engine passed). Ran Bazel-managed buildifier and rustfmt on the new test file.
- Look at: a separate attempted navigation test exposed the viewport-clamped counter on short reviews on both engines. The dependency owns that state; OVERVIEW records it as deferred because patching/repinning is excluded in this shared worktree. D67 remains Provisional and performance gates remain open.

### 28dc871a: the flaky idle-daemon test and the terminal_test recheck (Daemon and persistence)
- `daemon/humatad/src/serve.rs` `an_idle_daemon_with_a_connected_client_fires_no_timers`: waits for the start's restart report before counting the runtime's wakes. The reconcile runs beside serving, so its blocking steps (and, before 71c9071b, file events and the last good copy they set off) woke the runtime in the one-minute window or the ten-hour one by chance; the bound is "at most 4 more in ten hours".
- `terminal_test`'s `SizeOutOfRange { claimed: 2097152 }`: 2097152 is `SNAPSHOT_MAX`; a whole snapshot went as one chunk before snapshots were sent in pieces (5c795563). Nothing to change; the pieces test covers it.
- `restart_test`'s Unauthenticated: not reproduced; left open in OVERVIEW with what it implies.
- Verified: `bazel test //apps/humata2/daemon/humatad:humatad2_unit_test --runs_per_test=40 --local_test_jobs=16`; `restart_test` and `terminal_test` at `--runs_per_test=40 --local_test_jobs=16` and 25 more.
- Look at: whether a run of the old test can be made to fail on demand; it wasn't tried, since it needs a second checkout.

### (this commit): Markdown timings, scroll flows and formatting proposal (Markdown; Testing infrastructure) [minion C]
- `testing/flow/src/{flow,gpui_driver}.rs` adds pixel scroll actions to Flow, its artifact description and both driver modes. `testing/flow/tests/scroll.rs` proves scrolling in both directions and platform secondary shortcuts on both engines. `markdown/headless/tests/flows.rs` uses the action for its existing long-document behaviour test. The tests need the new API absent before this change. README documents the delta sign; the stale testing lockfile is removed and its manifest comment corrected.
- Inspected the Markdown goldens, recorded timing numbers in D78, ran buildifier, proposed root rustfmt.toml as D79, and allocated both numbers from the latest HEAD immediately before this commit. D67's author marker is normalized to the required title form. OVERVIEW splits completed work from the known selection, pixel-baseline, Fast-repin, Board and side-panel follow-ups.
- Verified: `bazel test //apps/humata2/testing/flow:scroll_test //apps/humata2/testing/flow:scroll_fast_test //apps/humata2/testing/flow:natural_frames_test //apps/humata2/testing/flow:natural_frames_fast_test //apps/humata2/markdown:markdown_test //apps/humata2/markdown:model_test //apps/humata2/markdown/headless:rendering` passed; `bazel test //apps/humata2/markdown/headless:flows --test_arg=the_scroll_wheel_moves_a_long_document_and_only_visible_blocks_are_drawn` passed; the opt timing target passed with numbers captured. Bazel-managed buildifier and rustfmt --check passed.
- Look at: full `headless:flows` is still 6/7 (the pre-existing cross-block selection toolbar failure). `headless:snapshots` is 4/6: both pixel baselines differ in code font rendering, while the semantic/streamed checks pass; references left intact. D78 and D79 are Provisional. Board investigation requires the excluded ui-prototype work; side-panel content remains for the next context, with Files/Info needing approved frames.

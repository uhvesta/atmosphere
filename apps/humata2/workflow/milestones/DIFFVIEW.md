# DIFFVIEW

**Outcome:** You can open any changed file in a session (several repos included) and read its whole diff, even big ones, smoothly. Long content expands to full size.

**Why (owner 2026-11-18 12:25):** "the diff view just doesnt work and/or it is janky"; "the UI looks borked. When things are long I cant expand to full. I cant open files i can only click and I see no diffs. For real bigger diffs it doesnt work."

**Stress cases (owner 12:50):** "big diffs as diffs with big files and diffs with lots of files and diffs where names are long and blah": one huge file diff, a 100k-char minified line, 500+ files across repos, 150+ char paths and long names with no spaces, long repo and branch names, binary and 10 MB files, unicode/emoji names, CRLF, no trailing newline, long renames, mode-only changes. Each must render or elide sensibly (full name reachable), scroll smoothly and stay responsive, with a test.

**Prepared PR view (owner 12:40):** likely broken by the same diff code; it's in scope.

**Proof:** screenshots of each case working, sent to the owner.

**Process (owner):** a tester drives an isolated app and daemon and finds causes, Fable 5.2 judges and writes the fix list, and Haiku 5.5 workers implement scoped fixes. Main runs bazel and git, and reruns the live check.

**Exit:** each owner symptom is reproduced, then fixed with a focused test. A live rerun on a multi-repo session with a big diff shows the diff text, expand to full, and smooth scrolling.

## Lane
- 12:24: Opus tester running (isolated /tmp/h2diff.*).
- Next: Fable judges the report into a fix list, then Haiku workers implement it.

## Fix list (Fable, 2026-11-18)

Ruling on `DIFFVIEW-report.md`: findings 2, 3, 4, 5 and 7 are accepted as written. Findings 1 and 6 are accepted in part; see "Rejected or changed" at the end. 14 tasks in 5 waves.

**Rules for every worker**
- Edit only the files your task names. Never run bazel, cargo or git; main runs the listed targets.
- Tests check behavior (bounds, presence, returned data), never message wording or call order.
- Element ids for tests need `.test_support()`, as the existing `change-…` ids do.
- New `desktop/tests/native/*.rs` and new `tests/*.rs` files in `clients`, `features/review`, `daemon/review_diff` and `daemon/humatad` are picked up by globs: no BUILD edit. A new native module also needs `mod <name>;` in `desktop/tests/native.rs`; only the task that names `native.rs` adds it.

### Waves

| Wave | Tasks (parallel inside a wave) | Owner of `ui/src/panel_changes.rs` |
|---|---|---|
| 1 | DV-1, DV-2, DV-3, DV-4 | DV-1 |
| 2 | DV-5, DV-6, DV-7, DV-8, DV-9 | DV-5 |
| 3 | DV-10, DV-11 | DV-10 |
| 4 | DV-12 | DV-12 |
| 5 | DV-13, DV-14 | none (tests and a harness only) |

Main runs the wave's targets after each wave, re-records the goldens listed, and sends changed frames to Fable for approval.

### Wave 1

**DV-1: the file list scrolls and the header buttons stay in the panel** (Haiku; findings 2, 3, 4, footer overlap)
- Files: `ui/src/panel_changes.rs` (only the final `col().size_full()` body at ~:645-670, the `list` builder at ~:403-415, and the header chip at ~:589-612), `ui/src/panels.rs` (`header`), `desktop/tests/native.rs` (add `mod diff_view;`), new `desktop/tests/native/diff_view.rs`.
- Behavior:
  1. When `inline_handle` is `None`, put `list` inside a new `div().id("changes-files")` with `.overflow_y_scroll().min_h_0().test_support()`. Move the full-mode sizing (`w(300).h_full().flex_shrink_0().border_r_1().border_color`) from `list` onto this wrapper. In narrow mode (`!full`) the wrapper gets `.flex_shrink_0().max_h(px(220.))`, so the code view (already `flex_1 min_h_0`) takes the rest. The inline review branch (`Some(handle)`) is unchanged.
  2. In `panels.rs::header`: the buttons row gets `.flex_shrink_0()`; `extra` is added as `extra.map(|e| e.min_w_0().flex_shrink())` (do not add `overflow_hidden` to it: the base menu is a deferred child and must not be clipped).
  3. In the chip: remove `.flex_shrink_0()` from the `div().relative()` wrapper and give it `.min_w_0()`; give `row().id("diff-base")` `.min_w_0()`; draw the base as `text(base.clone()).min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()`; the chevron icon goes in a `div().flex_shrink_0()`. The "against" word gets `.flex_shrink_0()`. The full base stays in the aria label.
- Tests (in `diff_view.rs`, using `App`, `fixture::design()`, `app.open(.., Panel::Changes)`, `window.find(id).bounds()` as `native/review.rs` does):
  - `a_long_file_list_leaves_room_for_the_code`: give one repo 200 files, narrow panel, select a file: `change-code <repo>/<path>` has height > 100 and its bottom is at or above the footer's top; `changes-files` height <= 220.
  - `the_file_list_scrolls_in_the_expanded_panel`: same data, `SessionAction::ToggleWiden`: `changes-files` bottom <= window height.
  - `a_long_base_keeps_expand_and_close_inside_the_panel`: set a diff's `base` to `"origin/main...HEAD + staged + unstaged + untracked"`, narrow panel: the right edges of `expand-panel` and `close-panel` are <= the panel's right edge.
- Target: `//apps/humata2/desktop:native_test`, `//apps/humata2/desktop:design_test`, `//apps/humata2/desktop:panels_test`.
- Goldens likely to change: `41-review-beside-terminal`, `changes-wrapped-long-line`, `changes-wrapped-long-line-dark`, `changes-pushed` (only if their list is taller than 220); `11-panel-full-screen-diff` should not move.

**DV-2: a changed file carries its kind, old path and binary flag** (Haiku; finding 1 binary, stress rename/binary)
- Files: `clients/src/model.rs`, `clients/live/src/diff.rs` (only the `files.push(FileDiff { … })` at ~:470), `clients/live/tests/diffs.rs`, and every literal of the client `FileDiff`: `clients/tests/diffs_per_repo.rs`, `clients/src/fixture/diffs.rs`, `clients/src/fixture/design_session.rs`, `features/review/tests/{default_branch,diffs,plan_rows,targets,repo_picker}.rs`, `features/session/tests/editor.rs`, `desktop/tests/native/editor.rs`, `desktop/tests/native/pr_session.rs`. (The `FileDiff` types in `diff/src` and `daemon/review_diff` are other types: leave them.)
- Behavior: in `model.rs` add `#[derive(Default)]` to `FileDiff` and these fields: `pub change: FileChange` (new enum `Added | Modified | Deleted | Renamed`, `#[default] Modified`), `pub old_path: Option<String>` (set only for a rename), `pub similarity: u32`, `pub binary: bool`, `pub error: Option<String>` (why this file's lines couldn't be read; `None` for now). Every existing literal gets `..FileDiff::default()`. In `diff.rs` fill them from `summary` (`pb::ChangeKind`, `old_path` when the change is a rename and it is non-empty).
- Test: in `clients/live/tests/diffs.rs` add `a_rename_and_a_binary_file_say_what_they_are`: the clone gets a `git mv` of a text file and a new file holding bytes `[0, 159, 146, 150]`; the session's diff has a file with `change == Renamed` and `old_path == Some(old)`, and a file with `binary == true` and no lines.
- Targets: `//apps/humata2/clients/live:diffs_live_test`, `//apps/humata2/clients/...`, `//apps/humata2/features/review/...`, `//apps/humata2/features/session/...`, `//apps/humata2/desktop:native_test`.
- Goldens: none.

**DV-3: a file that vanishes during a refresh no longer fails the diff** (Haiku; finding 7)
- Files: `daemon/review_diff/src/checkout.rs` (`new_tree`, the `update-index` call at ~:199), new `daemon/review_diff/tests/vanished.rs`.
- Behavior: pass `--remove` (`["update-index", "--add", "--remove", "-z", "--stdin"]`): git then skips a listed path that no longer exists. Do not use `--ignore-missing`; it only affects `--refresh`.
- Test `an_untracked_file_deleted_mid_snapshot_is_left_out`: a temp repo with one commit and two untracked files; open it with `Checkout::open_with_git(path, wrapper)` where `wrapper` is a shell script the test writes that, when its arguments contain `--others`, runs real git, then deletes one untracked file, and otherwise just execs real git. `diff_blocking` with untracked files included returns Ok, lists the surviving file and not the deleted one. Use the helpers in `tests/support/mod.rs` and copy the spec construction from `tests/specs.rs`.
- Target: `//apps/humata2/daemon/review_diff:vanished_test`, `//apps/humata2/daemon/review_diff/...`.

**DV-4: a file's hunks come from the trees the client was shown, without a new snapshot** (Haiku; finding 6 daemon)
- Files: `daemon/review_diff/src/lib.rs`, `daemon/review_diff/src/diff.rs` (visibility only), `daemon/humatad/src/review.rs` (`get_file_diff` ~:607-690 only), new `daemon/humatad/tests/review_file_trees.rs`.
- Behavior: add `pub async fn files_between(&self, trees: Trees, spec: RepoSpec) -> Result<Vec<FileSummary>, DiffError>` to `Checkout` in `lib.rs`, running the existing `Checkout::files(&trees, &spec)` on `blocking` like `diff` does. In `get_file_diff`, when `wire::trees_from_pb(request.trees.as_ref())` is `Some(trees)`, find the summary in `checkout.files_between(trees.clone(), repo_spec.clone())` and do not call `checkout.diff(...)`. When the request has no trees, keep today's path. The not-found status stays the same in both paths.
- Test `hunks_follow_the_shown_trees_after_the_file_changes_back`: copy the setup helpers from `daemon/humatad/tests/review.rs` (`daemon_with_clone`, `get_diff`, `file`, `uncommitted`). Edit a tracked file, `GetDiff`, restore the file with `git checkout -- <path>`, then `GetFileDiff` with the response's `trees`: it returns the hunks of the edit (today it answers not found). Second test: without `trees` the same call is not found.
- Targets: `//apps/humata2/daemon/humatad:review_file_trees_test`, `//apps/humata2/daemon/humatad:review_test`, `//apps/humata2/daemon/humatad:pr_base_test`, `//apps/humata2/daemon/review_diff/...`.

### Wave 2

**DV-5: code rows are virtualized** (**Opus**; finding 5. Not for Haiku: it changes scroll state, anchoring and the element tree at once.)
- Files: `ui/src/panel_changes.rs` (`code_rows`, `code_rows_focused`, `CodeRows`, `Target`, the code view at ~:466-565, `review_code`, `review_repo`, `reveal_focus`), `ui/src/document_tab.rs` (`DiffView`, `keep_top`, `diff_view`), `desktop/tests/native/diff_view.rs`, and `desktop/tests/native/review.rs` only where a test reads `DiffView.handle`.
- API: the repo's one virtual list is `uniform_list("sidebar-rows", …).track_scroll(&UniformListScrollHandle)` in `ui/src/sidebar.rs:504`. It does not fit here: `uniform_list` measures one item and assumes every item has that height, and diff rows differ (a wrapped line is `fragments × line_height`, comment cards and the open box are any height). Use gpui's `list(state, render_item)` with a `ListState::new(count, ListAlignment::Top, px(overdraw))` (`gpui-pre-0.3.12/src/elements/list.rs`; no use in the repo yet). It measures items as they come into view.
- Spec:
  1. `DiffView` holds the `ListState` and a key `(row count, wrap, columns, font size, lines identity)`. Row count or lines identity changed: `state.reset(count)`. Only wrap, columns or font changed: `state.remeasure()`. `remeasure` keeps `logical_scroll_top()` (item index plus offset in the item), which replaces `keep_top`'s child-fraction anchor; `widening_a_wrapped_diff_keeps_its_top_line_in_place` must still pass.
  2. Each frame may do O(lines) index arithmetic (a flat `Vec` of row descriptors: line i, sent card, comment card, open box), but must not build an element, wrap text or clone a `DiffLine` for rows off screen. `code_wrap::wrapped` runs inside `render_item` only. `Target.lines` stops cloning `file.lines` each frame (share an `Rc` kept in `DiffView`, keyed by lines identity). `digits` and `scrolling_width` (both O(all chars) today) are cached with the same key. `cards`/`sent` become maps keyed by line index, not a `Vec<Vec<_>>` per line.
  3. Wrap off: every code row is exactly `line_height`; the list sits in a `div().overflow_x_scroll()` whose child has `min_w(scrolling_width)` and full height, so horizontal scroll moves all rows together. Wrap on: no horizontal scroll, heights come from measurement.
  4. Ids stay (`change-line-N`, `change-gutter-N`, `change-code-N-P`, `change-hunk-N`, `review-focus-N`, `change-code <name>`): existing tests find them on visible rows.
  5. Prepared PR view (`review_code`, inline in the `review-diff` scroll): files up to 1,500 lines keep today's blocks, so frame 68 is unchanged. Above that, the file's code is the same virtual list in a box of bounded height (about 60% of the panel) with its own scroll; focus reveal uses `state.scroll_to_reveal_item(row)`. `review_repo` stops joining every file's text each frame for `bottom_up` (cache per diff identity).
- Tests (in `diff_view.rs`):
  - `a_huge_file_draws_only_the_rows_in_view`: a file of 40,000 lines, selected: `change-line-0` is found, `change-line-39999` is not; after scrolling the list to the end, `change-line-39999` is found and `change-line-0` is not.
  - `a_comment_card_far_down_shows_under_its_line_when_scrolled_to`.
  - `a_huge_file_in_a_pull_request_review_scrolls_in_its_own_box` (fixture from `native/pr_session.rs`).
  - Bench, not a pass/fail timing test: add a 40,000-line case to an existing bench or print the draw time from the test; main compares before and after.
- Targets: `//apps/humata2/desktop:native_test`, `//apps/humata2/desktop:design_test`, `//apps/humata2/ui:ui_unit_test`.
- Goldens likely to change (pixel-equal is the goal; any shift needs approval): `changes-wrapped-long-line`, `changes-wrapped-long-line-dark`, `11-panel-full-screen-diff`, `41-review-beside-terminal`. `68-pr-review-session` must not change.

**DV-6: hunk headers carry their line counts** (Haiku; finding 1 hunks)
- Files: `clients/live/src/diff.rs` (inside `session_diffs` plus one new private function and its unit tests in the same file).
- Behavior: the daemon sends `@@ -old_start +new_start @@ section` (`daemon/review_diff/src/wire.rs:579`), and a hunk can span pages, so the client writes the header once the file is whole. New pure function `fn with_counts(lines: &mut [DiffLine], starts: &[(usize, u32, u32, String)])`, or equivalent: for each hunk header line, old count = context + removed lines until the next header, new count = context + added; the text becomes `@@ -{old_start},{old} +{new_start},{new} @@ {section}` trimmed at the end. Record `old_start`, `new_start`, `section` from `pb::Hunk` when pushing the header line.
- Test (unit, same file): a hunk of 1 removed, 1 added, 1 context starting at 1/1 reads `@@ -1,2 +1,2 @@`; two hunks count separately; a hunk continued across two pages counts both pages' lines.
- Target: `//apps/humata2/clients/live:live_unit_test`, `//apps/humata2/clients/live:diffs_live_test`.
- Goldens: none (fixtures write their own headers).

**DV-7: a repo can be collapsed** (Haiku; finding 4 chevron, reducer half)
- Files: `features/review/src/lib.rs`, new `features/review/tests/collapse.rs`. If a test in `features/review/tests/` builds `ReviewState { … }` without `..ReviewState::default()`, add it there.
- Behavior: `ReviewState` gets `pub collapsed: HashSet<(SessionId, String)>`; new `ReviewAction::ToggleRepo { repo: String }` inserts or removes `(open session, repo)`. Not saved. Collapsing the repo that holds the selected file keeps the selection.
- Test: toggle twice returns to the first state; a toggle in one session leaves another session's set alone; no open session: no change.
- Target: `//apps/humata2/features/review:collapse_test`, `//apps/humata2/features/review/...`.

**DV-8: modes and "no newline at end of file" reach the wire** (Sonnet; stress mode-only and no-newline. Loosely scoped: git output parsing.)
- Files: `api/proto/humata/v1/review.proto`, `daemon/review_diff/src/{model,diff,wire}.rs`, new `daemon/review_diff/tests/modes.rs`.
- Behavior: `FileSummary` gets `uint32 old_mode = 10; uint32 new_mode = 11;` (octal value as a number, 0 for a missing side), read from the `--raw` listing `parse_listing` already parses. `DiffLine` gets `bool no_newline = 5;`, set on the line that git follows with `\ No newline at end of file` (`Collector::feed` drops that marker today at `diff.rs:394`). Every existing literal of the daemon's `FileSummary` and `DiffLine` is updated.
- Test: a `chmod +x` only change lists the file with modes 100644 and 100755 and no hunks; a file whose last line loses its newline has `no_newline` on the right line on the right side.
- Target: `//apps/humata2/daemon/review_diff/...`, `//apps/humata2/daemon/humatad:review_test`.

**DV-9: a pull request's diff can be exercised without GitHub** (**Opus**; owner's PR ask)
- Feasible: admin daemon builds run `HUMATA2_GH` as `gh` (`daemon/humatad/src/tool_path.rs:31`), and `daemon/humatad/tests/common/pr_github.rs` already has a fake `gh` plus local bare remotes with `refs/pull/<n>/head`.
- Files: new `daemon/humatad/tests/pr_diff_stress.rs`; `daemon/humatad/tests/common/pr_github.rs` only to add a helper that opens a pull request from given file contents; new `apps/humata2/testing/pr_demo/` (a `sh_binary` or small Rust binary plus BUILD).
- Behavior:
  1. Test: a pull request whose head has a 20,000-line change, 300 files, a 200-char path, a rename, a binary and a mode change; `GetDiff` on its review session lists them against the PR's base, and `GetFileDiff` with the listing's trees pages the big file to its end.
  2. `pr_demo`: `bazel run` builds an isolated home with those bare repos, the fake `gh` and one open pull request, and prints the environment (`HUMATA2_GH`, home) to start `humatad2_admin` and the app against it, so the live tester can open the prepared PR view with no network. It must never touch the real home or the real `gh` login.
- Target: `//apps/humata2/daemon/humatad:pr_diff_stress_test` (slow tier), `//apps/humata2/testing/pr_demo/...`.

### Wave 3

**DV-10: long names keep their end, each repo shows its own branch, repos collapse** (Haiku; finding 1 names, finding 4 chevron, stress long path/repo/branch)
- Files: `ui/src/panel_changes.rs` (`file_row`, `repo_head`, `plan_repo_line`, the code header row at ~:527-545, `review_repo_head`, the `rows` builder that calls `repo_head`), `desktop/tests/native/diff_view.rs`.
- Behavior:
  1. New private `fn repo_label(repo: &str) -> &str` that strips one leading `/`. Use it for the drawn repo name in `repo_head`, `plan_repo_line` and the code header. Ids, aria labels and actions keep the raw repo.
  2. New private `fn split_path(path: &str) -> (&str, &str)`: directory with its trailing `/` (empty if none), then file name. In `file_row` when `!short`, replace the single path text by a `row().flex_1().min_w_0()` of two texts: the directory with `.min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()`, and the name with `.flex_shrink_0().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis()` and id `change-file-name {repo}/{path}` (`.test_support()`). Same split in the code header, whose path block gets `min_w_0` so "Open in editor" stays inside. Give the row a tooltip with the full path if a tooltip helper for a plain element exists in `ui/src` (grep `.tooltip(`); if not, say so in the report and leave the aria label as the full name.
  3. `repo_head`: the repo name gets `.min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()`; the branch is that repo's clone's (`session.clones`, matched as `review_repo_head` does at ~:983), falling back to the session branch, with the same ellipsis and `.min_w_0()`; the totals get `.flex_shrink_0()`. `review_repo_head`'s branch gets `.text_ellipsis()`.
  4. The chevron in `repo_head` becomes a button (id `repo-toggle-{index}`, aria "Collapse {repo}" / "Expand {repo}") sending `ReviewAction::ToggleRepo`; `ChevronRight` and no file rows while `(session, repo)` is in `state.review.collapsed`.
- Tests: `two_long_paths_that_differ_in_the_file_name_both_show_it` (two 200-char paths: each `change-file-name …` has width > 0 and lies inside its row); `a_long_repo_and_branch_stay_inside_the_repo_row`; `each_repo_shows_its_own_clones_branch` (find by id you add: `repo-branch-{index}` with `.test_text`); `a_collapsed_repo_hides_its_files_and_opens_again`; `a_local_repo_reads_without_a_leading_slash`.
- Targets: `//apps/humata2/desktop:native_test`, `//apps/humata2/desktop:design_test`, `//apps/humata2/desktop:panels_test`.
- Goldens likely to change: none expected for short names; check `11-panel-full-screen-diff`, `41-review-beside-terminal`, `changes-pushed`, `68-pr-review-session`.

**DV-11: modes and the no-newline flag reach the client model** (Haiku; needs DV-2, DV-6, DV-8)
- Files: `clients/src/model.rs`, `clients/live/src/diff.rs`, `clients/live/tests/diffs.rs`.
- Behavior: `FileDiff` gets `pub old_mode: u32, pub new_mode: u32` (covered by `Default`, so no literal changes); `DiffLine` gets nothing: instead, after a wire line with `no_newline`, the client pushes one extra line of new `LineKind::Note` with text `\ No newline at end of file` and no numbers. Add the `Note` variant and fix every `match` on `LineKind` that the compiler would reject (grep `LineKind::` in `clients`, `features`, `ui`; only add arms that treat `Note` like `Hunk` for counting and commenting: not a code line, no comment target). If a needed `match` is in `ui/src/panel_changes.rs`, do not edit it: list it in the report for DV-12.
- Test in `diffs.rs`: a mode-only change has both modes and no lines; a file that loses its final newline has a `Note` line after the right line.
- Targets: `//apps/humata2/clients/live:diffs_live_test`, `//apps/humata2/clients/...`, `//apps/humata2/features/...`, `//apps/humata2/ui:ui_unit_test`.

### Wave 4

**DV-12: files with nothing to draw say why, and loading shows** (Haiku; finding 1 binary, finding 6 indicator, stress binary/rename/mode/no-newline)
- Files: `ui/src/panel_changes.rs` (`file_row`, the code view, the header chip, the row renderer from DV-5 for `LineKind::Note`), `desktop/tests/native/diff_view.rs`.
- Behavior:
  1. New private `fn file_note(file: &FileDiff) -> Option<String>`: binary: "Binary file"; rename: "Renamed from {old_path}" plus " ({similarity}% alike)" when under 100; mode change: "Mode {old:o} → {new:o}"; deleted or added with no lines: "Deleted" / "Empty file"; `file.error`: the error. Several apply: join with " · ".
  2. The code view shows the note in a row under the file header (id `change-file-note`), and when the file has no lines the note is the body instead of an empty area. In `file_row`, a binary file draws "bin" in place of `+0 −0`.
  3. A `LineKind::Note` row draws its text dimmed like a hunk header, with no gutter number and no click target.
  4. Loading: while `state.review.fetch.get(session)` has `loading == true`, the header chip names the picked base (`picked_base_name(state, &current_base)`), not the last diff's, and a dimmed "Loading…" (id `changes-loading`) shows beside it.
- Tests: `a_binary_file_says_binary_instead_of_an_empty_diff`; `a_rename_names_where_it_came_from`; `a_mode_only_change_shows_both_modes`; `picking_a_base_names_it_and_shows_loading_until_the_diff_arrives` (pick a base on the fake daemon with the reply held, as `native/review.rs` does for failed reads).
- Targets: `//apps/humata2/desktop:native_test`, `//apps/humata2/desktop:design_test`.
- Goldens: none of the existing frames should change.

### Wave 5

**DV-13: every stress case has a frame** (Haiku; the owner's proof)
- Files: new `desktop/tests/native/diff_stress.rs`, `desktop/tests/native.rs` (add `mod diff_stress;`).
- Behavior: one fixture builder and one test per owner case, each ending in `app.snapshot("diff-stress-<case>")` after a bounds assertion: `huge-file` (40,000 lines), `minified-line` (100k chars, wrap on and off), `many-files` (500 files over 3 repos, list scrolled to the last repo), `long-paths`, `long-repo-and-branch`, `binary`, `unicode-names`, `crlf`, `no-trailing-newline`, `long-rename`, `mode-only`, and the same huge file and long paths in a pull request review session (`pr-huge-file`, `pr-long-paths`; session fixture from `native/pr_session.rs`).
- Assertions are bounds only: nothing drawn outside the panel, the code view taller than 100, the header buttons inside the panel.
- Target: `//apps/humata2/desktop:native_test`. Goldens: all `diff-stress-*` frames are new; main records them and Fable approves each before they go to the owner.

**DV-14: live rerun checklist** (main session, no worker)
- Rerun the tester's 4-repo session and the `pr_demo` home. Record: first open and reselect time of big.txt, RSS, list reach of the last repo at 586 files, base switch time with the loading state, and screenshots for the owner. If 586 files still take over 5 s to load, open a follow-up for loading a file's lines only when it is selected (a change to the `Daemon::diffs` contract, Opus).

### Rejected or changed from the report

- **Repo names, `map::repo_text`:** not changed. Other callers parse `owner/name` differently (`clients/live/src/views.rs:420` reads a bare name as the owner; `file_manager.rs:256` drops owner-less names), and saved per-repo bases are keyed by the current text. The fix is display-only (DV-10).
- **"Push & PR on every repo with nothing to push":** rejected for this milestone. `RepoDiff` has no ahead-of-remote signal, and an empty diff against some bases still has commits to push; hiding the button on a guess would block a real push.
- **"The client drops hunk counts":** wrong place. The daemon formats the header without counts (`wire.rs:579`) and hunks span pages; the client computes them (DV-6).
- **"FileSummary has a mode field":** it doesn't. Modes and the no-newline marker need a proto and daemon change first (DV-8, then DV-11).
- **`--ignore-missing`:** wrong flag; `--remove` is the one that skips a vanished path (DV-3).
- **Fetching files in parallel in the client:** not now. `clients/live` has no `futures` dependency, and DV-4 removes the per-file snapshot that made each call slow. Per-file partial errors get their field (DV-2) and their note (DV-12), but `session_diffs` stays all-or-nothing until the rerun (DV-14) shows a per-file failure that matters.
- **`.test_support()` on every code element:** no separate task; virtualization bounds the count (DV-5).
- **Base menu clipped at the right edge:** not given its own task: after DV-1 the chip shrinks, which moves the menu's anchor. Recheck in DV-14; if it still clips, anchor the menu's right edge to the chip in a follow-up on `base_menu`/`repo_base_menu`.

## Lane

- 13:55: the live tester reported back (DIFFVIEW-report.md); Fable wrote the fix list (14 tasks, 5 waves).
- 14:00: wave 1 started with Haiku high: DV-1 (list scroll and header), DV-2 (FileDiff kind/old_path/binary), DV-3 (update-index --remove), DV-4 (file diff from the shown trees). Main gates after the wave.
- 14:40: wave 1 code landed. Main fixed: Provider `..Default::default()`, trailing commas after `..FileDiff::default()`, and `flex_shrink(1.)`. The daemon, clients and features gate is green (199/199). The UI gate is rerunning.
- 14:45: the owner asked how to reach full screen and for a tasteful redesign. Fable wrote DIFFVIEW-design.md (the code is the page, the list is a navigator, ⌥⌘↩ full screen, DV-15..19 new, DV-6 dropped). That proposal is now the plan: the later waves follow its order.
- 14:50: wave 2 started without the panel_changes.rs owner: DV-7 (Haiku), DV-8 (Sonnet), DV-9 (Opus). DV-5 and DV-18 start after the UI gate is green.
- 16:00 minion DIFFVIEW (Opus manager) took over DV-10..19, DV-13 and DV-14. DV-1..DV-9 landed in d12295f3.
- 16:05 verified against the code (all DV-10..19 open):
  - DV-10 not done: no `repo_label`/`split_path`, the file row draws one clipped path, `repo_head` draws the session branch and a dead chevron (the reducer's `ToggleRepo` from DV-7 is in `features/review/src/lib.rs:926`, unused by the UI).
  - DV-11 not done: proto and daemon carry `old_mode`/`new_mode`/`no_newline` (DV-8), the client `FileDiff` has no modes and `LineKind` has no `Note`.
  - DV-12 not done: no `file_note`, no status mark, no loading line; the chip names the last diff's base while loading.
  - DV-15 not done: no file bar, no next/previous file or change actions or keys.
  - DV-16 not done: the summary row, the per-repo second row and per-repo Push & PR are all still drawn; no push menu exists (there is no push-plan UI anywhere, only `Daemon::plan_push` used inside the push effect).
  - DV-17 not done: one number column, `@@` text drawn, no word highlight, no Wrap switch.
  - DV-18 not done: label "Expand panel", no tooltips, no ⌥⌘↩, no double-click, Esc does not leave full screen, full screen still draws the title bar and the panel's own 36 pt header. Main already pinned ⤢ ✕ in the review header and made the title bar's names ellipsize.
  - DV-19 not done: `nothing_against` has no Compare with…, a failed read still replaces the whole body.
- 16:05 decisions (reversible, for DECISIONS.md via main):
  - DV-16 Push & PR…: no push-plan screen exists, so the footer's quiet "Push & PR…" opens a small menu (`changes-push-menu`) listing each repo with its own Push & PR and outcome (the existing `push_control`, ids `changes-push-{repo}` kept). Menu state is `ReviewState.push_menu` with `TogglePushMenu`/`ClosePushMenu`. Always shown while the session has a diff, because no plan is known in state (DS-10: never hidden on a guess).
  - DV-16 per-repo base when all repos share the header's base: the repo row keeps the quiet chevron-only chip (the only way to give one repo its own base); words show only when the repo's base differs.
  - DV-18: `header()` draws the 52 pt title bar itself when `full`, and `panel()` stops drawing its own, so every full-screen panel has one bar with its extra (base chip / PR line) beside the title.

### Batch 1 (wave 3 + DV-18, then DV-12/DV-19 on panel_changes.rs)

| Worker | Task | Files | Status |
|---|---|---|---|
| W-A (Haiku high) | DV-11 modes and Note line | clients/src/model.rs, clients/live/src/diff.rs, clients/live/tests/diffs.rs, ui/src/request_card.rs, features/session/src/editor.rs, clients/src/fixture/diffs.rs (manager fixed the Note arm) | code done 16:15, waits on gate |
| W-B (Sonnet medium) | DV-10 + DV-16 rows, repo row, footer, push menu | ui/src/panel_changes.rs, features/review/src/lib.rs (push_menu only), new features/review/tests/push_menu.rs, desktop/tests/native/diff_view.rs, desktop/tests/native/push.rs | code done 16:20 (manager made repo-toggle a ghost button), waits on gate |
| W-C (Sonnet medium) | DV-18 full screen (one bar, name, tooltips, ⌥⌘↩, double-click, Esc, Back) | ui/src/panels.rs, ui/src/keys.rs, ui/src/shell.rs, ui/src/pages.rs, features/palette/src/lib.rs, features/app/src/{commands,overlay}.rs, features/app/tests/every_action.rs, new desktop/tests/native/diff_full_screen.rs, desktop/tests/native.rs | code done 16:25, waits on gate |
| W-D (after W-B) | DV-12 + DV-19 notes, status marks, loading, empty, Submit review in the footer | ui/src/panel_changes.rs, ui/src/pages.rs (after W-C), desktop/tests/native/diff_view.rs (pages.rs dropped: session-bar Submit review hide left for later) | code done 16:30, waits on gate |

- 16:32 Batch 1 code done, handed to main to gate: DV-10, DV-11, DV-12, DV-16, DV-18, DV-19 (DS-35 per-repo error left: no per-repo error in state; session-bar Submit review not yet hidden while Diff shows). panel_changes.rs still held for DV-15/DV-17/DV-13.
- 16:50 Gate 1 diagnostics. Fixed: ui/src/pages.rs terminal-full-screen tooltip removed (a div's tooltip takes a view builder; aria label stays). Claimed and fixed outside files: desktop/tests/panels.rs and native/shell.rs (leave full screen with show-terminal), live_flows/review.rs (summary assert dropped; open the Push & PR menu before each repo push), design/session.rs, design/diff_targets.rs, design/comments.rs (summary row and second repo line removed; base chip on the repo row). editor_comments.rs needs nothing: the editor's own footer draws its draft count.
- 17:05 Gate 2. Side panel full screen moves from ⌥⌘↩ to ⌃⌘↩ (ctrl-cmd-enter): off a Mac every ⌥⌘X and ⇧⌘X chord becomes Ctrl+Alt+Shift+X, so ⌥⌘↩ clashed with ⇧⌘↩ (terminal full screen). ⌃⌘↩ becomes Ctrl+Shift+Enter, which nothing else uses, and ⌃⌘ is the Mac's own full-screen chord. Changed keys.rs, panels.rs tooltip, palette hint, diff_full_screen.rs. Spec DS rule for ⌥⌘↩ reads ⌃⌘↩ from here on. The panels snapshot losing the clone's branch "humata/s-auth" is intended (DS-5: a repo row names its branch only when it differs from the session's).
- 17:15 Wave: W-E (Sonnet high) DV-15 file bar, file/change stepping (⌥⌘↓/↑ files, ⌃⌘↓/↑ changes: ⇧⌥⌘ would clash with ⌥⌘ off a Mac), list fold; plus the repo-row overflow fix and its bounds test (Fable 17:12). W-F (Haiku high) DV-13 diff_stress.rs. W-G (Sonnet high) design fixtures (design/session.rs, diff_targets.rs, comments.rs, desk.rs claimed) redrawn to the new Diff look after Fable rejected 16 design frames: the frames are hand-built copies, so they only showed what the fixtures dropped.
- 17:45 Wave code done, waits on gate. DV-15: file bar (`diff-file-bar`, N of M, ‹ ›, ↗), list folds once a file is picked, NextFile/PrevFile (⌥⌘↓/↑), NextChange/PrevChange (⌃⌘↓/↑) with the Change N of M stepper on files with >1 hunk and >60 lines; repo row totals pinned (`repo-totals-{i}`). Not done: the file bar in the inline PR review (DS-53), list keys (DS-84; no focus handle). DV-13: diff_stress.rs, 15 frames. Design fixtures redrawn (W-G); frame 98 footer keeps only Add repo…. targets.rs and diff_stress.rs open the folded list (manager).
- 18:00 Gate 3: fixed diff_stress.rs E0597 (try_find with an owned id); live_flows/review.rs `open_base_menu` waits for the read to finish before clicking "Against …", since the chip names the picked base while loading (DS-34). Frame 98 now changes too: its footer gained Add repo…, as the app draws. W-H (Sonnet high) started DV-17.
- 18:40 DV-17 code done (W-H): two number columns in full screen, a 2-cell sign column with add/remove tints over the code background, a hunk band (section text, or a 1 px rule) with no @@ numbers, word highlight on paired lines, code background end to end, Wrap switch (`diff-wrap`, ⌥Z in the diff). Not done: the 720 pt comment-card cap (ui/src/comment_cards.rs is not this lane's). ui_unit_test needed `use ::core::prelude::v1::test;` in panel_changes.rs tests (gpui's glob import shadows `test`). DV-10, 11, 12, 13, 15, 16, 17, 18, 19 code done; panel_changes.rs released to STACKVIEW.
- 19:32 Round 3. DS-35: a whole read that fails splits into one read per repository (`Daemon::repo_diff`), and only a split outcome shows per-repo errors (`Fetch.repo_errors`, rows `repo-error-{repo}` with Try again). DS-84: list cursor in the review feature (`ListDown/Up/Open/Collapse/Expand`), `DIFF_LIST` keys, focus handle in Documents. DS-53: the file bar shows in the inline PR review. The loading test is now event-driven (`FakeDaemon::hold/release`). SESSION_LINE: callers moved to `panel_header`; the thread-local goes with seam_header. STACKVIEW layer names in the base label and empty state. panel_changes.rs edits are seams in scratchpad/diffview (order: header, ds35, ds53, layer, ds84). features/review/... and clients/... pass.

Then DV-15 (Opus-level, after the batch-1 gate), then DV-17, then DV-13; DV-14 is main's.

## Spec and cases

- [DIFFVIEW-spec.md](DIFFVIEW-spec.md) writes the diff and review UI as rules DS-1 to DS-84, each marked Built, Partly built, Not built or In progress from the code on 2026-11-18. It also holds the key map, the budgets P-1 to P-9, and how source contradictions were settled.
- [flows/cases/diff/](../../flows/cases/diff/README.md) has 16 cases that check every DS rule step by step.
- [REVIEWUI.md](REVIEWUI.md) holds what this milestone doesn't, and lists the DS rules each DV task must meet. Its "What DIFFVIEW already owns" table is the acceptance list for DV-5 and DV-10 to DV-19.

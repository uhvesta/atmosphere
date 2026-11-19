# A file row reads its name first, and marks added, deleted, renamed, binary and mode-only files

Layer: Diff and review. Platforms: macOS desktop.

## Starting state

- Machine `box-1`, channel `#payments`, session "Refunds" (a local session, not a pull request). Its repos are `acme/api` and a local repo `scratch` with no owner.
- The changes in "Refunds", as the Diff panel should show them:
  - `acme/api` › `src/client.rs`, modified (+3 −1).
  - `acme/api` › `src/http.rs`, renamed to `src/transport/http.rs` (92% alike, +1 −1).
  - `acme/api` › `src/new_cache.rs`, new file (+40).
  - `acme/api` › `src/old_util.rs`, deleted (−22).
  - `acme/api` › `assets/logo.png`, binary (12 MB).
  - `acme/api` › `scripts/deploy.sh`, mode changed 644 → 755, no lines changed.
  - `acme/api` › a path of about 200 characters, ending in `…/retry_policy_for_the_refund_worker.rs`.
  - `scratch` › `notes.md`, modified (+2 −0).
- The Diff panel is open beside the terminal. No file is open.

## Reproduce headless

- Fakes, for the row and header: `bazel test //apps/humata2/desktop:native_test --test_filter=the_panels_draw_their_client_data` (`desktop/tests/panels.rs`, which draws the file rows). The rows' text and tooltips are not asserted by any test.
- Real daemon, for the data: `bazel test //apps/humata2/daemon/review_diff:modes_test` (`daemon/review_diff/tests/modes.rs`: `a_mode_only_change_lists_both_modes_and_no_hunks`, `an_added_file_has_no_old_mode`, `losing_the_final_newline_marks_the_right_line_on_the_right_side`).
- Real daemon, big PR: `bazel test //apps/humata2/daemon/humatad:pr_diff_stress_test` (`a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`). It checks that the 200-character path, the rename, the binary and the mode change come back from `humatad2` with the right change kinds. It does not check the UI.
- Not headless: how long names look in a real window at the panel's width, and how the tooltip looks on hover. No windows on the owner's screen.

## Steps

1. **I do:** look at the row for `acme/api` › `src/client.rs`.
   **I see:** a row with a file-code icon, then "src/client.rs" in one line, then "+3" in green and "−1" in red. The path is shown whole, and the name is not set apart from the folder (DS-7 wants the name first, and the folder dimmed).
   **State and events:** `file_row` (`ui/src/panel_changes.rs:67-160`). The icon is the same `IconName::FileCode` for every row (`:127`). The text is the whole `path` (`:131-140`), with `flex_1`, `min_w_0`, `overflow_hidden` and `whitespace_nowrap`.
   **Status:** Fakes only. Built (DV-10): the name comes first and the folder follows dimmed and ellipsized (`file_list.rs::file_row`, `split_path`). `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it` checks the name's bounds; the colour and order have no assertion.

2. **I do:** hover the row, or wait for a tooltip.
   **I see:** no tooltip. The accessibility label holds the full path ("acme/api/src/client.rs"), but nothing shows on screen.
   **State and events:** the label is `format!("{repo}/{path}")` (`panel_changes.rs:111`). `file_row` has no `.tooltip(...)`.
   **Status:** Fakes only. Built (DS-7): `file_list.rs::file_row` now uses `full_text`, so the row has the full `repo/path` as its hover and label. `diff_view.rs::a_rename_names_where_it_came_from` checks the file note's label, not the row's hover.

3. **I do:** look for the status of each file: added, deleted, renamed, binary.
   **I see:** every row has the same file icon and no letter. A new file is not marked "A". A deleted file is not marked "D". The renamed file is not marked "R". There is no "bin" for the binary file. The mode change has no mark.
   **State and events:** DS-8 wants a 14 pt status column: A in green, D in red, R in secondary, and nothing for a modified file. DS-82 wants letters, not colour alone.
   **Status:** Fakes only. Built (DV-12): the 14 pt mark column draws A (green), D (red) and R, "bin" and "mode" in place of +/−. `diff_view.rs::a_rename_names_where_it_came_from` asserts the R mark exists; A, D, "bin" and "mode" have no assertion.

4. **I do:** look at the new file, `src/new_cache.rs`.
   **I see:** "+40" in green, and no "New file" note under the header when I open it.
   **State and events:** DS-24 says a note line reads "New file" when there is something to say.
   **Status:** Code, no test. `file_list.rs::file_note` says "New file" only for an added file with no code lines (and "Empty file" when it adds nothing). An added file that has lines shows the A mark and its code but no note.

5. **I do:** look at the renamed file row, `src/transport/http.rs`.
   **I see:** the new path, "+1" and "−1". No "R" mark, and no "Renamed from src/http.rs (92% alike)" note.
   **State and events:** `FileDiff.old_path` and `similarity` (`clients/src/model.rs:1126-1140`) hold the old path and 92. The row does not draw them.
   **Status:** Fakes only. Built (DV-12): `diff_view.rs::a_rename_names_where_it_came_from` finds the note (`change-file-note`) and the mark; the words "Renamed from … (N% alike)" are not asserted.

6. **I do:** open `assets/logo.png`.
   **I see:** the row shows "+0" and "−0", and no "bin" where the counts would be (DS-8). The file's code area is empty, with no note saying it is binary (DS-24: "Binary file · 12 MB").
   **State and events:** `FileDiff.binary` is true and the file has no lines. The file's size is not in the model.
   **Status:** Fakes only. Built (DV-12): `diff_view.rs::a_binary_file_says_binary_instead_of_an_empty_diff` (the body is the note, and no code lines are drawn). The size ("12 MB") is not drawn: `file_note` says "Binary file" only.

7. **I do:** click `scripts/deploy.sh`, the mode-only change.
   **I see:** the row shows "+0" and "−0". The file has no lines, and no note says "Mode 644 → 755". It looks like an empty change.
   **State and events:** the daemon knows both modes (`daemon/review_diff`, `a_mode_only_change_lists_both_modes_and_no_hunks`), and the proto carries `old_mode` and `new_mode` (`api/proto/humata/v1/review.proto:277-278`). The client model `FileDiff` has no mode field, and `clients/live/src/diff.rs` does not read the mode. So the UI cannot show it.
   **Status:** Fakes only. Built (DV-12): `diff_view.rs::a_mode_only_change_shows_both_modes` (the note is the body). The daemon side is Works (real daemon), per `daemon/review_diff/tests/modes.rs`.

8. **I do:** click the long-path row.
   **I see:** the row shows the start of the path, and its end is cut off where the row runs out of width. The file name at the end is the part that gets lost. There is no ellipsis.
   **State and events:** the path text is `overflow_hidden` and `whitespace_nowrap`, so the clip falls at the end (`panel_changes.rs:131-140`). DS-47 wants the folder to ellipsize before the name does.
   **Status:** Fakes only. Built (DV-10): `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it`, `diff_stress.rs::a_200_byte_path_stays_inside_the_panel`. The daemon returns the 200-character path (`pr_diff_stress.rs`).

9. **I do:** click the row for `scratch` › `notes.md`, where the repo is local and has no owner.
   **I see:** the repo's name in the repo header and in the file row reads "/scratch", with a leading slash. DS-48 wants "scratch".
   **State and events:** `clients/live/src/diff.rs:129` maps the repo with `map::repo_text`, which always writes `{owner}/{name}` (`clients/live/src/map.rs:64`). With no owner, that is "/scratch". `clients/live/src/comments.rs:68-74` handles the no-owner case (`repo_name`), so the code already knows the right answer.
   **Status:** Fakes only. Likely bug cleared: `diff_view.rs::a_local_repo_reads_without_a_leading_slash`. The wire text is still "/scratch" (`clients/live_core/src/map.rs::repo_text` writes `owner/name`); only the drawing strips it (`file_list.rs::repo_label`), as DIFFVIEW's "Rejected" list decided.

10. **I do:** look at the file header after I open `src/client.rs`.
    **I see:** one line: "acme/api/src/client.rs" in a single text, then a dimmed "Open in editor" at half opacity. There is no note line under it, and the name is not set apart from the folder.
    **State and events:** the header is `format!("{}/{}", diff.repo, file.path)` (`ui/src/panel_changes.rs:962`), drawn at `:583-601`. The editor text is inert (see case 01).
    **Status:** Code, no test. The header reads `repo · folder/name` with the name in primary text and a note line under it (`panel_changes/mod.rs::changes`). The ↗ is an icon but does nothing (step 11). No test reads the header's text.

11. **I do:** hover "Open in editor" (the ↗ control in the header).
    **I see:** no change and no tooltip. It does nothing when clicked.
    **State and events:** `panel_changes.rs:594-600`, id `change-open-editor`, with no `on_click`.
    **Status:** Not built. `file_list.rs::open_in_editor` is an icon with the tooltip "Open in editor" and no `on_click`. No editor client exists; no open task owns it.

12. **I do:** open a PR review session and look at an inline file row.
    **I see:** only the file's name, without its folder. The folder is dropped in the inline review rows.
    **State and events:** `file_row` takes `short`, and `short` shows `rsplit('/').next()` only (`panel_changes.rs:127-137`). The spec's DS-7 wants the folder dimmed after the name, which this does not do.
    **Status:** Code, no test. A review's inline rows are a separate layout (frame 68, D861: the file glyph, the name, then the letter). `pr_session.rs::pr_review_session_frame_68` holds it as a snapshot; no assertion on the name-only form.

## Branches

### A file's lines cannot be read

When the daemon cannot read one file's lines, such as a file that is too large or unreadable.
I see the file's row, and the file's code area should say why. The spec says a file with no lines shows its note as its body (DS-24).
State and events: `FileDiff.error` is the per-file reason (`clients/src/model.rs:1140`). The live client sets it to `None` (`clients/live/src/diff.rs:482`), and the UI never reads `FileDiff.error` (only the fetch error, `panel_changes.rs:352-368`).
**Status:** Not built (DS-36): `file_note` draws `FileDiff.error` as the file's body, but `clients/live_diff/src/diff.rs` always writes `error: None`, and a failing file read fails the whole repo read. No open task.

### A file is renamed and also changed

When a renamed file's contents also changed (similarity below 100).
I see the new path, and its counts are the diff against the old file. The note "Renamed from … (N% alike)" should appear (DS-24).
**Status:** Fakes only. `diff_view.rs::a_rename_names_where_it_came_from` (similarity 92, one added line). Words not asserted.

### The PR is big, with 300 files

When the review has 300 files and one big file.
I see the rows, and the big file's code comes in pages.
**Status:** Works (real daemon) for the data: `pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`. The rows at 500 files: `diff_stress.rs::many_files_scroll_to_the_last_repositorys_last_file` (Fakes only).

### The row's name at a narrow panel

When I narrow the panel so the folder and the counts fight for the width.
I see the path clipped at the end, with the counts still visible (the counts are not shrunk).
**Status:** Fakes only. `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it` drags the panel to 220 pt and checks the name stays inside its row.

## Expected outcomes

- [ ] A file row shows its name first, then its folder dimmed and ellipsized, then its comments and its counts (DS-7).
- [ ] The row's tooltip and accessibility label carry the full path (DS-7).
- [ ] A status letter (A, D, R) shows for added, deleted and renamed files. A modified file shows nothing (DS-8).
- [ ] A binary file shows "bin" in place of its counts, and a mode-only change shows "mode" (DS-8).
- [ ] The status is a letter, not colour alone (DS-82).
- [ ] A long path keeps its name: the folder ellipsizes before the name does (DS-47).
- [ ] A local repo reads "scratch", not "/scratch" (DS-48).
- [ ] The file header reads `repo · folder/name` with the name in primary text, and a note line says what is special (DS-24).
- [ ] A per-file error from the daemon is shown where the file's lines would be.

## Status table

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Name first | Fakes only | `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it` |
| 2 Tooltip | Fakes only | `file_list.rs::file_row` (`full_text`); `diff_view.rs::a_rename_names_where_it_came_from` (note label only) |
| 3 Status letters | Fakes only | `diff_view.rs::a_rename_names_where_it_came_from` (R only); `file_list.rs::file_row` |
| 4 New file note | Code, no test | `file_list.rs::file_note` (only for a file with no lines) |
| 5 Renamed | Fakes only | `diff_view.rs::a_rename_names_where_it_came_from` |
| 6 Binary | Fakes only | `diff_view.rs::a_binary_file_says_binary_instead_of_an_empty_diff` |
| 7 Mode-only | Fakes only | `diff_view.rs::a_mode_only_change_shows_both_modes`; daemon: `daemon/review_diff/tests/modes.rs` |
| 8 Long path | Fakes only | `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it`; `diff_stress.rs::a_200_byte_path_stays_inside_the_panel` |
| 9 Local repo "/scratch" | Fakes only | `diff_view.rs::a_local_repo_reads_without_a_leading_slash` (Likely bug cleared) |
| 10 File header | Code, no test | `panel_changes/mod.rs::changes` (file header) |
| 11 Open in editor | Not built | `file_list.rs::open_in_editor` has no action; no open task |
| 12 Inline review row name only | Code, no test | `review_header.rs::review_repo`; frame 68 |
| Branch: lines cannot be read | Not built | `clients/live_diff/src/diff.rs` writes `error: None`; no open task |
| Branch: renamed and changed | Fakes only | `diff_view.rs::a_rename_names_where_it_came_from` |
| Branch: 300-file PR | Works (real daemon) for the data | `pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end` |
| Branch: narrow panel | Fakes only | `diff_view.rs::two_long_paths_that_differ_in_the_file_name_both_show_it` |

Counts (16 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 1 (the 300-file branch, data only); Fakes only 10 (steps 1, 2, 3, 5-9; the renamed-and-changed and narrow-panel branches); Code, no test 3 (steps 4, 10, 12); Not built 2 (step 11, and the unreadable-lines branch); Likely bug 0.

## Gaps and likely bugs

- Fakes only: a hover with the full path on a file row (`file_list.rs::file_row`, `full_text`). No test asserts the hover; `diff_view.rs::a_rename_names_where_it_came_from` checks only the note's label.
- Not built: a per-file read error. `file_note` and the body draw `FileDiff.error`, but `clients/live_diff/src/diff.rs` never sets it, so one unreadable file fails its whole repo (DS-36). No open task.
- Not built: the ↗ Open in editor does nothing (DS-24). No open task; it needs an editor client.
- Code, no test: "New file" shows as a note only when the added file has no lines (`file_note`); the A mark carries it otherwise. If the owner wants the note on every new file, that is a change to `file_note`.
- Gap in tests: A, D, "bin" and "mode" marks and the dimmed folder have no assertion; the note words are not asserted (by AGENTS.md).
- Cleared: "/scratch" reads "scratch". The wire text keeps its leading slash on purpose (other callers parse it).
- The step narratives above predate DV-10 and DV-12; only the Status lines and the table were re-audited on 2026-11-19.

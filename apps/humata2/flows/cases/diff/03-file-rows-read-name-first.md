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
   **Status:** Not built (DV-10). The folder is not dimmed and the name is not split out.

2. **I do:** hover the row, or wait for a tooltip.
   **I see:** no tooltip. The accessibility label holds the full path ("acme/api/src/client.rs"), but nothing shows on screen.
   **State and events:** the label is `format!("{repo}/{path}")` (`panel_changes.rs:111`). `file_row` has no `.tooltip(...)`.
   **Status:** Not built (DV-10). DS-7 wants the tooltip and the label to carry the full path. The label does, so the tooltip is the gap.

3. **I do:** look for the status of each file: added, deleted, renamed, binary.
   **I see:** every row has the same file icon and no letter. A new file is not marked "A". A deleted file is not marked "D". The renamed file is not marked "R". There is no "bin" for the binary file. The mode change has no mark.
   **State and events:** DS-8 wants a 14 pt status column: A in green, D in red, R in secondary, and nothing for a modified file. DS-82 wants letters, not colour alone.
   **Status:** Not built (DV-12). The data is there: `FileDiff.change` (`clients/src/model.rs:1108-1123`, `FileChange::Added`, `Modified`, `Deleted`, `Renamed`), and `FileDiff.binary` (`:1126-1140`). The daemon produces these kinds (`pr_diff_stress.rs` checks the mode change's kind).

4. **I do:** look at the new file, `src/new_cache.rs`.
   **I see:** "+40" in green, and no "New file" note under the header when I open it.
   **State and events:** DS-24 says a note line reads "New file" when there is something to say.
   **Status:** Not built (DV-12). Only the counts are drawn.

5. **I do:** look at the renamed file row, `src/transport/http.rs`.
   **I see:** the new path, "+1" and "−1". No "R" mark, and no "Renamed from src/http.rs (92% alike)" note.
   **State and events:** `FileDiff.old_path` and `similarity` (`clients/src/model.rs:1126-1140`) hold the old path and 92. The row does not draw them.
   **Status:** Not built (DV-12, DS-24). The daemon lists a renamed file once, under its new path (`pr_diff_stress.rs`, the check at lines 169-171).

6. **I do:** open `assets/logo.png`.
   **I see:** the row shows "+0" and "−0", and no "bin" where the counts would be (DS-8). The file's code area is empty, with no note saying it is binary (DS-24: "Binary file · 12 MB").
   **State and events:** `FileDiff.binary` is true and the file has no lines. The file's size is not in the model.
   **Status:** Not built (DV-12). The daemon reports the binary (`pr_diff_stress.rs`, the `binary()` file at line 59).

7. **I do:** click `scripts/deploy.sh`, the mode-only change.
   **I see:** the row shows "+0" and "−0". The file has no lines, and no note says "Mode 644 → 755". It looks like an empty change.
   **State and events:** the daemon knows both modes (`daemon/review_diff`, `a_mode_only_change_lists_both_modes_and_no_hunks`), and the proto carries `old_mode` and `new_mode` (`api/proto/humata/v1/review.proto:277-278`). The client model `FileDiff` has no mode field, and `clients/live/src/diff.rs` does not read the mode. So the UI cannot show it.
   **Status:** Not built (DV-12). The daemon side is Works (real daemon), per `modes.rs`.

8. **I do:** click the long-path row.
   **I see:** the row shows the start of the path, and its end is cut off where the row runs out of width. The file name at the end is the part that gets lost. There is no ellipsis.
   **State and events:** the path text is `overflow_hidden` and `whitespace_nowrap`, so the clip falls at the end (`panel_changes.rs:131-140`). DS-47 wants the folder to ellipsize before the name does.
   **Status:** Not built (DV-10, DS-47). The daemon returns the 200-character path (`pr_diff_stress.rs` `long_path`).

9. **I do:** click the row for `scratch` › `notes.md`, where the repo is local and has no owner.
   **I see:** the repo's name in the repo header and in the file row reads "/scratch", with a leading slash. DS-48 wants "scratch".
   **State and events:** `clients/live/src/diff.rs:129` maps the repo with `map::repo_text`, which always writes `{owner}/{name}` (`clients/live/src/map.rs:64`). With no owner, that is "/scratch". `clients/live/src/comments.rs:68-74` handles the no-owner case (`repo_name`), so the code already knows the right answer.
   **Status:** Likely bug. To confirm: open a session with a local repo and read the repo header. It should read "scratch", not "/scratch".

10. **I do:** look at the file header after I open `src/client.rs`.
    **I see:** one line: "acme/api/src/client.rs" in a single text, then a dimmed "Open in editor" at half opacity. There is no note line under it, and the name is not set apart from the folder.
    **State and events:** the header is `format!("{}/{}", diff.repo, file.path)` (`ui/src/panel_changes.rs:962`), drawn at `:583-601`. The editor text is inert (see case 01).
    **Status:** Not built (DV-12, DS-24). The header is the old form (`repo/path`), not `repo · folder/name`. The note line does not exist.

11. **I do:** hover "Open in editor" (the ↗ control in the header).
    **I see:** no change and no tooltip. It does nothing when clicked.
    **State and events:** `panel_changes.rs:594-600`, id `change-open-editor`, with no `on_click`.
    **Status:** Code, no test. Inert by design until an editor client exists.

12. **I do:** open a PR review session and look at an inline file row.
    **I see:** only the file's name, without its folder. The folder is dropped in the inline review rows.
    **State and events:** `file_row` takes `short`, and `short` shows `rsplit('/').next()` only (`panel_changes.rs:127-137`). The spec's DS-7 wants the folder dimmed after the name, which this does not do.
    **Status:** Code, no test. The review inline rows are a separate layout (frame 68). No test asserts the name-only form.

## Branches

### A file's lines cannot be read

When the daemon cannot read one file's lines, such as a file that is too large or unreadable.
I see the file's row, and the file's code area should say why. The spec says a file with no lines shows its note as its body (DS-24).
State and events: `FileDiff.error` is the per-file reason (`clients/src/model.rs:1140`). The live client sets it to `None` (`clients/live/src/diff.rs:482`), and the UI never reads `FileDiff.error` (only the fetch error, `panel_changes.rs:352-368`).
**Status:** Not built (DV-12, DS-36): the field is reserved for DV-12 to fill. `clients/live/src/diff.rs:482` drops the per-file error, and no UI code draws it. To confirm: have the daemon return a file with `error` set (a fake daemon that returns `error` set can do this) and see that the row and its code show nothing.

### A file is renamed and also changed

When a renamed file's contents also changed (similarity below 100).
I see the new path, and its counts are the diff against the old file. The note "Renamed from … (N% alike)" should appear (DS-24).
**Status:** Not built (DV-12). The counts are right in the daemon (`pr_diff_stress.rs`), but the note is not drawn.

### The PR is big, with 300 files

When the review has 300 files and one big file.
I see the rows, and the big file's code comes in pages.
**Status:** Works (real daemon) for the data: `pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`. The rows' own layout at that size is Code, no test.

### The row's name at a narrow panel

When I narrow the panel so the folder and the counts fight for the width.
I see the path clipped at the end, with the counts still visible (the counts are not shrunk).
**Status:** Not built (DV-10, DS-47).

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
| 1 Name first | Not built | `panel_changes.rs:131-140` (DV-10) |
| 2 Tooltip | Not built | `panel_changes.rs:111` has the label, no tooltip (DV-10) |
| 3 Status letters | Not built | DV-12; `clients/src/model.rs:1108-1140` has the data |
| 4 New file note | Not built | DV-12 |
| 5 Renamed | Not built | DV-12, DS-24; `pr_diff_stress.rs` lines 169-171 (daemon) |
| 6 Binary | Not built | DV-12; `pr_diff_stress.rs` `binary()` (daemon) |
| 7 Mode-only | Not built | DV-12; `daemon/review_diff/tests/modes.rs::a_mode_only_change_lists_both_modes_and_no_hunks` (daemon, Works) |
| 8 Long path | Not built | DV-10, DS-47; `pr_diff_stress.rs::long_path` (daemon) |
| 9 Local repo "/scratch" | Likely bug | `clients/live/src/diff.rs:129`; `clients/live/src/map.rs:64` |
| 10 File header | Not built | DV-12, DS-24; `panel_changes.rs:962` |
| 11 Open in editor | Code, no test | `panel_changes.rs:594-600` |
| 12 Inline review row name only | Code, no test | `panel_changes.rs:127-137` |
| Branch: lines cannot be read | Not built | `clients/live/src/diff.rs:482` |
| Branch: renamed and changed | Not built | DV-12 |
| Branch: 300-file PR | Works (real daemon) for the data | `pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end` |
| Branch: narrow panel | Not built | DV-10, DS-47 |

Counts (16 rows, by each row's first tag): Works (real shell) 0; Works (real daemon) 1 (the 300-file branch, data only); Fakes only 0; Code, no test 2 (steps 11, 12); Not built 12 (steps 1-8, 10, and the branches for a renamed-and-changed file, a narrow panel and unreadable lines); Likely bug 1 (step 9).

## Gaps and likely bugs

- Likely bug: `clients/live/src/diff.rs:129` writes "/scratch" for a local repo, because `map::repo_text` (`clients/live/src/map.rs:64`) always writes `owner/name`. `comments.rs:68-74` already handles the no-owner case; the diff path does not. Confirm with a session on a local repo and read the repo header.
- Not built (DV-12): `FileDiff.error` is never set from the daemon (`clients/live/src/diff.rs:482` sets `None`) and never drawn in `ui/src`. A file whose lines fail to read shows nothing.
- Not built: the status letter column (A, D, R, bin, mode), the name-first row, the tooltip, and the file header's note line (DS-7, DS-8, DS-24, DS-82; DV-10, DV-12).
- Not built: the mode-only change's modes do not reach the client model (no `old_mode` or `new_mode` in `FileDiff`, DV-12).
- Gap in tests: no test reads a row's text, tooltip or status mark. The layout is checked only by the frames.

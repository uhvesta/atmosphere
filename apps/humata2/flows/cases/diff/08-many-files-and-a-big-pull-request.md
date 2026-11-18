# A pull request with hundreds of files and one huge file lists every file and opens the huge one in its own box.

Layer: Diff and review. Platforms: macOS desktop.

**DV-5 in progress.** Another agent is changing `ui/src/panel_changes.rs` and `ui/src/document_tab.rs` now. The "today" below is the code as it stood on 2026-11-18. Recheck every line number after DV-5 lands.

## Starting state

- `acme/demo` pull request #1 is open against `main`. It changes 300 files, shaped like the stress test:
  - `big.txt`: 20,000 lines in and 20,000 out, in one file.
  - A 200-character path, `a_folder_with_a_rather_long_name/…`.
  - A rename, `notes/old_name.md` to `docs/new_name.md`.
  - A binary, `assets/logo.bin`.
  - A mode change, `scripts/run.sh` from 644 to 755.
  - 295 small files, each with one added line.
- A session "Refunds" in `#payments` is linked to that pull request as a review. Its Diff panel is open beside the terminal.
- The panel is at its normal width, then one step at a time I widen it to full screen.

## Reproduce headless

- Daemon, real: `bazel test //apps/humata2/daemon/humatad:pr_diff_stress_test --test_filter=a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end` (`daemon/humatad/tests/pr_diff_stress.rs`). It runs a real `humatad2` with a fake `gh`. It asserts 300 files, the big file's counts, the 200-character path, the rename, the binary, the mode change and the 295 small files, and that the big file's pages reach its last line.
- Offline demo: `bazel run //apps/humata2/testing/pr_demo -- <new empty folder>`. It builds that home with a bare remote, a fake `gh` and the same 300-file pull request, then prints how to start the daemon and the app against it. Its own check is `bazel test //apps/humata2/testing/pr_demo:pr_demo_test`, which checks the home is built. No test opens the app on it.
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=files_are_listed_bottom_up` (`desktop/tests/native/pr_session.rs`).
- Native, on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=the_file_list_scrolls_in_the_expanded_panel` (`desktop/tests/native/diff_view.rs`). This uses 200 files, not 300.
- Not headless: a real GitHub pull request, and the real time to load 300 or 586 files. The report's numbers came from a local run. No windows on the owner's screen.
- Target look: mockup `workflow/milestones/mockups/review.png`, section 1 (PR review session), and `diff-proposal.png`, section 3 (big file).

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Open the review's Diff. | The PR line reads "PR #1 against main · 1 repo". The repo's group row and its files are listed, bottom-up. (DS-52, DS-59) | `GetDiff` returns all 300 files. The daemon test asserts `repo.files.len() == 300`. The bottom-up order is `bottom_up` (`ui/src/panel_changes.rs:926`). | Works (real daemon) for the listing. The bottom-up order is Fakes only: `desktop/tests/native/pr_session.rs::files_are_listed_bottom_up`. |
| 2 | Wait for the list to fill. | The list fills in one piece, with no progress line. (DS-25, DIFFVIEW-report finding 6) | The client reads each file in turn, one page at a time, all or nothing (`clients/live/src/diff.rs:430-463`). The report measured 177 files at about 30 s. No loading state exists. | Not built (DV-12). |
| 3 | Scroll the file list to its last file. | The last file of the last repo is reachable. (DS-30) | The list scrolls in `review-diff` (`ui/src/panel_changes.rs:716-721`, the list at `:742`). The built test scrolls a 200-file list in the non-review Diff (`diff_view.rs::the_file_list_scrolls_in_the_expanded_panel`). No test scrolls the review's list to its end. | Code, no test: `ui/src/panel_changes.rs:716-721`. |
| 4 | Scroll to big.txt in the review. | big.txt is drawn in its own box, about 60% of the panel's height, with its own scroll. Shorter files keep the inline look. (DS-29) | `review_code` builds every line of the picked file: `for index in 0..file.lines.len()` (`ui/src/panel_changes.rs:1011`). No virtual list on this path. The `VIRTUAL_FROM` branch is only in the non-review view (`:522`). DV-5 step 5 is in progress; recheck. | Not built (DV-5 step 5). |
| 5 | Scroll to a short file in the review. | Its lines are drawn inline in the review, with the agent's focus marked. (frame 68) | `review_code` draws each file's lines in blocks (`ui/src/panel_changes.rs:1011-1020`). | Fakes only: `desktop/tests/native/pr_session.rs::the_line_the_agent_focuses_is_marked`. |
| 6 | Let the agent focus a line deep in big.txt. | The line is marked, and the box scrolls it into view. (DS-59, DS-29) | The focus is marked and scrolled by `reveal_focus` (`ui/src/panel_changes.rs:857-889`). For a short file, `pr_session.rs::a_focus_below_the_fold_is_scrolled_into_view` covers it. For a big file the focus has no box to scroll in. | Not built (DV-5 step 5) for a big file. |
| 7 | Read big.txt's rows in the box. | All 20,000 removed and 20,000 added lines in order, the last one "line 20000, changed" at the end. | The daemon pages the file. Each page carries lines and a `next` cursor (`daemon/humatad/tests/pr_diff_stress.rs`). The test asserts each old line comes once, in order, each new line once, in order, and the last added line. The client joins the pages (`clients/live/src/diff.rs:430-463`). | Works (real daemon): `daemon/humatad/tests/pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`. |
| 8 | Look at the rename, `docs/new_name.md`. | One row, under its new path, with the note "Renamed from notes/old_name.md (92% alike)". It is listed once. (DS-24) | The daemon lists it once, as Renamed, with `old_path` set (the daemon test). The client keeps `old_path` and `similarity` (`clients/live/src/diff.rs:471-481`, DV-2), but the UI draws no note (coach-checked). | Not built (DV-12). The daemon and client halves are built; the daemon half is Works (real daemon). |
| 9 | Look at `assets/logo.bin`. | "bin" in place of the counts, and "Binary file · 12 MB" as the note. (DS-24) | The daemon sets `binary` (the daemon test asserts it). The client keeps the flag (`clients/live/src/diff.rs:427`, `:481`, DV-2), but the row still shows "+0 −0" and no note (coach-checked). | Not built (DV-12). |
| 10 | Look at `scripts/run.sh`. | The note "Mode 644 → 755" under its header, and no lines. (DS-24) | The daemon lists it as Modified with 0 lines (the daemon test asserts that). The client has no mode field, so no note is drawn. | Not built (DV-8, DV-12). |
| 11 | Look at the 200-character path in the list. | The name keeps its file name, and the folder cuts first. (DS-47) | The path is cut with no ellipsis (`ui/src/panel_changes.rs:67-140`, see case 05 step 13). The daemon test asserts its length is 200. | Not built (DV-10). |
| 12 | Look at a small file, such as `pub fn f12`. | "+1" beside it, and its one line. | The daemon lists each of the 295 small files, each with one added line (the daemon test asserts that). | Works (real daemon): `daemon/humatad/tests/pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`. |
| 13 | Type in the "Filter files" field above the list. | The field appears above the list once a session has more than 20 changed files. (DS-9) | "Filter files" exists in the Files panel (`ui/src/panel_files.rs:280`) and in `ui/src/shell.rs:94`. No field sits above the Diff's list. | Not built (no DV task; REVIEWUI). |
| 14 | Open the pull request offline with `pr_demo`. | The same 300 files, with no network. (DS-31) | The demo builds the home with a fake `gh` and a bare remote. `pr_demo_test` checks the home, not the app's view. | Code, no test: `testing/pr_demo/pr_demo_test.sh`. |

## Branches

### The review has 586 files over four repos

When the session reviews four repos with 586 files in all. The list should reach every repo, and the first file should show within the budget.
**I see:** every repo's group row, and the list scrolls to the last.
**Expected:** P-5 (proposed in the spec, sourced from DV-14): under 5 s to load 586 files over four repos. Otherwise a follow-up loads a file's lines only when it is picked.
**Status:** Code, no test: `clients/live/src/diff.rs:430-463`. The report measured 177 files at about 30 s, and nothing has been re-measured since (DV-14 is still a checklist). Each file costs a daemon call that recomputes the bases (branch below).

### One file in the pull request fails to read

When one file's page fails. Today the whole review's diff fails, not only that file.
**I see:** an error banner for the whole diff, with "Try again".
**Expected (spec):** per-file errors, with the rest of the diff still shown (DV-2's `error` field, DV-12's note).
**Status:** Not built (DV-2, DV-12). `clients/live/src/diff.rs:430-463` uses `?` on each page, so one error ends the whole diff.

### A file vanishes during a refresh

When a file is deleted while the diff reloads. The report saw the whole diff fail with "update-index … does not exist". The checkout now passes `--remove` to `update-index` (`daemon/review_diff/src/checkout.rs:~199`, with the comment "`--remove` skips a listed path that vanished").
**Expected:** the diff reloads, without the vanished file.
**Status:** Works (real daemon): `daemon/review_diff/tests/vanished.rs` (DV-3).

### The base changes on the pull request

When I switch the base while the 300 files reload. The chip should name the new base at once.
**I see:** the chip shows the old base until the new files all load. The report saw 30–60 s and more.
**Status:** Likely bug: `ui/src/panel_changes.rs:611-615` (see the gaps).

### Each file is read again

When I open each of the 300 files. Each read calls the daemon, which re-runs the default and pull request bases. It reuses the trees the list came from, so it takes no new snapshot (`daemon/humatad/src/review.rs:652-661`, DV-4).
**I see:** the time grows with the file count, times the repo size.
**Expected:** the bases are worked out once per diff, and the reads reuse them.
**Status:** Code, no test: a cost to measure in DV-14 and REVIEWUI RV-8, not a wrong result (coach-checked).

## Expected outcomes

- [ ] The review lists all 300 files, and the list reaches its last file.
- [ ] big.txt is drawn in its own box with its own scroll, and the box is about 60% of the panel.
- [ ] A short file keeps its inline look in the review.
- [ ] An agent's focus deep in big.txt is marked and scrolled into view.
- [ ] big.txt's pages come to its last line, each line once, in order.
- [ ] The rename, the binary and the mode change each show their note.
- [ ] A long path keeps its file name visible.
- [ ] A "Filter files" field appears above the list over 20 files.
- [ ] Loading 586 files across four repos takes under 5 s, or a follow-up opens to load a file's lines when it is picked.

## Status table

Counts per tag. A step or branch counts once, under its tag.

| Tag | Steps (of 14) | Branches (of 5) | Total |
|---|---|---|---|
| Works (real shell) | 0 | 0 | 0 |
| Works (real daemon) | 3 (steps 1, 7, 12) | 1 (a file vanishes) | 4 |
| Fakes only | 1 (step 5) | 0 | 1 |
| Code, no test | 2 (steps 3, 14) | 2 (586 files; each file read again) | 4 |
| Not built | 8 (steps 2, 4, 6, 8, 9, 10, 11, 13) | 1 (one file fails) | 9 |
| Likely bug | 0 | 1 (base changes) | 1 |

## Gaps and likely bugs

Not built:

- `ui/src/panel_changes.rs:1011` (`review_code`): the review builds every line of the picked file, with no virtual list, and no box of bounded height for a big file (DS-29, DV-5 step 5).
- `ui/src/panel_changes.rs:716-742`: the review list has no progress line while files load (DS-28, DV-12).
- `clients/src/model.rs` (`FileDiff`): the client has no mode field (DV-11). It keeps binary, old path and similarity (`clients/live/src/diff.rs:471-481`), but the UI draws no note for any of them (DS-24, DV-12).
- `ui/src/panel_changes.rs:67-140`: long paths cut without an ellipsis (DS-47, DV-10).
- `ui/src/panel_changes.rs` and `ui/src/panel_files.rs:280`: no "Filter files" field above the Diff's list (DS-9; REVIEWUI, no DV task).
- Per-file partial errors: `clients/live/src/diff.rs:430-463` fails the whole diff (DV-2, DV-12).

Likely bugs:

- Performance note, not a bug (coach-checked): `get_file_diff` (`daemon/humatad/src/review.rs:607-690`) re-runs `default_bases` and `pull_request_bases` on every call. It reuses the shown trees (`:652-661`), so there is no new snapshot. Time it in DV-14.
- `ui/src/panel_changes.rs:611-615`: the base chip names `diffs.first()`'s base, not the picked base, so the chip shows the old base until the whole load ends (DIFFVIEW-report finding 6, DV-12). Confirm with a native test that switches the base on a fixture of 300 files and checks the chip's label before the fake replies.

Tests not yet written:

- A review-list scroll to its last file (step 3).
- A timing test for 586 files across four repos (P-5), which DV-14 asks for.
- A big-file box test: `a_huge_file_in_a_pull_request_review_scrolls_in_its_own_box` (DV-5 spec 5, not written).

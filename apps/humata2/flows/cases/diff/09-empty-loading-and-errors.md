# When there is nothing, something loading, or something broken, the diff says so where it matters and offers the fix

Layer: Diff and review. Platforms: macOS desktop.

Covers DS-32, DS-33, DS-34, DS-35, DS-36, DS-37, DS-38, DS-39 and P-7 in
`workflow/milestones/DIFFVIEW-spec.md`. Mockup: `diff-proposal` §4 (frames 98
and the new `empty` and `loading` frames, DV-13).

## Starting state

- Machine `box-1` runs the session "Refunds" in channel #payments. It compares two repos against `main`: `acme/api` and `acme/ledger`. (The four-repo session in the report is the slow case in step 5.)
- "Refunds" has no changes against `main` in `acme/api`. It has changes in `acme/ledger`.
- The app is open on "Refunds". The Diff panel is closed.
- The daemon `humatad2` is running and answering.
- This Mac has the base menu and the chip in view. No other session is open.

## Reproduce headless

- Fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=a_failed_` runs `desktop/tests/native/review.rs::a_failed_first_read_names_the_error_with_try_again` and `::a_failed_refresh_keeps_the_last_diff_with_try_again`. `bazel test //apps/humata2/desktop:native_test --test_filter=the_base_menu_picks` runs the base menu and Mark reviewed test, which also asserts the empty-state id (`diff-none-against-base`, review.rs:348).
- Real daemon: `bazel test //apps/humata2/daemon/review_diff:vanished_test` (a file deleted mid-snapshot, DS-38) and `bazel test //apps/humata2/daemon/humatad:review_file_trees_test` (a file that changes back, DS-39).
- Reducer: `bazel test //apps/humata2/features/review:default_branch_test` covers a failed find of the default branch, which the failed-read block uses.
- Not headless: the slow base switch (step 5) needs a daemon that holds the diff reply for 30 s or more. A fake that delays `Diffs` would show the chip result headless, but none exists yet. The Compare with… button and the "Reading changes…" line have no frame yet (DV-13).
- No window is opened on the owner's screen for any of these.

## Steps

| # | I do | I see | State and events | Status |
|---|---|---|---|---|
| 1 | Open the Diff panel on "Refunds" with `acme/api` picked (nothing against `main`). | "No changes yet", centred, in secondary text. No icon. | The text comes from `nothing_against` (`ui/src/panel_changes.rs:1103-1120`), id `diff-no-changes-yet`. No button is drawn. (DS-32) | Code, no test. `panel_changes.rs:1103-1120`; no test asserts the origin-main text. |
| 2 | Open the base menu and pick a base with nothing in it (origin/brin). | "No changes against that branch", with id `diff-none-against-base`. | The empty state follows the picked base. | Fakes only. `desktop/tests/native/review.rs::the_base_menu_picks_what_the_diff_is_against_and_mark_reviewed_starts_it_over` (asserts the id at review.rs:348). |
| 3 | Look for **Compare with…** under the empty text. | Nothing. There is no button. | No action opens the base menu from the empty state. (DS-32) | Not built. DV-19. `panel_changes.rs:1103-1120` draws no button. |
| 4 | Open the Diff panel on a session whose first read is still running (the daemon is slow). | The body is empty. No "Reading changes…" line and no spinner. | `features/review/src/lib.rs:500-504` sets `fetch.loading`, but no UI code reads it. (DS-33) | Not built. DV-12, DV-19. |
| 5 | While the diff is loading, pick a new base in the chip menu (origin/brin instead of main). | The chip still says the old base, `main`, until the new diff arrives. The new diff takes 30 to 60 s on a four-repo session (report finding 6). | `panel_changes.rs:608-614` names `diffs.first().base`, so the chip reads the old diff's base once one has arrived. The picked base is used only before the first diff. The report puts the label on the fetched diffs (`features/review/src/lib.rs:699-710` and `:489-506`). P-7 says the chip names the new base in the same frame. (DS-34) | Likely bug. `panel_changes.rs:608-614`. To confirm: hold the `Diffs` reply in the fake daemon, pick a base, and read the chip label before the reply. |
| 6 | Keep looking at the old files while the new base loads. | The old files are still there. They are not dimmed. | DS-34 says the previous files stay, dimmed. The panel has no dimming for them. The only opacity in the diff body is on the disabled "Open in editor" (`panel_changes.rs:598-601`). | Not built. DV-12. |
| 7 | In a four-repo session, one repo (acme/ledger) fails to read. | The whole body is replaced by the failure. The other repos do not show. | `RepoDiff` has no error field (`clients/src/model.rs:1345`). The client reads each file and page in turn with `?` (`clients/live/src/diff.rs:430-463`), so one failure fails the whole read. (DS-35) | Not built. DV-19. The spec says a failed read replaces the whole body today. |
| 8 | Open a file in acme/ledger whose lines cannot be read. | Its body should say why, and nothing else changes. | `FileDiff.error` (`clients/src/model.rs:1140`) is never set. Nothing in `clients/live/src/diff.rs` or `daemon/humatad/src/review.rs` writes it. (DS-36) | Not built. DV-12. |
| 9 | A new file is added in `acme/api` and deleted again before the read ends (an untracked file gone mid-snapshot). | The diff still loads. The vanished file is left out. | `daemon/review_diff/src/checkout.rs:203` passes `--remove` to `update-index`, so a path that vanished after the listing is skipped, not an error. (DS-38) | Works (real daemon). `daemon/review_diff/tests/vanished.rs::an_untracked_file_deleted_mid_snapshot_is_left_out`. |
| 10 | A file in the diff changes back to its old content while I have it open. | The lines I was shown still show the change I was shown. | The file's trees come from the list the client was shown (`daemon/humatad/src/review.rs:658`). (DS-39) | Works (real daemon). `daemon/humatad/tests/review_file_trees.rs::hunks_follow_the_shown_trees_after_the_file_changes_back`. |
| 11 | Open the Diff panel while the daemon is down, so the first read fails. | "Couldn't read the changes.", the error text, and **Try again**. The panel does not say "No changes yet". | `read_failed_block` (`panel_changes.rs:1122-1197`) stands in for the body. **Try again** sends `ReviewAction::Retry` (`panel_changes.rs:1152`). (DS-37) | Fakes only. `desktop/tests/native/review.rs::a_failed_first_read_names_the_error_with_try_again`. |
| 12 | With a diff open, pick a base while the daemon fails. | The last diff stays. Above it, a line says the refresh failed, with **Try again**. | `refresh_failed_row` (`panel_changes.rs:1201`) draws the line. `fetch.error` is set (`features/review/src/lib.rs:858-874`). (DS-37) | Fakes only. `desktop/tests/native/review.rs::a_failed_refresh_keeps_the_last_diff_with_try_again`. |
| 13 | Press **Try again** after the daemon is back. | The failure line goes. The diff reads again and shows. | `diff-retry` sends `ReviewAction::Retry`. The failure is cleared on the next good read (`features/review/src/lib.rs:858`). | Fakes only. Same two tests as steps 11 and 12; both assert the retry clears the failure. |
| 14 | Open a session whose `acme/ledger` clone has no default branch. | "Couldn't read the changes.", "acme/ledger has no default branch to compare with.", **Find the default branch**, and a chip that says "pick a branch…". | `read_failed_block` uses `fetch.no_default` (`panel_changes.rs:1122-1197`). **Find the default branch** sends `FindDefaultBranch`. (DS-37) | Fakes only. `features/review/tests/default_branch.rs::a_failed_find_keeps_its_reason_and_the_repo_until_a_retry_works` covers the find and its retry. The block's text is not asserted headless. |

## Branches

### The daemon restarts while the first read is running

When the daemon restarts during step 4 and the read fails. I press **Try again** after it is back.
I see the same failed-read block as step 11. Try again reads the changes and the diff shows.
Expected: no "No changes yet" in between, and the read is not lost.
**Status:** Fakes only. `desktop/tests/native/review.rs::a_failed_first_read_names_the_error_with_try_again`. The real restart is not tested for the diff.

### The clone has no default branch

When I open a session whose clone has no default branch (step 14). I press **Find the default branch**, and it fails.
I see the reason under the repo name, and **Try again** stays. A second try works.
Expected: the reason is kept until the retry works.
**Status:** Fakes only. `features/review/tests/default_branch.rs::a_failed_find_keeps_its_reason_and_the_repo_until_a_retry_works`.

### The base switch fails

When I pick a base and the daemon fails the new read (step 12). The old diff is still there.
I see the old diff and the refresh failure line. The chip label is not asserted by the test, so what it shows here is open. Step 5 already shows the chip reading the old base while a read runs.
Expected: the chip names the base the panel shows, and the failure line says what failed.
**Status:** Fakes only. `desktop/tests/native/review.rs::a_failed_refresh_keeps_the_last_diff_with_try_again` covers the failure line and the kept diff. It does not assert the chip label.

### A file vanishes during a refresh

When a file is deleted between the list and the read, during a refresh. Before DV-3 this failed the whole diff with "does not exist and --remove not passed".
I see the diff load, without the vanished file.
Expected: the vanished file is left out, and the diff does not fail.
**Status:** Works (real daemon). `daemon/review_diff/tests/vanished.rs::an_untracked_file_deleted_mid_snapshot_is_left_out`. The fix is `checkout.rs:203`.

## Expected outcomes

- [ ] A diff with no changes says "No changes yet" or the base's own empty text, with no button that does anything (DS-32).
- [ ] A base with no changes says "No changes against that branch" and keeps the picker usable.
- [ ] The chip names the base I picked at once, not the old base (DS-34, P-7).
- [ ] While a read runs, the body says "Reading changes…" and has no spinner (DS-33).
- [ ] When one repo fails, only its row shows the error and **Try again** (DS-35).
- [ ] When one file fails, its own body says why (DS-36).
- [ ] A first-read failure names the error with **Try again**, and never says "No changes yet" (DS-37).
- [ ] A refresh failure keeps the last diff under a line with **Try again** (DS-37).
- [ ] A file deleted while the diff is read is left out, and the diff does not fail (DS-38).
- [ ] A file that changes back still shows the change I was shown (DS-39).

## Status table

Counts per tag: Works (real shell) 0; Works (real daemon) 3; Fakes only 8;
Code, no test 1; Not built 5; Likely bug 1. Total 18 rows: 14 steps and 4
branches.

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Empty text | Code, no test | `panel_changes.rs:1103-1120` |
| 2 Base with no changes | Fakes only | `review.rs::the_base_menu_picks_what_the_diff_is_against_and_mark_reviewed_starts_it_over` |
| 3 Compare with… button | Not built | DV-19; `panel_changes.rs:1103-1120` |
| 4 Reading changes… | Not built | DV-12, DV-19; `features/review/src/lib.rs:500-504` |
| 5 Base switch, chip | Likely bug | `panel_changes.rs:608-614`; report finding 6 |
| 6 Old files dimmed | Not built | DV-12; DS-34 |
| 7 One repo fails | Not built | DV-19; `clients/src/model.rs:1345` |
| 8 One file fails | Not built | DV-12; `clients/src/model.rs:1140` |
| 9 File vanishes | Works (real daemon) | `review_diff/tests/vanished.rs::an_untracked_file_deleted_mid_snapshot_is_left_out` |
| 10 File changes back | Works (real daemon) | `review_file_trees.rs::hunks_follow_the_shown_trees_after_the_file_changes_back` |
| 11 First read fails | Fakes only | `review.rs::a_failed_first_read_names_the_error_with_try_again` |
| 12 Refresh fails | Fakes only | `review.rs::a_failed_refresh_keeps_the_last_diff_with_try_again` |
| 13 Try again | Fakes only | the two tests above |
| 14 No default branch | Fakes only | `default_branch.rs::a_failed_find_keeps_its_reason_and_the_repo_until_a_retry_works` |
| Daemon restart on first read | Fakes only | `review.rs::a_failed_first_read_names_the_error_with_try_again` |
| No default branch, find fails | Fakes only | `default_branch.rs::a_failed_find_keeps_its_reason_…` |
| Base switch fails | Fakes only | `review.rs::a_failed_refresh_keeps_the_last_diff_with_try_again` |
| File vanishes on refresh | Works (real daemon) | `vanished.rs::an_untracked_file_deleted_mid_snapshot_is_left_out` |

## Gaps and likely bugs

- **Not built, DV-19:** the **Compare with…** button under "No changes yet" (DS-32). The spec wants it; `nothing_against` draws only text.
- **Not built, DV-12:** the "Reading changes…" line on first load (DS-33). `fetch.loading` is set in the reducer (`features/review/src/lib.rs:500-504`) and read by no UI code.
- **Not built, DV-12 and DV-19:** a failed repo row with its own **Try again** (DS-35). `RepoDiff` has no error field.
- **Not built, DV-12:** a file's own failure reason (DS-36). `FileDiff.error` (`clients/src/model.rs:1140`) has no producer.
- **Not built, DV-12:** the old files dimmed while a new base loads (DS-34).
- **Likely bug, report finding 6 (high):** the base chip shows the old base until the new diff arrives. `panel_changes.rs:608-614` takes the base from `diffs.first()`. P-7 asks for the new base in the same frame, and proposes under 5 s for the new diff on the four-repo session. The report measured 30 to 60 s or more. To confirm: hold the `Diffs` reply in a fake, pick a base, and read the chip before it answers.
- **Wording, low:** DS-32 says "No changes against main"; the code says "No changes yet" for the origin-main base (`panel_changes.rs:1103-1120`). The tests assert ids, not wording (see the spec's §8 item 8). Decide which text is wanted.
- **Still not proven live:** the base switch time (P-7) and the four-repo failure (DS-35). Both need a real daemon with a slow or failing repo; neither is in a test today.
- The DV-5 virtual rows are in progress on `panel_changes.rs`. The empty and error blocks above are not changed by it, but recheck the line numbers after DV-5 lands.

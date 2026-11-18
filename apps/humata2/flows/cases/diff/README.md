# Diff and review cases

These cases check the diff and review UI: the Diff panel beside the terminal, full screen, big files, long names, the pull request review, and stacks over several repos. Each case cites the rules it checks by ID (DS-n) from [DIFFVIEW-spec.md](../../../workflow/milestones/DIFFVIEW-spec.md). The open work is in [REVIEWUI.md](../../../workflow/milestones/REVIEWUI.md), [DIFFVIEW.md](../../../workflow/milestones/DIFFVIEW.md) and [STACKVIEW.md](../../../workflow/milestones/STACKVIEW.md).

Each case follows [`briefs/_GUIDE.md`](../briefs/_GUIDE.md) and adds `## Reproduce headless`. Status tags were set from the code and tests on 2026-11-18, not from the spec. The coach checked every Likely bug against the code; four writer claims were wrong and were corrected (see the end).

**DV-5 is in progress.** Another lane is making code rows virtual in `ui/src/panel_changes.rs` and `ui/src/document_tab.rs`. Rows that touch big-file scrolling say "DV-5 in progress; recheck". Line numbers in those two files will move.

## Index

| # | Case | Covers |
|---|---|---|
| 01 | [A changed file opens beside the terminal and the code gets the room](01-open-a-file-beside-the-terminal.md) | DS-1–4, DS-30 |
| 02 | [Each repo is one row I can fold, and the footer only offers what I can do](02-repo-rows-and-the-footer.md) | DS-5, 6, 10, 11, 74, 77, 78; Add repo… |
| 03 | [A file row reads its name first, and marks added, deleted, renamed, binary and mode-only files](03-file-rows-read-name-first.md) | DS-7, 8, 24, 36, 47, 48, 82 |
| 04 | [Full screen opens three ways and Show the terminal puts me back exactly where I was, with the terminal still running](04-full-screen-and-back.md) | DS-12–18, 69–71, 83 |
| 05 | [However long the names get, Full screen and Close stay on screen and the names shorten instead](05-full-screen-never-disappears.md) | DS-45, 46, 49, 50, 79, 80 |
| 06 | [Changed code reads cleanly: numbers, a sign, the code, quiet tints and no @@ noise](06-code-reads-like-the-page.md) | DS-19–23, 81 |
| 07 | [A 40,000-line file opens at once and scrolls smoothly, with its header pinned and a way to step between changes](07-a-big-file-stays-smooth.md) | DS-25–28; P-1–4, P-9 |
| 08 | [A pull request with hundreds of files and one huge file lists every file and opens the huge one in its own box](08-many-files-and-a-big-pull-request.md) | DS-9, 29–31, 38; P-5 |
| 09 | [When there is nothing, something loading, or something broken, the diff says so where it matters and offers the fix](09-empty-loading-and-errors.md) | DS-32–39; P-7 |
| 10 | [I switch long lines between Wrap and Scroll from the file I'm reading, and Scroll keeps the numbers still](10-wrap-or-scroll.md) | DS-40–44, 51, 75; P-6 |
| 11 | [A pull request review shows what is reviewed, steps across both PRs, and keeps the agent's drafts clearly mine to send](11-pr-review-session.md) | DS-52–55, 59 |
| 12 | [I submit one review for every PR at once, see new commits as they land, and go back with one click](12-submit-review-new-commits-and-back.md) | DS-56–58 |
| 13 | [Each session keeps its own open file, scroll and full screen, and opening the panel doesn't read everything again](13-the-diff-remembers-each-session.md) | DS-70, 72, 73, 76; P-8 |
| 14 | [I can drive the diff from the keyboard: full screen, next file, next change, wrap, back](14-keys-for-the-diff.md) | Spec §3; DS-12, 13, 27, 40, 58, 84 |
| 15 | [A stack of changes over two repos shows as layers I can step through, each with only its own files](15-a-stack-across-repos.md) | DS-60–64, 67 |
| 16 | [When a repo moves after a stack was sent, that layer says so and offers one fix; a stack with no notes shows no review controls](16-a-stack-that-moved.md) | DS-65, 66, 68 |

Every DS rule, DS-1 to DS-84, is cited by at least one case.

## Counts

| Cases | Rows tagged | Works (shell) | Works (daemon) | Fakes only | Code, no test | Not built | Likely bug |
|---|---|---|---|---|---|---|---|
| 16 | 279 | 0 | 23 | 73 | 60 | 111 | 12 |

A row with mixed tags counts under its first tag. Stack rows lead with the screen ("Not built (screen). Engine: Works (real daemon)"), so the engine tests don't count the unbuilt UI as working.

## The rig

- **Headless UI:** `//apps/humata2/desktop:native_test`, files `desktop/tests/native/{diff_view,diff_big,review,pr_session,pr_review,push,document}.rs`. These run the real UI on fake clients and write `.new.png` and `.view.txt` when a frame differs. No window opens.
- **Real daemon:** `//apps/humata2/daemon/humatad:pr_diff_stress_test`, `:review_file_trees_test`; `//apps/humata2/daemon/review_diff:{vanished,modes,stack,stack_chat}_test`; `//apps/humata2/desktop:live_flows_test` with `desktop/tests/live_flows/pr_review.rs` (five tests on a real `humatad2` with a fake `gh`).
- **A big PR by hand:** `testing/pr_demo/pr_demo.sh` opens acme/demo#1 (300 files) in an isolated home with a fake `gh`, offline.
- **The 4-repo stress rig** (large, many, medium, small) from [DIFFVIEW-report.md](../../../workflow/milestones/DIFFVIEW-report.md) is what DV-14 and REVIEWUI RV-8 rerun for the budgets.

## What the cases found

### Likely bugs

1. **Full screen draws the Diff header twice**: two "Diff" titles and two ways back. `ui/src/panel_changes.rs:622`, `:695` call `header()` under the 52 pt title bar; snapshot `11-panel-full-screen-diff.view.txt` shows it. DV-18. Case 04.
2. **The repo chevron looks like a fold but does nothing**, in the Diff panel and in a review. `panel_changes.rs:1446`, `:1066`; nothing in `ui/src` sends `ReviewAction::ToggleRepo`. DV-10. Case 02.
3. **A local repo reads "/scratch"**. `clients/live/src/diff.rs:129` uses `map::repo_text` (`clients/live/src/map.rs:64`), which always writes `owner/name`. DV-10. Case 03.
4. **A long repo name can push the totals out of the panel**: the repo name has no `min_w_0` or ellipsis. `panel_changes.rs:1448` (Diff), `:1072` (review). DV-10. Case 05.
5. **The base chip names the old base while a new one loads**, for 30–60 s on the stress rig. `panel_changes.rs:611-615` reads `diffs.first()`'s base. DV-12. Cases 07, 08, 09.
6. **A force-push that also adds commits shows only the commit count**. `ui/src/pages.rs:536-540`: the `(n, _)` arm ignores `rewritten`. Case 12.
7. **Two sessions on the same file share one scroll position**. `ui/src/document_tab.rs:553` is keyed by `repo/path` (`panel_changes.rs:475`). REVIEWUI RV-3. Case 13.
8. **Suspected, perf only:** each virtual row looks up its file by name (`panel_changes.rs:2023`, `:544-550`). DV-5 in progress; confirm with the bench. Case 07.

### Not built, by owner

- **DIFFVIEW:** the file bar and stepping (DV-15); one row per repo, name-first rows, ellipsis on paths, repos and branches (DV-10); status letters, notes, loading lines and per-file errors (DV-11, DV-12); the hunk look and the Wrap switch (DV-17); ⌥⌘↩, Esc, double-click, tooltips, one header and the full-screen footer (DV-18); Compare with… and per-repo errors (DV-19); the PR review's own box for big files (DV-5 step 5).
- **REVIEWUI:** Submit review in the footer and the agent-draft outline (RV-1); a pinned file head and Scroll in a review (RV-2); per-session scroll and no refetch on panel toggles (RV-3); tooltips (RV-4); ⌥Z, the ⌘/ sheet (⇧⌘↩ is bound at `ui/src/keys.rs:223` but not listed) (RV-5); Filter files (RV-6); the base menu anchor (RV-7).
- **STACKVIEW:** the whole stack UI (T4, T6, T7, T8; `ui/src/panel_stack.rs` does not exist). The engine works: `daemon/review_diff/tests/stack.rs`, `stack_chat.rs`. `drift_blocking` (`daemon/review_diff/src/stack.rs:582`) has no caller yet.

### Proof gaps (tests, not features)

- No test scrolls the narrow list to its last row, or uses 586 files.
- No test reads a file row's text, tooltip or status mark.
- No full-screen round trip checks the scroll, line, Wrap, open comment box or drafts (`pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal` checks only the file).
- No test switches sessions while one is full screen.
- After a Missing report, `drift_blocking` resets `below` (`stack.rs:611-616`), so the next layer in that repo skips its chain check. Test it when T4 lands. Case 16.

### Coach corrections to writer claims

- Case 01 claimed "No changes yet" shows under a listed diff. It can't: `selected` (`ui/src/panels.rs:201-226`) falls back to the first file. Spec DS-4 was updated to match.
- Case 08 claimed the client drops `old_path`, `similarity` and `binary`, and that each file read takes a new snapshot. The client keeps them (`clients/live/src/diff.rs:471-481`), and `get_file_diff` reuses the shown trees (`daemon/humatad/src/review.rs:652-661`). Only the UI half is missing, and the per-call base work is a cost to time, not a bug.
- Case 10 called a review that always wraps, and the two Wrap sources, Likely bugs. The first is deliberate today and is now Not built (RV-2). Nothing sends `ReviewAction::ToggleWrap`, so the second is a risk for DV-17, not a bug.
- Cases 03 and 11 tagged a reserved field (`FileDiff.error`) and a settled spec change (Submit review placement) as Likely bugs. Both are now Not built.

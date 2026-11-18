# DIFFVIEW spec: how the diff and review UI looks and behaves

Written 2026-11-18 from the sources below. This is the one place the diff and
review UI is written down as rules. Each rule has a stable ID (DS-n) that
cases (`flows/cases/diff/`) and milestones (DIFFVIEW, REVIEWUI, STACKVIEW)
cite.

**Sources, highest first:** the owner's words (2026-11-18, quoted in
[REVIEWUI.md](REVIEWUI.md)); [DIFFVIEW-design.md](DIFFVIEW-design.md)
(Fable's approved design and the owner additions at its end);
[DIFFVIEW.md](DIFFVIEW.md) (the DV fix list); [DIFFVIEW-report.md](DIFFVIEW-report.md)
(the live test); [STACKVIEW.md](STACKVIEW.md) (stacks UI); [PR.md](PR.md);
and the mockups in [mockups/](mockups/): `diff-proposal` (§1 beside the
terminal, §2 full screen, §3 big file, §4 states), `review` (§1 PR review
session, §2 Submit review, §3 Wrap and Scroll, §4 Full screen never
disappears, §5 Show the terminal) and `stacks` (§1 Whole stack, §2 one
layer, §3 narrow with drift and the Inbox offer).

**Status of each rule** was set by reading the code on 2026-11-18, not the
design:

- **Built**: the code does it. A test name follows where one exists.
- **Partly built**: some of it is there; the rule says which part is missing.
- **Not built**: nothing does it yet. The owning task is named.
- **In progress**: another agent is changing it now (DV-5 virtual rows in
  `ui/src/panel_changes.rs` and `ui/src/document_tab.rs`). Do not count it
  as done.

Paths are under `apps/humata2/`. Line numbers are from 2026-11-18 and drift.

## 1. Purpose

I can read any change in a session, across one or many repos, at any size,
and act on it: comment, push, or review a pull request with an agent beside
me. The code is the page; the file list is a navigator. One row of chrome per
thing, one blue action per screen. Problems show only where they block an
action. Full screen is always one click away, and leaving it puts me back
exactly where I was, with the terminal still running.

## 2. Surfaces

### 2.1 Beside the terminal (narrow)

Mockup: `diff-proposal` §1 and §4. Frames: 84 `84-diff-per-repo-beside-terminal`,
41 `41-review-beside-terminal`, native `changes-pushed`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-1 | The panel header is one 36 pt row: "Diff", the base chip, then ⤢ Full screen and ✕ Close at the right. | Built | `ui/src/panels.rs:122-178` |
| DS-2 | With a file open, the list folds into a one-line **file bar**: ⌄, repo · file name · dim folder, "2 of 4", ‹ ›, and ↗ (Open in editor, an icon button). The code gets the rest of the height. | Not built (DV-15). Today the list sits above the code in a box of at most 220 pt. | `ui/src/panel_changes.rs:723-741` |
| DS-3 | Clicking the file bar or ⌄ opens the list in place, in a box of at most 220 pt that scrolls. Picking a file folds it again. | Partly built: the 220 pt scrolling box is built (DV-1, `desktop/tests/native/diff_view.rs::a_long_file_list_leaves_room_for_the_code`); folding is not (DV-15). | `panel_changes.rs:739` |
| DS-4 | With no file open, the list fills the panel. | Built by default: a file is always open while any exist (`selected` falls back to the first file, `ui/src/panels.rs:201-226`), so this state never arises. Recheck once DV-15 folds the list. | `panel_changes.rs:739` |
| DS-5 | Each repo is one row: a chevron that collapses it, the name without a leading "/", and its totals at the right. Its branch shows only when it differs from the session's branch. Its base shows ("vs release/2.4 ⌄") only when it differs from the header's. | Partly built: the collapse action exists (DV-7, `features/review/src/lib.rs:926`, `features/review/tests/collapse.rs`); the row is still two rows with a dead chevron and the session's branch (DV-10, DV-16). | `panel_changes.rs:1425-1465` |
| DS-6 | The "N files · M repos +a −r" summary row goes. **Add repo…** stays reachable (see §8, item 3). | Not built (DV-16). | `panel_changes.rs:413-450` |
| DS-7 | A file row shows the file name first, then the folder dimmed and ellipsized, then the comment count and +/−. Its tooltip and accessibility label carry the full path. | Partly built: the label is the full path (`panel_changes.rs:111`); the row draws the whole path in one clipped text with no ellipsis and no tooltip (DV-10). | `panel_changes.rs:131-140` |
| DS-8 | A 14 pt status column replaces the file icon: A (green), D (red), R (secondary); modified files show nothing. A binary file shows "bin" where +/− would be; a mode-only change shows "mode". | Not built (DV-12). The data is there: `FileDiff.change`, `binary`, `old_path` (DV-2, `clients/src/model.rs:1108-1140`). Every row draws the same file icon (`panel_changes.rs:127`). | |
| DS-9 | A "Filter files" field appears above the list once a session has more than 20 changed files. | Not built (no DV task: REVIEWUI). | |
| DS-10 | The footer shows only when it has an action. **Push & PR…** is one quiet text button at its left (it opens the push plan for every repo). It shows when the plan reports unpushed commits or uncommitted files, and also while no plan is known; never hidden on a guess. **Send N comments** is the one blue button at its right. | Not built (DV-16). Today the footer always shows a draft count, and Push & PR sits on every repo's second row. | `panel_changes.rs:745-792`, `:1787` |
| DS-11 | One blue action per screen. Counts, chips and the agent's name are not blue. | Partly built: "Push & PR", the comment count and Send are all accent blue today. | `panel_changes.rs:146-156`, `:1787-1800` |

### 2.2 Full screen

Mockup: `diff-proposal` §2, `review` §4 and §5. Frames: 85
`85-diff-per-repo-full-screen`, 11 `11-panel-full-screen-diff`, 68b
`68b-pr-review-full-screen`, 12 (the shared title bar, Board).

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-12 | Three ways in: the ⤢ button, ⌥⌘↩, and a double-click on the panel header. | Partly built: the button only. | `panels.rs:150-163` |
| DS-13 | Ways back: **Show the terminal**, ⌥⌘↩ again, or Esc when no menu, sheet or comment box is open. | Partly built: the button only (`panels.rs:341-350`). Esc does not leave full screen (`features/app/src/overlay.rs:56`). | |
| DS-14 | Full screen has **one** title bar (52 pt): the window lights, "# channel › session" (ellipsized), "Diff" with the base chip or the PR line beside it, then **Show the terminal** at the right. The panel's own 36 pt "Diff" header with its second ⤢ is not drawn. | Partly built: the title bar is built and its names ellipsize before Show the terminal (`panels.rs:297-354`). The panel header is still drawn under it (the body calls `header()` again, `panel_changes.rs:622`, `:695`). DV-18. | |
| DS-15 | The list stays 300 pt wide at the left; the code fills the rest in the content colour from top to bottom (no grey void). Comment cards and the comment box are at most 720 pt wide and start at the code column. | Partly built: the 300 pt list is built (`panel_changes.rs:729-735`); cards cap at 640 pt (`ui/src/comment_cards.rs:82`); the light-mode grey void is not fixed (DV-17). | |
| DS-16 | The full-screen footer is the narrow footer: the same actions, including **Submit review…** in a PR review. | Not built (DV-18). | `panel_changes.rs:745-792` |
| DS-17 | The panel's arrow is named **Full screen** and has a tooltip "Full screen ⌥⌘↩". The session header's arrow (terminal full screen) has "⇧⌘↩". In full screen the panel's button reads Show the terminal. | Not built (DV-18): the label is "Expand panel" (`panels.rs:157`), and `icon_action` sets no tooltip (`panels.rs:92-111`). | |
| DS-18 | Nothing switches to full screen by itself, however wide the content. | Built (nothing does). | |

### 2.3 How code reads

Mockup: `diff-proposal` §1–§2. Frames: native `changes-wrapped-long-line`
and `-dark`, 50 `50-diff-comment-box`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-19 | The gutter has two number columns (old, new) in full screen and one beside the terminal (the new number, or the old one on a removed line). Numbers are secondary, right-aligned, and are the comment click target. | Partly built: one number column, the click target works (`desktop/tests/native/review.rs::clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once`); two columns are DV-17. | `panel_changes.rs:2360-2400` |
| DS-20 | After the numbers: a one-character sign column, one space, then the code. Removed and added rows share one geometry. Row tints are the status green and red at about 12% (light) and 16% (dark), full bleed, the gutter tinted too, no borders between rows. | Not built (DV-17). | |
| DS-21 | Word-level highlight only where a run of removed lines is replaced by as many added lines and under half the line changed; the changed span gets about 28%. Computed only for rows in view. | Not built (DV-17). | |
| DS-22 | Hunks are separated by a 24 pt dim band showing only the section text ("impl Client {"), or a thin line when there is none. No `@@` numbers. | Not built (DV-17; DV-6 dropped). | |
| DS-23 | "No newline at end of file" is a dim row with no number and no click target. | Partly built: the daemon sends the flag (DV-8, `api/proto/humata/v1/review.proto:319`, `daemon/review_diff/tests/modes.rs`); the client and UI drop it (DV-11, DV-12). | |
| DS-24 | The file header reads `repo · folder/name` with the name in primary text and ↗ at the right (an icon button with a tooltip). Under it, one dim note line only when there is something to say: "Renamed from src/http.rs (92% alike)", "Binary file · 12 MB", "Mode 644 → 755", "New file", "Deleted". A file with no lines shows the note as its body. | Partly built: the header row is drawn with the whole `repo/path` and a dimmed, inert "Open in editor" text (`panel_changes.rs:583-601`). The note is DV-12. | |

### 2.4 Big files and many files

Mockup: `diff-proposal` §3. Owner's stress list: [DIFFVIEW.md](DIFFVIEW.md) "Stress cases".

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-25 | Only the rows on screen are built, so a 40,000-line diff opens at once. No "load anyway" gate. | In progress (DV-5): files over 1,000 lines use a virtual list (`panel_changes.rs:520-553`, `VIRTUAL_FROM` at `:2110`); tests in `desktop/tests/native/diff_big.rs`. | |
| DS-26 | The file header stays put while the code scrolls, in every mode. | Partly built: beside the terminal and in full screen the header is outside the scroll (`panel_changes.rs:583-603`). In a PR review the file's head scrolls with the list (`review_repo`, `:889`). | |
| DS-27 | When a file has more than one screen of changes, the file header gains "Change 3 of 212 ‹ ›" (⇧⌥⌘↓ / ⇧⌥⌘↑). It is how you move, not a statistic, so it never shows for a short file. | Not built (DV-15). | |
| DS-28 | While a big file's lines arrive, its body shows a dim "Loading big.txt…". | Not built (DV-12). | |
| DS-29 | In a PR review, a file over 1,500 lines is drawn in its own box of about 60% of the panel's height, with its own scroll; the agent's focus scrolls inside it. Shorter files keep frame 68's inline look. | Not built (DV-5 step 5): `review_code` builds every line (`panel_changes.rs:948`). | |
| DS-30 | A long file list scrolls in both widths, so the last repo of 586 files is reachable. | Built (DV-1): `diff_view.rs::a_long_file_list_leaves_room_for_the_code`, `::the_file_list_scrolls_in_the_expanded_panel`. | `panel_changes.rs:721-741` |
| DS-31 | A pull request with a 20,000-line change, 300 files, a 200-char path, a rename, a binary and a mode change lists every file and pages the big file to its end. `pr_demo` opens that PR in the app with no network. | Built (DV-9): `daemon/humatad/tests/pr_diff_stress.rs::a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end`, `testing/pr_demo/`. | |

### 2.5 States: loading, empty, error, partial

Mockup: `diff-proposal` §4. Frames: 98 `98-diff-no-default-branch`, 86
`86-diff-repo-base-picker`; new frames `empty` and `loading` (DV-13).

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-32 | **Empty:** centred secondary text, "No changes against main", and one button, **Compare with…**, that opens the base menu. No icon art. | Partly built: the text (`nothing_against`, `panel_changes.rs:1103-1120`); the button is DV-19. | |
| DS-33 | **First load:** a dim "Reading changes…". No spinner. | Not built (DV-12, DV-19). | |
| DS-34 | **Base switch:** the chip names the picked base at once; the previous files stay, dimmed, until the new diff arrives. | Partly built: the chip names the picked base only before the first diff arrives (`panel_changes.rs:603-608`); after that it shows the old base until the load ends (report finding 6). DV-12. | |
| DS-35 | **One repo fails:** its message and **Try again** sit under its own row; the other repos still work. | Not built (DV-19). A failed read replaces the whole body today. | `panel_changes.rs:1122-1197` |
| DS-36 | **One file fails:** the reason is that file's body. | Not built (DV-12). `FileDiff.error` exists (`clients/src/model.rs:1140`) and is never set. | |
| DS-37 | **The whole diff fails:** a failed first read names the error with Try again; a failed refresh keeps the last diff under a line with Try again. | Built: `desktop/tests/native/review.rs::a_failed_first_read_names_the_error_with_try_again`, `::a_failed_refresh_keeps_the_last_diff_with_try_again`. | `panel_changes.rs:1122`, `:1201` |
| DS-38 | A file deleted while the diff is read is left out; the diff does not fail. | Built (DV-3): `daemon/review_diff/tests/vanished.rs`. | `daemon/review_diff/src/checkout.rs:203` |
| DS-39 | A file's lines come from the snapshot the list was made from, so a file that changes back still shows the change I was shown. | Built (DV-4): `daemon/humatad/tests/review_file_trees.rs`. | `daemon/humatad/src/review.rs:658` |

### 2.6 Long lines: Wrap and Scroll

Mockup: `review` §3. Frames: native `changes-wrapped-long-line` and `-dark`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-40 | The file header has a **Wrap / Scroll** switch (and ⌥Z) that flips the same preference as Settings › Long lines. | Partly built: Settings only (`ui/src/settings.rs:515-545`, `features/settings/src/lib.rs:503`). `ReviewAction::ToggleWrap` exists but nothing sends it. DV-17. | |
| DS-41 | **Wrap:** a long line breaks under the code; continuation rows have no number. | Built: `desktop/tests/native/review.rs::wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row`. | |
| DS-42 | **Scroll:** the numbers and +/− stay put; only the code column slides sideways, as one block per file, as far as its longest line. | In progress (DV-5, DV-17): the Diff panel shifts only the code column (`panel_changes.rs:484-517`, `:2383-2391`). A PR review always draws wrapped (`self.wrap \|\| review`, `:2386`). | |
| DS-43 | A 194,000-character line wraps without freezing the app. | Partly built: about 2 s today (report finding 5); `diff_big.rs::a_huge_line_wraps_in_a_long_file`. To be measured again after DV-5 (P-6). | |
| DS-44 | Widening, narrowing, or going full screen and back keeps the top line in place under Wrap. | Partly built: width changes are covered (`review.rs::widening_a_wrapped_diff_keeps_its_top_line_in_place`); full screen and back is not tested (DV-18). | `ui/src/document_tab.rs:439` |

### 2.7 Long names and truncation

Mockup: `review` §4 ("Full screen never disappears"). Frames: 68, 68b, 68c.

**Owner (2026-11-18):** "Make sure the full screen button for the diff view
always appears and other things in that line truncate instead when there is
too much."

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-45 | **Controls that never shrink**, in every header and bar of the diff: in the panel header, ⤢ Full screen and ✕ Close; in the full-screen title bar, Show the terminal and "Change N of M ‹ ›"; in the file bar and file header, the "2 of 4" counter, ‹ ›, the Wrap switch and ↗; in the footer, every button (Push & PR…, Send N comments, Submit review…); on a repo row, the chevron and totals; on a file row, the status mark, the comment count and +/−. | Partly built: the panel header's buttons (`panels.rs:148-153`, tests `diff_view.rs::a_long_base_keeps_expand_and_close_inside_the_panel`, `desktop/tests/native/pr_session.rs::a_narrow_review_panel_keeps_full_screen_and_close`) and Show the terminal (`panels.rs:83`, `:314-350`). The file bar, Wrap switch and counter don't exist yet. | |
| DS-46 | **What gives way, first to last:** the base chip's text, then the PR line, then branch, then repo name, then the folder, each with an ellipsis. The full text is in the tooltip and the accessibility label. A PR line can shrink down to just "PR #482". | Partly built: the base chip (`panel_changes.rs:611-617`), the PR line (`:840-846`) and the title bar's names (`panels.rs:315-334`) ellipsize. Repo and branch don't (DV-10). No tooltips. | |
| DS-47 | A long path keeps its file name: the folder ellipsizes before the name does, so two long paths that differ only at the end both read differently. | Not built (DV-10). | `panel_changes.rs:131-140` |
| DS-48 | A local repo reads "large", not "/large". | Not built (DV-10). | `clients/live/src/map.rs:64` |
| DS-49 | Each repo row shows its own clone's branch when it differs from the session's. | Not built (DV-10): `repo_head` draws the session's branch. | `panel_changes.rs:1447` |
| DS-50 | The base menu opens anchored by its right edge and never clips at the panel edge. | Not built (DV-14 recheck, then a follow-up). | `panel_changes.rs:1283` |
| DS-51 | Unicode and emoji names, CRLF files and minified lines draw correctly. | Code, no test: correct in the live run (report, "Stress cases"); frames are DV-13. | |

### 2.8 PR review session

Mockup: `review` §1, §2, §5. Frames: 67 `67-pr-review-dialog`, 68
`68-pr-review-session`, 68b, 68c, 69 `69-pr-submit-review`.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-52 | The panel header names what is reviewed: "PR #482 against main · 2 repos", which ellipsizes before ⤢ and ✕. | Built: `pr_session.rs::the_diff_names_the_pull_request_and_each_repository`, `::a_narrow_review_panel_keeps_full_screen_and_close`. | `panel_changes.rs:818-846` |
| DS-53 | The file bar steps files across every PR of the review ("1 of 6 ‹ ›"). | Not built (DV-15). | |
| DS-54 | An agent's draft is outlined in blue and marked "Draft for GitHub", with Edit and Drop. | Partly built: the label, Edit and Drop (`ui/src/comment_cards.rs:143`, `:341`; `pr_session.rs::a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts`). The blue outline is only on the open box (`comment_cards.rs:86`). | |
| DS-55 | The footer counts the drafts ("4 drafts · 3 by Copilot, 1 by you") and holds the one blue **Submit review…**, beside the terminal and in full screen. | Partly built: the count is in the footer (`panel_changes.rs:755-765`); Submit review is in the session bar (`ui/src/pages.rs:316-330`), not the footer, and full screen has none (DV-18). | |
| DS-56 | **Submit review** opens one sheet for every PR of the review: what goes out and who wrote it, an optional summary, Comment / Approve / Request changes, then Cancel or Submit to GitHub. Nothing is sent before this. | Built: `ui/src/overlay_submit_review.rs`, frame 69, flow §26.17 (`desktop/tests/live_flows/pr_review.rs`). | |
| DS-57 | New commits on a PR show as a bar under the terminal ("2 new commits on the PR branch", or "The PR branch was force-pushed") with **Update the clone**. A merged or closed PR offers Archive. | Built: `pr_session.rs::update_the_clone_sends_the_update_and_clears_the_notice`, `::a_merged_pull_request_offers_archive`. | `ui/src/pages.rs:500-580` |
| DS-58 | **Back:** every session page has a ‹ ghost button at the start of its top bar, tooltip "Back ⌘[", that goes to wherever I was. | Partly built: ⌘[ works (`ui/src/keys.rs:204`); other pages draw the button (`ui/src/shell.rs:211-216`); the session header has none (`ui/src/pages.rs:129-200`). DV-18. | |
| DS-59 | The line the agent focuses is marked and scrolled into view; files are listed bottom-up. | Built: `pr_session.rs::the_line_the_agent_focuses_is_marked`, `::a_focus_below_the_fold_is_scrolled_into_view`, `::files_are_listed_bottom_up`. | |

### 2.9 Stacks across repos

Mockup: `stacks` §1–§3. Frames to come: 119 `119-diff-stack-whole`, 120
`120-diff-stack-layer`, 121 (the skim fold open). The stacks UI is owned by
[STACKVIEW.md](STACKVIEW.md); its tasks are named per rule.

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-60 | A stack shows a layer list above the files: **Whole stack** first, then the layers top-down with 1 at the bottom. Each row has its number, title, its PRs per repo ("api #412 · web #88", collapsing to "+N"), and its file count. About 6 rows show; the list scrolls and keeps the picked row in view. | Not built (STACKVIEW T8). | |
| DS-61 | The picked row is highlighted like a picked file and shows its one-line purpose. | Not built (T8). | |
| DS-62 | The header reads "Whole stack · 3 layers" or "Layer 2 of 3 · journal"; the base picker is hidden while a stack shows. | Not built (T7, T8). | |
| DS-63 | A layer shows only the repos it touches; a repo it doesn't touch gets one quiet line ("web isn't in this layer"). | Not built (T6 repo filter, T8). | |
| DS-64 | A layer's diff is its own base..head, right whatever is checked out. | Partly built: the engine (STACKVIEW T1, T2; `daemon/review_diff/tests/stack.rs`); the daemon service and app are T4, T6, T7. | |
| DS-65 | A repo that moved after the stack was sent says "web moved since it was sent" on that layer, with one action, **Ask the agent to send it again**. | Partly built: the engine's drift check (T2 `drift_blocking`); the row is T4, T8. | |
| DS-66 | A stack with no notes draws no review chrome: no note gutters, no draft affordances, no verdicts. | Not built (T8 test). | |
| DS-67 | Key files carry a dot; skim files fold into one row at the end of each repo ("3 files to skim · Cargo.lock, …") that opens in place. | Not built (T3c, T8; frame 121). | |
| DS-68 | Outside a review chat, a stack arrives as an Inbox offer ("3 layers over 2 repos", Open the stack, Later); opening it starts on Whole stack. | Not built (T4, T8). | |

## 3. Keyboard

Every key below goes in the ⌘/ shortcuts sheet (`ui/src/keys.rs:84`,
`SHORTCUTS`). None of the new ones is bound yet.

| Keys | Does | Where it works | Status |
|---|---|---|---|
| ⌥⌘↩ | Diff full screen on and off (DS-12, DS-13) | A session with its panel open | Not built (DV-18) |
| ⇧⌘↩ | Terminal full screen | A session | Built (`keys.rs:224`), but missing from the ⌘/ sheet |
| Esc | Leave full screen, after any menu, sheet or comment box has closed first (DS-13) | Full screen | Not built (DV-18) |
| ⌥⌘↓ / ⌥⌘↑ | Next / previous file (DS-2, DS-53) | Diff panel open | Not built (DV-15). **Conflict:** STACKVIEW binds these to next / previous layer (see §8, item 1) |
| ⇧⌥⌘↓ / ⇧⌥⌘↑ | Next / previous change (DS-27) | Diff panel open | Not built (DV-15) |
| ⌥⌘] / ⌥⌘[ | Next / previous layer (DS-60) | A stack shows | Proposed (§8, item 1). Not built (STACKVIEW T8) |
| ⌥Z | Wrap / Scroll (DS-40) | The diff has focus | Not built (DV-17). **Conflict:** ⌥Z types "Ω" in a text box and reaches the terminal as a key; bind it only in the diff's key context (§8, item 2) |
| ↑ ↓ ↩, ← → | In the focused file list: move, open, collapse or expand a repo | File list focused | Not built (DV-15) |
| ⌘[ | Back (DS-58) | Anywhere | Built (`keys.rs:204`) |
| Double-click the panel header | Full screen (DS-12) | Panel header | Not built (DV-18) |

The palette gets "Next layer", "Previous layer" and "Whole stack"
(STACKVIEW T8). The four ⌥⌘ arrows need a session context so they never
shadow the sidebar's ⌥⌘← and ⌥⌘→ (`keys.rs:211-212`).

## 4. Behaviour rules

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-69 | **Show the terminal keeps my place.** Going full screen and back keeps the open file, the layer, the focused line, the scroll position, the sideways shift, the Wrap choice, any open comment box and every draft. The terminal keeps running underneath; nothing is reloaded. | Partly built: the open file is kept (`pr_session.rs::show_the_terminal_returns_the_review_beside_the_terminal`). The toggle sends no fetch (`features/app/src/lib.rs:1153-1176`) and scroll lives in `Documents` across the toggle (`ui/src/document_tab.rs:553`), but neither is tested. | |
| DS-70 | Full screen is remembered per session: another session opens in its own state, and coming back restores this one's. | Built: `AppState.full_screens` per session (`features/app/src/lib.rs:356-363`). Code, no test for the switch. | |
| DS-71 | Closing the panel ends full screen. | Built (`features/session/src/lib.rs:1260-1265`). | |
| DS-72 | The open file is remembered per session and survives a restart. | Built (`features/review/src/lib.rs:102-108`). | |
| DS-73 | Scroll position is remembered per session and file. | Likely bug: kept by `repo/path` only (`ui/src/document_tab.rs:553`, called with `format!("{}/{}", diff.repo, file.path)` at `panel_changes.rs:475`), so two sessions on the same repo and file share one scroll. | |
| DS-74 | Collapsed repos are per session and are not kept across a restart. | Partly built: the reducer (DV-7); no UI (DV-10). | `features/review/src/lib.rs:226` |
| DS-75 | Wrap is one app-wide preference. | Built (`features/app/src/lib.rs:317`, `:1215`). | |
| DS-76 | Opening, closing or widening the panel does not read the whole diff again. | Not built: toggling the panel refetches everything (report finding 6). REVIEWUI. | |
| DS-77 | No status that doesn't help me act: no summary row, no repeated session branch, no "N behind". A problem shows where it blocks: under its repo, in its file, or on the action it stops. | Partly built: see DS-6, DS-35, DS-36, DS-49. | |
| DS-78 | Picking a file in a collapsed repo, or stepping onto it with ⌥⌘↓, opens that repo. Collapsing the repo that holds the open file keeps the file open. | Partly built: the second half (DV-7 test); the first is DV-15. | |

## 5. Performance budgets

Numbers marked **source** come from the report or the DV list. Numbers
marked **proposed** are not in any source; they are this spec's proposal and
need main's bench to confirm (DV-14).

| ID | Case | Budget | Origin | Today (report) |
|---|---|---|---|---|
| P-1 | First open of a 40,000-line file (+20,000 −20,000) | Opens "at once": proposed under 300 ms to first rows | source (design §2 "opens at once"); number proposed | ~70 s |
| P-2 | Reselecting that file | Proposed under 100 ms | proposed | ~25 s at 100% CPU |
| P-3 | Scrolling that file | Proposed: no dropped frames at 60 Hz, under 30% of one core | proposed | 72–99% CPU |
| P-4 | Memory with that file open | Proposed: under 300 MB above the idle app | proposed | 0.5 → 4.8 GB |
| P-5 | Loading 586 files over 4 repos | Under 5 s, or a follow-up opens to load a file's lines only when it is picked | source (DV-14) | 177 files ~30 s |
| P-6 | Wrapping a 194,000-character line | Measure again after DV-5; proposed under 500 ms | source (design, "measure it again"); number proposed | ~2 s |
| P-7 | Base switch | The chip names the new base in the same frame; proposed under 5 s for the new diff on the 4-repo session | source (DS-34); number proposed | 30–60 s+ |
| P-8 | Idle | About 0% CPU with the panel open | source (report "What works") | ~0% |
| P-9 | Word highlight and wrap | Computed only for rows in view | source (design §2, DV-5 spec 2) | n/a |

## 6. Accessibility

| ID | Rule | Status | Where |
|---|---|---|---|
| DS-79 | Every icon button has an accessibility label and a tooltip that names it and its key ("Full screen ⌥⌘↩", "Close panel", "Open in editor", "Previous file ⌥⌘↑"). | Partly built: labels yes (`panels.rs:92-111`); tooltips no. | |
| DS-80 | Every ellipsized text shows its full text on hover and carries it in its label (file paths, repo, branch, base, PR line, session title). | Partly built: labels for paths and the base (`panel_changes.rs:111`, `:640`); no tooltips (DV-10 notes the helper question). | |
| DS-81 | The code area is a labelled code region; file rows are list options that say whether they are picked. | Built (`panel_changes.rs:106-113`, `:528-530`). | |
| DS-82 | Status marks are letters (A, D, R, bin, mode), never colour alone. | Not built (DV-12). | |
| DS-83 | After Show the terminal, focus goes back to where it was before full screen (the terminal, or the diff). | Not built (proposed; DV-18). | |
| DS-84 | The file list can be driven by keys alone (DS keyboard map), and the focus ring is visible. | Not built (DV-15). | |

## 7. Frames that hold these rules

Design (`desktop/tests/__snapshots__/design/`): 11, 12, 41, 50, 51–53, 67,
68, 69, 84, 85, 86, 98. Native (`desktop/tests/__snapshots__/native/`):
`11-panel-full-screen-diff`, `41-review-beside-terminal`, `68-pr-review-session`,
`68b-pr-review-full-screen`, `68c-pr-review-after-show-terminal`,
`69-pr-submit-review`, `changes-pushed`, `changes-wrapped-long-line`,
`changes-wrapped-long-line-dark`. New frames planned: `narrow-list-folded`,
`narrow-list-open`, `empty`, `loading` (DV-13), every `diff-stress-*`
(DV-13), 119–121 (STACKVIEW). Each new or changed frame goes to Fable for a
pick before it goes to the owner (D76).

## 8. Contradictions between sources, and how this spec settles them

1. **⌥⌘↓ / ⌥⌘↑ mean two things.** The design (and mockup `diff-proposal` §1)
   uses them for next / previous file; STACKVIEW T8 and mockup `stacks` use
   them for next / previous layer, and T8's test expects "alt-cmd-down picks
   layer 1". Files move far more often than layers, and the design ranks
   higher. **Settled:** files keep ⌥⌘↓/↑; layers move to ⌥⌘] / ⌥⌘[ (free in
   `keys.rs`), and the palette keeps Next layer and Previous layer. STACKVIEW
   T8's test and the `stacks` mockup's key hint change with it. Fable to
   confirm before T8.
2. **⌥Z for Wrap.** Option is not Alt in the terminal (`terminal/vt/src/terminal.rs:223`),
   so ⌥Z types "Ω" there and in text boxes. **Settled:** ⌥Z is bound only in
   the diff's own key context (the code or the file list focused), never
   app-wide.
3. **The summary row goes, but Add repo… lives in it** (`panel_changes.rs:424-445`,
   REPOS CH-21). **Settled:** Add repo… moves to the footer's left, beside
   Push & PR…, as a second quiet text button; it is always an action, so a
   session's footer always shows. Fable to pick in DV-16's frames.
4. **Where Submit review lives.** Frame 68 puts it in the session bar
   (`pages.rs:316-330`); the owner-approved mockup (`review` §1) puts it in
   the panel footer. **Settled:** the footer holds it beside the terminal and
   in full screen (DS-55). While the Diff panel shows, the session bar's
   button is hidden, so the screen has one blue action; with the panel closed,
   the session bar keeps it. Frame 68 is re-recorded.
5. **Where "Change N of M" sits.** The design puts it in the pinned file
   header, only for files with more than a screen of changes; mockup `review`
   §5 draws "Change 3 of 12 ‹ ›" in the full-screen title bar of a small
   review. **Settled:** the file header, under the design's condition
   (DS-27). The title-bar placement is treated as a sketch.
6. **Card width.** The design caps cards at 720 pt; the code caps them at
   640 pt (`comment_cards.rs:82`). 640 is inside the cap, so it stays; only the
   start at the code column is new.
7. **When rows go virtual.** The Diff panel switches at 1,000 lines
   (`panel_changes.rs:2110`); DV-5 keeps PR review files whole up to 1,500
   lines so frame 68 doesn't move. Both stand: they are different surfaces
   (DS-25, DS-29).
8. **Draft count wording.** Code: "Drafts: 4 (3 by agent, 1 by you)";
   mockup: "4 drafts · 3 by Copilot, 1 by you". The mockup's wording is the
   target, naming the agent. Tests don't assert wording.

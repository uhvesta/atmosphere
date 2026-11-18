# DIFFVIEW design proposal (Fable, 2026-11-18)

A redesign of the Diff side panel, judged from frames 84, 85, 86, 11, 50 and 41 against the house style (61 Files, 29 Settings, providers-a-at-rest). Proposal only: no code is changed here.

## 1. What looks wrong now

- **More chrome than content.** In 84, four file rows sit under seven rows of chrome: the header, "4 files · 2 repos", and for each repo a head row plus an "against … / Push & PR" row. Files and Settings never spend two rows on one thing.
- **The diff gets what is left.** Beside the terminal, the list, the file header, the code and the comment cards stack in one 530 px column. The code is six lines squeezed between the list and two cards, and the removed line is cut off at the rail.
- **Too many blue things.** "Push & PR" twice, the comment count, "Draft", "Send" and "Send 1 comment" are all accent blue in one column. Elsewhere a screen has one blue action (Copy in Files, Save version in 41).
- **Status that doesn't help you act.** The "4 files · 2 repos +285 −42" row, and the branch "ledger-schema" repeated on every repo although it is the session's branch.
- **Broken or vague chips.** In 11 the per-repo base row is a lone chevron under each repo. "Comparing per repo" is a sentence in a chip. "Open in editor" is grey text that doesn't read as a button.
- **Raw git in a quiet app.** `@@ −40,7 +40,12 @@ impl Client {` is the only raw syntax on screen. One number column mixes old and new numbers ("41 −" then "41 +"). The sign sits four spaces from the code. In 84 the removed row's gutter is tinted and shifted and the added rows' gutters aren't.
- **Full screen is a stretched sidebar** (85, 11, 50). Two header bars both say "Diff". There are three ways out (Show the terminal, the shrink icon, X). Comment cards run 1,500 px wide. In light mode the code is a white strip floating over a grey void, so it reads as a widget, not a page.
- **Every file has the same icon**, so the icon column carries nothing, and added, renamed and binary files look like any other.
- **Two identical expand arrows** in 84 (session header and panel header) do different things, with no tooltip to tell them apart.

## 2. The proposed design

One idea runs through all three states: **the code is the page; the file list is a navigator.** Chrome is one row per thing, and one blue action per screen.

### Beside the terminal (narrow)

```
 Diff  [vs main ⌄]                    ⤢  ✕     panel header, 36 px (unchanged)
 ⌄ api · client.rs  src/   2 of 4  ‹ ›  ↗      file bar: click = open the list
 ───────────────────────────────────────
     impl Client {                             hunk separator (dim band)
 40    pub async fn post(&self, entry…
 41 −  self.db.insert_legacy(&entry)…
 41 +  let tx = self.db.begin().await?;
        ┌ You · Draft ────────── Send ┐        card under its line
 ───────────────────────────────────────
 Push & PR…                [Send 1 comment]    footer, only when it has an action
```

- With a file open, the list **folds into the file bar**: repo · file name · dim directory, "2 of 4", ‹ › to step files, and ↗ (Open in editor, an icon button with a tooltip). The code gets all the height.
- Clicking the bar (or ⌄) opens the list in place as DV-1's `changes-files` box (max 220 px, scrolls). Picking a file folds it again. With no file open, the list fills the panel.

### Full screen

```
 ● ● ●  # payments-rewrite › Ledger schema migration
        Diff  [vs main ⌄]                              [Show the terminal]
 ─────────────────┬───────────────────────────────────────────────────────
 Filter files     │ api · src/client.rs   renamed from src/http.rs      ↗
 ⌄ ledger +142 −18│──────────────────────────────────────────────────────
   0042_accounts… │        impl Client {
   posting.rs     │ 40 40    pub async fn post(&self, entry: Entry)
 ⌄ api    +143 −24│ 41     − self.db.insert_legacy(&entry).await?;
  ▸client.rs  □2  │    41  + let tx = self.db.begin().await?;
   entry.rs       │          ┌ You · line 41 ─────────────┐  cards ≤ 720 px
 ─────────────────┴───────────────────────────────────────────────────────
 Push & PR…                                             [Send 1 comment]
```

- **One header bar.** The base chip moves into the 52 px title bar beside "Diff"; the second bar (title again, shrink icon, X) goes. "Show the terminal" is the one way back.
- The list stays 300 px wide. The code area is the content colour from top to bottom (no grey void). Comment cards and the comment box are capped at 720 px and start at the code column.

### A single big file

```
 api · big.txt   +20,000 −20,000          Change 3 of 212  ‹ ›   ↗
 ──────────────────────────────────────────────────────────────────
  (virtual rows; the file header stays put while the code scrolls)
```

- Rows are virtualized (DV-5). The file header is pinned and gains "Change 3 of 212 ‹ ›" **only when the file has more than one screen of hunks**: it is how you move, not a statistic.
- No "load anyway" gate: the diff opens at once. While the lines arrive, the body shows a dim "Loading big.txt…".

### File list

- **Flat per repo, not a tree.** Changed sets are sparse, and folder rows add chrome. Each row: **file name first**, then the directory dimmed and ellipsized, then the comment count and +/−. The name is always readable; the tooltip and aria label carry the full path.
- **One row per repo:** chevron (collapses, DV-7), name without a leading "/", totals at the right. The branch shows only when it differs from the session's branch. The base shows on that row as a dim "vs release/2.4 ⌄" only when it differs from the header's base. The summary row goes.
- **A status mark replaces the file icon**, in a 14 px column: A (green), D (red), R (secondary). Modified files show nothing. A binary file shows "bin" where +/− would be; a mode-only change shows "mode".
- A "Filter files" field appears above the list once a session has more than 20 files.

### File header

- `repo · dir/name` with the name in primary text; ↗ at the right. Under it, one dim note line only when there is something to say: "Renamed from src/http.rs (92% alike)", "Binary file · 12 MB", "Mode 644 → 755", "New file", "Deleted". For a file with no lines, the note is the body.

### Hunk look

- **Gutter:** full screen has two number columns (old, new); narrow has one (the new number, or the old one on a removed line). Numbers are secondary text, right-aligned, and are the comment click target as today. Then a one-character sign column, one space, then the code. Removed and added rows share the same geometry.
- **Colours:** row tints are the status green and red at about 12% (light) and 16% (dark), full bleed, with the gutter the same tint. No borders between rows.
- **Word-level highlight: yes, but narrow.** Only where a run of removed lines is replaced by the same number of added lines and under half the line changed, the changed span gets the same colour at about 28%. It is computed only for rows in view.
- **Hunk separator:** a 24 px band in the panel's background showing only the section text ("impl Client {") dimmed, or a thin line when there is none. The `@@` numbers go: the gutter already has them. "\ No newline at end of file" is a dim row with no number.

### Base picker and Push & PR

- **Base:** one chip in the header reading "vs main" (or "Per repo" when repos differ). Its menu is today's (86), anchored by its right edge so it never clips. A repo's own base row appears only in per-repo mode.
- **Push & PR:** one quiet text button at the left of the footer, not one per repo. It opens the existing push plan, which lists each repo. It shows when the push plan reports unpushed commits or uncommitted files in some repo, and also while no plan is known (never hidden on a guess). The footer is hidden when there is nothing to push and no draft.

### Empty, loading and error states

- **Empty:** centered secondary text, "No changes against main", with one button, "Compare with…", which opens the base menu. No icon art.
- **Loading:** the chip names the picked base at once; the list shows the previous files dimmed; the first load shows a dim "Reading changes…". No spinner.
- **Error:** shown where it blocks. A repo that can't be read gets the message and "Try again" under its own row, and the other repos still work. A file that can't be read gets the reason in its body. A whole-diff failure uses the existing banner.

### Keyboard

All free in `ui/src/keys.rs` today.

| Keys | Does |
|---|---|
| ⌥⌘↩ | Diff full screen on and off (pairs with ⇧⌘↩, terminal full screen) |
| ⌥⌘↓ / ⌥⌘↑ | Next / previous file, from anywhere while the panel is open |
| ⇧⌥⌘↓ / ⇧⌥⌘↑ | Next / previous change (hunk) |
| ↑ ↓ ↩, ← → | In the focused list: move, open, collapse or expand a repo |
| Esc | In full screen with no menu or box open: show the terminal |

The four ⌥⌘ arrow keys need a session context so they don't shadow the sidebar's ⌥⌘← / ⌥⌘→. All five go in the ⌘/ shortcuts sheet.

## 3. Reaching full screen

- **"Expand" is the right control with the wrong name.** Keep the arrow glyph: the session header's arrow makes the terminal full screen, and the panel's arrow makes the panel full screen. Rename it "Full screen" and give both arrows a tooltip with their shortcut (⌥⌘↩ and ⇧⌘↩), so they stop reading as duplicates.
- **Three ways in:** the arrow button, ⌥⌘↩, and a double-click on the panel header (the macOS zoom habit).
- **Ways back:** "Show the terminal", ⌥⌘↩ again, or Esc. The shrink icon and X leave the full-screen view; closing the panel is done from the split view.
- No automatic switch to full screen for wide content: the owner decides.

## 4. Changes to the DV task list

**Wave 1 stays valid.** DV-1's `changes-files` box becomes the opened list, and its header changes are kept. DV-2's fields feed the status marks and notes. DV-3 and DV-4 are untouched. One DV-1 assertion changes: with a file selected in the narrow panel the list is folded, so `a_long_file_list_leaves_room_for_the_code` opens the list before measuring it (done in DV-15).

**Changed**
- **DV-5:** unchanged in scope, but it draws rows through one row function that DV-17 restyles, and it pins the file header.
- **DV-6:** dropped. The `@@` text is no longer drawn, so counts have no reader.
- **DV-10:** file rows are name first with a dim directory (replacing the two-part ellipsis). The branch shows only when it differs from the session's. The repo row is a single row. The rest stands.
- **DV-12:** `file_note` stands. The note goes on the file header's second line. "bin" and "mode" replace +/−, and the status mark is added. Loading follows the section above.
- **DV-13:** frames are recorded in the new look; add `narrow-list-folded`, `narrow-list-open`, `empty` and `loading`.
- **The earlier "Push & PR" rejection is replaced:** one footer button, driven by the push plan (DV-16).

**New**
- **DV-15: the narrow file bar and stepping** (Opus). Review reducer actions for next/previous file and change and list open/closed; the file bar; the bindings in `keys.rs`. After DV-5.
- **DV-16: one row per repo and the footer** (Haiku). Remove the summary row and the per-repo second row; base on the repo row only when it differs; Push & PR in the footer; hide the footer when it has no action; right-anchor the base menu. With DV-10.
- **DV-17: the hunk look** (Opus). Gutter columns, tints, the hunk separator and the word-level highlight. After DV-5.
- **DV-18: full screen** (Haiku). One header bar in `panels.rs`, the "Full screen" name and tooltips, double-click, ⌥⌘↩ and Esc.
- **DV-19: empty and error states** (Haiku). "Compare with…", and per-repo and per-file errors in place. With DV-12.

**Order:** wave 2 is DV-5, DV-7, DV-8, DV-9, DV-18. Wave 3 is DV-10 with DV-16, and DV-11. Wave 4 is DV-12 with DV-19, then DV-15, then DV-17. Wave 5 is DV-13 and DV-14. One owner of `ui/src/panel_changes.rs` at a time, as before.

**Goldens to re-record** (each sent to Fable for approval)
- Design: 11, 41 (only if the shared header tooltip changes it), 50, 51–53, 68, 84, 85, 86, 98, and 12 (the full-screen header is shared with Board).
- Native: `11-panel-full-screen-diff`, `changes-pushed`, `changes-wrapped-long-line` and its dark twin.
- Frames with a side panel in the split view (07, 08) should not move: the header keeps its glyphs.

## Long lines and the way back (main, 2026-11-18, from the owner's question)

Gaps found in the code today, to fold into DV-17 and DV-18:
- **Wrap is in Settings only** (Settings › Long lines: Wrap / Scroll; `prefs.wrap_diffs`). Add a "Wrap" toggle to the file header (and ⌥Z) that flips the same preference.
- **Scroll mode:** the whole code body scrolls sideways (`panel_changes.rs` `overflow_x_scroll` on the body), so the line numbers and +/− marks scroll off with the code. Keep the gutter pinned and scroll only the text column. Each file scrolls as one block to its longest line.
- **Wrap mode:** continuation rows sit under the code with no number (frame `changes-wrapped-long-line`). Keep that. A 194k-char line wraps in about 2 s today; with virtual rows (DV-5), measure it again.
- **Show the terminal** keeps the file, the layer and the scroll position. `DiffView::keep_top` already holds the top row across width changes under wrap; test that leaving and entering full screen keeps it (DV-18).
- **Back on sessions:** session pages have no back button (only ⌘[ works). Add the shell's ‹ "Back ⌘[" ghost button at the start of the session top bar (DV-18).
- **PR review in the new look:** the panel header names what's reviewed ("PR #482 · against main · 2 repos"), the file bar steps files across both PRs, agent drafts get a blue outline and "Draft for GitHub", and the footer holds the draft count and the one blue "Submit review…". Mockup: scratchpad review.png (sent to the owner 2026-11-18).
- **Full screen never disappears (owner, 2026-11-18):** in every header and bar of the diff (panel header, file bar, full-screen title bar), the controls never shrink: Full screen/Show the terminal, Close, counters, steppers, ↗. Names, base chips and branches ellipsize first. Done in main for the review panel (its header now carries ⤢ and ✕ and the PR line, which ellipsizes) and the full-screen title bar. Tests: `a_narrow_review_panel_keeps_full_screen_and_close`, `show_the_terminal_returns_the_review_beside_the_terminal`. New frames 68b and 68c; 68 re-recorded. All three need Fable's approval.
- **Still open from those frames:** full screen draws two headers (the title bar plus the panel's own "Diff" header with a second ⤢), and its footer has no Submit review. Both go in DV-18.

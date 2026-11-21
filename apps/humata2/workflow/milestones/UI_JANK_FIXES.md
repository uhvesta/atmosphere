# UI_JANK_FIXES — UI/UX Jank, Edge-Case Breakage, Text Cutoff & Dead-End Audit

## Goal & Constraints
Fix all 26 verified UI/UX jank, crash, keystroke-loss, painted-over menu, focus-trap, text-cutoff, dead-end, and virtualization bugs across `apps/humata2/ui/` and `apps/humata2/features/`.

- Follow `AGENTS.md`: build/test only via `tools/bq/bq` (never `cargo`, `bazel clean`, or `bazel shutdown`), keep crate/visibility boundaries intact, and verify every fix with targeted unit tests (TDD) + desktop snapshot checks where visuals change.
- Coordinate file ownership with [MACOS_POLISH.md](MACOS_POLISH.md) so parallel lanes never collide on the same files.

---

## Wave A — Critical Crashes, Keystroke Loss & Focus/Escape Traps (P0)

### Lane A1 — Crashes, Draft Leaks & `last_synced` Input Guards (`1.1`, `1.2`, `1.3`)
- **1.1 UTF-8 byte-slicing panic on non-ASCII terminal links** (`apps/humata2/ui/workspace/src/terminals.rs:1398`):
  - Replace `uri[..7].eq_ignore_ascii_case("file://")` with `uri.as_bytes().get(..7).is_some_and(|p| p.eq_ignore_ascii_case(b"file://"))` + unit test with CJK/emoji at byte 7.
- **1.2 To-do drawer leaks draft text onto the wrong item** (`apps/humata2/ui/workspace/src/todo.rs:781-797, 892-944`):
  - Track `last_open` in `Inputs::sync()`; clear `ask_input`, `comment_input`, `split_input`, `ext_block_input` and reset open sub-form flags whenever `state.open` changes.
- **1.3 Background redraws wipe in-flight keystrokes & reset carets (7 sites)**:
  - `apps/humata2/ui/changes/src/document_tab.rs:1125-1149`
  - `apps/humata2/ui/workspace/src/request_card.rs:270-274`
  - `apps/humata2/ui/changes/src/doc_history.rs:162-168`
  - `apps/humata2/ui/settings/src/settings_providers.rs:123-218`
  - `apps/humata2/ui/machines/src/machine_page.rs:120-132`
  - `apps/humata2/ui/overlays/src/overlays.rs:339-344`
  - Track `last_synced: String` (matching `shell.rs:563` / `todo.rs:909`) and only call `set_value` when `last_synced != store_text`.

### Lane A2 — Keyboard Hijack, Escape & Focus Traps (`1.4`, `2.4`, `2.7`, `2.8`, `4.8`)
- **1.4 Mass-delete select mode hijacks keys in terminals & inputs** (`apps/humata2/ui/src/shell.rs:200-279, 695-706`):
  - Gate `handle_select_key` so it yields when focus is inside `"HumataTerminal"` or any `"Input"`/`Textarea`.
- **2.4 File Manager modals** (`apps/humata2/ui/workspace/src/file_manager.rs:802-901`, `apps/humata2/features/app/src/overlay.rs:66-278`):
  - Add `.occlude()` + dismiss click to scrim, focus `files_name` on open, and wire `state.files` modals into `dialog_holds_keys()` & `App::escape()`.
- **2.7 `Escape` bugs** (`features/app/src/overlay.rs:95-126, 263-276`, `features/session/src/lib.rs:1782-1814`, `ui/src/shell.rs:1034-1048`):
  - Close inner pickers/`+ Org` before popping `NewSession`/`NewChannel`/`PrReview` sheets; close all 4 review menus (`base_menu`, `repo_picker`, `push_menu`, etc.) in `App::escape` + `escape_closes_something`; clear `state.terminal_menu = None` in `CloseTab`/`CloseGroup`.
- **2.8 To-do `on_key_down` ignores `⌘`/`Ctrl`/`Alt` modifiers** (`ui/workspace/src/todo.rs:1116-1154`):
  - Return early if `platform || control` (and `alt` before `triage_command`); clear local popover cells on bare `Escape`.
- **4.8 Stuck splitter drags, stale Submit Review edit, undismissable toasts & `ReportView` focus** (`shell.rs:870-887`, `panel_changes/mod.rs:1006-1031`, `overlay_submit_review.rs:36-46`, `overlay_toasts.rs:120-142`, `overlays.rs:312-313`):
  - Add `.on_mouse_up_out` + window-deactivation drag cleanup; reset `pr.submit` on open; always show `X` on toasts; add `self.report.form.is_some()` to `dialog_holds_keys()`.

### Lane A3 — Deferred Menus, Clamped Popovers, Scrim `.occlude()` & Modal Scroll (`2.1`, `2.2`, `2.3`, `2.5`, `2.6`, `2.9`)
- **2.1 `machine-menu-card` painted over by page body** (`ui/machines/src/machine_page.rs:471-525`): wrap `action_menu::scrim` and `action_menu::card` in `deferred(...)`.
- **2.2 `new-terminal-menu` clipped by split tab group** (`ui/workspace/src/session_tabs.rs:782, 944-951`): wrap scrim + menu in `deferred(...)`; right-align for group 1 / clamp to viewport.
- **2.3 6 modals lack `.overflow_y_scroll()`** (`overlays.rs:428-443`, `overlay_fork.rs:21-30`, `overlay_pr_review.rs`, `overlay_continue.rs`, `overlay_move.rs`, `overlay_root.rs`, `channel.rs:264-320`): add `.overflow_y_scroll()` (or `.overflow_y_scrollbar()`) to `sheet()`, `fork::card()`, `channel-delete-modal`, and viewport-clamped `max_h` on `overlay_pr_review.rs:380`.
- **2.5 Unclamped menus clip off bottom/right edges** (`sidebar.rs:1005-1068`, `agent_tab.rs:526-558`, `review_header.rs:807-815`, `machine_running.rs:98-100`, `sync.rs:299-319`): clamp `top`/`left` against `window.viewport_size()`, flip upward/leftward near edges, add `max_h` + scroll on `diff-base-menu`/`repo-base-menu`, wrap scrims in `deferred(...)` + `.occlude()`.
- **2.6 Scrims lack `.occlude()`** (`action_menu.rs:19-51`, `channel.rs:252-278`, `shell.rs:1006-1017`, `doc_history.rs:266-277`): add `.occlude()` to all scrims and menu cards.
- **2.9 Tall terminal notices clip off bottom of full terminals** (`terminals.rs:1036-1047`): anchor notices to `.bottom(px(10.))` when they don't fit below `content_rows` (or render as a `flex_shrink_0()` footer).

---

## Wave B — Text/Button Cutoffs, Tooltips & Tree Indentation (`3.1`–`3.5`)

- **3.1 Desk bars & Doc History banner clip escape/action buttons** (`ui/desk/src/desk_bar.rs:52-119, 489-521`, `ui/changes/src/doc_history.rs:475-519`): replace fixed `h(...)` with `min_h(...).flex_wrap()` or `.overflow_x_scroll()`.
- **3.2 Long titles push primary header/row buttons off-screen** (`shell.rs:328-367`, `channel.rs:447-480`, `lists.rs:230-288`, `todo.rs:1194-1244, 1474-1551`, `inbox_page.rs:192-215`, `resume_failed.rs:130-175`): add `.min_w_0().overflow_hidden().text_ellipsis()` on titles, `.flex_shrink_0()` on right-side buttons, `.flex_wrap()` / `.overflow_x_scroll()` on chip/action bars.
- **3.3 Truncated names/paths/errors with no ellipsis or tooltip** (`sidebar.rs:210-326`, `sidebar_pinned.rs:251-349`, `overlay_palette.rs:134-166`, `session_tabs.rs:111-163`, `file_manager.rs:170-299`, `machine_page.rs:987-997`, `access.rs:386-436`): add `.text_ellipsis()` + `full_text(...)` tooltips; remove `.whitespace_nowrap()` (or add horizontal scroll + copy) on `machine_page` script error tails.
- **3.4 Uncapped indentation collapses deep fork/file trees to 0px** (`sidebar.rs:265-278`, `panel_files.rs:311`, `desk_files.rs:220`): clamp visual depth (`depth.min(6)` / `depth.min(8)`) or add `.overflow_x_scroll()`.
- **3.5 Fixed `520px` `#todo-drawer` crushes `#todo-list` + uncapped tab width** (`todo.rs:1164-1178`, `session_tabs.rs:144-164`): add `.max_w(relative(0.55))` on `#todo-drawer` and `.max_w(px(180.)).text_ellipsis()` on tab labels.

---

## Wave C — UX Dead Ends, Double-Click Races & Large-List Virtualization (`4.1`–`4.7`)

- **4.1 Permanently disabled buttons that look enabled or give zero explanation** (`panel_files.rs:633-648`, `panel_info.rs:201-210`, `hosts.rs:239-250`): wire up, hide until implemented, or remove `.opacity(1.)` and add explanatory `.tooltip(...)`.
- **4.2 Inbox filter returning 0 items hides the filter bar** (`inbox_page.rs:184-188`): keep rendering `bar` (or add `"Show all channels"` CTA in `empty()`) whenever `filter != Filter::All`.
- **4.3 Machine Page `"Storage"` tab renders a blank screen** (`machine_page.rs:324-330, 549-571`): hide/disable the tab until implemented or link to Settings › Storage / File Manager Storage.
- **4.4 Trash sheet stuck forever on `"Reading the trash…"` on RPC failure** (`file_manager.rs:907-908`, `features/files/src/manager/trash.rs:20-60`): render error + **Try again** inside `trash_sheet`; disable `files-open-trash` when offline.
- **4.5 Dead-end empty/error states with no Back, Retry, or Clear CTA** (`pages.rs:94-156`, `panel_files.rs:603-615`, `session_tabs.rs:641-647`, `starting.rs:108-122`, `sidebar_pinned.rs:74-88`): add **Go back / Home** on missing session/channel, **Clear filter** on empty file search, and **Retry / Dismiss** on error banners.
- **4.6 Double-click duplicate RPC races** (`file_manager.rs:794`, `settings_providers.rs:375-384`): add in-flight busy flags and `.disabled(busy)` on confirm/delete buttons.
- **4.7 Non-virtualized O(N) rendering freezes UI on large files/repos** (`code_editor.rs:299, 449`, `diff_body.rs:535`, `panel_files.rs:614`): virtualize with `ListState` / `uniform_list`.

---

## Per-Wave Verification (Full Lint, no `--fast`)
- **Wave A:** `tools/bq/bq //apps/humata2/features/app:all //apps/humata2/features/session:all //apps/humata2/ui/...` + targeted unit tests for UTF-8 terminal link slicing, To-do draft isolation, `Escape` ordering, and select-mode focus gating.
- **Wave B:** `tools/bq/bq //apps/humata2/ui/... //apps/humata2/desktop:panels_test //apps/humata2/desktop:native_test` + snapshot verification at narrow widths.
- **Wave C:** `tools/bq/bq //apps/humata2/features/files:all //apps/humata2/ui/... //apps/humata2/desktop:unit_test //apps/humata2/desktop:panels_test //apps/humata2/desktop:native_test` + presubmit gate `apps/humata2/tools/tests/presubmit.sh`.

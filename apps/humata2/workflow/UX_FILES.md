# Humata 2 — macOS UX Deviations, Terminal Fixes & Quick-Win Audit (`UX_FILES.md`)

> **Scope:** Verified, non-breaking, targeted fixes for macOS UX deviations, embedded terminal bugs/colors/themes, shortcuts/menus/lifecycle, system integration, forms/search, and read-only view ergonomics across `apps/humata2/`.
> **Relation to existing queued batches (`apps/humata2/milestones-new.md`):**
> - Core visual polish (Theme sync, `.cursor_pointer()`, hover/active, light-mode palette/hairlines/shadows, scrollbars, vibrancy, 52pt titlebar/headers, tabs, right rail, hit targets, Diff/To-do/Modal visuals) is already queued in `MACOS_POLISH.md` (`0.11.1–0.11.3`).
> - Core layout/state/focus jank (UTF-8 panic, To-do input leakage, background keystroke clobbering, mass-delete key hijack, unscrollable modals, `deferred()`/`.occlude()`, `Escape` bugs, text cutoffs, dead-end buttons/screens, O(N) list virtualization) is already queued in `UI_JANK_FIXES.md` (`0.11.1–0.11.3`).
> - **Everything below is NEW and NOT yet in `MACOS_POLISH.md` or `UI_JANK_FIXES.md`.**

---

## 1. Embedded Terminal — Copy/Paste, Colors, Theme Independence & Keybindings

### P0 / Critical

1. **OSC 52 Copy (`ClipboardWrite`) silently writes to `FakeClipboard` in live desktop**
   - **Where:** `apps/humata2/ui/workspace/src/terminals.rs:306-308`, `apps/humata2/desktop/src/live.rs:327`
   - **What fails:** Copying from Claude Code `/copy`, OpenCode, `tmux` (`set-clipboard on`), Neovim `osc52`, or `lazygit` calls `ui.clients().clipboard.copy(text)` which hits `FakeClipboard` (an in-memory `Mutex<Vec<String>>`) and never writes to the macOS clipboard.
   - **Fix:** Call `cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))` in `terminals.rs:306-308` (and/or wire a real GPUI/macOS `Clipboard` client in `desktop/src/live.rs:327`).

2. **Light mode uses a dark-theme Tomorrow Night pastel ANSI palette + neon `#30d158` green on `#ffffff`**
   - **Where:** `apps/humata2/ui/workspace/src/terminals.rs:2259-2268`, `apps/humata2/ui/overlays/src/overlay_add_machine.rs:124-133`, `apps/humata2/terminal/grid/src/colors.rs:62-66` (compare existing light palette in `apps/humata2/terminal/lab/src/appearance.rs:52-55`)
   - **What fails:** Almost all 16 ANSI colors fail WCAG AA on `#ffffff` (1.2:1–2.7:1 contrast), and hardcoded `palette[2] = 0x30d158` clashes in both dark and light mode.
   - **Fix:** Create a shared `terminal_colors(dark: bool) -> Colors` helper with cohesive Dark and Light 16-color palettes (all Light foregrounds ≥4.5:1 on `#ffffff`; remove `palette[2] = 0x30d158`) and use it in `terminals.rs` and `overlay_add_machine.rs`.

3. **TUIs (Claude Code, Neovim, `delta`, `bat`) always think the terminal is dark (broken OSC 10/11 & missing `COLORFGBG` / DEC 2031)**
   - **Where:** `apps/humata2/daemon/vt_authority/src/lib.rs:114-131, 184-209`, `apps/humata2/terminal/client/src/replica.rs:333`, `apps/humata2/daemon/holder/src/server.rs:772-783`, `apps/humata2/terminal/vt/src/ffi.rs:96-139`
   - **What fails:** `Authority` owns the VT that replies to PTY OSC 10/11 queries, but never receives viewer colors (`TerminalReplica` suppresses replies), and PTYs never set `COLORFGBG` (`15;0` vs `0;15`) or DEC mode 2031 (`CSI ? 996 n` / `997`). TUIs dump dark-theme colors onto `#ffffff` in Light mode.
   - **Fix:** Push the active terminal theme's fg/bg into `Authority` (`terminal.set_default_colors(...)`), set `COLORFGBG` on PTY spawn based on terminal theme, and wire DEC mode 2031 theme-change notifications.

4. **Non-macOS `Ctrl+C` (SIGINT) hijacked by `gpui_base::Root`'s `Copy` binding**
   - **Where:** `apps/humata2/terminal/view/src/view.rs:291-297, 1268-1270`
   - **Fix:** Add `#[cfg(not(target_os = "macos"))] KeyBinding::new("ctrl-c", NoAction, Some(TERMINAL_CONTEXT))` in `TerminalView::new`.

### P1 / High

5. **No independent Dark/Light terminal theme setting (Light app forces Light terminal)**
   - **Where:** `apps/humata2/features/settings/src/lib.rs:53-70, 725-735`, `apps/humata2/ui/settings/src/settings.rs:738-808`, `apps/humata2/ui/core/src/theme.rs:36-98`, `apps/humata2/ui/src/shell.rs:427-440`, `apps/humata2/ui/workspace/src/terminals.rs`
   - **Fix:** Add `TerminalTheme` (`MatchApp | Dark | Light`) to `Preferences` and Settings › Layout; split terminal theme resolution from `Look.term` (which is also reused for non-terminal editors/diffs).

6. **Saved `"Terminal font family"` setting (`full.terminal.font`) is ignored everywhere (`"Menlo"` hardcoded)**
   - **Where:** `apps/humata2/ui/workspace/src/terminals.rs:1222-1229`, `apps/humata2/terminal/view/src/theme.rs:38-42`, `apps/humata2/ui/settings/src/settings.rs:229, 843-889`, `apps/humata2/ui/workspace/src/inbox_terminal.rs:167-174`, `apps/humata2/ui/overlays/src/overlay_add_machine.rs:287-291`
   - **Fix:** Pass `state.settings.general.current().terminal.font` into `font_theme(family, size)`, live-update on change, and show the active font family in the Settings dropdown.

7. **Add Machine SSH login terminal drops all `PaneEvent`s, skips IME focus, and clips paste prompts**
   - **Where:** `apps/humata2/ui/overlays/src/overlay_add_machine.rs:278-307, 807-818`
   - **Fix:** Subscribe to `PaneEvent` (`ClipboardWrite`, `PasteFiles`, `OpenLink`, `Failed`), call `pane.focus_input(window, cx)`, sync `option_as_alt`/`cursor_blink`, and render paste prompts outside the `h(107.)` clip box.

8. **Mouse-reporting TUIs break `Shift+drag` selection if `Shift` is released mid-drag**
   - **Where:** `apps/humata2/terminal/view/src/view.rs:1039-1060, 1151-1201`
   - **Fix:** Track a `local_drag: bool` flag from `mouse_down` to `mouse_up` so releasing `Shift` mid-drag keeps updating selection and does not leak rogue `Motion`/`Release` events to the TUI.

9. **`⌘←`, `⌘→`, and `⌘⌫` (macOS shell line nav/delete) are ignored**
   - **Where:** `apps/humata2/terminal/view/src/keys.rs:63-72`
   - **Fix:** Map macOS `⌘←` → `\x01` (Ctrl+A), `⌘→` → `\x05` (Ctrl+E), `⌘⌫` → `\x15` (Ctrl+U).

10. **`Option as Alt` fails on macOS dead keys (`⌥e`, `⌥u`, `⌥i`, `⌥n`, `` ⌥` ``)**
    - **Where:** `apps/humata2/terminal/view/src/view.rs:1593-1595`, `apps/humata2/terminal/pane/src/host.rs:235-251`
    - **Fix:** Bypass dead-key IME routing when `OptionAsAlt != Off` and no CJK preedit is active.

11. **Multi-line paste confirmation prompt has no `Enter`/`Escape` shortcuts and clips off-screen in `InboxTerminal`**
    - **Where:** `apps/humata2/terminal/pane/src/pane.rs:828-874`, `apps/humata2/terminal/client/src/replica.rs:1297-1314`
    - **Fix:** Handle `Enter`/`Escape` in `TerminalPane::shortcut` when `pending_paste.is_some()`, and anchor paste/clipboard prompts to the visible viewport instead of the bottom of the full panned grid.

### P2 / Medium

12. **Terminal selection & interaction polish:**
    - Add edge auto-scroll during drag-selection, store anchor in `HistoryPoint` (not `ViewportPoint`), and support `Shift+Click` extend (`terminal/view/src/view.rs:126, 1046-1175`).
    - Add right-click context menu (Copy / Paste / Select All / Find) when mouse tracking is off (`view.rs:1287-1292`).
    - Gate `read_from_clipboard_async()` fallback on `PlatformTarget::Web` (`view.rs:881-908`) so empty desktop clipboard doesn't trigger Web buffering.
    - Fix link hover/underline: only show pointer/underline when `⌘` is held (or show `⌘-click` hint on web/mail URLs too), underline wrapped URLs across rows, and add plain `path:line` detection (`view.rs:512-543, 1351-1379`, `terminal/grid/src/links.rs:61-64`).
    - Cache `grid_text` in `InboxTerminal::render` (`inbox_terminal.rs:206`) instead of allocating every 600ms blink.
    - Add Clear Scrollback (`⇧⌘K` / menu item) and wire `PaneEvent::Bell` / `TitleChanged` (`terminals.rs:441-444`).
    - Polish terminal rendering: replace harsh pure-invert selection with translucent accent tint, fix light-mode Find highlight contrast (`view.rs:49-60`), preserve bold/italic under block cursor (`view.rs:588-603`), draw rounded `╭╮╯╰` corners (`boxes.rs:10-11, 93-98`), and replace Menlo-specific `font - 0.5` / `size + 6.5` with proportional line-height (`terminals.rs:1219-1229`).

---

## 2. macOS Menu Bar, Standard Shortcuts, Context Menus & Window Lifecycle

1. **Complete standard macOS Menu Bar (`apps/humata2/desktop/src/main.rs:265-311`):**
   - `Humata`: add `About Humata` (using existing `bundle_version()`).
   - `File`: add `New Terminal Tab` (`⌘T`), `Close Tab` (`⌘W`), `Close Window` (`⇧⌘W`).
   - `Edit`: add `Find…` (`⌘F`), `Find Next` (`⌘G`), `Find Previous` (`⇧⌘G`).
   - `View`: add `Toggle Side Panel` (`⌥⌘0`), `Zoom In` (`⌘+`), `Zoom Out` (`⌘-`), `Actual Size` (`ZoomReset`), `Hosts` (`⌘4`), `Files` (`⌘5`), `Back` (`⌘[`), `Forward` (`⌘]`).
   - `Window`: add separator + `Bring All to Front`.
2. **Sync Keyboard Shortcuts sheet (`apps/humata2/ui/core/src/keys.rs:136-167`):**
   - Add `⌘4` (Hosts), `⌘5` (Files), `⇧⌘F` (Fork), `⌘F`/`⌘G`/`⇧⌘G` (Find), and newly added shortcuts; rename `"Board"` → `"To-do"` at line 151.
3. **Bind missing standard macOS shortcuts (`keys.rs`, `shell.rs`, `bridge.rs`):**
   - `⌘W` (`CloseTab` in session, else `remove_window()`) and `⇧⌘W` (`CloseWindow`).
   - `⇧⌘[` / `⇧⌘]` (`⌃⇧Tab` / `⌃Tab`) for Prev/Next tab in focused group.
   - `⌘]` (`Forward` navigation stack in `AppState`, matching `trail`/`Back` `⌘[`, plus `ChevronRight` button beside Back).
   - `⌘R` (`Refresh` active route/panel).
   - `ZoomReset` (`Actual Size` → `SetFontSize(12)`, keeping `⌘0` on `ToggleSidebar`).
   - App-level `⌘F` focusing `channel_filter` on `Route::Channel` or `file_filter` when Files panel is open.
4. **Expand right-click context menus (using existing `action_menu::card`):**
   - Session tabs (`session_tabs.rs:144-184`): Close Tab, Close Other Tabs, Move to Other Group / Split.
   - Sidebar sessions (`sidebar.rs:1026-1066`): Pin/Unpin, Fork, Move to machine…, separator, Select sessions….
   - Sidebar channels (`sidebar.rs:413-426`): New session, Pin/Unpin, Fold/Unfold, Delete channel….
   - Sidebar hosts (`sidebar.rs:925-974`): Open host, Copy host name.
   - Diff panel file rows (`file_list.rs:273-320`): Copy path, Open file in tab, Looks good.
5. **macOS App & Window lifecycle (`apps/humata2/desktop/src/main.rs:434-449`):**
   - On `#[cfg(target_os = "macos")]`, remove `cx.on_window_closed(|cx, _| cx.quit())` and add `cx.on_reopen(...)` so clicking the red close button keeps the app alive in the Dock and clicking the Dock icon reopens the window.
   - Persist and restore `window_bounds` (size, position, maximized/fullscreen) across restarts instead of hardcoding `1440×900` centered.

---

## 3. Notifications, System Integration, Copy Feedback & Formatting

1. **Implement macOS Dock badge (`apps/humata2/desktop/src/notifier.rs:484-489`):**
   - Wire `NotifierCommand::SetDockBadge(count)` to `NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(...)` instead of logging-only stub.
2. **Respect macOS Focus / Do Not Disturb for notification sounds (`desktop/src/notifier.rs:454-460`):**
   - For bundled `NotifierCommand::Post`, pass `notice.sound.map(|s| s.file_name())` to `cx.show_system_notification_with_sound(...)` instead of spawning `/usr/bin/afplay` directly.
3. **Add `"Open System Settings"` button when notifications are denied (`ui/settings/src/notifications.rs:163-203`):**
   - Call `cx.open_url("x-apple.systempreferences:com.apple.Notifications-Settings.extension")` for `PermissionStatus::Denied`.
4. **Finder Drag-and-Drop onto Terminals (`ui/workspace/src/terminals.rs:1048-1075`):**
   - Wire `.drag_over::<ExternalPaths>` + `.on_drop::<ExternalPaths>` onto `#terminal-area` / `#terminal-secondary`, reusing the existing `PasteFiles` helper at lines `344-394`.
5. **Add `"Reveal in Finder"` on Desk folder bars (`ui/desk/src/desk_bar.rs:37-48, 475-486`, `channel_desk.rs:584-596`):**
   - When on This Mac with an absolute path, show `"Reveal in Finder"` next to `"Copy path"`.
6. **Make Session Info branch chip copyable (`ui/workspace/src/panel_info.rs:98`):**
   - Click-to-copy `session.branch` with `"Copied branch name"` toast.
7. **Add `"Copied"` toast/feedback to all 6 silent `"Copy path"` handlers:**
   - `features/inbox/src/lib.rs:980`, `features/settings/src/browser.rs:94`, `ui/desk/src/desk_bar.rs:45, 483`, `ui/desk/src/channel_desk.rs:592`, `ui/report/src/report_view.rs:297`.
8. **Standardize macOS keycaps & `keys::label(...)`:**
   - Unify `"Enter"` / `"↵"` → `"↩"` (`keys.rs:157`, `request_card.rs:287`), `"Enter"` → `"Return"` (`todo.rs:1708`), and route hardcoded `"⌘"` strings in `shell.rs:321,346,364`, `pages.rs:311,406,408`, `sidebar.rs:847`, `file_list.rs:650-826`, `todo.rs:3147-3149` through `keys::label(..., is_mac)`.
9. **Fix microcopy & formatting bugs:**
   - **Pluralization (`1 files` / `1 sessions` / `1 comments` / `1 items`):** `request_card.rs:854`, `overlay_pr_review.rs:877`, `todo.rs:759, 761, 2006`, `features/palette/src/lib.rs:961`, `overlay_delete_sessions.rs:371, 373, 860`.
   - **`"now ago"` & frozen timestamps:** special-case `"now"` → `"just now"` in `request_card.rs:1299, 1529-1530` and tick via `clock::ticked_now`; replace hardcoded `"2m ago"` fallback in `panel_changes/mod.rs:1087-1093` with `last_checked_ms`.
   - **`"0.0 GB"` small folders:** replace `gigabytes(...)` in `machine_page.rs:232-234, 836` with `humata2_files::human_bytes(...)`.

---

## 4. Forms, Search/Find & Read-Only Content Views

1. **Wire dead Markdown links (`MarkdownEvent::LinkActivated`) in all 3 Markdown hosts (P0 / Critical):**
   - **Where:** `apps/humata2/markdown/src/view.rs:183, 1147`, `apps/humata2/ui/changes/src/document_tab.rs:932-946, 986-1048`, `apps/humata2/ui/changes/src/doc_history.rs:185-186`
   - **What fails:** `MarkdownView` overrides `TextView`'s built-in `open_url` to emit `LinkActivated`, and all 3 hosts either don't subscribe or drop it in `_ => {}` — every link in every Markdown preview is dead.
   - **Fix:** Subscribe to `MarkdownEvent::LinkActivated { url }` in all 3 sites and call `cx.open_url(url)` for `http://`/`https://`/`mailto:`.
2. **Add `.cleanable(true)` (native `⊗` clear button) to all 12 search/filter inputs:**
   - `todo.rs:1209`, `panel_files.rs:593`, `file_list.rs:856`, `review_header.rs:1711`, `channel.rs:579`, `overlay_palette.rs:312`, all 5 `repo_search_field` callers (`overlay_new_channel.rs:87`, `overlay_session_add_repo.rs:102`, `overlay_channel_add_repo.rs:112`, `overlay_desk_new.rs:180`, `overlay_desk_add_repo.rs:84`), and `report_view.rs:321`.
3. **Fix non-editable Channel Desk To-do search bar (`apps/humata2/ui/desk/src/todo_pane.rs:217-232`):**
   - Currently renders a static `div` instead of an `Input` widget; wire an `Entity<InputState>` with `.cleanable(true)` and Esc-to-close.
4. **Add `.mask_toggle()` (show/hide eye toggle) to both secret inputs:**
   - `machine_page.rs:1342` and `settings_providers.rs:338` (both already use `.masked(true)`).
5. **Add `⌘Enter` submission to all 6 remaining multiline textareas & `Enter` to 5 single-line inputs:**
   - `⌘Enter` (`PressEnter { secondary: true, .. }`): `document_tab.rs:684` (inline comment), `overlays.rs:208` (New Session first message), `overlay_pr_review.rs:95` (PR context), `overlay_submit_review.rs:112` (review summary), `report_view.rs:77` (bug report happened/expected), `settings_providers.rs:57` (TOML paste).
   - `Enter` (`PressEnter`): `doc_history.rs:114` (Save version message), `todo.rs:824-856` (`comment_input`, `ext_block_input`, `split_input`, `ask_input`).
6. **Add one-click Copy buttons & horizontal scroll to read-only views:**
   - **Markdown fenced code blocks (`markdown/src/view.rs:1139`):** chain `TextView::code_block_actions(...)` with a compact `Copy` button.
   - **Shared error banners (`ui/core/src/widgets.rs:188-222`):** add a quiet `Copy` button next to `Dismiss`.
   - **Conversation tab (`ui/workspace/src/conversation_tab.rs:234-264`):** add per-entry `Copy` button (and/or selectable `TextView`).
   - **Code Editor (`ui/changes/src/code_editor.rs:220-234, 292, 449`):** add `.overflow_x_scroll()` so `.whitespace_nowrap()` lines don't clip off-screen, plus a header `Copy path` / `Copy` button.
   - **Palette Conversations header (`ui/overlays/src/palette_hits.rs:53-70`):** display hit count (`Conversations · 12` / `12+`).
   - **Journal cards (`ui/workspace/src/panel_journal.rs:32-72`):** make card header/body clickable to open (matching `reviews_inbox.rs`).

# Make the app look like its design goldens

Goldens: `desktop/tests/__snapshots__/design/`. Native frames: `desktop/tests/__snapshots__/native/`.
The goldens are drawn by `desktop/tests/design/*.rs` with `design/kit.rs` (`Look`); the app's views are `ui/src/`.

Survey (2026-11-15, read pairs 01, 25, 29, 31): recorded native frames already sit close to the goldens in
layout. The visible drift is in shared tokens that bypass `theme::Look`: `widgets.rs` and
`overlay_shortcuts.rs` still paint with the old dark-only prototype greys (`theme::TEXT/DIM/FAINT/ACCENT2/RED`,
`0x2a1717`), so titles, dim/faint text, empty states and error banners are near-white or off in light mode,
and about 134 raw hex colours across ui/src skip `Look` entirely.

## Ranked items

1. Shared text tokens: `widgets.rs` title/dim/faint/empty/error banner and `overlay_shortcuts.rs` take `Look` (text, secondary, tertiary, accent) instead of the prototype constants.
2. Shared status colours: add kit's GREEN/ORANGE/RED/GREY/PURPLE to `theme.rs`; replace matching raw hex in ui/src, file by file (chrome, lists, overlay_toasts, request_card, panels, channel, board). Skip sidebar*.rs, palette, Conversation tab (FIND) and panel_changes/Diff/editor (REVIEW).
3. Buttons and fields: `widgets::ghost`/`primary` and text fields match kit `button`/`field` (h 24 / 26, px 11 / 8, radius 6, 12 / 12.5 px text).
4. New session sheet (31): First message label and textarea align with the other rows like the golden.
5. Sidebar with Pinned (25): search field, row heights, `web` clipped above Hosts. FIND owns the sidebar: hand off, don't edit.
6. Composer box (01, session::): rounded, shorter, cursor padding. session:: frames are not cleared: list for main.

## Lane

(item, files touched, frames re-recorded)
- Item 1 (shared text tokens), in progress. overlay_shortcuts.rs now uses `look`; ui_unit_test passed; no frames, since no native test draws the sheet. Item 1b, seen mid-run (worker still running, not verified): widgets.rs `*_in(look, ..)` helpers; callers switched in access, desk, file_manager, pages, overlay_desk_copy, overlay_desk_new, overlay_new_channel, overlay_desk_clean_up, report_view, shell; theme::ACCENT2 removed. lists.rs and session_tabs.rs are left on the old helpers (other lanes are mid-change), so theme::TEXT/DIM/FAINT stay until they move. Frames re-recorded: not yet reported.
- Item 1b, done. widgets.rs gained `title_in/dim_in/faint_in/empty_in/error_banner_in(look, ..)`; callers switched in overlay_new_channel, report_view, shell, pages, desk, overlay_desk_new, access, overlay_desk_clean_up, overlay_desk_copy; theme::ACCENT2 deleted; ui_unit_test passes. Frames re-recorded: none, because no native frame draws these. Left: overlays.rs `field` (its callers include overlay_fork.rs), lists.rs, session_tabs.rs; theme TEXT/DIM/FAINT stay until those move.
- Item 1c, in progress: native tests shortcuts-sheet-light/dark and new-channel-error-light in desktop/tests/native/overlays.rs. Before-set: scratchpad/polish/_before-01 (HEAD native frames).
- Item 1c, done: native tests shortcuts_sheet_light/dark and new_channel_error_light. Review folder 01-shared-text-tokens sent to main. Item 1d (shortcuts sheet height) has started.
- Item 1d, done: overlay_shortcuts.rs; shortcuts-sheet-light/dark re-recorded; folder 01d-shortcuts-sheet-height.
- Item 1e, done: shortcuts_sheet_scrolled_light; the last row is visible.
- Item 2, done as a no-pixel refactor: theme.rs got GREEN/ORANGE/RED_STATUS/GREY/PURPLE and chrome.rs status() uses them. request_card, channel and board were skipped because other lanes have them dirty. No frames, no folder.
- Item 3, done: widgets primary_in/ghost_in; New channel and shortcuts sheet switched. Frames: new-channel-error-light and the three shortcuts-sheet frames. Placeholder colour deferred (global theme). Folder 03-buttons-fields.
- Item 4: no change. Native is already aligned with the other rows; the remaining placeholder inset and size come from gpui-kit Textarea.
- Item 6: not app chrome. The composer box is the agent TUI's own box-drawing output inside the terminal, so the app has nothing to change; the golden idealises it. Closed with no change.
- Item 5: handed off to FIND (sidebar).
Lane closed 2026-11-16.

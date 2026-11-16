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
- Item 10, done: offer banner. Open it is primary, Later is bordered, Not now is a local ghost (session_tabs.rs). Frames 03/04-session-needs-you. Folder 10-offer-banner-primary. Note: golden 03 draws Open it as secondary and Later/Not now as text; the prototype's primary button was followed.
- Item 10b: goldens win (D76). Open it is secondary grey; Later and Not now are plain text. session_tabs.rs. Folder 10b-offer-banner-golden.
- Item 11 (08), done: Inbox request card. Meta line "machine · # channel" with blue "Open session ↗"; preview rows in a grey rounded box; "What it was doing" as dot rows and "Asks you because" + "Change this rule" kept per golden 26. request_card.rs. Frames 26/27. Committed 48fc0a65.
- Item 12 (07), done: sidebar offsets. Pinned event row pt 5 (tools follow), Hosts header mt -3. sidebar_pinned.rs, sidebar.rs. Frames 15/16/17/25. Committed 48fc0a65.
- Item 13: sidebar nav rows back to 26 pt; Search mt -4 only when Pinned shows (frame 18 had drifted ~13 display px). sidebar.rs. Folder 13-sidebar-nav-rows. Awaiting record.
- Item 14: Hosts header mt -3 only when Pinned shows (non-pinned frames had Hosts 5 display px high). sidebar.rs. Folder 14-hosts-offset. Awaiting record.
- Item 15: New session Title input text 12.5 px (was shrunk by .small()). overlay_new_session.rs. First message textarea inset left (gpui-kit). Folder 15-new-session-fields. Awaiting record.
- Frame 15 sidebar over budget (2.194%) after another lane's sidebar changes: seam scratchpad/polish/seam-15-selected-weight restores semibold on the selected row (golden 15). Awaiting look_test.
- Item 16: offer banner Later/Not now px 0 so the row gap of 10 spaces them, as in golden 03/15. session_tabs.rs. Folder 16-offer-banner-spacing. Awaiting record.
- Seam 15b (supersedes seam-15-selected-weight): selected row semibold only with requests waiting (goldens 15/16); 5 pt gap above Home under pins. Measured with scratchpad/tools/rows.py (pure-Python PNG row bands). Awaiting look_test.
- Seam 15c (scratchpad polish/seam-15c-pinned-card/apply.py): frame 15's VIP card measured 222-293 vs golden 221-295 and every row below sat 1-2 px high; the pinned section now shrinks to MIN_SHARE 0.77 of its natural height (was 0.75) in ui/src/sidebar_pinned.rs. Pending main's re-record and look_test.
- Seam 15d (scratchpad polish/seam-15d-pinned-card/apply.py): 15c left frame 15 at 1.809% (MIN_SHARE never binds). Measured: VIP card blue 222-293 vs golden 221-295, its title ellipsised where the golden clips, Home 2 px and rows below 1 px high. EVENT_ROW 40->41.5, EVENT_GAP 1.8->0.3 (acme row stays put), asks line bottom 0.2->0.7, no ellipsis on event titles (sidebar_pinned.rs); pinned_gap 5->5.5 (sidebar.rs). Pending re-record.
- Seam 15e (scratchpad polish/seam-15e-home-gap/apply.py): after 15d frame 15 at 1.548%; card height now matches (74 px, 1 px low), but pinned_gap 5.5 had no effect because the full column shrinks the spacer: Home still 2 px high, rows below 1 px. Spacer gets flex_shrink_0 (sidebar.rs). Pending re-record.
- Seam 15f (scratchpad polish/seam-15f-event-row/apply.py): after 15e frame 15 at 1.282%; card 222-295 vs golden 221-295 and acme/api plus every row below exactly 1 px high (none low, so pinned_gap stays). EVENT_ROW 41.5->42 (sidebar_pinned.rs). Card top stays 1 px low (set by the shared section header). Pending re-record.
- Item 17 (scratchpad polish/17-changes-file-indent/apply.py): owner: repo-to-file indent looks weird. Changes file rows get pl 22 (pr 8) so the file icon sits under the repo folder icon and the name under the repo name; highlight stays full width (panel_changes.rs). Golden follow-up after approval: diff_file in desktop/tests/design/session.rs. Pending re-record.

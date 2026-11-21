# MACOS_POLISH — macOS Native Visual & Interaction Polish

## Goal & Constraints
Close the visual and interaction gap between the design renders (`apps/humata2/workflow/milestones/mockups/` and `renders/`) and the actual GPUI desktop app (`apps/humata2/desktop/tests/__snapshots__/native/`), so Humata 2 looks and feels like a native macOS app rather than a custom Rust GUI.

- **No large rewrites** — keep changes surgical and concentrated in shared UI primitives (`apps/humata2/ui/core/`), desktop window setup (`apps/humata2/desktop/src/`), and targeted per-screen polish.
- Feeling strongly macOS-native on Linux too is explicitly acceptable.
- Follow `AGENTS.md`: build/test only via `tools/bq/bq` (never `cargo`, `bazel clean`, or `bazel shutdown`), keep crate/visibility boundaries intact (`ui/*` depends on `//apps/humata2/clients:api`), and update/verify desktop native snapshots as part of each wave's exit criteria.

---

## Wave 1 — Global Primitives, Theme Sync, Cursors & Scrollbars

### Scope
1. **Sync `Look` into `gpui-kit` `Theme`** (`apps/humata2/ui/core/src/lib.rs`, `apps/humata2/ui/src/shell.rs`, `apps/humata2/ui/src/starting.rs`):
   - On init and appearance change, sync `Look::of(cx)` into `Theme::update(cx, ...)` (`primary`, `primary_hover`, `primary_active`, `secondary`, `secondary_hover`, `secondary_active`, `border`, `input`, `popover`, `foreground`, `muted_foreground`, and `ring = accent @ 45% alpha`).
   - Eliminates blue primary buttons flashing to shadcn monochrome on hover and turns grey focus rings into macOS translucent accent rings.
2. **Fix missing pointing-hand cursors & hover/active states** (`apps/humata2/ui/core/src/widgets.rs`, `chrome.rs`, `action_menu.rs`, `apps/humata2/ui/workspace/src/sidebar.rs`, `session_tabs.rs`, `apps/humata2/ui/src/panels.rs`, `shell.rs`):
   - Add `.cursor_pointer()` to shared builders (`widgets::push`, `widgets::send`, `chrome::panel_action`, `chrome::icon_action`, `action_menu::item`, `panels::strip`) because `gpui-component`'s `Button::render` forces `.cursor_default()`.
   - Add `.cursor_pointer()`, subtle `.hover()` fill, and distinct `.active()` press fill to sidebar nav/channel/session/host/desk rows, sidebar search, fold chevrons, session tabs, `chrome::action`, and `panel-restore-strip`.
3. **Light-mode palette, crisp secondary buttons & Retina hairlines** (`apps/humata2/ui/core/src/theme.rs`, `widgets.rs`, `chrome.rs`, `sidebar.rs`):
   - Lighten light-mode `Look`: `sidebar: 0xf3f3f6`, `fill: 0xf3f3f5`, `line: 0xe5e5ea`, plus a `control_border: 0xd4d4da` token. Give sidebar Search a recessed fill (`rgba(0x0000000e)` light / `rgba(0xffffff12)` dark).
   - Switch `widgets::push_quiet_shape` in light mode from flat grey `look.fill` to crisp `look.card` (`#ffffff`) + border + subtle `shadow_xs()` (using `look.fill` on `:hover`), height `26–28px` (`rounded(7)`), keeping `24px` only for dense inline controls.
   - Add `Look::hairline()` (`rgba(0x0000001a)` light / `rgba(0xffffff16)` dark) and adopt `0.5pt` hairlines for pane dividers, toolbar borders, and card/row separators instead of opaque `1.0pt` borders.
   - Add 3 reusable layered `BoxShadow` helpers in `theme.rs` (`shadow_control`, `shadow_popover`, `shadow_sheet`).
4. **Native overlay scrollbars & rubber-band bounce** (across `apps/humata2/ui/`):
   - Replace raw `.overflow_y_scroll()` calls (starting with `sidebar.rs`, `todo.rs`, `settings.rs`, `overlay_palette.rs`, `panel_changes/`, `channel.rs`, `lists.rs`, `document_tab.rs`, `conversation_tab.rs`) with `gpui-kit`'s `.overflow_y_scrollbar()` / `.vertical_scrollbar(&handle)`.

### Wave 1 Verification (Full Lint, no `--fast`)
- Unit tests: `tools/bq/bq //apps/humata2/ui/core:all //apps/humata2/ui/workspace:all //apps/humata2/ui/changes:all //apps/humata2/ui/overlays:all //apps/humata2/ui/settings:all //apps/humata2/ui:all`
- Snapshot update & visual check: re-record changed desktop snapshots via `bazel run //apps/humata2/desktop:panels_test_record` and `bazel run //apps/humata2/desktop:native_test_record`, then verify `tools/bq/bq //apps/humata2/desktop:panels_test //apps/humata2/desktop:native_test` and inspect rendered PNGs against `mockups/` / `renders/` (light + dark).

---

## Wave 2 — macOS Window Chrome, Vibrancy & Layout Alignment

### Scope
1. **Window vibrancy & macOS `WindowOptions` polish** (`apps/humata2/desktop/src/main.rs`, `starting.rs`, `apps/humata2/ui/src/shell.rs`, `sidebar.rs`, `panels.rs`):
   - Enable `window_background: WindowBackgroundAppearance::Blurred`, `use_display_p3: true`, `app_owns_titlebar_drag: true`, `window_min_size: Some(size(px(640.), px(480.)))`, and `traffic_light_position: Some(point(px(20.), px(18.)))` (centered in 52pt bar).
   - Remove opaque `.bg()` on outer `#shell` (keep on `#main`), and use semi-transparent sidebar/rail tints so `NSVisualEffectView` blur shows through.
   - Lock `starting.rs` splash window (`is_resizable: false`, transparent titlebar, `.font_family(".SystemUIFont")`); add `About Humata` and `Close Window (⌘W)` to menus.
2. **Unified 52pt titlebar drag, double-click zoom & traffic-light clearance** (`chrome.rs`, `shell.rs`, `pages.rs`, `lists.rs`, `settings.rs`, `hosts.rs`, `channel.rs`):
   - Add a shared `chrome::titlebar_drag()` helper (`start_window_move` + double-click `zoom_window`) and apply it to all 52pt toolbars.
   - Insert `chrome::lights()` spacer (with matching `pl(px(20.))`) in `top_bar`, `settings::toolbar`, `hosts::toolbar`, and `channel::header` whenever the sidebar is collapsed so traffic lights never overlap buttons/titles.
3. **Inactive-window selection dimming** (`shell.rs`, `sidebar.rs`):
   - When `!window.is_window_active()`, render selected sidebar rows with neutral `look.fill` + `look.text` instead of bright blue `look.selection`.
4. **Chrome alignment, truncation, tabs & right rail**:
   - **Mid-character clipping:** Add `.text_ellipsis()` to all sidebar channel/session/pinned rows (`sidebar.rs`, `sidebar_pinned.rs` — including `event` rows) and Kanban card labels.
   - **52px vs 36px header stair-step:** Match non-fullscreen `chrome::header_with` (`chrome.rs`) height to `52px` (`theme::TOP_BAR`) so top toolbar hairlines align continuously across panes.
   - **Session tabs (`session_tabs.rs`):** Bump tab bar to `36px`, add `FontWeight::MEDIUM` and a `2px` bottom accent indicator on the active tab so it isn't sliced off by the bottom border.
   - **Right icon rail (`panels.rs`):** Replace hardcoded `#e9e7ea`/`#2a2a2d` with `look.panel`/translucent tint, shorten `"Agent inbox"` → `"Inbox"` so all tiles share uniform `48px` height, give active tile visible contrast + blue icon/label, and tighten badges (`h(14)`).
   - **Typography & icons (`theme.rs`, `pages.rs`, `chrome.rs`, `ui/core/src/lib.rs`):** Snap fractional sizes (`9.5/10.5/11.5/12.5`) to integer HIG steps (`10/11/12/13/15`), bump `session-title` to `15px`, prefer `"SF Mono"`, add `tnum` on counters/timers, and remove duplicate `SquareTerminal` in `icon_assets!`.

### Wave 2 Verification (Full Lint, no `--fast`)
- Unit & desktop tests: `tools/bq/bq //apps/humata2/ui/... //apps/humata2/desktop:unit_test //apps/humata2/desktop:panels_test //apps/humata2/desktop:native_test`
- Snapshot review: re-record affected native snapshots (sidebar collapsed + expanded, right rail, session tabs, toolbar alignment) and verify traffic-light clearance, hairline continuity across 52pt headers, and inactive-window dimming.

---

## Wave 3 — Hit Targets, Drag/Motion & Per-Screen Component Polish

### Scope
1. **Hit targets, resize drag & micro-animations**:
   - Pad `12–16px` chrome icons (`sidebar-fold`, `+`, tab close `X`, `chrome::action`, `chrome::icon_action`) to `22–26px` rounded hover wells.
   - Lock `cursor_col_resize()` on `#shell` and keep the splitter grip visible while `self.dragging.is_some()` (`shell.rs`, `panel_changes/mod.rs`, `agent_tab.rs`); theme `TabDrag` via `Look::of(cx)`.
   - Add `100–140ms` `ease_out_quint` fade/slide entrance animations to `overlays.rs`, `overlay_toasts.rs`, and `action_menu.rs`.
   - Dismiss `action_menu::scrim` on `on_mouse_down` and switch `file_manager.rs` row clicks to `on_click`.
2. **Menus (`machine_page.rs`, `action_menu.rs`)**:
   - Replace stacked `widgets::ghost_in` buttons in `machine_page.rs` with `action_menu::item`.
   - Add `action_menu::separator()` + destructive red styling and concentric corners (`rounded(9)` outer, `p(4)`, `rounded(5)` items, `h(24)`).
3. **Diff panel & comment cards (`diff_body.rs`, `comment_cards.rs`, `code_wrap.rs`)**:
   - Break wrapped code lines at whitespace/punctuation before falling back to char splits; color `+`/`-` signs with `GREEN`/`RED`; remove duplicate `↗` in narrow file bars so filenames don't clip; align comment cards to `code_start()` and move secondary actions (`Edit`, `Drop`) to muted top-right header.
4. **To-do list (`todo.rs`)**:
   - Replace `.rounded(9).border_t_1()` curved row borders with flat inset hairlines only *between* rows (`i > 0`).
   - Render Sort/Filter popovers as floating `deferred()` overlays with `shadow_popover` instead of pushing the list down inline.
   - Put `S`/`M`/`L` effort in `18×18` rounded-5 sunken badges, add faint checkmark inside open circles, tint `"Pinned"` pills blue, dim channel chip counts (`opacity: 0.65`), and use `.border_dashed()` on `Show closed`.
5. **Modals, Palette, Settings & Channel trees (`overlay_new_session.rs`, `overlay_palette.rs`, `settings.rs`, `channel.rs`, `sidebar_pinned.rs`)**:
   - Top-align multi-line form labels (`items_start` + `pt(4)`), center `"11 GB free"` with picker, wrap/shorten clipped textarea placeholder, unify sheet padding/radius (`p(22)`, `rounded(12)`).
   - Clamp Command Palette list height so the hint footer never slices a row in half.
   - Give active segmented control pills `SEMIBOLD` + border + `shadow_xs()`; split `Settings > Agents` into separate `12px`-gapped cards; replace disconnected Unicode `├`/`│`/`└` tree glyphs in `channel.rs` with clean indentation + `GitFork` icon; center-align the status dot in `sidebar_pinned.rs`.

### Wave 3 Verification (Full Lint, no `--fast`)
- Unit & UI tests: `tools/bq/bq //apps/humata2/ui/... //apps/humata2/desktop:unit_test //apps/humata2/desktop:panels_test //apps/humata2/desktop:native_test //apps/humata2/desktop:design_test`
- Snapshot review: re-record changed native/panel/design snapshots in one batch and judge against `mockups/` / `renders/` (menus, diff/comment cards, To-do list + floating popovers, New Session modal, Command Palette, Settings › Agents, Channel tree).
- Final gate: `apps/humata2/tools/tests/presubmit.sh` green with zero lint warnings.

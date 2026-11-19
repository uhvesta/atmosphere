# SPLIT Phase 3 (`ui`) & Remaining Daemon Services: Detailed Split Plan

- **Parent docs:** [`SPLIT.md`](./SPLIT.md) (§3, §4, §6.2), [`SPLIT_PHASE4_5.md`](./SPLIT_PHASE4_5.md)
- **Status:** verified design blueprint (read-only static audit + independent Gemini 4 Neon High verification)
- **Scope:**
  - **Part 1 (Phase 3):** `apps/humata2/ui` (87 `.rs` files, 44,242 LOC; `panel_changes.rs` is 4,005 LOC) and `apps/humata2/desktop/tests`
  - **Part 2 (Phase 4 daemon):** Remaining 11 `daemon/humatad` service crates (`svc_inbox`, `svc_terminal`, `svc_transfers`, `svc_away`, `svc_peer`, `svc_machines`, `svc_lifecycle`, `svc_channels`, `svc_sessions_{local,transfer,archive}`) across 254 `src/` files (148,132 LOC, 82 top-level modules) and all **216** `daemon/humatad/tests/*.rs` targets
  - **Part 3:** One unified, always-green move order across both

> [!IMPORTANT]
> Static inspection and Neon High verification surfaced five build-critical constraints that refine `SPLIT.md`:
> 1. **`ui/src/panels.rs` (`L32–L43`) and `ui/src/overlays.rs` (`L22–L25`) both use `#[path = "..."]` submodules:**
>    - `panel_{changes,board,files,inbox,info,journal}.rs` inherit 16 helper primitives from `panels.rs` (`panels.rs:28–30, 52–339`), which must move to `ui/core::chrome`.
>    - `overlays.rs` owns `overlay_add_machine.rs` (which calls `super::{sheet, sync}` and links `ghostty_vt`, `terminal_pane`, `terminal_view`) and all 7 `overlay_desk*.rs` files (all 7 contain inherent `impl Overlays` blocks!). Keeping `overlay_desk*.rs` and `overlay_add_machine.rs` in `ui/overlays` avoids `E0116` orphan-rule errors and `ui/overlays <-> ui/machines` cycles while keeping `ui/desk` and `ui/machines` 100% leaf crates.
> 2. **`ui/src/bridge.rs` reverse cycles:** `bridge.rs:23` imports `InboxRoom` (plus `Measured`, `DETAIL_PAD`, `DETAIL_GAP`, ~68 LOC from `inbox_terminal.rs`), and `bridge.rs:24,56,270` exposes `pub const fn recorder(&self) -> &Recorder` (called ~15× by `report_view.rs` and `desktop/tests/report.rs`). Decoupling requires passing `Recorder` directly to `ReportView`/`Shell`/tests and using a pre/post-reduce `ActionRecorder` hook (`before_reduce(&AppAction) -> Option<String>`, `after_reduce(&str, &AppState)`) in `ui/core::bridge`.
> 3. **`desktop:look_test` golden constraint:** 23 of the 45 `desktop/tests/native/*.rs` files record full 1280×760 `Shell` PNGs that `look_test` diffs against `design/*.png`. Only single-crate behavioral/component tests move to `ui/*`; full-window golden-recording frames and multi-crate flows stay in `desktop`.
> 4. **Daemon Step 6/7 ordering, `#[cfg(feature = "admin")]`, and `impl SessionSvc`:** Pure zero-service helpers move to `rpc_kit`/`runtime` first, while `answer_gated*`/`answer_plan`, `start_message`, and `start_ready` must wait until `InboxPort` & `SessionPort` exist. Also, 8 library files across 6 crates use `#[cfg(feature = "admin")]` (converting these to runtime config eliminates `_admin` library twins), and 17 files in `svc_sessions_{transfer,archive}` write inherent `impl SessionSvc` blocks (requiring tonic dispatch via `SessionTransferPort`/`SessionArchivePort` + a shared `svc_sessions_common` crate).
> 5. **`remote_rig.rs:199` (`HUMATAD2_RELEASE`):** All 69 `RIG_TESTS` spawn `HUMATAD2_RELEASE` for the fake box daemon. Scoped Tier B profiles therefore need **scoped `*_release` twins** (e.g. `humatad2_test_power_cut_release`), otherwise editing `svc_review`/`svc_board` still invalidates all 69 rig tests via monolithic `:humatad2`.

---

## Part 1 — Phase 3: Splitting `apps/humata2/ui`

### 1.1 Step 0: Splitting `ui/src/panel_changes.rs` (4,005 lines) into submodules

Split `apps/humata2/ui/src/panel_changes.rs` into `apps/humata2/ui/src/panel_changes/` **first**. Note: `apps/humata2/ui/BUILD.bazel:31` currently uses non-recursive `srcs = glob(["src/*.rs"])` — it **must** change to `glob(["src/**/*.rs"])` in this step.

| Submodule | Approx. LOC | Exact functions, types, statics, and line ranges from `ui/src/panel_changes.rs` (all 86 top-level items) |
|---|---:|---|
| `panel_changes/mod.rs` | ~905 | `struct Quiet` (`L22–L26`), `impl RenderOnce for Quiet` (`L28–L38`), `pub(super) fn changes` (`L794–L1567` — top-level orchestrator) |
| `panel_changes/file_list.rs` | ~554 | `fn repo_label` (`L41–L43`), `fn split_path` (`L47–L52`), `fn on_path` (`L56–L62`), `pub(super) fn file_comments` (`L67–L82`), `fn is_code` (`L86–L88`), `fn mode_changed` (`L91–L93`), `fn mode_words` (`L96–L103`), `fn file_note` (`L108–L141`), `fn file_row` (`L144–L316`), `fn list_rows` (`L321–L344`), `fn at_repo` (`L347–L349`), `fn at_file` (`L352–L357`), `fn ringed` (`L361–L381`), `static STEPPED` (`L527`), `fn change_moved` (`L532–L538`), `fn put_at_top` (`L541–L552`), `fn step_button` (`L555–L580`), `fn open_in_editor` (`L583–L595`), `fn wrap_switch` (`L599–L617`), `fn change_stepper` (`L620–L649`), `fn file_bar` (`L657–L760`), `fn file_filter_field` (`L764–L792`) |
| `panel_changes/plan.rs` | ~155 | `fn plan_note_label` (`L385–L391`), `fn plan_list` (`L396–L522`), `fn plan_repo_line` (`L2601–L2618`) |
| `panel_changes/review_header.rs` | ~1,135 | `fn drafts_line` (`L1571–L1593`), `pub(super) fn github_draft_counts` (`L1596–L1603`), `fn review_pulls` (`L1606–L1613`), `fn review_line` (`L1617–L1646`), `static REVEALED` (`L1651`), `fn reveal_focus` (`L1656–L1684`), `fn review_repo` (`L1690–L1778`), `static PINNED` (`L1783`), `fn pinned_shift` (`L1789–L1795`), `fn note_pinned` (`L1799–L1813`), `fn review_code` (`L1818–L1929`), `fn review_repo_head` (`L1933–L1982`), `fn picked_base_name` (`L1987–L2004`), `enum StackScope` (`L2008–L2011`), `impl StackScope` (`L2013–L2029`), `fn stack_scope` (`L2034–L2050`), `fn base_scrim` (`L2241–L2252`), `fn push_scrim` (`L2256–L2268`), `fn push_menu` (`L2272–L2308`), `fn base_row` (`L2312–L2342`), `fn base_menu` (`L2347–L2484`), `fn repo_head` (`L2491–L2597`), `fn repo_base` (`L2621–L2716`), `fn repo_base_scrim` (`L2720–L2732`), `fn repo_base_menu` (`L2737–L2919`), `fn push_control` (`L2924–L2995`) |
| `panel_changes/states.rs` | ~180 | `fn nothing_against` (`L2052–L2083`), `fn read_failed_block` (`L2087–L2162`), `fn repo_error_row` (`L2166–L2205`), `fn refresh_failed_row` (`L2209–L2237`) |
| `panel_changes/diff_body.rs` | ~927 | `fn sent_card` (`L2997–L3007`), `struct Metrics` (`L3011–L3026`), `impl Metrics` (`L3028–L3033`), `fn code_start_of` (`L3038–L3042`), `fn reviewing` (`L3048–L3053`), `fn span` (`L3058–L3073`), `fn hunk_lines` (`L3076–L3083`), `fn ask_over` (`L3088–L3129`), `fn side_of` (`L3131–L3137`), `fn at` (`L3141–L3146`), `fn under` (`L3150–L3155`), `fn exactly_on` (`L3158–L3162`), `struct Target` (`L3166–L3173`), `fn file_lines` (`L3176–L3185`), `impl Target` (`L3187–L3259`), `const VIRTUAL_FROM` (`L3265`), `struct CodeRows` (`L3271–L3293`), `impl CodeRows` (`L3295–L3709`), `fn blend` (`L3712–L3719`), `fn tint` (`L3723–L3731`), `fn hunk_section` (`L3735–L3740`), `fn run_of` (`L3743–L3756`), `fn paired` (`L3761–L3786`), `fn changed_span` (`L3793–L3821`), `fn drawn` (`L3825–L3838`), `fn display_range` (`L3844–L3868`), `fn line_facts` (`L3872–L3888`), **`mod tests` (`L3891–L4005`)** (tests private `diff_body.rs` helpers `changed_span`, `paired`, `hunk_section`, `display_range`) |

- **Public surface from `panel_changes`:** `pub fn changes(...)` (once extracted to `ui/changes`); `file_comments` and `github_draft_counts` have zero callers outside `panel_changes.rs` and stay `pub(super)`.

### 1.2 Shared pieces that must go in `ui/core`

| Module / item in `ui/core` | Callers in `ui/src` | Why it must live in `ui/core` |
|---|---:|---|
| `theme.rs` (99 LOC) | 73 | `Look` palette and theme tokens used across nearly every view. |
| `bridge.rs` (288 LOC) | 68 | `Ui`, `Part`, `Logged`, store observers. **Must also absorb:** (a) `InboxRoom`, `Measured`, `DETAIL_PAD`, `DETAIL_GAP` (`inbox_terminal.rs:37–105`, ~68 LOC), (b) `FocusTarget` + `focus_target` (`terminals.rs`), (c) a pre/post-reduce `pub trait ActionRecorder: Send + Sync` (`before_reduce(&self, action: &AppAction) -> Option<String>`, `after_reduce(&self, name: &str, state: &AppState)`). Remove `Ui::recorder() -> &Recorder` (`bridge.rs:270`) and pass `Recorder` directly to `ReportView`, `Shell`, and `desktop/tests/report.rs`. |
| `widgets.rs` (517 LOC) | 68 | `col`, `row`, `push_primary`, `push_quiet`, shared controls. |
| `chrome.rs` (116 LOC) + **relocated panel & label primitives** (~230 LOC) | 49+ | Icons, headers, badges; **must also absorb:** (a) the 16 shared panel primitives from `panels.rs:28–30, 52–339` (`GREEN`, `ORANGE`, `RED`, `panel_header`, `card`, `status`, `text`, `secondary`, `icon`, `action`, `icon_action`, `title`, `selected`, `family`, `session_line`, `header_with`), (b) `desk::runs` (**8 callers** across 5 crates), (c) `request_card::release_label` (used by `machine_page`), (d) `machine_running::agent_label` (used by `overlay_running`). |
| `keys.rs` (483 LOC) | 11 | Keybindings & GPUI actions. **Bazel note:** move `exports_files(["src/keys.rs"], visibility = ["//apps/humata2/features/app:__pkg__"])` (`ui/BUILD.bazel:3–6`) to `ui/core/BUILD.bazel` and update `features/app/BUILD.bazel:44`. |
| `action_menu.rs` (109 LOC) | 5 | Shared dropdown/popover menu. |
| `repo_search_field.rs` (371 LOC) | 5 | Shared repo picker used by 5 overlay files. |
| `code_wrap.rs` (128 LOC) | 1+ | Line-wrapping math shared by `panels` and `panel_changes`/`diff_body`. |
| `full_text.rs` (86 LOC) | 1+ | Pure text helper used by `panel_changes`. |
| `ConversationScroll` (~30 LOC, from `conversation_tab.rs`) | 2 | Scroll-state handle shared by `document_tab.rs:282` (`ui/changes`) and `conversation_tab.rs` (`ui/workspace`); keeps `ui/changes` a 100% leaf crate. |
| `Assets`, `Icons`, `init`, `WINDOW` (`lib.rs:77–168`) | all tests + `desktop` | Lets UI sub-crate component tests boot GPUI without linking `humata2_ui` (`Shell`). |

### 1.3 Proposed UI crates and file mapping (all 87 files, 44,242 LOC)

Keeping `overlay_desk*.rs` (7 files with inherent `impl Overlays`) and `overlay_add_machine.rs` (`#[path]` child of `overlays.rs`, calling `super::{sheet, sync}`) inside `ui/overlays` avoids `E0116` orphan-rule errors and `ui/overlays <-> ui/machines` cycles while keeping `ui/changes`, `ui/desk`, `ui/machines`, `ui/settings`, and `ui/report` 100% leaf crates.

| Proposed crate (`crate_name`) | Target dir | Files from `apps/humata2/ui/src/` | LOC | Internal UI deps |
|---|---|---|---:|---|
| `ui/core` (`humata2_ui_core`) | `apps/humata2/ui/core` | `lib.rs` (`Assets`, `Icons`, `init`, `WINDOW`), `bridge.rs` (288), `theme.rs` (99), `keys.rs` (483), `widgets.rs` (517), `chrome.rs` (116 + ~230 relocated helpers), `action_menu.rs` (109), `code_wrap.rs` (128), `full_text.rs` (86), `repo_search_field.rs` (371) — **9 source files** | ~2,600 | none |
| `ui/changes` (`humata2_ui_changes`) | `apps/humata2/ui/changes` | `panel_changes/` (4,005), `changes_loading.rs` (112), `comment_cards.rs` (361), `panel_stack.rs` (651), `doc_history.rs` (728), `document_tab.rs` (1,325), `code_editor.rs` (746) — **7 source files** | 7,928 | `ui/core` (**100% leaf**) |
| `ui/desk` (`humata2_ui_desk`) | `apps/humata2/ui/desk` | `desk.rs` (544), `desk_bar.rs` (523), `desk_board.rs` (195), `desk_files.rs` (310), `desk_info.rs` (326), `channel_desk.rs` (378), `sidebar_desk.rs` (212) — **7 source files** | 2,488 | `ui/core` (**100% leaf**) |
| `ui/machines` (`humata2_ui_machines`) | `apps/humata2/ui/machines` | `hosts.rs` (447), `machine_page.rs` (1,922), `machine_running.rs` (407), `away_left_out.rs` (136), `transfers_tray.rs` (187) — **5 source files** | 3,099 | `ui/core` (**100% leaf**) |
| `ui/settings` (`humata2_ui_settings`) | `apps/humata2/ui/settings` | `settings.rs` (797), `settings_providers.rs` (496), `settings_repositories.rs` (280), `access.rs` (211) — **4 source files** | 1,784 | `ui/core` (**100% leaf**) |
| `ui/overlays` (`humata2_ui_overlays`) | `apps/humata2/ui/overlays` | `overlays.rs` (618), `overlay_root.rs` (190), `overlay_palette.rs` (270), `palette_hits.rs` (400), `overlay_pr_review.rs` (802), `overlay_submit_review.rs` (612), `overlay_new_session.rs` (369), `overlay_new_channel.rs` (425), `overlay_fork.rs` (491), `overlay_move.rs` (180), `overlay_archive.rs` (150), `overlay_close.rs` (239), `overlay_rename.rs` (137), `overlay_continue.rs` (189), `overlay_running.rs` (202), `overlay_toasts.rs` (165), `overlay_shortcuts.rs` (58), `overlay_channel_add_repo.rs` (193), `overlay_session_add_repo.rs` (176), **`overlay_add_machine.rs`** (1,320 — links `ghostty_vt`, `terminal_pane`, `terminal_view`), **`overlay_desk.rs`** (423), **`overlay_desk_add_repo.rs`** (124), **`overlay_desk_channel.rs`** (476), **`overlay_desk_clean_up.rs`** (223), **`overlay_desk_close.rs`** (76), **`overlay_desk_copy.rs`** (128), **`overlay_desk_new.rs`** (304) — **27 source files** | 8,940 | `ui/core`, `ui/desk` (`desk_info::{blocked_line, switch_row}`), `ui/machines` (`machine_page::{stop_confirm, start_confirm}`) |
| `ui/report` (`humata2_ui_report`) | `apps/humata2/ui/report` | `record.rs` (1,494), `record_video.rs` (122), `report_view.rs` (1,011), `report_draw.rs` (181) — **4 source files** | 2,808 | `ui/core` (**100% leaf**) |
| `ui/workspace` (`humata2_ui_workspace`) | `apps/humata2/ui/workspace` | `sidebar.rs` (826), `sidebar_pinned.rs` (521), `channel.rs` (1,031), `session_tabs.rs` (1,153), `conversation_tab.rs` (396), `view_tab.rs` (436), `terminals.rs` (1,370), `board.rs` (1,575), `panel_board.rs` (214), `panel_files.rs` (336), `panel_info.rs` (314), `panel_inbox.rs` (90), `panel_journal.rs` (80), `file_manager.rs` (1,255), `request_card.rs` (1,193), `inbox_terminal.rs` (332), `resume_failed.rs` (193), plus `lists::inbox` moved from `lists.rs` — **17 source files** | ~11,550 | `ui/core`, `ui/changes`, `ui/desk`, `ui/machines`, `ui/settings` *(links `terminal_*` alongside `ui/overlays`)* |
| `ui` (`humata2_ui` thin shell) | `apps/humata2/ui` | `shell.rs` (886), `pages.rs` (716), `panels.rs` (~540 after moving primitives), `lists.rs` (~620 after moving `inbox`), `starting.rs` (156), `restart_report.rs` (158), `lib.rs` (~60) — **7 source files** | ~3,130 | All 8 sub-crates |

### 1.4 Exhaustive cross-crate `crate::` references (`apps/humata2/ui`)

All references to `theme`, `bridge`, `widgets`, `chrome`, `keys`, `action_menu`, `code_wrap`, `full_text`, and `repo_search_field` become `humata2_ui_core::*`. Every **non-`ui/core`** cross-crate `crate::` reference across all 87 files:

| Source file (new crate) | Line(s) | Current `crate::` reference | Target crate | Resolution |
|---|---|---|---|---|
| `bridge.rs` (`ui/core`) | L24, L56, L270 | `crate::record::{Recorder, action_name}` + `Ui::recorder()` | `ui/report` | **Break cycle:** pre/post-reduce `ActionRecorder` trait in `ui/core::bridge`; pass `Recorder` directly to `ReportView`/`Shell`/tests |
| `bridge.rs` (`ui/core`) | L23 | `crate::inbox_terminal::InboxRoom` | `ui/workspace` | **Break cycle:** move `InboxRoom`, `Measured`, `DETAIL_PAD`, `DETAIL_GAP` (`inbox_terminal.rs:37–105`, ~68 LOC) to `ui/core::bridge` |
| `document_tab.rs` (`ui/changes`) | L282 | `crate::conversation_tab::ConversationScroll` | `ui/workspace` | **Move** `ConversationScroll` (~30 LOC) to `ui/core` |
| `desk_board.rs`, `desk_files.rs` (`ui/desk`) | L171, L270 | `crate::panels::panel_header` | `ui` | **Move** `panel_header` (+ 15 sibling panel primitives) to `ui/core::chrome` |
| `machine_page.rs` (`ui/machines`) | L11 | `crate::request_card::release_label` | `ui/workspace` | **Move** `release_label` (~15 LOC) to `ui/core::chrome` |
| `settings.rs` (`ui/settings`) | L54 | `crate::desk::runs` | `ui/desk` | **Move** `runs` (~20 LOC, 8 callers total) to `ui/core::chrome` |
| `overlay_pr_review.rs`, `overlay_submit_review.rs`, `overlay_desk.rs` (`ui/overlays`) | L16, L15, L8 | `crate::desk::runs` | `ui/desk` | Uses `ui/core::chrome::runs` |
| `overlay_desk_channel.rs` (`ui/overlays`) | L8 | `crate::desk_info::{blocked_line, switch_row}` | `ui/desk` | Direct dep `ui/overlays -> ui/desk` |
| `overlay_running.rs` (`ui/overlays`) | L8 | `crate::machine_running::agent_label` | `ui/machines` | **Move** `agent_label` (~15 LOC) to `ui/core::chrome` |
| `overlays.rs` (`ui/overlays`) | L125, L270, L300 | `crate::terminals::{FocusTarget, focus_target}` | `ui/workspace` | **Move** `FocusTarget` + `focus_target` to `ui/core::bridge` |
| `overlays.rs` (`ui/overlays`) | L349, L367 | `crate::machine_page::{stop_confirm, start_confirm}` | `ui/machines` | Direct dep `ui/overlays -> ui/machines` |
| `board.rs` (`ui/workspace`) | L32, L985 | `crate::shell::Shell` (`Context<Shell>`), `crate::desk::runs` | `ui`, `ui/desk` | Generalize `board::Inputs::new<V: 'static>(..., cx: &mut Context<V>)`; use `ui/core::chrome::runs` |
| `request_card.rs` (`ui/workspace`) | L219 | `crate::desk::runs` | `ui/desk` | Uses `ui/core::chrome::runs` |
| `channel.rs` (`ui/workspace`) | L2, L4, L1022 | `crate::lists::inbox` | `ui` | **Break cycle:** move `lists::inbox` (full Inbox page view) into `ui/workspace` |
| `channel.rs` (`ui/workspace`) | L1019, L1029 | `crate::channel_desk::tab`, `crate::access::tab` | `ui/desk`, `ui/settings` | Direct deps `ui/workspace -> {ui/desk, ui/settings}` |
| `file_manager.rs` (`ui/workspace`) | L1185 | `crate::transfers_tray::tray` | `ui/machines` | Direct dep `ui/workspace -> ui/machines` |
| `panel_board.rs`, `panel_files.rs`, `panel_info.rs`, `sidebar.rs` (`ui/workspace`) | L7, L170, L229–231, L519/522/708 | `desk_board::panel`, `desk_files::panel`, `desk_info::{access, channel_switches, targeting}`, `sidebar_desk::{channel_row, session, rows}` | `ui/desk` | Direct dep `ui/workspace -> ui/desk` |
| `session_tabs.rs` (`ui/workspace`) | L15, L201, L512, L654, L695–696, L844–846 | `code_editor`, `document_tab::{mode_switch, Documents}`, `desk_bar::{bar, channel_bar}`, `resume_failed::{reboot_line, note}`, `away_left_out::line` | `ui/changes`, `ui/desk`, `ui/workspace`, `ui/machines` | Direct deps (`resume_failed.rs` moves into `ui/workspace`) |
| `lists.rs`, `pages.rs`, `panels.rs`, `shell.rs` (`ui` shell) | wiring calls | All 8 sub-crates | All 8 sub-crates | Top-down wiring from thin `ui` shell |

### 1.5 Test migration map (`apps/humata2/desktop/tests/*` & `ui`)

23 of the 45 `desktop/tests/native/*.rs` files record full 1280×760 `Shell` PNGs consumed by `desktop:look_test` (`tests/__snapshots__/native/*.png` vs `design/*.png`). Keep full-window golden-recording frames and multi-crate flows in `desktop`, and migrate single-crate component/behavioral tests into their owning UI sub-crates (mounting views directly over `ui_core::Ui` + `ui_core::Assets` without `Shell`):

| Current test target / file | Destination crate | Rationale |
|---|---|---|
| `ui:ui_unit_test` | Split into 9 `*_unit_test` targets (one per UI crate) | Each crate runs its own pure unit tests (`diff_body::tests` in `ui/changes`, `keys::tests` in `ui/core`, etc.). |
| Single-crate `native/*` diff/review/doc tests: `diff_big`, `diff_full_screen`, `diff_keys`, `diff_stress`, `diff_view`, `doc_versions`, `editor`, `editor_comments`, `pr_ask`, `push`, `review`, `review_switches`, `review_ui`, `stack`, `targets`, `views` + changes cases in `panels.rs` *(total 19 review/diff `native/*` files hold 176 tests; the 3 full-window golden files `document`, `pr_review`, `pr_session` stay in `desktop` or split out their golden capture)* | **`ui/changes/tests/`** | Pure review/diff/editor/doc-history rendering; largest UI cache win. |
| Single-crate `native/*` desk tests: `desk_events`, `desk_target`, `channel_desk`, `channel_desk_reopen`, `channel_desk_target` | **`ui/desk/tests/`** | Single-crate Desk & channel-desk behavioral tests (full-window `native/desk.rs` golden stays in `desktop`). |
| Single-crate `native/*` overlay tests: `archive_drafts`, `close_running`, `fork_move`, `session_menu` | **`ui/overlays/tests/`** | Standalone modal/overlay tests (full-window `overlays`, `new_channel`, `pr_review` goldens stay in `desktop`). |
| `native/providers_settings.rs`, `native/agent_switches.rs` | **`ui/settings/tests/`** | Pure settings views. |
| `desktop/tests/report.rs` (10 tests) | **`ui/report/tests/`** | Bug-report recorder, drawing, replay, and report sheet (passes `Recorder` directly). |
| `desktop/tests/files_page.rs`, `files_row_menu.rs`, `inbox_copy.rs`, `terminal_replica.rs`, single-crate `native/{inbox_terminal, marks, queue, terminal}.rs` + workspace cases in `panels.rs` *(total 8 workspace `native/*` files hold 46 tests; full-window `board`, `channel`, `lists`, `session` goldens stay in `desktop`)* | **`ui/workspace/tests/`** | Workspace pages, files page, inbox cards, terminal replica. |
| Full-window `look_test` goldens (`native/{board, channel, desk, document, hosts_settings, lists, machines, new_channel, overlays, pr_review, pr_session, session, shell, stop}.rs`, `design.rs` + `design/*.rs`, `look.rs`), cross-crate flows (`machines_flow.rs`, `root_change.rs`, `shell.rs`), and live-daemon E2E (`live_flows_cases` / `live_flows_test` + `live_flows_shard.sh`, `live_flows_ssh_test`, `live_flows_shard_test`, `live_inbox_test`, `live_machines_test`, `live_boards_test`, `desktop:unit_test`) | **Stay in `desktop`** | Require full `Shell` window for `look_test` pixel parity, span multiple UI crates (`root_change` = `settings` + `overlays`; `machines_flow` = `machines` + `overlays` + `settings`), or run against real `humatad2`. |

---

## Part 2 — Remaining `daemon/humatad` Services (Phase 4)

### 2.1 Foundation cleanup prerequisite (split around `daemon/ports`)

Relocating pure helpers breaks ~half of the **29 direct 2-cycles** across the 16 daemon services. **Ordering matters:** pure zero-service helpers move *before* `daemon/ports` (Step 6a), while service-dependent helpers move *after* `InboxPort`/`SessionPort` exist (Step 7b):

- **Step 6a — Zero-service helpers into `daemon/rpc_kit`:**
  - From `channel_service.rs`: `blocking`, `failed`, `error_of`, `timestamp_ms`, `Gated`, `gate`.
  - From `trash_service.rs`: `lock`, `now_ms`, `Answers`, `answer_key`, `caller_data`, `decode_caller_data`.
  - From `machine_service.rs`: `lock`.
  - From `history_service`: `history_service/rfc3339.rs` (submodule) and `timestamp`.
  - From `serve.rs` / `daemon.rs`: `Role`, `SOCKET_PATH_MAX`, `API_MAJOR`, `API_MINOR`.
- **Step 6a — Zero-service helpers into `daemon/runtime`:**
  - `holder_end.rs` (85 LOC), `screen_question.rs` (100 LOC), `agent_switches.rs` (707 LOC), `stop/gate.rs` (86 LOC), and `struct Release` (`upgrade/build.rs:30`).
- **Step 6a — Eliminate `#[cfg(feature = "admin")]` library twins:**
  - Currently **8 library files across 6 crates** gate behavior on `#[cfg(feature = "admin")]`: `identity.rs:734–867` (`rpc_kit`), `fault.rs:23–77` & `tool_path.rs:34` (`runtime`), `desk_service/view.rs:17` (`svc_channels`), `peer_client.rs:62` & `relay.rs:260,266` (`svc_peer`), `review/pull_requests/{heads.rs:585,poll.rs:39}` (`svc_review`), and `session_service/archive.rs:163` (`svc_sessions_archive`). Convert these to a runtime `AdminMode` / `TestConfig` flag passed from `main.rs` (keeping `HUMATA2_STOP_AT`/`HUMATA2_HOLD_AT` under `runtime_faults`) so zero library crates need `_admin` twins.
- **Step 7b (AFTER `InboxPort`/`SessionPort` exist):**
  - Move `channel_service::{answer_gated, answer_gated_showing, answer_plan}` (which take `&dyn InboxPort`) into `daemon/rpc_kit`.
  - Move `start_message.rs` (284 LOC) and `start_ready.rs` (250 LOC) (rewired to `InboxPort`/`SessionPort`) into `daemon/runtime`.
  - Break the remaining Phase 2 `rpc_kit`/`runtime` reverse edges into services (e.g. `identity` -> `cli_login`, `desk_service::channel_desk::access_of`, `relay_auth`, `terminal_session`; `attach`/`events` -> `session_service`, `terminal`, `stop`) via narrow `daemon/ports` traits.

### 2.2 Concrete port traits in `daemon/ports` (9 Phase 4 + 5 Phase 2 = 14 ports)

```rust
// apps/humata2/daemon/ports/src/lib.rs (Phase 4 port traits)

#[async_trait::async_trait]
pub trait InboxPort: Send + Sync {
    async fn approved_on(&self, machine: &str, id: &str, session: &str) -> Option<pb::InboxRequest>;
    async fn pending_ids_of(&self, sessions: &[String]) -> Vec<String>;
    async fn withdraw_pending_of(&self, session: &str, reason: pb::WithdrawReason) -> Result<(), PortError>;
    async fn withdraw_all_pending(&self, reason: pb::WithdrawReason) -> Result<(), PortError>;
    async fn answer_relayed(&self, relayed: &pb::InboxAnswerRelay) -> Option<pb::InboxAnswerRefused>;
    async fn post_board_ask(&self, ask: BoardAsk) -> Result<Deferred, PortError>;
    async fn ask_permission(&self, req: PermissionAsk) -> Result<PermissionDecision, PortError>;
    fn screen_output(&self, session: &str, holder_pid: u32);
    fn screen_quiet(&self, session: &str, holder_pid: u32);
    fn screen_gone(&self, session: &str);
}

#[async_trait::async_trait]
pub trait AwayPort: Send + Sync {
    async fn forward(&self, method: AwayMethod, req: ForwardRequest) -> Result<ForwardReply, PortError>;
    fn record_journal_now(&self, kind: &'static str, payload: serde_json::Value);
    fn policy_source(&self) -> Arc<dyn PolicySource>;
    fn uplinks(&self) -> UplinksHandle;
    // Also covers passphrase/policy (check_passphrase, set_approval_passphrase, verifier, kept_policy_stamp)
    // and host-routing helpers (ask_host, agent_session, names_another, unnamed_here, record_view_journal).
}

#[async_trait::async_trait]
pub trait PeerRelayPort: Send + Sync {
    fn channel(&self, machine: &str) -> Result<tonic::transport::Channel, RelayError>;
    fn link_state(&self, machine: &str) -> LinkState;
    fn watch_link(&self, machine: &str) -> watch::Receiver<LinkState>;
    fn states(&self) -> Vec<(String, LinkState)>;
    async fn create_channel_on(&self, machine: &str, channel: &str, req: pb::CreateChannelRequest) -> Result<(), RemoteError>;
    async fn run_remote_op(&self, machine: &str, op: RemoteOpSpec) -> Result<MachineOutcome, RemoteError>;
    // Also covers peer_client::{Dial, supervise}, peer_reports, peer_skills::agent_sources,
    // remote_end::{RemoteEnd, RouterEnd, virtual_path}, and relay::proven_channel.
}

#[async_trait::async_trait]
pub trait TransferPort: Send + Sync {
    async fn pack(&self, machine: &str, op: &str, session: &str) -> Result<pb::CarryPackResponse, PortError>;
    async fn unpack(&self, machine: &str, op: &str, session: &str, channel: &str) -> Result<pb::CarryUnpackResponse, PortError>;
    async fn copy_stage(&self, id: &str, from: PlaceRef, to: PlaceRef) -> Result<(), PortError>;
    async fn release_stage(&self, machine: &str, op: &str, session: &str) -> Result<(), PortError>;
    async fn abort_stage(&self, machine: &str, op: &str, session: &str) -> Result<(), PortError>;
}

#[async_trait::async_trait]
pub trait TerminalPort: Send + Sync {
    fn writers(&self) -> Arc<dyn WritersHandle>;
    fn forget_tab(&self, tab: &str);
    async fn type_for_script(&self, session: &str, text: &str, enter: bool, key: &str) -> Result<(), PortError>;
    async fn list_running_holders(&self, machine: &str) -> Result<Vec<pb::RunningHolder>, PortError>;
    async fn control_running_holder(&self, tab: &str, op: HolderOp) -> Result<(), PortError>;
    // Also covers comments_target/type_comments/comments_arrived (for svc_review) and terminal_session::Processes.
}

#[async_trait::async_trait]
pub trait MachinePort: Send + Sync {
    fn saw(&self, machine: &str, reach: pb::Reachability);
    fn link_up(&self, machine: &str, runtime: &tokio::runtime::Handle);
    fn link_down(&self, machine: &str);
    fn is_forgetting(&self, machine: &str) -> bool;
    async fn run_machine_script(&self, ctx: ScriptContext) -> Result<(), PortError>;
    // Also covers machine_of/status_of/read_providers/launch_observed for svc_catalog & svc_peer.
}

#[async_trait::async_trait]
pub trait LifecyclePort: Send + Sync {
    fn require_running(&self, caller: &Caller) -> Result<(), PortError>;
    fn machine_ready(&self, machine: &pb::MachineId) -> Result<(), PortError>;
    async fn wait_started(&self, doing: Awaited) -> Result<(), PortError>;
    fn tab_token_env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)>; // wraps restart::Tokens::env (L755)
    async fn approve_migration(&self, id: &str) -> Result<pb::InboxRequest, PortError>;
    fn host_stops(&self) -> Arc<dyn HostStopsHandle>;
    // Also covers restart::{Operations, Started}, stop::{StopFolder, remote::link_up}, upgrade::{Migrations, Watch, Builds}.
}

#[async_trait::async_trait]
pub trait ChannelPort: Send + Sync {
    fn tell_channel(&self, channel: &str, change: pb::Change, by: Option<pb::Actor>);
    async fn refuse_desk_channel_action(&self, session: &str, action: &str) -> Result<(), PortError>;
    async fn close_channel_desks(&self, channel: &str) -> Result<(), PortError>;
    async fn migrate_desk_folders(&self) -> Result<(), PortError>;
    async fn rewrite_channel_folders(&self, from: &Path, to: &Path) -> Result<(), PortError>;
    // Also covers finish_restore, gh_repos, pick_place, carry_conversations, desk_service::user_only.
}

#[async_trait::async_trait]
pub trait SessionPort: Send + Sync {
    async fn publish_session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>);
    fn watch_agent(&self, session: String);
    fn is_moving(&self, session: &str) -> bool;
    fn is_asking(&self, session: &str) -> bool;
    async fn resume_loaded(&self, caller: &Caller, loaded: LoadedSession) -> Result<(), PortError>;
    async fn begin_review(&self, caller: &Caller, channel: ChannelRecord, req: &pb::StartReviewRequest) -> Result<pb::StartSessionResponse, PortError>;
    async fn repair_session(&self, caller: &Caller, id: String, approved: bool) -> Result<Gated<pb::Operation>, PortError>;
    async fn adopted(&self, id: &str);
    async fn restart_session_shells(&self, session: &str) -> Result<(), PortError>;
    // Also covers after_restart, cut_rebuilds, moving_changes, paused_for_move, refuse_input_while_moving, running_turn.
}
```

### 2.3 Per-service breakdown (the 11 remaining `daemon/humatad` services)

| Service crate (`*_faults`?) | `daemon/humatad/src/` modules absorbed | LOC | Port traits needed from `daemon/ports` | Cycles broken |
|---|---|---:|---|---|
| **`svc_inbox`** (No faults) | `inbox/` (12 files, 6,813), `screen_ask.rs` (1,011) | 7,824 | `AwayPort`, `LifecyclePort`, `SessionPort`, `TerminalPort` | Breaks `svc_inbox <-> {svc_away, svc_lifecycle, svc_sessions, svc_terminal, svc_channels}` |
| **`svc_terminal`** (Yes: `terminal.rs:1236`) | `terminal/` (2 files, 2,355), `remote_terminal.rs` (1,211), `terminal_session.rs` (457), `running_holders/` (3 files, 987) | 5,010 | `InboxPort`, `LifecyclePort`, `PeerRelayPort`, `SessionPort`, `ChannelPort` | Breaks `svc_terminal <-> {svc_inbox, svc_sessions, svc_channels, svc_machines}` |
| **`svc_transfers`** (Yes: `carry`, `carry_service`) | `transfer_service.rs` (1,471), `transfer_end_service.rs` (670), `carry.rs` (1,296), `carry_service/` (2 files, 734) | 4,171 | `AwayPort`, `InboxPort`, `PeerRelayPort`, `TrashBinPort`, `FilesPort` | Breaks `svc_transfers <-> {svc_sessions, svc_channels, svc_peer}` |
| **`svc_away`** (No faults) | `away/` (10 files, 3,636) | 3,636 | `InboxPort` | Breaks `svc_away <-> {svc_inbox, svc_catalog}` |
| **`svc_peer`** (Yes: `relay.rs:260` + `power_cut`) | `peer.rs` (1,279), `peer_client.rs` (1,238), `peer_link.rs` (414), `peer_reports.rs` (400), `peer_skills.rs` (884), `relay.rs` (1,368), `relay_auth.rs` (624), `host_relay.rs` (692), `remote_end.rs` (1,304), `remote_work.rs` (3,832) | 12,035 | `AwayPort`, `InboxPort`, `LifecyclePort`, `BoardPort`, `FilesPort`, `ReviewPort`, `MachinePort`, plus inject `connect_or_start` + `stop_signal` callbacks into `peer_link.rs:41,48` (breaks `svc_peer -> humatad`) | Breaks `svc_peer <-> {svc_machines, svc_lifecycle, svc_sessions, svc_files, svc_review, svc_catalog, humatad}` |
| **`svc_machines`** (Yes: `machine_service`) | `machine_service/` (8 files, 5,258), `machine_scripts/` (4 files, 1,906) | 7,164 | `AwayPort`, `InboxPort`, `PeerRelayPort`, `LifecyclePort`, `ChannelPort`, `TerminalPort`, `SessionPort`, `BoardPort`, `ReviewPort`, `TrashBinPort`, `CatalogPort` | Breaks `svc_machines <-> {svc_peer, svc_lifecycle, svc_channels, svc_sessions, svc_board, svc_catalog}` |
| **`svc_lifecycle`** (Yes: `upgrade`, `stop`) | `upgrade/` (11 files, 7,909), `stop/` (11 files, 4,636), `restart/` (2 files, 1,427), `doctor.rs` (186), `daemon.rs` (363) *(can split into `svc_upgrade` 7.9k + `svc_stop_restart` 6.6k)* | 14,521 | `AwayPort`, `InboxPort`, `PeerRelayPort`, `MachinePort`, `ChannelPort`, `SessionPort` | Breaks `svc_lifecycle <-> {svc_peer, svc_machines, svc_sessions, svc_channels, svc_inbox}` |
| **`svc_channels`** (Yes: `channel_service`, `desk_service`) | `channel_service/` (15 files, 8,337), `desk_service/` (18 files, 8,260) *(recommended: split into `svc_channels` 8.3k + `svc_desk` 8.3k)* | 16,597 | `InboxPort`, `LifecyclePort`, `PeerRelayPort`, `MachinePort`, `SessionPort`, `TerminalPort`, `TrashBinPort`, `FilesPort`, `CatalogPort` | Breaks `svc_channels <-> {svc_sessions, svc_lifecycle, svc_machines, svc_inbox, svc_terminal, svc_transfers, svc_catalog, svc_review}` |
| **`svc_sessions_common`** (Yes, ~800 LOC) | Shared state struct `SessionCore`, `Tell`, `Loaded`, `Lineage`/`publish_family`, `watch::agent`, `fork::{human_bytes, not_built_part}`, `archive::{PlanKind, failure_of, session_id}`, `archive_remote::{finished, follow_to_end, machine_failed, next_link_up}`, `push::{PullRequestText, RepoPush}`, `start_fresh::{Handover, Source, retype_pending}`, `remote::elsewhere`, `now_ms`, `clean_title`, `start_error`, `resume_failed` | ~800 | `ChannelPort`, `InboxPort`, `LifecyclePort`, `PeerRelayPort` | Eliminates all 10 cross-group calls and lets `local`, `transfer`, and `archive` compile as **parallel siblings**. |
| **`svc_sessions_local`** (Yes) | `session_service.rs` (3,315), `add_repo.rs` (323), `conversation_read.rs` (286), `ended.rs` (57), `fork.rs` (727), `fork_card.rs` (92), `hooks.rs` (516), `lineage.rs` (294), `notes.rs` (474), `own_copy.rs` (493), `pointers.rs` (520), `rename_branch.rs` (170), `script_steps.rs` (100), `status_since.rs` (135), `stop_launching.rs` (119), `stop_running.rs` (101), `turns.rs` (1,220), `turns_list.rs` (95), `watch.rs` (366) — **19 files** | 9,403 | `svc_sessions_common`, `ChannelPort`, `InboxPort`, `LifecyclePort`, `MachinePort`, `PeerRelayPort`, `ReviewPort`, plus `SessionTransferPort` & `SessionArchivePort` | **Orphan-rule resolution:** `SessionService` tonic impl lives in `svc_sessions_local` (or thin `humatad` dispatcher) and delegates transfer/archive RPCs through `SessionTransferPort` / `SessionArchivePort` (eliminating cross-crate inherent `impl SessionSvc` in all 17 transfer/archive files). |
| **`svc_sessions_transfer`** (Yes) | `move_session.rs` (1,554), `move_session/scripts.rs` (64), `move_recover.rs` (244), `fork_remote.rs` (1,140), `fork_remote/context.rs` (511), `rebuild.rs` (1,454), `push.rs` (1,108), `push/resume.rs` (405), `start_fresh.rs` (1,339), `continue_in.rs` (324), `continue_in/there.rs` (755), `continue_in/there/kept.rs` (336), `remote.rs` (497), `remote_review.rs` (417), `remote_start.rs` (432) — **15 files** | 10,580 | `svc_sessions_common`, `ChannelPort`, `InboxPort`, `LifecyclePort`, `MachinePort`, `PeerRelayPort`, `TransferPort`, `SessionPort` | Implements `SessionTransferPort`; parallel sibling to `svc_sessions_local` and `svc_sessions_archive`. |
| **`svc_sessions_archive`** (Yes) | `archive.rs` (1,809), `archive_remote.rs` (2,051), `archive_remote/bring.rs` (419), `archive_remote/compare.rs` (431), `archive_remote/pending.rs` (648), `forget.rs` (422) — **6 files** | 5,780 | `svc_sessions_common`, `ChannelPort`, `LifecyclePort`, `MachinePort`, `PeerRelayPort`, `TransferPort`, `ReviewPort`, `SessionPort` | Implements `SessionArchivePort`; parallel sibling to `svc_sessions_local` and `svc_sessions_transfer`. |

### 2.4 Classification of all 216 `daemon/humatad/tests/*.rs` targets

Every single one of the **216** integration test files under `apps/humata2/daemon/humatad/tests/` is assigned below: **72 Tier A** (true single-service in-process gRPC), **131 Tier B** (scoped test daemon binaries + scoped `*_release` twins for `remote_rig.rs:199`), and **13 Tier C** (full-daemon E2E / release-auth / CLI / MCP).

| Owning service crate | Tier A — In-process scoped gRPC (`svc_<name>/tests/*.rs`) (72) | Tier B — Scoped test daemon binary (`humatad2_test_*` + `*_release` twins) (131) | Tier C — Full daemon (`humatad/tests/`) (13) |
|---|---|---|---|
| **`svc_review`** (19) | `pr_base`, `pr_commits`, `pr_diff_stress`, `pr_refresh`, `review`, `review_bases`, `review_file_trees`, `review_per_repo`, `review_snapshots`, `review_stack`, `views_forks`, `views_plan_notes` (12) | `comment_tools` (MCP/stdio), `comments`, `pr_review`, `pr_stopped` (local faults); `pr_review_move`, `pr_review_remote`, `views_remote` (`humatad2_test_review_remote` — includes `svc_review_faults` + `svc_sessions_{local,transfer}` + `svc_channels` + `svc_transfers` + `svc_peer`) (7) | — |
| **`svc_board`** (2) | `boards` (1) | `board_tools` (MCP/stdio) (1) | — |
| **`svc_files`** (4) | `file_changes`, `file_ops`, `files`, `find_files` (4) | — | — |
| **`svc_mcp`** (4) | — | `mcp_desk_folder`, `mcp_doc_versions` (scoped `mcp` + `desk`) (2) | `mcp`, `session_delete_other` (2) |
| **`svc_catalog`** (24) | `branch_names`, `branch_settings`, `codex_trust`, `history`, `marks`, `provider_import`, `provider_remove`, `read_marks`, `repo_owners`, `repos`, `trash` (11) | `access`, `settings_sections` (faults); `history_tools`, `repo_tools` (MCP/stdio); `login`, `providers`, `skills`, `read_marks_remote` (`humatad2_test_catalog_remote`: `catalog` + `machines` + `peer` + `sessions_local` + `channels`) (8) | `auth`, `cli_actor`, `cli_files`, `cli_login`, `cli_waiting` (5) |
| **`svc_inbox`** (6) | `inbox` (1) | `matrix`, `ready_gate`, `screen_questions` (scoped + `humata` CLI); `screen_questions_remote` (`humatad2_test_power_cut`); `flow_26_10` (`humatad2_test_away_board`) (5) | — |
| **`svc_terminal`** (12) | `checkpoint`, `holder_gone`, `lagged_attach`, `paste`, `resume_exits_at_once`, `terminal_program` (6) | `adopt_holder`, `destroy_holders`, `holder_swap_identity`, `running_holders`, `tab_tools`, `terminal` (scoped + `humata2_holder` + `machines`/`access`/CLI/faults) (6) | — |
| **`svc_transfers`** (6) | `received_root`, `transfer_end` (2) | `transfers` (`SIGKILL` + `files`/`trash`), `inbox_copy`, `remote_files`, `remote_transfers` (`humatad2_test_transfers`: `transfers` + `files` + `machines` + `catalog/trash` + `peer`) (4) | — |
| **`svc_away`** (12) | — | `away_board_journal`, `away_boundary`, `away_contain_copy`, `away_forward`, `away_inbox`, `away_link`, `away_login`, `away_login_pipe`, `away_policy_copy`, `away_replay`, `away_approved_board`, `away_left_out` (`humatad2_test_away_board`: `away` + `board` + `inbox` + `catalog/access` + `peer` + `*_release`) (12) | — |
| **`svc_peer`** (3) | — | `peer_stdio`, `relay` (`humatad2_test_machines_full`) (2) | `bridge` (1) |
| **`svc_machines`** (12) | `host_variables` (1) | `machines`, `host_variables_remote`, `machine_shell_remote`, `machine_shell_remote_linux`, `resume_exits_at_once_remote`, `running_holders_remote`, `storage_remote`, `two_machines` (`humatad2_test_machines`); `forget_machine`, `remote`, `remote_history` (`humatad2_test_machines_full`: adds `faults` + `sessions` + `channels` + `terminal` + `catalog/history`) (11) | — |
| **`svc_lifecycle`** (21) | — | `daemon`, `doctor`, `ready`, `restart`, `socket_paths`, `state_lock`, `stop`, `faults` (single-daemon subprocess/signals); **`stop_box`, `stop_sessions`**, **`upgrade_box`, `upgrade_cut`, `upgrade_holders`, `upgrade_inbox`, `upgrade_kinds`, `upgrade_refusals`, `upgrade_remote_holders`** (`humatad2_test_power_cut` + `STACK`); `stop_access` (`humatad2_test_away_board`) (18) | **`upgrade_holders_move`, `upgrade_identity`, `upgrade_local`** (3) |
| **`svc_channels`** (45) | `channel_add`, `channel_delete_live`, `channel_desk_close`, `channel_desk_retarget`, `channel_folder_follows`, `channel_grid`, `channel_repo_propose`, `channels`, `delete_launching`, `desk_agent_missing`, `desk_events`, `desk_folder_files`, `desk_folder_migration`, `desk_hide_live`, `desk_move`, `doc_comments`, `doc_history`, `doc_version_diff`, `doc_versions`, `gh_repos`, `remove_machine_plan`, `repo_library_adds` (22) | `channel_desk`, `channel_rename_live`, `delete_channel`, `desk`, `desk_into_channel`, `desk_move_live`, `restore_channel_cut` (local faults); `change_root`, `channel_delete_remote`, `channel_rename_roam`, `desk_close_remote`, `desk_events_remote`, `desk_into_channel_roam`, `desk_migrate_remote`, `desk_picked_remote`, `desk_roam`, `desk_roam_upgrade`, `desk_start_host_cut`, `desk_stuck_remote`, `desk_versions_remote`, `remove_machine_cut`, `remove_machine_live`, `restart_operations` (`humatad2_test_power_cut`) (23) | — |
| **`svc_sessions_local`** (12) | `conversation_follows`, `fork_channel`, `hooks`, `replies`, `session_add_repo`, `session_events`, `session_model`, `session_notes`, `sessions`, `turns` (10) | `context_fork`, `fork` (local faults) (2) | — |
| **`svc_sessions_transfer`** (31) | `push`, `push_base`, `push_many` (3) | `continue_in`, `start_fresh` (local faults); **`power_cut_both`, `power_cut_focus`, `power_cut_host`, `power_cut_machine`, `power_cut_start_box`**, **`move_session`, `move_session_freeze`, `move_cheap`, `move_typing`, `rebuild_session`, `fork_remote`, `own_copy_remote`**, `agent_send_machine`, `continue_in_journal`, `continue_in_remote`, `detach_live`, `hooks_remote`, `host_variables_move`, `pointers`, `push_remote`, `push_remote_restart`, `remote_rename`, `start_resume_failed_remote`, `start_stuck_remote`, `stop_launching` (`humatad2_test_power_cut`) (27) | `remote_stack` (`AgentService` + `STACK` + rig) (1) |
| **`svc_sessions_archive`** (3) | — | `archive`, `archive_live` (local faults); **`remote_archive`** (`humatad2_test_power_cut`) (3) | — |

> **Full-binary E2E summary:** Every `power_cut_*`, `stop_box`, `stop_sessions`, `upgrade_*`, `move_session*`, `rebuild_session`, `fork_remote`, and `remote_archive` test stays end-to-end on **`humatad2_test_power_cut`** (plus its `humatad2_test_power_cut_release` twin for `remote_rig.rs:199`) — **never** linked against `svc_review`, `svc_board`, `svc_files`, `svc_mcp`, or `svc_catalog`. Only the **13 Tier C** tests require the full `:humatad2` / `:humatad2_admin` binary.

---

## Part 3 — Unified Always-Green Move Order (Phase 3 + Remaining Daemon Services)

Each step is a self-contained PR that keeps `bazel test //apps/humata2/...` green:

| Step | Track | Concrete change | Verification / build guarantee |
|---:|---|---|---|
| **1** | UI (Phase 0/3 prep) | Split `ui/src/panel_changes.rs` into `ui/src/panel_changes/{mod,file_list,plan,review_header,states,diff_body}.rs` (§1.1, with `mod tests` in `diff_body.rs`) and change `ui/BUILD.bazel:31` `glob(["src/*.rs"])` -> `glob(["src/**/*.rs"])`. | `ui_unit_test` + `desktop:native_test` + `desktop:panels_test` stay green. |
| **2** | UI (Phase 3 prep) | Inside `ui/src/`: move `InboxRoom`/`Measured`/`DETAIL_PAD`/`DETAIL_GAP`, `FocusTarget`/`focus_target`, `ConversationScroll`, `desk::runs`, `request_card::release_label`, `machine_running::agent_label`, and the 16 `panels.rs` primitives into `bridge.rs`/`chrome.rs`; introduce pre/post-reduce `ActionRecorder` in `bridge.rs` and pass `Recorder` directly to `ReportView`/`Shell`/tests; generalize `board::Inputs::new<V>`; move `lists::inbox` out of `lists.rs`. | All 87 files still in `//apps/humata2/ui`; eliminates every reverse cycle before creating any new crate. |
| **3** | UI | Extract **`ui/core`**; move `exports_files(["src/keys.rs"])` to `ui/core/BUILD.bazel` (updating `features/app/BUILD.bazel:44`), and widen `apps/humata2/features/*/BUILD.bazel` visibility to `//apps/humata2/ui:__subpackages__`. | `ui` depends on `ui/core` and re-exports `Assets`, `init`, `WINDOW`, `bridge`, `keys`; `desktop` builds unchanged. |
| **4** | UI | Extract leaf crates **`ui/changes`** (+ 16 single-crate `native/*` diff/review/doc tests), **`ui/report`** (+ `report.rs`), **`ui/settings`** (+ 2 `native/*` tests), **`ui/machines`**, **`ui/desk`** (+ 5 `native/*` tests), and **`ui/overlays`** (carrying `overlay_desk*.rs` & `overlay_add_machine.rs` + 4 `native/*` tests) — one PR per crate. | Editing `panel_changes` now rebuilds only ~8k LOC + `ui/changes` tests. |
| **5** | UI | Extract **`ui/workspace`** (+ `files_page.rs`, `files_row_menu.rs`, `inbox_copy.rs`, `terminal_replica.rs`, 4 single-crate `native/*` tests), leaving `apps/humata2/ui` as the ~3.1k-LOC thin shell. | `desktop` keeps full-window `look_test` golden frames, cross-crate flows, and `live_*` tests. |
| **6** | Daemon (Step 6a prep) | Relocate **pure zero-service helpers** from §2.1 into `daemon/rpc_kit` and `daemon/runtime`, and convert the 8 `#[cfg(feature = "admin")]` library sites to runtime config/flags. | Zero new service crates yet; zero `_admin` library twins needed; all 216 tests stay green. |
| **7** | Daemon (ports + Step 7b) | Add the 9 Phase 4 port traits to `daemon/ports` (§2.2), implement them on existing structs in `daemon/humatad`, then move `answer_gated*`/`answer_plan` -> `rpc_kit` and `start_message`/`start_ready` -> `runtime`, and inject `connect_or_start`/`stop_signal` into `peer_link.rs`. | Purely additive + breaks remaining `rpc_kit`/`runtime`/`peer_link` reverse edges; all 216 tests stay green. |
| **8** | Daemon (Tier 2 leaves) | Extract **`svc_away`** (3.6k, 1× compile), **`svc_inbox`** (7.8k, 1× compile), **`svc_terminal`** (5.0k, +`_faults`), and **`svc_transfers`** (4.2k, +`_faults`) one PR at a time, migrating their Tier A tests (§2.4) to in-process harnesses. | Each extraction compiles and passes its own unit + Tier A tests independently. |
| **9** | Daemon (Tier 3 peer/machines/lifecycle) | Extract **`svc_peer`** (12.0k, +`_faults`), **`svc_machines`** (7.2k, +`_faults`), and **`svc_lifecycle`** (14.5k, +`_faults`). Add `humatad2_test_machines{,_full}`, `humatad2_test_transfers`, `humatad2_test_away_board`, and `humatad2_test_catalog_remote` (each with a `*_release` twin for `remote_rig.rs:199`) and re-point their Tier B tests. | Editing `svc_review`/`svc_board`/`svc_files` no longer invalidates `machines` or `upgrade_*`. |
| **10** | Daemon (channels/desk) | Extract **`svc_channels`** (`channel_service/` + `desk_service/`, or separate `svc_channels` 8.3k + `svc_desk` 8.3k, +`_faults`) and migrate its 22 Tier A tests to in-process harnesses. | `ChannelPort` decouples channels/desk from sessions and machines. |
| **11** | Daemon (3-way sessions + `humatad2_test_power_cut`) | Extract **`svc_sessions_common`** (~800 LOC, +`_faults`), then extract **`svc_sessions_local`** (9.4k), **`svc_sessions_archive`** (5.8k), and **`svc_sessions_transfer`** (10.6k) (all +`_faults`) as parallel siblings behind `SessionTransferPort`/`SessionArchivePort`. Add `humatad2_test_power_cut` + `humatad2_test_power_cut_release` and re-point all Tier B power-cut/move/stop/upgrade tests (§2.4). | `daemon/humatad` shrinks to ~2.5k LOC of wiring; editing `turns.rs` or `notes.rs` no longer rebuilds or reruns `move_session`, `rebuild_session`, `fork_remote`, or `remote_archive`. |

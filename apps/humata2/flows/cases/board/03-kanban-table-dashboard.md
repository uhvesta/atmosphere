# The ranked To-do list, filters, and channel To-do tab

Layer: verify (To-do, the person). Platforms: macOS.

BOARDSIMPLE (D830–D874) removed the old Kanban, Table and Dashboard views, the Scope picker, and VIP. One ranked **To-do** list replaces them, with channel chips, a **Filters** popover, search, **Show closed**, folded groups, **Do next**, and a channel's **To-do** tab narrowed to that channel. Saved board routes and prepared board views from earlier builds now open on the To-do page with the matching filter (D874).

## Starting state

- Multiple channels (`payments-rewrite`, `infra`, `web`, `pr-reviews`) plus items with no channel.
- Items across **Needs attention** (pending asks and suggested items), **In progress**, **Next** (ranked by Pin, due urgency, and impact ÷ effort, plus pull-request review items), **Waiting on another item**, and **Hidden until a date**.
- The app is open on the sidebar's **To-do** page.

## Reproduce headless

- Channel To-do tab and ask approval in the headless window on fakes: `bazel test //apps/humata2/desktop:native_test --test_filter=todo_asks`.
- Reducer, filters, search, Do next and watch on fakes: `bazel test //apps/humata2/features/todo:todo_test`.
- Pure ranking, filtering and due chips: `bazel test //apps/humata2/todo:all`.
- Route loading, Do next sheet, Start the review, prepared board views and saved board route migration: `bazel test //apps/humata2/features/app:todo_route_test //apps/humata2/features/app:views_revert_test //apps/humata2/features/app:presentation_test`.
- Design goldens (frames 180–191, signed off from `mockups/boardsimple/`): `bazel test //apps/humata2/desktop:design_test`. (Old goldens 21–24 are no longer read by any test.)
- Not headless: nothing. Light and dark are both rendered headless.

## Steps

1. **I do:** open **To-do** from the sidebar (or ⌘3).
   **I see:** one list with a header count ("N open across M channels"), search box, `Sort: Ranked`, **Filters**, and **Do next · `<key>`**. Below are channel chips (`All`, each channel with its open count, `No channel`) and **Show closed**, quick add, and the groups in order: **Needs attention**, **In progress**, **Next** (numbered 1, 2, … with rank reason chips), and folded **Waiting on another item** and **Hidden until a date**.
   **State and events:** `TodoAction::Load` then `TodoAction::Watch` (`TodoService.ListTodo` + `EventService.Subscribe` on `Boards` and `Inbox`).
   **Status:** Fakes only for the UI and reducer: `features/todo/tests/todo.rs::load_shows_the_order_of_the_signed_off_frame`, `features/app/tests/todo_route.rs::opening_the_to_do_page_loads_the_list`, `todo/tests/rank.rs::first_frame_order_and_reasons`, `ui/workspace/src/todo.rs` unit tests. Live list + watch proven in `desktop/tests/live_flows/todo_asks.rs` and `clients/live_daemon/tests/todo.rs`.

2. **I do:** click a channel chip (e.g. `payments-rewrite` or `No channel`), or open a channel page and click its **To-do** tab.
   **I see:** only that channel's items; the header reads "N open in `<channel>`"; **Waiting** and **Hidden** unfold by default for a single channel; the quick-add placeholder names the channel.
   **State and events:** `TodoAction::SetFilter` (local filter; channel To-do tab sets the channel filter on entry).
   **Status:** Fakes only. `desktop/tests/native/todo_asks.rs::a_channels_todo_tab_lists_only_that_channels_items`, `features/todo/tests/todo.rs::a_channel_filter_narrows_the_list_and_the_header`, `ui/workspace/src/todo.rs::tests::waiting_and_hidden_fold_until_a_channel_is_picked`.

3. **I do:** open **Filters** and pick **Due** (`Overdue`, `This week`, `After this week`, `No date`), **Tags**, **Session** (including **None yet**), **Effort**, **Impact**, or **Only** (`Needs attention`, `Pull request reviews`).
   **I see:** the **Filters · N** badge counts active filters; a removable `Due: … ×` outline chip shows on the bar; the footer counts "X of Y items"; **Reset** clears every filter while leaving **Show closed** alone.
   **State and events:** local `ViewFilter` applied through `humata2_todo::apply`.
   **Status:** Fakes only. `features/todo/tests/todo.rs::the_more_filters_narrow_by_only_effort_and_impact`, `::session_none_yet_filter_keeps_only_items_with_no_session`, `todo/tests/filter.rs`, `ui/workspace/src/todo.rs::tests::the_filter_footer_counts_what_is_shown_of_the_channel`.

4. **I do:** type in **Search**.
   **I see:** items whose title, key or tags match, keeping the rest of the filter intact.
   **State and events:** `TodoAction::SetSearch`, local.
   **Status:** Fakes only. `features/todo/tests/todo.rs::searching_keeps_the_rest_of_the_filter`, `todo/tests/filter.rs::tags_or_and_search_and_no_channel`.

5. **I do:** click **Do next · `<key>`**.
   **I see:** the **New session** sheet prefilled with the top ranked **Next** item's title, channel, online machine, item link and first message, with a **Skip to `<next>`** button to step to the following item.
   **State and events:** `TodoAction::DoNext` / `SkipDoNext` -> `App::fill_do_next`.
   **Status:** Fakes only. `features/app/tests/todo_route.rs::do_next_opens_new_session_filled_from_the_top_item`, `::skip_moves_the_sheet_to_the_next_item_and_closing_it_ends_do_next`, `::do_next_with_nothing_to_do_opens_no_sheet`.

6. **I do:** open a prepared board view offered by an agent, then click **Revert**.
   **I see:** the To-do page opens with the view's filter and a **Showing** bar; **Revert** restores the previous screen and To-do filter.
   **State and events:** `ViewsAction::Open` / `Revert` on `ViewContent::Board` and `ViewContent::SessionBoard`.
   **Status:** Fakes only. `features/app/tests/views_revert.rs::reverting_a_board_view_brings_back_the_page_and_the_todo_filter`, `::a_session_board_view_opens_the_sessions_todo_list_and_revert_comes_back`.

## Branches

### An empty list

When there are no items at all. The header reads `no items`, **Do next** is disabled ("Do next · nothing yet"), and the page shows "Nothing to do yet" with three short sentences explaining quick add, agent suggestions and PR reviews.
**Status:** Unit-tested: `ui/workspace/src/todo.rs::tests::empty_list_says_so_and_offers_no_do_next`; design frame 184.

### A large list (more than 40 items)

When more than 40 items are shown. **Needs attention** and **In progress** fold to one summary line each above **Next**, groups other than Next cap at 10 rows behind **Show all N**, and a range footer ("1–12 of 48 ranked") follows the scroll.
**Status:** Unit-tested: `ui/workspace/src/todo.rs::tests::a_large_list_folds_the_groups_above_next`, `::groups_other_than_next_show_a_few_rows_until_all_are_asked_for`, `::the_visible_range_follows_the_scroll_and_stays_inside_the_list`; design frame 186.

### An agent changes an item while I look

When another client or an agent adds, updates, or raises an ask on an item. `TodoClient::watch` pushes the updated list without a reload.
**Status:** Works (real daemon) in `desktop/tests/live_flows/todo_asks.rs`. Reducer watch: `features/todo/tests/todo.rs::a_change_from_another_screen_arrives_through_the_watch`.

### A saved board route from an earlier build

When reopening the app with a saved `Route::Board` from before BOARDSIMPLE. It restores on `Route::Todo`.
**Status:** Fakes only. `features/app/tests/presentation.rs::an_earlier_builds_saved_board_opens_the_todo_list`.

## Expected outcomes

- [ ] One ranked To-do list shows items across all channels and sessions with computed reasons.
- [ ] Channel chips, the channel To-do tab, the Filters popover, Search and Show closed narrow the list without losing each other.
- [ ] Do next prefills New session from the top ranked item and Skip advances through Next.
- [ ] Changes from agents or other screens arrive live via watch.
- [ ] Saved board routes and prepared board views open cleanly in To-do.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Ranked list & groups | Fakes only; live load/watch Works | `features/todo/tests/todo.rs`, `todo/tests/rank.rs`, `live_flows/todo_asks.rs` |
| 2 Channel chips & channel To-do tab | Fakes only | `native/todo_asks.rs`, `features/todo/tests/todo.rs` |
| 3 Filters popover | Fakes only | `features/todo/tests/todo.rs`, `todo/tests/filter.rs` |
| 4 Search | Fakes only | `features/todo/tests/todo.rs::searching_keeps_the_rest_of_the_filter` |
| 5 Do next & Skip | Fakes only | `features/app/tests/todo_route.rs` |
| 6 Prepared board views & Revert | Fakes only | `features/app/tests/views_revert.rs` |
| Empty & large lists | Unit-tested | `ui/workspace/src/todo.rs` unit tests |
| Live updates via watch | Works (real daemon) | `live_flows/todo_asks.rs`, `features/todo/tests/todo.rs` |
| Saved board route migration | Fakes only | `features/app/tests/presentation.rs` |

## Gaps and likely bugs

- `Sort: Ranked` is currently display-only (no alternate sort picker for due/effort/impact yet).
- Filtering, search and Do next are proven on fakes; live UI tests currently cover asks and Start a session (`desktop/tests/live_flows/todo_asks.rs`).
- Persisting `TodoState.view` (active filters and `Show closed`) across a real app restart is not yet tested end-to-end.

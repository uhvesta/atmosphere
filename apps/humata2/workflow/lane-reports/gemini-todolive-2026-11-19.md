warning: --mode auto is not available for this account; continuing in the default mode.
### Files changed, new, and deleted

**Changed**
- [todo.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/workspace/src/todo.rs) — added `start_session_label`, the `"todo-start-session"` quiet button on the left of the page drawer footer (`"Start a session on this"` / `"Start another session on this"` per frames `b1-2` and `b4-2`, hidden on suggested items, PR review items, and in the session panel), sending `TodoAction::StartSession(key)`, plus unit test `start_session_label_tracks_running_sessions`.
- [todo_panel.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/workspace/src/todo_panel.rs) — passed `start_session = false` to `todo::footer` so the session To-do panel keeps its approved `History · Pin · Hide until…` footer (frames `07`, `12`, `191`).
- [todo_asks.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/native/todo_asks.rs) — added `starting_a_session_from_the_todo_drawer_opens_the_prefilled_new_session_dialog` (verifying `todo-start-session` is absent on `SUG-1` and `REV-1`, and clicking it on `PAY-7` opens the prefilled `Overlay::NewSession` dialog).
- [live_flows.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/live_flows.rs) — registered `#[path = "live_flows/todo_asks.rs"] mod todo_asks;`.
- [README.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/flows/cases/board/README.md) and cases [01](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/flows/cases/board/01-items-and-statuses.md)–[07](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/flows/cases/board/07-ownership.md), [09](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/flows/cases/board/09-agent-learns-the-board-exists.md)–[14](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/flows/cases/board/14-real-copilot-plans-a-task.md) — updated to describe the To-do screens and current tests (`08` left untouched).
- [BOARDSIMPLE.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/BOARDSIMPLE.md) — appended Gemini lane entry to the Lane log.

**New**
- [todo_asks.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/live_flows/todo_asks.rs) — three live real-daemon tests:
  - `approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`
  - `denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call`
  - `starting_a_session_from_the_todo_drawer_opens_it_and_moves_the_item_in_progress`

**Deleted**
- None.

---

### Seam lines for the main agent

1. **[features/todo/src/lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/todo/src/lib.rs#L434-L438)** — add `StartSession(String)` to `TodoAction` (required for `ui/workspace/src/todo.rs` to compile):
   - Anchor line 436: `    StartReview(String),`
   - Insert immediately after:
     ```rust
     /// Starts a new session for this item (by key). The app routes it to
     /// `AppAction::StartNewForItem`; the list does not change.
     StartSession(String),
     ```

2. **[features/todo/src/lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/todo/src/lib.rs#L583-L588)** — handle `TodoAction::StartSession(_)` in `Todo::reduce`:
   - Anchor line 585: `            | TodoAction::StartReview(_)`
   - Insert immediately after:
     ```rust
                 | TodoAction::StartSession(_)
     ```

3. **[features/app/src/lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/app/src/lib.rs#L1192)** — route `TodoAction::StartSession` to `AppAction::StartNewForItem`:
   - Anchor line 1192: `            AppAction::Todo(TodoAction::StartReview(key)) => self.start_todo_review(state, &key),`
   - Insert immediately after:
     ```rust
                 AppAction::Todo(TodoAction::StartSession(key)) => {
                     self.start_new_for_item(state, ItemId::new(key))
                 }
     ```

4. *(Recommended)* **[features/app/src/new_session.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/app/src/new_session.rs#L137-L147)** — fallback to `state.todo.items` when an item isn't in `state.workspace.items` (e.g., `INF-6`, `WEB-8`, `TD-3` in `FakeTodo::frame_b1_1()`), leaving existing `workspace.items` items (like `PAY-9` in golden `31-new-session`) untouched:
   - Anchor lines 138–146:
     ```rust
         let Some(item) = state
             .workspace
             .items
             .iter()
             .find(|item| item.id == id)
             .cloned()
         else {
             return Effect::none();
         };
     ```
   - Replace the `else` branch with:
     ```rust
         else if let Some(entry) = state
             .todo
             .items
             .iter()
             .find(|entry| entry.todo.key == id.as_str())
             .cloned()
         {
             let effect = self.reduce(state, AppAction::StartNewSession);
             if let Some(Overlay::NewSession(sheet)) = state.overlays.last_mut() {
                 sheet.title = entry.todo.title;
                 sheet.item = Some(id);
                 if let Some(channel) = entry.todo.channel.as_deref().and_then(|name| {
                     state
                         .workspace
                         .channels
                         .iter()
                         .find(|c| c.name == name || c.id.as_str() == name)
                         .map(|c| c.id.clone())
                 }) {
                     sheet.machine = state
                         .workspace
                         .channel_machines(&channel)
                         .find(|m| m.online)
                         .map(|m| m.id.clone());
                     sheet.channel = Some(channel);
                 }
             }
             return effect;
         } else {
             return Effect::none();
         };
     ```

---

### Focused Bazel targets to build and test

After applying the seam lines above:
- `//apps/humata2/features/todo:todo_test`
- `//apps/humata2/features/app:todo_route_test`
- `//apps/humata2/features/app:board_start_test`
- `//apps/humata2/ui/workspace:workspace_unit_test`
- `//apps/humata2/desktop:native_test` (specifically `--test_arg=todo_asks::` and `session::session_lifecycle_and_full_screen_reference_images`)
- `//apps/humata2/desktop:panels_test`
- `//apps/humata2/desktop:live_flows_cases` with `--test_arg=todo_asks::` (or `//apps/humata2/desktop:live_flows_test`)
- `//apps/humata2/desktop:humata2` (build check)

---

### Goldens and frames for Fable

- **No existing goldens should change:** `todo_panel.rs` passes `start_session = false`, keeping native frames `07-session-board-panel` and `12-panel-full-screen-board` unchanged, and `start_new_for_item` keeps `31-new-session` unchanged.
- **Design question for Fable:** `ui/overlays/src/overlay_new_session.rs` (outside this lane's claims) still renders the frame-31 New session sheet layout (`New session` + `in # <channel> · for <id>`, with `Skip to` when opened from Do next) rather than frame `b4-3`'s layout (`New session for “<title>”`, `To-do` row with `<key>` badge and `"moves to In progress when the session starts"`, and prefilled `FIRST MESSAGE` on `StartNewForItem`). Please confirm whether `overlay_new_session.rs` should be updated to frame `b4-3` (which will require re-recording native golden `31-new-session`) or kept as frame 31.

---

### What is still open

- Main agent needs to apply seam lines 1–3 (and optional 4) above and run the Bazel build/test targets (with `--config=fast` then default `--config=lint`).
- Decision from Fable/owner on whether `overlay_new_session.rs` should be redrawn to frame `b4-3` (and `31-new-session` re-recorded) and whether `StartNewForItem` should also prefill `first_message` the way `fill_do_next` does.
agy exit 0 for todolive

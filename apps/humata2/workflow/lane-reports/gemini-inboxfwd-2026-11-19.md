warning: --mode auto is not available for this account; continuing in the default mode.
### Files changed

- [registry.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_away/src/registry.rs) — Refactored `Forwarder::try_call` to take `principal: pb::forwarded_call::Principal`, added `Forwarder::call_as_user` sending `Principal::User(pb::MachineUser { presence: None, outside_sessions: true })`, and added unit test `a_user_call_goes_up_with_a_user_principal_and_gets_the_hosts_response`.
- [ports.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_board/src/ports.rs) — Added `Method::AskWhy` (`/humata.v1.InboxService/AskWhy`), `Method::ExplainAsk` (`/humata.v1.InboxService/ExplainAsk`), their `ROWS` entries, and `BoardInbox::{ask_why_as, explain_as}`.
- [test_support.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_board/src/test_support.rs) — Implemented `ask_why_as` and `explain_as` on `NoopInbox`.
- [replay.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_board/src/replay.rs) — Updated `run_forwarded` to dispatch `Method::AskWhy` for a trusted peer `Principal::User` with `outside_sessions == true` as `Caller::User(UserVia::Host)` (refusing in-session users with `Refusal::PresenceRequired` and agents with `Refusal::UserOnly { capability: Some(Capability::InboxAnswer) }`) and `Method::ExplainAsk` for an agent placed on the forwarding machine via `me.inbox.explain_as`. Added unit tests `by_name_finds_ask_why_and_explain_ask`, `forwarded_ask_why_allows_trusted_peer_user_and_refuses_agent_or_in_session_user`, and `forwarded_explain_ask_routes_for_placed_session_and_refuses_session_elsewhere`.
- [ports.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_inbox/src/ports.rs) — Added default method `InboxGuardPort::allowed_without_asking_today(&self) -> u32 { 0 }` for IN-8.
- [forwarded.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_inbox/src/inbox/forwarded.rs) — Added `ask_why_on_host`, `explain_on_host`, `ask_why_as`, and `explain_as`.
- [inbox.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_inbox/src/inbox.rs) — Updated `ask_why` (explicit `!matches!(caller, Caller::User(_))` guard returning `Refusal::UserOnly` while allowing `Caller::User(UserVia::Host)` alongside `App`/`CliLogin`, plus route-up via `ask_why_on_host` on `NotFound`) and `explain_ask` (route-up via `explain_on_host` on `NotFound`). Left `kind_of` untouched. Added unit tests `ask_why_refuses_agent_and_accepts_forwarded_host_user`, `explain_ask_routes_up_to_host_when_not_held_locally`, `ask_why_routes_up_to_host_when_not_held_locally`, `explain_ask_returns_host_unreachable_when_host_is_away_and_holds_request`, and `ask_read_builds_file_access_preview_and_rich_card_details`.
- [facts.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_inbox/src/inbox/facts.rs) — Handled `Preview::FileAccess` in `details()`, added `file_read_details` (resolving source session title/agent and channel name from `Store`, formatting `From`, `Files`, `Access` rows, calm `affects`, and `undo_text`), and added unit tests `a_file_read_preview_names_source_paths_access_mode_and_calm_effects` and `file_read_details_resolves_session_title_and_agent_from_store`.
- [deferred.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_inbox/src/inbox/deferred.rs) — Added `repo_path_of`, `file_read_preview`, and `Inbox::ask_read_reason`; updated `card()` to emit `Preview::FileAccess` + `facts::file_read_details` for every `Call::ReadFiles` ask.
- [lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/inbox/src/lib.rs) and [agent_inbox.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/inbox/tests/agent_inbox.rs) — Added `humata2_inbox::today_summary(answered, allowed_without_asking)` (omits the rule-followed clause when `0`, pluralizes when `> 0`) + unit test `today_summary_omits_zero_rule_count_and_pluralizes_nonzero_counts`.
- [inbox_page.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/workspace/src/inbox_page.rs) — Wired the empty Agent inbox summary line to `humata2_inbox::today_summary(answered, 0)` (no change to [request_card.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/workspace/src/request_card.rs), which already renders rich cards whenever `facts`/`affects` are non-empty).
- [INBOXHOOKS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/INBOXHOOKS.md) — Added the Batch 4 entry in the `Lane` section.

*(New files: none. Deleted files: none.)*

---

### Seam lines for the main agent

#### 1. Required for Task 1 to build (`wire_board.rs`)

- [wire_board.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/wire_board.rs#L149-L166) — Inside `impl BoardInbox for InboxAdapter`, after `withdraw_as`:
```rust
    async fn ask_why_as(
        &self,
        caller: Caller,
        request: pb::AskWhyRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let local = to_local_caller(&caller);
        self.0.ask_why_as(local, request).await
    }

    async fn explain_as(
        &self,
        caller: Caller,
        request: pb::ExplainAskRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let local = to_local_caller(&caller);
        self.0.explain_as(local, request).await
    }
```
- [wire_board.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/wire_board.rs#L181-L185) — Inside `away_method(method: Method)`, after `Method::WithdrawRequest => away::Method::WithdrawRequest,`:
```rust
        Method::AskWhy => away::Method::AskWhy,
        Method::ExplainAsk => away::Method::ExplainAsk,
```

#### 2. Task 2 (IN-8: "actions followed your rules without asking" end-to-end)

Allowed-without-asking agent decisions are judged inside `Guard::judge` in `daemon/humatad/src/guard.rs` and never reach `svc_inbox`, so wiring the live count end-to-end requires these seam lines:

- [inbox.proto](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/api/proto/humata/v1/inbox.proto#L601-L606) — In `ListRequestsResponse`, add field 4:
```protobuf
message ListRequestsResponse {
  // VIP first (NV-22), then oldest first.
  repeated InboxRequest requests = 1;
  uint32 pending_count = 2;
  PageResponse page = 3;
  // IN-8: agent actions allowed today by a channel rule, grant, desk switch
  // or review push setting without asking the user.
  uint32 allowed_without_asking_today = 4;
}
```
- [guard.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/guard.rs#L8-L11) — Add `Source` to the `humata2_access` import; in `struct Guard` ([guard.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/guard.rs#L34-L49)), add:
```rust
    /// IN-8: `(utc_day, count)` of agent actions allowed today by a rule/grant
    /// without asking.
    allowed_today: Mutex<(u64, u32)>,
```
  Initialize `allowed_today: Mutex::new((utc_day_now(), 0))` in `Guard::new` ([guard.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/guard.rs#L90-L102)).
  In `Guard::judge` ([guard.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/guard.rs#L237-L255)), before matching `verdict` (or inside the `Verdict::Allowed { source, .. }` arms), increment when `matches!(caller, Caller::Agent(_))` and `matches!(source, Source::Rule | Source::Grant(_) | Source::DeskSwitch | Source::ReviewPushSetting)` via a helper `self.record_allowed_today()` (rolling over when `utc_day_now() != day`). Expose:
```rust
    /// IN-8: how many agent actions followed rules or grants without asking today (UTC day).
    pub fn allowed_without_asking_today(&self) -> u32 {
        let today = utc_day_now();
        let state = lock(&self.allowed_today);
        if state.0 == today { state.1 } else { 0 }
    }
```
  where `fn utc_day_now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) }`.
- [wire_inbox.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/wire_inbox.rs#L137-L145) — Inside `impl InboxGuardPort for GuardAdapter`, add:
```rust
    fn allowed_without_asking_today(&self) -> u32 {
        self.0.allowed_without_asking_today()
    }
```
- [inbox.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_inbox/src/inbox.rs#L926-L930) — In `InboxService::list_requests`, pass the new proto field once generated:
```rust
        Ok(Response::new(pb::ListRequestsResponse {
            requests,
            pending_count,
            page: None,
            allowed_without_asking_today: self.guard.allowed_without_asking_today(),
        }))
```
- [workspace.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/src/workspace.rs#L36-L38) — Add to `pub struct Workspace`:
```rust
    /// IN-8: agent actions allowed today by rules or grants without asking.
    pub allowed_without_asking_today: u32,
```
- [inbox.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/live_daemon/src/inbox.rs#L645-L674) — Change `load_requests(&self)` to return `Result<(Vec<Request>, u32), DaemonError>`, capturing `let mut allowed_without_asking_today = 0; ... allowed_without_asking_today = reply.allowed_without_asking_today;`, and update [daemon.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/live_daemon/src/daemon.rs#L220-L243) to destructure `let (requests, allowed_without_asking_today) = self.load_requests().await?;` and set `allowed_without_asking_today` on `Workspace`.
- [inbox_page.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/workspace/src/inbox_page.rs#L164-L165) — Replace `0` with `state.workspace.allowed_without_asking_today`:
```rust
                .child(humata2_inbox::today_summary(answered, state.workspace.allowed_without_asking_today))
```

#### 3. Optional enhancement for Task 3 (`ReadFileRequest.reason` forwarding)

`FileAccessPreview` and rich `facts`/`affects`/`undo_text` already work for every file/folder read ask without any seam. To also display `ReadFileRequest.reason` (already field 4 in [files.proto](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/api/proto/humata/v1/files.proto#L90)) on the card:

- [ports.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_files/src/ports.rs#L44-L52) — Add a default method `ask_read_reason` on `FileInboxPort` (defaulting to `ask_read`), implement it in [wire_files.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/humatad/src/wire_files.rs#L55-L65) by calling `self.0.ask_read_reason(caller, asked, operation, reason, run).await`, and thread `request.reason` through `Reads::require_with_reason` in [lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_files/src/lib.rs#L74-L85), [lib.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_files/src/lib.rs#L491-L511), and [forward.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_files/src/forward.rs#L247-L261) / [forward.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/daemon/svc_files/src/forward.rs#L533-L541).

#### 4. Decision log note (`DECISIONS.md`)

- [DECISIONS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/DECISIONS.md#L5111-L5115) — Append under `D871`:
```markdown
- Batch 4 (2026-11-19):
  - Host-side forwarding of `AskWhy` and `ExplainAsk`: `svc_away::Forwarder::call_as_user` carries `Principal::User(MachineUser { outside_sessions: true })`; `svc_board` replays `AskWhy` as `Caller::User(UserVia::Host)` (refusing agents and in-session users) and `ExplainAsk` for sessions placed on the forwarding machine; `svc_inbox` routes missing `AskWhy`/`ExplainAsk` up to the host and allows `Caller::User(UserVia::Host)` on `AskWhy`.
  - File-read asks (`Call::ReadFiles`) now build `FileAccessPreview` with `From` (resolved session title/agent or channel name), `Files`, `Access`, calm `affects`, and `undo_text`.
  - IN-8 count formatting lives in `humata2_inbox::today_summary` (omitting the rule-followed clause when `0`) pending the `ListRequestsResponse.allowed_without_asking_today` proto + `Guard` counter seam.
```

---

### Focused Bazel targets to build and test

Unit tests for the changed crates (can run immediately after applying seam 1):
- `//apps/humata2/daemon/svc_away:svc_away_unit_test`
- `//apps/humata2/daemon/svc_board:svc_board_unit_test`
- `//apps/humata2/daemon/svc_inbox:svc_inbox_unit_test`
- `//apps/humata2/features/inbox:agent_inbox_test`

Downstream compile / integration checks (after applying seam 1, and again after seam 2 if landed together):
- `//apps/humata2/ui/workspace:workspace`
- `//apps/humata2/daemon/humatad:humatad2_test_away_board`
- `//apps/humata2/daemon/humatad:humatad2_test_machines`

---

### Goldens and frames

- **Goldens to re-record**: none newly shifted by Batch 4 (`today_summary(answered, 0)` keeps the exact empty Agent inbox text until non-zero IN-8 counts are populated in fixtures).
- **Frames Fable must judge**: none newly added in Batch 4; Batch 3 frames **206** and **209** remain open for re-render and Fable re-judge.

---

### What is still open

1. Applying seam 1 (`wire_board.rs`) so `humatad` compiles against the new `BoardInbox` methods and `Method` variants.
2. Applying seam 2 (`inbox.proto`, `guard.rs`, `wire_inbox.rs`, `svc_inbox/src/inbox.rs`, `clients`, `inbox_page.rs`) to feed the live IN-8 count into the empty Agent inbox page (and optionally persisting the daily counter across daemon restarts if desired; the seam above tracks the current UTC day in memory).
3. Applying seam 3 (`svc_files` + `wire_files.rs`) if `ReadFileRequest.reason` should populate the file-read card's reason line.
4. Building, running the focused tests above, and re-judging Batch 3 frames 206 and 209 with Fable.
agy exit 0 for inboxfwd

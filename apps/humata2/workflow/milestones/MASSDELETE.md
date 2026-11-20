# MASSDELETE: I can delete many sessions at once, or all of them

- **Status:** open (owner, 2026-11-19: "add a 'nuke everything' command where we delete all sessions or we can mass delete sessions")
- **Depends on:** nothing. Builds on the one-session delete (`PlanDelete` + `DeleteSession` with a plan token in `api/proto/humata/v1/sessions.proto`; `humata2 sessions delete` in `cli/src/commands/sessions.rs`).
- **Unblocks:** nothing.

## Outcome
I can pick several sessions (or a whole channel, a machine, or everything) and delete them in one go. Before anything goes, I see one plan listing what will be lost. "Delete everything" exists, but it is hard to hit by accident.

## Exit
- **Daemon:** a bulk plan RPC returns one plan covering every chosen session. It lists running terminals that will end, unpushed work, and forks that go with their parent, and it carries one token. A bulk delete RPC takes that token and deletes exactly the planned set. A session that changed since the plan (new commits, a new terminal) refuses the whole delete with the reason, so nothing is half-done. It works across machines; offline machines are listed as "can't reach, left alone".
- **CLI:** `humata2 sessions delete <id>...`, `--channel <c>`, `--machine <m>`, and `--all`. Each prints the plan and asks for confirmation; `--all` requires typing the session count. `--yes` skips the prompt for scripts, except with `--all`.
- **Desktop:**
  - A multi-select mode in the sessions lists (sidebar, Home, the machine page) with a "Delete N sessions…" action.
  - A "Delete all sessions…" entry in the palette and in Settings.
  - Both open one confirmation sheet showing the plan. "Delete everything" needs the count typed.
- **Agents:** the bulk path never runs for agents by default. It sits behind its own Agent access switch, off by default (destroys off by default, D-rule from humata-agent-access), and is never offered to MCP unless that switch is on.
- **Running terminals** end only because the owner confirmed this plan (sessions-accountable / terminals-keep-running rules). Nothing else ends them.
- **Tests:** fake-daemon unit tests for planning and the stale-plan refusal; one Tier-B daemon test on the narrowest profile; native and design frames for the select mode and the sheet, judged by Fable 5.2.

## Pointers
- One-session delete: `daemon/humatad/src/session_service.rs` (PlanDelete/DeleteSession), `features/app` delete flow, `ui/overlays`.
- Agent access rows: `features/settings/src/access_rows.rs` (`session_delete_other`).
- The design goes to Fable 5.2 (copy signed-off frames fully); log decisions in `DECISIONS.md`.

## Out of scope
- Deleting machines, channels or repos. Undo after the delete (the plan is the safeguard; archive is the undoable path).

## Lane
Manager: Opus 5.5 (MASSDELETE), 2026-11-19. **Batch 2 running (seams, T4, T5, PER-32, restore precheck, T7, T12, T9); T10, T11, T13 after T9.** Main's go for batch 2 (2026-11-19): all 16 seams are mine to apply; channel_service/delete.rs is mine (LIVEHOLDER closed); visible CLI count line approved; DIFFUI and INBOXHOOKS are idle (keep their lists.rs Home-group hunks).

### What exists today (read end to end)
- `sessions.proto` has `PlanDelete`/`DeleteSession` (one id, `DeletePlan{id, unpushed, token}`), but **the one-session delete was never built**: `session_service/archive.rs::delete_of` runs the access gate (own: `OwnSessionEnd`; other: `SessionDeleteOther`, Never) and then answers `not_built`. `svc_sessions_local` and the peer fake answer not_built too. So `humata2 sessions delete <id>` (`cli/src/commands/sessions.rs`, `ctx.planned` + `confirm`) and the MCP `session_delete` tool reach a not_built after the gate.
- The desktop has **no** session delete at all (no client method, no feature, no overlay). The closest pieces: Desk Clean up (`features/desk/src/clean_up.rs`, `ui/overlays/src/overlay_desk_clean_up.rs`, `desk.proto` `PlanDeskCleanUp`/`CleanUpDesk`, which answers `changed` with the new plan, NV-27), the Archive sheet (`overlay_archive.rs`, frame 65), and frame 71 (delete channel).
- The built delete to copy: `daemon/humatad/src/channel_service/delete.rs` (token store with a 10 min life, `Listing` + `changes_since`, `stop_all`, inbox withdraw, `end_grants`, side file, `Bin::put`, intent note under `.deleting`, restart recovery, fault points). Store has `export_session`/`import_session` (session_snapshot.rs) and `remove_session`.
- So MASSDELETE builds the session delete itself, bulk-first; the one-id RPCs become a one-item bulk call.

### Goal and exit
As the milestone's Exit above. Done when: the daemon's bulk plan/delete works locally and across machines with the stale-plan refusal; the CLI flags work; the desktop select mode + sheet match Fable's frames; the agent switch is off by default and hides the MCP tool; the tests below pass.

### Design notes
**Proto (`api/proto/humata/v1/sessions.proto`, additive; seam lines for main):**
```proto
// MASSDELETE: one plan for many sessions (picked ids, a channel, a machine
// or all), one token, one delete. LD-5: each session's folder and records
// go to its machine's trash. NV-27: a delete whose sessions changed since
// the plan deletes nothing and answers the new plan.
rpc PlanDeleteSessions(PlanDeleteSessionsRequest) returns (DeleteSessionsPlan);
rpc DeleteSessions(DeleteSessionsRequest) returns (DeleteSessionsResponse);

message DeleteScope {
  message Picked { repeated SessionId ids = 1; }
  oneof scope {
    Picked sessions = 1;
    ChannelId channel = 2;
    MachineId machine = 3;
    // Every channel session, live and archived. Desk sessions are not in
    // it (the Desk's Clean up deletes them).
    bool all = 4;
  }
}
message PlanDeleteSessionsRequest { DeleteScope scope = 1; }
message SessionToDelete {
  SessionId session = 1;
  string title = 2;
  ChannelId channel = 3;
  MachineId machine = 4;
  bool archived = 5;
  repeated UnpushedRepo unpushed = 6;
  // Agent and shell terminals that end because of this delete (PER-32:
  // the owner confirming this plan is what ends them).
  uint32 running_terminals = 7;
  // Set for a context-only fork pulled in because it works in this
  // parent's folder (D187).
  SessionId goes_with = 8;
}
message LeftAlone {
  MachineId machine = 1;
  // From the host's last pointers (SL-3).
  repeated SessionWork sessions = 2;
}
message DeleteSessionsPlan {
  repeated SessionToDelete sessions = 1;
  // Offline machines: their sessions are not deleted.
  repeated LeftAlone left_alone = 2;
  // Code forks of a deleted session: they keep their folder and run on.
  repeated SessionWork forks_staying = 3;
  uint32 desk_sessions_left_out = 4;
  PlanToken token = 5;
  // Scope all: DeleteSessions needs typed_count equal to sessions.size().
  bool needs_count = 6;
}
message DeleteSessionsRequest {
  IdempotencyKey idempotency_key = 1;
  PlanToken plan = 2;
  bool delete_unpushed_work = 3;
  uint32 typed_count = 4;
}
message DeleteSessionsResponse {
  message Deleted {
    uint32 sessions = 1;
    repeated TrashItemId trash_items = 2;
    Operation operation = 3;
  }
  message Changed {
    DeleteSessionsPlan plan = 1;
    // In words, e.g. "session \"Ledger\" now has 2 unpushed commits".
    repeated string changes = 2;
  }
  oneof result { Deleted deleted = 1; Changed changed = 2; }
}
```
Also: `operations.proto` `OPERATION_KIND_DELETE_SESSIONS = 22;`, `capabilities.proto` `CAPABILITY_SESSIONS_DELETE_MANY = 74;`.

**Daemon (`daemon/humatad`, new file `session_service/delete_many.rs` plus two arms):**
- Plan: resolve the scope on the host from the store (SL-1); pull in context-only forks of every picked session (`goes_with`); list code forks as staying; per local session read git work as `channel_service::delete::list` does (FETCH_HEAD cleanup kept), count running terminals (agent holder + shell tabs, socket answers); for sessions on an online remote machine relay `PlanDeleteSessions{Picked}` over the peer link and keep that machine's token; sessions on an offline machine go to `left_alone` from the host's pointers. The listing is kept in a token store (10 min life, one use) like channel delete's `State`.
- Delete: peek the token; re-list everything; `changes_since` non-empty answers `Changed` (token spent, nothing touched). Work without `delete_unpushed_work` is a FailedPrecondition naming the session (as channel delete). Scope all checks `typed_count`. Then a check pass on every remote machine (relay with its token, `check_only`) before any session is touched; then local sessions, then each remote machine.
- Per session (local), each step re-runnable, an intent note at `<root>/.deleting-sessions/<id>` first, restart recovery as channel delete's: stop agent + end shells (`stop_all`), withdraw pending inbox requests (`WithdrawReason::SessionDeleted`, new enum value if missing), end grants, `export_session` to the side file in the session folder (a session whose folder is gone gets a fresh folder holding only the side file), `Bin::put` with `was = Session`, then `remove_session`. Publish `Change::Deleted` per session. One `Operation` with steps "Ending terminals", "Withdrawing requests", "Moving to the trash".
- Trash restore: `trash_service.rs` restores a `was = Session` item by `import_session` from its side file as an archived session (LD-7), so "30 days in the trash" is true.
- `PlanDelete`/`DeleteSession` (one id) call the same code with a one-id scope, keeping their own gate (`OwnSessionEnd` / `SessionDeleteOther`). `session_delete_other.rs` expectations that read not_built move to a real delete.
- Cross-machine atomicity: a check pass on every machine before any delete; a machine that fails after the checks leaves its sessions and the error names which went (Provisional, D876).

**Access:**
- `Capability::SessionsDeleteMany`, tier NeverByDefault, needs_host, label "delete many sessions at once". The bulk RPCs from an agent need it; scope `all` is user-only and present (as ChannelDelete, CH-25), never an agent's. An agent's plan never includes its own session. A plan that ends running terminals always goes to the owner's Agent inbox even under Allow (PER-32: the owner ends terminals).
- Agent access row (features/settings/src/access_rows.rs, group FILES AND SESSIONS): key `sessions-delete-many`, "Delete many sessions at once", detail "Never ends a running terminal without asking you", capabilities `["sessions_delete_many"]`, options ALL, default Never.
- MCP: new `Operation::SessionsDeleteMany` (`sessions_delete_many`, `session.delete_many`, Effect::Remove). `tools/list` offers it only while the caller's rule is not Never: the MCP server keeps an offered-set refreshed from `AccessService.Explain` at `initialize` and on each `tools/list`, and sends `notifications/tools/list_changed` when it flips (Provisional, D876).

**CLI (`cli/src/commands/sessions.rs`, `cli/src/args.rs`):**
`humata2 sessions delete [<id>...] [--channel <c>] [--machine <m>] [--all] [--delete-work] [--count N]` (exactly one scope). Prints the plan (human form groups by channel; terminals that end, work lost, forks going along, machines left alone) then confirms. `--yes` skips the prompt except with `--all`; `--all` asks the user to type the count; non-interactive `--all` needs `--count N` equal to the planned count (the daemon checks it too). `--dry-run` prints the plan only. A `Changed` answer prints what changed and exits with the stale-plan exit code (CLI.md; new code only if none fits). Needs presence for `--all` (SEC-10).

**Clients (TCA split):** new `clients/src/mass_delete.rs` (types `DeleteScope`, `DeleteSessionsPlan`, `SessionToDelete`, `DeleteSessionsOutcome::{Deleted, Changed}`), two default-NotServedYet `Daemon` methods `plan_delete_sessions(scope)` / `delete_sessions(token, delete_work, typed_count)`; fake in new `clients/src/daemon/fake/mass_delete.rs` (deterministic, with a `change_after_plan` hook for the stale path); live in new `clients/live_daemon/src/mass_delete.rs`; parity cases in `clients/live_daemon/tests/mass_delete.rs`.

**Features:** new crate `features/mass_delete` (`Selection` for the sidebar/Home/machine page: surface, ticked set, anchor for shift ranges, select channel/machine/all; `DeleteSheet` state: loading, plan, delete-work tick, typed count, changed note, deleting, error) with TestStore tests. App wiring: `AppAction::MassDelete`, `AppState.{selection, delete_sheet}`, palette command "Delete all sessions…", Settings entry, keyboard (seam lines for main).

**UI:** per Fable's frames (pending): select-mode hunks in the three lists, an action bar, `ui/overlays/src/overlay_delete_sessions.rs` (the sheet), the Settings entry, design frames in new `desktop/tests/design/mass_delete.rs`, native frames in new `desktop/tests/native/mass_delete.rs`.

### Fable's frames and strings (D875)
Frames: `mockups/massdelete/` m1-1 sidebar select, m1-2 the sheet for a picked set, m1-3 delete-all empty, m1-4 stale plan, m2-1 Home select a channel, m2-2 machine page select all, m2-3 palette, m2-4 Settings > Storage (dark), m2-5 delete-all typed (dark), m2-6 after delete. Copy them exactly.

Strings (verbatim):
- **Entries:** `Select` · `Done` · `Select sessions…` · `Delete N sessions…` · `Delete all sessions…` · `Select all on {machine}` · `{k} of {n}`
- **Palette notes:** `Tick sessions, channels or machines, then delete them` · `{N} sessions · shows the plan first`
- **Bar and card:** `{N} selected` · `{p} picked · 1 fork goes along` / `… {k} forks go along` · `All of # {channel}` · `Every session on {machine}` · `{title} is not on Home` · `Cancel`
- **Key hint:** `⌘A all · ⇧-click a range · space ticks · esc cancels`
- **Select menu:** `Every session in a channel` · `Every session on a machine` · `Everything on Home` · `Nothing` · `offline`
- **Settings:**
  - Section `Sessions`, title `Delete all sessions`.
  - Note: `Every live and archived session on every machine that can be reached: {N} now. You see the full plan first and type the count to confirm. Desk sessions are not part of this.`
  - Below it: `To delete only some, tick them in the sidebar, on Home or on a machine’s page: ⌘-click a session, or use Select.`
- **Sheet titles:** `Delete {N} sessions?` · `Delete 1 session?` · `Delete all {N} sessions?`
- **Scope line:** `In {c} channels, on {machines}.` · `Every session, live and archived: {N} in {c} channels, on {machines}.`
- **Totals:** `{t} running terminals end, in {s} sessions.` · `{w} sessions hold work that was never pushed.` · `1 fork goes with its parent. {k} forks stay.` · `{machine} can’t be reached. Its {k} sessions are left alone.`
- **Group head:** `# {channel}` + `{k} of {n} sessions` (picked) or `{n} sessions` (all)
- **Row:**
  - Columns: `{Agent} · {machine}` · `live` / `archived`.
  - Terminals: `Ends {k} running terminals: claude (working), zsh.` (singular `Ends 1 running terminal: …`; states `working`, `waiting for you`, `idle`).
  - Work: `{repo}: 3 uncommitted files on {branch}, never pushed.` · `{repo}: 2 commits on {branch} never pushed.` · `{repo}: 1 commit on {branch} not pushed.`
  - Box: `Delete that work too`.
  - Forks: `Goes with {parent}: this fork works in the same folder.` · `{A} and {B} stay. These forks have their own folders; they move up in the tree.`
  - Empty: `Nothing running. Nothing unpushed.`
- **Folded:** `{k} archived, nothing unpushed: {names}` · `{k} archived, nothing unpushed · Show` · `{k} more, nothing running and nothing unpushed: {names} · Show`
- **Unreachable group:** `{machine}` `can’t reach, left alone · {k} sessions` · `Not deleted: {names}. Delete them when {machine} is back.`
- **Desk line:** `Desk sessions are not part of this. The Desk has its own Clean up.`
- **Trash box:** `The terminals end for good. The sessions go to the trash on {machines} for 30 days: restore one from the machine’s Storage > Trash and it comes back archived, with its work. There is no Undo.`
- **Count field:** `Type the number of sessions, {N}, to confirm`
- **Footer hints:** `Tick “Delete that work too” for {title} to continue` · `Tick “Delete that work too” for {k} sessions and type {N} to continue` · `Type {N} to continue` · `Return deletes`
- **Buttons:** `Cancel` · `Delete {N} sessions` · `Delete all {N} sessions`
- **Stale:**
  - Notice: `Something changed since this plan. Nothing was deleted.`, then one line per change, then `This is the new plan. Check it and delete again.`
  - Change lines: `{title}: 1 new commit on {branch}.` · `{title}: a new terminal started (zsh).` · `{title}: a session started.` · `{title}: its work was pushed.`
  - Tag `changed`; inline `(new)` / `(1 new)`.
- **Toast:** `Deleted {N} sessions. They are in the trash on {machines} for 30 days.` With unreachable machines, add `{k} on {machine} were left alone.`

### Tasks (failing test first; each worker runs only its target)
| # | Task | Files | Test (written first) | Worker |
|---|---|---|---|---|
| T0 | Proto + capability + op kind seam lines | script for main: sessions/operations/capabilities.proto | builds | main applies |
| T1 | Access: capability, tier, label, default Never; scope-all user-only | `daemon/access/src/{capability,policy}.rs`, `daemon/access/tests/sessions_delete_many.rs` (new) | defaults Never; Ask asks; agent never all | Haiku |
| T2 | Daemon plan + stale + delete, local only | new `daemon/humatad/src/session_service/delete_many.rs`; arms in `session_service.rs` (2 handlers + `mod`), `archive.rs` `delete_of` → one-id bulk; `daemon/humatad/tests/delete_sessions.rs` (new, Tier B `humatad2_test_power_cut`) + BUILD row | plan lists terminals/work/ctx fork; changed → nothing deleted; work blocks; all needs count; folder in trash, records gone | Sonnet (loosely scoped) |
| T3 | Restore a deleted session from the trash, archived | `daemon/humatad/src/trash_service.rs` (session arm), same test file | restore → archived session with its record | Haiku |
| T4 | Restart finishes a cut delete | `delete_many.rs` recovery + one fault point; `tests/delete_sessions_cut.rs` (new, `_faults` twin) | cut after intent → restart completes | Haiku |
| T5 | Across machines: relay, offline left alone, check pass | `delete_many.rs`, `svc_peer` relay arm if needed, `tests/delete_sessions_remote.rs` (new, `remote_rig`, `_release` twin) | remote session deleted on its machine; offline listed, untouched | Sonnet |
| T6 | CLI flags, prompts, count | `cli/src/args.rs` (Delete variant), `cli/src/commands/sessions.rs` (Delete arm + new `sessions_delete.rs` helper), `cli/tests/sessions_delete.rs` (new) | --all without count refused non-interactive; --yes ignored with --all; one scope only | Haiku |
| T7 | MCP tool + offered only when on | `daemon/mcp/src/tools.rs` seam lines (main), `daemon/svc_mcp/src/mcp.rs` (tool + offered set) | tools/list hides it under Never, shows it under Ask | Haiku |
| T8 | Clients: types, trait methods, fake, live | new `clients/src/mass_delete.rs`, `clients/src/daemon/fake/mass_delete.rs`, `clients/live_daemon/src/mass_delete.rs`; seam lines in `clients/src/lib.rs`, `clients/src/daemon.rs`, `fake.rs`, live `state.rs`; `clients/live_daemon/tests/mass_delete.rs` | fake plan/stale; live parity | Haiku |
| T9 | Feature reducer: selection + sheet | new `features/mass_delete/**` | TestStore: tick/shift range/select all/escape; sheet: count gate, work tick, changed redraws, success toast | Haiku |
| T10 | UI: select mode in sidebar, Home, machine page; action bar | hunks in `ui/workspace/src/sidebar.rs`, `ui/src/lists.rs` (home rows), `ui/machines/src/machine_page.rs` (sessions list + menu item only) | native frames | Sonnet |
| T11 | UI: the sheet + palette + Settings entry | new `ui/overlays/src/overlay_delete_sessions.rs`; `features/palette` command; settings entry file per Fable | design + native frames | Haiku |
| T12 | Access row | `features/settings/src/access_rows.rs` one row; `features/settings/tests` case | row reads Never by default | Haiku |
| T13 | Frames | new `desktop/tests/design/mass_delete.rs`, `desktop/tests/native/mass_delete.rs`; frame lines in `desktop/tests/design.rs` (main) | Fable judges against `mockups/massdelete/` | Sonnet |

Order: T0 → (T1, T2, T6, T8, T9 in parallel; files don't overlap) → T3, T4, T5, T7, T12 → T10, T11 → T13.

### Tests per tier
- **Unit / in-process (presubmit):** `daemon/access` (T1); `features/mass_delete` TestStore (T9); clients fake (T8); `features/settings` row (T12); CLI arg parsing and prompt rules (T6); MCP list filter (T7).
- **Tier B daemon (`humatad2_test_power_cut`):** `delete_sessions` (plan, stale refusal, work blocker, count gate, trash + restore, agent refusals); `delete_sessions_cut` (`_faults`); `delete_sessions_remote` (`_release`, remote_rig; may go to slow.txt if over 60 s).
- **Live parity:** `clients/live_daemon/tests/mass_delete.rs` on `humatad2_test_power_cut`.
- **Frames:** design + native for the select mode and the sheet, judged by Fable 5.2 against `mockups/massdelete/`.
- No sleeps ≥20 ms; wait on events/operations; own temp roots.

### Claims (asked for; not yet granted)
New files (no conflict): `daemon/humatad/src/session_service/delete_many.rs`, `daemon/humatad/tests/delete_sessions{,_cut,_remote}.rs`, `daemon/access/tests/sessions_delete_many.rs`, `cli/src/commands/sessions_delete.rs`, `cli/tests/sessions_delete.rs`, `clients/src/mass_delete.rs`, `clients/src/daemon/fake/mass_delete.rs`, `clients/live_daemon/src/mass_delete.rs`, `clients/live_daemon/tests/mass_delete.rs`, `features/mass_delete/**`, `ui/overlays/src/overlay_delete_sessions.rs`, `desktop/tests/{design,native}/mass_delete.rs`, `workflow/milestones/mockups/massdelete/**`.
Existing files (hunks only):
- `daemon/humatad/src/session_service.rs` (plan_delete/delete_session handlers + two new handlers + `mod delete_many`), `session_service/archive.rs` (`delete_of` only), `trash_service.rs` (Session restore arm), `daemon/humatad/BUILD.bazel` (three test rows).
- `daemon/access/src/{capability,policy}.rs` (new capability).
- `cli/src/args.rs` (`SessionCommand::Delete`), `cli/src/commands/sessions.rs` (Delete arm).
- `daemon/svc_mcp/src/mcp.rs` (new tool + offered set; **INBOX claims svc_mcp**).
- `features/settings/src/access_rows.rs` (one row; **INBOX claims Agent access settings**).
- `ui/workspace/src/sidebar.rs`, `ui/src/lists.rs`, `ui/machines/src/machine_page.rs` sessions list (**DIFFUI claims `ui/**`; LIVEHOLDER/PROGRESS hold other machine_page hunks**).
- `features/palette/src/lib.rs` (one command), the Settings entry file (per Fable).
Seam lines for main (main-only files): the three protos, `daemon/mcp/src/tools.rs`, `clients/src/lib.rs`, `clients/src/daemon.rs` + `fake.rs` mod lines, `features/app/src/lib.rs` (AppAction/AppState), `ui/src/lib.rs`/`shell.rs` overlay layer, `desktop/BUILD.bazel`, `desktop/tests/design.rs` frame lines, CLI registration, Cargo/MODULE for the new crate.

### Workers
Main's go (2026-11-19): release done; owned: sessions.proto, session_service/ (delete*, archive.rs), cli/src/commands/sessions.rs, trash_service.rs, new overlay/feature files; access_rows.rs one row only; leave ui/overlays/src/overlay_new_session.rs alone (BOARDSIMPLE).
T0 proto: written by the manager (sessions.proto rpcs + messages). operations.proto / capabilities.proto lines go to main.
| Worker | Task | Files | Status |
|---|---|---|---|
| W-daemon (Sonnet medium) | T2 + T3 | session_service/delete_many.rs (new), session_service.rs handlers + mod, archive.rs delete_of, channel_service/delete.rs (pub(crate) + `repos_listed` extracted from list() + `trash_bin()`), trash_service.rs (Session arm, `finish_restore_sessions`), tests/delete_sessions.rs (new), session_delete_other.rs (2 asserts), BUILD POWER_CUT_SCOPED_TESTS row | written, reviewed; manager added the policy `Target::Global` arm. Seams: operations.proto 22, channel_service.rs `pub(crate) mod delete;`, serve.rs recovery call. Open: T4 cut test, T5 relay, PER-32 always-ask |
| W-access (Haiku high) | T1 | daemon/access/src/{capability,policy}.rs, humatad/src/{guard,access_service}.rs, daemon/access/tests/sessions_delete_many.rs (new) | written, reviewed; needs main's seam lines (capabilities.proto 74, mcp tools.rs capability_name, svc_inbox RequestKind::Session arm, fixture rules.rs, refusals.rs NeverByDefault count 5→6) |
| W-clients (Haiku high) | T8 | clients/src/mass_delete.rs, clients/src/daemon/fake/mass_delete.rs, clients/live_daemon/src/mass_delete.rs (all new) | written, reviewed; seam lines in api.rs, daemon.rs, fake.rs, live lib.rs/daemon.rs (`unpushed` pub(crate)), live BUILD srcs, plus a crate-test target for the fake's unit tests |
| W-cli (Haiku high) | T6 | cli/src/commands/sessions.rs Delete arm + mod line, cli/src/commands/sessions_delete.rs (new, unit tests inside) | written, reviewed; seam lines in args.rs (Delete variant), cli/src/daemon.rs (2 rpc names + ALL), call.rs READ_ONLY (PLAN_DELETE_SESSIONS). Follow-up: typed count reads via console.secret (echo off); a visible line read needs a Console method (main's file) |
| W-seams (Haiku high) | 16 seams + Console::line | operations/capabilities.proto, svc_inbox inbox.rs (one arm), fixture/rules.rs, access tests/refusals.rs, humatad BUILD (scope map), channel_service.rs, serve.rs, cli args/daemon/call/console.rs + sessions_delete.rs, clients api/daemon/fake/BUILD, live_daemon lib/daemon/BUILD, cli/tests/support/mod.rs (FakeConsole.line) | done; humatad BUILD scope map not needed (keyed by test file); args.rs uses ids delete_channel/delete_machine to avoid the global flags |
| W-daemon2 (Sonnet medium) | T4, T5, PER-32, restore precheck | delete_many.rs (relay modelled on archive_remote: plan per machine, check_only pass, then delete), trash_service.rs (precheck; channel carried in the item data), svc_peer/src/relay.rs (allowlist), tests/delete_sessions{,_cut,_remote}.rs, BUILD (RIG_TESTS, TEST_NEEDS, POWER_CUT_SCOPED_TESTS) | done; also fixed trash items naming the scope instead of each session. Manager added the PlanDeleteSessions/DeleteSessions stubs in svc_sessions_local/src/lib.rs and svc_peer/src/remote_work.rs (test Machine) |
| W-mcp (Haiku high) | T7 + T12 | mcp/src/{tools,server}.rs, svc_mcp/src/mcp.rs + new mcp/delete_many.rs, features/settings access_rows.rs one row + tests/access_rows.rs one test | done; manager added `sessions_delete_many` to daemon/mcp/tests/tools.txt and a row to api/MCP.md. Rule is learned at connect (no list_changed: no rules-changed event reaches the MCP process); agents can't pass delete_unpushed_work (work blocks, by choice) |
| W-feature (Sonnet medium) | T9 | features/mass_delete/** (new; + EnterEmpty) | done; app wiring lines for main |
| W-ui-lists (Sonnet medium) | T10 | ui/workspace/src/sidebar.rs, ui/src/lists.rs, ui/machines/src/machine_page.rs (sessions list), their BUILD/Cargo deps | running |
| W-ui-sheet (Sonnet medium) | T11 | ui/overlays/src/overlay_delete_sessions.rs (new), overlays lib/root/palette hunks, features/palette, ui/settings/src/settings.rs Storage card, deps | running |

### Handoff state (2026-11-19, for the next manager)

The work moved to another manager on the owner's call. Nothing below has been built or tested. Main runs Bazel, so the first job is a build of the targets listed at the end, followed by fixes.

| Task | State |
|---|---|
| T0 seams (all 16) | Applied. The humatad BUILD scope map was not needed. |
| T1 access | Written. Includes the `(SessionsDeleteMany, Target::Global)` arm in `policy.rs`. Test: `daemon/access/tests/sessions_delete_many.rs`. |
| T2 plan/stale/delete | Written: `session_service/delete_many.rs`, handlers, and `archive.rs` `delete_of`. Test: `delete_sessions.rs`. The other SessionService impls got stubs (`svc_sessions_local` not_built, `svc_peer/remote_work.rs` test Machine). |
| T3 restore | Written in `trash_service.rs`, with the restore precheck. |
| T4 restart recovery | Written. `recover_session_deletes` runs from `serve.rs`. Test: `delete_sessions_cut.rs`. |
| T5 remote | Written, modelled on archive_remote, with the `svc_peer/relay.rs` allowlist. Test: `delete_sessions_remote.rs`. The box-id assumption is unchecked. |
| PER-32 | Written: an agent plan that ends terminals always asks. |
| T6 CLI | Written: `cli/src/commands/sessions_delete.rs`, plus the Console `line` method and FakeConsole. |
| T7 MCP | Written: `svc_mcp/src/mcp/delete_many.rs`, `tools.rs`, `server.rs` (tools/list awaits bind), and the `tools.txt` and `MCP.md` rows. |
| T8 clients | Written: types, fake and live. |
| T9 feature | Written: `features/mass_delete/**` with TestStore tests `selection_test` and `sheet_test`. |
| T10 select mode | **Partial.** Home (`ui/src/lists.rs`) and the machine page are done, with shared helpers in `ui/machines/src/machine_page/select_mode.rs`. The sidebar (`ui/workspace/src/sidebar.rs`) is untouched; its helpers (`tick_box`, `tint`, `card`, `row_click`, `channel_click`/`machine_click`) are ready in `select_mode.rs`. Keys (⌘A, space, arrows, Return, ⌘⌫, Esc) are not wired. The row context-menu item is not added. |
| T11 sheet/palette/Settings | Written: `ui/overlays/src/overlay_delete_sessions.rs`, `overlays.rs` dispatch, `overlay_palette.rs` icons, `features/palette` `SelectSessions`/`DeleteAllSessions`, and the Settings `sessions_block`. The gpui names are unchecked (`IconName::Trash`/`ListChecks`, `Role::Button`, `border_1`). The Show-expanded set is not cleared when the sheet closes. |
| T12 access row | Written, with a test. |
| T13 frames | Not started. |

**The tree does not compile until the `features/app` wiring lands.** `features/app` is main-only. The T10/T11 UI and the new palette commands reference these lines:

- `features/app/{Cargo.toml,BUILD.bazel}`: add the dependency on `features/mass_delete`, and add `"apps/humata2/features/mass_delete"` to the root `Cargo.toml` members.
- `AppState` gets `pub mass_delete: humata2_mass_delete::MassDeleteState`, with `MassDeleteState::default()` in the constructor.
- `AppAction` gets `MassDelete(humata2_mass_delete::MassDeleteAction)`.
- `App` gets `mass_delete: MassDelete`, built with `MassDelete::new(clients.daemon.clone())`.
- In the child reduce: `AppAction::MassDelete(action) => self.mass_delete.reduce(&mut state.mass_delete, action).map(AppAction::MassDelete)`.
- In the main reduce, before the generic arm: `AppAction::MassDelete(MassDeleteAction::Delegate(Delegate::Toast(text))) => state.toast(text.clone(), &clock)`.
- `commands.rs`:
  - `Command::SelectSessions => self.child(state, AppAction::MassDelete(MassDeleteAction::EnterEmpty(surface)))`, where `surface` is `Surface::Machine(id)` on `Route::Machine(id)`, `Surface::Home` on Home, and `Surface::Sidebar` otherwise.
  - `Command::DeleteAllSessions => self.child(state, AppAction::MassDelete(MassDeleteAction::OpenAll))`.
- `overlay.rs` `dialog_holds_keys`: add `|| self.mass_delete.sheet.is_some()`. `AppAction::Escape` sends `MassDeleteAction::Escape` while the sheet is open or select mode is on.
- Feed `MassDeleteAction::Show(Listing)` whenever the workspace changes. It needs every session, `desk` set for Desk sessions, and the offline machines. Send `SurfaceChanged` on route changes.

If the wiring has to wait, the fallback is to back out the UI hunks in `ui/src/lists.rs`, `ui/machines/src/machine_page.rs` and `ui/overlays/src/overlays.rs`. Keep the DIFFUI/INBOXHOOKS Home-group hunks in `lists.rs`.

**Open gaps:**
- There is no MCP list_changed notification after a rule change.
- Agents cannot pass `delete_unpushed_work`.
- Remote `forks_staying` is lost after a re-plan.
- The box-id assumption in `delete_sessions_remote` is unchecked.

**Targets to build first:**
- `//apps/humata2/daemon/access:sessions_delete_many_test`
- `//apps/humata2/daemon/humatad:{delete_sessions_test,delete_sessions_cut_test,delete_sessions_remote_test,session_delete_other_test}`, plus the channel delete and trash tests
- `//apps/humata2/clients:clients_unit_test` and the live_daemon tests
- `//apps/humata2/cli:cli_unit_test`
- the svc_mcp unit test, and the daemon/mcp tools/schema tests
- the features/settings access_rows test
- `//apps/humata2/features/mass_delete:{selection_test,sheet_test}`
- `//apps/humata2/ui/...`, `//apps/humata2/features/app/...`

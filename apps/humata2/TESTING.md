# Testing `Humata 2`

How every `Humata 2` test is written. Every milestone from I6 on follows it,
and a milestone is ticked only when its tests do.

Read first: `SPEC.md` for what to prove; `api/BOUNDARIES.md`, `api/GRPC.md`
with `api/proto/`, `api/MCP.md`, `api/CLI.md` and `api/TRACE.md` for where
to prove it; `LAYOUT.md` for where tests live; `libs/tca-rust/README.md` for
`TestStore`.

Names in the examples that aren't in the protos or in `tca` are the test
harness's (§3). The harness is built with I3, I4 and I7; the examples fix
its shape.

## 1. Principles

- A test gives inputs and checks the result or the error. It proves one
  behaviour that the spec or an API doc names.
- Its doc comment starts with the requirement ids it proves (`/// TP-14`,
  `/// GRPC.md §5.2`), so a search for an id finds its tests.
- Test from outside: an RPC, an MCP tool, a CLI command, a reducer action or
  a person's click. Inside a crate, test only its public items.
- Assert on data: `Error.kind` and its fields, exit codes, `--json` fields,
  state, actions and view trees. Never on text written for people.
- Name the test as a sentence about the behaviour:
  `move_item_retried_with_the_same_key_moves_it_once`.
- Every test is deterministic: a `TestClock`, its own temp folder, no
  network and no sleeps. A flaky test is a failing test. Fix it or delete
  it; never mark it `flaky`.

## 2. Where tests run

- **macOS, always.** `bazel test //apps/humata2/... //libs/tca-rust/...` in
  the main worktree, with its one Bazel server. For a quick loop add
  `--test_tag_filters=-slow,-e2e`.
- **Linux, batched (D18).** `apps/humata2/testing/linux/run.sh <targets>`
  builds each test for static `musl` and runs it in the `humata2-linux`
  `Lima` VM (2 CPUs, 4 GiB, 20 GiB disk). It runs every few milestones, not
  on every change. Code that differs on Linux (the process-tree check, the
  sandbox for contained agents) gets Linux tests anyway; its milestone says
  "Linux: pending" until a batch passes.
- **Headless only.** No test opens a window on the developer's screen,
  takes focus or plays a sound. Views render through `GPUI`'s headless test
  platform and image snapshots render off screen. A test that shows a window
  is a bug.
- **Nothing of the user's.** Each test makes its own root under
  `TEST_TMPDIR`. No test reads or writes `~/Library/Application Support/Humata`,
  `~/.humata2`, `~/.ssh`, the `Keychain`, the `gh` config or the network. An
  `e2e` test reaches only the VMs it starts.

| Tag | For | Notes |
| --- | --- | --- |
| none | unit, reducer and boundary tests on the fake clock | `size = "small"` or `"medium"` |
| `slow` | anything over about 10 seconds, or needing real time | |
| `e2e` | real daemons in `Lima` VMs (MP-22), the kill matrix (PER-23) | also `slow`; `size = "large"` |
| `image` | image snapshots (§8.3) | macOS only; Linux runs leave them out |

```python
rust_test(
    name = "attach_test",
    size = "medium",
    srcs = ["tests/attach.rs"],
    deps = ["//apps/humata2/testing/boundary"],
)

rust_test(
    name = "kill_matrix_test",
    size = "large",
    srcs = ["tests/kill_matrix.rs"],
    tags = ["e2e", "slow"],
    deps = ["//apps/humata2/testing/boundary"],
)
```

## 3. The harness

Boundary suites share one harness: `testing/boundary`, crate
`humata2_boundary`, visible only to test targets.

- `World::start()` runs the real daemon code in process: the host and each
  machine on its own Unix socket under `TEST_TMPDIR`, joined by in-memory
  peer links. It holds the spec's own fixture: channel Payments; machines
  This Mac, devbox-1 and gpu-box; sessions "API rate limiter" on devbox-1
  and "Ledger schema migration" on This Mac; board item PAY-7. Tests then
  read like the spec.
- The outside world is fake: a `TestClock`, local bare `git` remotes, a
  `gh` stub, scripted providers, and a fake agent program that the holder
  runs. The fake agent prints, reads input and calls MCP tools when the test
  tells it to.
- Principals connect the real way, never by a flag: `world.app()`,
  `world.cli_user()` (no credential), `world.cli_logged_in()`,
  `world.agent(&session)` (a process under that session's holder with its
  token), `world.script(&token)` and `world.double_forked(&session)`. Each
  hands out the generated `tonic` clients: `.boards()`, `.sessions()`,
  `.terminal()`, `.events()`, `.ops()`, `.inbox()`.
- `world.mcp(&session)` runs `humatad mcp` for that session over stdio.
  `world.cli()` runs the real `humata` binary from `runfiles`.
- Faults: `restart_host()`, `drop_connections()`, `cut_link(machine)` (the
  Mac is away for that machine), `restore_link(machine)`, `stall(machine)`.
- Reads to compare against: `policy_stores()` (rules, grants, the away rule,
  identity bindings), `inbox()`, `item(id)`, `holder_output(&session)`.
- `humata_error(&status)` decodes the one `humata.v1.Error` in
  `google.rpc.Status.details`. `key()` makes a fresh `IdempotencyKey` for a
  request field. `every_rpc()` and `every_error_kind()` come from the
  descriptor set, so they grow with the protos.

Features don't use the harness. They use the fakes in `humata2_clients`
(§7).

## 4. `gRPC`

Suites live in `daemon/humatad/tests/`. The examples import:

```rust
use humata2_boundary::{World, every_rpc, humata_error, key};
use humata2_proto::humata::v1 as pb;
use pb::error::Kind;
```

### 4.1 Every RPC is in the contract

- Every RPC in the `humata.v1` descriptor set has a row in the access table
  of `humata2_access` (its capability, user-only, or none) and a
  valid-request builder in the harness. A new RPC without both fails.
- No RPC called by an agent changes a policy store, with every rule Allow
  and with every rule Never (BOUNDARIES.md §6.1).
- Every user-only RPC fails with `user_only` for an agent and for a script,
  and with `presence_required` for the CLI with no credential. The policy
  stores stay as they were.

```rust
/// BOUNDARIES.md §6.1: no agent call widens what an agent may do.
#[tokio::test]
async fn no_rpc_called_by_an_agent_changes_a_policy_store() {
    for decision in [pb::Decision::Allow, pb::Decision::Never] {
        let world = World::start_with(Rules::all(decision)).await;
        let agent = world.agent(&world.session("API rate limiter").await).await;
        let before = world.policy_stores().await;

        for rpc in every_rpc() {
            let request = world.valid_request(rpc).await;
            let _any_outcome = agent.call(rpc, request).await;
        }

        assert_eq!(world.policy_stores().await, before, "rules all {decision:?}");
    }
}
```

### 4.2 Errors carry one kind and its status code

- Each `Error.kind` travels with the status code in GRPC.md §10, and the
  `Error` comes back whole from `details`. One table test covers every
  kind. The table is a `match` with no `_` arm, so a new kind doesn't
  compile until it has a row.
- `DEADLINE_EXCEEDED` and `CANCELLED` carry no `Error`.
- Each RPC's tests provoke, for real, the kinds its doc comment names: a
  stale revision, a deleted item, a missing session. Assert the code, the
  kind and the kind's fields (§4.4 has one).
- `message` and `fix` are checked only for being there. Clients branch on
  `kind`, never on the code alone: several kinds share `ABORTED`.

```rust
/// GRPC.md §10. Abbreviated: the real table has one arm per kind.
fn documented_code(kind: &Kind) -> tonic::Code {
    use tonic::Code;
    match kind {
        Kind::NotFound(_) => Code::NotFound,
        Kind::InvalidField(_) | Kind::OutsideHumataFolders(_) => Code::InvalidArgument,
        Kind::IdentityMismatch(_) => Code::Unauthenticated,
        Kind::RevisionConflict(_) | Kind::PlanStale(_) | Kind::AlreadyAnswered(_) => Code::Aborted,
        Kind::OffsetTrimmed(_) => Code::OutOfRange,
        Kind::ChecksumMismatch(_) => Code::DataLoss,
        Kind::Internal(_) => Code::Internal,
        // … every other kind
    }
}

#[test]
fn every_error_kind_travels_with_its_documented_code() {
    for kind in every_error_kind() {
        let error = pb::Error { kind: Some(kind.clone()), ..Default::default() };
        let status = humata2_model::error::to_status(&error);
        assert_eq!(status.code(), documented_code(&kind), "{kind:?}");
        assert_eq!(humata_error(&status), error, "{kind:?} comes back whole");
    }
}
```

### 4.3 Retries with the same key

- A mutation sent twice with one key returns the first response and acts
  once: one revision, one history entry, one event. This holds across
  `restart_host()`.
- A client that lost the response finds the operation with
  `FindOperation` by its key.
- A journal replayed twice applies once (`JournalPosition`).
- A key reused with a different request fails with `invalid_field` naming
  `idempotency_key` (provisional, see the decisions in the I5 report).

```rust
/// GRPC.md §5.2: a retry with the same key moves the item once, even after a restart.
#[tokio::test]
async fn move_item_retried_with_the_same_key_moves_it_once() {
    let mut world = World::start().await;
    let item = world.item("PAY-7").await;
    let history = world.item_history("PAY-7").await.len();
    let request = pb::MoveItemRequest {
        idempotency_key: key(),
        id: item.id.clone(),
        column: pb::BoardColumn::Done as i32,
        expected_revision: item.revision.clone(),
        ..Default::default()
    };

    let first = world.app().await.boards().move_item(request.clone()).await.expect("moved");
    world.restart_host().await;
    let retry = world.app().await.boards().move_item(request).await.expect("same answer");

    assert_eq!(retry.into_inner(), first.into_inner());
    assert_eq!(world.item_history("PAY-7").await.len(), history + 1);
}
```

### 4.4 Revisions, deletes, plans and pages

- An update against an old `expected_revision` fails with
  `revision_conflict`, whose `actual` is the current revision. Nothing
  changes.
- An update with no `expected_revision` applies, and history keeps the
  earlier value.
- An edit or move of a deleted item fails with `not_found`. The attempt is
  in history and the item stays deleted.
- A `Plan*` call followed by a change to what it lists makes the act fail
  with `plan_stale` (NV-27).
- A page token pins its snapshot. After `clock.advance(10 minutes)` it
  fails with `invalid_field` naming `page_token`.

```rust
/// SL-7, GRPC.md §5.3: an edit against an old revision fails and names the current one.
#[tokio::test]
async fn update_item_against_a_stale_revision_fails_with_the_current_revision() {
    let world = World::start().await;
    let item = world.item("PAY-7").await;
    let edit = |title: &str| pb::UpdateItemRequest {
        idempotency_key: key(),
        id: item.id.clone(),
        patch: Some(pb::ItemPatch { title: Some(title.into()), ..Default::default() }),
        expected_revision: item.revision.clone(),
        ..Default::default()
    };
    let mut boards = world.app().await.boards();
    boards.update_item(edit("Retry budget")).await.expect("the first edit applies");

    let status = boards.update_item(edit("Backoff")).await.expect_err("the second is stale");

    assert_eq!(status.code(), tonic::Code::Aborted);
    let Some(Kind::RevisionConflict(conflict)) = humata_error(&status).kind else {
        panic!("expected revision_conflict, got {status:?}");
    };
    let now = world.item("PAY-7").await;
    assert_eq!(conflict.expected, item.revision);
    assert_eq!(conflict.actual, now.revision);
    assert_eq!(now.title, "Retry budget");
}
```

### 4.5 Streams resume exactly

| Stream | Resume test | Behind what is kept |
| --- | --- | --- |
| `TerminalService.Attach` | drop, attach `from_offset` the last byte seen: no gap, no repeat | `from_offset` 0 or before `earliest_offset`: a `TerminalSnapshot`, then output from its offset |
| `EventService.Subscribe` | `after` the last seq: every visible event after it, in order | `Resync{earliest}` first, then live events |
| `WatchOperation`, `WatchTransfer` | `from_version`: versions only rise, none skipped that changed something | n/a |
| `ReadFile`, `ExportConversation` | `offset`: bytes match the source | per the pick on `offset_trimmed` |
| `Upload`, `WriteFile` | `UploadHeader.resume`: the file's `sha256` matches | `checksum_mismatch` when it doesn't |

Also:

- Attach flow control: a client that stops acknowledging gets at most
  `window_bytes` more. One that falls behind the kept scrollback gets a new
  snapshot, not a growing buffer.
- TR-6, tagged `slow`: with `stall("gpu-box")`, an attach to devbox-1
  keeps its output rate, and the app's store keeps reducing input.

```rust
/// TR-4, PER-3: attaching again from the last offset continues with no gap and no repeat.
#[tokio::test]
async fn attach_again_from_the_last_offset_continues_exactly() {
    let world = World::start().await;
    let session = world.session("API rate limiter").await;
    world.agent_prints(&session, 64 * 1024).await;
    let open = pb::AttachOpen {
        session: Some(session.clone()),
        read_only: true,
        cols: 80,
        rows: 24,
        window_bytes: 1 << 20,
        ..Default::default()
    };

    let mut tab = world.app().await.attach(open.clone()).await;
    let snapshot = tab.snapshot().await; // from_offset 0 starts with one
    let mut seen = tab.output_bytes(4096).await;
    world.drop_connections().await;

    let resume_at = snapshot.offset + seen.len() as u64;
    let mut tab = world.app().await.attach(pb::AttachOpen { from_offset: resume_at, ..open }).await;
    let next = tab.output().await;
    assert_eq!(next.offset, resume_at);
    seen.extend(next.data);
    seen.extend(tab.output_until_idle().await);
    assert_eq!(seen, world.holder_output(&session).await[snapshot.offset as usize..]);
}

/// TR-4: an event stream behind the kept events gets Resync, then live events.
#[tokio::test]
async fn subscribe_from_a_trimmed_seq_gets_resync_then_live_events() {
    use pb::event::Payload;
    let world = World::start_with(Retention::events(100)).await;
    let old = world.last_event_seq().await;
    world.make_changes(150).await;

    let mut events = world.app().await.events()
        .subscribe(pb::SubscribeRequest { after: Some(old), ..Default::default() })
        .await.expect("subscribed").into_inner();
    let first = events.message().await.expect("open").expect("an event");
    assert!(matches!(first.payload, Some(Payload::Resync(_))));

    world.move_item("PAY-7", pb::BoardColumn::Done).await;
    let moved = events.message().await.expect("open").expect("an event");
    assert!(matches!(
        moved.payload,
        Some(Payload::BoardItem(item)) if item.column() == pb::BoardColumn::Done
    ));
}
```

### 4.6 Operations

- Closing a watch or dropping the connection never cancels an operation.
- `CancelOperation` before the point of no return undoes what GRPC.md §7
  says, and the steps read `STEP_STATE_UNDONE`. After it, the call fails
  with `not_cancellable` and `cancellable` is false.
- `RetryOperation` reruns from the failed step. Steps already done stay
  done, and their effects happen once: the fake remote served each clone
  once.

```rust
/// GRPC.md §7: past `launch` a start can't be cancelled, and it finishes with nobody watching.
#[tokio::test]
async fn a_start_past_launch_refuses_cancel_and_finishes_unwatched() {
    let world = World::start().await;
    world.fake_agents().hold_before_ready();
    let app = world.app().await;
    let started = app.sessions().start_session(world.start_request("devbox-1")).await
        .expect("started").into_inner();
    let id = started.operation.expect("an operation").id;
    world.wait_for_step(&id, "verify", pb::StepState::Running).await;
    world.drop_connections().await;

    let status = world.app().await.ops()
        .cancel_operation(pb::CancelOperationRequest { id: id.clone(), idempotency_key: key() })
        .await.expect_err("past launch");
    assert!(matches!(humata_error(&status).kind, Some(Kind::NotCancellable(_))));

    world.fake_agents().release();
    assert_eq!(world.finished_operation(&id).await.state(), pb::OperationState::Succeeded);
}
```

### 4.7 The host away

- With `cut_link(machine)`, a write the machine's policy copy allows
  returns `DELIVERY_JOURNALED` with a `JournalPosition` and a provisional id
  (`~devbox-1.41`). After `restore_link`, an event resolves the id.
- Replay checks each entry against the host's current rules. An entry the
  rules now refuse ends as `journal_entry_rejected`, and the host is
  unchanged.
- Conflicts merge field by field from the base revision. The loser is in
  history with `lost_conflict`.
- Two answers to one Inbox request: the first wins, the second gets
  `already_answered`.

```rust
/// D3, BOUNDARIES.md §6.1: a write journaled under a rule since set to Never is rejected at replay.
#[tokio::test]
async fn a_journaled_move_is_rejected_at_replay_once_the_rule_is_never() {
    let world = World::start_with(Rules::default().allow(pb::Capability::ChannelBoardEditOthers)).await;
    let before = world.item("PAY-7").await;
    let mut agent = world.mcp(&world.session("API rate limiter").await).await;
    world.cut_link("devbox-1").await;

    let moved = agent.call("board_item_move", json!({"id": "PAY-7", "column": "done"})).await;
    assert_eq!(moved.outcome(), "journaled");

    world.set_rule("Payments", pb::Capability::ChannelBoardEditOthers, pb::Decision::Never).await;
    world.restore_link("devbox-1").await;
    world.wait_for_replay("devbox-1").await;

    assert_eq!(world.item("PAY-7").await, before);
    let journal = agent.call("journal_status", json!({})).await;
    assert_eq!(journal.result()["entries"][0]["state"], "rejected");
}
```

### 4.8 The protocol only grows

- `humata.v1` changes are additive (GRPC.md §11). Each release copies its
  descriptor set to `api/proto/released/`. A test compares the current set
  with it: no field number reused or retyped, removed fields `reserved`.
- A client given an enum value it doesn't know reads `UNSPECIFIED` and goes
  on.

```rust
/// GRPC.md §11: nothing released is removed, renumbered or retyped.
#[test]
fn humata_v1_changes_are_additive_since_the_last_release() {
    let released = descriptor_set(runfile("apps/humata2/api/proto/released/humata_v1.binpb"));
    let current = descriptor_set(runfile("apps/humata2/api/proto/humata2_proto-descriptor-set.proto.bin"));
    assert_eq!(breaking_changes(&released, &current), []);
}
```

## 5. `MCP`

Suites live in `daemon/mcp/tests/`. The client is `world.mcp(&session)`,
which speaks `JSON-RPC` to the real `humatad mcp` over stdio.

### 5.1 Tool schemas

- `tools/list` for an agent equals the tools of MCP.md §6.1 and §6.2, kept
  sorted in `daemon/mcp/tests/tools.txt`. Adding a tool changes that file
  in the same review.
- Each name is `<area>_<verb>` (TP-16). Each input schema is valid
  `JSON Schema`, with `additionalProperties: false` and the limits MCP.md
  gives (titles at most 200 characters, `wait_seconds` from 0 to 600). Each
  output schema is the envelope.
- A call the schema rejects is a `JSON-RPC` error and reaches no handler. A
  call that fits the schema but names something missing is a tool error
  with a kind, such as `not_found`.

```rust
/// TP-16, MCP.md §3 and §6: every tool is strict about its input and returns the envelope.
#[tokio::test]
async fn every_listed_tool_has_a_strict_schema_and_returns_the_envelope() {
    let world = World::start().await;
    let mut agent = world.mcp(&world.session("API rate limiter").await).await;
    let tools = agent.tools_list().await;

    let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    names.sort();
    assert_eq!(names, expected_tools("tests/tools.txt"));
    for tool in &tools {
        assert!(is_area_verb(&tool.name), "{}", tool.name);
        assert_valid_json_schema(&tool.input_schema);
        assert_eq!(tool.input_schema["additionalProperties"], false, "{}", tool.name);
        assert_eq!(tool.output_schema["$ref"], ENVELOPE_ID, "{}", tool.name);
    }
}

/// MCP.md §3: a call the schema rejects is a protocol error and changes nothing.
#[tokio::test]
async fn board_item_update_with_a_201_character_title_is_rejected_by_the_schema() {
    let world = World::start().await;
    let before = world.item("PAY-7").await;
    let mut agent = world.mcp(&world.session("API rate limiter").await).await;

    let reply = agent.call_raw("board_item_update", json!({"id": "PAY-7", "title": "x".repeat(201)})).await;

    assert_eq!(reply.json_rpc_error_code(), Some(-32602));
    assert_eq!(world.item("PAY-7").await, before);
}
```

### 5.2 Tiers, as a table

Every row of MCP.md §6.1 and §6.2 is a row of a data table: the tool, its
target, its capability and what the default rule makes of it. Default
Allow and no rule: `applied`. Ask me: `pending`, with a request. Never:
`denied` at once. User-only names: `user_only`.

- The table's defaults are checked against `AccessService.ListCapabilities`.
  A capability without a row fails, and so does a default that differs.
- A second pass sets each editable rule to Allow, Ask me and Never in turn,
  and checks the outcome follows.

```rust
/// MCP.md §6.1, §6.2. Abbreviated: one row per row of the two tables.
const DEFAULTS: &[(&str, Target, Expect)] = &[
    ("board_item_create", Target::OwnSessionBoard, Expect::Applied), // OWN_BOARD_WRITE
    ("board_item_create", Target::ChannelBoard, Expect::Applied), // CHANNEL_BOARD_ADD
    ("board_item_update", Target::OthersChannelItem, Expect::Pending), // CHANNEL_BOARD_EDIT_OTHERS
    ("channels_list", Target::None, Expect::Pending), // GLOBAL_READ, set to Ask first (default Allow, D127.4)
    ("session_fork", Target::Machine("gpu-box"), Expect::Pending), // SESSION_START_OTHER_MACHINE
    ("network_connect", Target::Host("db.internal", 5432), Expect::Pending), // NETWORK_REACH, D20
    ("session_stop", Target::OtherSession, Expect::Denied), // SESSION_STOP_OTHER: Never
    ("machine_remove", Target::Machine("gpu-box"), Expect::Denied), // MACHINE_DESTROY: Never
];

#[tokio::test]
async fn each_tool_does_what_its_default_rule_says() {
    let mut failures = Vec::new();
    for &(tool, target, expect) in DEFAULTS {
        let world = World::start().await;
        let mut agent = world.mcp(&world.session("API rate limiter").await).await;
        let args = world.valid_args(tool, target).await; // sets wait_seconds to 0
        let got = Expect::of(&agent.call(tool, args).await);
        if got != expect {
            failures.push(format!("{tool} on {target:?}: got {got:?}, expected {expect:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
```

### 5.3 Refusals

A refusal is a tool result with `isError: true` and
`structuredContent.error`:

- `kind` is an `errors.proto` kind. `message` and `fix` are present and not
  empty. `retry.retryable` is true only when retrying can succeed.
- `details` holds the kind's fields, read in the shape the decisions pick.
- `denied` from a Never rule adds nothing to the Inbox and names the
  capability. `user_only` adds nothing to the Inbox either.
- A user-only operation has no tool. Called by name anyway, it fails with
  `user_only` and reaches no handler (MCP.md §7).
- A session token presented from another process tree fails with
  `identity_mismatch`. The token is revoked and Activity records it
  (SEC-15).

```rust
/// IN-18: a Never rule refuses at once, names the capability and adds nothing to the Inbox.
#[tokio::test]
async fn stopping_another_session_is_denied_at_once() {
    let world = World::start().await;
    let other = world.session("Ledger schema migration").await;
    let mut agent = world.mcp(&world.session("API rate limiter").await).await;

    let result = agent.call("session_stop", json!({"session": other.value})).await;

    assert!(result.is_error);
    let error = &result.structured["error"];
    assert_eq!(error["kind"], "denied");
    assert_eq!(error["details"]["source"], "never_rule");
    assert_eq!(error["details"]["capability"], "session_stop_other");
    assert!(error["fix"].as_str().is_some_and(|fix| !fix.is_empty()));
    assert_ne!(error["retry"]["retryable"], true);
    assert!(world.inbox().await.is_empty());
    assert!(world.session_is_running(&other).await);
}

/// SEC-7, MCP.md §7: no user-only operation has a tool, and calling one by name is refused.
#[tokio::test]
async fn user_only_operations_have_no_tool_and_are_refused_by_name() {
    let world = World::start().await;
    let mut agent = world.mcp(&world.session("API rate limiter").await).await;
    let listed: Vec<String> = agent.tools_list().await.into_iter().map(|tool| tool.name).collect();

    for operation in humata2_access::user_only_operations() {
        let name = operation.replace('.', "_"); // "inbox.answer" is called as "inbox_answer"
        assert!(!listed.contains(&name), "{name} is listed");
        let result = agent.call(&name, json!({})).await;
        assert_eq!(result.error_kind(), Some("user_only"), "{name}");
    }
    assert!(world.inbox().await.is_empty());
}
```

### 5.4 Waiting for the user

- A call that asks, with `wait_seconds: 0`, returns `pending` with a
  `request_id`, and the request is in the Inbox (TP-11).
- With a `progressToken`, each `clock.advance(15 seconds)` while the call
  waits brings a progress notification.
- The client's retry of the same `JSON-RPC` request returns the same
  request. The Inbox still holds one (TP-14).
- Approval runs the call whether or not anyone waits. `inbox_wait` then
  returns what the original call would have (TP-12, TP-13).
- Withdrawal (TP-15): an `MCP` process that exits while its call is open,
  before `pending`, withdraws the request. One that exits after `pending`
  leaves it answerable.
- With the Mac away, each While I'm away rule (Wait, Skip the step, Allow
  low-risk) gives the outcome in the MCP.md §4 table.

```rust
/// TP-11, TP-13, TP-14: pending, the same request on a retry, and the result once approved.
#[tokio::test]
async fn a_fork_that_asks_returns_pending_and_runs_once_approved() {
    let world = World::start().await;
    let mut agent = world.mcp(&world.session("API rate limiter").await).await;
    let args = json!({
        "machine": "gpu-box", "kind": "code_and_context",
        "reason": "the benchmarks need a GPU", "wait_seconds": 0,
    });

    let first = agent.call_with_id(7, "session_fork", args.clone()).await;
    let retry = agent.call_with_id(7, "session_fork", args).await;
    assert_eq!(first.outcome(), "pending");
    assert_eq!(retry.request_id(), first.request_id());
    assert_eq!(world.inbox().await.len(), 1);

    world.app_answers(first.request_id(), Answer::Approve).await;
    let done = agent.call("inbox_wait", json!({"request_id": first.request_id()})).await;
    assert_eq!(done.outcome(), "applied");
    assert!(done.result()["session"].is_string());
}
```

### 5.5 The capability matrix

TRACE.md §2 is a data table in `testing/boundary/tests/matrix.rs`: one cell
per actor and tier. Agents act through `MCP`, the CLI tiers and scripts
through `humata`, the app through `gRPC`. Each tier has one representative
call, and a cell may name its own call when the row's words need it (a
`whoami` with no credential).

| Tier | Representative call | Capability |
| --- | --- | --- |
| No rule | `file_write` into its own clone; `humata files put` | none |
| Default Allow | `board_item_create` on the channel board; `humata board add` | `CHANNEL_BOARD_ADD` |
| Ask me | `channels_list`; `humata channels` (the test sets `GLOBAL_READ` to Ask first; its default is Allow, D127.4) | `GLOBAL_READ` |
| Never by default | `session_stop` of another session; `humata session stop` | `SESSION_STOP_OTHER` |
| User-only | `inbox.answer`; `humata inbox approve` | `INBOX_ANSWER` |

The peer row is a test of its own: `PeerService` is on no socket, so every
socket connection that calls it gets `UNIMPLEMENTED`.

```rust
/// TRACE.md §2. Abbreviated: one cell per actor and tier.
const MATRIX: &[Cell] = &[
    Cell::new(Actor::AgentOwnSession, Tier::NoRule, Expect::Applied),
    Cell::new(Actor::AgentOwnSession, Tier::AskMe, Expect::Pending),
    Cell::new(Actor::AgentOwnSession, Tier::Never, Expect::Denied),
    Cell::new(Actor::AgentOwnSession, Tier::UserOnly, Expect::UserOnly),
    Cell::new(Actor::CliInSessionTree, Tier::UserOnly, Expect::UserOnly),
    Cell::new(Actor::CliPresent, Tier::AskMe, Expect::Applied),
    Cell::new(Actor::CliNoCredential, Tier::DefaultAllow, Expect::PresenceRequired),
    Cell::new(Actor::CliNoCredential, Tier::NoRule, Expect::PresenceRequired),
    Cell::new(Actor::MachineWithMacAway, Tier::DefaultAllow, Expect::Journaled),
    Cell::new(Actor::Script, Tier::UserOnly, Expect::UserOnly),
];

#[tokio::test]
async fn the_capability_matrix_holds_cell_by_cell() {
    let mut failures = Vec::new();
    for cell in MATRIX {
        let world = World::start().await;
        let got = world.as_actor(cell.actor).await.run(cell.call()).await;
        if got != cell.expect {
            failures.push(format!("{cell:?}: got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
```

### 5.6 Each tool with the Mac away

MCP.md §5 gives each tool one of local, journal, host or stale. A table
test cuts the link and calls each tool from devbox-1:

| Mode | Expect |
| --- | --- |
| local | `applied`, as with the Mac there |
| journal | `journaled`, with `journal` set |
| host | waits, or `host_unreachable` at once under Skip the step |
| stale | `applied`, with `stale_as_of` set |

```rust
/// MCP.md §5. Abbreviated: one row per tool.
const OFFLINE: &[(&str, Mode)] = &[
    ("whoami", Mode::Local),
    ("board_item_move", Mode::Journal),
    ("session_move", Mode::Host),
    ("board_items_list", Mode::Stale),
];

#[tokio::test]
async fn each_tool_behaves_as_its_host_away_mode_says() {
    let mut failures = Vec::new();
    for &(tool, mode) in OFFLINE {
        let world = World::start_with(Rules::default().away(AwayRule::Skip)).await;
        let mut agent = world.mcp(&world.session("API rate limiter").await).await;
        world.cut_link("devbox-1").await;
        let result = agent.call(tool, world.valid_args(tool, Target::Default).await).await;
        if !mode.matches(&result) {
            failures.push(format!("{tool}: {result:?} is not {mode:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
```

## 6. The CLI

Suites live in `cli/tests/` and run the real `humata` binary. Without
`--json`, the CLI writes for people on `stderr`; tests don't read it.

### 6.1 Exit codes

- Every `Error.kind` exits with the code in CLI.md §7. A stub daemon
  answers every call with one kind, and the table test runs a command
  against it for each kind. As in §4.2, the table is a `match` with no `_`
  arm.
- The codes with no error kind get their own tests: 0, 2 for a bad flag, 12
  for `pending`, 20 for `--strict` and 130 for Ctrl-C.

```rust
/// CLI.md §7. Abbreviated: the real table has one arm per kind.
fn documented_exit(kind: &Kind) -> i32 {
    match kind {
        Kind::InvalidField(_) | Kind::OutsideHumataFolders(_) => 2, // provisional
        Kind::NotFound(_) => 3,
        Kind::Denied(_) | Kind::NotInChannel(_) | Kind::PushRefused(_) => 4,
        Kind::UserOnly(_) | Kind::PresenceRequired(_) | Kind::IdentityMismatch(_) => 5,
        Kind::RevisionConflict(_) | Kind::PlanStale(_) | Kind::NameTaken(_) => 6,
        Kind::MachineOffline(_) | Kind::HostUnreachable(_) | Kind::SshFailed(_) => 8,
        Kind::Internal(_) => 70,
        // … every other kind
    }
}

#[tokio::test]
async fn every_error_kind_exits_with_its_documented_code() {
    for kind in every_error_kind() {
        let daemon = StubDaemon::failing_every_call_with(kind.clone()).await;
        let run = daemon.cli().run(["board", "move", "PAY-7", "done", "--json"]).await;
        assert_eq!(run.code, documented_exit(&kind), "{kind:?}");
        assert_eq!(run.json()["error"]["kind"], kind_name(&kind), "{kind:?}");
    }
}
```

### 6.2 `--json`

- `stdout` holds exactly one object and nothing else: on success, on
  `journaled`, on `pending` and on every error.
- `operation` is the dotted operation id. `ok` is true exactly when the
  command did what it was asked, and `outcome` says how.

```rust
/// CLI.md §7: `--json` prints one object, and the move happened.
#[tokio::test]
async fn board_move_with_json_prints_one_object() {
    let world = World::start().await;

    let run = world.cli().logged_in().run(["board", "move", "PAY-7", "done", "--json"]).await;

    assert_eq!(run.code, 0);
    let objects: Vec<serde_json::Value> = serde_json::Deserializer::from_str(&run.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .expect("stdout is JSON");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["ok"], true);
    assert_eq!(objects[0]["operation"], "board.item_move");
    assert_eq!(objects[0]["outcome"], "applied");
    assert_eq!(world.item("PAY-7").await.column(), pb::BoardColumn::Done);
}
```

### 6.3 Who it runs as

Where `humata` runs decides its actor, never a flag (CLI.md §2):

- Inside a session's process tree it is that session's agent, with its
  rules. User-only commands fail with `user_only`, exit 5.
- Elsewhere, not logged in, it has no credential. Every command but
  `humata whoami` and `humata login` fails with `presence_required`,
  exit 5.
- After `humata login` it is the present user, in that terminal session
  only. A child that calls `setsid` has no credential again (SEC-11).
- With `HUMATA_TOKEN` it is a script: only the token's capabilities on its
  channels.
- A session token used outside its tree fails with `identity_mismatch`
  (provisional, see the decisions).
- A presence prompt inside a session's terminal is refused, with
  `caller_in_session_terminal` set.

```rust
/// SEC-8, SEC-9, SEC-10, SEC-11, SEC-14: the actor comes from where `humata` runs.
#[tokio::test]
async fn whoami_reports_the_actor_of_where_it_runs() {
    let world = World::start().await;
    let session = world.session("API rate limiter").await;
    let token = world.script_token(&[pb::Capability::ChannelBoardAdd], "Payments").await;

    for (cli, actor) in [
        (world.cli().in_session(&session), "agent"),
        (world.cli(), "none"),
        (world.cli().logged_in(), "user_present"),
        (world.cli().logged_in().after_setsid(), "none"),
        (world.cli().with_token(&token), "script"),
    ] {
        let run = cli.run(["whoami", "--json"]).await;
        assert_eq!(run.json()["result"]["actor"], actor, "{cli:?}");
    }
}

/// SEC-7, SEC-8: an agent's own `humata` can't answer its request.
#[tokio::test]
async fn inbox_approve_inside_a_session_fails_as_user_only() {
    let world = World::start().await;
    let session = world.session("API rate limiter").await;
    let request = world.pending_request_from(&session).await;

    let run = world.cli().in_session(&session).run(["inbox", "approve", &request, "--json"]).await;

    assert_eq!(run.code, 5);
    assert_eq!(run.json()["error"]["kind"], "user_only");
    assert_eq!(world.request_state(&request).await, pb::RequestState::Pending);
}
```

### 6.4 Waiting, `--strict`, plans and keys

- `--wait 0 --json` on a call that asks exits 12 with the request id.
  Ctrl-C while waiting exits 130 and leaves the request in the Inbox.
- `--strict` turns `journaled` into exit 20; the write is still journaled.
  Without it, `journaled` exits 0.
- `--dry-run` changes nothing. `--plan` after the listed things changed
  exits 6 with `plan_stale`.
- `--key` reused gives the first result again.

```rust
/// CLI.md §4: `--strict` makes a journaled move exit 20, and the move is still journaled.
#[tokio::test]
async fn strict_turns_a_journaled_move_into_exit_20() {
    let world = World::start().await;
    world.cut_link("devbox-1").await;

    let run = world.cli().on_machine("devbox-1").logged_in()
        .run(["board", "move", "PAY-7", "done", "--json", "--strict"]).await;

    assert_eq!(run.code, 20);
    assert_eq!(run.json()["outcome"], "journaled");
    assert_eq!(world.journal("devbox-1").await.len(), 1);
}
```

## 7. Reducers

Features (`features/<name>`) are tested with `TestStore` from `tca`.

- **Exhaustive by default.** Each step asserts the whole change, every
  action an effect sends is received, and no effect runs at the end.
- **Non-exhaustive** (`.non_exhaustive()`) only for a test about one part
  of a large composed feature, such as the app root, and inside flows. Say
  why in a comment.
- Clients come from `Dependencies::with`, using the fakes in
  `humata2_clients`. Time moves with `TestClock::advance`.
- Actions and state are the model, so they are fair to assert on. What an
  `Effect` holds inside is not.
- A stream effect keeps its cursor in state. Test the resume by ending the
  fake stream and checking the cursor the feature reopens from.
- Use `receive_matching` for actions that hold errors and can't be
  compared.

```rust
use humata2_clients::{DaemonClient, FakeDaemon};
use humata2_inbox::{Action, Inbox, State};
use tca::{Dependencies, TestStore};

/// GRPC.md §9.4: an approval that lost to an answer elsewhere shows that answer.
#[test]
fn approving_a_request_answered_elsewhere_shows_the_other_answer() {
    let daemon = FakeDaemon::new().with_answered_elsewhere("rq_1", Answer::Deny);
    let dependencies = Dependencies::new().with(DaemonClient::from(daemon));
    let inbox = Inbox::new(&dependencies).expect("the inbox's clients");
    let mut store = TestStore::new(State::with_pending(["rq_1"]), inbox);

    store.send(Action::Approve(rq("rq_1")), |state| state.request_mut("rq_1").answering = true);
    store.receive_matching(
        "Answered(already_answered)",
        |action| matches!(action, Action::Answered(_, Err(_))),
        |state| {
            let request = state.request_mut("rq_1");
            request.answering = false;
            request.state = RequestState::Denied;
            request.answered_elsewhere = true;
        },
    );
    store.finish();
}

/// 26.4 step 6: counts drop everywhere once a request is answered.
#[test]
fn the_rail_count_drops_when_a_request_is_answered() {
    // Non-exhaustive: the app root has many children, and this test is about the rail.
    let mut store = TestStore::new(AppState::fixture(), app(&fake_dependencies())).non_exhaustive();
    store.send(Action::Event(answered("rq_1")), |app| app.rail.inbox_count = 6);
    store.finish();
}
```

## 8. User-facing tests

### 8.1 Flows

- A flow is a script of what a person does (click, type, key, drag, paste)
  against the real state and views, with fake clients. It lives in
  `desktop/tests/flows/` and is named after the spec flow it proves:
  `flow_26_04_approve_an_inbox_request_after_opening_the_requester`.
- Steps find elements by role and label, the way a person does. Never by
  position or internal id.
- Assert on what a person sees (view snapshots) and on what reached the
  daemon (the fake's received requests). Both are boundaries.
- Each run writes its steps and snapshots, in order, to
  `TEST_UNDECLARED_OUTPUTS_DIR`, to review beside the spec flow.

```rust
use humata2_flow::{Flow, Role};

/// 26.4 Approve an Inbox request after opening the requester.
fn approve_after_opening_the_requester(flow: &mut Flow) {
    let card = "Read files of “Ledger schema migration” on This Mac";
    flow.click(Role::Button, "Inbox 7");
    flow.click(Role::Row, card);
    flow.snapshot("card");
    flow.click(Role::Button, "Open session ↗");
    flow.expect_selected(Role::Row, card);
    flow.click(Role::Button, "Approve");
    flow.snapshot("approved");
}

#[test]
fn flow_26_04_approve_an_inbox_request_after_opening_the_requester() {
    let mut flow = Flow::new(Fixture::payments().with_inbox(7));
    approve_after_opening_the_requester(&mut flow);

    assert_eq!(flow.daemon().answers(), [(rq("rq_1"), Answer::Approve)]);
    assert!(flow.daemon().typed_into("API rate limiter").is_empty()); // IN-12
}
```

### 8.2 View snapshots

A snapshot is the data under what is drawn, written as text, not a picture
of it. Text is cheaper to record and compare, and an agent or a reviewer
can read it and its diff directly. The view tree and the layout below are
the default; an image (8.3) is the fallback for what only pixels show.

The view tree is the default for anything a person sees. A view renders
headless to a semantic text tree: roles, labels, text, focus, selection, enabled. It says
what the view shows and does, with no pixels, sizes, colours or fonts, so
the tree is the same on every machine and changes only when behaviour does.

When how the view is laid out matters, add a layout snapshot
(`layout::capture`, `assert_layout`). It is the same window with each
element's, text's, background's and border's position and size in whole
pixels, its colours, radii and clipping. It changes with any layout tweak,
so take it where layout is what the test proves, rather than an image. Under
`TestAppContext` it is one file for every OS. Only what it can't show
(glyphs, text colour and font, icons, shadows) needs an image snapshot
(8.3).

```text
panel "Inbox" focused
  row "Read files of “Ledger schema migration” on This Mac" selected
    text "From API rate limiter · devbox-1"
    link "migrations/0042_balance_cents.sql"
    button "Open session ↗" enabled
    button "Approve" enabled
    button "Deny" enabled
```

```rust
/// IN-9: a card names the action, the requester and its machine.
#[test]
fn an_inbox_card_names_the_action_requester_and_machine() {
    let view = render_tree(InboxPanel::fixture(read_files_request()));
    assert_view_snapshot!("inbox_card_read_files", view);
}
```

- Snapshots sit next to their test as text and are reviewed like code.
  `testing/snapshot` gives one command that records them; `bazel test`
  verifies.
- In Bazel, declare the test with `humata2_snapshot_test` from
  `//apps/humata2/testing:defs.bzl`, depending on
  `//apps/humata2/testing/snapshot` and `//apps/humata2/testing/flow` (or
  their `_fast` twins on GPUI Fast). It makes the snapshots the test's data
  and adds `<name>_record`: `bazel run` it to record into the source tree,
  then read `git diff`. A failed check's files land in
  `bazel-testlogs/<package>/<name>/test.outputs/`. `testing/README.md` has
  the details.
- A failed check prints the tree's diff. Record again only when the change
  is meant.

### 8.3 Image snapshots

The fallback, only where pixels are the behaviour and neither the view tree
nor the layout can say it: diff colours, the terminal (wide
characters, emoji, scripts) and Wrap. macOS only, tagged `image`, rendered
off screen with a fixed size, scale and bundled fonts, compared with a
tolerance. A failure writes the old image, the new one and their diff to
`TEST_UNDECLARED_OUTPUTS_DIR`.

```rust
/// 26.14 step 1: a wrapped line keeps its number and + mark on its first visual line.
#[test]
fn a_long_added_line_wraps_with_its_mark_on_the_first_visual_line() {
    let image = render_image(DiffPanel::fixture("client.rs", long_added_line()), Size::new(480, 160));
    assert_image_snapshot!("diff_wrap_long_added_line", image);
}
```

```python
rust_test(
    name = "diff_images_test",
    srcs = ["tests/diff_images.rs"],
    tags = ["image"],
    target_compatible_with = ["@platforms//os:macos"],
    deps = ["//apps/humata2/testing/snapshot"],
)
```

### 8.4 End to end

The same flow functions run against real daemons: the host on this Mac and
machines in `Lima` VMs through the Lima provider (MP-22). These tests are
tagged `e2e` and `slow`. The kill matrix (PER-23) runs here: `kill -9` a
holder or the daemon, cut the SSH link, hard-reset a VM, then check the
outcome its tier promises.

```rust
/// 26.4, end to end.
#[test]
fn flow_26_04_against_real_daemons() {
    let mut flow = Flow::against(RealDaemons::lima(["devbox-1"]).with_inbox(7));
    approve_after_opening_the_requester(&mut flow);
    assert_eq!(flow.daemon().request_state("rq_1"), RequestState::Approved);
}
```

## 9. What never to test

- **Wording.** Not `Error.message`, `Error.fix`, CLI `stderr`, history
  `change` text or log lines. Check that `fix` is there; branch on `kind`.
  Labels in flows and view snapshots are the exception: they are the UI the
  spec names. Flow fakes return their own error text, so a daemon's wording
  never reaches a snapshot.
- **Call order.** Not which function calls which, or how often a fake was
  called. A count is fair only when the spec names it at a boundary: one
  Inbox card for a retried call, one clone per repository after a retry.
- **Private helpers.** Not through `pub(crate)` or `#[cfg(test)] pub` made
  for a test. Test the behaviour they serve, through the boundary.
- **Generated code and timing.** Not the `prost` output, struct layouts,
  how long something took (outside a marked benchmark) or which thread ran
  it.

```rust
// Don't: wording.
assert_eq!(error.message, "Couldn't move “PAY-7”: it changed since you read it");
// Do: the kind and its fields.
assert!(matches!(error.kind, Some(Kind::RevisionConflict(_))));

// Don't: call order on an internal fake.
assert_eq!(store.calls(), ["begin", "write", "commit"]);
// Do: what is true afterwards.
assert_eq!(world.item("PAY-7").await.column(), pb::BoardColumn::Done);

// Don't: a helper made visible for the test.
assert_eq!(board::rank_between("a", "c"), "b");
// Do: the order a person sees after the move.
assert_eq!(world.column_titles(pb::BoardColumn::ToDo).await, ["PAY-3", "PAY-7", "PAY-9"]);
```

## 10. Before a milestone is ticked

- Each new RPC: an access-table row, a request builder, its documented
  kinds provoked, and a resume test if it streams.
- Each new `MCP` tool: a line in `tools.txt`, a tier row, a host-away row
  and its refusals.
- Each new CLI command: its exit codes, its `--json` object, and actor
  tests when it is user-only or needs presence.
- Each new feature: exhaustive `TestStore` tests.
- Each spec flow it touches: a flow test named after it, with snapshots
  reviewed.
- macOS green. Linux noted as pending until the next batch (D18).

# Humata 2: the internal gRPC API

This is the contract between Humata's own processes: the app and `humatad` on
the Mac, the `humata` CLI and its local daemon, `humatad mcp` and its local
daemon, and the Mac's daemon and each remote daemon. The protos are in
[`proto/humata/v1/`](proto/humata/v1/).

Agents never see this API. They see MCP ([MCP.md](MCP.md)), which maps each
tool to one of the calls here and adds nothing. People and scripts use the
CLI ([CLI.md](CLI.md)), which does the same. Who may call what, and why no
agent can widen its own access, is in [BOUNDARIES.md](BOUNDARIES.md).

Where this document and SPEC.md disagree, SPEC.md wins and this document is
wrong. Requirement ids (SL-1, TP-9, …) refer to SPEC.md.

## 1. Layering

```text
 agent ──MCP stdio──▶ humatad mcp ──┐                       people, scripts
                     (one per       │ gRPC, unix socket        │
                      agent)        ▼ (0600)                   ▼
                              humatad (machine) ◀──── humata CLI on the machine
                                    │
                                    │ PeerService + relayed services,
                                    │ gRPC over SSH stdio (TR-2)
                                    ▼
 GPUI app ──gRPC, unix socket──▶ humatad (host, the Mac) ◀── humata CLI on the Mac
                                    ▲
 humatad mcp on the Mac ────────────┘
```

- AR-2 holds: the app, the CLI and every MCP server are gRPC clients. They
  differ only in the principal their connection is bound to (§3).
- Each machine's daemon is the only process its local clients talk to. A call
  the machine can't answer itself is forwarded to the host over the Uplink
  stream as a `ForwardedCall` and checked there (TP-8).
- Remote machines never talk to each other (AR-4, HO-9). A copy from devbox-1
  to gpu-box is two streams through the host.

## 2. Endpoints

| Endpoint | Where | Who connects | Services served |
| --- | --- | --- | --- |
| Host socket | `~/Library/Application Support/Humata/humatad.sock`, mode 0600, directory 0700 (SEC-4) | the app, `humata` on the Mac, `humatad mcp` of sessions on the Mac | every client service (§4) |
| Machine socket | `<root>/.humatad/humatad.sock`, 0600, directory 0700 | `humata` on that machine (AW-2, AW-5), `humatad mcp` of its sessions | DaemonService, TerminalService and FileService for its own sessions, and every other client service by forwarding to the host |
| Peer link | stdio of `humatad serve --stdio`, started by the host through the machine's connect command or SSH (MP-7) | the host's daemon only | PeerService, plus TerminalService, FileService, DiffService for relaying |

Until the app exists, `humatad2 serve` defaults to `~/.humata2/humatad.sock`
(state in `~/.humata2/state`), and `humata` tries the host socket, the
machine socket, then that one, in that order (`HUMATA_SOCKET` overrides all
three). The app's daemon will serve the host socket above.

No TCP port is opened anywhere (TR-5). PeerService is never registered on a
socket, so no local process on a machine (agent, CLI, script) can call it
there. That alone isn't enough, since any local process can also run
`humatad serve --stdio` itself, so the link serves nothing but the handshake
until the host proves it holds the machine's enrollment key (§8).

### 2.1 Keepalive and polls that remain (TR-7, TR-9, D54)

A link pushes; nothing asks again to see whether something changed. A
waiting call holds one stream (`WaitForAnswer`, `Subscribe`, `Attach`) and
its answer comes when it exists. The peer link sends an HTTP/2 PING after
30 seconds without traffic and ends the link when one goes unanswered for
20 seconds: that PING is how a dead link is noticed and the only timer an
idle link runs. The local sockets need none: a closed socket ends the
connection at once. The ssh carrying a link keeps its own
`ServerAliveInterval`, which matches (30 s in machines' `link.rs`, with
`ServerAliveCountMax` 4, so ssh gives up after two minutes unanswered).

Every interval or poll still in the daemon, the CLI and the MCP server,
and why no notification replaces it yet. Each runs only while what it waits
for is pending; none runs on an idle daemon.

| Where | Interval | Waits for | Why it isn't a notification |
| --- | --- | --- | --- |
| `WaitForAnswer` keepalive (humatad `inbox.rs`) | 15 s, only while a wait is open | nothing: it says the request is still pending, which MCP turns into `notifications/progress` (TP-11, TP-12) | it's a heartbeat the API promises, not a check; the answer itself is a watch channel |
| `humatad2 bridge` connect (humatad `bridge.rs`) | one retry after 0.5 s; past that, no interval | the daemon's socket to accept | a daemon the bridge starts writes a line on its `--ready-fd` pipe once it listens, so the bridge waits on the pipe (10 s at most) instead of retrying the connect |
| Holder scrollback sync (holder `server.rs`, `saved.rs`) | 500 ms, forever, per tab | new output to save | not this API's code; to become a notify on output with a 500 ms debounce |
| Holder hang-up (holder `server.rs`) | 20 ms, up to the hang-up grace | the program to exit | to become a watch on exit |
| Revive agent output (revive `agent.rs`) | the crate's `POLL`, while reviving | output and exit | to read the holder's event stream |
| Machine setup retries (machines `setup.rs`, `RetryPolicy`) | 2 s, doubling to 30 s, at most 10 attempts, only while `ADD_MACHINE`'s `connect` fails as a machine still booting would | ssh to answer | a machine that isn't up yet sends nothing to be told by; an ssh refusal or a host key change fails at once |
| Last good copy (humatad `last_good.rs`) | 30 s after the last committed write, at most 10 min while writes continue | writes to go quiet before saving a copy of the state (PER-31, D77) | it is a debounce, woken by the store's commit hook; it runs only while a write is pending |
| Remote service start (machines `service.rs`, run over ssh) | 1 s, at most 10 times, then a 2 s settle | the systemd user manager, then `humatad2.service` to be active, and an old daemon to exit | systemd's `is-active` and `pgrep` are what a POSIX shell has; it runs only during `install`/`start` |

The command runner (machines `system.rs`) no longer polls: a thread waits
on the child and the runner waits on that thread, with the command's
timeout as the only timer. `WatchOperation` pushes each change of an
operation, coalesced to the latest, and ends when it finishes.

`ListTabs` and `TypeInto` no longer connect to each tab's holder per
call: humatad keeps one connection to each holder (`holders.rs`) and reads
where the tab stands from it, so listing tabs or typing is a message on an
open socket. An attach coalesces output already queued, up to 64 KiB and
its window, into one `Output` message. A test (`serve.rs`,
`an_idle_daemon_with_a_connected_client_fires_no_timers`) holds a client
connection open to an idle daemon for ten hours of paused time and checks
no timer woke it.

Nothing watches agent conversation files, git state or `shared/` yet; when
they're read they're to come from file notifications (FSEvents, inotify),
never a rescan on a timer. POLLING-AUDIT-2026-11-14.md in `research/` has
the detail and the fixes.

### 2.2 Starting again (PER-29, PER-31, D77)

`humatad2 serve` binds its socket, then opens its state, restoring the last
good copy if the database is damaged, and serves at once. Bringing back the
agents and shells whose holders are gone runs beside serving, since an
agent may take 30 seconds to resume. Until that's done,
`GetServerInfo.last_start` is unset; then it names the restart report's
folder, has one `NotBroughtBack` line for each agent (by session) or shell
(by tab) that didn't come back, and after a restore says when the copy was
taken and how many rows it changed. Only the user gets `last_start`.

The restart report includes the last 50 lines humatad2 logged in the run
before (`logs/humatad2.previous.log`), so it can say what the daemon was
doing when it stopped.

## 3. Principals on a connection

Every connection is bound to one principal when it is accepted; no request
field names the caller. `DaemonService.WhoAmI` returns it. How each is
established is in BOUNDARIES.md §2; in short:

| Principal | Bound how | Notes |
| --- | --- | --- |
| App | Host socket, peer uid matches, and the app proves the client key it keeps in its keychain item | The only connection that can show `ConfirmInApp` dialogs |
| User via CLI | Socket peer uid matches and the calling process is not inside any session's process tree | Reads of what this daemon stores. Every write, every live read relayed from another machine, and every user-only call needs a `PresenceProof` or a CLI login (BOUNDARIES.md §2.2, §2.3), because a double-forked agent process also lands outside every tree |
| User via CLI, logged in | As above, and the process is in a terminal session that `humata login` bound (`CliAuthService`) | The present user: needs no further proof until the login expires (12 hours at most) |
| Script | `HUMATA_TOKEN` from `CreateScriptToken` | Only the token's capabilities on its channels; never a user-only one |
| Agent | `humatad mcp` presents the session token from its launch environment; the daemon checks the presenting process descends from that session's holder | Also any `humata` process inside a session's tree: it is that session's agent, not the user |
| Peer | The SSH stdio link the host itself started, then the mutual enrollment proof: `PeerService.Hello` (the machine's) and `PeerService.ProveHost` (the host's) | A machine may assert an `AgentPrincipal` only for sessions the host placed on it |

## 4. Services and which daemon owns them

"Owner" is the daemon whose state the call reads or changes and which decides
it. The host checks access for every call that crosses a machine boundary;
a machine checks only calls it answers alone, using the host's last
`PolicySnapshot` (§9).

| Service (file) | Owner | Served on | Notes |
| --- | --- | --- | --- |
| DaemonService (daemon) | each daemon | every endpoint | `GetServerInfo.last_start` says what the start brought back (§2.2); `BeginRelay` and `CompleteRelay` let the peer link prove itself to a machine's socket daemon (§8) |
| ChannelService, ActivityService (channels) | host | host socket; forwarded from machines | Channel records, repos, machines (SL-1) |
| SessionService (sessions) | host for records; the session's machine for holders, clones, conversation | host socket; forwarded | The host writes the record, then drives the machine through PeerService. `ReadConversation` reads the host's mirror (LD-8). `ReportTurn` is the session's own agent saying what turn it is in, from its hooks (D169): any other caller is refused, an older report than the last one kept is dropped, and repeating the state changes nothing. `Hook` (server stream) is the agent's blocking permission hook (D191): the daemon parses the payload, raises one Inbox card (kind SESSION, the whole command shown, Approve or Deny), sends a keepalive every 15 s and ends with the bytes the hook prints. The card lives only while a stream holds it: the same ask again holds the same card, the last stream dropping withdraws it (CALL_ENDED), Answer all skips it, and it is saved only once answered or withdrawn. At 3590 s it ends with no decision (D192) |
| TerminalService (terminal) | the session's machine | machine socket for its own sessions; host socket, relayed over the peer link | One HTTP/2 stream per attached tab end to end. `PasteImage` saves a pasted image (32 MiB at most) in the session's own folder on its machine, file 0600, gone with the session, and returns the path there for the client to type as input; a text paste is plain input |
| FileService (files) | the machine that has the files | machine socket for its own files; host socket, relayed | Every location is a Humata root plus a relative path |
| TransferService (files) | host | host socket; forwarded | The host drives both ends; chunks pass through it |
| TrashService (files) | each machine's trash; the host for session records | host socket; forwarded | |
| TransferEndService (files) | the machine that has the files | machine socket; host socket, relayed whole | The far end of a transfer the host drives: one call per step the engine takes on a local disk (D156). For the user and the host's relayed calls, never an agent |
| BoardService (boards) | host | host socket; forwarded, or journaled (§9) | |
| InboxService (inbox) | host; a machine while its host is away, for requests it holds (§9.4) | host socket; machine socket for `humata inbox` | |
| AccessService, AuditService, CliAuthService (access) | host | host socket; forwarded | The policy a machine applies offline is a copy (§9.2). A machine checks CLI logins and the approval passphrase itself while the host is away |
| DiffService, CommentService, ViewService, PullRequestReviewService (review) | host for comments, views, drafts, review state; the machine computes diffs | host socket; forwarded; comment and view drafts journaled | |
| ReviewChatService (review) | the review session's machine, which keeps the review chat's view state, notes and switches (PR-22–PR-26) | host socket; forwarded | The app reports its view; agents in the review change it through MCP (review_*), each change undoable. Switches off are denied with `SOURCE_REVIEW_SWITCH_OFF` |
| OperationService (operations) | host | host socket; forwarded | |
| MachineService, ProviderService, SkillsService (hosts) | host | host socket only for user-only calls; forwarded otherwise | Provider commands run on the Mac or `run_on` (MP-5) |
| BridgeService (hosts) | host; the forward's two ends are the session's machine and the Mac | host socket; forwarded from the session's machine | HO-10, HO-11: the only way anything but Humata's own calls crosses the bridge |
| SettingsService, AgentService, MarkService, ViewStateService (settings) | host | host socket | User-only writes (BOUNDARIES.md §5) |
| EventService (events) | host | host socket; machine socket relays a filtered feed | |
| SessionCarryService (sessions) | the machine a session leaves or reaches | machine socket; host socket, relayed. Host only | Moving a session (Move): `Pack` freezes the session and packs its work and conversation, `Unpack` lays it down on the new machine, `Release` deletes the old copy once the new one runs, and `Abort` undoes a half-done move. A fork onto another machine packs through the same calls: `CarryPackRequest.fork` (a `CarryFork`: the fork's id, title, kind, turn and typed branch) packs a copy and leaves the parent running (a typed branch git won't take, or one a parent on this Mac already uses, is refused when ForkSession is called; a box parent's taken names are checked at pack), and `CarryPackResponse.branch` and `conversation_id` give the fork's branch and conversation. `PreviewFork` answers, on the parent's machine, the branch a fork onto another machine would get and the bytes it would copy, staging nothing; the host's SessionService.PreviewFork uses it for a fork elsewhere. A context-only fork runs only on its parent's machine, so a fork or preview of one elsewhere is refused (D208). The host keeps a journal of each fork elsewhere at `<state>/carry/forks/<operation>.json`, written at every phase; after a restart it finishes a fork whose record was written and undoes any other one, staged files included. Only the host's relayed calls are honoured, never the user or an agent (D203) |
| PeerService (peer) | each remote machine | peer link only | §8 |

## 5. Conventions

### 5.1 Ids

Each id is its own message (`SessionId`, `ChannelId`, …) so the compiler
catches a session id passed as a channel id. Values are opaque ULID-based
strings; clients compare them and never parse them. Board item ids are the
human ones (`PAY-7`); items made in a journal get a provisional id
(`~devbox-1.3`) until the host replays them (§9.6).

### 5.2 Idempotency

Every mutation takes an `IdempotencyKey`, one per user intent. A retry with
the same key returns the first result and does nothing again, including
after a daemon restart. The host keeps keys for 30 days. A client that lost
the response to a call that started an operation finds it with
`OperationService.FindOperation`. Journal replays are deduplicated by
`JournalPosition`, which never expires.

### 5.3 Revisions

Every mutable entity has a `Revision` that goes up with each change (SL-7).
Updates name the revision they were made against in `expected_revision`; a
stale one fails with `revision_conflict`, which carries the current revision
so the client can show the newer state and ask again. Unset
`expected_revision` means "apply whatever is there now" and is allowed only
for fields that last-writer-wins already covers (board item fields, marks,
view state); the earlier value is then kept in history.

Deletes are tombstones kept in history (SL-8). An edit or move that names a
deleted item fails with `not_found` whose fix says it was deleted, and the
attempt is kept in history; it never brings the item back.

### 5.4 Plans before destructive calls

Archive, delete a session, delete a channel (CH-24), drop a repo, remove a
repo or machine from a channel, remove or forget a machine, empty trash,
change a root (`PlanChangeRoot`), import a provider (MP-23) and submit a
review are two calls (NV-27): `Plan*` returns what will happen (unpushed work, sessions affected,
bytes freed) and a `PlanToken`; the executing call passes the token back and
fails with `plan_stale` if anything the plan listed has changed since. This
turns the confirmation dialog into a check instead of a race: the user
confirmed the list they saw, not whatever is true a minute later.

### 5.5 Patches

Update requests use `optional` scalars and set-or-clear pairs for message
fields. Only fields present change. There are no field masks.

### 5.6 Paging

Lists take `PageRequest` and return `PageResponse`. A page token pins the
snapshot its first page read, so paging never skips or repeats an entry.
Tokens expire after 10 minutes (`invalid_field` naming `page_token`; start
again from the first page).

### 5.7 Reasons

Calls an agent can make that may become Inbox requests take a `reason`
string. It is shown on the card (IN-9) as the agent's words, marked as such.
It never changes what is checked.

## 6. Streams

| Stream | Keyed by | Resume |
| --- | --- | --- |
| `TerminalService.Attach` | byte offset of the tab's output | `AttachOpen.from_offset`; older than `earliest_offset` gets a `TerminalSnapshot` |
| `EventService.Subscribe` | `EventSeq` | `after`; older than the oldest kept gets `Resync`, then the present |
| `OperationService.WatchOperation` | operation `version` | `from_version` |
| `TransferService.WatchTransfer` | transfer `version` | `from_version` |
| `FileService.ReadFile`, `PeerService.ExportConversation` | byte offset | `offset` |
| `TransferService.Upload`, `FileService.WriteFile` | byte offset | `UploadHeader.resume` |
| `PeerService.Uplink` | `UplinkAck` per kind | `UplinkResume` both ways after Hello |

Rules:

- Gapless and without repeats (TR-4, PER-3). A resumed stream starts exactly
  after the last offset the client names.
- Flow control is per stream. Attach sends at most `window_bytes` of
  unacknowledged output; a client that falls further behind than the kept
  scrollback gets a fresh snapshot instead of an ever-growing buffer.
- The host never lets one machine stall another (TR-6): every relayed stream
  has a bounded buffer per direction (1 MiB by default). When a remote's
  buffer is full, that stream stops reading from it; other streams on the
  same SSH connection go on. When the app's side is full, the host stops
  relaying that tab and the app catches up from the offset later.
- A machine's events reach the host's feed only after the host has
  committed them; the feed never shows a state the host doesn't have.
- Event payloads are the entity after the change, so clients update in place
  without a fetch. `PointersUpdated` is coalesced to one per session per
  second.
- A `Pointer` also names the repo's dirty paths and unpushed commit
  subjects (`dirty_paths`, `unpushed_subjects`, 50 of each at most), read
  in two git passes that must agree. The host keeps the last ones, so
  `PlanForgetMachine` can list a lost machine's work without reaching it,
  and `ArchiveSummary.Repo.unpushed_subjects` carries them into the
  archive.

Forget and rebuild (LD-11, LD-12, SL-4). `ForgetMachine` is user-only. It
archives each session on the lost machine as machine-lost from the host's
records, takes the machine out of every channel and forgets it on this
host without dialing it: its entry, peer key and pinned host key go, so
adding it again makes a new machine. What each idempotency key answered
is kept in `forget-answers.json` beside `hosts.json` (the newest 256), so
a retry after a host restart answers the same. `RebuildSession` sends the target
machine a host-only `StartSession` with `rebuild` set (a `RebuildFrom`:
each repo's branch, the folder name, the old folder and where the mirror's
scrubbed conversation lines were staged). The host copies those lines as a
file to `.carry/<name>/conversation.jsonl` in the channel's folder on the
machine, the way a move copies files, so no message size bounds them, and
drops its own copy. The machine clones each branch at its origin
tip, the last pushed sha, writes the lines with the old folder's paths made
its own, removes the staged folder, resumes the agent and tells it what
was lost. The record moves to the new machine only once that succeeds.
`RebuildFrom.conversation`, the lines inline, is read only when nothing
was staged.

What humatad2 builds of `EventService` so far (D82): machine events only
(`Machine` payloads, to the user; an agent sees none, as ListMachines
lists none), each written to the machine list before it is sent. The
feed is kept in memory, its last 1024 events: numbers start from the
daemon's start time, so an `after` from an earlier run, or one older than
the oldest kept, gets `Resync`. A subscriber that falls more than 1024
events behind gets `Resync` and goes on from the present. `by` is unset.

### 6.1 Chunks and compression (TR-11, TR-13, D60, D65)

Byte streams travel in chunks of at most 1 MiB (`humata2_chunk::CHUNK_MAX`),
each compressed on its own so a resumed stream can switch codec mid-file.

- **Codec ids** are `ChunkCodec` numbers, never strings: 0 none, 1 lz4,
  2 zstd. The one-byte id at rest is the same number. Ids are never reused.
- **Negotiation:** each end lists the codecs it decodes, best first.
  `ServerInfo.chunk_codecs` is what a daemon offers (a writer picks the
  first it supports); `ReadFileRequest.accept_codecs` is what a reader
  decodes (the daemon picks from it in its own order). An empty list means
  "none only", so an old peer still gets plain bytes.
  `HelloRequest/HelloResponse.chunk_codecs` carry the same for the peer
  link; a machine's Hello lists what its setting ranks for a remote link.
  `AttachOpen.accept_codecs` is what an attaching client decodes.
- **Policy:** `compression.toml` in the daemon's state folder,
  `compression = "auto" | "lz4" | "zstd" | "off"`, with
  `[machines.<id>] compression = ...` overrides; read per stream. Auto is
  lz4 locally and on fast links, zstd when the link's measured throughput is
  under 50 MB/s. Off offers only none.
- **Throughput** is measured from real transfers, never probes or timers:
  each finished ReadFile or WriteFile of at least 1 MiB on the remote link
  gives a rate (encoded bytes over first-to-last chunk). It counts as the
  link's rate when this end spent at least half that time waiting on the
  link; otherwise only when it is already fast. The estimate averages the
  old one with each new rate, so a link that changes is noticed within a
  few transfers. An unmeasured link is taken as fast.
- **Chunk fields** (`FileChunk`): `codec`, `raw_size` (the decoded length;
  0 with none means `len(data)`), `crc32` of the decoded bytes (required
  for lz4 and zstd). Chunks under 512 bytes, chunks that save less than
  1/16, and streams that start with a known compressed format (images,
  archives, media) go as none.
- **At rest** (store rows, blobs, snapshots, reports) the same codecs sit
  behind a 22-byte little-endian frame header: magic `\x89H2Z`, header
  format (1), codec id (u8), message type id (u16), format version (u16),
  decoded length (u64), CRC-32 (u32). See `humata2_chunk::encode_framed`.
- **Terminals:** `TerminalOutput` and `TerminalSnapshot` carry the same
  `codec`, `raw_size` and `crc32`; offsets, windows and acks count decoded
  bytes. To a client that lists codecs, a snapshot over 1 MiB comes as
  several `TerminalSnapshot` messages with the same offset, `more` set on
  all but the last. A client that lists none gets each snapshot whole and
  raw, as before.
- **Applied so far:** `FileService.ReadFile` and `WriteFile` (session
  clones on the local socket), `humata files get/put`, terminal attach
  snapshots and output (`humata attach`), and the peer-link Hello lists.
  FileService isn't on the peer link yet, so the remote throughput is
  measured only once it is. `TransferService.Upload`, diffs and bug
  reports are not compressed yet.

## 7. Operations and cancellation

Long work (start, fork, move, archive with push, restore, rebuild, give own
copy, push and open PRs, add repo, add/remove machine, change root, provider
steps, skills sync, PR review start/update/submit, delete a channel) returns an `Operation` at
once. Its steps have stable keys, shown as the progress list the spec draws.

- Closing a stream or dropping a connection never cancels a mutation. Only
  `CancelOperation` does.
- `CancelOperation` stops at the next step boundary and undoes what the table
  below says. After the point of no return it fails with `not_cancellable`,
  whose `fix` says what to do instead (usually let it finish, then undo
  with the reverse action).
- `RetryOperation` reruns a failed operation from its failed step (HO-2,
  MP-20); steps already done are not repeated.
- Failed steps keep their output tail (MP-20).

| Kind | Step keys | Point of no return | Cancel undoes |
| --- | --- | --- | --- |
| START_SESSION | `write_record`, `clone:<repo>`…, `launch`, `verify` | `launch` | Removes clones and the record (never started, so nothing to trash) |
| FORK_SESSION | `write_record`, `copy_code:<repo>`…, `copy_conversation`, `launch` | `launch` | Same as start; the parent is never touched |
| MOVE_SESSION | `check_target`, `bundle:<repo>`…, `send`, `prepare_clones`, `copy_conversation`, `stop_old`, `launch_new`, `verify`, `retire_old` | `stop_old` | Deletes what reached the target; the session keeps running where it was |
| ARCHIVE_SESSION | `commit:<repo>`, `push:<repo>`, `open_pr:<repo>` (when chosen), `stop`, `summarize`, `free_clones` | first `push` | Before: nothing happened. After: finishes; restore undoes |
| RESTORE_SESSION | `clone:<repo>`…, `import_conversation`, `launch`, `verify` | `launch` | Removes the new clones; stays archived |
| REBUILD_SESSION | `clone:<repo>` (branch, else last pushed sha), `import_conversation`, `mark_lost`, `launch` | `launch` | Removes the new clones; stays archived |
| GIVE_OWN_COPY | `clone:<repo>`, `switch` | `switch` | Removes the new clone |
| PUSH_AND_OPEN_PR | per repo `commit`, `push`, `open_pr` | first `push` | Before: nothing. After: finishes the repos already pushed, skips the rest, and says which |
| ADD_REPO | `clone_main:<machine>`…, `record` | `record` | Removes clones made |
| ADD_MACHINE | `connect`, `check`, `install`, `start`, `find_agents` | none | Stops the daemon it started and removes the binary it installed |
| REMOVE_MACHINE | `dispose:<session>`… (each a move or archive), `leave_channels`, `stop_daemon`, `provider_delete` | `stop_daemon` | Sessions already moved or archived stay so; says which |
| CHANGE_ROOT | `copy`, `verify`, `switch`, `retire_old` | `switch` | Removes the copy |
| PROVIDER_CREATE | `create`, `identity`, `connect`, `check`, `install`, `service`, `register` | `create` once it ran | Before `create` finished: nothing. After: offers Delete what was made (MP-20) |
| PROVIDER_ACTION | `run` | `run` | — |
| SKILLS_SYNC | per machine `diff`, `upload`, `swap`; an offline machine's steps are skipped and named (SK-10) | `swap` (the whole set at once, SK-9) | Removes the staged copy |
| START_PR_REVIEW | `resolve:<pr>`…, `clone:<pr>`…, `launch` | `launch` | Removes clones |
| UPDATE_PR_CLONE | `fetch`, `reset`, `reattach_drafts` | `reset` | Nothing changed |
| SUBMIT_PR_REVIEW | `post:<pr>`… | first `post` | Finishes; a failure after the first post returns `review_partly_posted` naming which PRs got it |
| DELETE_CHANNEL | `stop_agents`, `withdraw_requests`, `trash:<machine>`…, `remove_record` | `stop_agents` | Before: nothing. After: finishes; offline machines' `trash:` steps wait for them; restoring the Mac's trash item undoes it within 30 days (CH-24) |

A step that finished may carry `facts`, what it read, by name; a fact that
isn't known is left out, and the app words them. The `check` step of
ADD_MACHINE and PROVIDER_CREATE carries `os` and `arch` (spelled as
`Machine.os` and `Machine.arch`), `git` and `gh` (full version numbers),
and `disk_free_bytes` (a decimal number).

What humatad2 builds of this so far (D61): `ADD_MACHINE` and
`REMOVE_MACHINE` only, kept in memory (a restart forgets finished
operations; a machine left half added or half removed is finished or
undone when the daemon starts, PER-27). `CancelOperation` stops an
`ADD_MACHINE` at its next step boundary (between steps, or between connect
attempts) and undoes it, and the operation ends `CANCELLED`; cancelling
one that already ended answers with how it ended. `REMOVE_MACHINE` is
past its point of no return from its first step, so cancelling it fails
with `not_cancellable`. `MOVE_SESSION` (its steps are `check`, `pause`,
`pack`, `copy`, `verify`, `resume`, `switch`, `release`) stops at its next
step boundary until the old agent stops for `resume`: what reached the
target is deleted, the session keeps running where it was, and the
operation ends `CANCELLED`. From `resume` on it shows `cancellable` false
and a cancel fails with `not_cancellable`; one asked just before that
point wins and undoes the move. A failed add is undone at once, so
`RetryOperation` runs a failed operation's work again from its first step
under the same id (D81); a retry resent with the same key answers with
that run, and any other state fails with `invalid_field` on `id`.
`REMOVE_MACHINE` runs `stop_daemon` and `forget`; `dispose:`,
`leave_channels` and `provider_delete` come with sessions, channels and
providers.

## 8. The peer link

`PeerService` (peer.proto) is served by every remote daemon and called only
by the host over the stdio link the host started.

- **Hello and ProveHost** prove each side holds the enrollment key: when
  the host installs `humatad` it writes a random key of at least 32 bytes,
  as hex in a 0600 file in the machine daemon's state folder
  (`enrollment-key`), and keeps its own copy (for now `peer-keys/<machine>`,
  0600, in the host's state folder; later the Mac's keychain, D84). Hello
  sends 32 to 64 random bytes as `nonce`; the machine answers with 32 of
  its own as `machine_nonce` and the proof
  HMAC-SHA256(key, "humata-peer-machine-v1" ‖ nonce ‖ machine_nonce). A
  wrong proof is `identity_mismatch` on the host (the machine's reachability
  becomes `REACHABILITY_IDENTITY_MISMATCH` and the host stops dialling): the
  machine was re-imaged or swapped, and LD-12 applies (add it again as a
  new machine). The host
  then calls ProveHost with
  HMAC-SHA256(key, "humata-peer-host-v1" ‖ machine_nonce ‖ nonce). Until
  that succeeds the link answers only Hello, ProveHost and
  `DaemonService.WhoAmI`; everything else, `GetServerInfo` included, is
  `peer_unproven` with reason `HOST_NOT_PROVEN`. A wrong host proof is
  `peer_unproven` `HOST_PROOF_WRONG`, and the machine closes the link and
  exits with a failure. A link has one Hello, whatever its outcome; a second,
  or ProveHost before Hello or after it succeeded, is `OUT_OF_ORDER` and
  leaves the link open. A machine with no key file refuses Hello with
  `NOT_ENROLLED`, and one whose key file other users can open or don't own,
  or that is a link, with `KEY_UNSAFE`: its fix says not to just tighten
  the mode, since the key may already have been read or replaced, but to
  forget the machine and add it again. Once the link is up its stderr goes
  to `logs/peer-link.log` (0600) in the state folder, so no error line can
  corrupt the stream. This keeps out a swapped machine and a contained
  agent posing as the host. An uncontained agent (SPEC D1) runs as the user
  and can read the key file, so it can still pose as the host.
- **Uplink** is opened right after Hello and lives as long as the link.
  Machine to host: pointers (SL-3), mirror chunks (LD-8), status, journal
  entries (SL-5), forwarded calls from its local clients, local answers,
  push requests (FC-7). Host to machine: acks, forwarded results, policy
  snapshots, Inbox answers and withdrawals made elsewhere, push results.
  Each side acks what it has stored durably; after a reconnect both resume
  from the last ack, so nothing is lost or applied twice.
- **Relayed calls (D139, D144, D146).** After ProveHost the link relays the
  calls the host may make (terminal, file, trash and diff; session start,
  stop, resume, get, list and branch-name preview; channel create and get;
  operation get, watch and cancel; the event feed; HistoryService/ReadMirror)
  to the machine's own socket
  daemon, which runs them as the user's, marked as from the host. Before
  answering Hello the link calls `DaemonService.BeginRelay` on that socket and
  offers the nonce it gets as Hello's `machine_nonce`; when ProveHost succeeds
  it calls `CompleteRelay` with the host's nonce and proof as received. The
  daemon checks the host's proof itself, over that connection's nonce, and
  only when its role is machine and the peer process runs as its user outside
  every session's terminal. Both calls are open to any caller and grant
  nothing alone. Until then, and for any method off the list, relayed calls
  are refused. A link started with `--state` and no `--socket` relays nothing
  and answers relayed calls as unavailable.
- **The conversation mirror (D189).** `HistoryService.ReadMirror`, on a
  machine, returns the next part of a session's conversation for the host's
  mirror, from `offset`: complete lines, scrubbed of host variable values
  (LD-21), at most 1 MiB before encoding, with the file's `source` identity,
  `rewritten`, `more` and codec fields as FileChunk. Only the host calls it,
  and only through the relay (the one HistoryService method on the D139
  allowlist); every other caller, on any connection, gets permission_denied.
  Offsets count the machine's own file bytes, not scrubbed bytes.
  `SearchSessions` and `ReadTurns` are never relayed: the host keeps the copy
  and answers them itself.
- A forwarded call or journal entry names an `AgentPrincipal`. The host
  accepts it only for a session it placed on that machine. A machine taken
  over by an attacker can therefore act only as its own sessions, never as
  another machine's or as the user.
- A machine-side `humata` user call (AW-5) is forwarded as `MachineUser` with
  the approval passphrase; the host checks the passphrase, never the
  machine.
- Holder, clone, conversation and skills calls are host-driven steps of the
  operations in §7. They are idempotent by key, so a step retried after a
  dropped link does no double work.
- Versions: the host serves `api_minor` N and accepts machines from N-2. An
  older machine fails Hello with `peer_version_unsupported`; Hosts offers
  Update, which reinstalls `humatad` while holders keep running (PER-5,
  PER-6).

## 9. When the host is away

A machine considers its host away when its Uplink is down. The Mac being
asleep, off or offline all look the same from the machine (AW-1). What
follows applies to calls from the machine's own clients (its agents' MCP
servers and `humata` run there).

### 9.1 What the spec asks

AW-7 says what each call does with the Mac away, AW-8 that the machine
judges calls against its last copy of the policy, AW-9 that journaled
writes are checked again at replay, AW-10 that the first answer wins, AW-11
what low-risk can never include and AW-12 how a grant given on the machine
reaches the Mac. SL-5 says only AW-7's journaled writes and requests made on
the machine are journaled; SL-6 to SL-8 say how replays meet the host's
state. The sections below implement them; none of D3's open picks is
decided here.

### 9.2 The machine's copy of the policy

While connected, the host sends a `PolicySnapshot` on every change: the rules
of each channel with a session on the machine, the grants of those sessions,
the While I'm away rule and its low-risk kinds, and the board items those
sessions own or may edit. The machine stores it and applies only newer ones.
While the host is away the machine judges calls against it. The snapshot
never holds a secret, and no part of it can be changed on the machine: there
is no call for it there.

### 9.3 What happens to each call

| The call | Host reachable | Host away |
| --- | --- | --- |
| Reads of its own session's files and clones | applied (machine) | applied (machine) |
| Writes to its own clones and `shared/` (TP-2, FC-8) | applied (machine) | applied (machine) |
| Reads of host state (channel topology, boards, its comments) | applied (host) | answered from the last snapshot and mirror, marked `stale_as_of` |
| Host-state writes the snapshot allows (own board, channel board add, edit its own items, comment and view drafts, `inbox_ask`) | applied | **journaled**: response `{outcome: journaled}` with provisional ids |
| Anything the snapshot says Never, or a hard limit | denied | denied |
| An Ask on something on this machine (another session's files here, start or fork here, type into a session here) | Inbox request; the call waits | per the away rule: **Wait** (request journaled, the call waits, answerable on the machine), **Skip** (denied with `user_away`), **Allow low-risk** (applied if the kind is low-risk and journaled for the record, else Wait) |
| Anything that needs another machine or GitHub through the Mac (read or copy from another machine, start or fork elsewhere, a note to a session elsewhere, push, port forward) | applied or asked | **waits** (TP-9) or, under Skip, denied with `host_unreachable`. Never low-risk (AW-11): the host must route it. A port forward can't be approved on the machine (HO-11) |
| User-only calls | per BOUNDARIES.md | only answering requests the machine holds, with the approval passphrase |

### 9.4 Answers

An Inbox request made while the host is away lives in the machine's journal.
It is shown by `humata inbox` on that machine and nowhere else until the link
returns (AW-3).

- **The first answer at the site that holds the call wins.** While the host
  is away, that is the machine: the user answers there with the approval
  passphrase (AW-5), the machine acts on it at once, and sends a
  `LocalAnswer` up on reconnect.
- On reconnect the journal replays (AW-6). A request already answered
  locally arrives answered, attributed to the user via CLI on that machine.
  A request still pending becomes an ordinary Inbox request on the host,
  and from then on the host holds it; its answer goes down as an
  `InboxAnswerRelay`.
- A second answer anywhere fails with `already_answered`, naming who
  answered and how. Last-writer-wins never applies to answers: an approval
  that ran can't be taken back by a later deny, and treating them as
  last-writer-wins would show a deny for something that happened.
- AW-12: "Always allow" from a local answer applies on the machine at once and
  creates its grant on the host at replay, with the same target (IN-22), so
  the snapshot carries it from then on.

### 9.5 Board and comment conflicts (SL-6)

The host replays each machine's journal in `seq` order, interleaved with its
own writes in arrival order. For each entry:

1. **Re-check access** against the current rules. IN-21 says rule changes
   apply to the next call; a journaled call hasn't reached the host yet, so
   it is judged again. A rule tightened while the host was away rejects the
   entry (`journal_entry_rejected`, recorded in history, sent to the agent's
   terminal as one line). A loosened rule changes nothing.
2. **Compare `base_revision`** with the item's current revision. Equal: apply.
   Different: apply field by field; fields the host changed since the base
   keep the later write in host commit order, which is the replayed one; the
   other value is kept in history with `lost_conflict` (BD-3).
3. **Deletes are tombstones.** An edit or move replayed onto a deleted item
   does not bring it back; it is kept in history. A journaled delete made
   against an older revision than the current one is applied only if no
   field it would delete changed since its base; otherwise the item stays
   and the delete is kept in history as lost. Nothing the user typed is lost
   by a replay.
4. **Moves** treat column and rank as one field.

### 9.6 Provisional ids

Items created in a journal get `~<machine>.<seq>`. On replay the host gives
the final id, rewrites later journal entries from the same machine that name
the provisional one, and emits `ProvisionalIdResolved`. For 30 days the host
also accepts the provisional id anywhere an id is taken, so an agent holding
it keeps working.

### 9.7 The journal itself

Append-only, fsynced per entry, at `<root>/.humatad/journal/`. The host acks
`journal_through` once entries are applied or rejected; the machine deletes
entries at or below the ack. `HelloResponse.journal_high_water` tells the
host what to expect. If the disk is full, a journaled write fails with
`disk_full` rather than being dropped.

## 10. Errors

Every failure is a gRPC status whose details hold one `humata.v1.Error`
(errors.proto) in `google.rpc.Status.details`. `Error.message` says what
failed; `Error.fix` says how to fix it, and says so when the obvious fix is
unsafe; `Error.retry` says whether waiting helps and until when. The status
code is for generic handling; clients branch on `Error.kind`, never on text.

| gRPC code | Error kinds |
| --- | --- |
| `NOT_FOUND` | `not_found` |
| `INVALID_ARGUMENT` | `invalid_field`, `outside_humata_folders` |
| `ALREADY_EXISTS` | `name_taken` |
| `PERMISSION_DENIED` | `denied`, `user_only`, `presence_required`, `not_in_channel`, `parent_read_only`, `main_clone_read_only`, `push_refused` |
| `UNAUTHENTICATED` | `identity_mismatch`, `peer_unproven` (the peer link's handshake: which step failed is in its `reason`) |
| `ABORTED` | `revision_conflict`, `plan_stale`, `already_answered` |
| `FAILED_PRECONDITION` | `unpushed_work`, `session_busy`, `agent_lacks_feature`, `resume_failed`, `request_withdrawn`, `user_away`, `pr_closed`, `skill_name_clash`, `login_needs_user`, `host_key_changed`, `not_cancellable`, `journal_entry_rejected`, `linger_unavailable`, `daemon_already_running`, `daemon_not_running`, `repo_not_visible`, `github_cli` (not signed in, missing scope), `provider_identity_invalid` |
| `UNAVAILABLE` | `machine_offline`, `host_unreachable`, `ssh_failed` |
| `RESOURCE_EXHAUSTED` | `disk_full`, `rate_limited` (its `fix` says to wait until `resets_at`; a token only raises the limit, it doesn't lift this one), `passphrase_locked` (SEC-13: five wrong tries; its `fix` says to wait until `until` or answer from the app, and that trying again early does not shorten the lock) |
| `OUT_OF_RANGE` | `offset_trimmed` |
| `UNIMPLEMENTED` | `peer_version_unsupported` |
| `DATA_LOSS` | `checksum_mismatch` (its `fix` says the bad copy was removed and the source is untouched) |
| `UNKNOWN` | `git_failed`, `provider_step_failed`, `step_failed`: an external tool failed; its output tail is attached. `review_partly_posted`: GitHub took some of a review; it lists what was posted and what wasn't, so a retry posts only the rest |
| `INTERNAL` | `internal` (incident id and log path, never a stack trace) |
| `DEADLINE_EXCEEDED`, `CANCELLED` | the transport's own; no `Error` |

`host_key_changed` is not retryable and its fix never says "accept the new
key": it says the key differs from the one pinned, that this is what an
interception looks like, and how to check the machine's real key before
`TrustNewHostKey`.

A first add has no pinned key: `AddSshMachine` without
`trust_host_key_fingerprint` fails at `connect` with `host_key_changed`
whose `expected_fingerprint` is empty and `presented_fingerprint` is the
machine's key, for the user to check and confirm by sending the add again
with that fingerprint. A
fingerprint that doesn't match what the machine shows fails the same way
and nothing is sent to it.

## 11. Versioning

- The package is `humata.v1` for its whole life. Changes are additive only:
  new fields, messages, enum values, methods and services. A removed field's
  number and name are `reserved`. Renaming a field is a removal.
- `ServerInfo.api_minor` grows with every addition. Clients gate new calls on
  it and treat unknown enum values as UNSPECIFIED.
- The app and its host daemon ship together; the CLI and `humatad mcp` are
  the same binary as `humatad`, so a machine's clients always match its
  daemon. Only the peer link spans versions (§8).
- A breaking change would be `humata.v2`, served beside v1 for one release.

## 12. What is not here

- UI-only state and commands: Expand all / Collapse all (NV-18, NV-19) are
  app commands; their result is stored like any fold state with
  `ViewStateService`. Pane sizes, scroll, which rows are shown (SE-13,
  NV-16) the same.
- The command palette (KP-1..6) has no API of its own: each command is one of
  the calls here, and agents can't reach it (KP-6).
- The holder protocol between a daemon and its holders is private to the
  machine and versioned with it (PER-6).

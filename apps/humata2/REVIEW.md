# Humata 2 spec review

A fresh read of [SPEC.md](SPEC.md) (2265 lines, 2026-11-14) while designing
its API boundaries ([api/GRPC.md](api/GRPC.md), [api/MCP.md](api/MCP.md),
[api/CLI.md](api/CLI.md), [api/BOUNDARIES.md](api/BOUNDARIES.md)). Each
finding says what is wrong, what the API assumes for now, and what the spec
should decide. Ranked by how much each one blocks building or puts the user
at risk.

## Critical

**C1. Agent access rules are not a boundary against an agent running as the
user.** IN-16..21, SEC-3 and AR-4 read as if "Never" stops an agent. Agents
run as the user's uid: an agent can read other sessions' clones, `~/.ssh`,
the gh token and Humata's own store straight from disk, and open any network
connection, without calling Humata. The rules only govern calls through
Humata. *API now:* every Humata call is checked; the CLI can't be used to
become the user (C2); BOUNDARIES.md §1 states the gap. *Spec should:* say
the rules are guidance for cooperative agents, or add a **Contain agents**
setting that starts each agent in an OS sandbox (macOS `sandbox-exec`,
Linux Landlock and a network namespace) allowing only its session folder,
`shared/` and Humata's bridge socket. Without one of these, IN-17's "Reach a
machine outside its clones: Ask me" can't be honoured (C5).

**C2. The spec never says who the CLI is, so an agent can approve its own
requests.** AW-2, AW-5 and §26.10 have the user run `humata inbox` and
approve. An agent can run the same command in its own terminal. If the CLI
counts as the user, every Ask me is advisory. Process ancestry doesn't
settle it: a double-forked process leaves the session's tree. *API now:*
inside a session's tree `humata` is that agent; outside, it is an unverified
user who may only read local state; writes and user-only commands need
presence (a confirmation in the app, `humata login` for the terminal
session, or the approval passphrase while the host is away); scripts use
scoped tokens (BOUNDARIES.md §2.2, §2.3; `CliAuthService`). *Spec should:*
adopt a presence rule for the CLI and say how the user proves presence on a
machine over SSH with the Mac closed. The approval passphrase is new.

**C3. The offline rules contradict each other.** TP-9 says cross-machine
calls wait and only writes "that need no answer" are journaled. AW-4 says
requests that would ask follow While I'm away (Wait, Skip, Allow low-risk).
SL-5 says Inbox requests are journaled, and SL-6 says conflicts resolve last
writer wins, which can't apply to Inbox answers: a deny written after an
approval that already ran would show a deny for something that happened.
Also undefined: who checks a call while the host is away, against which copy
of the rules, and whether a journaled write is checked again at replay.
*API now (GRPC.md §9):* the machine judges from the host's last policy
snapshot; anything that needs another machine or GitHub waits, never
low-risk; asks on this machine follow the away rule; the first answer at the
site holding the call wins and a second gets `already_answered`; every
journaled write is re-checked on the host at replay and kept as rejected
if the rules changed; SL-6 applies field by field against a base revision.
*Spec should:* replace TP-9, AW-4, SL-5 and SL-6's overlap with one table of
call kind × host state.

**C4. Waiting tool calls will be killed by the agents' own timeouts.**
TP-9, TP-10 and IN-12 have an MCP call wait until the user answers, which
may be hours. MCP clients time out calls (often 60 seconds to 10 minutes)
and many don't reset on progress. *API now (MCP.md §4):* calls wait up to
`wait_seconds` (default 45) with progress every 15 s, then return
`{outcome: "pending", request_id}`; `inbox_wait` waits again; approval runs
the action under its original key whether or not anyone waits, and the
result is kept for 24 hours. *Spec should:* say what the agent sees when it
stops waiting, and whether an approval with nobody waiting still acts.

**C5. q7 blocks the bridge, and the network rule has nothing to enforce.**
IN-17 has "Reach a machine outside its clones (network)", HO-* draws a
bridge, and the spec MCP name `humata.bridge.connect` exists, but q7 (what
may cross the bridge, and which always ask) is open. With no sandbox (C1) an
agent opens sockets directly, so the rule gates only Humata's forwarder.
*API now:* `network_connect` opens a forwarded unix socket through the Mac,
needs NETWORK_REACH (Ask me), never low-risk, never offline. *Spec should:*
answer q7 and say what the network rule promises.

## High

**H1. "Always let …" has no defined scope.** IN-14 says "scoped exactly as
its label says (this session, this target)" but never defines a target per
kind of action (a file? a session? a machine? a board?), whether a grant
outlives the session, or whether forks inherit it. *API now:* the daemon
computes one standard scope per capability (this session plus the narrowest
named target); grants end with the session; forks and rebuilt sessions get
none (BOUNDARIES.md §4).

**H2. One IN-17 row mixes something an agent may be granted with something
it must never be able to get.** "Change channel settings, repos or access
rules: Never" means a user can set it to Allow, letting agents change access
rules, which widens every agent's grant. *API now:* access rules are
user-only (no rule, no tool); CHANNEL_CHANGE covers settings and repos only.
*Spec should:* split the row.

**H3. Binding an agent to its session is not specified.** TP-* and IN-*
assume the daemon knows which session is calling. *API now:* a per-launch
session token in the environment, checked against the process tree at
connect, rotated on resume; a mismatch is `identity_mismatch` (BOUNDARIES.md
§2.4).

**H4. Several features depend on knowing when an agent's turn ends, and the
spec never says how.** SM-10 queues comments "until the turn ends", IN-12
prints on answer, and the status set (working, idle, needs input) drives the
sidebar and the board. For terminal agents this needs per-agent hooks or
output heuristics. *API now:* `AgentDefinition.supports_turn_signal`; when
false, status is `RUNNING_UNKNOWN_TURN` and queued typing waits for the user
to send it. *Spec should:* name the source per built-in agent.

**H5. Concurrent edits have no revision model.** SL-6 says "last writer
wins", but the app, CLI, agents and journals can all edit a board item, and
the spec has no way to see that an edit was made against stale data.
*API now:* every mutable entity has a revision; writes may give
`expected_revision`; journaled writes carry their base revision and resolve
field by field, with the loser kept in history.

**H6. Destructive confirmations race with changes.** Archive, delete,
remove a machine, forget a machine, drop a repo, empty trash and submit a
review show "what will be lost", but the list can change between the dialog
and the click. *API now:* Plan* calls return a plan token; the executing call
fails with `plan_stale` if the plan's contents changed.

**H7. There is no way to delete a channel.** CH-* creates, renames and
changes channels; nothing removes one. *API now:* PlanDeleteChannel and
DeleteChannel exist, user-only, flagged as unrequired.

**H8. The conversation mirror sends every agent's transcript to the Mac,
with no rule for secrets in it.** LD-8 mirrors continuously; agents often
print environment values and tokens. SEC-1 says host variable values never
cross, but they will inside transcripts. *Spec should:* say whether the
mirror is scrubbed of the machine's variable values before it leaves the
machine, and how long it is kept (LD-16..18 cover clones and logs).

**H9. Two push paths with different rules.** MV-12 and FC-7 push a branch
and open a PR through the Mac; PR-8 lets a review session's agent push only
when the user allows it. The spec doesn't say whether a normal session's
agent may push without asking. *API now:* GIT_PUSH, Ask me; review sessions
ignore it and use PR-8 only.

**H10. Dotted MCP tool names break common agent clients.** The spec names
tools `humata.review.draft`, `humata.view.prepare` and so on. Several
clients reject dots in function names. *API now:* tools are `review_draft`
on server `humata`; the dotted form is the operation id used in audit and
history (MCP.md §2).

## Medium

**M1. Broken and missing references.** SE-4 cites SE-20 for the session
record (should be PER-11); SE-15 cites PER-16 for the pinned conversation id
(should be PER-12); FC-8 cites TP-5 for copying into `shared/` (should be
TP-4). §24 numbers ST-15 and ST-16 between ST-2 and ST-3, and §22 places
NV-11 after NV-23. The open questions skip q3 and q10 in the list (answered
below it).

**M2. Where UI state lives after AR-3.** AR-3 says the app holds no state that
matters, yet SE-13 (expanded rows), NV-16, NV-19 (collapsed headings) and
NV-21 (pinned order) are state the user expects back. *API now:*
`ViewStateService` keeps opaque per-key blobs on the host.

**M3. Requirements worded so they can't be tested.** For example "feels
instant", "as the CLI shows any pending tool call" (TP-10), and "cost" on
the fork card (TP-6) with no price source. *API now:* the fork card carries
a usage note in words.

**M4. How a question to another session gets its answer is undefined.**
TP-5 says the answer "is returned to the asking agent's call", but the
target is a terminal agent that only sees typed text. *API now:* the
question is typed with an id, and the target answers through
`session_answer`; if it never does, the asker's call returns `pending` like
any wait.

**M5. "Addressed" is decided automatically with no rule.** SM-11 marks a
comment addressed "when the agent changes its passage". Reformatting, moves
and nearby edits make this ambiguous. *Spec should:* define the match
(passage text gone from the file, or lines in the hunk).

**M6. Provider scripts run with the user's full rights.** MP-* runs
user-written create, start, stop and delete commands on the Mac or
`run_on`. *Spec should:* say a provider TOML from a repo is never trusted
without the user reading it, and whether agents may run provider actions
(the API: MACHINE_PROVISION Ask me, MACHINE_DESTROY Never).

**M7. Skills sync gaps.** SK-1..8 don't say what happens to a skill an agent
is using mid-turn, or to a machine that's offline at sync. *API now:*
PreviewSkillsSync returns a plan with clashes; SyncSkills skips offline
machines and reports them.

**M8. Pinned and VIP need events and an order.** NV-20..23 say what can be
pinned (sessions, channels, machines, PR reviews, board items) but not
whether pins are per user across devices, their order, or what happens when
a pinned thing is deleted. *API now:* MarkService on the host, an explicit
order, marks removed with the thing.

**M9. "Idle" is used for two things.** The status table uses Idle for "the
agent waits for input", and LD retention uses "sessions idle for N days" for
"no activity". *Spec should:* name the second "inactive".

**M10. The coordinator's "fetch a file" overlaps IN-17's copy-in rule.**
IN-17 has "Copy files into its clone or `shared/` from another session or
machine: Ask me". Fetching from `shared/`, the channel folder or a main
clone on its own machine into its own clone isn't named. *API now:*
`file_fetch` covers those sources and is allowed; anything from another
session or machine is `files_copy_in` and asks.

**M11. Several people or tools typing into one terminal.** AW-2 attach,
the app, `session_type` and queued comments can all write to one PTY at
once. *API now:* several writers are allowed, as in tmux; each sees how
many others are attached with input (`TerminalState.other_writers`), and
`humata attach --read-only` only watches. Queued typing is pasted whole at
the turn end, so it never interleaves with an agent's call. *Spec should:*
say whether a second writer is allowed or warned about.

**M12. Resume past retention.** TR-4 promises "exactly what it missed"
after a reconnect, but scrollback and event logs are trimmed. *API now:*
a terminal client behind the kept scrollback gets a fresh snapshot; an
event subscriber whose position is gone gets a `Resync` first and re-reads
current state. *Spec should:* say what "exactly what it missed" means once
the log has been trimmed.

**M13. `humata.inbox.ask` is used but not specified.** TP-* and IN-* have
agents ask the user questions, but no requirement names the tool, its
choices, or how a reply returns. *API now:* `inbox_ask` with up to 4
choices; the reply is the call's result.

**M14. IN-17 doesn't let an agent delete its own channel items.** "Edit or
move items it created" omits delete, while its own session board allows
delete. *API now:* delete is allowed for items it created (marked in
MCP.md), pending the spec.

**M15. "Its own clones" is unclear for forks.** IN-16 says it owns "its own
clones". A code-only fork has a new clone; a context-only fork reads its
parent's (IN-20); a session given its own copy (FC-*) has one per repo.
*API now:* own = clones under this session's folder; a context-only fork's
view of its parent is read-only.

**M16. Host variable values have no place to be entered.** SEC-1 says names
cross and values don't, and Hosts shows variables, but no screen sets a
value. *API now:* `SetHostVariable`, user-only and write-only, with the
value read from a TTY or stdin.

## Design choices the API made that the spec should confirm

- Host is the only writer of host state; a machine answers its own files,
  terminals and holders, and serves its journal while the host is away
  (GRPC.md §4).
- Sessions rebuild from the host's record, mirror and pointers (SL-4) as an
  Operation with steps and a "lost" list from the last pointers.
- Expand all and Collapse all (NV-18, NV-19) have no API.
- Pinned and VIP are user-only and have no MCP tool.
- Every write has an idempotency key; the host keeps keys for 30 days.

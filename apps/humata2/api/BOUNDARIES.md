# Humata 2: who may do what, and where it is checked

This file holds the access model behind [GRPC.md](GRPC.md),
[MCP.md](MCP.md) and [CLI.md](CLI.md): the principals and how each one is
proved, the capability tiers, how grants are made and removed, where each
check runs (with the host and with the host away), and why no agent can
widen its own access. [TRACE.md](TRACE.md) traces every requirement that
crosses a boundary to the API.

## 1. Threat model

- **What Humata protects against:** an agent that is wrong, confused, or
  steered by text it read (a README, an issue, a web page). It uses the
  tools it has, and it may try any tool with any arguments.
- **What Humata alone cannot protect against:** an agent process that runs
  as the user's own uid and sets out to get around Humata. Such a process
  can read and write the user's files, Humata's state on disk, `~/.ssh` and
  `~/.config/gh`, and can open network connections directly. The rules in
  Agent access are checked on every Humata call, but nothing stops a
  same-uid process from not using Humata. SEC-5 says so, and the Agent
  access tab and each uncontained session's Info panel say it in its words
  (`Session.contained`, `WhoAmIResponse.contained`). The design keeps the
  gap explicit and testable:
  - every rule is enforced at the daemon for every MCP and CLI call;
  - the one call path that could turn a same-uid process into the user
    (the CLI) needs presence (§2.3, SEC-9, SEC-10);
  - **Contain agents** (SEC-6, provisional D1) is off by default, set in
    Settings › Security (`SecuritySettings.contain_agents`) with a
    per-channel override (`ChannelSettings.contain_agents`), and applies
    when a session next launches or resumes. A contained agent starts
    inside an OS sandbox (`sandbox-exec` profile on macOS, Landlock plus a
    network namespace on Linux) that allows writes only inside its session
    folder and `shared/`, fails reads of other sessions' folders, Humata's
    own folders, `~/.ssh` and the gh config, and allows no network but its
    daemon's socket. Only for contained agents are "Never" and the network
    rule a boundary rather than a convention (HO-12).
- **Peers.** A remote machine is trusted for the sessions the host placed on
  it and nothing else. A machine compromised by root can lie about its own
  sessions, but it can't act for sessions on other machines, read the host's
  secrets, or answer requests it doesn't hold (§2.5).

## 2. Principals

Every connection is bound to one principal when it is accepted
(GRPC.md §3). No request carries a requester field; `WhoAmI` reports the
binding.

### 2.1 The app

The app connects to the host socket (0600 in a 0700 folder, SEC-4) and
proves a client key it keeps in a keychain item whose access list names only
the signed app. Only the app can show presence confirmations. The app has
every capability; user-only calls from the app need no extra proof, because
the person is using the app.

### 2.2 The CLI

| How `humata` was started | Principal |
| --- | --- |
| Inside a session's process tree, or with `HUMATA_SESSION_TOKEN` set | That session's agent. It can do exactly what the agent can through MCP |
| Elsewhere, no login | None. Every call is refused with `presence_required` except `WhoAmI`, which reports no actor, and `humata login` |
| In a terminal session bound by `humata login` | User, present, until the login ends (12 hours at most) |
| With `HUMATA_TOKEN` | Script: the token's capabilities on its channels |

Why a process with no credential gets nothing: the daemon decides "inside
a session's tree" from the caller's audit token (pid, then parents up to
launchd or init, comparing pid and start time with each holder). An agent
can double-fork so its child is reparented to launchd and has no holder
above it. Treating that process as the user would make every rule
advisory. So a process with no session token, script token, CLI login or
app key is refused everything except `WhoAmI` and logging in. There is no
unverified read tier.

Admin mode is for test daemons only. A daemon built with the `admin`
feature and started with `humatad2 serve --admin` takes a call with no
credential for the present user, so tests can drive it without logging in,
and says so in a warning when it starts. The feature is a compile-time
switch that release builds don't have, so a release daemon has neither the
flag nor the code.

A CLI login binds the terminal session that ran `humata login`: its session
id, the session leader's start time, and its controlling tty. A process the
agent starts can't join another terminal session (`setsid` makes a new one,
and a session can't be entered from outside), so it can't use the login.
What a login doesn't stop: an agent that the user runs by hand in that same
terminal, outside Humata, acts as the user. That is the user's own choice of
where to run it.

### 2.3 Presence

A user-only call from the CLI needs one of:

| Proof | Where | How it can't be faked |
| --- | --- | --- |
| `ConfirmInApp` | Host reachable (on the Mac or relayed from a machine) | The app shows the command, its machine and terminal; the call waits for a click. Agents have no app connection. Clicking in the app through accessibility scripting would need the user to have granted that to the agent's terminal; the dialog can require Touch ID (`LocalAuthentication`) to rule it out |
| CLI login | As above, set up once with `ConfirmInApp` or the passphrase | §2.2 |
| Approval passphrase | A machine while its host is away (AW-5) | Set in Settings › Security (`SetApprovalPassphrase`, user-only); stored on each machine as an Argon2id hash in `<root>/.humatad/`. Read from the TTY with echo off. Five wrong tries lock it for 15 minutes on that machine. A same-uid agent could read the hash and guess offline, so the setting asks for at least 5 random words |

A presence prompt is refused inside a session's terminal (it would be typed
by whatever drives that terminal), and the error says so
(`PresenceRequired.caller_in_session_terminal`).

### 2.4 Agents

- The holder starts `humatad mcp` with `HUMATA_SESSION_TOKEN`: 128 random
  bits, new on every launch and resume, kept by the daemon only as a hash.
- At connect the daemon checks the token and that the connecting process
  descends from that session's holder. A token presented from another
  session's tree, or from outside every tree, fails with
  `identity_mismatch` and is revoked: the session gets a new one at its next
  resume, and the user sees an Activity entry.
- On a remote machine the machine binds the agent and forwards its calls up
  as `ForwardedCall{AgentPrincipal}` (peer.proto). The host accepts an
  `AgentPrincipal` only for a session it placed on that machine; otherwise
  `identity_mismatch`.

### 2.5 Peers

- The host starts every peer link itself, over SSH or the provider's connect
  command (MP-7); no machine dials in, and no port is open (TR-5).
- `Hello` proves the machine holds the enrollment key written at install
  (HMAC over a host nonce), so a reinstalled or swapped machine is noticed.
  The SSH host key is pinned; a changed key stops the link with
  `host_key_changed` until the user checks and trusts it (user-only).
- PeerService is served only on that link, never on a socket, so nothing
  local to a machine can call it.
- What a peer may assert: pointers, mirror chunks, status and journal entries
  for sessions placed on it; agent calls for those sessions; local answers
  for requests it holds, each with the passphrase check it made. Anything
  else fails with `identity_mismatch` and is logged.

## 3. Capabilities and tiers

capabilities.proto has the closed list. Each MCP tool and CLI command checks
exactly one capability against one target. Tiers:

| Tier | Meaning | Capabilities |
| --- | --- | --- |
| No rule (owned) | IN-16's set only: its own session (title, status, repos, branches), reading and writing its own clones, its own views and its own requests, comments sent to it (D21), its own port forwards. Not a rule; no setting turns it off. A context-only fork reads its parent's clones here but never writes them (IN-20) | none: checked as "the target is the caller's own" |
| Default Allow | A rule, Allow for new channels (IN-17). The user may set Ask me or Never | OWN_BOARD_WRITE, CHANNEL_READ, CHANNEL_BOARD_ADD, CHANNEL_BOARD_EDIT_OWN, SHARED_WRITE, FILES_FETCH_LOCAL, VIEWS_PREPARE, REVIEW_DRAFT, INBOX_ASK |
| Ask me | A rule, Ask me for new channels; "Always let …" may add an IN-22 grant | CHANNEL_BOARD_EDIT_OTHERS, GLOBAL_READ, FILES_READ_OTHER_SESSION, FILES_READ_OTHER_MACHINE, FILES_COPY_IN, SESSION_NOTE, SESSION_TYPE, SESSION_START_SAME_MACHINE, SESSION_START_OTHER_MACHINE, PR_REVIEW_START, OWN_SESSION_MANAGE, OWN_SESSION_END, SESSION_MOVE, GIT_PUSH, MACHINE_PROVISION, TRASH_RESTORE, CHANNEL_CREATE |
| Ask me, no Allow | As Ask me, but the rule offers only Ask me and Never (D20); never low-risk, never approved on a machine while the Mac is away (HO-11) | NETWORK_REACH |
| Never by default | A rule, Never for new channels: denied at once with the rule named (IN-18) until the user changes it | SESSION_STOP_OTHER, CHANNEL_CHANGE, MACHINE_DESTROY |
| User-only | No rule, no grant, no MCP tool, no Inbox request (SEC-7, IN-23); from the CLI, presence | INBOX_ANSWER, ACCESS_CHANGE, SETTINGS_CHANGE, MARKS_CHANGE, PR_SUBMIT, USER_REVIEW_STATE, TRASH_EMPTY, HOST_SECRETS, CHANNEL_DELETE, MACHINE_SETTINGS, TERMINAL_INPUT |
| Peer | Only on the peer link | PEER_REPORT, HOST_CONTROL |

`GLOBAL_BOARD_WRITE` and `OWN_COMMENTS_READ` are deprecated (D21): the
Global board is part of the GLOBAL_READ row and comments to its own session
are a no-rule read. A rule stored under either is read as its new row.

Hard limits that no rule or grant lifts:

- A context-only fork never writes its parent's clones (IN-20, SEC-3):
  `parent_read_only`.
- Main clones are read-only to everyone but Humata's own fetch (FC-*):
  `main_clone_read_only`.
- Every path resolves inside a Humata root after following symlinks, and
  never into `.humata/` or `.humatad/`: `outside_humata_folders`.
- A PR review session pushes only when the user set PR-8; GIT_PUSH grants
  don't apply to it (IN-24).
- The user's own **Push and open PR(s)** (MV-12, VW-13) needs no rule:
  GIT_PUSH is checked only for an agent's `session_push`.
- No call takes the caller's identity as an argument (SEC-15). An identity
  used from outside its session's process tree is refused with
  `identity_mismatch`, replaced at the next resume and recorded in Activity
  (`IDENTITY_MISUSED`).

## 4. Asking, grants and their lifetime

- A call whose rule is Ask me becomes an Inbox request with the preview
  (IN-9, IN-10) and the one "Always let …" target IN-22 fixes for that kind
  of action. The agent doesn't choose the offered target; `inbox_request`
  (IN-26) names one, and the daemon narrows it to the same standard target:

  | Kind of action | Capabilities | `GrantTarget` |
  | --- | --- | --- |
  | Read or copy another session's files | FILES_READ_OTHER_SESSION, FILES_COPY_IN | `path` {session, folder asked for} |
  | Read or copy files on another machine | FILES_READ_OTHER_MACHINE, FILES_COPY_IN | `path` {machine, folder asked for} |
  | Note, question or typing into another session | SESSION_NOTE, SESSION_TYPE | `session` |
  | Start or fork a session | SESSION_START_SAME_MACHINE, SESSION_START_OTHER_MACHINE | `agent_on_machine` |
  | Edit, move or delete others' items | CHANNEL_BOARD_EDIT_OTHERS, GLOBAL_READ (boards) | `board` |
  | Network connection | NETWORK_REACH | `network` {machine, host, port} |
  | Push or pull request | GIT_PUSH | `repo_branch` |
  | Machines | MACHINE_PROVISION, MACHINE_DESTROY | `machine_action` |

  Other Ask me capabilities (OWN_SESSION_MANAGE, OWN_SESSION_END,
  SESSION_MOVE, PR_REVIEW_START, TRASH_RESTORE, CHANNEL_CREATE, GLOBAL_READ
  reads) offer no "Always let …": each approval is one call.
- Grants are made only by `AnswerRequest` with `AlwaysLet`, from the app or
  a present user, and its target must equal the card's offer
  (`invalid_field` otherwise). No other call writes a grant.
- A grant belongs to one session. Forks and rebuilt sessions (SL-4) start
  with none; a moved session keeps its own, since it is the same session.
- A grant is suspended while its rule is Never, ends when its session is
  archived or deleted, and is removed by `RevokeGrant` (IN-19).
- Rule changes apply from the next call (IN-21). A request already pending
  stays pending, and when it is answered the answer is checked against the
  current rule: if the rule became Never, approving fails with `denied` and
  says why.
- While I'm away (AW-4): Wait, Skip, or Allow low-risk. Low-risk kinds can
  only be chosen among capabilities with `may_be_low_risk`, which excludes
  anything that crosses machines, the network rule, typing, and every
  Never-by-default capability (AW-11).
- An "Always let …" given on a machine while the Mac is away applies there
  at once and becomes the same grant on the Mac at replay (AW-12).

## 5. User-only operations

These are the person's alone. They have no MCP tool (MCP.md §7) and no
capability an agent or script token can hold. From the CLI they need
presence. The gRPC handlers take a `PresentUser` value that only the
authentication module can construct (from the app's connection, a CLI login
or a checked proof), so a handler for one of these can't be reached with an
agent principal by mistake.

SEC-7's list, each with its capability:

- Answer Inbox requests, one or all (IN-12..14, IN-6): INBOX_ANSWER.
- Change rules, While I'm away, default access, grants, the approval
  passphrase, CLI logins and script tokens (IN-23, SEC-13, SEC-14):
  ACCESS_CHANGE.
- Settings, agents and their launch commands (ST-*), Contain agents
  (SEC-6), providers (MP-23), channel settings that aren't CHANNEL_CHANGE:
  SETTINGS_CHANGE.
- Pin, unpin, VIP, the pinned order (NV-20..25); view state (AR-5):
  MARKS_CHANGE.
- Submit a PR review; "Let the agent push" (PR-8, PR-12): PR_SUBMIT.
- The user's review state: Mark reviewed, view offers (VW-*), the user's
  comments and sending them (SM-*): USER_REVIEW_STATE.
- Empty trash; arbitrary transfers in the file manager (FM-*): TRASH_EMPTY.
- Host variable values, set and remove (HO-13), SSH host key trust, provider
  login, making a login unattended (SEC-1, MP-*): HOST_SECRETS.
- Delete a channel (CH-25): CHANNEL_DELETE.
- Change a machine's root or VM tier (FC-10, PER-8): MACHINE_SETTINGS.
- A machine shell (HO-4), attaching with input (AW-2), opening a shell tab,
  Send now on a session's queue (SE-23): TERMINAL_INPUT.
- Reading the audit log and the raw event stream.

## 6. No tool lets an agent widen its own grant

What an agent may do is a function of four stores: channel rules, grants,
the While I'm away rule, and the identity bindings (session tokens, CLI
logins, script tokens, the passphrase). Call these the policy stores. Claim:
no sequence of agent calls changes a policy store in a way that lets any
agent do more.

1. **Every write to a policy store is user-only.** SetChannelRule,
   SetAwayRule, SetDefaultRules, RevokeGrant, SetApprovalPassphrase,
   CliAuthService and AnswerRequest (the only grant writer) all take a
   `PresentUser` (§5). An agent principal can't produce one: its connection
   is bound to its session (§2.4), and the CLI inside its tree is the same
   agent (§2.2).
2. **No user-only capability has an MCP tool,** so there is no tool to call.
   A name called anyway returns `user_only` without reaching a handler.
3. **No allowed tool's effects include a policy store.** Each operation
   declares its effect set (the stores it writes). The test in §6.1 checks
   every agent-holdable operation's set against the policy stores.
4. **Indirect paths, each closed:**
   - *Approve its own request:* answering is user-only; a process the agent
     starts outside its tree has no credential, so the daemon refuses it
     everything but `WhoAmI` and logging in.
   - *Ask for a wider grant:* the card offers only the daemon's standard
     scope for that capability (§4); the agent can't name another session,
     "all sessions" or the channel.
   - *Edit the config Humata reads:* `file_write` can't reach `.humata/`,
     `.humatad/`, Humata's own folders or any path outside a root (§3 hard
     limits). Settings and launch commands are user-only.
   - *Act through another agent:* `session_note` and `session_type` reach
     another session whose own grants are the channel's; that agent can't
     answer requests either. Text typed into a shell session's tab runs
     `humata` as that shell session's agent, not as the user.
   - *Start or fork a helper:* new sessions and forks start with no grants,
     the channel's rules, and an agent from Settings, never a command line
     the caller writes.
   - *Rewrite history offline:* a machine's policy copy has no API to
     change it. A same-uid process could edit the file on disk (§1), so
     every journaled write is checked again on the host against the current
     rules at replay. Writes that fail are kept as `journal_entry_rejected`
     and shown; the snapshot copy never outranks the host.
   - *Forge a peer:* PeerService isn't on any socket, and a peer can assert
     only its own sessions (§2.5).
   - *Phish the user:* a view, note or question can carry any text, but the
     app renders agent text as text, never as controls; the only controls
     on an Inbox card are the app's own, and every card names the requester
     and its machine.

### 6.1 Tests that keep this true

Behaviour tests, not wording tests:

- For every RPC, call it with an agent principal on a channel whose rules
  are all Allow and a generated valid request; then read every policy store.
  Nothing changed. Run it with rules all Never as well.
- For every user-only RPC, call it as an agent, with no credential and as a
  script: each fails with `user_only` or `presence_required`, and the
  stores are unchanged.
- `tools/list` for an agent contains no tool whose operation is user-only;
  calling each user-only name fails with `user_only`.
- An agent's request for access, approved with "Always let", creates exactly
  the card's grant, for that session only; a fork of that session has none.
- A process double-forked from a session's agent connects with no
  credential: every call but `WhoAmI` returns `presence_required`.
- A journaled write accepted under a policy copy that has since changed to
  Never is rejected at replay, and the item is unchanged on the host.

## 7. Where checks run

| The call | Checked by | Against |
| --- | --- | --- |
| On the caller's own machine (its own files, its holder, local fork or start, another session here) | The machine, host reachable or not | The host's latest PolicySnapshot (GRPC.md §9.2). With the host reachable, an Ask goes to the host's Inbox; with it away, the away rule applies |
| Anything that crosses machines, touches GitHub or writes host state | The host (AR-4, TP-8) | The live rules and grants |
| Journaled writes, at replay | The host | The live rules; base revisions (SL-6) |
| User-only calls | Whichever daemon holds the state; presence checked there | §2.3 |

## 8. Secrets

- Host variable values (SEC-1): written by `SetHostVariable` (user-only,
  write-only), read by nobody through the API. `ListHostVariables` returns
  names. The value goes to the machine once and is stored there 0600; it is
  set in the agent's environment at launch, which is the point, so an agent
  can read its own variables.
- The GitHub token (SEC-2) stays on the Mac. Machines without `gh` use
  bundles and pushes through the Mac (`CreateBundle`, `CreatePushPack`). No
  RPC returns it, and peer messages never carry it.
- Provider TOML may name a secret only by reference (a keychain item or an
  environment variable on the Mac); `SetProvidersConfig` refuses a literal
  secret with `invalid_field`.
- Session tokens, script tokens and the passphrase are stored as hashes.
  Errors and logs never include them.

## 9. Coverage and trace

[TRACE.md](TRACE.md) lists every requirement that crosses a boundary with
its RPCs, MCP tools and CLI commands, the capability matrix per actor and
tier, and the gaps still open. Every RPC answers some requirement there;
two kinds of API answer the spec's needs without one requirement naming
them:

- `CliAuthService` (`humata login`, script tokens) is how SEC-10, SEC-11
  and SEC-14 are carried out.
- Session-board items are the agent's todos (OWN_BOARD_WRITE); the spec
  has no separate todo list.

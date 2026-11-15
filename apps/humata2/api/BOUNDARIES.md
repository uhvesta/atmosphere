# Humata 2: who may do what, and where it is checked

This file holds the access model behind [GRPC.md](GRPC.md),
[MCP.md](MCP.md) and [CLI.md](CLI.md): the principals and how each one is
proved, the capability tiers, how grants are made and removed, where each
check runs (with the host and with the host away), and why no agent can
widen its own access. §9 traces the spec to the API.

## 1. Threat model

- **What Humata protects against:** an agent that is wrong, confused, or
  steered by text it read (a README, an issue, a web page). It uses the
  tools it has, and it may try any tool with any arguments.
- **What Humata alone cannot protect against:** an agent process that runs
  as the user's own uid and sets out to get around Humata. Such a process
  can read and write the user's files, Humata's state on disk, `~/.ssh` and
  `~/.config/gh`, and can open network connections directly. The rules in
  Agent access are checked on every Humata call, but nothing stops a
  same-uid process from not using Humata. REVIEW.md C1 says what the spec
  must decide; this design makes the gap explicit and testable instead of
  hiding it:
  - every rule is enforced at the daemon for every MCP and CLI call;
  - the one call path that could turn a same-uid process into the user
    (the CLI) needs presence (§2.3);
  - an optional **Contain agents** setting (not in the spec; proposed in
    REVIEW.md C1) would start each agent inside an OS sandbox
    (`sandbox-exec` profile on macOS, Landlock plus a network namespace on
    Linux). The sandbox allows writes only inside the session folder and
    `shared/`, denies reads of Humata's own folders, `~/.ssh` and the gh
    config, and denies network except Humata's bridge socket. Only then do
    "Never" and the network rule become a boundary instead of a convention.
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
| Elsewhere, no login | User, unverified: reads of what this daemon stores. Every write, every relayed read from another machine and every user-only call needs presence (§2.3) |
| In a terminal session bound by `humata login` | User, present, until the login ends (12 hours at most) |
| With `HUMATA_TOKEN` | Script: the token's capabilities on its channels |

Why the unverified tier: the daemon decides "inside a session's tree" from
the caller's audit token (pid, then parents up to launchd or init, comparing
pid and start time with each holder). An agent can double-fork so its child
is reparented to launchd and has no holder above it. Treating that process
as the user would make every rule advisory. So a process outside every tree
is only trusted to read what it could already read from disk (§1).

A CLI login binds the terminal session that ran `humata login`: its session
id, the session leader's start time, and its controlling tty. A process the
agent starts can't join another terminal session (`setsid` makes a new one,
and a session can't be entered from outside), so it can't use the login.
What a login doesn't stop: an agent that the user runs by hand in that same
terminal, outside Humata, acts as the user. That is the user's own choice of
where to run it.

### 2.3 Presence

A user-only call from the CLI, and any write from the unverified tier, needs
one of:

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
| Owned | Always allowed on the caller's own session, its clones, its session board and items it created (IN-16). Not a rule; no setting turns it off | own session reads, own clone read and write, own session board, own views, own requests |
| Default | A rule, Allow for new channels (IN-17 and the narrow set). The user may set Ask me or Never | CHANNEL_READ, OWN_BOARD_WRITE, CHANNEL_BOARD_ADD, CHANNEL_BOARD_EDIT_OWN, SHARED_WRITE, VIEWS_PREPARE, REVIEW_DRAFT, OWN_COMMENTS_READ, INBOX_ASK |
| Needs approval | A rule, Ask me for new channels; "Always let …" may add a scoped grant | GLOBAL_READ, CHANNEL_BOARD_EDIT_OTHERS, GLOBAL_BOARD_WRITE, FILES_READ_OTHER_SESSION, FILES_READ_OTHER_MACHINE, FILES_COPY_IN, SESSION_NOTE, SESSION_TYPE, SESSION_START_SAME_MACHINE, SESSION_START_OTHER_MACHINE, NETWORK_REACH, OWN_SESSION_MANAGE, OWN_SESSION_END, SESSION_MOVE, GIT_PUSH, PR_REVIEW_START, MACHINE_PROVISION, TRASH_RESTORE, CHANNEL_CREATE |
| Needs approval, Never by default | As above, but denied at once until the user changes the rule | SESSION_STOP_OTHER, CHANNEL_CHANGE, MACHINE_DESTROY |
| User-only | No rule, no grant, no MCP tool | INBOX_ANSWER, ACCESS_CHANGE, SETTINGS_CHANGE, MARKS_CHANGE, PR_SUBMIT, USER_REVIEW_STATE, TRASH_EMPTY, HOST_SECRETS, and the interactive and machine-level calls in §5 |
| Peer | Only on the peer link | PEER_REPORT, HOST_CONTROL |

Hard limits that no rule or grant lifts:

- A context-only fork never writes its parent's clones (IN-20, SEC-3):
  `parent_read_only`.
- Main clones are read-only to everyone but Humata's own fetch (FC-*):
  `main_clone_read_only`.
- Every path resolves inside a Humata root after following symlinks, and
  never into `.humata/` or `.humatad/`: `outside_humata_folders`.
- A PR review session pushes only when the user set PR-8; GIT_PUSH grants
  don't apply to it.

## 4. Asking, grants and their lifetime

- A call whose rule is Ask me becomes an Inbox request with the preview
  (IN-9, IN-10) and the one "Always let …" scope the daemon computes for
  that capability: this session plus the narrowest target the call names
  (the session read from, the machine, the board). The agent doesn't choose
  the offered scope; `inbox_request` names a target, and the daemon narrows
  it to the same standard scope.
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
  Never-by-default capability.

## 5. User-only operations

These are the person's alone. They have no MCP tool (MCP.md §7) and no
capability an agent or script token can hold. From the CLI they need
presence. The gRPC handlers take a `PresentUser` value that only the
authentication module can construct (from the app's connection, a CLI login
or a checked proof), so a handler for one of these can't be reached with an
agent principal by mistake.

- Answer Inbox requests, one or all (IN-12..14, IN-6).
- Change rules, While I'm away, default access, grants, the approval
  passphrase, CLI logins and script tokens.
- Settings, agents and their launch commands (ST-*), channel settings that
  aren't CHANNEL_CHANGE.
- Pin, unpin, VIP (NV-20..23); view state (SE-13, NV-16, NV-19).
- Submit a PR review; "Let the agent push" (PR-8, PR-12).
- The user's review state: Mark reviewed, view offers (VW-*), the user's
  comments and sending them (SM-*).
- Empty trash; arbitrary transfers in the file manager (FM-*).
- Host variable values, SSH host key trust, provider TOML, provider login,
  making a login unattended (SEC-1, MP-*).
- A machine shell (HO-4), changing a machine's root or VM tier, interactive
  terminal attach (AW-2), and opening a shell tab in a session.
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
     starts outside its tree is the unverified tier, which can't answer
     without presence.
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
- For every user-only RPC, call it as an agent, as an unverified CLI user and
  as a script: each fails with `user_only` or `presence_required`, and the
  stores are unchanged.
- `tools/list` for an agent contains no tool whose operation is user-only;
  calling each user-only name fails with `user_only`.
- An agent's request for access, approved with "Always let", creates exactly
  the card's grant, for that session only; a fork of that session has none.
- A process double-forked from a session's agent connects as the
  unverified tier: its reads succeed, its writes return `presence_required`.
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

| Spec | gRPC (proto) | MCP | CLI |
| --- | --- | --- | --- |
| §3 CH channels and machines | ChannelService, ActivityService (channels) | `topology_get`, `channels_list`, `channel_create`, channel changes (Never) | `channels`, `topology`, `activity` |
| §4 FC folders and clones | SessionService.GiveOwnCopy, FileService, PeerService.PrepareClone | `file_*`, `files_*` | `files`, `session own-copy` |
| §5 SE sessions and terminals | SessionService, TerminalService | `session_*`, `whoami` | `session`, `attach`, `ls` |
| §6 FK forks | SessionService.PreviewFork, ForkSession | `session_fork` | `session fork` |
| §7 MV moving and archiving | MoveSession, Plan/ArchiveSession, RestoreSession | `session_move`, `session_archive` | `session move`, `archive`, `restore` |
| §8 LD lifecycle and disk | Plan/DeleteSession, TrashService, MachineService.GetStorage, ClearBuildCache, TrimLogs | `session_delete`, `trash_restore` | `session delete`, `trash`, `host storage` |
| §9 RV review | DiffService, CommentService (review) | `comments_list`, own diff via `file_read` | `diff`, `comment` |
| §10 PR reviews | PullRequestReviewService | `pr_review_start`, `review_draft*`, `review_summary_draft` | `review` |
| §11 SM spec mode and comments | SetSpecMode, CommentService | `session_spec_mode`, `comments_list` | `session spec`, `comment` |
| §12 FM files and transfers | FileService, TransferService | `file_fetch`, `files_copy_in` | `files`, `cp`, `mv`, `transfers` |
| §13 BD boards | BoardService | `board_*` | `board` |
| §14 IN Inbox and access | InboxService, AccessService, AuditService | `inbox_*`, `access_explain` | `inbox`, `access`, `audit` |
| §15 VW views | ViewService | `view_*`, `views_list` | `views`, `view` |
| §16 TP topology | DaemonService.WhoAmI, SessionService.SendNote/AskSession/AnswerQuestion | MCP.md §6 | `topology`, `session note/ask` |
| §17 HO, SK hosts and skills | MachineService, SkillsService, TerminalService.OpenMachineShell | `machine_*`, `skills_sync` | `host`, `skills sync`, `shell` |
| §18 MP providers | ProviderService | `machine_create`, `machine_start/stop` | `provider` |
| §19 SL, PER state and resume | PeerService (pointers, mirror, journal), EventService, SessionService.RebuildSession | `journal_status`, envelope `outcome` | `ops`, `events` |
| §20 AW away | InboxService on machines, PresenceProof passphrase | MCP.md §4, §5 | `inbox`, `login`, `attach` |
| §21 TR transport | GRPC.md §2, §6 | stdio | sockets |
| §22 NV-18, NV-19 Expand/Collapse all | none: UI only | none | none |
| §22 NV-20..23 pinned and VIP | MarkService (settings) | none (user-only) | `pin`, `vip`, `pinned` |
| §24 ST settings | SettingsService, AgentService, AccessService defaults | none (user-only) | `settings`, `agents` |
| §25 SEC | §1, §2, §8 here | | |

API that no requirement asks for (kept, flagged for the spec owner):

- `PullRequestDraft` in views: the coordinator's "prepare a PR"; the spec
  has review and board views only (VW-*).
- `PlanDeleteChannel` / `DeleteChannel`: the spec never deletes a channel
  (REVIEW.md H7).
- `SetHostVariable`: the spec lists variables (SEC-1) but has no place to
  enter a value.
- `RequestAccess` (`inbox_request`): asking ahead, so a long plan is
  approved once.
- `ViewStateService`, `ReorderPinned`: where the spec's UI state lives once
  the app holds none (AR-3, NV-21).
- `CliAuthService`: CLI login and script tokens (§2.2), needed because the
  spec doesn't say how the CLI proves a person.
- Session-board todos: the coordinator's "edit todos" maps to the session
  board (OWN_BOARD_WRITE); the spec has no separate todo list.

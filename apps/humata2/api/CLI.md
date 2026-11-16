# Humata 2: the `humata` command

`humata` is for people and their scripts. It is a gRPC client like the app
(AR-2): each command is one operation from [GRPC.md](GRPC.md) under the same
operation id that MCP ([MCP.md](MCP.md)), audit and history use, and it
passes the same capability check ([BOUNDARIES.md](BOUNDARIES.md)). It adds no
behaviour of its own except how it prints, asks and waits.

## 1. Where it connects

- On the Mac: the host socket.
- On any other machine: that machine's socket. The machine answers its own
  sessions' terminals and files itself and forwards everything else to the
  host. While the host is away it answers from its journal and its copy of
  the policy (GRPC.md §9). `humata status` says which of these applies.

## 2. Who it acts as

Each connection gets one principal, and no flag chooses it
(BOUNDARIES.md §2.2):

| Where `humata` runs | It acts as | It can |
| --- | --- | --- |
| Inside a session's terminal, or any process the agent started (the session's process tree, or `HUMATA_SESSION_TOKEN` set) | that session's agent (SEC-8), wherever it was typed | exactly what the agent can through MCP. User-only commands fail with `user_only`, and the error says to run them from a terminal outside Humata's sessions |
| Any other terminal, not logged in | no one (SEC-9) | only `humata whoami`, which says it has no credential, and `humata login`. Every other command fails with `presence_required`, and the error says to log in or use the app |
| A terminal session where `humata login` succeeded | the user, present (SEC-10) | everything, until the login ends (12 hours at most, or `humata logout`). Only that terminal session: a process that leaves it (`setsid`) is not covered (SEC-11) |
| With `HUMATA_TOKEN` set | a script (SEC-14) | only the token's capabilities on its channels, until it expires; never a user-only command |

A terminal with no credential gets nothing because "not inside a session's
tree" doesn't prove a person: an agent can start a process that its holder
no longer parents (a double fork leaves it with launchd or init as parent).
So a process with no session token, script token, CLI login or app key is
refused everything except `humata whoami` and `humata login`; there is no
unverified read tier. Scripts use a token (§5.4). Test daemons built with
the `admin` feature and started with `--admin` take such a process for the
user (BOUNDARIES.md §2.2); release builds can't.

## 3. Presence

A command that needs presence asks for it once and then runs:

| Where | Host reachable | Host away |
| --- | --- | --- |
| On the Mac | The app shows "`humata inbox approve` in Terminal (ttys004) wants to approve …" with Allow and Don't Allow. The command waits for the answer | n/a |
| On another machine | The same confirmation on the Mac | The approval passphrase (Settings › Security, SEC-13), typed at a prompt that reads the TTY directly with echo off. Five wrong tries lock it on that machine for 15 minutes (`passphrase_locked`) |

`humata login` asks once and covers the rest of that terminal session, which
is what to use over SSH (scenario 26.10). There is no "type yes to confirm":
an agent can drive a pseudo-terminal.

A presence prompt shown inside a session's terminal is refused before it
asks, because anything typed there may come from the agent.

## 4. Global flags

| Flag | Meaning |
| --- | --- |
| `--json` | One JSON object on stdout (§7); nothing else on stdout |
| `--channel <name\|id>`, `--machine <name\|id>` | Pick the context. Names are matched as in the app; an ambiguous name fails with `invalid_field`, listing the matches |
| `--wait <seconds>` | For a call that needs an answer (as an agent, or a waiting operation): how long to wait. Default: until done in a TTY, 0 with `--json` (then it returns `pending` and the request id) |
| `--strict` | Treat `journaled` as failure (exit 20): for scripts that need the host to have applied it |
| `--key <idempotency key>` | Reuse a key to retry the same intent safely. Default: a new key per run |
| `--plan <token>` | Run a destructive command against a plan printed earlier by `--dry-run`, so a script confirms what it saw (GRPC.md §5.4) |
| `--dry-run` | Print the plan (what will be lost, what is kept) and its token; change nothing |
| `-y`, `--yes` | Skip the CLI's own "Continue?" after printing a plan. Never skips presence |

## 5. Commands

Each row: command, operation id, gRPC call, who may run it, behaviour with the
host away (as in MCP.md §5). "User" means the person (logged in or
confirming, §3) or a script token holding the capability named after
"agent:", which is also what an agent needs; "user-only" means agents and
scripts can never run it. "Anyone" means any caller with a credential; with
none, only `humata whoami` and `humata login` run.

### 5.1 Looking around

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata status` | `daemon.status` | GetServerInfo, WhoAmI | anyone | local |
| `humata whoami` | `daemon.whoami` | WhoAmI | anyone | local |
| `humata ls [--all] [--channel c]` | `session.list` | ListSessions | anyone; an agent sees its channel | stale on a machine; its own sessions live |
| `humata channels` | `channel.list` | ListChannels | user; agent: GLOBAL_READ | stale |
| `humata channel create <name> [--repo r]… [--machine m]…` | `channel.create` | CreateChannel | user; agent: CHANNEL_CREATE | host only |
| `humata channel rename <c> <name>`, `humata channel repo add\|rm <c> <repo> [--dry-run\|--plan t]`, `humata channel machine add\|rm <c> <m> [--dry-run\|--plan t]`, `humata channel grid <c>` | `channel.rename`, `channel.add_repo`, … | RenameChannel, AddRepo, PlanRemoveRepo/RemoveRepo, AddMachine, PlanRemoveMachine/RemoveMachine, GetGrid | user; agent: CHANNEL_CHANGE (Never by default); grid: CHANNEL_READ | host only |
| `humata channel settings <c> [get\|set <key> <value>]` | `channel_settings.*` | Get/UpdateChannelSettings | set: user-only | host only |
| `humata channel delete <c> [--dry-run\|--plan t] [--delete-work]` | `channel.delete` | PlanDeleteChannel, DeleteChannel | user-only with presence (CH-25). The plan lists every session, board item and pending request and each session's unpushed work; `--delete-work` is CH-24's "Delete that work" | host only |
| `humata topology` | `channel.topology` | GetChannel + ListSessions | anyone in the channel | stale |
| `humata activity [--channel c]` | `activity.list` | ListActivity | user; agent: CHANNEL_READ | stale |
| `humata events [--after seq] [--topic t]` | `events.subscribe` | EventService.Subscribe | user-only | host only |
| `humata audit [--channel c]` | `audit.list` | ListAudit | user-only | host only |
| `humata ops [id]`, `humata ops cancel id`, `humata ops retry id` | `operation.*` | OperationService | user; agent: its own | stale |

### 5.2 Sessions and terminals

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata attach <session> [--tab t]` | `terminal.attach` | TerminalService.Attach | user-only with presence (TERMINAL_INPUT, AW-2); several writers may attach (SE-24) | local for this machine's sessions (AW-2) |
| `humata attach --read-only <session> [--tab t]` | `terminal.watch` | TerminalService.Attach(read_only) | anyone for this machine's sessions; agent: its own, others FILES_READ_OTHER_SESSION (D21). Sends no input and is not counted as a writer (SE-25) | local |
| `humata session queue <session>` | `terminal.list_queued` | ListQueued | user; agent: its own | local |
| `humata session send-now <session> [<queued-id>…]` | `terminal.send_queued_now` | SendQueuedNow | user-only (TERMINAL_INPUT, SE-23) | local |
| `humata session start --machine m [--agent a] [--repo r]… [--title t] [--message m]` | `session.start` | StartSession | user; agent: SESSION_START_* | waits (other machine), or per away rule (this machine) |
| `humata session fork <session> --kind code-and-context\|code-only\|context-only --machine m` | `session.fork` | PreviewFork, ForkSession | as start | as start |
| `humata session rename <session> <title>` | `session.rename` | RenameSession | user; agent: OWN_SESSION_MANAGE (self) | host |
| `humata session stop\|resume <session>` | `session.stop`, `session.resume` | StopSession, ResumeSession | user; agent: OWN_SESSION_END (self), SESSION_STOP_OTHER | local for this machine's holders, then journaled |
| `humata session archive\|delete <session> [--dry-run\|--plan t]` | `session.archive`, `session.delete` | PlanArchive/ArchiveSession, PlanDelete/DeleteSession | user; agent: OWN_SESSION_END (self), SESSION_STOP_OTHER (others, D21) | host |
| `humata session restore\|rebuild <session> [--machine m]` | `session.restore`, `session.rebuild` | RestoreSession, RebuildSession | user; agent: SESSION_START_* (D21) | host |
| `humata session move <session> --to m` | `session.move` | MoveSession | user; agent: SESSION_MOVE | host |
| `humata session repo add\|drop <session> <repo>` | `session.add_repo`, `session.drop_repo` | AddRepoToSession, PlanDropRepo/DropRepo | user; agent: OWN_SESSION_MANAGE | host |
| `humata session own-copy <session> <repo>` | `session.own_copy` | GiveOwnCopy | as repo | host |
| `humata session push <session> [--repo r]… [--pr] [--from-view v]` | `session.push` | PlanPush, PushAndOpenPullRequests | user (no rule, IN-24); agent: GIT_PUSH. `--from-view` opens a prepared PR view's fields (VW-13) | host (GitHub goes through the Mac) |
| `humata session note\|ask <session> <text>` | `session.note`, `session.ask` | SendNote, AskSession | user; agent: SESSION_NOTE | per away rule here; waits elsewhere |
| `humata session type <session> <text> [--at-turn-end]` | `session.type` | TerminalService.Type | user; agent: SESSION_TYPE. With the output fallback or in a shell, `--at-turn-end` queues it as "waiting: turn end not certain" (SE-23) | as note |
| `humata shell <machine>` | `machine.shell` | OpenMachineShell | user-only (TERMINAL_INPUT) | local on that machine |
| `humata conversation <session>` | `session.conversation` | ReadConversation | user; agent: FILES_READ_OTHER_SESSION for others | stale |

### 5.3 Boards, views, comments, reviews

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata board [--global\|--channel c\|--session s] [--column c]` | `board.items_list` | ListItems | user; agent per MCP.md | stale |
| `humata board add <title> [--column c] [--assign s]` | `board.item_create` | CreateItem | user; agent per MCP.md | journal |
| `humata board edit <item> [--title] [--description] [--assign]` | `board.item_update` | UpdateItem | as add | journal |
| `humata board move <item> <column> [--after item]` | `board.item_move` | MoveItem | as add | journal |
| `humata board rm <item>` | `board.item_delete` | DeleteItem | as add | journal |
| `humata board history <item>` | `board.item_history` | GetItemHistory | as list | stale |
| `humata views [--session s]`, `humata view open\|later\|not-now\|revert\|keep <view>` | `view.list`, `view.set_state` | ListViews, SetViewState | list: anyone; state: user-only | stale / host |
| `humata diff <session> [--repo r] [--against base\|last-turn\|turn n]` | `diff.get` | DiffService | user; agent: own session, others need FILES_READ_* | local for this machine |
| `humata diff mark-reviewed <session> <path>…` | `diff.mark_reviewed` | MarkReviewed | user-only | host |
| `humata comment add <session> <repo>:<path>:<a>-<b> <text>`, `send`, `ls`, `rm` | `comment.*` | CommentService | user-only for writes | journal |
| `humata review start <pr>… --machine m` | `review.start` | StartReview | user; agent: PR_REVIEW_START | host |
| `humata review drafts <session>`, `humata review submit <session> --as comment\|approve\|request-changes [--dry-run\|--plan t]` | `review.list_drafts`, `review.submit` | ListComments, PlanSubmit, SubmitReview | submit: user-only | host |
| `humata review allow-push <session> on\|off` | `review.set_push_allowed` | SetPushAllowed | user-only | host |

### 5.4 The Inbox and access

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata inbox [--all] [--channel c\|--session s [--with-forks]]` | `inbox.list` | ListRequests | user; agent: its own requests only | the machine's journal: requests it holds (AW-3) |
| `humata inbox show <request>` | `inbox.get` | GetRequest | as list | as list |
| `humata inbox approve\|deny <request> [--note text] [--always]` | `inbox.answer` | AnswerRequest | user-only | answered on the machine, passphrase (AW-5) |
| `humata inbox reply <request> <text>` | `inbox.answer` | AnswerRequest (Reply) | user-only | as approve |
| `humata inbox approve-all\|deny-all [--channel c]` | `inbox.answer_all` | AnswerAll with the ids just printed | user-only | as approve |
| `humata access [--channel c]` | `access.get_rules` | GetChannelRules | anyone (agents see their channel) | stale |
| `humata access set <capability> allow\|ask\|never [--channel c]` | `access.set_rule` | SetChannelRule | user-only | host only |
| `humata access away wait\|skip\|low-risk [--kinds k…]` | `access.set_away_rule` | SetAwayRule | user-only | host only |
| `humata access grants`, `humata access revoke <grant>` | `access.grants`, `access.revoke_grant` | ListGrants, RevokeGrant | revoke: user-only | host only |
| `humata access why <session> <capability> [target]` | `access.explain` | Explain | user; agent: itself | local (policy copy) |
| `humata access passphrase [--generate]` | `access.set_passphrase` | SetApprovalPassphrase | user-only. At least 5 words; `--generate` picks 5 random words and prints them once (SEC-13) | host only |
| `humata access ask-ahead <capability> <target> --reason r` | `inbox.request` | RequestAccess | agent: INBOX_ASK (IN-26); the user has no need of it | as `inbox_ask` |
| `humata login [--hours n]`, `humata logout`, `humata logins` | `cli.login`, … | CliAuthService | the person; at most 12 hours, this terminal session only (SEC-10, SEC-11) | passphrase |
| `humata token create\|ls\|revoke` | `cli.token_*` | CreateScriptToken, … | user-only; a token lists capabilities, channels and expiry and never holds a user-only one (SEC-14) | host only |

`approve-all` sends only the ids it printed, so a request that arrives while
the person reads is left pending (IN-6).

### 5.5 Files, transfers, trash

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata files ls\|cat\|find <location>` | `file.list`, `file.read`, `file.find` | FileService | user; agent per MCP.md | local for this machine |
| `humata files put <local-path> <location>`, `mkdir`, `mv` | `file.write`, `file.folder_create`, `file.rename` | FileService | user; agent: own clone and `shared/` | local |
| `humata files rm <location>…` | `file.trash` | MoveToTrash | user; agent: own clone | local |
| `humata cp\|mv <location>… <location> [--on-existing fail\|replace\|keep-both]` | `transfer.start` | StartTransfer | user; agent: `files_copy_in` and `file_fetch` only | waits when it needs another machine |
| `humata transfers`, `humata transfer pause\|resume\|cancel <id>` | `transfer.*` | TransferService | user; agent: its own | host |
| `humata trash [--machine m]`, `humata trash restore <item>` | `trash.list`, `trash.restore` | TrashService | user; agent: TRASH_RESTORE | host |
| `humata trash empty [--machine m] [--dry-run\|--plan t]` | `trash.empty` | PlanEmptyTrash, EmptyTrash | user-only | host only |

A location is `<machine>:<session>/<repo>/<path>`, `<machine>:shared/<path>`,
`<machine>:channel/<path>` or `<machine>:main/<repo>/<path>` (read-only).
Inside a session, a bare relative path is the agent's own clone.

### 5.6 Machines, providers, skills, settings

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata host ls`, `humata host show <m>` | `machine.list`, `machine.get` | ListMachines, GetMachine | anyone (agents: their channel's) | stale |
| `humata host add <ssh-host> [--name n]` | `machine.add_ssh` | AddSshMachine | user; agent: MACHINE_PROVISION | host only |
| `humata host reconnect <m>` | `machine.reconnect` | ReconnectMachine | as add | host only |
| `humata host remove\|forget <m> [--dry-run\|--plan t]` | `machine.remove`, `machine.forget` | PlanRemove/RemoveMachine, PlanForget/ForgetMachine | user; agent: MACHINE_DESTROY (Never by default) | host only |
| `humata host root <m> <path> [--move-channels] [--dry-run\|--plan t]` | `machine.change_root` | PlanChangeRoot, ChangeRoot | user-only (MACHINE_SETTINGS). The plan lists the channels under the old root (FC-10); without `--move-channels` the new root applies to new channels only | host only |
| `humata host vm-tier <m> on\|off` | `machine.set_vm_tier` | SetVmTier | user-only (MACHINE_SETTINGS) | host only |
| `humata host storage <m>`, `clear-build-cache`, `trim-logs` | `machine.storage`, … | GetStorage, ClearBuildCache, TrimLogs | storage: anyone; actions: user | host only |
| `humata host vars <m>`, `humata host var set <m> <NAME>`, `humata host var rm <m> <NAME>` | `host.list_variables`, `host.set_variable`, `host.remove_variable` | ListHostVariables, SetHostVariable, RemoveHostVariable | set, rm: user-only (HO-13); the value is read from the TTY with echo off or from stdin, never from an argument. Running agents keep the old value | host only |
| `humata host trust-key <m> --fingerprint <sha256>` | `host.trust_new_key` | TrustNewHostKey | user-only; the fingerprint must be typed, not pasted from the error | host only |
| `humata provider ls`, `humata provider config edit` | `providers.*` | ListProviders, Get/SetProvidersConfig | edit: user-only | host only |
| `humata provider import <file> [--dry-run\|--plan t]` | `provider.import` | PreviewProviderImport, ImportProvider | user-only (MP-23); prints the file's full text and every command it will run, then asks for presence | host only |
| `humata provider create <provider> <name> [--param k=v]…`, `start\|stop\|status <m>` | `provider.*` | CreateMachine, RunProviderAction | user; agent: MACHINE_PROVISION | host only |
| `humata provider delete <m> [--dry-run\|--plan t]` | `provider.delete` | RunProviderAction(DELETE), DeleteWhatWasMade | user; agent: MACHINE_DESTROY | host only |
| `humata provider login <provider>` | `provider.login` | InteractiveLogin | user-only (interactive) | host only |
| `humata skills sync [machine…] [--dry-run]` | `skills.sync` | PreviewSkillsSync, SyncSkills | user; agent: MACHINE_PROVISION (SK-2). `--dry-run` works for every sync (SK-11); offline machines are named as skipped (SK-10) | host only |
| `humata bridge ls [--machine m\|--session s]`, `humata bridge close <forward>` | `bridge.list`, `bridge.close` | ListPortForwards, ClosePortForward | ls: anyone; close: user, agent its own (HO-11) | host only |
| `humata settings [get\|set <key> <value>]` | `settings.*` | Get/UpdateSettings | set: user-only | host only |
| `humata settings contain on\|off [--channel c \| --channel c --inherit]` | `settings.update`, `channel_settings.update` | UpdateSettings(security), UpdateChannelSettings(contain_agents) | user-only (SEC-6); applies when a session next launches or resumes | host only |
| `humata agents ls\|add\|edit` | `agents.*` | AgentService | add, edit: user-only | host only |
| `humata pin\|unpin\|vip\|unvip <thing>`, `humata pinned` | `marks.set`, `pinned.list` | MarkService | user-only (NV-20); a new pin goes at the end (NV-25) | host only |
| `humata pinned reorder <thing>…` | `pinned.reorder` | ReorderPinned | user-only (NV-25) | host only |

Expand all and Collapse all (NV-18, NV-19) are the app's own and have no
command.

## 6. Waiting

A command that needs an answer prints the request it made and waits:

```text
$ humata session ask "API rate limiter" "Which retry budget did you pick?"
Waiting for “API rate limiter” to answer (request rq_01J…). Ctrl-C stops waiting; the question stays.
```

Ctrl-C ends the wait but leaves the request; `humata inbox show <id>` shows
its answer later. With the host away a command that needs another machine
says so and waits for the Mac, or fails with `host_unreachable` under Skip.

## 7. Output and exit codes

Without `--json` the CLI prints `Error.message` and then `Error.fix` on
stderr, and when `retry.after` is set, "Try again after 14:32". With
`--json`, stdout is one object:

```json
{ "ok": true, "operation": "board.item_move", "outcome": "applied", "result": { } }
{ "ok": true, "operation": "board.item_move", "outcome": "journaled", "journal": { "machine": "ma_…", "seq": 41 }, "result": { "provisional_id": "~devbox-1.41" } }
{ "ok": false, "operation": "inbox.answer", "error": { "kind": "presence_required", "message": "…", "fix": "…", "details": { "accepted": ["confirm_in_app", "cli_login"] } } }
```

| Exit | When |
| --- | --- |
| 0 | Done: applied, or journaled (unless `--strict`) |
| 2 | Usage: unknown command, bad flag, ambiguous name (`invalid_field` from parsing) |
| 3 | `not_found` |
| 4 | `denied`, `not_in_channel`, `parent_read_only`, `main_clone_read_only`, `push_refused` |
| 5 | `user_only`, `presence_required`, `identity_mismatch` |
| 6 | `revision_conflict`, `plan_stale`, `already_answered`, `name_taken` |
| 7 | Other preconditions: `unpushed_work`, `session_busy`, `user_away`, `request_withdrawn`, `host_key_changed`, … |
| 8 | `machine_offline`, `host_unreachable`, `ssh_failed` |
| 9 | An external tool failed: `git_failed`, `github_cli`, `provider_step_failed`, `step_failed`, `review_partly_posted` |
| 10 | `disk_full`, `rate_limited`, `passphrase_locked` |
| 11 | `checksum_mismatch`, `offset_trimmed` |
| 12 | `pending`: still waiting for the user when `--wait` ran out |
| 20 | `journaled` with `--strict` |
| 70 | `internal` |
| 130 | Interrupted |

Scripts branch on the exit code or `error.kind`, never on the text.

## 8. Examples

Scenario 26.10, over SSH on devbox-1 with the Mac closed:

```text
$ humata ls
$ humata attach api-rate-limiter        # asks for the approval passphrase once if not logged in
$ humata login                          # or log in first; covers this SSH session
$ humata inbox
$ humata inbox approve rq_01J…          # answered here; the Mac shows it answered on reconnect (AW-6)
```

A script that moves board items, run from cron on the Mac:

```text
$ humata token create --name triage --capability channel_board_add --capability channel_board_edit_others --channel Payments --days 30
$ HUMATA_TOKEN=… humata board move PAY-7 done --json --strict
```

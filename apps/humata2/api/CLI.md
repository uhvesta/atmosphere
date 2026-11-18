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
| `humata channel settings <c> [get\|set <key> <value>]` | `channel_settings.*` | Get/UpdateChannelSettings | set: user-only; a channel's own default agent (cleared back to Settings) and retention are served; unset retention reads as the Settings defaults (D770) | host only |
| `humata channel delete <c> [--dry-run\|--plan t] [--delete-work]` | `channel.delete` | PlanDeleteChannel, DeleteChannel | user-only with presence (CH-25). The plan lists every session, board item and pending request and each session's unpushed work; `--delete-work` is CH-24's "Delete that work" | host only |
| `humata topology` | `channel.topology` | GetChannel + ListSessions | anyone in the channel | stale |
| `humata activity [--channel c]` | `activity.list` | ListActivity | user; agent: CHANNEL_READ | stale |
| `humata events [--after seq] [--topic t]` | `events.subscribe` | EventService.Subscribe | user-only | host only |
| `humata audit [--channel c]` | `audit.list` | ListAudit | user-only | host only |
| `humata ops [id]`, `humata ops cancel id`, `humata ops retry id` | `operation.*` | OperationService | user; agent: its own | stale |
| `humata doctor restarts [report] [--explain] [--allow id]… [--diagnosis file\|-] [--state dir]` | none for the reads (files on disk); `daemon.repair_restart` and `session.resume` for repairs | RepairRestart, ResumeSession | user; works with the daemon down under `--explain` | this machine only |

`humata doctor restarts` reads the newest restart report (or the named one) in the state folder's `restarts/`: `restart.json` and `operations.jsonl`. It changes nothing under `--explain`; otherwise it runs the safe repairs, and a lossy one only when `--allow <id>` names it. With `--json`, `result` holds `report`, `machine`, `brought_back`, `not_brought_back`, `operations`, `stuck`, `owed`, `repairs` and `applied`, and `diagnosis` (the path of the `diagnosis.md` it wrote). Each operation has `id`, `kind`, `step`, `state` and `detail`. The latest line of an id wins. Every kind is shown, including one this CLI doesn't know. `state` is `pending`, `finished`, `undone` or `stuck` as the daemon writes it, and any other state a newer daemon writes is shown as written. `stuck` lists the stuck ones, with `detail` as the fix; `owed` lists those with no end line (pending, or an unknown state). Exit codes: 0 done (a repair that failed is in `applied`, not the exit code); 2 (`invalid_field`) an `--allow` id the report doesn't offer, a `--diagnosis` that can't be read or isn't UTF-8, a file it can't read, or no state folder found; 3 (`not_found`) no report, or no report of that name; 70 (`internal`) a report file that isn't a restart report, or a `diagnosis.md` it can't write. Without `--json` the same result is printed as `key: value` lines under the same names, so the `owed` and `stuck` lists, and unknown kinds and states, appear with each entry's `kind`, `id`, `state` and `detail`. A line cut short by a power cut is skipped; a line with no id is shown as `line <n>`.

### 5.2 Sessions and terminals

`humata attach <session>` without `--tab` opens the session's agent tab (`TerminalService.ListTabs`, kind agent). When the session has no agent tab, it opens the only tab it has; with several and no agent tab it stops (exit 7, `error.kind` `no_agent_tab`) and lists their ids, so pick one with `--tab`. `humata session tabs <session>` prints those tabs.

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata attach <session> [--tab t]` | `terminal.attach` | TerminalService.Attach | user-only with presence (TERMINAL_INPUT, AW-2); several writers may attach (SE-24) | local for this machine's sessions (AW-2) |
| `humata attach --read-only <session> [--tab t]` | `terminal.watch` | TerminalService.Attach(read_only) | anyone for this machine's sessions; agent: its own, others FILES_READ_OTHER_SESSION (D21). Sends no input and is not counted as a writer (SE-25) | local |
| `humata session queue <session>` | `terminal.list_queued` | ListQueued | user; agent: its own | local |
| `humata session tabs <session>` | `terminal.list_tabs` | ListTabs | user; agent: its own | host (the session's machine answers) |
| `humata session send-now <session> [<queued-id>…]` | `terminal.send_queued_now` | SendQueuedNow | user-only (TERMINAL_INPUT, SE-23) | local |
| `humata session start [--machine m] [--agent a] [--repo r]… [--title t] [--message m]` | `session.start` | StartSession | user; agent: SESSION_START_* | waits (other machine), or per away rule (this machine). Without `--machine` it starts on this machine (D672 (Provisional)) |
| `humata session fork <session> --kind\|--copy code-and-context\|code-only\|context-only (short: code, context) [--agent a] [--before-turn n] [--title t] [--branch b] --machine m` (a fork stays in its parent's channel: `--channel` naming another is a usage error that points at `session continue`, CV-8) | `session.fork` | PreviewFork, ForkSession | as start | as start |
| `humata session continue <session> --channel c [--before-turn n]` (CV-8: a new session in another channel, set up as its New session, whose agent goes on from a copy of this conversation, whole or up to turn n; the session is untouched; `--channel` is required and is never the session's own channel or the Desk; the new session keeps this session's title, and `session rename` changes it) | `session.continue_in_channel` | ContinueInChannel | as start | as start |
| `humata session rename <session> <title>` | `session.rename` | RenameSession | user; agent: OWN_SESSION_MANAGE (self) | host |
| `humata session stop\|resume <session>` | `session.stop`, `session.resume` | StopSession, ResumeSession | user; agent: OWN_SESSION_END (self), SESSION_STOP_OTHER | local for this machine's holders, then journaled |
| `humata session stop\|resume --tab t [--machine m]` (D784: a shell tab, a session's, a machine's own or one no channel has, as `session running` lists it; stopped, it keeps its tab and saved output and resumes in the same folder. An agent's tab is refused: stop or resume its session) | `machine.stop_running_holder`, `machine.resume_running_holder` | StopRunningHolder, ResumeRunningHolder | user-only | refused while the machine is offline: reconnect it and run it again; nothing is queued |
| `humata session end <session> [--dry-run\|--plan t]` (D784 "End for good": Archive by the plan; it can't be resumed. `humata session restore` brings it back stopped) | `session.end` | PlanArchive/ArchiveSession | as `session archive` | as `session archive`; refused while the machine is offline |
| `humata session end --tab t [--machine m]` (D784: a shell tab ends and is gone; nothing is archived. An agent's tab is refused: use `session end <session>`) | `machine.end_running_holder` | EndRunningHolder | user-only | refused while the machine is offline: reconnect it and run it again; nothing is queued |
| `humata session running [--machine m]` (D784: every terminal of ours on the machine, this one by default: running sessions by title, then stopped sessions, then those no session names by start, with tab, agent, pid, folder and whether one is attached) | `machine.list_running_holders` | ListRunningHolders | user; agent: its own machine's | host (relayed to the machine named); refused while it is offline |
| `humata session archive\|delete <session> [--dry-run\|--plan t]` | `session.archive`, `session.delete` | PlanArchive/ArchiveSession, PlanDelete/DeleteSession | user; agent: OWN_SESSION_END (self), SESSION_STOP_OTHER (archive of others, D21), SESSION_DELETE_OTHER (delete of others) | host |
| `humata session restore\|rebuild <session> [--machine m]` | `session.restore`, `session.rebuild` | RestoreSession, RebuildSession | user; agent: SESSION_START_* (D21) | host |
| `humata session move <session> --to m` | `session.move` | MoveSession | user; agent: SESSION_MOVE | host |
| `humata session repo add\|drop <session> <repo>` | `session.add_repo`, `session.drop_repo` | AddRepoToSession, PlanDropRepo/DropRepo | user; agent: OWN_SESSION_MANAGE | host |
| `humata session own-copy <session>` (copies every repo of a context-only fork, FK-9) | `session.own_copy` | GiveOwnCopy | as repo | host |
| `humata session push <session> [--repo r]… [--pr] [--from-view v]` | `session.push` | PlanPush, PushAndOpenPullRequests | user (no rule, IN-24); agent: GIT_PUSH. `--from-view` opens a prepared PR view's fields (VW-13) | host (GitHub goes through the Mac) |
| `humata session note\|ask <session> <text>` | `session.note`, `session.ask` | SendNote, AskSession | user; agent: SESSION_NOTE. Typed once into the session's agent after its turn, as one line naming the sender, through the input gate (D774). A note still mid-turn after 45 s is refused (send it again); an ask waits for the answer (60 s interactive, else 30 min) and prints ANSWERED, DECLINED or EXPIRED. A session not running, or on another machine, gets nothing | the session's machine (this Mac only, for now) |
| `humata session answer <qu_id> [answer] [--decline]` | `session.answer` | AnswerQuestion | only the asked session's own agent; any other caller is refused. An answered, expired or abandoned question is not found | as note |
| `humata session type <session> <text> [--no-submit] [--at-turn-end] [--tab <id>]` | `session.type` | TerminalService.Type | user; agent: SESSION_TYPE. Types the text into the session's tab and presses Enter, so the agent or shell receives it. `--no-submit` types it and leaves it in the input. The reply says `typed` and `submitted`; text that is only typed has not been sent. `agent_busy` is true when the text was written mid-turn: the agent decides whether to queue it, and Humata can't tell (`queued` is only Humata's own queue, for `--at-turn-end`). With the output fallback or in a shell, `--at-turn-end` queues it as "waiting: turn end not certain" (SE-23) | as note |
| `humata turn working\|idle\|needs-input` | `session.report_turn` | ReportTurn | the session's own agent only: it is what its Claude Code hooks run (D169); user, script and peer callers are refused. Prints nothing and exits 0 whatever happens, so a hook can't fail or wait on it | the session's machine |
| `humata hook claude\|codex\|copilot permission` | `session.hook` | Hook (stream) | the session's own agent only: its blocking permission hook runs it with the agent's payload on stdin (D191). The daemon raises one Inbox card and holds; the command prints the answer in that agent's shape. It prints nothing, which is no decision so the agent's own prompt asks, when the daemon is down, refuses, ends the hold at 3590 s, or the payload isn't an ask or is over 1 MiB, or the session's agent has Settings › Agents › Ask in the Inbox off (D782). Always exits 0; gives up itself at 3595 s (D192). With `--dry-run`, which only a person types, it is refused as usage (exit 2) before reading stdin | the session's machine |
| `humata hook copilot question` | `session.hook` | Hook (stream) | the session's own agent only: Copilot's `preToolUse` hook for its `ask_user` form runs it (D761). The daemon raises one Inbox question with the whole question and every choice and holds; the session is waiting on you meanwhile. `humata inbox reply <id> <text>` prints a `deny` whose reason carries the answer, so the form never opens; Deny says the person declined. No answer prints nothing, and Copilot's form opens; so does an agent whose Ask in the Inbox is off (D782). Exits and limits as for `permission` | the session's machine |
| `humata shell <machine>` | `machine.shell` | OpenMachineShell | user-only (TERMINAL_INPUT) | local on that machine |
| `humata conversation <session>` | `session.conversation` | ReadConversation | user; agent: its own, CHANNEL_READ in its channel, GLOBAL_READ for others, like any read of a session | local for this machine's sessions |

### 5.3 Boards, views, comments, reviews

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata board [--global\|--channel c\|--session s] [--column c]` | `board.items_list` | ListItems | user; agent per MCP.md | stale |
| `humata board add <title> [--column c] [--assign s]… [--depends-on item]…` | `board.item_create` | CreateItem | user; agent per MCP.md | journal |
| `humata board edit <item> [--title] [--description] [--assign s]… [--unassign s]… [--depends-on item]… [--drop-depends-on item]… [--revision n]` | `board.item_update` | UpdateItem | as add | journal |
| `humata board move <item> <column> [--after item] [--revision n]` | `board.item_move` | MoveItem | as add | journal |
| `humata board rm <item> [--revision n]` | `board.item_delete` | DeleteItem | as add | journal |
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
| `humata inbox approve\|deny <request> [--note text] [--always]` | `inbox.answer` | AnswerRequest | user-only; deny on a question the agent asks on its screen (D767) is the app's Dismiss (D781): it types nothing, and prints the item withdrawn as resolved when the dialog is already gone, or denied while the screen still asks, the session still needing you until it is answered there | answered on the machine, passphrase (AW-5) |
| `humata inbox reply <request> <text>` | `inbox.answer` | AnswerRequest (Reply) | user-only; a question the agent asks on its screen (folder trust, D767) takes only one of its choices, by its words or its number, and anything else is refused as usage | as approve |
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
| `humata files ls\|cat\|find <location>` | `file.list`, `file.read`, `file.find` | FileService | user; agent per MCP.md; an agent's read the rules ask about exits 12 `pending` with one request per session and path; `--wait` (or a terminal) waits for the answer and prints the read's result, `cat` included; once approved it also goes through when asked again (D768) | local for this machine |
| `humata files put <local-path> <location>`, `mkdir`, `mv` | `file.write`, `file.folder_create`, `file.rename` | FileService | user; agent: own clone and `shared/` on its machine (D755) | local |
| `humata files rm <location>…` | `file.trash` | MoveToTrash | user; agent: own clone | local |
| `humata cp\|mv <location>… <location\|machine> [--on-existing fail\|replace\|keep-both]` | `transfer.start` | StartTransfer | user; agent: `files_copy_in` and `file_fetch` only | waits when it needs another machine |
| `humata transfers`, `humata transfer pause\|resume\|cancel <id>` | `transfer.*` | TransferService | user; agent: its own | host |
| `humata trash [--machine m]`, `humata trash restore <item>` | `trash.list`, `trash.restore` | TrashService | user; agent: TRASH_RESTORE | host |
| `humata trash empty [--machine m] [--dry-run\|--plan t]` | `trash.empty` | PlanEmptyTrash, EmptyTrash | user-only | host only |

A location is `<machine>:<session>/<repo>/<path>`, `<machine>:shared/<path>`,
`<machine>:channel/<path>` or `<machine>:main/<repo>/<path>` (read-only).
A Desk session has no repository: `<machine>:folder/<session>/<path>` names
the session's own folder, with the same access as a clone of that session (D382).
Inside a session, a bare relative path is the agent's own clone, except
`shared`, `shared/` and `shared/<path>`: that is the channel's shared folder on the
machine the agent runs on (D777). The clone's own folder of that name is
`./shared`.

`humata cp|mv <location>… <machine>` names only the machine, and each source
lands where PL-3 says (PL-12): a channel's shared folder in that channel's
shared folder there, a session's clone or folder in its channel's shared
folder under a folder named for the session, both only on a machine of that
channel; anything else in `<machine>:received/<source machine>/` (PL-4).
Sources that land in different places start one transfer each, and the
result is then a list of them. A name already taken there keeps both unless
`--on-existing` says otherwise (PL-13); a send to a location fails on a
taken name unless it says otherwise. The machine is a word with no `:` and
no `/`. For the person that word is always a machine. Inside a session the
agent's last word of `cp|mv` is read in three steps (D756, amending D655):
1. A word that names a machine the agent can see (its own and its channel's,
   TP-1) is that machine.
2. Otherwise the word `shared` is the channel's shared folder on the agent's
   machine (D777), and a word with a `.` is a path in the agent's own clone.
3. Any other word is refused as usage (exit 2, `invalid_field`) and nothing
   starts, the same whether or not a machine the agent can't see has that
   name.

`./name` always means the agent's own clone, and `name/` does too, except
that `shared/` is the channel's shared folder (D777). A "not found" for a
bare `shared…` location says it looked in the channel's shared folder and that a
clone's folder of that name is written `./shared/…`.

and `name:` is a machine that must exist.

With one source, a destination that names a new file or an existing file
(not a folder, and not ending in `/`) copies to that file, as plain `cp a b`
does: the CLI stats it, and sends its folder with the last part as
StartTransfer's `name` (D769). Two or more sources always go into a folder.

On a machine's own daemon (not the host's), an agent's `host ls`,
`host show`, its look at another machine's files and a copy that names
another machine go up to the host, which answers as that agent; `this-mac`
names the host Mac there, not the machine. While the host is away these
are `host_unreachable` (exit 4), and nothing is kept for later (D768).

### 5.6 Machines, providers, skills, settings

| Command | Operation | gRPC | Who | Host away |
| --- | --- | --- | --- | --- |
| `humata host ls`, `humata host show <m>` | `machine.list`, `machine.get` | ListMachines, GetMachine | anyone (agents: their channel's). `host ls` also lists the machine the daemon runs on (its id from GetServerInfo; `--machine this-mac` names it too), which ListMachines never holds (D672 (Provisional)); on a machine's daemon an agent's call goes up to the host, which lists This Mac once (D768) | stale |
| `humata host add <ssh-host> [--name n]` | `machine.add_ssh` | AddSshMachine | user; agent: MACHINE_PROVISION. A name still held by a removal that hasn't finished is refused with kind `aborted` (exit 6), not `name_taken`: wait for the removal, then add again (D760) | host only |
| `humata host reconnect <m>` | `machine.reconnect` | ReconnectMachine | as add | host only |
| `humata host remove\|forget <m> [--dry-run\|--plan t]` | `machine.remove`, `machine.forget` | PlanRemove/RemoveMachine, PlanForget/ForgetMachine | user; agent: MACHINE_DESTROY (Never by default). Both clear the machine's skills sync state, so a machine added again under the name gets a first sync that asks (SK-5, D760), and take it out of every channel's machine list once it is removed (a removal that fails leaves it listed and in its channels) | host only |
| `humata host root <m> [<path>\|--default] [--move-channels] [--dry-run\|--plan t]` | `machine.change_root` | PlanChangeRoot, ChangeRoot | With no path it prints the root (`~/Humata` unless changed, PL-12); `--default` resets it to `~/Humata`. Changing is user-only (MACHINE_SETTINGS). The plan lists the channels under the old root (FC-10); without `--move-channels` the new root applies to new channels only | host only |
| `humata host vm-tier <m> on\|off` | `machine.set_vm_tier` | SetVmTier | user-only (MACHINE_SETTINGS) | host only |
| `humata host storage <m>`, `clear-build-cache`, `trim-logs` | `machine.storage`, … | GetStorage, ClearBuildCache, TrimLogs | storage: anyone; actions: user | host only |
| `humata host vars <m>`, `humata host var set <m> <NAME>`, `humata host var rm <m> <NAME>` | `host.list_variables`, `host.set_variable`, `host.remove_variable` | ListHostVariables, SetHostVariable, RemoveHostVariable | set, rm: user-only (HO-13); the value is read from the TTY with echo off or from stdin, never from an argument. Running agents keep the old value | host only |
| `humata host trust-key <m> --fingerprint <sha256>` | `host.trust_new_key` | TrustNewHostKey | user-only; the fingerprint must be typed, not pasted from the error | host only |
| `humata machine script list [<m>]` | `machine.script_list` | ListMachineScripts | anyone who may read the machine (`machine_read`); prints each rule, its triggers and steps, each machine's last outcome and the script names | stale |
| `humata machine script check` | `machine.script_check` | CheckMachineScripts | as list; reads the rules file and script names, lists every problem and warning; exit 0 when valid, 2 (usage) when not | stale |
| `humata machine script run <rule> <m> [--session s] [--wait]` | `machine.script_run` | RunMachineScript → Operation (MACHINE_SCRIPT) | user; agent: MACHINE_SCRIPT_RUN (Never by default, D688). The trigger is `manual`, or the session's when `--session` is given; `--wait` follows the operation (§6) | host only |
| `humata machine script log <m> [--rule r] [--limit n]` | `machine.script_log` | ListMachineScriptRuns | as list; the host's run log for the machine, newest first, with each step's exit and output tail | stale |
| `humata provider ls`, `humata provider config edit` | `providers.*` | ListProviders, Get/SetProvidersConfig | edit: user-only | host only |
| `humata provider import <file> [--dry-run\|--plan t]` | `provider.import` | PreviewProviderImport, ImportProvider | user-only (MP-23); prints the file's full text and every command it will run, then asks for presence; the plan is good once, for ten minutes, and only while the providers file is unchanged (D771) | host only |
| `humata provider create <provider> <name> [--param k=v]…`, `start\|stop\|status <m>` | `provider.*` | CreateMachine, RunProviderAction | user; agent: MACHINE_PROVISION | host only |
| `humata provider delete <m> [--dry-run\|--plan t]` | `provider.delete` | RunProviderAction(DELETE), DeleteWhatWasMade | user; agent: MACHINE_DESTROY | host only |
| `humata provider login <provider>` | `provider.login` | InteractiveLogin | user-only (interactive) | host only |
| `humata skills sync [machine…] [--dry-run]` | `skills.sync` | PreviewSkillsSync, SyncSkills | user; agent: MACHINE_PROVISION (SK-2). `--dry-run` works for every sync (SK-11); offline machines are named as skipped (SK-10) | host only |
| `humata bridge ls [--machine m\|--session s]`, `humata bridge close <forward>` | `bridge.list`, `bridge.close` | ListPortForwards, ClosePortForward | ls: anyone; close: user, agent its own (HO-11) | host only |
| `humata settings [get\|set <key> <value>]` | `settings.*` | Get/UpdateSettings | set: user-only; every section is served: default agent, terminal, retention defaults, long lines and notifications are kept in the humata home's `config/settings.toml`, skills in `skills-settings.json` (D770) | host only |
| `humata settings contain on\|off [--channel c \| --channel c --inherit]` | `settings.update`, `channel_settings.update` | UpdateSettings(security), UpdateChannelSettings(contain_agents) | user-only (SEC-6); applies when a session next launches or resumes | host only |
| `humata agents ls\|add\|edit` | `agents.*` | AgentService | add, edit: user-only. `ls` lists the enabled agents; `edit` also finds the ones turned off (`include_disabled`) | host only |
| `humata pin\|unpin\|vip\|unvip <thing>`, `humata pinned` | `marks.set`, `pinned.list` | MarkService | user-only (NV-20); a new pin goes at the end (NV-25) | host only |
| `humata pinned reorder <thing>…` | `pinned.reorder` | ReorderPinned | user-only (NV-25) | host only |
| `humata sessions search <words…> [--channel c] [--machine m] [--agent a] [--since t] [--until t]`, `humata session read <session> [--from n] [--to n]` | `session.search`, `session.read` | SearchSessions, ReadTurns (HistoryService) | anyone; an agent is limited to its own channel by the host | host only |
| `humata repos search <query…> [--owner o]… [--all-of-github] [--your-repos]`, `humata repos list`, `humata repos remove <owner/name>`, `humata repos forget-all`, `humata repos scope show`, `humata repos scope set [--owner o]… [--no-include-me]` | `repo.search`, `repo.library`, `repo.remove`, `repo.forget`, `repo.scope`, `repo.update_scope` | SearchRepos, ListRepoLibrary, RemoveFromRepoLibrary, ForgetRepoLibrary, GetRepoScope, UpdateRepoScope (RepoService) | search, list, scope show: anyone (global reads, D278); remove, forget-all, scope set: user-only (RL-4, RL-7, D479). With no flag a search uses the saved scope; a flag changes it for that one search and is never saved. A GitHub failure is printed as the problem line and the exit code is still 0 (RL-10). forget-all says how many repos go and asks (or --yes; --dry-run only says it); scope set with no --owner saves your own repos and refuses --no-include-me alone (D567) | host only; a machine's command asks the host and says so while it is away (RL-12) |

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

A read the machine answered from its own copy while the host is away adds `"stale_as_of": "<RFC 3339 UTC>"` to the object (the time the host answered it, AW-7); without `--json` the same time is printed on stderr. A live read has neither.

| Exit | When |
| --- | --- |
| 0 | Done: applied, or journaled (unless `--strict`) |
| 2 | Usage: unknown command, bad flag, ambiguous name (`invalid_field` from parsing) |
| 3 | `not_found` |
| 4 | `denied`, `not_in_channel`, `main_clone_read_only`, `push_refused` |
| 5 | `user_only`, `presence_required`, `identity_mismatch` |
| 6 | `revision_conflict`, `plan_stale`, `already_answered`, `name_taken` |
| 7 | Other preconditions: `unpushed_work`, `session_busy`, `user_away`, `request_withdrawn`, `host_key_changed`, `host_key_unknown`, … |
| 8 | `machine_offline`, `host_unreachable`, `ssh_failed` |
| 9 | An external tool failed: `git_failed`, `github_cli`, `provider_step_failed`, `step_failed`, `review_partly_posted` |
| 10 | `disk_full`, `rate_limited`, `passphrase_locked` |
| 11 | `checksum_mismatch`, `offset_trimmed` |
| 12 | `pending`: still waiting for the user when `--wait` ran out, or when the wait ended with no answer (the request stays pending) |
| 13 | `declined`: you answered no to the CLI's own "Continue?"; nothing was changed |
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

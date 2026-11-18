# CLI: the whole `humata` surface, as a person meets it

Layer: explored · macOS · harness: throwaway `humatad2 serve` plus the `humata` binary (phase 2 only). Phase 1 is code and test mapping; no run has happened, so every run step is PENDING RUN.

Status of this record: phase 1 handed back early, on the coach's pause. Steps 1 to 4 are mapped from code. Steps 5 onward are not yet written in full. See "Left to do".

## Setup

- Throwaway daemon, phase 2 only. From the repo root:
  `export PATH=$PWD/.tools/bin:$PATH; bazel build --jobs=4 //apps/humata2/cli:humata //apps/humata2/daemon/humatad:humatad2 //apps/humata2/daemon/holder:humata2_holder`
- Then, with `SCRATCH` set to the scratchpad `verify/evidence/X-N/` folder and `HOME=$SCRATCH/home`:
  `bazel-bin/apps/humata2/daemon/humatad/humatad2 serve --socket $SCRATCH/humatad.sock --state $SCRATCH/state --root $SCRATCH/root --holder bazel-bin/apps/humata2/daemon/holder/humata2_holder --shell /bin/sh --shell-arg=-i`
  (flags from `daemon/humatad/src/serve.rs` `ServeArgs`; the holder path is an assumption to confirm with `humatad2 serve --help`).
- CLI: `export HUMATA_SOCKET=$SCRATCH/humatad.sock HOME=$SCRATCH/home; bazel-bin/apps/humata2/cli/humata <args>`.
- Never the owner's `~/.humata2`, `~/.claude`, `~/.codex`, `~/.copilot`.

## Steps

### 1. `humata --help` and the top-level command list
- **Screen:** clap help from `cli/src/args.rs` (`#[command(name = "humata", about = "Humata from a terminal: ...")]`). Subcommands listed with their `///` text.
- **Offered:** every top-level command in `args.rs` `enum Command`: status, whoami, stop, start, ls, channels, channel, topology, activity, events, audit, ops, attach, session, shell, conversation, board, views, view, diff, comment, review, inbox, access, login, logout, logins, token, files, cp, mv, transfers, transfer, trash, host (alias `machine`), provider, skills, bridge, settings, agents, pin, unpin, vip, unvip, pinned, sessions, desk, repos, report, doctor, turn, hook, opencode.
- **Action:** `humata --help`.
- **Result:** PENDING RUN. Code predicts that variants with no `///` line show blank descriptions in the list. From `args.rs`, these have no doc line: `channel`, `session`, `files`, `token`, `provider`, `skills`, `bridge`, `agents`, `logout`, `shell` (has none), `conversation`, `diff` (has one), `hook`/`turn` (have one). Confirm by reading the rendered list.
- **Evidence:** `verify/evidence/X-N/help-top.txt` (to write).
- **Replay check:** `humata --help` exits 0 and lists `session` and `channel`.

### 2. Usage errors (exit 2) and the `--json` form
- **Screen:** clap error on stderr (plain) or the JSON envelope on stdout (with `--json`), built in `lib.rs` `usage()`.
- **Offered:** none; this is a refusal.
- **Action:** `humata nope`; `humata session start --bogus`; `humata session fork s1` (missing `--kind`); `humata nope --json`.
- **Result:** PENDING RUN. Code: exit 2 (`exit::USAGE`). With `--json` the message is the first clap line and the fix says "Fix the command line; `humata --help` lists the commands...". Plain form prints clap's text.
- **Evidence:** `verify/evidence/X-N/usage-*.txt`.
- **Replay check:** `humata nope; echo $?` prints 2.

### 3. No daemon running (exit 7)
- **Screen:** not a screen; stderr text.
- **Offered:** none.
- **Action:** `env -u HUMATA_SOCKET HOME=$SCRATCH/empty humata status; echo $?`, and the same with `HUMATA_SOCKET=$SCRATCH/none.sock`.
- **Result:** PENDING RUN. Code (`cli/src/daemon.rs` `NotRunning`, `cli/src/error.rs` `daemon_failure`): message "humatad isn't running: no socket at <the paths tried>", kind `daemon_not_running`, exit 7. Fix text: "Start Humata (on the Mac, open the app), or set HUMATA_SOCKET to the daemon's socket."
- **Surprise (code):** the fix never names `humatad2 serve`, so a person running a throwaway daemon is told to open the Mac app.
- **Surprise (code):** `humata turn` and `humata hook` connect before they run (`cli/src/lib.rs` `main`). With no daemon they print the error and exit 7. `api/CLI.md` §5.2 says `turn` "exits 0 whatever happens" and `commands/hook.rs` says "always exits 0, and prints nothing when there's no answer: no daemon". Likely bug. Check: `HUMATA_SOCKET=$SCRATCH/none.sock humata turn working; echo $?` should print nothing and exit 0.
- **Evidence:** `verify/evidence/X-N/no-daemon-*.txt`.
- **Replay check:** `HUMATA_SOCKET=/nonexistent humata status; echo $?` prints 7.

### 4. Name lookups (not found, exit 3; ambiguous, exit 2)
- **Screen:** stderr text.
- **Offered:** none.
- **Action:** `humata attach nosuch`; `humata channel rename nosuch x`; `humata session stop nosuch`. Ambiguity needs two sessions with the same title.
- **Result:** PENDING RUN. Code (`cli/src/resolve.rs` `pick`, `cli/src/error.rs` `NoMatch`): "no session is named \"nosuch\"", kind `not_found`, exit 3, fix "Check the name, or list them to find the session's id." Ambiguous: exit 2, lists matches as "title (id)".
- **Evidence:** `verify/evidence/X-N/notfound-*.txt`.
- **Replay check:** `humata session stop nosuch; echo $?` prints 3.

## Commands not yet walked

Steps 5 onward, one group each, each with `--help`, a happy run, a usage error and a not-found, with exit codes. Not written yet. Groups, from `args.rs`:

- Looking: status, whoami, ls, channels, channel (create/rename/repo/machine/grid/settings/delete), topology, activity, events, audit, ops.
- Sessions: session (start/fork/read/rename/stop/resume/archive/delete/restore/rebuild/move/repo/own-copy/continue/push/note/ask/type/queue/tabs/send-now), attach, shell, conversation, sessions search, desk keep.
- Boards and reviews: board, views, view, diff, comment, review.
- Inbox and access: inbox, access, login, logout, logins, token.
- Files: files ls/cat/find/put/mkdir/mv/rm, cp, mv, transfers, transfer, trash.
- Machines: host (alias machine) ls/show/add/reconnect/remove/forget/root/vm-tier/storage/clear-build-cache/trim-logs/stop/start/vars/var/trust-key, machine script list/check/run/log.
- Providers, skills, bridge, settings, agents, pin/unpin/vip/unvip, pinned.
- Repos, report, doctor restarts, turn, hook, opencode, stop, start.

## Not reachable headless

- `humata login` needs a real terminal (`CliError::LoginNeedsTerminal`) and a presence answer from the Mac app. Headless, only the error path is checkable.
- `humata attach`, `shell`, `hook permission`, and anything that waits for presence need the app or a TTY.
- Provider commands need a real provider.

## Compared with api/CLI.md

Present in `args.rs`, not in CLI.md:
- `humata desk keep`, `humata report record|stop|snapshot`, `humata opencode`, and the top-level `humata stop` and `humata start` (CLI.md documents only `session stop` and `host stop`).
- `humata session archive --push-first` (CLI.md documents `archive` without flags).

Documented in CLI.md, different in code:
- `humata diff --against`: CLI.md says `base|last-turn|turn n`. Code (`args.rs`) defaults to `main` and its help lists `main, uncommitted, last-reviewed, fork, pr, or another session`.

Not in `args.rs`:
- `humata search`: there is no top-level search. The search is `humata sessions search`.
- `humata marks`: there is none. Marks are `pin`, `unpin`, `vip`, `unvip`, `pinned`.

## Surprises

- The no-daemon fix tells a person to open the Mac app and never mentions `humatad2 serve` (step 3).
- `turn` and `hook` exit 7 with stderr when no daemon is up. Docs and code comments say they exit 0 silently (step 3). Likely bug.
- Many top-level commands have no help line, so `humata --help` shows blanks (step 1). Confirm in phase 2.
- `humata turn` and `hook` with a bad argument exit 2 from clap, before their own always-exit-0 logic runs. Hooks always pass valid values, so this is minor.

## Left to do

- Phase 2 (after the coach's go): run steps 1 to 4 and evidence.
- Write steps 5 onward, one group each, with PENDING RUN commands.
- Split into `X-N-*.md` files if the record gets long.

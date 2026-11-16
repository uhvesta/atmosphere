# HOOKS: Agents tell Humata when a turn ends and ask for approval in the Inbox, without terminal guesswork

- **Status:** open
- **Depends on:**
  - ASK: approval asks land as Inbox requests
  - FL: turn ends are already detected (hooks for Claude, output quiet for others, D169)
- **Unblocks:** V1
- **Layer:** L3 · **Lane:** daemon

## Outcome
Every agent stays its own TUI on the owner's subscription (no ACP, no SDK adapter: owner, 2026-11-15). Where the agent offers hooks or a local API into the running TUI, Humata uses them; otherwise it types into the PTY as today.
- Codex and Copilot report turn ends through their hooks (Stop / agentStop) instead of the 3 s output-quiet fallback.
- Claude Code, Codex and Copilot send permission asks through a blocking PermissionRequest hook to the Inbox; answering in the Inbox allows or denies in the TUI. The TUI's own prompt stays as the fallback when the hook times out or the daemon is gone.
- OpenCode is driven through its local HTTP server (prompt, abort, events, permission replies); typing into the PTY stays as the fallback.
- Spikes, behind a developer switch: Codex `--remote` (second client on the TUI's thread), Claude Code Channels (research preview; startup warning).

## Exit criterion
Boundary suites with recorded hook payloads per agent: a turn end fires the turn-ended event; a permission hook raises one Inbox request; approve/deny/timeout each give the agent the right answer; with the daemon down the hook exits so the TUI prompts itself. Fake-agent e2e: an Inbox approval reaches a waiting hook. Hook timeouts are checked per agent and written down.

## Read these (pointers, nothing else)
- Research: `workflow/milestones/HOOKS-research.md` (sources, per-agent table, unverified items)
- SPEC: SE-21, SE-22 (turn ends); IN-* rows for agent asks. Decisions: D169.
- Code (under `apps/humata2/`): `daemon/humatad/src/session_service/turns.rs`, `session_service/watch.rs`, the hook install code next to them (grep `hook`).

## Out of scope
ACP and any adapter process that replaces the TUI. Gemini permission answers (its event is observe-only).

## Lane (written by the manager)
- Design notes (D191–D193; facts in HOOKS-research.md › Verified from primary sources):
  - The agent's blocking permission hook runs `humata hook <agent> permission` (stdin payload, `2>/dev/null || true`). The CLI forwards the bytes on the server stream `SessionService.Hook`; the daemon parses them with the pure crate `daemon/agent_hooks`, raises one Inbox card (kind SESSION, CommandPreview, Approve/Deny), sends keepalives, then once the bytes the hook prints. Empty bytes are no decision: the TUI asks.
  - The card lives while a hook holds the stream; the last stream dropping withdraws it (CALL_ENDED). An answer commits only while a stream is alive. Same ask again (session + hash) attaches to the pending card. Not saved when raised; answers and withdrawals are. Answer all skips them. The Inbox entry gets a `permission` discriminator.
  - Hold: 3600 s in every agent's config, daemon ends at 3590 s, CLI cap 3595 s. Turn hooks 10 s (Codex Interrupt 3 s).
  - Claude: the blocking hook joins D169's async `turn needs-input` in the PermissionRequest group of `claude-hooks.json`. Codex: `-c hooks=<inline table>` (`{codex_hooks}` in a kind's args), no trust bypass; user trusts once in `/hooks`. Copilot: `--plugin-dir <state>/copilot-plugin` (`{copilot_plugin}`). Both stay `turn_end: output` until live checks (owed, GAPS_REVIEW).
  - No builtin Codex/Copilot kinds yet: their start args can't pin a conversation id (PER-12), so that is a separate step; a kind in `--agents` JSON uses the placeholders today.
- Claims: new `daemon/agent_hooks/` (all); new `daemon/humatad/src/session_service/hooks.rs`; new `daemon/humatad/src/inbox/permission.rs` and small edits in `daemon/humatad/src/inbox.rs` (`mod permission;`, the `permission` field, its `decide()` arm, Answer all skip); `daemon/humatad/src/session_service/turns.rs` (`hooks_json` only: one blocking entry); `daemon/sessions/src/agents.rs` (placeholders); new `daemon/humatad/tests/hooks.rs`; new `cli/src/commands/hook.rs`, new `cli/tests/hook.rs`; this Lane; HOOKS-research.md; DECISIONS D191–D193. Seams to main (scratchpad `hooks/seam_proto.py`, `hooks/seam_wiring.py`, `--check`): `sessions.proto`, `session_service.rs` (mod + trait method), `remote_work.rs` (test impl), `daemon/humatad/BUILD.bazel` (dep), CLI registration (`args.rs`, `commands/mod.rs`, `cli/src/daemon.rs`); later `serve.rs` (install Codex/Copilot config, `with_agent_hooks`).
- Tasks:
  - T1 crate `humata2_agent_hooks` (parse, render, config): tests `agent_hooks:{payloads,config}_test`. Done (Haiku), green.
  - T2 CLI `humata hook`: `cli/src/commands/hook.rs` and `cli/tests/hook.rs` written by the manager; run `cli:hook_test` after seam 2.
  - T3 daemon: `session_service/hooks.rs` + `inbox/permission.rs` written by the manager; tests `humatad:hooks_test` (e2e through the fake agent and direct streams) and `humatad2_unit_test` (paused-time 3590 s end, plugin install). Run after seam 2.
  - T4 config install: `{codex_hooks}`/`{copilot_plugin}` in agents.rs, done, `sessions_unit_test` green; `install_agent_hooks` in hooks.rs; seam 3 `hooks/seam_agent_hooks.py` (serve.rs). Claude blocking entry plus 10 s turn-hook timeouts in `hooks_json`: scratchpad `hooks/own_turns_edit.py`, applied after seam 2.
  - T5 docs: seam 4 `hooks/seam_docs.py` (CLI.md row, GRPC.md SessionService). GAPS_REVIEW owed live checks go to main in the report.
  - T6 app side: a withdrawn request (hook stopped, agent answered in its own prompt) used to stay waiting in the app; now the live client maps a WITHDRAWN update to `Event::RequestGone` and the app drops it. Tests `features/app:request_gone_test`; live flow `desktop:live_inbox_test` › `an_agents_permission_ask_waits_in_the_inbox_and_allow_reaches_its_hook` (real daemon, real `humata hook` from the agent's tab: card shows the command, withdrawal leaves the Inbox, Allow in the app reaches the hook).
  - OpenCode driver and the two spikes: later batch.
- Worker log: T1 Haiku 5.5 high (crate) green first run. T2-T4 by the manager. Green 2026-11-16: agent_hooks:all, cli:all (hook_test 6), humatad hooks_test 10, turns_test 9, inbox_test 24, humatad2_unit_test 290, sessions_unit_test.
- Waiting on: main applying seam_docs.py (seams 1-3 applied). Later batch: OpenCode HTTP/SSE driver, Codex `--remote` and Claude Channels spikes, builtin Codex/Copilot kinds (need conversation pinning, PER-12), Settings › Agents Codex trust step, live checks owed (Codex `-c hooks` loads after `/hooks` trust; Copilot `--plugin-dir` hooks run and permissionRequest fields match testdata).
- Build gates: `//apps/humata2/daemon/agent_hooks:all`, `//apps/humata2/cli:hook_test`, `//apps/humata2/daemon/humatad:hooks_test`, `:turns_test`, `:inbox_test`; `//apps/humata2/daemon/humatad:humatad2`, `//apps/humata2/desktop:humata2`.
- 2026-11-16 D199 (owner via main): Settings › Agents › "Ask for permission in the Inbox", one switch, on by default. When off, `hook_of` answers each ask with no decision and raises no card, read at each ask. The setting lives on the host (`Settings.agents`, proto seam 5 `scratchpad/hooks/seam_settings_proto.py`). Failing tests first: `hooks_test` +2, `features/settings:agent_asks_test`, `preferences_test` receives the read. App side by a Haiku worker.
- 2026-11-16 OpenCode driver (D201, D201a; pinned v2.3.0, facts in HOOKS-research.md › OpenCode):
  - Shape: the built-in `opencode` kind runs `humata opencode --conversation … --state <state>/opencode/<conv>.json`. That wrapper starts `opencode serve --hostname 127.0.0.1 --port 0` with a fresh password, creates session `ses_<26 of the conversation id>` and runs the TUI `--server <url> -s <id>`. Its link reads `/api/event` and reports turns via ReportTurn. Each `permission.asked` opens the existing `SessionService.Hook` stream (agent `opencode`) and the answer is posted to `/api/session/<ses>/permission/<id>/reply`. Stop is typed into the PTY. The daemon has no HTTP client.
  - Claims: `daemon/agent_hooks` (OpenCode parse/render, tests/opencode.rs, testdata/opencode_asks); new crate `daemon/opencode/` (sse, event, activity, serve, state, link, http, wrapper; tests per module; fixtures in testdata/); `daemon/sessions/src/agents.rs` (`opencode()` kind, `with_wrapper`); new `cli/src/commands/opencode.rs`, `cli/tests/opencode.rs`; small edits `cli/src/{call,output,error}.rs` (`Outcome::Exited`, `CliError::OpenCode`).
  - Seams: 6 (hyper, http-body-util; applied); 7 `scratchpad/hooks/seam_opencode_cli.py` (args.rs, commands/mod.rs, cli BUILD dep, serve.rs `with_wrapper`).
  - Workers: Haiku (sse, event, activity, serve, state), Sonnet (link, http, wrapper), Sonnet (live verification of v2.3.0). agent_hooks:all and humatad:hooks_test green; the opencode crate, sessions_unit_test and cli:opencode_test are pending main's run.
  - Follow-ups: OpenCode API prompts (typing stays the fallback); `/new` in the TUI leaves the mapped session; the too-old fallback prints a note, not a check fact; a real end-to-end run is owed (including whether OpenCode takes the 30-char custom id).
- 2026-11-16 Codex trust step (D202, Fable-advised):
  - What it does: Settings › Agents shows "Trust Humata's hooks in Codex" while some kind carries Humata's `codex_hooks` value and that value's sha256 hasn't been heard. Heard means a Hook or ReportTurn arrived from such a session; it is stored in `agents.codex_hooks.heard`. The step shows from the start and has no dismiss.
  - Daemon: `inbox/permission/codex_trust.rs`; calls in hooks.rs and turns.rs; `settings_service.rs` sets the output-only `AgentSettings.codex_hooks_need_trust` (proto seam 8 `scratchpad/hooks/seam_codex_trust_proto.py`, applied). Test: `humatad:codex_trust_test`.
  - App: `Daemon::codex_needs_trust` (fake defaults to false, live reads it from settings) and `features/settings` `CodexTrustShown`. The card is in ui/src/settings.rs. Test: `features/settings:codex_trust_test`. The Sonnet worker updated the agent_asks and preferences tests.
  - Follow-ups: the step clears only when Settings is reopened (no live push); a heard turn hook doesn't prove the permission hook is trusted.
  - Note for later crates: each new crate needs a Cargo.toml and a root members entry, sent as a seam. Main added them for daemon/opencode.
- 2026-11-16 Spikes:
  - Codex `--remote` second client: verified live on 0.169.0, facts in HOOKS-research.md. A second app-server client sees the TUI's thread, turns and approvals, and can answer them (first answer wins, `serverRequest/resolved` is the truth) and interrupt. A `humata codex` wrapper like OpenCode's is possible but not built, and it rests on an experimental API.
  - Claude Channels: deferred, docs only. A live run needs a claude.ai login, a dev flag with a startup dialog, and changes to the real ~/.claude. No developer switch gates any code yet.
- Ready to commit: core yes; T6 after `features/app:request_gone_test`, `desktop:live_inbox_test`, `clients/live:all` pass; docs seam pending; D199 after seam 5, hooks_test, settings tests, clients:all, clients/live:all, ui:all, every_action_test pass

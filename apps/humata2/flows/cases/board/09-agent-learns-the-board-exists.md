# An agent finds out that To-do / the Board exists

Layer: verify (To-do, agents). Platforms: macOS (this Mac). A remote machine is the same code.

The owner's rule: agents must know To-do / the Board exists. This case walks every way an agent could find out, and says which are real.

## Starting state

- A real `humatad2` runs. `#payments` has one session, "Ledger", with an agent (Claude Code, Codex or Copilot).
- The agent has been told nothing about Humata. Its prompt is empty.
- The owner's own agent config is not used: the run has its own `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `COPILOT_HOME`.

## Reproduce headless

- What the MCP server tells an agent: `bazel test //apps/humata2/daemon/mcp:schemas_test --test_filter=tools_list_offers_exactly_the_tools_of_mcp_md` and `bazel test //apps/humata2/daemon/mcp:protocol_test --test_filter=initialize_names_the_server_humata_and_offers_tools`.
- `Do next` prefilling the agent's first message on fakes: `bazel test //apps/humata2/features/app:todo_route_test --test_filter=do_next_opens_new_session_filled_from_the_top_item`.
- By hand on the rig, inside a session tab: `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | humatad2_admin mcp --socket "$HUMATA_SOCKET" > "$D/tools.json"`. Then check the names starting with `todo_` and `board_`.
- What the agent was started with: `humata --json session tabs Ledger`, and read the agent's argv from the session record under the rig's state folder. Look for an MCP config argument.
- In the tab: `command -v humata; echo $?` and `env | grep HUMATA`.
- Not headless: a real model deciding to use To-do. Case 14 does that with a real Copilot.

## Steps

1. **I do:** (as the agent) call `tools/list` on Humata's MCP server.
   **I see:** three To-do tools (`todo_next`, `todo_add`, `todo_close`, BOARDSIMPLE D839) alongside the seven board tools (`board_items_list`, `board_item_get`, `board_item_history`, `board_item_create`, `board_item_update`, `board_item_move`, `board_item_delete`). Each has a one-line description.
   **State and events:** none. `tools/list` works even without a token.
   **Status:** Fakes only. `daemon/mcp/tests/schemas.rs::tools_list_offers_exactly_the_tools_of_mcp_md`; `daemon/mcp/tests/binding.rs::without_a_token_tools_list_still_works`.

2. **I do:** read the server's greeting (`initialize`).
   **I see:** "Humata's tools for this session…", how pending calls work, and to branch on `error.kind`. It does not name To-do or the Board specifically.
   **Status:** Works on fakes for the greeting. Not built for a mention of To-do / the Board: `daemon/mcp/src/server.rs:552`.

3. **I do:** check that my agent was started with that server.
   **I see:** it was not. Claude Code is started with `--session-id … --settings <hooks>`. Codex with `-c hooks=…`. Copilot with `--session-id … --plugin-dir <plugin>`, and the plugin holds `plugin.json` and `hooks.json` only.
   **State and events:** no MCP config is written for any agent.
   **Status:** Not built, no milestone. `daemon/sessions/src/agents.rs:572` (Copilot), `:552` (Codex), `:615` (Claude); `daemon/agent_hooks/src/lib.rs:373`. `api/MCP.md:13` says "the holder writes the agent's MCP config", so steps 1 and 2 never happen automatically for a real agent.

4. **I do:** look for a skill about the Board / To-do.
   **I see:** `humata-board` in my skills folder (`~/.claude/skills`, `~/.codex/skills` or `~/.copilot/skills`), with `board.md` and `shared/basics.md` beside its `SKILL.md`. The daemon installed it at its start (D790).
   **Status:** Works (real daemon) for the install: `daemon/humatad/tests/skills.rs::the_daemon_installs_its_own_skills_for_the_agents_set_up_and_never_syncs_them`. Not headless: a real model picking the skill up from a plain ask.

5. **I do:** run `humata --help` in my shell.
   **I see:** (if found) a `board` line: `humata board --help` lists `add`, `edit`, `move`, `rm`, `history`. There is no `humata todo` subcommand yet.
   **State and events:** none.
   **Status:** Code, no test for the help text (`cli/src/args.rs`). `cli/tests/exit_codes.rs` checks only that `--help` exits 0.

6. **I do:** check that `humata` is on my `PATH`.
   **I see:** not unless the person's own shell profile put it there. The tab gets `HUMATA_SESSION_TOKEN` and `HUMATA_SOCKET` only.
   **Status:** Not built, no milestone. `daemon/humatad/src/identity.rs`. Humata's own hooks call `humata` by its full path for this reason (`daemon/agent_hooks/src/lib.rs`).

7. **I do:** run `humata whoami` and `humata board` by full path.
   **I see:** I am the agent of session Ledger in `#payments`; then the channel's items.
   **State and events:** the token binds the call to my session.
   **Status:** Works (real daemon) for token-bound `humata board` calls: `desktop/tests/live_flows/todo_asks.rs::approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call`. A plain list from a tab: Code, no test.

8. **I do:** get started from a To-do item (**Do next** or **Start a session on this** in the drawer).
   **I see:** from **Do next**, the New session sheet prefills `first_message` with the item's title and description (`features/app/src/route.rs::fill_do_next`); from the drawer's **Start a session on this** (`StartNewForItem`), the sheet's title and item link are filled, while `first_message` starts blank with a placeholder naming the item.
   **Status:** Fakes only for **Do next**: `features/app/tests/todo_route.rs::do_next_opens_new_session_filled_from_the_top_item`. Works (real daemon) for starting from the drawer: `desktop/tests/live_flows/todo_asks.rs::starting_a_session_from_the_todo_drawer_opens_it_and_moves_the_item_in_progress`.

## Branches

### The person tells the agent

When the person types "Use `<path>/humata board --help` to plan on the Board." The agent can then do everything in cases 10, 12 and 13.
**Status:** Works (real daemon) for the calls (`desktop/tests/live_flows/todo_asks.rs`). This is today's only way without hand-registered MCP.

### The person adds the MCP server by hand

When the person registers `humatad2 mcp` in the agent's own config. The server needs the session's token and must descend from the session's holder, so a user-level config works only if the agent passes its environment on. Humata's contained launches keep MCP servers in the copy and out of the real config.
**Status:** Code, no test. No test starts a real agent CLI with `humatad2 mcp`.

## Expected outcomes

- [ ] Every agent Humata starts has Humata's MCP server, with the `todo_*` and `board_*` tools listed.
- [ ] `humata` is on the `PATH` of every session tab.
- [ ] Something the agent reads at start says To-do exists and when to use it.
- [ ] An agent started for an item is told which item in its first message.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1 Tool list (`todo_*` + `board_*`) | Fakes only | `mcp/tests/schemas.rs` |
| 2 Greeting | Not built (no To-do/Board mention) | `daemon/mcp/src/server.rs:552` |
| 3 Server registered | Not built | `daemon/sessions/src/agents.rs:572` |
| 4 Skill (`humata-board`) | Works (real daemon) for the install | `humatad/tests/skills.rs` |
| 5 `--help` | Code, no test | `cli/src/args.rs` |
| 6 `PATH` | Not built | `daemon/humatad/src/identity.rs` |
| 7 By full path | Works (real daemon) | `live_flows/todo_asks.rs` |
| 8 Told its item | Fakes only for Do next; blank on drawer start | `todo_route.rs`, `new_session.rs` |
| Told by the person | Works (real daemon) | `live_flows/todo_asks.rs` |
| Hand-registered MCP | Code, no test | none found |

## Gaps and likely bugs

- Not built: registering the MCP server with each agent at launch. Every MCP tool (including `todo_next`, `todo_add`, `todo_close` and `board_*`) is unreachable automatically by a real agent.
- Not built: `humata` on the tab's `PATH`, and a `humata todo` CLI subcommand.
- `StartNewForItem` from the drawer leaves `first_message` blank (unlike `Do next`, which prefills title + description).
- Likely bug: `api/MCP.md:3` says "Agents reach Humata only through MCP". In the build they can reach it only through the CLI unless MCP is configured by hand.

# A Copilot session on Claude Haiku 5.5 does a tiny task, asks through the Inbox, and resumes after a stop

Layer: real (real Copilot CLI, real `humatad2`, this Mac). Platforms: macOS. The VM is not used here; `real/06` moves the same session there.

## Starting state

- This Mac runs a real `humatad2` in a temp state folder. It was started with an `--agents` JSON file that defines a custom kind `copilot` (`conversation_format` `copilot-session-state`, `turn_end` `hooks`, `--plugin-dir {copilot_plugin}` in its start arguments). Built-in `copilot` does not exist yet (see the branch on built-in kinds).
- The channel `acme/api` has this Mac as a machine and one clone. `README.md` is clean.
- The Inbox is empty. Permission asks wait in the Inbox (the default, D199).
- The Copilot CLI is the installed one, version 1.0.112-0 or later (the Copilot adapter is pinned there).
- No session exists yet. The app is open on the channel, or I use the `humata` CLI.

## Test setup

- Use a fresh temp folder for the run. Pass `COPILOT_HOME`, `COPILOT_CACHE_HOME` and `HOME` pointing into it to `humatad2`. The daemon hands `COPILOT_HOME` to every Copilot it starts and places conversations there (`daemon/humatad/src/serve.rs:455-466`). Never touch the owner's real `~/.copilot` or `~/.humata2`.
- Start `humatad2 serve --agents <file>` with that state folder and root. Bazel builds it (no cargo).
- **Needs the owner's login.** The real model needs a Copilot device login. The token must land only in the temp `COPILOT_HOME`. A free Copilot account may not offer Claude Haiku 5.5; say so if `/model` does not list it.
- **Alternative with no login:** a loopback model server with private state, as in `terminal/provider/tests/*_resume.py`. Run `//apps/humata2/terminal/provider:copilot_resume_test` and `:copilot_fork_test` (manual, macOS only). That proves Copilot resume and fork in the terminal component with a deterministic model. It does not go through `humatad2` and does not prove the real model.
- **Home leak.** In the REALFLOW run Copilot wrote `recentModelIds` to the owner's `~/.copilot/config.json` despite "this session only". Always use an isolated `COPILOT_HOME` with a copied login. A run without one touches the owner's home.
- **Needs a free screen** for every step that opens the app window. The headless equivalent is the `humata` CLI, or a `desktop/tests/live_flows/*` style test against a real `humatad2`.
- Cleanup: stop the daemon, delete the temp folder, and confirm the owner's own Copilot home is unchanged.

## Steps

1. **I do:** start `humatad2` with the `--agents` file and open channel `acme/api`.

   **I see:** Settings › Agents lists `copilot` beside Claude Code and OpenCode.

   **State and events:** the kind is read from the JSON. Its hook placeholders are filled with the daemon's Copilot plugin folder (`daemon/agent_hooks/src/lib.rs:350`, `copilot_plugin`: `permissionRequest`, `agentStop`, `userPromptSubmitted`, `postToolUse`, `errorOccurred`).

   **Status:** Fakes only. `daemon/sessions/src/agents.rs::codex_and_copilot_hooks_fill_their_placeholders` checks the fill with strings. No test starts the daemon with a real Copilot kind.

2. **I do:** start a session with the `copilot` kind in `acme/api` on this Mac, titled `Tiny README edit`.

   **I see:** a session with an agent tab. The Copilot CLI starts. A folder-trust prompt comes first; I accept it, then the prompt shows.

   **State and events:** the session record gets an `AgentLaunch` with the conversation id, `argv`, `resume_argv`, the conversation file under `COPILOT_HOME/session-state/<id>/events.jsonl` and the CLI version Copilot printed (CV-3). It has no field for a model.

   **Status:** Works (real daemon) for the daemon half. `daemon/humatad/tests/continue_in.rs::a_copilot_session_keeps_the_version_its_cli_said_at_launch` starts a Copilot kind with a fixture script and checks the launch. The real CLI half is unproven through `humatad2`. The REALFLOW manual run started the real CLI this way and it worked (no test pins it). **Needs the owner's login** (or the loopback model).

3. **I do:** type `/model` in the agent tab and press Return. In the picker I choose Claude Haiku 5.5, then the lowest reasoning level, then the smallest context, and I pick "this session only".

   **I see:** Copilot's own picker, in the terminal. After the last choice it prints the new model name.

   **State and events:** Humata only types these keys into the agent's terminal. It has no model picker for agent CLIs. The keys go through the same path as `agents/02-typing-into-sessions.md`.

   **Status:** Works (real CLI, manual REALFLOW run) for choosing Claude Haiku 5.5, low reasoning, 100K context, "for this session"; no test pins it. Fixed (B12, D754): `humata session type` presses Enter by default, so a typed line needs no separate Return. Likely bug (seen live): Copilot wrote `recentModelIds` to `~/.copilot/config.json` despite "session only". **Needs the owner's login.** In the app window this is **Needs a free screen**; headless, type the keys with `humata`'s terminal input command.

4. **I do:** look for what Humata knows about the model.

   **I see:** nothing. No label names the model, and nothing warns that the choice lives only in the CLI.

   **State and events:** `AgentLaunch` (`daemon/store/src/records.rs:156`) holds no model, reasoning or context setting. Whether a resumed, moved or forked session keeps them is the CLI's own session state, and is unproven.

   **Status:** Not built, no milestone. This is a gap only if the owner wants Humata to show or pin the model. I searched `daemon/store/src/records.rs` and the Copilot adapter for a model field and found none.

5. **I do:** type `In the api/ folder, add the line "Tested on this Mac." to the end of README.md.` and press Return. Humata does not tell the agent where the repo is; the prompt names the clone's folder (`api/`).

   **I see:** Copilot works. It plans a small edit and wants to write the file.

   **State and events:** the agent hooks report `working` (`userPromptSubmitted`, `postToolUse`). The session shows Working.

   **Status:** Works (real daemon) for the state reports. `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` drives the daemon with hook calls from a shell. Works (real CLI, manual REALFLOW run) for the real task; no test pins it. **Needs the owner's login.**

6. **I do:** open the Inbox.

   **I see:** a card: the agent wants to run a command or write `README.md`. The whole command is shown. The card offers Allow and Deny, and no Always.

   **State and events:** Copilot's `permissionRequest` hook runs `humata hook copilot permission`. The daemon raises one Inbox request of kind session, and holds the hook open with keepalives. The session shows Needs input.

   **Status:** Works (real daemon) for the daemon half. `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` sends Copilot's real payload shape (`COPILOT_SHELL`, `permissionRequest`) and checks the card. In the manual REALFLOW run three asks reached the Inbox and I approved them with `humata inbox approve` (Works (real CLI, manual REALFLOW run); no test pins it). Always on a permission card is Not built (`agents/03`, step 7).

7. **I do:** press Allow.

   **I see:** the hook prints `{"behavior":"allow"}`, not wrapped in `hookSpecificOutput`, and exits 0. Copilot runs the edit and prints that it is done.

   **State and events:** the card leaves the Inbox as approved. `README.md` has the new line.

   **Status:** Works (real daemon) for the answer's shape: `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape`. The REALFLOW run did the edit after approval (no test pins it). **Needs the owner's login.**

8. **I do:** wait for Copilot to finish its turn.

   **I see:** the session shows Idle. The Inbox is empty.

   **State and events:** the plugin's `agentStop` hook runs `humata turn idle`, so the turn end is learned from hooks (`TurnEnd::Hooks`). Without hooks the kind falls back to output gone quiet (`TurnEnd::Output`, SE-22) and shows "probably idle".

   **Status:** Works (real daemon) for the daemon half: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` and `::an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again`. Idle detection worked live in the REALFLOW run (no test pins it).

9. **I do:** open the session's Changes panel (⌘⇧D, or the Changes tab).

   **I see:** `README.md` listed as changed, with one added line in its diff.

   **State and events:** the daemon diffs the session's clone against its base. The app shows the files and hunks.

   **Status:** Works (real daemon) for the diff: `daemon/humatad/tests/review.rs::a_diff_lists_the_changed_files_and_gives_each_ones_hunks`. The app side runs headless against a real `humatad2` with a fake agent in `desktop/tests/live_flows/review.rs::a_three_repo_diff_is_reviewed_commented_on_and_marked_reviewed`. `humata diff` worked live in the REALFLOW run (Works (real CLI, manual REALFLOW run)). The panel drawn in the real window is **Needs a free screen**.

10. **I do:** stop this Mac's sessions (Machine page › Stop sessions, confirm).

    **I see:** the agent tab ends. Nothing runs. The Inbox request, if any, is withdrawn as stopped.

    **State and events:** the daemon hangs up the agent, writes the stop record and no restart report.

    **Status:** Works (real daemon). `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report`, with a fake agent. The app path is `desktop/tests/live_flows/stop_and_start.rs::this_macs_page_stops_its_sessions_only_when_confirmed_and_starts_them_again`.

11. **I do:** look at the session's row while it is stopped.

    **I see (expected):** Stopped. **I see (REALFLOW):** Idle.

    **State and events:** the session's agent state is not mapped to Stopped for a custom kind, so a stopped Copilot looks like one that finished its turn.

    **Status:** Likely bug (seen live, B6). The REALFLOW tester saw Idle. Confirm by stopping a session of the custom kind and reading `humata session list` and the row.

12. **I do:** start the sessions again.

    **I see:** the agent tab returns, Copilot resumes the same conversation and shows the earlier turn and the README edit. The conversation id is unchanged.

    **State and events:** the daemon runs `resume_argv` with the pinned conversation id, after checking that `conversation_file` exists (PER-14). The restart report names the stop.

    **Status:** Works (real daemon) for the daemon half with a fake agent: `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop`. Likely bug (seen live, B5) for the real Copilot: resume looked for the conversation under `--root` as home instead of `$HOME` or `COPILOT_HOME`. Likely bug (seen live, B7): a resume after a stop is treated as an interruption and kills the correctly resumed agent. Likely bug (seen live, B10): attach is refused while the agent is resuming. Confirm by checking where `conversation_file` points for the custom kind, and whether `COPILOT_HOME` reached the resume.

13. **I do:** read the conversation with `humata session read <session>`.

    **I see (expected):** the turns of the conversation. **I see (REALFLOW):** nothing.

    **State and events:** `ReadConversation` answers `not_built` in the daemon (`daemon/humatad/src/session_service.rs:2681`). The CLI's `conversation` command calls it (`cli/src/commands/sessions.rs:52`).

    **Status:** Likely bug (seen live, B4/B8) as the tester saw it; reading the code it is Not built: the RPC is not implemented for any agent kind. Turns are listed by other means (`daemon/humatad/tests/continue_in.rs::a_copilot_session_lists_its_turns`, Works (real daemon)).

14. **I do:** type `/model` in the resumed session and read the current model.

    **I see:** unknown. It may still be Claude Haiku 5.5, or it may be Copilot's default.

    **State and events:** the model is the CLI's session setting. Humata does not restore it. Write down the answer; that is what this step is for.

    **Status:** Code, no test. Unproven. **Needs the owner's login.**

15. **I do:** fork the session on this Mac, code and context, before the first turn's reply.

    **I see:** a new session with its own branch and clone, and its own conversation id. The parent is unchanged.

    **State and events:** Humata copies the Copilot session state through its adapter (new id, new folder, rewritten metadata). It does not script the CLI (`daemon/sessions/src/fork.rs`).

    **Status:** Works (real daemon) for the copy: `daemon/humatad/tests/continue_in.rs::a_copilot_fork_before_turn_2_resumes_with_turn_1_only` and `daemon/humatad/tests/fork.rs::code_and_context_copies_the_work_and_the_conversation`. Likely bug (seen live, B8): the real fork said "can't read this Copilot conversation" (ROAM's real-CLI captures and fixes are pending). The terminal experiment proves it with a loopback model: `//apps/humata2/terminal/provider:copilot_fork_test`.

## Branches

### Copilot is not a built-in kind

**When:** I start `humatad2` without an `--agents` file.

**I see:** no `copilot` kind in Settings › Agents. I cannot start one.

**Expected:** a built-in kind, with the hook plugin and the version check wired.

**Status:** Not built (ROAM batch 3, C4, X2). `daemon/sessions/src/agents.rs` `Agents::builtin` lists Claude and OpenCode only.

### I deny a permission ask with a note

**When:** step 6, and I press Deny with the note "do not touch the README".

**I see:** the hook prints Copilot's deny shape with the note, and Copilot tells the model.

**Status:** Fakes only for the shape: `daemon/agent_hooks/tests/payloads.rs::copilot_reads_a_flat_behavior`. The deny and note path with a held hook is Works (real daemon) for Claude: `daemon/humatad/tests/hooks.rs::an_inbox_deny_reaches_the_hook_with_its_note`. No test denies a Copilot hold.

### I answer in Copilot's own prompt

**When:** the card is pending and I answer in the terminal instead.

**I see:** the card is withdrawn and cannot be answered.

**Status:** Works (real daemon). `daemon/humatad/tests/hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered`.

### The CLI is older than Humata knows

**When:** the installed Copilot is older than 1.0.112-0.

**I see:** a fork or move is refused up front, naming the version.

**Status:** Works (real daemon). `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front`.

### The resume fails and I start fresh

**When:** step 12 cannot find the conversation (the REALFLOW bug).

**I see:** the session offers Start fresh with a summary. I accept.

**Expected:** a new conversation that hears a summary of the old one.

**Status:** Works (real daemon). `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary`.

## Expected outcomes

- [ ] A custom `copilot` kind starts a Copilot session in a channel on this Mac, and its launch record has the conversation id and file.
- [ ] `/model` typed as real keys sets Claude Haiku 5.5, low reasoning and the smallest context for this session only, and the owner's Copilot home is untouched.
- [ ] A permission ask reaches the Inbox as one card, and Allow reaches Copilot in its flat `behavior` shape.
- [ ] The turn end shows Idle from `agentStop`, or probably Idle from quiet output when hooks are off.
- [ ] The README edit shows in the Changes panel as one added line.
- [ ] After stop and start the same conversation resumes, and a stopped session shows Stopped.
- [ ] A fork on this Mac copies the work and the conversation, and the parent is unchanged.
- [ ] Whether the model survives the resume and the fork is written down.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Daemon with a custom Copilot kind | Fakes only | `daemon/sessions/src/agents.rs::codex_and_copilot_hooks_fill_their_placeholders` |
| 2. Start the session | Works (real daemon); Works (real CLI, manual REALFLOW run) | `daemon/humatad/tests/continue_in.rs::a_copilot_session_keeps_the_version_its_cli_said_at_launch` |
| 3. `/model` keystrokes | Works (real CLI, manual REALFLOW run); Fixed (B12, D754): `humata session type` presses Enter by default | `agents/02-typing-into-sessions.md` |
| 4. Humata records the model | Not built (no milestone) | `daemon/store/src/records.rs:156` |
| 5. Tiny task, Working | Works (real daemon), Works (real CLI, manual REALFLOW run) | `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` |
| 6. Inbox card | Works (real daemon), Works (real CLI, manual REALFLOW run) | `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` |
| 7. Allow reaches Copilot | Works (real daemon), Works (real CLI, manual REALFLOW run) | `daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` |
| 8. Turn end | Works (real daemon), Works (real CLI, manual REALFLOW run) | `daemon/humatad/tests/turns.rs::an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again` |
| 9. Changes panel | Works (real daemon); window needs a free screen | `daemon/humatad/tests/review.rs::a_diff_lists_the_changed_files_and_gives_each_ones_hunks` |
| 10. Stop | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report` |
| 11. Stopped shows Stopped | Likely bug (seen live, B6) | REALFLOW run; row shows Idle |
| 12. Resume the same conversation | Works (real daemon) with a fake agent; Likely bug (seen live, B5, B7, B10) with real Copilot | `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop` |
| 13. Read the conversation | Likely bug (seen live, B4/B8) | `daemon/humatad/src/session_service.rs:2681` |
| 14. Model after resume | Code, no test | unproven |
| 15. Fork on this Mac | Works (real daemon) with fixtures; Likely bug (seen live, B8) with real Copilot | `daemon/humatad/tests/continue_in.rs::a_copilot_fork_before_turn_2_resumes_with_turn_1_only` |
| Branch: built-in kind | Not built (ROAM batch 3, C4, X2) | `daemon/sessions/src/agents.rs` `Agents::builtin` |
| Branch: Deny with a note | Fakes only | `daemon/agent_hooks/tests/payloads.rs::copilot_reads_a_flat_behavior` |
| Branch: answer in Copilot's prompt | Works (real daemon) | `daemon/humatad/tests/hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered` |
| Branch: CLI too old | Works (real daemon) | `daemon/humatad/tests/continue_in.rs::a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front` |
| Branch: start fresh | Works (real daemon) | `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary` |

## Gaps and likely bugs

- Not built: built-in `copilot` kind (ROAM batch 3, C4, X2).
- Not built: Humata records no model, reasoning or context for an agent session (no milestone). Whether a resume, fork or move keeps the CLI's choice is unproven.
- Not built: `ReadConversation` is `not_built` for every kind (`daemon/humatad/src/session_service.rs:2681`).
- Not built: Always on a permission card (`agents/03`, no milestone).
- Likely bug (seen live, B6): a stopped session shows Idle, not Stopped.
- Likely bug (seen live, B5): resume looks for the conversation under `--root` as home.
- Likely bug (seen live, B7): resume after a stop is treated as an interruption and kills the resumed agent.
- Likely bug (seen live, B10): attach is refused while the agent resumes.
- Likely bug (seen live, B4/B8): `humata session read` is empty for Copilot (the code says not built).
- Likely bug (seen live, B8): fork says "can't read this Copilot conversation".
- Fixed (B12, D754): `humata session type` presses Enter by default; `--no-submit` leaves the text unsent.
- Likely bug (seen live): Copilot writes `recentModelIds` to `~/.copilot/config.json` despite "session only".
- Manual only: the real-CLI steps ran once by hand (REALFLOW); no test pins them.
- Unproven: the app window path for steps 3 and 9 (**Needs a free screen**).

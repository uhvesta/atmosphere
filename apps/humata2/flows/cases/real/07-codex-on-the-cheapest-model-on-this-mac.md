# A Codex session on GPT-6 Luna does a tiny task, asks through the Inbox, and resumes after a stop

Layer: real (real Codex CLI, real `humatad2`, this Mac). Platforms: macOS. The VM is not used here; `real/08` moves the same session there.

## Starting state

- This Mac runs a real `humatad2` in a temp state folder. It was started with an `--agents` JSON file that defines a custom kind `codex` (`conversation_format` `codex-rollout`, `turn_end` `hooks`, `-c hooks={codex_hooks}` in its start and resume arguments). This case uses a custom kind on purpose; the built-in `codex` kind runs through `humata codex` (D806).
- The channel `acme/api` has this Mac as a machine and one clone. `README.md` is clean.
- The Inbox is empty. Permission asks wait in the Inbox (the default, D199).
- The Codex CLI is the installed one, version 0.169.0, the version the terminal experiment used.
- No session exists yet. The app is open on the channel, or I use the `humata` CLI.

## Test setup

- Use a fresh temp folder for the run. Pass `CODEX_HOME` and `HOME` pointing into it to `humatad2`. The daemon hands `CODEX_HOME` to every Codex it starts and places conversations there (`daemon/humatad/src/serve.rs:456-466`). Never touch the owner's real `~/.codex` or `~/.humata2`.
- Start `humatad2 serve --agents <file>` with that state folder and root. Bazel builds it (no cargo).
- **Needs the owner's login.** The real model needs a Codex `login`. Run it with the temp `CODEX_HOME` set, so the token lands only there. A plan without GPT-6 Luna in `/model` stops step 3; say so.
- **Alternative with no login:** the loopback model server in `terminal/provider/tests/codex_resume.py`. Run `//apps/humata2/terminal/provider:codex_resume_test` and `:codex_fork_test` (manual, macOS only, installed CLI), and `//apps/humata2/terminal/host:agent_resume_codex_test`. They prove Codex resume and fork in the terminal component with a deterministic model. They do not go through `humatad2` and do not prove the real model.
- **Needs a free screen** for every step that opens the app window. The headless equivalent is the `humata` CLI, or a `desktop/tests/live_flows/*` style test against a real `humatad2`.
- Cleanup: stop the daemon, delete the temp folder, and confirm the owner's own Codex home is unchanged.

## Steps

1. **I do:** start `humatad2` with the `--agents` file and open channel `acme/api`.

   **I see:** Settings › Agents lists `codex` beside Claude Code and OpenCode. Because the kind carries `{codex_hooks}` and no hook has been heard, Settings › Agents shows Codex's trust step.

   **State and events:** the kind is read from the JSON. `{codex_hooks}` becomes the TOML hook value (`daemon/agent_hooks/src/lib.rs:299`, `codex_hooks`: `UserPromptSubmit`, `PostToolUse`, `Stop`, `Interrupt`, `PermissionRequest`). The daemon remembers the hash of that value (`daemon/humatad/src/inbox/permission/codex_trust.rs`, D202).

   **Status:** Works (real daemon). `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard` shows the step with a fixture agent, and `::a_kind_without_codexs_hooks_never_shows_the_step` shows it hides. The fill of the placeholder is Fakes only: `daemon/sessions/src/agents.rs::codex_and_copilot_hooks_fill_their_placeholders`. No test starts the daemon with the real Codex.

2. **I do:** start a session with the `codex` kind in `acme/api` on this Mac, titled `Tiny README edit`.

   **I see:** a session with an agent tab. Codex starts. First it shows "Hooks need review".

   **State and events:** the session record gets an `AgentLaunch` with a conversation id Humata chose, `argv`, `resume_argv` and a conversation file path with the id in it. Real Codex does not use that id. It writes its own dated rollout, `sessions/<year>/<month>/<day>/rollout-<stamp>-<its id>.jsonl`, under `CODEX_HOME`.

   **Status:** Works (real daemon) for the daemon half with a fixture script: `daemon/humatad/tests/continue_in.rs::a_codex_session_lists_its_turns` starts a Codex kind and reads a fixture rollout that a test puts at the pinned path. Works (real CLI, manual REALFLOW run) for the real CLI starting; no test pins it. **Needs the owner's login.** Likely bug (seen live, B9): the pinned path is not where the real CLI writes. See step 12.

3. **I do:** at "Hooks need review" I choose Continue without trusting.

   **I see:** the Codex prompt shows. Codex does not run Humata's hooks yet.

   **State and events:** Humata does not answer this prompt. Its trust step is only a Settings flag (`codex_hooks_need_trust`) that stays on until some hook of the remembered hash reports a turn (`codex_trust.rs`). While the hooks are not trusted, no `working`, `idle` or permission hook reaches the daemon, so the session cannot show Working, Needs input or Idle from hooks, and an ask cannot reach the Inbox.

   **Status:** Works (real CLI, manual REALFLOW run) for choosing Continue without trusting; no test pins it. Works (real daemon) for the flag clearing on a heard turn: `codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard` (the test reports a turn with `ReportTurn`, not from Codex). What the owner sees if they trust the hooks in `/hooks` instead is Code, no test. Humata has no step that opens `/hooks` for the owner.

4. **I do:** type `/model` in the agent tab and press Return. I choose GPT-6 Luna, then the lowest reasoning level, then `s` for this session only.

   **I see:** Codex's own picker, in the terminal. After the last choice it prints the new model.

   **State and events:** Humata only types these keys into the agent's terminal. It has no model picker for agent CLIs. The keys go through the same path as `agents/02-typing-into-sessions.md`.

   **Status:** Works (real CLI, manual REALFLOW run) for choosing GPT-6 Luna, low reasoning, `s`; no test pins it. Fixed (B12, D754): `humata session type` presses Enter by default, so a typed line needs no separate Return. **Needs the owner's login.** In the app window this is **Needs a free screen**; headless, type the keys with the `humata` terminal input command.

5. **I do:** look for what Humata knows about the model.

   **I see:** nothing. No label names the model, and nothing warns that the choice lives only in the CLI.

   **State and events:** `AgentLaunch` (`daemon/store/src/records.rs:156`) holds no model or reasoning setting. Whether a resumed, moved or forked session keeps them is the CLI's own state, and is unproven.

   **Status:** Not built, no milestone. It is a gap only if the owner wants Humata to show or pin the model. I searched `daemon/store/src/records.rs` and the Codex adapter (`daemon/conversation/src/codex.rs`) for a model field and found none.

6. **I do:** type `In the api/ folder, add the line "Tested on this Mac." to the end of README.md.` and press Return. Humata does not tell the agent where the repo is; the prompt names the clone's folder (`api/`).

   **I see:** Codex works. It wants to write the file.

   **State and events:** with trusted hooks, `UserPromptSubmit` and `PostToolUse` run `humata turn working` and the session shows Working. With untrusted hooks (step 3) the row does not move.

   **Status:** Works (real daemon) for the state reports from a shell: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input`. Works (real CLI, manual REALFLOW run) for the task running; no test pins it. **Needs the owner's login.**

7. **I do:** open the Inbox.

   **I see (expected):** a card that shows the whole command or file write, with Allow and Deny.

   **State and events:** Codex's `PermissionRequest` hook runs `humata hook codex permission`. The daemon raises one Inbox request of kind session and holds the hook open. The session shows Needs input.

   **Status:** Fakes only for the shape of Codex's payload and answer: `daemon/agent_hooks/tests/payloads.rs::claude_and_codex_read_allow_and_deny_from_hook_specific_output` and `::codex_never_gets_fields_it_fails_closed_on`. The held hook through a real daemon is tested for Claude and Copilot only (`daemon/humatad/tests/hooks.rs::each_agent_gets_the_answer_in_its_own_shape` uses Copilot); no test holds a Codex hook. The REALFLOW notes say the Codex task ran; they do not say an ask reached the Inbox, so this is Code, no test and unseen live. **Needs the owner's login** and trusted hooks.

8. **I do:** press Allow, then wait for the turn to end.

   **I see:** Codex runs the edit and prints that it is done. The session shows Idle. The Inbox is empty.

   **State and events:** the card leaves the Inbox as approved. The `Stop` hook runs `humata turn idle` and prints `{}` for Codex. Without trusted hooks the kind falls back to quiet output (`TurnEnd::Output`) and shows "probably idle".

   **Status:** Works (real daemon) for the turn end from a shell: `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` and `::an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again`. Allow reaching Codex is Code, no test (see step 7).

9. **I do:** open the session's Changes panel (⌘⇧D), or run `humata diff`.

   **I see:** `README.md` with one added line. (The REALFLOW prompt did not name the folder, so Codex wrote beside the clone and Changes was empty; naming `api/` in the prompt is the fix, not a Humata change.)

   **State and events:** the daemon diffs the session's clone against its base. The agent starts in the session folder; the prompt told it to work in `api/`, the clone.

   **Status:** Works (real daemon) for the diff: `daemon/humatad/tests/review.rs::a_diff_lists_the_changed_files_and_gives_each_ones_hunks`. The panel in the real window is **Needs a free screen**.

10. **I do:** stop this Mac's sessions (Machine page › Stop sessions, confirm).

    **I see:** the agent tab ends. Nothing runs.

    **State and events:** the daemon hangs up the agent, writes the stop record and no restart report.

    **Status:** Works (real daemon) with a fake agent: `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report`. The app path is `desktop/tests/live_flows/stop_and_start.rs::this_macs_page_stops_its_sessions_only_when_confirmed_and_starts_them_again`.

11. **I do:** look at the session's row while it is stopped.

    **I see (expected):** Stopped.

    **State and events:** the Copilot REALFLOW run showed Idle (B6). The Codex run did not record this row.

    **Status:** Likely bug (seen live for Copilot, B6); unchecked for Codex. Confirm with `humata session list`.

12. **I do:** start the sessions again.

    **I see (expected):** the agent tab returns and Codex resumes the same conversation with the README turn. **I see (REALFLOW):** resume failed.

    **State and events:** the daemon runs `resume_argv` with the conversation id it pinned, after checking that `conversation_file` exists (PER-14). Humata never learned the id Codex made (a dated rollout path), so the pinned file does not exist and `codex resume` is given an id Codex does not know.

    **Status:** Works (real daemon) for the daemon half with a fake agent: `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop`. Likely bug (seen live, B9) with real Codex. Confirm by comparing `AgentLaunch.conversation_file` with the rollout under `$CODEX_HOME/sessions/`. The terminal experiment resumes real Codex by its own id with a loopback model (`//apps/humata2/terminal/provider:codex_resume_test`), so the CLI itself resumes; the gap is in Humata learning the id.

13. **I do:** read the conversation with `humata session read <session>`.

    **I see (expected):** the turns. **I see:** nothing, or an error.

    **State and events:** `ReadConversation` answers `not_built` (`daemon/humatad/src/session_service.rs:2681`). Turns are listed by other means, from a rollout at the pinned path.

    **Status:** Not built for the read RPC (no milestone). Listing turns is Works (real daemon) with a fixture rollout: `continue_in.rs::a_codex_session_lists_its_turns`. With real Codex the rollout is not at the pinned path (B9), so this likely finds nothing; unconfirmed.

14. **I do:** fork the session on this Mac, code and context, before the first turn's reply.

    **I see (expected):** a new session with its own branch, clone and conversation id. The parent is unchanged.

    **State and events:** Humata copies the rollout through the Codex adapter (new id in `session_meta`, new folder, thread row rewritten). It does not script the CLI (`daemon/sessions/src/fork.rs`). The adapter's header says the format "waits on the layout fixtures (CV-7)" (`daemon/conversation/src/lib.rs:52`), and the Codex fixtures in `daemon/conversation/tests/codex.rs` are written as codex-cli 0.169.0 writes them.

    **Status:** Works (real daemon) with fixtures: `daemon/humatad/tests/continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only` and `daemon/humatad/tests/fork.rs::a_fork_of_a_codex_parent_is_handed_a_summary_read_from_its_rollout`. The real CLI half is untested live (no REALFLOW fork). With B9 the parent's real rollout may not be found, so the real fork is Likely bug (B9) until confirmed.

## Branches

### Codex is not a built-in kind

**When:** I start `humatad2` without an `--agents` file.

**I see:** no `codex` kind in Settings › Agents. I cannot start one.

**Expected:** a built-in kind with the hooks and version check wired.

**Status:** Not built (ROAM batch 3, C4, X2). `daemon/sessions/src/agents.rs` `Agents::builtin` lists Claude and OpenCode only.

### I trust the hooks in Codex's own `/hooks`

**When:** step 3, and I trust the hooks instead of continuing without.

**I see:** the next turn reports Working, and the Settings flag clears.

**Status:** Works (real daemon) for the flag clearing: `codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard`. Codex keying its trust on the hook's hash is the CLI's rule; a changed `{codex_hooks}` value (a new `humata` path) asks again. That re-ask is Code, no test.

### I deny a permission ask with a note

**When:** step 7, and I press Deny with the note "do not touch the README".

**I see:** the hook prints Codex's deny shape with the note.

**Status:** Fakes only for the shape: `payloads.rs::claude_and_codex_read_allow_and_deny_from_hook_specific_output`. A held Codex hook that is denied is Code, no test. The Claude path is Works (real daemon): `hooks.rs::an_inbox_deny_reaches_the_hook_with_its_note`.

### I answer in Codex's own prompt

**When:** the card is pending and I answer in the terminal instead.

**I see:** the card is withdrawn and cannot be answered.

**Status:** Works (real daemon) for a Claude hold: `hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered`. Not run for Codex.

### The resume fails and I start fresh

**When:** step 12 fails because the conversation file is missing.

**I do:** choose to start fresh.

**I see:** a new conversation on the same session, told a summary of the old one.

**Status:** Works (real daemon): `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary`. It uses a fake agent, so it does not show that a real Codex failure is the one that triggers it.

### Typing a line from the CLI

**When:** I send `/model` with `humata session type`.

**I see:** the text is typed into Codex's prompt and submitted; the reply says `submitted` true. With `--no-submit` it stays in the prompt until Enter is pressed.

**Status:** Fixed (B12, D754): `humata session type` presses Enter by default. Seen live before the fix (REALFLOW): the text sat in the prompt. Confirm live by typing a line and checking that Codex shows it as submitted.

## Expected outcomes

- [ ] The owner's own Codex home and `~/.humata2` are untouched.
- [ ] Settings shows Codex's trust step until a hook is heard, and Humata does not answer the prompt itself.
- [ ] Choosing Continue without trusting leaves the session working, with a turn end that falls back to quiet output.
- [ ] A permission ask reaches the Inbox as one card, and Allow reaches Codex in its own shape.
- [ ] The README edit shows in Changes as one added line, inside the clone.
- [ ] After stop and start the same conversation resumes, and a stopped session shows Stopped.
- [ ] A fork on this Mac copies the work and the conversation, and the parent is unchanged.
- [ ] Whether the model survives the resume and the fork is written down.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. Daemon with a Codex kind, trust step | Works (real daemon) | `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard` |
| 2. Start the session | Works (real daemon); Works (real CLI, manual REALFLOW run); Likely bug (seen live, B9) for the pinned path | `daemon/humatad/tests/continue_in.rs::a_codex_session_lists_its_turns` |
| 3. Continue without trusting | Works (real CLI, manual REALFLOW run) | no test pins it |
| 4. `/model` keystrokes | Works (real CLI, manual REALFLOW run); Fixed (B12, D754): `humata session type` presses Enter by default | `flows/cases/agents/02-typing-into-sessions.md` |
| 5. Humata records the model | Not built (no milestone) | `daemon/store/src/records.rs:156` |
| 6. Tiny task, Working | Works (real daemon) from a shell; Works (real CLI, manual REALFLOW run) | `daemon/humatad/tests/turns.rs::the_hooks_move_a_session_between_idle_working_and_needing_input` |
| 7. Inbox card for Codex | Fakes only (shape); Code, no test (held hook) | `daemon/agent_hooks/tests/payloads.rs::codex_never_gets_fields_it_fails_closed_on` |
| 8. Allow, turn end | Works (real daemon) for the turn end; Code, no test for Allow | `daemon/humatad/tests/turns.rs::an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again` |
| 9. Changes panel | Works (real daemon) for the diff; window Needs a free screen | `daemon/humatad/tests/review.rs::a_diff_lists_the_changed_files_and_gives_each_ones_hunks` |
| 10. Stop | Works (real daemon) | `daemon/humatad/tests/stop_sessions.rs::a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report` |
| 11. Stopped shows Stopped | Likely bug (seen live for Copilot, B6); unchecked for Codex | `flows/cases/real/05-copilot-on-the-cheapest-model-on-this-mac.md` |
| 12. Resume the same conversation | Works (real daemon) with a fake agent; Likely bug (seen live, B9) | `daemon/humatad/tests/stop_sessions.rs::start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop` |
| 13. Read the conversation | Not built (no milestone) | `daemon/humatad/src/session_service.rs:2681` |
| 14. Fork on this Mac | Works (real daemon) with fixtures; real CLI untested | `daemon/humatad/tests/continue_in.rs::a_codex_fork_before_turn_2_resumes_with_turn_1_only` |
| Branch: built-in kind | Not built (ROAM batch 3, C4, X2) | `daemon/sessions/src/agents.rs` `Agents::builtin` |
| Branch: trust in `/hooks` | Works (real daemon) for the flag | `daemon/humatad/tests/codex_trust.rs::codexs_hooks_need_trust_until_one_of_them_is_heard` |
| Branch: Deny with a note | Fakes only; Code, no test for a held Codex hook | `daemon/agent_hooks/tests/payloads.rs::claude_and_codex_read_allow_and_deny_from_hook_specific_output` |
| Branch: answer in Codex's prompt | Works (real daemon) for Claude only | `daemon/humatad/tests/hooks.rs::a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered` |
| Branch: start fresh | Works (real daemon) with a fake agent | `daemon/humatad/tests/start_fresh.rs::a_failed_resume_starts_fresh_on_a_new_conversation_and_hears_the_summary` |
| Branch: `humata session type` | Fixed (B12, D754): `humata session type` presses Enter by default | `cli/tests/sessions.rs::type_presses_enter_after_the_text_by_default` |

## Gaps and likely bugs

- Not built: built-in `codex` kind (ROAM batch 3, C4, X2).
- Not built: Humata records no model or reasoning for an agent session (no milestone). Whether a resume or fork keeps the CLI's choice is unproven.
- Not built: `ReadConversation` is `not_built` for every kind (`daemon/humatad/src/session_service.rs:2681`).
- Not built: any step in Humata that guides the owner through Codex's "Hooks need review" prompt; it only shows a Settings flag.
- Likely bug (seen live, B9): resume fails because Humata never learns Codex's real session id (the dated rollout path). Every Codex daemon test puts its fixture rollout at the path the daemon pinned (`continue_in.rs`, `fork_remote.rs::codex_rollout`), so none can see this.
- Fixed (B12, D754): `humata session type` presses Enter by default; `--no-submit` leaves the text unsent.
- Likely bug (seen live for Copilot, B6; unchecked for Codex): a stopped session shows Idle.
- Unproven: a Codex permission ask through the Inbox, and Allow or Deny reaching real Codex.
- Unproven: fork of a real Codex rollout. The adapter doc still says it "waits on the layout fixtures (CV-7)".
- Manual only: the real-CLI steps ran once by hand (REALFLOW); no test pins them.
- Unproven: the app window path for steps 4 and 9 (**Needs a free screen**).

# SCREENPARSE: agent prompts on screen are read right for every agent, tested against the real CLIs

- **Status:** open (owner, 2026-11-20). Finishes HOOKCLEAR (0.6.1, RESUME.md › Release plan, step 18).
- **Outcome sentence:** When Claude Code, Codex, Copilot or agy shows a question or permission prompt, it shows up in my Inbox, and it clears when the prompt is gone. This holds for prompts their real CLIs draw today, not only the ones we hand-wrote.

## Why
`daemon/svc_inbox/src/screen_ask.rs` reads each agent's terminal screen to find prompts. Its fixtures are hand-written screens. The owner suspects real CLIs draw more cases than we cover (2026-11-20).

## Plan
1. **Fix the HOOKCLEAR regressions first.** These come from the 12305aba review. The first three were fixed on 2026-11-20 (main):
   - `Working` no longer clears held asks. `clear_stale_hook_asks_on_lifecycle` and the unused `ScreenAsk::on_lifecycle` are gone.
   - An unsupported agent's screen hold ends when its dialog is gone, as it did before HOOKCLEAR.
   - Signal B clears a hook ask only after its dialog was seen on screen (`hook_prompt_seen`). A held ask that is never drawn, with only the agent's spinner moving, stays. Test: `held_hook_ask_never_drawn_stays_while_output_moves`.

   The `std::thread::sleep` in the test helper is still open. The original items:
   - turns.rs:654-659: `Hook::Working` clears every held hook ask, which reverses D798. Drop the clear and keep `withdraw_notice`.
   - screen_ask.rs:509: for unsupported agents the hold is never ended.
   - Signal B can auto-withdraw a card while the agent's spinner is still showing.
   - screen_ask.rs:718: `std::thread::sleep` inside the test helper. Replace it with an event or channel wait.
2. **Record a live corpus.** Drive each harness with a cheap model, in an isolated home, never the real `~/.claude`, `~/.codex`, `~/.copilot` or `~/.gemini`. The approval to copy logins into isolated homes and test VMs (2026-11-19) covers this.
   - **Claude Code:** Haiku 5.5 (`claude-haiku-5-5`).
   - **Codex:** its smallest current model.
   - **Copilot:** its cheapest model.
   - **agy:** open question: which model? Ask Fable. If none is cheap enough, use agy's default model on a few short scripted turns only.
   - **Scripts:** short scripted tasks that trigger every prompt kind:
     - a tool permission (bash, edit, write, web fetch);
     - plan approval;
     - a multiple-choice question;
     - a free-text question;
     - trust-this-folder;
     - login expired;
     - a rate limit;
     - an MCP tool permission;
     - a prompt cancelled with Escape;
     - a prompt answered from a different terminal;
     - output resuming after an answer.
   - Run each script at several terminal widths: 80, 120 and 200 columns, plus a narrow 60.
   - **Capture:** save the holder's screen snapshots (text grid + cursor) at each step, with a label for what was on screen. They go into `daemon/svc_inbox/tests/screens/<agent>/<cli-version>/`.
3. **Replay in unit tests.** Write `svc_inbox` tests that feed the recorded screens to `screen_ask` and check:
   - a prompt was found or not;
   - its kind and its choices;
   - that it cleared when the prompt went away.

   These are deterministic and run in the fast tier, with no model calls.
4. **Live runs stay in the slow tier.**
   - A `screenparse_live` target re-records only when it is asked to, or before a minor release. It needs the agent logins, and it is skipped (reported as such, not failed) when a login is missing.
   - Newer CLI versions get a new folder. A diff against the previous version shows what changed.
5. **Fix the parser** for every case the corpus shows. Each fix gets a replayed screen that failed before it.

## Exit
- The HOOKCLEAR regressions are fixed and their tests pass.
- There is a corpus for all four agents (or for three, with the agy model question logged in DECISIONS).
- The replay tests are green in the fast tier.

## Lane

**Status 2026-11-20 (resumed after 0.9.0; target 0.10.0): batch 1 written, waiting on main's build.**

- **Item 1 (no thread sleep in screen_ask tests):** done in c1d3eb09.
- **agy:** model `gemini-3.8-flash-low` (Fable). Not recorded: its sign-in is a keychain item, and a scratch HOME starts signed out. It will be recorded only with `GEMINI_API_KEY` in the environment (D900; main confirms the ID).
- **Batch 1 (recorder + corpus runner), files:**
  - `daemon/svc_inbox/tests/screenparse/{lib,script,snapshot,record,record_main}.rs`: crate `humata2_screenparse` and the binary `screenparse_record`. The binary runs a CLI under the real holder, feeds the daemon's `vt_authority` engine and follows a step script (wait, wait-gone, quiet, type, keys, resize, snap). Each snap writes `<script>.w<cols>.<NN>-<label>.txt` with a header (agent, cli, script, label, expect, cols, rows), then `---`, then the screen text, at the current width and at each `--widths` value by resizing the holder.
  - `daemon/svc_inbox/tests/screenparse/scripts/*.script`: exploration scripts for bash, write, edit, fetch, question (choice + free text), plan, mcp and login. Each one opens with the trust screen. Snaps are labelled `unknown` and are labelled by hand after review.
  - `daemon/svc_inbox/tests/screenparse/mcp_echo.py`: a stdio MCP server with one echo tool, for the MCP-permission prompt.
  - `daemon/svc_inbox/tests/screenparse/record_corpus.sh`: sets up an isolated home per agent and run, then runs the recorder.
    - Claude: `CLAUDE_CONFIG_DIR` with a trimmed `.claude.json` and empty settings (no hooks), plus keychain login via an empty `CLAUDE_SECURESTORAGE_CONFIG_DIR`.
    - Codex: `CODEX_HOME` with a copy of auth.json.
    - Copilot: `COPILOT_HOME` with a trimmed config.json; an optional `SCREENPARSE_COPILOT_TOKEN` is passed through the environment only.
    - Afterwards it checks that the real config folders did not change.
  - `daemon/svc_inbox/BUILD.bazel`: new targets `:screenparse`, `:screenparse_record` and `:screenparse_unit_test`.
  - `daemon/vt_authority/BUILD.bazel`: visibility widened to `//apps/humata2/daemon/svc_inbox:__pkg__`.
- **Workers:** the recorder (Sonnet) is done and reviewed. The `unknown` expect was allowed in snaps by the manager.
- **Next:**
  1. Main builds the batch 1 targets.
  2. Run the recorder under a clean env: `env -i HOME=$HOME USER=$USER PATH=$PATH TMPDIR=$TMPDIR record_corpus.sh bazel-bin/apps/humata2/daemon/svc_inbox/screenparse_record <scratch>`.
  3. Review the screens and set their `expect`s.
  4. Write `tests/screen_replay.rs` (fast tier), `prompt_on` (kind + choices) in screen_question.rs, and `screenparse_live` (slow tier, skips when no login).

**2026-11-20 recorder stop.** A trial Claude `bash` run worked, but it showed two recorder bugs, both now fixed in source and not yet rebuilt:
- Steps ran after the program had exited.
- Claude Code's trust list starts on "No, exit".

The codex+copilot `probe` run was stopped by the before/after check: `~/.codex/tmp/arg0-macos/codex-arg00PsbI3/.lock` (a new dir) was written in the same second the run started, even with `CODEX_HOME` set to the scratch copy. It could also be the running Codex app-server. Waiting for main's call. Codex 0.169 also rejects `--ask-for-approval untrusted`; it accepts only on-request or never.

**2026-11-20 cause found.** The runner's own `<agent> --version` call ran under the real HOME and made the `~/.codex/tmp/arg0-macos/codex-arg0*` folders (00PsbI3, wT0FaD). The app-server didn't make them: they were born a second before the run's own copies, and only its two 11-19 folders are held open. Both stay untouched. Version calls now run in `$SCRATCH/version-home`, and codex runs set `HOME` and `CODEX_HOME` to scratch. The re-probe of codex and copilot passed the check with no exclusions.

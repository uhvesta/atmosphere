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
**PAUSED 2026-11-20 (main: full-suite 0.9.0 release first; this lane now targets 0.10.0).** Stopped during planning. No workers started, no corpus recorded, no isolated homes made, no repo files changed except this Lane section.

Findings so far, so a fresh manager can resume:
- **Plan item 1 (screen_ask.rs test sleep): already done** in c1d3eb09. `settle_tasks` yields until no debounce timer is due (`clear_in_flight`), and screen_ask.rs no longer contains `std::thread::sleep`.
- **Parser today:** `daemon/runtime/src/screen_question.rs` has `asked_on` (FolderTrust/Offer) and `has_active_prompt` (bool, phrase list + `❯ 1.`/`2.` + y/n suffixes). It returns no kind and no choices for permission, plan or choice prompts. Add a `prompt_on(screen) -> Option<ScreenPrompt { kind, choices }>` only after the corpus shows the real screens.
- **What the parser sees:** `humatad/src/holders.rs` `Link::screen_text` / `screen_at_if_changed`, which is `humata2_vt_authority::Authority::screen_text()` fed with the holder's output bytes. The recorder must use the same path: `humata2_holder::spawn(HolderSpec{cols,rows,env,..})`, then `attach`, then feed `Event` output to `Authority`, then write `Step` replies back via `Control::input`, with `Control::resize` for widths. `vt_authority` is visible only to `//apps/humata2/daemon/humatad:__pkg__`, so main must widen its visibility to the recorder's package (seam line).
- **Installed CLIs (2026-11-20):** claude 2.1.329, codex-cli 0.169.0, copilot 1.0.115-1, agy 1.5.13.
- **Isolation recipes:**
  - Claude: `CLAUDE_CONFIG_DIR=<copy>` with `CLAUDE_SECURESTORAGE_CONFIG_DIR=""`, which uses the keychain login (see `daemon/contain/src/login.rs`; 2.1.329 is not yet in its verified list).
  - Codex: `CODEX_HOME=<dir>` with a copy of auth.json.
  - Copilot: `COPILOT_HOME=<dir>`, plus `COPILOT_GITHUB_TOKEN` passed in the environment only.
  - agy: state is in ~/.gemini/antigravity-cli. Its home override is still unknown.
- **agy model:** `agy models` lists `gemini-3.8-flash-low`, the likely cheap pick. Still to confirm with Fable and log in DECISIONS.
- **Planned design (not started):**
  - A Rust recorder binary, `screenparse_record` (testonly, `bazel run`, writes under BUILD_WORKSPACE_DIRECTORY). It reads a step script (`wait <text>`, `quiet <ms>`, `type <text>`, `keys <enter|esc|up|down|tab|shift-tab|ctrl-c|…>`, `snap <label> <none|prompt:<kind>>`). Each snap is captured at 60/80/120/200 columns by resizing the holder, to save model calls (decision to log).
  - Snapshot files go in `tests/screens/<agent>/<version>/<script>.w<cols>.<n>-<label>.txt`, with a header (agent, cli, cols, rows, expect) above the screen text.
  - `tests/screen_replay.rs` is the fast tier and replays every snapshot against the parser.
  - `screenparse_live` is a slow-tier test that skips when a login is missing (writes `<skipped>` to XML_OUTPUT_FILE and logs it) and goes in slow.txt.
- **Claims to add on resume:** `daemon/svc_inbox/tests/screenparse/**` (recorder, live test, scripts).

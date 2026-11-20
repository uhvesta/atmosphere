# SCREENPARSE: agent prompts on screen are read right for every agent, tested against the real CLIs

- **Status:** open (owner, 2026-11-20). Finishes HOOKCLEAR (0.6.1, RESUME.md › Release plan, step 18).
- **Outcome sentence:** When Claude Code, Codex, Copilot or agy shows a question or permission prompt, it shows up in my Inbox, and it clears when the prompt is gone. This holds for prompts their real CLIs draw today, not only the ones we hand-wrote.

## Why
`daemon/svc_inbox/src/screen_ask.rs` reads each agent's terminal screen to find prompts. Its fixtures are hand-written screens. The owner suspects real CLIs draw more cases than we cover (2026-11-20).

## Plan
1. **Fix the HOOKCLEAR regressions first.** These come from the 12305aba review:
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
(empty: the manager fills this in)

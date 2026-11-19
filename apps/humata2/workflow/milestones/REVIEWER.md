# REVIEWER: I open fresh review terminals beside any diff for a second pair of eyes

- **Status:** open (owner, 2026-11-19: "add a button … 'Open Review Session' which makes it so that in the diff UI it opens a terminal I can open and close with a model. This is intentionally separate because when I review a PR I might want a fresh pair of eyes")
- **Depends on:** UXSIMPLE (its verdict on REVIEWER) and GOLDENFIX (the Diff panel header and footer it adds to). Can open in parallel with STACKVIEW; they share `ui/changes`, so claims must not overlap.
- **Unblocks:** nothing.
- **Lane:** daemon (session start) + features/review + ui/changes + desktop goldens

## Outcome
From any Diff panel, whether it shows my own session's change, a PR review or a
stack, I press **Open review terminal**. A fresh terminal
opens docked in the Diff panel. That agent did not write the change: it starts
with no history and gets only the diff's context. I can hide and show its
terminal without ending it, and I can end it explicitly. The diff stays the
main view. The reviewer can drive it with `review_focus` like the review chat
does.

## Why it is separate
- The working session's agent wrote the change, so it is anchored on its own
  reasoning.
- A PR review session's chat is set up to help me look, not to judge (D59/D791).
- This reviewer is for a fresh judgment. It runs the review skill (D791) only when I ask it to.

## Behaviour (proposed defaults; Fable rules on the open points)
- **Button:** "Open review terminal" goes in the Diff panel's header beside Show the
  terminal, or in the full-screen title bar. Once a reviewer exists, the button
  reads "Show review terminals" / "Hide review terminals". A palette entry and a shortcut do the same thing.
- **No model picker (owner, 2026-11-19):** the button opens a plain terminal. Its ⌄ menu
  can start an installed agent TUI (claude, copilot, codex) instead. The owner picks the model
  inside the TUI; Humata never picks one.
- **Context given to the reviewer:**
  - the diff spec that is showing (base/head per repo, or the stack's layers);
  - the repo checkouts, read-only by default;
  - the PR's URL, title and author when there is one.
  - It gets no conversation history from the working session.
- **Placement:** a terminal docked in the Diff panel, at the bottom or the side, resizable. It
  stays when I switch files or layers.
- **Lifetime:** hiding it never ends it (terminals keep running). It ends only
  when I press End. It is listed as its own session under the working session or
  PR (like a fork), so it is accountable and survives restarts and moves.
- **Findings flow:** the reviewer can leave draft comments on lines, which show as
  "Reviewer" drafts that I can send, edit or drop. It never posts to GitHub itself (PR-8, PR-12).
- **Agent access switches** apply as for any agent session.

## Owner's clarification (2026-11-19)
- A reviewer is a **fresh review terminal**: there can be several at once, each a tab in which I run whatever agent TUI I want, beside the diff.
- **Render first.** No code until the owner has seen and approved the mockups in `mockups/reviewer/`:
  1. `1-open-review-terminal.png`: "Open review terminal ⌄" opens a terminal; the menu offers Terminal (⌥⌘T) or an agent TUI. There is no model list.
  2. `2-two-review-terminals.png`: two review terminal tabs (`claude · review`, `zsh · review`) docked on the right of the full-screen diff, with an agent's draft comment offering Edit / Drop / Send as mine.
  3. `3-review-terminals-hidden.png`: the review terminals are hidden but still running, and the button shows the count and a busy dot.

## Open points for Fable 5.2
1. Is the reviewer read-only, or may it run tests or builds in a scratch copy?
2. Where does it dock: the bottom of the Diff panel or a split beside it? Which design frames does that need?
4. Do reviewer comments go into the same draft queue as mine, or a separate one?

## Exit
- Native and design frames exist for: the button and its menu, the review terminals
  docked, the reviewer hidden, and reviewer drafts. Fable has judged them.
- A live flow passes with a fakes daemon: open, hide, show, end, and a restart with the reviewer
  still listed.
- A real-agent run on the cheapest Copilot model reviews a small diff and
  leaves one draft comment.
- Each pick is logged in DECISIONS.md.

## Pointers
- Review chat session start: `daemon/humatad` review/PR session code, D59, D631, D751, D791.
- Diff panel header: `ui/core/src/chrome.rs` `panel_header`, `ui/changes/src/panel_changes/`.
- Frames to start from: 41, 68, 68c, 84–86.

## Log
- 2026-11-19 (DIFFUI): built as UXSIMPLE ruled (D849): New agent tab ⌄ in the full-screen Diff header. Its menu lists Terminal and the offered agents. A pick opens a tab in the session's own strip, leaves full screen and is marked "last used".
  - Tests: native `diff_full_screen::new_agent_tab_*` and `escape_closes_the_new_agent_tab_menu_first`; review `push_menu::the_new_agent_tab_menu_*`.
  - Not built yet: "not installed" (no daemon data), ⌘T, the opening prompt. The "copilot · Note" card with Send to claude / Edit / Drop comes with UXSIMPLE's comment states.
- 2026-11-19 (DIFFUI4), plan for main, not built: "codex · not installed" (D857). The daemon already probes which agent CLIs a machine has: `ProbeResponse.agents_installed` (peer.proto), then `MachineHealth.agents_installed` (hosts.proto), then the client's `MachineHealth.agents`. But that health is read only when a machine page asks for it, so the menu has nothing pushed to read. The steps:
  1. **Proto** (hosts.proto, shared; main): add `repeated AgentId agents_installed = 21;` and `bool agents_known = 22;` to `Machine`, so the presence travels with the machine's state and its change events. When `agents_known` is false, presence is unknown.
  2. **svc_machines** (`machine_service/mod.rs` `machine()` ~598 and the event builders ~806/~1732): fill both fields from the last probe kept for that machine (`health_from` ~316). On a probe whose list differs from the last one, push the machine's change event. Never probe on behalf of the menu.
  3. **Live client** (`clients/live_machines/src/machines.rs`, where `pb::Machine` becomes `model::Machine`): map to a new `model::Machine.agents_installed: Option<Vec<Agent>>` (clients/src/model.rs is a seam file; main), using `shared.agents().agent(..)` as `health_of` does. None when `agents_known` is false.
  4. **Fake** (clients/src/fixture + fake machines): This Mac carries `Some(every agent)`. Add a fake knob that drops one agent, so tests and goldens can show a missing one.
  5. **The menu** (`ui/changes/src/panel_changes/agent_tab.rs` `menu`/`entry`):
     - The rows are Settings' `offered_agents()`, installed first, otherwise in Settings order. Presence is read from the open session's machine.
     - A missing agent draws a grey dot, dimmed, labelled "codex · not installed". It has no `on_click`, no hover and no "last used".
     - Unknown presence (None) draws a row as installed.
     - `ReviewAction::AgentTabPicked` ignores a missing agent, in case it arrives anyway.
  6. **Tests:**
     - features/review: a missing agent is never "last used".
     - native `diff_full_screen`: the menu orders installed agents first, and clicking a missing row opens nothing.
     - The native frame of the open menu is re-recorded, and Fable judges it against 3-3.
- 2026-11-19 (DIFFUI4), as D858 records:
  - **Built:**
    - A New agent tab pick always opens a fresh shell tab, starts the agent in it and types the opening prompt once the agent is quiet.
    - The tab is labelled with the agent's command and dot (app-side).
    - Done comments fold only on a ticked file.
  - **Exit status:**
    - The live flow `reviewer::` passes against a real humatad2: open, hide, show, reboot (the tab is listed again), then End.
    - Frames: 4-2/4-3 were judged by Fable. The 3-3/3-4 frames come from DIFFUI.
    - Still open: the real-agent Copilot run.
  - **Follow-ups for main:**
    1. The daemon keeps a shell tab's label and agent kind, so the label survives app restarts and reloads. Today it is `SessionState::agent_tabs`, in memory.
    2. A daemon "open a tab running agent X", instead of typing the command into a shell.
    3. "N agents" in the strip doesn't count `terminal_agents` tabs yet.
    4. After End for good on a shell tab, the app's `session.shells` stays stale until a reload: the strip and the daemon are right, but no session update is pushed. `end_running_holder` should push one.
    5. Seam lines in features/app/src/lib.rs, so the app's workspace gets the label without waiting for the next session event:
       - after `state.workspace = Arc::new(workspace);` (~1045, Loaded), add `state.session.keep_agent_tabs(Arc::make_mut(&mut state.workspace));`;
       - the same after the child reduce of `AppAction::Session(SessionAction::AgentTabOpened { .. })`.

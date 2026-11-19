# Frames: the four common flows in the proposed simpler loop

HTML mockups at 1600x950, not renders of the app. Sources and the generator are in `src/` (`python3 src/gen.py`, then screenshot each HTML). Content: a stack of 3 layers (schema, dual write, reads) across 5 repos (api, infra, ledger, sdk, web).

## Flow 1: open a terminal, start working, make a stack across many repos
- `1-1-start-working.png`: I type the task into the session's `claude` tab. I see the agent plan three layers and write across five repos. Shows: a session is just terminals in tabs; nothing else to learn to start.
- `1-2-stack-made.png`: I open Diff from the rail after the agent submits the stack. I see the 3 layers the stack tool already knows, and every repo row reading "against main" or "against release/2.4" in words. Shows: no bare chevron (reverses D739 pick A); the stack appears from the repos' own metadata (D803).

## Flow 2: ask the agent to prepare the stack for me to look at
- `2-1-ask-agent-to-show.png`: I type "show me the stack". I see the Diff panel change at once to layer 1 with a one-line purpose, key files first, and a strip with Undo. Shows: one stepper (the stack); no offer bar, view tab or Revert / Keep; the agent says it only arranged, not reviewed.
- `2-2-stack-full-screen.png`: I go full screen. I see layers, then only the repos layer 1 touches, the diff, Previous / Next layer, and one Looks good button. Shows: "Exit full screen" in place of "Show the terminal"; one header bar.

## Flow 3: review it, comment, sign off layers
- `3-1-comment-on-a-line.png`: On layer 2, I write a second comment on `client.rs`. I see my drafts as one kind of card, a drafts count on the layer, and one primary, "Send to claude". Shows: three comment states (draft, sent, done); the footer primary names its destination.
- `3-2-looks-good-per-layer.png`: I press Looks good on layer 3 and open the send summary. I see ticks on files and layers, and one message that carries 3 comments plus "layers 1 and 3 look good". Shows: Looks good replaces Mark reviewed (D183) and is the sign-off, per file and per layer.
- `3-3-new-agent-tab-menu.png`: I press New agent tab in the Diff header. I see Terminal, claude, copilot, and codex as not installed, with no model list. Shows: REVIEWER kept as a button, but it opens a tab in this session.
- `3-4-fresh-agent-beside-diff.png`: I pick copilot. I see a third tab in the session's own tab strip, a fresh agent with no history reading layer 2, and its finding as a "copilot · Note" card with Send to claude / Edit / Drop. Shows: REVIEWER folded into session tabs (frame 14, D103); one agent-authored card everywhere; no dock, no second tab strip, no "Send as mine".

## Flow 4: the agent applies it; I look again by hand
- `4-1-agent-applies-comments.png`: I press Send to claude. I see the message arrive in the terminal (4 comments, 2 layers look good), the agent's edits, and in the panel layer 2 marked "Changed since you looked" while layers 1 and 3 keep their ticks. Shows: what a sign-off buys me when I come back.
- `4-2-manual-diff-since-last-look.png`: Without asking the agent for anything, I switch the Diff to "Since I last looked". I see only the 2 files that changed, each comment as "Done by claude" with its reply and Reopen. Shows: the two-way switch replaces the preset menu's bookmark entry; addressed and resolved collapse into Done.
- `4-3-everything-looks-good.png`: I press Looks good on the last two files. I see every layer ticked, done comments folded to one line, and the pull requests up to date. Shows: the end state of the loop on one screen.

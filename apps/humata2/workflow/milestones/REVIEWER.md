# REVIEWER: I open a fresh agent beside any diff for a second pair of eyes

- **Status:** open (owner, 2026-11-19: "add a button … 'Open Review Session' which makes it so that in the diff UI it opens a terminal I can open and close with a model. This is intentionally separate because when I review a PR I might want a fresh pair of eyes")
- **Depends on:** GOLDENFIX (the Diff panel header and footer it adds to). Can open in parallel with STACKVIEW; they share `ui/changes`, so claims must not overlap.
- **Unblocks:** nothing.
- **Lane:** daemon (session start) + features/review + ui/changes + desktop goldens

## Outcome
From any Diff panel, whether it shows my own session's change, a PR review or a
stack, I press **Open review session**. I pick a model, and a new agent terminal
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
- **Button:** "Open review session" goes in the Diff panel's header beside Show the
  terminal, or in the full-screen title bar. Once a reviewer exists, the button
  reads "Show reviewer" / "Hide reviewer". A palette entry and a shortcut do the same thing.
- **Model picker:** a small sheet lists the installed agent kinds and their models, with the
  last pick remembered per user (portable config). There is no default spend.
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

## Open points for Fable 5.2
1. Is the reviewer read-only, or may it run tests or builds in a scratch copy?
2. Where does it dock: the bottom of the Diff panel or a split beside it? Which design frames does that need?
3. One reviewer per diff, or several with different models side by side?
4. Do reviewer comments go into the same draft queue as mine, or a separate one?

## Exit
- Native and design frames exist for: the button, the model sheet, the reviewer
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

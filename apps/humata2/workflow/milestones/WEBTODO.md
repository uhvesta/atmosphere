# WEBTODO: Humata's To-do in a browser, served by the host daemon

**Opens only after 0.10.0 ships and FUSEFSWIRE, SCREENPARSE, BROWSER and TODO2WIRE are validated** (owner, 2026-11-20: "at the very very very very end").

## Outcome
The owner opens a local URL in a browser and sees Humata's To-do UI — the same view as the desktop app's To-do page — live, to check Humata's overall state now and then.

## Exit
- The root/host humatad2 serves the To-do UI at a local URL (loopback only by default).
- A switch in the desktop app's Settings turns it on and off; off by default; the URL is shown next to the switch.
- The page is the real To-do UI built through GPUI's web support, not a separate web app. Where GPUI's web path lacks something the To-do UI needs, we fix the gap in our own crates.
- The page stays live (push, never poll) and survives daemon restarts.
- Tests: fast-tier unit tests for the switch and server; a headless browser check that the page loads and shows the board.

## Pointers
- D905 (DECISIONS.md).
- Existing web code to start from: apps/humata2/terminal/web/.
- To-do UI: apps/humata2/ui/workspace/src/todo.rs, features/todo.
- Settings switch: features/settings + ui/settings (follow the Sync section's flow).

## Out of scope
- Any other page than To-do; remote (non-loopback) access unless the owner asks; a browser client for the rest of the app.

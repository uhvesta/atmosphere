# NOTIFY: I can turn on desktop notifications for what needs me

- **Status:** queued (owner, 2026-11-19: "One more milestone on top might be optional desktop notifications? Do this literally at the very very end")
- **Order:** last. Opens only after FASTREL (0.6.0) has shipped; releases as 0.6.1.
- **Depends on:** FASTREL.
- **Unblocks:** nothing.

## Outcome
Desktop notifications are off by default. When I turn them on in Settings,
Humata 2 posts a macOS notification for things that need me while the app is
in the background, and clicking one opens that item.

## To decide when it opens (Fable rules; log in DECISIONS.md)
- Which events notify: inbox requests and agent questions are the obvious
  ones; finished agent turns, failed checks and Migrate items are candidates.
- Per-event switches or one switch, and quiet rules (never while the app is
  frontmost on that item; collapse bursts).
- Design frames for the Settings rows; Fable judges them.

## Exit
- Off by default; on posts notifications only for the chosen events, and a
  click opens the item.
- Tests: the event-to-notification rules in a leaf crate; native frames for
  the Settings rows.
- Each pick is logged in DECISIONS.md.

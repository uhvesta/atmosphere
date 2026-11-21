# NOTIFYAUDIT — Notifications Actually Alert (Desktop, Toast, and Sound)

Tracks milestone **NOTIFYAUDIT** (`ATM-47`–`ATM-53`, plus locked-in UX decisions from `DECISIONS.md` §4 / `ATM-46`).

## Overview

**Problem (reproduced 2026-11-20):** With `enabled`, `inbox_requests`, `agent_questions`, `play_sound`, and `quiet_when_frontmost` all `true` in `~/Library/Application Support/Humata 2/humata2-ui.json`, a test question from `#atmosphere` Desk (`rq_37d36158…`) produced no desktop alert, no in-app toast, and no sound, with nothing logged to the unified log.

**Outcome:** Every request and question reliably alerts:
- A **macOS desktop notification** when Humata is not frontmost and macOS allows delivery.
- An **in-app Deck toast** (8 seconds, pauses on hover, 1-slot replacement, never evicts `Undo`) when Humata is frontmost on another page or macOS notifications are off/unavailable.
- A **decoupled in-app chime** (`humata2-inbox.wav` / `humata2-todo.wav`) whenever a notice or toast fires and `Play a sound` is on (respecting `cooldown_secs`), **silent when the exact request is already open in front of you**.
- **Settings › Notifications** shows live `macOS permission` status, a conditional warning banner when denied/unbundled, `Toasts in Humata`, decoupled `Play a sound`, and a `Send a test notification` button.

## Locked-In Product & UX Decisions (`DECISIONS.md` §4)

1. **Deck toasts:** 8s duration, pauses on hover, 1-slot notification toast that **never evicts `Undo`**.
2. **Exact-item silence:** Never chime or toast when the exact request is already open in front of you (`viewing_request == Some(request_id)`).
3. **Warning placement:** The `"macOS notifications are off"` warning lives **only** in `Settings › Notifications` — **never** as a permanent strip inside `Agent inbox`.
4. **Decoupled sound:** `Play a sound` stays enabled and interactive even when `Desktop notifications` is off.
5. **Dock badge:** Tied strictly to blocked `Agent inbox` requests (`dock_badge_count(blocked_inbox_asks)` -> `None` when 0, `Some(n)` when > 0).

## Root-Cause Audit Summary (`ATM-47`–`ATM-50`)

| Ticket | Gate / Area | Root Cause | Fix |
|---|---|---|---|
| **ATM-47** | Tracing at every gate | Silent drops in `events.rs`, `humata2_notify::evaluate`, and `desktop/src/notifier.rs` left zero diagnostics in logs. | `trace_gate(gate, outcome, detail)` + `GateOutcome` / `NotifyTrace` logging every drop/post (silenceable via `HUMATA2_NOTIFY_TRACE=0`). |
| **ATM-48** | macOS permission & errors | `UNUserNotificationCenter` silently no-ops without a bundle id, `RequestPermission` was not surfaced in Settings, and post errors were dropped. | `PermissionStatus` (`Unknown`, `Authorized`, `Denied`, `NotDetermined`, `Provisional`, `Ephemeral`, `Unbundled`), live status query + `Request permission` trigger, error logging, and Settings status row/banner. |
| **ATM-49** | Frontmost suppression & Deck toast | `quiet_when_frontmost` suppressed OS notices with no in-app replacement; viewing another item in Inbox could suppress new requests. | Exact-item check via `FrontmostContext::viewing_request`; frontmost-on-other-page and permission-denied fall back to `DeckToast` (`DECK_TOAST_DURATION_SECS = 8`, hover-pause, never evicts `Undo`). |
| **ATM-50** | Decoupled in-app sound | Sound rode exclusively on `show_system_notification_with_sound`, so suppressed/denied/failed OS notices were silent. | `NotifyDecision::in_app_sound` plays bundled `humata2-inbox.wav` / `humata2-todo.wav` in-app whenever a notice or toast fires (keeping `burst.last_sound_secs` cooldown), even when `enabled == false`. |

## Source-to-Notifier Audit Table (`ATM-51`)

| Source | Event / `RequestKind` | `humata2_inbox::notification` & `session_meta` | Routed `NotifyEvent` | Notified? | Notes |
|---|---|---|---|---|---|
| Permission asks | `Command`, `FileEdit`, `Network`, `Merge`, `Destroy`, `BrowserRead`, `BrowserWrite` | Resolved via `humata2_inbox::notification` + `session_meta` | `NotifyEvent::InboxRequest` | Yes | Gated by `inbox_requests` (and `enabled` / `in_app_toasts`). |
| Agent questions | `RequestKind::Question` (`inbox_ask`), including Desk & channel-desk sessions filed under `'desk'` | Resolved via `humata2_inbox::notification`; `channel_id: None` / `channel_name: None` for Desk | `NotifyEvent::InboxRequest` | Yes | Gated by `agent_questions`. Verify `'desk'` sessions resolve `session_meta` without requiring a non-desk channel. |
| Terminal prompts | `Status::NeedsYou` with no pending Agent-inbox request | Resolved via session transition in `events.rs` | `NotifyEvent::TerminalNeedsYou` | Yes | Suppressed only when `has_pending_inbox_request == true` (already covered by `InboxRequest`). |
| Session failures | `resume_failed`, `Status::Exited { code != 0 }`, `Event::Restart { not_brought_back }` | Resolved via session/host transitions in `events.rs` | `ResumeFailed` / `AbnormalExit` / `RestartFailed` | Yes | Gated by `failed_resumes`; `AbnormalExit` skipped if `already_resume_failed`. |
| To-do asks | `RequestKind::TodoAsk` (excluded from `InboxRequest` path at `events.rs`) | Routed explicitly to the To-do ask branch | `NotifyEvent::TodoAsk` | Yes | Uses `NoticeSound::Todo` (`humata2-todo.wav`). |
| Journaled requests from away machines | Replayed / journaled `Event::Request` from remote daemons | Must pass `live && is_new` when arriving live from a connected away machine | `NotifyEvent::InboxRequest` | Yes | Historical backlog on initial connect stays quiet; live away-machine requests notify. |

## Shared Wiring Seams (for `@coordinator`)

1. **`ui/settings` wiring (3-line seam):**
   - Add `"//apps/humata2/features/notify"` to `deps` in `apps/humata2/ui/settings/BUILD.bazel` (or re-export `humata2_notify::PermissionStatus` from `apps/humata2/features/settings/src/lib.rs`).
   - Add `pub mod notifications;` to `apps/humata2/ui/settings/src/lib.rs`.
   - Replace the inline `Section::Notifications` top card in `apps/humata2/ui/settings/src/settings.rs` with `crate::notifications::section(...)` (keeping the Quiet hours card directly below it).
2. **`Notifier` trait & desktop bridge:**
   - Extend `Notifier` to expose `query_permission()`, `request_permission()`, `play_sound(NoticeSound)`, and `set_dock_badge(Option<usize>)` backed by `desktop/src/notifier.rs`.
3. **App reducer & Deck toast host:**
   - Wire `evaluate_decision` in `features/app/src/events.rs`, store `DeckToastQueue` and live `PermissionStatus` in `AppState`, wire `SettingsAction::ToggleInAppToasts` / `RequestNotificationPermission` / `SendTestNotification`, and render the Deck toast overlay with hover-pause without touching the `Undo` toast slot.
4. **Dock badge:**
   - Call `set_dock_badge(humata2_notify::dock_badge_count(blocked_inbox_asks))` whenever pending blocked `Agent inbox` count changes.

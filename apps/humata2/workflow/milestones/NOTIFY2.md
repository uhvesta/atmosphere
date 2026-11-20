# NOTIFY2: To-do asks stay on the To-do, and Mac notifications really arrive, with a gentle sound

- **Status:** open (owner, 2026-11-20)
- **Ships:** 0.10.2, with NEWCHCLOSE, MACHINEARCHIVE and HIGHLIGHT.
- **Owner's words:** "stuff that goes to TODO should not go to inbox, like status change requests. Also the desktop notifications on mac feel like they dont work. They could also use a noise if we want. One for INBOX and another for TODOs because they are slightly different imho. Make it pleasant and gentle please and short please."

## Outcome
1. An agent's To-do request (a suggested status change, a close, a suggested
   step) shows on the To-do, where the owner accepts or dismisses it, and never
   as an Inbox card. The Inbox keeps access approvals, questions and Migrate items.
2. With notifications on, the installed, signed Humata 2 in /Applications
   really posts macOS notifications: permission is asked, they appear while
   the app is in the background, and a click opens the item.
3. Optional sounds, with their own switch, both short (under about 0.6 s),
   soft and pleasant:
   - an Inbox sound;
   - a different To-do sound.
   The sounds are made by us, never taken from elsewhere.

## Leads
- **Routing:**
  - daemon/svc_inbox/src/inbox.rs ~1826 maps `C::TodoSuggest | C::TodoClose` to `RequestKind::Board`, so they become Inbox requests.
  - Also check daemon/humatad/src/access_service.rs ~787 and guard.rs ~461.
  - The To-do model and its pending suggestions are in features/todo and ui/workspace/src/todo.rs (TODO2 and TODO2WIRE).
  - Decide how a To-do-bound ask is resolved without an Inbox card. Fable rules.
- **Notifications:**
  - desktop/src/notifier.rs (D889) uses GPUI's UNUserNotificationCenter path, which is a no-op without a bundle identifier.
  - Find out why it feels broken in the installed app. Candidates:
    - the permission is never requested;
    - the bundle id or entitlements;
    - suppression while frontmost;
    - posting before the bridge is attached;
    - events never reaching the notifier;
    - the Settings switch is not wired.
  - Prove it with a check on the real installed app; headless tests can't.
- **Sounds:**
  - A UNNotificationSound named file in the bundle's Library/Sounds (aiff/wav/caf), added by app/app.bzl.
  - Check whether gpui's SystemNotification takes a sound; if not, add the smallest seam.
  - Generate the two sounds with a script checked into the repo, for example soft sine partials with a quick attack and a gentle decay. Fable judges that they sound pleasant, gentle and distinct.
- **Settings:** the existing notifications rows. Add a "Play a sound" switch, off by default. Fable judges the frames.

## Exit
- Unit tests:
  - a To-do ask never makes an Inbox request;
  - the event-to-notification rules include which sound plays.
- A native frame for the Settings rows, judged by Fable.
- A live check on the installed app shows a notification posted and clicked.
- Picks are logged in DECISIONS.md.

## Lane

Manager: minion NOTIFY2 (Opus 5.5), 2026-11-20.

### Diagnosis: why notifications don't arrive (verified read-only on /Applications, 0.9.0)
- The bundle is fine: id `com.avestabarzegar.humata2`, Developer ID signed (team 7H66Q22DJD), hardened runtime, no entitlements needed.
- **Cause 1 (proved): the switch is off and nothing asks.** `~/Library/Application Support/Humata 2/humata2-ui.json` has `preferences.notify.enabled = false` (the default). Turning it on calls `request_permission()`, but `desktop/src/notifier.rs` handles `RequestPermission` as a no-op. So no prompt appears, and Humata 2 is not listed in System Settings › Notifications: `defaults read com.apple.ncprefs apps` has no `com.avestabarzegar.humata2` entry.
- **Cause 2 (proved): nothing was ever posted.** Over the last 4 days the unified log shows only "Creating a user notification center" at each launch. There is no authorization request and no added notification request.
- **Cause 3 (code read): the first post can be lost.** GPUI asks for authorization inside `show()` and adds the notification at once, before the user has answered.
- **Cause 4 (code read): there is no sound anywhere.** The content has no `setSound`, and the will-present options are Banner|List without Sound.
- **Cause 5 (code read): failures are silent.** GPUI logs through `log::`, and no logger is installed.
- **Fix:**
  - a GPUI patch with a real `request_system_notification_permission` and a `sound` on `SystemNotification`;
  - the bridge's RequestPermission calls it, and so does launch when the switch is already on.

### Routing: why To-do asks reach the Inbox (verified in code)
- The daemon already marks them correctly. Previews BoardChange and NewBoardItem are left out of `agent_inbox` lists (D846, svc_inbox/src/inbox/agent_inbox.rs).
- The live event stream (`clients/live_daemon/src/events.rs` Payload::Request) still publishes them as `Event::Request`.
- The client recognises them only by `operation.starts_with("board.")`, but TodoService asks carry `todo.create` / `todo.update` (`ports::operation_of`). Also, `Preview` maps BoardChange and NewBoardItem to `Preview::Text`, so the kind is lost.
- As a result:
  - they show as Inbox cards (`features/inbox` `shown()` and `chips()` filter nothing);
  - they count in `Workspace::waiting()` badges;
  - they post an Inbox notification (features/notify, features/inbox/notice.rs, features/app/events.rs).

### Fable rulings (2026-11-20)
1. To-do asks notify under the master switch, with no sub-toggle.
   - Title for a new item: "<agent> suggests a To-do: <item>".
   - Title for a change: "<agent> wants to mark '<item>' <status>".
   - Title when the agent is blocked: "<agent> is waiting: close '<item>'?".
   - Body: "Accept or dismiss it on the To-do."
2. Clicking one opens the To-do route with the item selected and its ask in view. New click target: `NotifyTarget::TodoItem`.
3. To-do asks count only in the To-do count. `waiting()`, Inbox `shown()` and `chips()` exclude them. They are recognised by preview kind, not by operation prefix.
4. "Why?" on a To-do ask shows the reason in place on the item. The jump to the Inbox goes away.
5. A To-do ask is To-do-only even when its agent is blocked.
6. Settings rows, in order: "Desktop notifications", "Play a sound", "Permission request alerts".
   - "Play a sound" is off by default, and greyed out while the master is off (its value is kept).
   - Its description: "A short, soft chime with each notification; one for the Inbox, another for the To-do."
   - Sounds stay under about 0.4 s, the To-do one lower and softer, and a burst of asks never stacks them.

### Tasks
- N1 (done, unverified by ear): `app/sounds/make_sounds.py` writes `humata2-inbox.wav` (0.38 s, E5→A5, -16 dBFS) and `humata2-todo.wav` (0.30 s, G4, -19 dBFS). It uses only the standard library and is deterministic.
- N2: model marks a To-do ask. This is main's seam, `clients/src/model.rs`:
  - add `Request.to_do: Option<ItemId>`, or a `to_do_ask: bool`;
  - live_daemon sets it from preview BoardChange / NewBoardItem;
  - add one predicate, `Request::is_to_do_ask()`.
- N3: failing-first tests, then the fix:
  - `waiting()`, Inbox `shown()` and `chips()`, `notification()`, notify `evaluate()`, app `events.rs`, and the `ui/src` lists use the predicate in place of `"board."`;
  - To-do asks get their own notice and the TodoItem target.
- N4: sound in notices.
  - `DesktopNotice.sound: Option<Sound>` with `Sound::{Inbox, Todo}`;
  - `NotifyConfig.play_sound: false`;
  - `evaluate` picks the sound, and no sound while one played within the cooldown;
  - tests.
- N5: GPUI patch, written by main:
  - patch `gpui-pre` and `gpui-pre-macos` for permission and sound, with the `objc2-user-notifications` `UNNotificationSound` feature;
  - add the MODULE.bazel annotation, then repin.
- N6: `app.bzl`, a `sounds` attr copied into Contents/Resources. FUSEFSWIRE has uncommitted lines there, so this waits for a hand-over.
- N7: Settings row and toggle, a native frame for Fable, D-entries in DECISIONS.md, and the live check.

### Claims
- `app/sounds/**` (new) and this file.
- Granted by main on 2026-11-20:
  - `clients/src/{model.rs (to_do only),notifier.rs,workspace.rs,api.rs (re-export)}`, `clients/live_daemon/src/{events.rs,inbox.rs}`, `desktop/src/notifier.rs`;
  - `features/notify/**`, `features/inbox/src/{lib.rs,notice.rs}`, the NotificationClicked, frontmost and Request hunks in `features/app/src/{events.rs,lib.rs}`;
  - `ui/src/lists.rs` and the `ui/src/shell.rs` hunk;
  - `app/app.bzl` (sounds attr), `engine/patches` (two new files), and the MODULE.bazel annotations.
- Requested from main:
  - `clients/src/model.rs`, `clients/src/notifier.rs`, `clients/src/workspace.rs`;
  - `features/notify/**`, `features/inbox/src/{lib.rs,notice.rs}`, `features/app/src/{events.rs,lib.rs}` (NotificationClicked / AskWhy hunks), `desktop/src/notifier.rs`, `clients/live_daemon/src/{events.rs,inbox.rs}`, `ui/src/lists.rs`, and the `ui/src/shell.rs` hunk at 1111;
  - from FUSEFSWIRE: `features/settings` and `ui/settings` notification hunks, and `app/app.bzl`;
  - from TODO2WIRE: `features/todo` (AskWhy) and `ui/workspace/src/todo.rs`.

### Batch 2 (2026-11-20), written and not yet built
- N1: Fable judged the sounds and raised both levels: Inbox peak -12 dBFS, To-do -16 dBFS.
- N2: `model.rs` has `to_do` and `ToDoAsk`. live_daemon sets it from the preview kind.
- N3 (Sonnet worker): routing filters, To-do notice text, and the events.rs To-do path. Also fixed `features/app/src/home.rs` `needs_you()`.
- N4 (Sonnet worker): notify has `play_sound`, `TodoAsk`, sound picks and the no-stacking rule. The click goes to the To-do, and `desktop/src/notifier.rs` handles sound and permission.
- N5 (Sonnet worker): the two GPUI patches and the MODULE.bazel annotations. The manager wrote the `app.bzl` `sounds` attr and `app/sounds/BUILD.bazel`.
- D912 to D914 are logged.

### Planned, after FUSEFSWIRE and TODO2WIRE release their claims
- `features/settings`:
  - `Row::PlaySound` after DesktopNotifications, titled "Play a sound", with the description from Fable's ruling 6;
  - a `TogglePlaySound` action that flips `notify.play_sound`;
  - the row is disabled while `!notify.enabled`;
  - at load, when `notify.enabled` is already on, call `notifier.request_permission()` once.
- `ui/settings`: a toggle row `settings-play-sound`, greyed while the master switch is off. Then a native frame for Fable.
- `features/todo` and `ui/workspace/src/todo.rs`: "Why?" on a To-do ask expands the reason on the item, and the AskWhy jump to the Inbox goes (`features/app/src/lib.rs` AskWhy hunk). The item also says "<agent> is waiting on this" when its agent is blocked.
- Live check on the installed app, after the 0.10.2 build.

### Worker log
- 2026-11-20: diagnosis done (read-only). Fable ruled on routing and Settings. N1 written and run.

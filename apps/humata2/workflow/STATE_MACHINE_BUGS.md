# Humata 2 state machine bugs

Written 2026-11-20, before the GPUI work starts. This is every state machine bug the sweep found: eleven finders read the code, three verifiers re-read each finding against the tree as it is now (uncommitted work included), and this file merges what they kept. Nothing here was reproduced by running code. Each entry names a failing-first test, and that test is the reproduction.

Paths are relative to `apps/humata2/` unless they start with `apps/` or `libs/`. Line numbers are from the verifiers' reads on 2026-11-20 and will drift.

**Verdicts.** CONFIRMED means a verifier traced a concrete path to the bad state. PLAUSIBLE means the mechanism is real, but it depends on timing or a rare failure the verifier couldn't rule out. IN FLIGHT means the FUSEFSWIRE lane is already fixing it, so it was not re-verified.

**Severity.** P0 is data loss, a permission fail-open, a terminal or agent left stopped against "terminals keep running", or a user stuck with no way out. P1 is a silent failure in normal use, or a stuck UI that needs a restart. P2 is a rare race, or something cosmetic but wrong.

---

## 1. Summary

**82 bugs kept, 5 in flight, 15 rejected.**

| | P0 | P1 | P2 | Total |
|---|---|---|---|---|
| CONFIRMED | 1 | 10 | 21 | 32 |
| PLAUSIBLE | 2 | 7 | 41 | 50 |
| **Total** | **3** | **17** | **62** | **82** |

Plus 5 IN FLIGHT (FUSEFSWIRE) and 15 REJECTED (section 5).

**By slot:** MACHINEARCHIVE 0.10.2: 9. NEWCHCLOSE 0.10.2: 1. FUSEFSWIRE 0.10.0: 5 (in flight). SMFIX-P0: 3. SMFIX-P1: 13. SMFIX-P2: 56.

**Top 10:**
1. [SMB-064](#smb-064) P0: if agents.toml stops parsing, every agent you turned off is turned back on, with no warning in the app.
2. [SMB-072](#smb-072) P0: a move whose pause timed out can leave the old agent frozen for good while Humata shows it running.
3. [SMB-065](#smb-065) P0: after a crash during a channel rename, the agents the rename stopped can stay stopped.
4. [SMB-030](#smb-030) P1: an archived session whose shells still run stays on the Machine page as an ordinary row.
5. [SMB-002](#smb-002) P1: a failed archive from the Machine, Files or PR page shows no message; the row just stays.
6. [SMB-031](#smb-031) P1: End for good on the Machine page archives the session but ends nothing.
7. [SMB-001](#smb-001) P1: the New channel sheet stays open for the whole clone, and gets no progress at all for names with capitals or spaces.
8. [SMB-032](#smb-032) P1: after a daemon restart, To-do and Pinned stop updating with no error, and Pinned never comes back until relaunch.
9. [SMB-073](#smb-073) P1: one failed write of a terminal's output turns saving off for good, so the next restart loses what printed since.
10. [SMB-003](#smb-003) P1: while the daemon is down, the app re-subscribes in a tight loop and the reconnect toasts push out any Undo.

---

## 2. Why archiving from the Machine page sometimes leaves the session

You asked: "why is it when i archive closed sessions or stopped sessions or whatever in the hosts screen sometimes it doesnt delete. Is it that the state was errored to begin with or something?"

Partly, yes. There are three separate causes, and any one of them alone leaves the row on the page.

**1. The archive works, but the row stays.** A plain archive keeps the session's terminals running on purpose (D801 F: the app always archives with `end: None`). So the session's shell terminals, and the holder of an agent that has already exited, are still alive after the archive. The daemon's running-holder list still includes them under the session's title. It marks running ones `archived: true` (`daemon/svc_terminal/src/running_holders.rs:372-400`) but hard-codes `archived: false` for stopped shells (`:446-448`). The client then drops the `archived` flag entirely when it maps holders (`clients/live_daemon/src/running.rs:29-62`). After the reload, the Machine page has no way to tell these apart from live sessions, so the session is still listed with Open, Stop and End. The "sometimes" is whenever the session still had a shell, or an exited agent's holder, alive. Stop and Archive both leave an exited agent's holder running ([SMB-041](#smb-041)). See [SMB-030](#smb-030).

**2. The archive is refused, and nothing tells you why.** The Machine page row's Archive goes through `RunningDelegate::ArchiveSession`, then `features/app/src/desk.rs:176-188` `archive_session`, which borrows the session feature's archive flow. Every failure, at the plan step or the archive step, is written to `state.session.error` (`features/session/src/lib.rs` ~2008 and ~2062). Only the session page draws that field (`ui/workspace/src/session_tabs.rs:705`), plus the session dialogs while one is open. The Machine page draws only `state.running.error` (`ui/machines/src/machine_running.rs:361`). With no unpushed work there is no dialog, so the error is invisible and the row just stays. It then turns up later as a stale banner on whichever session page you open next. The refusals that reach this path are:
- `AlreadyArchived` after a double click ([SMB-044](#smb-044));
- "still starting" or "being resumed" for a session stuck in Launching or Resuming with nothing actually running ([SMB-042](#smb-042)). This is your "errored to begin with" case. Stop already handles it, so Stop then Archive gets out today;
- git failures while checking for unpushed work, and store errors;
- a pending approval from the access gate.

The same silence hits Archive from the Files and PR pages. See [SMB-002](#smb-002).

**3. End for good on the Machine page doesn't end anything.** End for good on a running row sends the same archive delegate (`features/hosts/src/running.rs:250-262`), and `Daemon::archive` always sends `end: None` (`clients/live_daemon/src/daemon.rs:650-675`). So the agent and its shells keep running, archived, and the row stays. See [SMB-031](#smb-031).

**The fix, together:**
- Map `archived` into `RunningHolder`, set it truthfully on stopped shells, and draw archived rows apart under "Archived, still running", offering End.
- Show archive errors on the Machine page, or toast them.
- Give `Daemon::archive` an `end` argument, and have End for good pass `true`.
- Let archive accept a stale Launching or Resuming state the way Stop does.
- Carry the session id in the archive action instead of swapping `state.session.open`.

All of this is milestone [MACHINEARCHIVE](milestones/MACHINEARCHIVE.md), shipping in **0.10.2**. Its items are SMB-002, 013, 014, 030, 031, 041, 042, 043 and 044. The first failing tests to write are in `//apps/humata2/features/app:machines_test` (archive a stopped session that still has a running shell on the fake daemon, and expect no ordinary row for it; make the fake archive refuse, and expect visible text on the Machine page) and `//apps/humata2/daemon/svc_terminal:svc_terminal_unit_test` (a stopped shell of an archived session is listed with `archived: true`).

---

## 3. Findings by area

Each entry has a checkbox. Tick it and fill in **Fixed in** with the commit when the fix lands.

### 3.1 App shell and events (features/app, ui/core)

#### SMB-001
**New channel sheet stays open for the whole clone, and gets no progress for names with capitals or spaces**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** NEWCHCLOSE 0.10.2
- **Where:** `features/app/src/new_channel.rs:290` (`None => sheet.folder() == setup.name`), `:296-305` (`listed` compares `channel.name == setup.name`), `:322` (`channel_created`); `clients/live_daemon/src/daemon.rs:247-301` (`create_channel` stores the raw typed name as the setup's name and awaits `watch_setup` before it returns the id).
- **What you see:** You press Create channel on "Q4 Billing". The channel appears in the sidebar almost at once, but the sheet keeps spinning, with no progress rows, until every repo has finished cloning.
- **Cause:** Two bugs. (a) Clone wait: the live `create_channel` holds its reply until the whole operation, clones included, is done, and the sheet closes only on that reply. (b) Slug mismatch: `setup.name` is the name as typed (the daemon only trims it), but the sheet compares it with `folder()`, the lowercase slug. Any name that isn't already a slug never adopts its setup, so the early close (all steps Done and the channel listed) never runs. Even when the names match, `listed` is checked only when a Done event arrives, and a later channel-list reload checks nothing.
- **Fix:** Match on the name the sheet sent (store it on the sheet), or have the daemon echo the operation id. Close the sheet, or hand its progress to the sidebar row, as soon as the `record` step is Done and the channel is listed. Check again on each workspace reload.
- **Failing-first test:** `//apps/humata2/features/app:new_channel_setup_test`: create "Q4 Billing" on the fake with a slow clone step. Assert that the sheet shows setup rows and closes once `record` is Done and the channel is listed, before the create reply. The existing case uses "billing-exports", which is already a slug and hides the bug.
- **Source:** V1 (03)

#### SMB-002
**Archive from the Machine, Files or PR page fails with no message**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `features/app/src/desk.rs:176-188` (`archive_session` swaps `session.open`); `features/session/src/lib.rs:2008-2011` (`ArchivePlanned` Err) and `:2055-2062` (`Archived` Err); `ui/workspace/src/session_tabs.rs:705` (the only reader of `session.error` outside dialogs); `ui/machines/src/machine_running.rs:361` (draws only `running.error`).
- **What you see:** You archive a session from the Machine page, for example while its machine is unreachable. Nothing happens. The error appears later as a stale banner on whichever session page you open next.
- **Cause:** Both Err arms only set `state.session.error`. With no archive dialog open, nothing on the Machine, Files or PR pages draws it, and no toast is sent. `Open` clears `error` (lib.rs:1120), so you see it only if you open a session page some other way. The `open` swap works today, because `Archive` reads `open` synchronously, but it is fragile.
- **Fix:** Add `SessionAction::ArchiveById(SessionId)` and drop the swap. Send `Delegate::Toast(error)` from both Err arms when the session isn't `state.open` or no dialog is up, or return the result to the Machine page as a `RunningAction::Done(Err)`.
- **Failing-first test:** `//apps/humata2/features/app:machines_test`: the fake `plan_archive` (and, in a second case, `archive`) returns Err. Route to the Machine page and send `RunningDelegate::ArchiveSession`. Assert that a toast, or Machine page state, carries the error.
- **Source:** V1 (01) and V2 (owner's bug), merged

#### SMB-003
**Events stream reconnects in a tight loop with no backoff**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `features/app/src/events.rs:448-459` (`events_ended`); `features/app/src/lib.rs:627-631` (`follow`); `libs/tca-rust/src/stream.rs:69-75` (an open failure ends the stream at once).
- **What you see:** While the daemon is down or restarting (an upgrade, say), the app spins: it re-subscribes and reloads over and over, and "Lost updates from the daemon… Reconnecting." toasts keep replacing each other, pushing out any Undo toast.
- **Cause:** `events_ended` merges `toast + load() + follow()` with no delay. The live `subscribe` fails as fast as the socket connect is refused, `Effect::stream` turns that into `StreamEnd::Failed`, and the next cycle starts straight away.
- **Fix:** Wait before `follow()` with `clients.clock.sleep(backoff)`, doubling up to a cap and resetting on the first `Event`. Toast once per outage, not once per attempt.
- **Failing-first test:** `//apps/humata2/features/app:reconnect_reload_test`: the fake daemon's `events()` fails every time. Send `EventsEnded(Failed)` and assert that no new subscribe happens until the test clock has advanced by the backoff, and that only one toast shows.
- **Note:** The client side of this loop also leaks: on the error path, live_daemon `subscribe` leaves its `Subscriber` in `state.subscribers`, so dead entries pile up for the length of the outage. That code is under the in-flight Resync fix ([SMB-040](#smb-040)); fold it in there.
- **Source:** V1 (01)

#### SMB-004
**Pressing Esc on New session, New channel or Add repo while it is working throws away the error**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `features/app/src/overlay.rs:70-75` (Escape pops the top overlay with no `creating`/`adding` guard); `new_session.rs:322-329`, `new_channel.rs:349-357`, `channel_add_repo.rs:119-124`, `session_add_repo.rs:131-138` (each Err arm writes only into a sheet that still exists).
- **What you see:** You press Create, then Esc because it is slow. If the call then fails, nothing is shown: no session, channel or repo appears, and nothing says why.
- **Cause:** `escape()` pops any overlay. Each result handler finds no sheet and returns `Effect::none()`. The Desk sheets keep a close in flight, but these four don't.
- **Fix:** In each Err arm, `if sheet.is_none() { return state.toast(error, &clock) }`, or make Esc refuse while `creating` or `adding` is set.
- **Failing-first test:** `//apps/humata2/features/app:new_session_test`, and the same shape in `:new_channel_test`, `:channel_add_repo_test` and `:session_add_repo_test`: Create, Escape, deliver `SessionCreated(Err)`. Assert that a toast appears.
- **Note:** `new_channel.rs` is also changed by NEWCHCLOSE ([SMB-001](#smb-001)). Land this after it, or in the same lane.
- **Source:** V1 (01)

#### SMB-005
**A session created after its sheet was closed closes a fresh sheet and jumps away**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/app/src/new_session.rs:315-320` (Ok removes every `NewSession` sheet and navigates unconditionally); compare the guard in `new_channel.rs:339-346`.
- **What you see:** You create a session, press Esc and open New session again. The late reply closes the sheet you are filling in and jumps to the old session.
- **Cause:** The Ok arm has no `creating` check, unlike `channel_created`.
- **Fix:** Copy the channel guard: close and navigate only when the found sheet is `creating`, otherwise toast "Created X" with Open.
- **Failing-first test:** `//apps/humata2/features/app:new_session_test`: Create, Escape, open a new sheet, deliver `SessionCreated(Ok)`. Assert the fresh sheet is still there.
- **Source:** V1 (01)

#### SMB-006
**The store bridge doesn't redraw for several fields the panes draw**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `ui/core/src/bridge.rs:221-313` (observation tuples). `deleting_channel` and `channel_menu` (drawn by Shell, `Part::Main`, `ui/src/shell.rs:991`) and `pr_dialog`/`pr_submit` (drawn by Overlays, `ui/overlays/src/overlay_pr_review.rs:129-139`) are in no tuple.
- **What you see:** The channel Delete dialog can stay on "loading", and a failed delete's error stays hidden, until you move the mouse.
- **Cause:** `flush` notifies a pane only when one of its observed tuples changes. Any other notify redraws everything, and clicks usually cause one through hover styles, so it mostly works. But async-only results such as `DeletePlanned` and `DeleteFinished(Err)` change only unobserved fields.
- **Fix:** Add `deleting_channel`, `channel_menu`, `pr_dialog`, `pr_submit`, `pr_ask`, `pr_info`, `pr_heads`, `pr_follower`, `comments`, `pinned`, `find`, and whichever other `AppState` fields a pane reads, to the matching tuple.
- **Failing-first test:** `//apps/humata2/ui/core:core_unit_test`: build a `Ui` on fakes, send `Channel(StartDelete)`, clear `dirty`, deliver `DeletePlanned`. Assert the Main bit is set.
- **Note:** The GPUI work will touch this file. Fix it as part of that work rather than separately.
- **Source:** V1 (01)

#### SMB-007
**A failed UI save is not retried**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/app/src/persist.rs:34` (`last_saved = now` before the write), `:55-64`.
- **What you see:** After one failed save (which is toasted), a restart before your next change brings back the older route, trail or draft.
- **Cause:** `persist` compares against a `last_saved` that was never written, so it skips until something changes again. `Saved` carries no generation.
- **Fix:** Carry the generation in `Saved`. On Err, reset `last_saved` (or schedule `SaveDue`) when it is the latest generation.
- **Failing-first test:** `//apps/humata2/features/app:navigation_test` (or a new persist test): a saver that fails once. Change the route, then send a no-op action. Assert a second save attempt.
- **Source:** V1 (01)

#### SMB-008
**Restart "Try again" and "Start fresh" can be sent twice**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/app/src/restart.rs:167-175`.
- **What you see:** A double click, or Try again followed by Start fresh, sends two calls, and the second one's error toast makes a working resume look failed.
- **Cause:** There is no in-flight set. What the daemon does on a second `resume` or `start_fresh` was not checked.
- **Fix:** Keep `in_flight: BTreeSet<SessionId>`, ignore repeats, and clear it on `Finished` (which then needs to carry the session).
- **Failing-first test:** `//apps/humata2/features/app:restart_test`: send TryAgain twice. Assert the fake sees one `resume` call.
- **Source:** V1 (01)

#### SMB-009
**A repo scope picked during "Save as default" is reverted**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `features/app/src/repo_search.rs:704-708` (`ScopeSaved(Ok)` clears `scope` unconditionally).
- **What you see:** You change the scope while the save runs, and your new scope snaps back to the saved one with no message.
- **Cause:** The rescope paths don't check `saving`, and the Ok arm doesn't compare scopes.
- **Fix:** Clear `scope` only when it still equals the saved scope.
- **Failing-first test:** `//apps/humata2/features/app:repo_search_test`: `SaveScopeAsDefault(A)`, rescope to B, `ScopeSaved(Ok(A))`. Assert the scope is B.
- **Source:** V1 (01)

#### SMB-010
**The toast cap pushes out an Undo toast**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `features/app/src/toast.rs:55-58`.
- **What you see:** You archive a session, then three more toasts arrive (reconnect messages, say) and the archive's Undo is gone.
- **Cause:** The oldest toast is removed whatever its action. The reconnect loop ([SMB-003](#smb-003)) makes this likely.
- **Fix:** Evict toasts with no action first.
- **Failing-first test:** `//apps/humata2/features/app:session_menu_test`: archive (Undo toast), then three plain toasts. Assert the Undo toast is still there.
- **Source:** V1 (01)

### 3.2 Sessions, hosts and Desk UI (features/session, features/desk, features/hosts, features/mass_delete)

#### SMB-011
**Desk Cancel during a start, clean up or save-before-close drops its failure**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `features/desk/src/lib.rs:549-579` (`CancelDialog` clears `new`, `channel_new`, `clean_up` and `close` with no `starting`/`deleting`/`saving` guard); `lib.rs:485-494` (`Started` Err); `clean_up.rs:151-157` (`CleanedUp` Err); `close.rs:108-114` (`saved_before_close` Err); `features/app/src/desk.rs:146-148` (toasts only `Changed` errors).
- **What you see:** You click Start on a new Desk session and press Esc while it starts. If the start fails, nothing says so, and you don't know it never started. A failed clean-up delete or version save after Esc is just as silent.
- **Cause:** `CancelDialog` clears the form or dialog while its call is in flight, and every Err arm writes only into a form or dialog that still exists. A start that succeeds after Cancel still opens the session (desk.rs:140), which is acceptable but inconsistent. A clean up that succeeds still toasts, so only failures are lost.
- **Fix:** Keep the form or dialog while `starting`, `deleting` or `saving` (the guard `channel_close` already uses), or set `state.error` / toast when no form is left.
- **Failing-first test:** `//apps/humata2/features/desk:start_test`: `StartNew`, submit, `CancelDialog`, `Started(Err)`. Assert `state.error` is set, or the form is still there with the error. Add the same shape in `:clean_up_test` and `:close_confirm_test`.
- **Source:** V1 (01, 02; three findings merged)

#### SMB-012
**The channel Desk close sheet gets stuck if another change for that session is in flight**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `features/desk/src/channel.rs:398-413` (`confirm_close_channel` sets `closing = true`, then calls `change()`); `features/desk/src/lib.rs:361-373` (`change` returns `Effect::none()` when `pending` already holds the session); `lib.rs:571-574` (`CancelDialog` keeps a sheet that is `closing`).
- **What you see:** You toggle Keep, Access, Switches or Target on a channel Desk session and click Close before the toggle answers. The close sheet stays, Close disabled and Cancel doing nothing, until Humata restarts.
- **Cause:** `closing` is set before `change` refuses because of `pending`, so no Close call goes out. The earlier change answers with a non-Close kind, so `close_sheet_answered` never runs. Nothing else clears `channel_close` (the only writers are lib.rs:573 and channel.rs:478).
- **Fix:** Check `state.pending.contains(&session)` before setting `closing` and say "a change is still saving", or reset `closing` when `change` returns no effect.
- **Failing-first test:** `//apps/humata2/features/desk:channel_desk_test`: hold a `set_switch` reply on the fake, `AskClose`, `ConfirmCloseChannel`, release the switch reply, `CancelDialog`. Assert `channel_close` is None, or `closing` is false.
- **Source:** V1 (02)

#### SMB-013
**The End-for-good flag survives `Open`, so a later plain archive reports "Ended"**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `features/session/src/lib.rs:1117` (`Open` clears `archiving` but not `ending_for_good`); `close.rs:292`; `lib.rs:2040-2047` (`Archived` Ok checks `ending_for_good`).
- **What you see:** A later plain archive of that session says "Ended and archived", and its Undo restores the session as Stopped.
- **Cause:** `ending_for_good` is cleared on Cancel, Err and Archived, but not when `Open` drops the archive dialog.
- **Fix:** Clear `ending_for_good` in `Open` together with `archiving`, or store it inside `ArchiveConfirm`.
- **Failing-first test:** `//apps/humata2/features/session:archive_test`: End for good with unpushed work (dialog up), `Open(B)`, then archive A from elsewhere. Assert the `Archived` delegate, not `EndedForGood`.
- **Source:** V1 (11)

#### SMB-014
**The close-tab flow acts on whichever session is open when it answers**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `features/session/src/close.rs:316-345` (`close_finished` sends `CloseTab{group,index}`); `lib.rs:1435-1447` (`CloseTab` uses `view_mut()` of the open session); `close.rs:290-294` (End for good writes `state.open = confirm.session`); `lib.rs:1104-1120` (`Open` doesn't clear `closing`).
- **What you see:** If the open session changes while a Stop is in flight (a notification opens another session, say), the answer closes a tab in the other session, and End for good can point `state.open` back at the first session while the screen shows the second.
- **Cause:** `CloseTab` carries no session, and `Open` leaves `closing` alone. The dialog is modal, so only navigation that isn't a click reaches this.
- **Fix:** Carry the session in `CloseTab` (or skip it when `state.open != Some(confirm.session)`). Pass the id to Archive instead of writing `open` (the same change as [SMB-002](#smb-002)), and clear `closing` in `Open` unless it is sending.
- **Failing-first test:** `//apps/humata2/features/session:close_tab_test`: ask to close a tab in A, choose Stop, `Open(B)`, deliver `Stopped(Ok)`. Assert B's view is unchanged and A's tab is removed.
- **Source:** V1 (02, 11; merged)

#### SMB-015
**A second agent tab with a prompt overwrites the first one's pending prompt**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/session/src/lib.rs:1234` (`OpenAgentTab` overwrites the single `opening_prompt` slot), `:1250-1254` (`take()` then filter), `:1264-1268` (`PersonTyped` cancels only through `awaits_opening_prompt`).
- **What you see:** You open two agent tabs with opening prompts back to back. The first tab's prompt can still be typed in after you have started typing there yourself.
- **Cause:** `opening_prompt` is one slot. `opening_terminal` blocks only until the first tab's shell opens, while its prompt keeps waiting for up to 15 s. A second `OpenAgentTab`, in any session, replaces the slot, so `PersonTyped` on the first tab finds nothing to cancel.
- **Fix:** Key the prompts by `(session, tab)` in a map, and take one only when it matches.
- **Failing-first test:** `//apps/humata2/features/session:opening_prompt_test`: open an agent tab with prompt P1, let it open, open a second with P2, send `PersonTyped` on tab 1. Assert the cancel for tab 1 is emitted and P1 is never typed.
- **Source:** V1 (01)

#### SMB-016
**An opening prompt is dropped silently when its tab can't be linked or the agent is slow**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/session/src/opening_prompt.rs:152-180` (`run` returns `Ok(())` on a link failure and on the 15 s `DEADLINE`).
- **What you see:** Your opening prompt never reaches the agent (for example when the agent takes more than 15 s to start), and nothing says so. The tab is labeled as an agent tab even if the agent command was never typed.
- **Cause:** It "gives up quietly" by design, and `OpeningPromptDone(Ok)` shows nothing.
- **Fix:** Return a distinct give-up result and toast "Your prompt wasn't sent; paste it into the agent", keeping the text so it can be copied.
- **Failing-first test:** `//apps/humata2/features/session:opening_prompt_test`: the fake `link_unsized` fails. Assert a notice or toast delegate is sent.
- **Source:** V1 (11)

#### SMB-017
**A late rename success clears another session's rename draft**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/session/src/menu.rs:93-94` (`Renamed(Ok)` clears `renaming` without checking which session).
- **What you see:** Text you typed into a new rename dialog vanishes when an earlier rename of another session finishes.
- **Cause:** `RenameDraft` and `Renamed` carry no session. You have to switch sessions during a rename send, which the modal mostly prevents.
- **Fix:** Carry the session in `Renamed` and the draft, and clear only on a match.
- **Failing-first test:** `//apps/humata2/features/session:session_menu_test`: rename A (held), `Open(B)`, StartRename, deliver `Renamed(Ok)` for A. Assert B's draft remains.
- **Source:** V1 (02)

#### SMB-018
**Conversation: one empty "Show later turns" hides the button for good and skips turns**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/session/src/conversation.rs:124-128` (`merge`, Later), `:83-85` (`can_show_later`).
- **What you see:** On a running session, once one "Show later turns" comes back empty, the button disappears and new turns never show until you reopen the tab.
- **Cause:** `at_end` is never cleared, and the empty Later read still moves `last` forward by STEP, so even with the flag cleared, turns in that range would be skipped.
- **Fix:** On an empty Later read, leave `last` where it was. Clear `at_end` on each new mirror snapshot or after a short delay.
- **Failing-first test:** `//apps/humata2/features/session:conversation_test`: an empty Later read, then the fake history gains turns. Assert `can_show_later()` is true again and the next Later range starts at the old `last + 1`.
- **Source:** V1 (11)

#### SMB-019
**Conversation: a stale reply for the same target clears `loading` early**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/session/src/lib.rs:1385-1405` (`ConversationLoaded` checks only `target`), `:914-925` (`start_conversation`).
- **What you see:** You reopen the same search hit while a read is in flight. The old reply merges into the fresh conversation, can hide "Show later", and lets a second read start before the first lands.
- **Cause:** Reads have no generation.
- **Fix:** Add a read generation to `Conversation` and `ConversationLoaded`, and drop mismatches.
- **Failing-first test:** `//apps/humata2/features/session:conversation_test`: a held Later read R1, reopen the same target (R2), deliver R1. Assert `loading` is still `Around`.
- **Source:** V1 (11)

#### SMB-020
**Add machine: a progress stream that ends with no outcome leaves no retry**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/hosts/src/add.rs:647-655` (`ProgressEnded`), `:657-660` (`TryAgain` needs `failure`); `ui/overlays/src/overlay_add_machine.rs:742-770` (Add is disabled while `operation` is Some).
- **What you see:** If the link breaks during an add, the steps freeze, Try again is missing and Add is disabled, so Cancel is the only way out. If the stream ends cleanly there isn't even an error.
- **Cause:** `ProgressEnded` clears `running` but sets no `failure`. The UI offers Try again only when `failure` is set, and Add only when `operation` is None.
- **Fix:** When the stream ends without an outcome, set an error and allow `TryAgain` when `operation` is Some and `!running` (or re-follow the operation).
- **Failing-first test:** `//apps/humata2/features/hosts:add_test`: Start, a progress event, `ProgressEnded(Finished)`. Assert TryAgain re-opens the stream.
- **Source:** V1 (02)

#### SMB-021
**Add machine: the login stays "finishing" if its stream ends cleanly**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/hosts/src/add.rs:780-784` (`LoginEnded` handles only `Failed`).
- **What you see:** After Done in the login sheet, if the stream ends with no Ready or Failed, Done stays disabled and Start and Manage providers stay blocked. Only Cancel works.
- **Cause:** `StreamEnd::Finished` with the login still open doesn't clear `finishing`. Whether the daemon ever ends that way was not checked.
- **Fix:** On `Finished` with the login still open, clear `finishing` and set "The login ended before it finished. Try again."
- **Failing-first test:** `//apps/humata2/features/hosts:add_test`: LoginDone, `LoginEnded(Finished)`. Assert `finishing` is false and an error is set.
- **Source:** V1 (02)

#### SMB-022
**The Hosts "stopped" check reads the workspace, which a successful start doesn't update**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/hosts/src/lib.rs:301-305` (reads `workspace.machine().stop`), `:319-325` (clears only `health[..].stop`).
- **What you see:** Right after a successful Start sessions, the row still says stopped, and Start sessions asks you to confirm again.
- **Cause:** Two sources of truth. The gap lasts until the next workspace reload from the Machines event.
- **Fix:** Read the stop from one place (health, with the workspace as a fallback), or clear both.
- **Failing-first test:** `//apps/humata2/features/hosts:health_test`: a stopped machine, ConfirmStart, `SessionsStarted(Ok)`, then StartSessions. Assert `start_confirm` is None.
- **Source:** V1 (02)

#### SMB-023
**Mass delete: the plan for an abandoned sheet fills the next sheet**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/mass_delete/src/lib.rs:112-119` (`open`, not cancellable), `:144-150` (Escape while loading); `sheet.rs:179-188` (`planned` doesn't check the scope); `lib.rs:176-193` (`confirm` deletes by `plan.token`).
- **What you see:** You open delete for selection A, press Esc, pick B and open again. A's plan fills the new sheet, and Confirm moves A's sessions to the trash while the sheet says B.
- **Cause:** `Planned` carries no scope or generation, and the sheet takes any plan.
- **Fix:** Tag `Planned` with the scope (or a generation) and drop it unless it matches `sheet.scope`, or make it `.cancellable("mass-delete-plan", true)` and cancel on Escape.
- **Failing-first test:** `//apps/humata2/features/mass_delete:sheet_test`: OpenPicked(A), Escape, OpenPicked(B), deliver A's plan. Assert `sheet.plan` is None and `loading` is true.
- **Source:** V1 (02)

### 3.3 Other features (features/pr_review, todo, views, settings, files)

#### SMB-024
**PR Submit review: a plan that answers after a re-open applies to the new session**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED (rare, but it can post reviews to the wrong pull requests)
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/pr_review/src/submit.rs:203-206` (`plan`, not keyed or cancellable), `:258-270` (`Planned` guarded only by `step == Closed`).
- **What you see:** You close Submit review on A and open it on B before A's plan answers. A's pull requests and drafts appear under B, and Post uses A's token.
- **Cause:** `Planned` has no session, and `Open` resets state but can't stop the old future.
- **Fix:** Carry the session in `Planned` and drop it unless `state.session == Some(session)`, or make the plan `.cancellable("pr-submit-plan", true)`.
- **Failing-first test:** `//apps/humata2/features/pr_review:submit_test`: Open(A) held, Close, Open(B), deliver A's plan. Assert step is Planning and there are no reviews.
- **Source:** V1 (03)

#### SMB-025
**To-do: overlapping writes on one item restore or apply a stale copy**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/todo/src/lib.rs:851-862` (`write` snapshots `before` and bumps the revision optimistically), `:1307-1312` (`Written` Ok/Err replaces the whole entry), `:484-489` (`replace` doesn't compare revisions).
- **What you see:** After two quick changes to one item (status, then priority), the row can show a value the host doesn't have until the next reload.
- **Cause:** Replies can arrive out of order, and both Ok and Err overwrite the whole entry with no revision check. If A fails, B (sent against A's revision) usually fails too, and B's `before` includes A's optimistic change. If B's error lands last, the row shows A's change, which never happened.
- **Fix:** In `replace`, ignore entries older than the revision held. On Err, refetch the item instead of restoring a snapshot.
- **Failing-first test:** `//apps/humata2/features/todo:todo_test`: two patches on one key, then deliver the `Written` replies in reverse order (Ok then Ok, and Err then Err). Assert the final entry matches the host.
- **Note:** WEBTODO serves this feature on the web. Do this one before WEBTODO if it is cheap.
- **Source:** V1 (03)

#### SMB-026
**Views: a refused Keep, Close or Revert hides the view while saying it is "back"**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `features/views/src/lib.rs:321-341` (`showing.take()` before the answer), `:343-366` (`Answered` Err restores the record only).
- **What you see:** The daemon refuses Keep. The error says "the view is back as it was", but it is gone from the screen, and you have to find it in the journal to answer again.
- **Cause:** On Err the record goes back to its prior state, but `showing` is not restored.
- **Fix:** On Err, set `showing = Some(Showing{id: prior.id})` when the prior state was Opened and nothing else is showing.
- **Failing-first test:** `//apps/humata2/features/views:journal_test`: Open, Keep, `Answered(Err)`. Assert `showing` is the view's id.
- **Source:** V1 (03)

#### SMB-027
**Repositories settings: a load reply clears `busy` while Remove is running**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/settings/src/repositories.rs:155-172` (every Shown/Saved/Err arm sets `busy = false`), `:175-182` (`Remove` sets it).
- **What you see:** You reopen the section during a Remove. The buttons come back early, and a stale list can show the removed repo again until the next reload.
- **Cause:** One shared boolean for several concurrent calls.
- **Fix:** Track the call in flight as an enum and let only its own reply clear it. Drop `LibraryShown` replies that started before a write.
- **Failing-first test:** `//apps/humata2/features/settings:repositories_test`: Remove (held), Open, deliver `LibraryShown(Ok)`. Assert `busy` is true.
- **Source:** V1 (03)

#### SMB-028
**Providers import: Cancel is ignored while Review is pending**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `features/settings/src/providers.rs:194-198`, `:208-224`.
- **What you see:** If the preview call hangs, the import sheet can't be closed.
- **Cause:** `CancelImport` refuses while `busy`, and the client sets no timeout. A dead daemon errors out, so only a hung daemon traps the sheet.
- **Fix:** Let Cancel close the sheet, and ignore a late `Reviewed` (`state.import` being None is already handled).
- **Failing-first test:** `//apps/humata2/features/settings:providers_test`: Review (never answers), CancelImport. Assert `import` is None.
- **Source:** V1 (03)

#### SMB-029
**Files: two listings of one folder can land out of order**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `features/files/src/manager/pane.rs:274-287` (`load_pane`, not keyed), `:147-160` (`listed` checks only the folder).
- **What you see:** After a refresh races a change event, or a quick F to G to F, the pane can show an older listing until the next change.
- **Cause:** No generation. `read_unix_s` exists but isn't compared.
- **Fix:** Carry a per-pane load generation in `Listed` and drop older ones (as `find` does), or drop listings with an older `read_unix_s`.
- **Failing-first test:** `//apps/humata2/features/files:manager_panes_test`: two loads of F, deliver the second then the first. Assert the entries are the second's.
- **Source:** V1 (03)

### 3.4 Clients (clients/live_daemon)

#### SMB-030
**The Machine page lists an archived session's live terminals as ordinary session rows**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `clients/live_daemon/src/running.rs:29-62` (drops `archived`); `daemon/svc_terminal/src/running_holders.rs:446-448` (stopped shells hard-code `archived: false`).
- **What you see:** After you archive a session from its machine's page, it is still listed there, with Open, Stop and End.
- **Cause:** By design (D801 F), a plain archive keeps the agent and shells running. The daemon flags those holders `archived` (hosts.proto:929), but the client model has no such field, so the UI can't tell them from live sessions. Stopped-shell rows don't even set the flag.
- **Fix:** Add `archived: bool` to `RunningHolder` and map it. Set it from `store.archive()` for stopped shells too. Draw archived rows apart, with only End.
- **Failing-first test:** `//apps/humata2/daemon/svc_terminal:svc_terminal_unit_test` (an archived session's stopped shell has `archived: true`), plus a `clients/live_daemon` mapping test and the `//apps/humata2/features/app:machines_test` case in section 2.
- **Source:** V2 (owner's bug)

#### SMB-031
**End for good on the Machine page archives without ending the terminal**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `features/hosts/src/running.rs:250-262`; `clients/live_daemon/src/daemon.rs:650-675` (`end: None`).
- **What you see:** "End for good" on a running session leaves its agent running, archived, and the row stays.
- **Cause:** The client's `Daemon::archive(session, push_first)` has no end argument, so every app archive sends `end: None`, which keeps terminals running.
- **Fix:** Add `end: bool` to `Daemon::archive`. End for good passes `true`, plain Archive keeps `false`.
- **Failing-first test:** `//apps/humata2/features/app:machines_test` with a recording fake: End for good sends `end = true`.
- **Source:** V2 (owner's bug)

#### SMB-032
**To-do and Pinned stop updating silently after a daemon restart, and Pinned is never followed again**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `clients/live_daemon/src/todo.rs:915-917`, `:922`; `clients/live_daemon/src/marks.rs:142-144`, `:149`; consumers `features/todo/src/lib.rs:1014-1016`, `features/find/src/pinned.rs:126-128`; reconnect `features/app/src/events.rs:448-461`.
- **What you see:** After a daemon restart or upgrade, Pinned stops changing until you relaunch, and To-do stops until you reopen the page. No error shows.
- **Cause:** Both `forward` loops return silently when the stream ends or a list read fails, and the consumer ends on `None`. `events_ended` and Resync only `load()` and `follow()`. `PinnedAction::Start` is sent only on Launch or Retry, and `TodoAction::Watch` only when you enter the route.
- **Fix:** Send `Err(Offline)` on the feed before returning, and retry a failed list read. Re-dispatch `PinnedAction::Start` (and `TodoAction::Watch` when To-do is showing) on `EventsEnded` and Resync.
- **Failing-first test:** `//apps/humata2/clients/live_daemon:marks_live_test` and `:todo_test`: restart the daemon, expect an error item. In the `features/app` TestStore, `EventsEnded` must dispatch `Pinned(Start)`.
- **Note:** WEBTODO serves To-do on the web, so this must land first.
- **Source:** V2 (04)

#### SMB-033
**A second subscriber gets no event stream when the first subscribe call fails**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `clients/live_daemon/src/events.rs:134-141`, `:156-159`; callers `terminals.rs:108`, `daemon.rs:1015`.
- **What you see:** If a terminal tab and the app's main subscription start together while the daemon restarts, live updates stop with no error and no reconnect.
- **Cause:** B joins while A's call is in flight and returns `Ok`. A's call fails and resets `subscribed`, but B's sender stays with no follow task and never gets `Offline`, so `EventsEnded` never fires.
- **Fix:** In the `Err` arm, call `end_all()` (send `Offline` to every waiting subscriber), or share one start result between callers.
- **Failing-first test:** `//apps/humata2/clients/live_daemon:live_daemon_unit_test`: hold and then fail the subscribe call with two callers. The second gets `Err(Offline)` after its Resync.
- **Note:** This is a different bug from the in-flight Resync fix ([SMB-040](#smb-040)), but it is in the same function. Land it on top of the FUSEFSWIRE change; don't rewrite `subscribe` in parallel.
- **Source:** V2 (04)

#### SMB-034
**A reload whose server-info read fails clears `restart_known`, so the restart report shows again**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `clients/live_daemon/src/daemon.rs:224-227` (`load`), `:28-34` (`read_restart` returns None on a failed call).
- **What you see:** A restart notice, or a "not brought back" notification, can appear a second time after a reconnect.
- **Cause:** `load` overwrites the flag from a read that can't tell a failure from "no report". The finder's `|=` fix is wrong, because the overwrite is needed after a real restart.
- **Fix:** Have `read_restart` return failures distinctly, and leave `restart_known` alone on failure.
- **Failing-first test:** `//apps/humata2/clients/live_daemon:live_daemon_unit_test`: a known report, then a reload whose call fails, keeps `restart_cue()` false.
- **Source:** V2 (04)

#### SMB-035
**A mass delete that partly succeeds and then fails loses the deleted count**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `clients/live_daemon/src/mass_delete.rs:147-153`; `features/mass_delete/src/sheet.rs:219-243`.
- **What you see:** The sheet says the delete failed although some sessions went to the trash, and a second Delete fails because the plan token is spent.
- **Cause:** The `?` on `finish` drops `done.sessions` (a count). The session list itself follows events, so it stays right.
- **Fix:** Return an outcome carrying both the count and the failure, and ask for a fresh plan.
- **Failing-first test:** A `//apps/humata2/features/mass_delete` reducer test (`:sheet_test`) for the partial-failure outcome.
- **Source:** V2 (04)

#### SMB-036
**The Inbox answer cache can keep a stale pending request when a page read and an event overlap**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `clients/live_daemon/src/inbox.rs:673-690`, `:717`.
- **What you see:** Approving a request that was just answered elsewhere fails, and the stale pending row stays until reload.
- **Cause:** The older page read re-inserts the request after a newer event removed it, with no revision compare.
- **Fix:** Write or remove only when `wire.revision` is at least the cached revision, and remember removed ids with their revision.
- **Failing-first test:** `//apps/humata2/clients/live_daemon:live_daemon_unit_test`: map rev 4 answered, then rev 3 pending. Assert there is no pending request.
- **Source:** V2 (04)

#### SMB-037
**Two tab-list refreshes that finish out of order can cache a closed tab**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `clients/live_daemon/src/terminals.rs:67`, `:75-80`, `:102`.
- **What you see:** Rarely, opening a session's first tab, or a link, targets a tab that was just closed.
- **Cause:** The last write wins. The next session event usually fixes it.
- **Fix:** Stamp each refresh with a per-session sequence number and drop older results.
- **Failing-first test:** `//apps/humata2/clients/live_daemon:live_daemon_unit_test`: answer two refreshes in reverse order. Assert the newer list wins.
- **Source:** V2 (04)

#### SMB-038
**Typing a line waits forever if the daemon never sends a snapshot or ack**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `clients/live_daemon/src/terminals.rs:234-243`, `:252-260`.
- **What you see:** A send to a hung terminal never returns, and the composer stays on "sending".
- **Cause:** No deadline on either `recv()` loop. The file uses `START_WAIT` elsewhere.
- **Fix:** Wrap both in `tokio::time::timeout` and return `Refused` with "open the tab again".
- **Failing-first test:** `//apps/humata2/clients/live_daemon:live_daemon_unit_test`: on a silent open link, `type_line` errors within the timeout.
- **Source:** V2 (04)

#### SMB-039
**A finished move can stay shown as Moving because the watcher publishes after the final event**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `clients/live_daemon/src/events.rs:180-196` (copy under lock, publish after), `:53-56`.
- **What you see:** Rarely, a session whose move has finished keeps showing Moving until its next event.
- **Cause:** The daemon does publish at move end (move_session.rs:650-651). But `watch_move` can publish a stale Moving copy after the follow task has already published the finished one.
- **Fix:** Re-check the entry before publishing, publish under the lock, or skip once the operation is no longer Running.
- **Failing-first test:** `//apps/humata2/clients/live_daemon:live_daemon_unit_test`: interleave the final update with a not-moving event. Assert the last published session has `moving == None`.
- **Source:** V2 (04; the finder's mechanism was wrong, see section 5)

#### SMB-040
**Events subscribe Resync (in flight)**
- [ ] **Fixed in:**
- **Severity / verdict:** IN FLIGHT (not re-verified)
- **Slot:** FUSEFSWIRE 0.10.0 (in flight)
- **Where:** `clients/live_daemon/src/events.rs` (`subscribe`, ~128-169).
- **What it is:** The subscribe Resync ordering fix the FUSEFSWIRE lane is already making. Listed so nobody fixes it twice.
- **Also fold in:** On the error path, `subscribe` leaves its `Subscriber` in `state.subscribers`, so dead entries pile up during an outage (from [SMB-003](#smb-003)). [SMB-033](#smb-033) lands on top of this change.
- **Source:** VERIFY.md in-flight list

### 3.5 Daemon sessions (humatad session_service, svc_desk)

#### SMB-041
**An exited agent's terminal is never ended by Stop, or by Archive with end**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `daemon/humatad/src/session_service.rs:1101-1103`; `daemon/humatad/src/session_service/archive.rs:1125-1127` (`stop_agent` acts only on `Running`).
- **What you see:** Stop, or archive and end, on an agent that has exited leaves its terminal listed on the Machine page, and the session's folder is never freed.
- **Cause:** `record_exit` saves `Exited` while the holder keeps serving its scrollback. Stop returns `Done` for `Exited`, `Stopped` and `ResumeFailed` without touching the holder. With `end = true`, `stop_agent` returns `Ok` early for anything but `Running`. `free_folder` then sees the socket still answering and keeps the folder.
- **Fix:** For `Exited`, end the holder if it answers, remove the agent tab and save `Stopped`, in both Stop and `stop_agent`.
- **Failing-first test:** `//apps/humata2/daemon/humatad:stop_test`: an agent exits; Stop; assert the holder socket is gone and the state is Stopped. The same in `:archive_test` with `end = true`, asserting the row reaches Freed.
- **Source:** V2 (05; two findings merged)

#### SMB-042
**Archive refuses a session left Launching or Resuming with nothing actually starting**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `daemon/humatad/src/session_service/archive.rs:536-540` (compare `refuse_while_starting` at `:1596`, and Stop's `launch_in_flight` / `left_resuming` at `session_service.rs:1080-1100`).
- **What you see:** Archive (and End for good) on a stuck session says "wait for that to finish", but there is nothing to wait for. On the Machine page it fails silently.
- **Cause:** The match arm refuses `Launching` and `Resuming` unconditionally, before the liveness check in `refuse_while_starting`, whose doc says a stuck session can be archived. A restart can leave Resuming (`relaunch_end` gives `Stuck`), and so can a failed start-fresh rollback ([SMB-046](#smb-046)). Stop handles both, so Stop then Archive gets out today.
- **Fix:** Refuse only when `launch_in_flight(id)` or a resume is live. Otherwise end the old holder the way Stop does, and go on.
- **Failing-first test:** `//apps/humata2/daemon/humatad:archive_test`: save an agent as Launching with no start running. Assert archive succeeds.
- **Source:** V2 (05)

#### SMB-043
**An archived session's folder free is deferred once and never retried; failures are only logged**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `daemon/humatad/src/session_service/archive.rs:1333-1340` (`free_now`), `:1344-1350` (`free_folder` returns Ok while terminals keep running).
- **What you see:** An archived session's clones stay on disk after its last terminal is closed, until the daemon restarts. A failed delete shows nowhere.
- **Cause:** `schedule_free` runs once. While terminals still run, it returns without rescheduling. The only other trigger is restart recovery. Errors go to the log only.
- **Fix:** Re-arm the free when the session's last holder ends (or on a holder-gone event), and record a failed free on the archive row or an operation.
- **Failing-first test:** `//apps/humata2/daemon/humatad:archive_test`: archive with a shell running, close the shell after the undo window, and assert the row reaches Freed with no restart.
- **Source:** V2 (05)

#### SMB-044
**Two archives of one session can both run**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** MACHINEARCHIVE 0.10.2
- **Where:** `daemon/humatad/src/session_service/archive.rs:519-526` (lineage check), `:672-676` (row saved without the session lock), `:1204` (`run_archive` locks).
- **What you see:** A double click on Archive can briefly flip the session back to Archiving, push its free time later, or show a silent AlreadyArchived on the Machine page.
- **Cause:** The client sends a fresh idempotency key per call. Both calls can pass `is_archived` across the git check before either saves its row. Both then run `finish_archive` and `schedule_free`. It heals itself.
- **Fix:** Take the session lock across the check and `save_archive`. Mark the Machine page row busy while the archive is in flight.
- **Failing-first test:** `//apps/humata2/daemon/humatad:archive_test`: two concurrent archives. Assert exactly one succeeds and the other is refused AlreadyArchived.
- **Source:** V2 (05)

#### SMB-045
**Restoring a fork can reclone into a folder that a concurrent free deletes**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/humatad/src/session_service/archive.rs:1444` (own lock only), `:1487-1490` (reclone while the row is still Freed), `:1344-1409` (free under the folder lock), `:202-220` (`sharers_of` counts Freed as freed).
- **What you see:** Restoring a fork reports success, but its folder and fresh clones are gone.
- **Cause:** `run_restore` never takes the `folder:<path>` lock, and the fork's row stays Freed until after the reclone. Meanwhile the parent's free sees no unfreed sharer and runs `remove_dir_all` on the shared folder.
- **Fix:** Take the folder lock around the reclone, or mark the row Restoring first and count Restoring as unfreed.
- **Failing-first test:** `//apps/humata2/daemon/humatad:archive_test`: archive a context-only fork and its parent, then free the fork. Hold the fork's restore mid-reclone with a fault point and fire the parent's free. Assert the fork's folder exists.
- **Source:** V2 (10)

#### SMB-046
**A failed Start fresh can leave the session "being resumed" until the daemon restarts**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/humatad/src/session_service/start_fresh.rs:362-370`; `session_service.rs:1100`, `:1291-1297` (`left_resuming`).
- **What you see:** After a failed Start fresh, the session says "being resumed" for good. Resume, Start fresh, Stop and Archive all refuse until humatad restarts.
- **Cause:** The rollback's `put_agent` result is thrown away. If it failed, the record stays Resuming with nothing running, and `left_resuming` is false because no restart named it. It needs two failures in a row.
- **Fix:** Check the rollback result, and on failure save `ResumeFailed { reason }`. Or let `left_resuming` accept a session with no live operation.
- **Failing-first test:** `//apps/humata2/daemon/humatad:start_fresh_test`: fail both the launch and the rollback write. Assert Stop succeeds, or the state is ResumeFailed.
- **Source:** V2 (09)

#### SMB-047
**Stop withdraws no Inbox requests when retried after a partial stop**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `daemon/humatad/src/session_service.rs:1101-1103`, `:1118-1128`.
- **What you see:** A stopped session's pending Inbox requests can stay pending until it is archived.
- **Cause:** `Stopped` is saved before `withdraw_requests_of`. If the withdraw fails or the daemon dies in between, a retried Stop hits the already-stopped arm and returns without withdrawing. Nothing at boot withdraws per-session requests (archive does, archive.rs:1307).
- **Fix:** Call `withdraw_requests_of` (it is idempotent) in the already-stopped arm too.
- **Failing-first test:** `//apps/humata2/daemon/humatad:stop_test`: fault after the `put_agent`, restart, Stop again, and assert the requests are withdrawn.
- **Source:** V2 (09)

#### SMB-048
**Two mass deletes with the same plan token can both run**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/session_service/delete_many.rs:70-80`, `:1035-1044`; `clients/live_daemon/src/mass_delete.rs:135`.
- **What you see:** A double confirm can report "couldn't be moved to the trash" for sessions that were in fact deleted.
- **Cause:** Each call gets a fresh idempotency key. `peek_token` and `take_token` are separated by awaits, and `run_one` takes no session lock.
- **Fix:** Spend the token atomically when the call commits, and use one key per plan.
- **Failing-first test:** `//apps/humata2/daemon/humatad:delete_sessions_test`: two concurrent calls with the same token run exactly one operation.
- **Source:** V2 (10; pairs with [SMB-035](#smb-035))

#### SMB-049
**A crash between the trash put and the marker write makes a second, near-empty trash item**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/session_service/delete_many.rs:1238-1257`, `:638-660`.
- **What you see:** The trash shows two items for one session, and restoring the second gives you the session without its files.
- **Cause:** The marker is written after the put, and its error is discarded. On recovery the folder is gone, so a new `<id>.folder` is trashed.
- **Fix:** Write the marker before the put, or look for the existing trash item on recovery, and return the write error.
- **Failing-first test:** `//apps/humata2/daemon/humatad:delete_sessions_cut_test`: cut between the put and the marker, restart, and assert there is one trash item.
- **Source:** V2 (10)

#### SMB-050
**Rebuild's switch-over writes in three separate transactions**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/session_service/rebuild.rs:698-713`.
- **What you see:** After a store failure mid-switch, the session lives on the target machine but still shows as archived, and the rebuild can't be retried.
- **Cause:** `put_session` commits and then `remove_archive` fails. A retry is refused as LostMachine (~:415), and `free_folder` skips the row because the session is elsewhere.
- **Fix:** Do the three writes in one store transaction.
- **Failing-first test:** `//apps/humata2/daemon/humatad:rebuild_session_test`: fail between `put_session` and `remove_archive`, and assert the row can still be settled.
- **Source:** V2 (10)

#### SMB-051
**The host deletes its record of a remote Desk start whose answer was lost**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/svc_desk/src/desk_service/remote.rs:302-312` (any `Err` removes the session row and drops the journal); `remote_asked.rs:306-327` (only `RelayError::Offline` retries); call at `daemon/svc_peer/src/remote_work.rs:927-931` (transport errors, `idempotency_key: None`).
- **What you see:** A Desk session started on another machine shows as failed and vanishes from the host, while its agent runs on that machine.
- **Cause:** The machine creates the session and starts the agent before it answers. If the link drops or the call times out after that, the error is a transport error, not `Offline`. Both the first try and the re-ask path treat it as "the machine won't run it", so they remove the host row and drop the journal that would have re-asked under the same id. The machine would answer a re-ask correctly (`known_start`), but the host never sends one.
- **Fix:** Remove the row only on codes that mean "not started" (InvalidArgument, FailedPrecondition, Unimplemented, AlreadyExists). On transport errors (Unavailable, DeadlineExceeded, Cancelled, Unknown), keep the journal and re-ask once linked, as the Offline branch does.
- **Failing-first test:** `//apps/humata2/daemon/svc_desk:svc_desk_unit_test`: a fake `start_desk_on` returns Unavailable on the first call and success on the second. Assert the session row and journal survive the first, and the row exists after the second.
- **Source:** V3 (11)

#### SMB-052
**Desk close records the agent Stopped and forgets its tabs even when the process survived**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_desk/src/desk_service/close.rs:264-279` (`stop_agent`), `:294-308` (`stop_shells`, which also does `remove_tab`).
- **What you see:** Closing a Desk session reports success while its agent or shell may still run, with no tab or record pointing at it. Only the orphan list shows it.
- **Cause:** Every `shut_or_end` error is only logged: `ListProcesses` (ps failed, so nothing was signalled), `Survived`, or `ProgramSurvived`. The code then forgets the tab, sets `Stopped` and, for shells, deletes the tab row. The error's own text says the process still runs and must be ended by hand, but you never see it.
- **Fix:** Return the error (at least `ListProcesses` and `Survived`) from `stop_agent` and `stop_shells`, so the close fails with that message. Keep the record Running and the tab row until the holder is gone.
- **Failing-first test:** `//apps/humata2/daemon/svc_desk:svc_desk_unit_test`: a holder-end fake returns `Survived`. Assert close returns an error and the agent stays Running with its tab row kept.
- **Source:** V3 (11)

#### SMB-053
**Remote clean-up reports failure, and skips the rest, after the machine already deleted**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_desk/src/desk_service/remote_close.rs:163-176` (`?` on `settle_remote` after `Deleted`, and on `clean_up_desk` itself if the link drops after the machine acted); `daemon/svc_desk/src/desk_service/clean_up.rs:182-187` (`?` aborts every later machine and all local deletes).
- **What you see:** Clean up says it failed, although the machine's sessions went to its Bin. Local sessions in the same clean up are not deleted, and the Bin items it made aren't shown.
- **Cause:** `settle_remote` makes a second call (`list_desk_sessions`) after the deletion. If that fails, the whole operation returns Err and the counts and trash items are dropped. The host rows heal later through the sweep (clean_up.rs:244), but you saw a failure.
- **Fix:** In `clean_up_there`, log a failed settle and still return `Gone`. In `clean_up`, collect per-machine errors, carry on with the other machines and local ids, and report the partial result.
- **Failing-first test:** `//apps/humata2/daemon/svc_desk:svc_desk_unit_test`: a fake remote answers `Deleted` to `clean_up_desk` and fails `list_desk_sessions`. Assert clean up returns the deleted count and still deletes a local closed session in the same request.
- **Source:** V3 (11)

#### SMB-054
**Desk start marks the failed step by error code, not by the step that was running**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_desk/src/desk_service/start.rs:154-159`; codes from `daemon/svc_desk/src/desk_service.rs:692-722` and `ports.rs:580-590`; `daemon/humatad/src/operations.rs:653-661` (`fail` leaves Running steps alone).
- **What you see:** A Desk start that fails while saving (a bad repo name, or a repo folder that already exists) shows "Starting the agent" as failed and "Saving the session" spinning for ever.
- **Cause:** Every non-`Internal` code marks "launch", including errors raised while writing the record (BadRepoName, RepoExists, CloneFailed). An `Internal` error during launch would mark the record step instead. `fail` never moves the Running step.
- **Fix:** Remember the last step `note` set Running and fail that one. Have `Progress::fail` turn any still-Running step into Failed or Waiting, as `end_cancelled` does.
- **Failing-first test:** `//apps/humata2/daemon/svc_desk:svc_desk_unit_test`: start with an invalid repo name. Assert `write_record` is Failed and no step is Running in the final operation.
- **Source:** V3 (11)

#### SMB-055
**The host's remote Desk start writes three rows with no transaction or rollback**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_desk/src/desk_service/remote.rs:244-259`.
- **What you see:** If the second or third write fails, a Desk session stays listed on the host with no detail or Desk row, and nothing runs it.
- **Cause:** `put_session`, `put_session_detail` and `put_desk`/`put_channel_desk` run one after another, not in a store transaction. The error branch fails the operation and drops the journal, but never calls `remove_session`. The local path (sessions_desk/src/desk.rs:404-407) does undo on the same failure.
- **Fix:** Wrap the three writes in one store transaction, or call `remove_session` in the error branch.
- **Failing-first test:** `//apps/humata2/daemon/svc_desk:svc_desk_unit_test`: a store that fails `put_session_detail`. Assert no session row remains.
- **Source:** V3 (11)

### 3.6 Upgrade, machines and transfers (svc_upgrade, svc_machines, svc_transfers, svc_peer)

#### SMB-056
**This Mac's daemon Migrate card is stuck at Migrating for good when a newer release is staged mid-migration**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/svc_upgrade/src/items.rs:521-523` (the update keeps `migrating` but stores the new `to`); `items.rs:965-968` (`arrived` only matches `known.to.bundle == bundle`); `items.rs:1290-1307` (`restored` rebuilds `migrating` and `to` from the card after the restart); `local.rs:200-210`.
- **What you see:** The Migrate card for this Mac's daemon says Migrating forever. Clicking does nothing, auto-migrate is skipped, and restarting doesn't fix it.
- **Cause:** Migrate to B1 swaps the files in and asks for a restart. If a check runs before the restart and sees B2 staged, the card's `to` becomes B2 with `migrating` still true. The daemon comes back at B1, and `arrived(B1)` returns early because `to` is B2. Approve refuses an item already Migrating, and the local carry has no deadline (items.rs:113), so nothing ends it. The window is short; the stuck state is permanent until the daemon reaches B2.
- **Fix:** When `to.bundle` changes, clear `migrating` and set the card to Waiting. Or have `arrived` settle a Migrating daemon item whenever the running bundle differs from the `from` it started on.
- **Failing-first test:** `//apps/humata2/daemon/svc_upgrade:svc_upgrade_unit_test`: observe daemon B0→B1, approve, observe B0→B2, then `arrived(machine, B1)`. Then observe B1→B2 and assert `is_migrating` is false and approve is accepted.
- **Source:** V3 (06)

#### SMB-057
**A failed holders move on this Mac loses its reason at the next look**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_upgrade/src/holders.rs:733` (the host's `look()` builds `Detail { failed: None }`); `items.rs:859`, `:879-890`, `:1510-1530` (`show_holders` writes `last_error: detail.failed`); `items.rs:932-958` (`fail_holders`).
- **What you see:** After a holder move fails on this Mac, the Migrate card still says Failed, but the reason and the fix vanish a moment later.
- **Cause:** `Holders::fail` sets the card Failed with `last_error`. `move_in_background` then wakes `look()`, whose rows differ from the last look, so the update path runs. `show_holders` keeps phase Failed but overwrites `last_error` with `None`, and `Pending.failed` goes false. Remote machines don't hit this, because their report carries the failure.
- **Fix:** Fill `Detail.failed` in the host's `look()` from a stored failure (set in `fail`, cleared in `begin`), or have `show_holders` keep `last_error` while the phase stays Failed.
- **Failing-first test:** `//apps/humata2/daemon/svc_upgrade:svc_upgrade_unit_test`: `observe_holders` (migrating), `fail_holders(problem)`, then `observe_holders` with changed rows and `failed: None`. Assert the card still has `last_error == problem` and `is_migrating` is false.
- **Source:** V3 (06)

#### SMB-058
**A remote daemon or CLI Migrate that is slow to answer, or whose link drops, shows Failed and trips the breaker**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_upgrade/src/remote.rs:191` (`self.reach.upgrade(...).await?` on the daemon/CLI/tools path; compare `:167-175`, where the holders path forgives `Slow` and dropped links); `remote.rs:309-316` (30 s `CALL_DEADLINE`); `items.rs:699-722` (Err sets Failed and trips the breaker).
- **What you see:** Migrate on a slow remote machine shows "Failed" and stops auto-migrate, although the machine finishes the upgrade and comes back on the new release.
- **Cause:** The machine verifies, records and swaps the files before it answers, then restarts. If that takes over 30 s, or the link drops as the daemon restarts, the host gets `Slow` or `Refused`, marks the item Failed and trips the breaker. The later arrival settles the item, but the false Failed card and the blocked auto-migrate show in the meantime.
- **Fix:** Treat `Slow`, and `Refused` where the link dropped, as success on the daemon path too: keep the item Migrating and let the 60 s `expect_by` deadline or the arrival settle it.
- **Failing-first test:** `//apps/humata2/daemon/svc_upgrade:svc_upgrade_unit_test`: a fake `Reach` that stages correctly and returns `RemoteError::Slow` from `upgrade`. Approve a daemon item and assert it is still Migrating and the breaker is not tripped.
- **Source:** V3 (06)

#### SMB-059
**The peer reporter stops for the life of the link after one failed subscribe or listing**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_peer/src/peer.rs:1062-1092`, `:1131-1135` (`return` on subscribe, list or Resync-list failure); `peer.rs:464-476` (the uplink keeps running when the reporter task ends).
- **What you see:** After a brief hiccup on a machine, the host's session list for that machine goes stale (no status changes) until the link reconnects, and nothing says so.
- **Cause:** `report_sessions` runs as its own task, and every failure path, and the feed ending, just returns. The `select!` waits only on `carry_uplink` and `ending`, so the uplink stays up with no reporter and nothing retries.
- **Fix:** When `report_sessions` returns, end the uplink (a third `select!` arm) so the host redials and resyncs, or retry with a bounded backoff.
- **Failing-first test:** `//apps/humata2/daemon/svc_peer:svc_peer_unit_test`: a fake socket daemon whose first `Subscribe` returns Unavailable. Open an uplink and assert it ends, or that a session change made after the failure still reaches the host.
- **Source:** V3 (06)

#### SMB-060
**The upload idempotency key is checked and stored under two separate locks**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `daemon/svc_transfers/src/transfer_service.rs:668-669` (`started_by` takes and drops `keys`), `:699-707` (`begin_upload`, then `keys.insert`). The copy path at `:505-517` holds `keys` across both.
- **What you see:** Two simultaneous uploads with the same key write the file twice, and the key points at only one of the two transfers.
- **Cause:** Check, start and insert happen under different locks, so two concurrent calls both miss. A retry after a broken stream is sequential and finds the key, so only true concurrent duplicates hit this.
- **Fix:** Hold the `keys` lock across the check, `begin_upload` and the insert, as `copy` does, or reserve the key first.
- **Failing-first test:** `//apps/humata2/daemon/svc_transfers:svc_transfers_unit_test`: from two threads behind a barrier, call `begin_upload` with the same key. Assert one returns `Began::Existing` with the other's id, and the engine lists one transfer.
- **Source:** V3 (06)

#### SMB-061
**Unanswered forget records can be trimmed out by later answers**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE (practically unreachable)
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `daemon/svc_machines/src/machine_service/forget.rs:224-245` (`keep` drains the oldest past `ANSWERS_KEPT = 256`, `:29`), called from `begin` (`:375`).
- **What you see:** In theory, a forget interrupted by a crash leaves its sessions unarchived, and nothing finishes it on restart.
- **Cause:** The trim ignores `done`, so an unfinished record can be dropped. It needs 256 other forgets to finish while one is in flight.
- **Fix:** Trim only `done == true` entries.
- **Failing-first test:** `//apps/humata2/daemon/svc_machines:svc_machines_unit_test`: `begin` key A, then `remember` 256 other keys. Assert `unanswered` still holds A.
- **Source:** V3 (06)

#### SMB-062
**Transfers keep the old link generation after a machine is forgotten and re-added**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_peer/src/host_relay.rs:144-150` (`forget` removes the slot, so generations restart at 1); `daemon/svc_transfers/src/remote_end.rs:194-205`, `:261` (`forget_machine` clears `failed_on`, but nothing outside tests calls `TransferService::forget_machine` at transfer_service.rs:230; `forget_link` at svc_machines machine_service/mod.rs:211 only calls `relay_links.forget`); `remote_end.rs:175-191`.
- **What you see:** You forget a machine and add it back under the same name. A transfer that fails offline just as the link returns stays "waiting for the machine" instead of going on by itself.
- **Cause:** `failed_on[machine]` keeps the old link's high generation (say 3), while the re-added machine's links count from 1, so `relinked_since_failure` stays false until the count passes 3. The main link-up resume path is unaffected.
- **Fix:** Call `TransferService::forget_machine` from the forget and remove paths, or keep link generations increasing across `forget`.
- **Failing-first test:** `//apps/humata2/daemon/svc_peer:svc_peer_unit_test`: `up` "devbox" 3 times, `forget`, `up` again, and assert the new generation is greater than 3. Or in `//apps/humata2/daemon/svc_transfers:svc_transfers_unit_test`.
- **Source:** V3 (06)

#### SMB-063
**Changing the Humata folder saves the older roots before the root, with no rollback**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_machines/src/machine_service/change_root.rs:586-611` (`switch_for_new_channels`); read back at `:151-176` (`start_layout`).
- **What you see:** If the second save fails and you restart without retrying, a folder that still holds channels can drop off the older-roots list, so those channels aren't found.
- **Cause:** `save_older_roots` succeeds and `save_this_root` fails, while memory stays unchanged. Switching back to an older root B saves older = [A, …] without B, while the root stays A. After a restart, channels under B aren't looked up. A retry in the same run writes both correctly.
- **Fix:** Write the root first and roll back on failure, or write both settings in one store transaction.
- **Failing-first test:** `//apps/humata2/daemon/svc_machines:svc_machines_unit_test`: a store whose second `set_setting` fails. Switch A→B where B was an older root, then `start_layout` from the store, and assert B is still in `older_roots()`.
- **Source:** V3 (06)

### 3.7 Inbox, board, channels and agent switches (svc_catalog, svc_away, svc_inbox, svc_board, channel_service)

#### SMB-064
**A damaged agents.toml turns every agent back on**
- [ ] **Fixed in:**
- **Severity / verdict:** P0, CONFIRMED (permission fail-open)
- **Slot:** SMFIX-P0 (before WEBTODO)
- **Where:** `daemon/svc_catalog/src/agent_gate.rs:100-119` (`switches` returns the defaults on any read or parse error), `:122-128` (`enabled`); `daemon/runtime/src/agent_switches.rs:110-117` (`enabled` defaults to `true`), `:73-79` (`deny_unknown_fields`). Callers that trust it: `daemon/humatad/src/session_service.rs:505`, `:628` (the host's check before it relays a remote start), `:663`; `session_service/fork.rs:381`; `session_service/remote_review.rs:116`; `daemon/svc_desk/src/desk_service/start.rs:88`, `:232`; `channel_desk.rs:157`, `:534`. Listing: `daemon/svc_catalog/src/agent_service.rs:159-176`.
- **What you see:** You turn Claude off. Later the file stops parsing, and from then on Humata offers Claude in the pickers again and starts it, with no warning in the app.
- **Cause:** On any read or parse error (permission denied, a hand-edit typo, or an unknown key such as `asks_in_inbox` reaching a machine on an older daemon, because `KindSwitch` denies unknown fields), `switches()` logs once and returns the defaults, so `enabled()` is true for every kind. Every new start, fork, review and Desk start then passes, including remote starts. `list_agents` uses the same defaults, so the disabled kind reappears in pickers. The only signal is one daemon log line. Only `enabled` fails open: `install_when_missing` and `asks_in_inbox` fail closed. Writes are atomic and shipped copies are checksummed, so the triggers are hand-edits, permissions and version skew, not torn writes.
- **Fix:** Make a bad file fail closed. `enabled`/`check` return a new `GateError::BadSwitches { source }` for every kind except `zsh`, with a fix line naming the file. `list_agents` reports the same error so Settings › Agents shows "fix or remove agents.toml". Optionally keep the last good parse in memory and honour its `enabled = false` entries while the file is bad.
- **Failing-first test:** `//apps/humata2/daemon/svc_catalog:svc_catalog_unit_test`, in agent_gate.rs tests: `a_bad_switches_file_refuses_agents_but_not_the_shell`. Use `rig(dir, "[agents.fake]\nenabled = false\nbogus = 1\n")`, then `"not toml ["`, then a file at mode 000. Assert `enabled(&agents, "fake")` is `Err`, `enabled(&agents, "zsh")` is `Ok`, and `ready(&agents, "fake")` is `Err`. All three return `Ok` today.
- **Source:** V3 (07)

#### SMB-065
**Rename recovery that can't finish the move never resumes the agents it stopped**
- [ ] **Fixed in:**
- **Severity / verdict:** P0, PLAUSIBLE (agents left stopped against "terminals keep running"; needs a store failure)
- **Slot:** SMFIX-P0 (before WEBTODO)
- **Where:** `daemon/humatad/src/channel_service/rename_folder.rs:1005-1013` (returns before `resume_all` at `:1024`).
- **What you see:** After a crash during a channel rename, the agents the rename stopped stay stopped. Only the log says so.
- **Cause:** If `finish_move` fails during recovery, `recover_one` logs and returns before `self.resume_all(&intent.resume)`. The agents stay stopped until a later start succeeds.
- **Fix:** Call `resume_all(&intent.resume)` on the Err branch too, keeping the note, or mark those sessions Stuck with a resume reason so the restart report shows them.
- **Failing-first test:** `//apps/humata2/daemon/humatad:channel_rename_live_test`: fault `finish_move` during recovery, and assert the agent is Running after the restart.
- **Source:** V2 (10)

#### SMB-066
**After a machine reconnects, its journal never reaches the host if the Resume ack failed to save**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/svc_away/src/uplink.rs:154-163` (`continue` before `resumed = true`), `:303-339` (`acked` returns false when `journal::acked` fails); host side `daemon/svc_peer/src/peer_client.rs:745-756` (Resume sent once at link-up), `:545-547`.
- **What you see:** After a machine reconnects, the asks and answers its agents made while the host was away never reach your Inbox, and the host gets no holders report, until the link drops and redials.
- **Cause:** If storing the ack fails (store busy, disk full), the Resume branch hits `continue`, and the journal and holders arms stay disabled for the rest of the link. The host sends Resume only once at link-up, so nothing re-sends it, and the machine stays silent while the link looks Up. The ack is only bookkeeping, so blocking the send buys nothing.
- **Fix:** Set `resumed = true` and send the journal whether or not the ack was kept. The next Ack retries it, which `acked` already does.
- **Failing-first test:** `//apps/humata2/daemon/svc_away:svc_away_unit_test`: make `journal::acked` fail (read-only or failing store), send a Resume then a journal write, and assert a `Journal` message reaches `to_host`.
- **Source:** V3 (07)

#### SMB-067
**Deleting a board item leaves later asks about it Pending after one save fails**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `daemon/svc_inbox/src/inbox/deferred.rs:487-515` (`withdraw_about_item`, `?` on `commit`); caller `daemon/svc_board/src/lib.rs:1091-1103` (logs and still reports the delete applied); compare `inbox.rs:449-480`.
- **What you see:** After a delete, the Inbox can still show "approve this change" cards for the deleted item, and approving one runs a write that fails because the item is gone.
- **Cause:** The first failed save returns before the remaining entries are withdrawn, and the caller only logs. `withdraw_pending_where` carries on past failures; this loop doesn't. In both, the entry whose save failed stays Pending.
- **Fix:** Collect the first error and continue. Better, implement it as `withdraw_pending_where(ItemDeleted, |e| about(e, item))`.
- **Failing-first test:** `//apps/humata2/daemon/svc_inbox:svc_inbox_unit_test`: two deferred board asks about item K, the store fails the first `put_inbox_request`, call `withdraw_about_item(K)`, and assert the second is Withdrawn.
- **Source:** V3 (07)

#### SMB-068
**Queued archive and restore jobs for a closed to-do are never run after a restart**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_board/src/automation.rs:255-313` (`after_close` queues `archive_pending`), `:32-34` (`is_pending`, hidden at todo.rs:863 and :1095), `:406-429` (`settle_key`); callers are request paths only (todo.rs:1642, :1659, :2165, :2218; automation.rs:370) and pr_watcher.rs:153, which skips Closed items (pr_watcher.rs:105-111).
- **What you see:** The daemon restarts (an upgrade, say) just after you close a to-do set to Auto archive. Its sessions are never archived, and nothing on the card says so.
- **Cause:** The write saves a hidden pending row and returns; `settle_key` runs after it. Nothing sweeps those rows at startup, and the PR watcher skips closed items. Only a later write on that same closed item runs them.
- **Fix:** At startup, and on the session tick, call `settle_key` for every item whose fields hold a pending row.
- **Failing-first test:** `//apps/humata2/daemon/svc_board:svc_board_unit_test`: write fields with an `archive_pending` row directly, build a fresh `BoardSvc` with a fake `SessionControl`, run the startup hook, and assert `archive_if_clean` was called and the row replaced.
- **Note:** WEBTODO exposes closing to-dos on the web. Consider doing this before WEBTODO.
- **Source:** V3 (07)

#### SMB-069
**Two concurrent settles run the same queued archive or restore**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_board/src/automation.rs:406-429` (`settle_key`), `:470-488` (`queued_work` reads without claiming), `:491-505`, `:519-536` (`finish_jobs` rewrites the shared `sessions-archived:{key}` prompt).
- **What you see:** You press Undo on "Archived 2 sessions" while another write to that closed item lands. The card can flip to "No sessions archived" or "Couldn't restore" and lose its Undo, though the sessions are actually restored.
- **Cause:** `queued_work` reads the pending rows outside the `writes` lock and marks nothing as taken, so two settles both run the jobs, and the second's result overwrites the first's card.
- **Fix:** Claim jobs under `self.writes` by renaming the row to an in-flight kind, and finish or put it back in `finish_jobs`.
- **Failing-first test:** `//apps/humata2/daemon/svc_board:svc_board_unit_test`: a fake `SessionControl` whose `restore` waits on a barrier, two `settle_key` for one key under `tokio::join!`. Assert `restore` was called once per session.
- **Source:** V3 (07)

#### SMB-070
**Closing a to-do saves the column before the fields**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_board/src/todo.rs:1561-1588` (`change_board_item`, then `saved::save` at `:1587`).
- **What you see:** After a failed save or a crash mid-write, the to-do shows Closed, but its closed time and its auto-archive never happen, and the app says the close failed.
- **Cause:** The board row and the fields are separate store writes. If the second fails, the column change is already committed. `closed_at_ms`, the pending archive row and the blocker-gone notices are lost, and a retry with the old revision conflicts.
- **Fix:** Do both writes in one store transaction. Failing that, when the fields save fails, return the applied item with a warning.
- **Failing-first test:** `//apps/humata2/daemon/svc_board:svc_board_unit_test`: inject a failure into the fields write after `change_board_item`, close an item with Auto archive, and assert item and fields agree (both unchanged, or both Closed with the pending row).
- **Source:** V3 (07)

#### SMB-071
**A channel delete whose carried move's folder vanished fails at every start**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `daemon/humatad/src/channel_service/delete.rs:465-470` (replayed at ~1070-1094).
- **What you see:** A channel whose delete was cut short never goes away, and the same error is logged at each start.
- **Cause:** The note's moves are replayed without being re-planned, and a folder deleted outside Humata gives NotFound, which aborts the delete.
- **Fix:** Treat a vanished folder as nothing to move.
- **Failing-first test:** `//apps/humata2/daemon/humatad:channel_delete_live_test`: cut after the note, remove a folder by hand, restart, and assert the channel is gone.
- **Source:** V2 (10)

### 3.8 Holder, terminal and files (daemon/holder, svc_terminal, revive, svc_files)

#### SMB-072
**The freeze/thaw race can leave an agent paused with nothing left to resume it**
- [ ] **Fixed in:**
- **Severity / verdict:** P0, PLAUSIBLE (terminal left stopped against "terminals keep running")
- **Slot:** SMFIX-P0 (before WEBTODO)
- **Where:** `daemon/holder/src/server.rs:248-298` (freeze: the first process snapshot at 258 before `frozen` is set at 275; the second snapshot at 281 before `frozen.extend(late)` at 289-296), `:304-313` (thaw), `:1093-1101` (handlers). Trigger: `daemon/humatad/src/session_service/move_session.rs:178` (15 s `FREEZE_WAIT` timeout), then `:941-951` (`freeze_old` thaws at once) and `:1271-1273` (undo thaws again).
- **What you see:** After a move whose pause timed out, the old agent stays frozen. Typing into it is refused or does nothing, and Humata shows it as running and not paused.
- **Cause:** The holder's freeze has two await points where it holds no claim. (a) A thaw during the first snapshot finds nothing frozen, then freeze marks `frozen` and stops every process group, so input stays refused. (b) A thaw during the second snapshot resumes the first groups and clears `frozen`, then the late groups get stopped and are recorded nowhere. The daemon reaches this when its freeze call times out: it drops its side, but the holder keeps running `freeze()`, and both thaws answer "not frozen". Nothing else asks the holder whether it is frozen, so no later pass notices. Only ending the terminal gets out.
- **Fix:** Under the `frozen` lock, mark a freezing generation before the first snapshot. Thaw bumps or clears it, and freeze re-checks it under the lock before each stop batch, returning without stopping if it changed. Record the late groups in `frozen` before signalling them.
- **Failing-first test:** `//apps/humata2/daemon/holder:freeze_test` (tests/freeze.rs): make the snapshot slow (a faults hook, or a fake `ps` on PATH that sleeps). Send Freeze on one connection and Thaw on another during the sleep. Assert the program still runs (the fake agent echoes typed input) and `is_frozen` is false.
- **Source:** V3 (08, 11; merged)

#### SMB-073
**Output saving stops for good after one failed write, and the message suggests freeing space fixes it**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, CONFIRMED
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/holder/src/saved.rs:369-380` (`output` sets `failed` and never clears it), `:334-341`, `:362` (`failed` carried across take-over), `:429` (sync also skipped).
- **What you see:** After a brief disk-full, a terminal's output is never saved again, even once space is free, so the next restart loses everything printed since.
- **Cause:** `failed` is sticky for the life of the holder and survives take-overs. Nothing retries. The message says "Free some disk space; the terminal itself goes on", but freeing space changes nothing.
- **Fix:** On later output, after a backoff, retry with a full rewrite of the log from the scrollback and clear `failed` on success. Otherwise change the message to say saving stays off until the terminal restarts.
- **Failing-first test:** `//apps/humata2/daemon/holder:holder_unit_test` (saved.rs): make the save fail once (read-only file, or a faults hook), then succeed. Assert output written after recovery is in the log `read_saved` returns.
- **Source:** V3 (11)

#### SMB-074
**A failed shell Stop leaves the stopped mark on a shell that still runs**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_terminal/src/running_holders.rs:589-608`.
- **What you see:** A shell whose Stop failed (a ps error, or a holder that survives SIGKILL) keeps running but carries the stopped mark, so after a daemon restart it isn't reattached (revive skips marked tabs, revive/src/reconcile.rs:146).
- **Cause:** The mark is written first on purpose and never removed when `find_holder` or `end_by_pid` errors. You do see the Stop error and can retry, so this is a leftover inconsistency, not a silent one.
- **Fix:** On any error after the mark is written, remove the mark unless `find_holder` now reports no holder.
- **Failing-first test:** `//apps/humata2/daemon/svc_terminal:svc_terminal_unit_test`: make `ps` fail (PATH without ps) with a live fake holder. Assert Stop errs and the mark file is absent.
- **Source:** V3 (11)

#### SMB-075
**Stopping a terminal whose holder already crashed leaves a program that ignores hang-up running**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/holder/src/end.rs:110-117`.
- **What you see:** Stop reports success, while a program that ignored SIGHUP keeps running.
- **Cause:** When the holder pid is no longer a holder, `end_by_pid` returns `Ok(())` before collecting groups. Those groups were reparented when the holder died, so `groups_under(pid)` can't find them anyway. The holder's death HUPs the foreground group, so only a program that ignores HUP survives.
- **Fix:** Persist the program's process group in the tab or launch record, and HUP/KILL it in the dead-holder case.
- **Failing-first test:** `//apps/humata2/daemon/holder:end_test` (tests/end.rs): spawn a holder whose program traps HUP, SIGKILL the holder, call `end_by_pid`, and assert the program's group is gone.
- **Source:** V3 (11)

#### SMB-076
**A stale `.handoff` marker after a failed take-over can hide a running session**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/holder/src/take_over.rs:349-409` (error returns before the marker removal at `:418`); `daemon/revive/src/reconcile.rs:335`, `:410-416`.
- **What you see:** Rarely, a session whose terminal runs fine is skipped on every reconcile and never shows as Running.
- **Cause:** A take-over error after exec leaves the marker. Revive removes it when the holder is dead (revive/src/report.rs:434-440), which covers the usual case. It survives only if something else (a Resume you clicked) starts a new holder on that socket first, and the record is then caught in Launching or Resuming at the next reconcile.
- **Fix:** Remove the marker on every take-over error path, and have the holder's normal `serve` start remove any marker beside its socket.
- **Failing-first test:** `//apps/humata2/daemon/revive:revive_unit_test`: with a live holder, a stale marker and a Resuming record, assert reconcile doesn't return `Left` for ever.
- **Source:** V3 (11)

#### SMB-077
**Moving several files to the Bin stops half-way and loses the ids of those it moved**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/svc_files/src/ops.rs:221-246` (the `?` on `trash_path` drops `moved`; the key is stored only on success at `:247-249`); adapter `daemon/humatad/src/wire_files.rs:86-103`.
- **What you see:** You delete three files and the third fails. You get an error, the first two are gone from the folder with no Undo, and a retry fails because they are no longer there. They can still be restored by hand from the Bin view.
- **Cause:** The moves run one by one with an early return. Ids already moved are discarded, and nothing is cached under the idempotency key.
- **Fix:** Collect per-entry results, return the moved ids with the error (or as a partial response), and cache that under the key so a retry returns it.
- **Failing-first test:** `//apps/humata2/daemon/svc_files:svc_files_unit_test`: a fake `FileTrashPort` fails on the third path. Assert the response or error carries the two ids, and a retry with the same key returns them.
- **Source:** V3 (08)

### 3.9 humatad core (config sync, history, settings, watch, start gate, FUSEFS)

#### SMB-078
**Composed Claude config homes drop the keychain login setting**
- [ ] **Fixed in:**
- **Severity / verdict:** P1, PLAUSIBLE
- **Slot:** SMFIX-P1 (before WEBTODO)
- **Where:** `daemon/humatad/src/contained.rs:268-273`; `wire_stop_restart.rs:288-305`; `daemon/config_compose/src/compose.rs:253-256` (`env_plan` is never read).
- **What you see:** A Claude session with a composed config home may start logged out on macOS.
- **Cause:** The plain path sets `CLAUDE_SECURESTORAGE_CONFIG_DIR=""` for a Keychain login (contain/src/claude.rs:58). Both composed branches set only `CLAUDE_CONFIG_DIR`. How Claude behaves at run time without it was not checked.
- **Fix:** Set the variable in both composed paths when the login is Keychain, or apply `env_plan`.
- **Failing-first test:** `//apps/humata2/daemon/humatad:humatad2_unit_test`: `own_state` and `ComposeEnv::env` for a composed Claude home contain the variable.
- **Note:** FUSEFSWIRE is wiring composed config homes and has `contained.rs` and `config_compose` open. Hand this to that lane, or land it right after FUSEFSWIRE closes.
- **Source:** V2 (09)

#### SMB-079
**Config sync keeps re-composing archived sessions' config homes**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, CONFIRMED
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/wire_stop_restart.rs:314-317`, `:341-347`; `config_sync.rs:324-326`, `:366-371`.
- **What you see:** Changing a sync setting rewrites archived sessions' config homes, and one broken archived home makes the Settings save report an error.
- **Cause:** `forget_session_home` has no callers. `apply` drops only sessions whose folder is gone, and returns the first recompose failure, which settings_service.rs:331 turns into an error reply.
- **Fix:** Call `forget_session_home` on archive and delete, and register again on restore.
- **Failing-first test:** `//apps/humata2/daemon/humatad:settings_sync_test`: archive, change sync, and assert the home is untouched.
- **Note:** Same files as FUSEFSWIRE; land after it closes.
- **Source:** V2 (09)

#### SMB-080
**The history follower skips session events when it falls behind, without catching up**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (trivial: do it early)
- **Where:** `daemon/humatad/src/history_service.rs:402`.
- **What you see:** Search can miss a stopped session's last turns after a burst of events.
- **Cause:** `Lagged` goes to `continue` with no new catch-up. The board's pump reseeds on Lagged; this loop doesn't.
- **Fix:** On Lagged, run the full store catch-up again.
- **Failing-first test:** `//apps/humata2/daemon/humatad:humatad2_unit_test`: overflow a small-buffer feed during catch-up, and assert every session is mirrored.
- **Source:** V2 (09)

#### SMB-081
**A Settings save that fails can still have changed the sync settings, with no revision bump**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/settings_service.rs:327-338`, `:366-371`.
- **What you see:** A Settings save shows an error, but the sync settings did change, and the app still thinks nothing changed.
- **Cause:** Sync is applied before the sections are saved. Sections are validated first, so only an IO error in `places.save` after `sync.apply` triggers this. This is not the FUSEFS switch.
- **Fix:** Save the sections before applying sync, or bump the revision whenever sync was applied.
- **Failing-first test:** `//apps/humata2/daemon/humatad:settings_sections_test`: with an unwritable sections file, either sync is unchanged or the revision moved.
- **Note:** FUSEFSWIRE is editing `settings_service.rs`; land after it closes.
- **Source:** V2 (09)

#### SMB-082
**The agent watch doesn't reattach after a live holder hangs up on it**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/session_service/watch.rs:73-87`.
- **What you see:** An agent that exits after its watch dropped still shows Running until the next daemon start.
- **Cause:** Documented as left to the next reconcile. Rare, because the holder catches slow clients up rather than dropping them.
- **Fix:** Reattach with backoff while the record is Running and the pid is alive.
- **Failing-first test:** `//apps/humata2/daemon/humatad:humatad2_unit_test`: drop the watch socket while the holder lives, exit the agent, and assert the state is Exited.
- **Source:** V2 (09)

#### SMB-083
**Sends to a session stay blocked after its start card is denied or withdrawn**
- [ ] **Fixed in:**
- **Severity / verdict:** P2, PLAUSIBLE
- **Slot:** SMFIX-P2 (after WEBTODO unless trivial)
- **Where:** `daemon/humatad/src/start_ready.rs:236-245`, `:84-95`.
- **What you see:** After you Deny the "waiting in its terminal" card, every send to that session waits 30 s and is then refused.
- **Cause:** After a Deny or withdraw, the hold clears only when the agent is heard or exits.
- **Fix:** On Deny or withdraw, release the sends, or raise the card again.
- **Failing-first test:** `//apps/humata2/daemon/humatad:humatad2_unit_test`: deny the card, and assert `behind_start` returns at once.
- **Source:** V2 (09)

#### SMB-084
**FUSEFS helper death leaves the mount flag on (in flight)**
- [ ] **Fixed in:**
- **Severity / verdict:** IN FLIGHT (not re-verified)
- **Slot:** FUSEFSWIRE 0.10.0 (in flight)
- **Where:** `daemon/humatad/src/fusefs.rs:135-146`.
- **What it is:** When the helper exits, the reaper only logs. `mounted` stays true, new agents get `HUMATA_FUSE_ROOT` pointing at an unmounted tree, nothing respawns the helper, and Settings still shows the mount on.
- **Source:** 09 (finder); VERIFY.md in-flight list

#### SMB-085
**One failed FUSEFS topology push turns pushes off (in flight)**
- [ ] **Fixed in:**
- **Severity / verdict:** IN FLIGHT (not re-verified)
- **Slot:** FUSEFSWIRE 0.10.0 (in flight)
- **Where:** `daemon/humatad/src/fusefs.rs:263-283`.
- **What it is:** A single failed push sets `live = false`, and later pushes are skipped, so the helper's view of sessions and machines goes stale while the mount still looks healthy, until the switch is flipped off and on.
- **Source:** 09 (finder); VERIFY.md in-flight list

#### SMB-086
**Rapid FUSEFS on/off toggles can end in the wrong state (in flight)**
- [ ] **Fixed in:**
- **Severity / verdict:** IN FLIGHT (not re-verified)
- **Slot:** FUSEFSWIRE 0.10.0 (in flight)
- **Where:** `daemon/humatad/src/fusefs.rs:239-254`.
- **What it is:** Each `set_enabled` spawns its own thread, and the lock isn't first-come-first-served, so "on" can run after "off" and leave the mount on while settings.toml says off.
- **Source:** 09 (finder); VERIFY.md in-flight list

#### SMB-087
**Settings shows only the saved FUSEFS flag, not the live mount (in flight)**
- [ ] **Fixed in:**
- **Severity / verdict:** IN FLIGHT (not re-verified)
- **Slot:** FUSEFSWIRE 0.10.0 (in flight)
- **Where:** `daemon/humatad/src/settings_service/sections.rs:458-460`.
- **What it is:** Mount failures are only logged, and Settings reads the saved flag, so the switch shows on while nothing is mounted.
- **Source:** 09 (finder); VERIFY.md in-flight list

---

## 4. Slots

Each bug is in exactly one slot. This table is the master list: when a fix lands, put its commit in **Fixed in**, tick the entry's box in section 3, and move the item if its slot changes.

| Slot | When | Items |
|---|---|---|
| `MACHINEARCHIVE 0.10.2` | with NEWCHCLOSE, after FUSEFSWIRE ([milestone](milestones/MACHINEARCHIVE.md)) | 9 |
| `NEWCHCLOSE 0.10.2` | after FUSEFSWIRE ([milestone](milestones/NEWCHCLOSE.md)) | 1 |
| `FUSEFSWIRE 0.10.0 (in flight)` | the running lane ([milestone](milestones/FUSEFSWIRE.md)) | 5 |
| `SMFIX-P0 (before WEBTODO)` | before WEBTODO | 3 |
| `SMFIX-P1 (before WEBTODO)` | before WEBTODO | 13 |
| `SMFIX-P2 (after WEBTODO unless trivial)` | after WEBTODO; the ones marked trivial can go any time | 56 |

| ID | Sev | Verdict | Bug | Slot | Fixed in |
|---|---|---|---|---|---|
| SMB-001 | P1 | CONFIRMED | New channel sheet waits for the clone; slug mismatch | NEWCHCLOSE 0.10.2 | |
| SMB-002 | P1 | CONFIRMED | Archive errors invisible on Machine, Files, PR pages | MACHINEARCHIVE 0.10.2 | |
| SMB-003 | P1 | CONFIRMED | Events reconnect tight loop | SMFIX-P1 | |
| SMB-004 | P1 | CONFIRMED | Esc on create sheets throws away the error | SMFIX-P1 | |
| SMB-005 | P2 | CONFIRMED | Late session create closes a fresh sheet | SMFIX-P2 | |
| SMB-006 | P2 | PLAUSIBLE | Store bridge misses fields panes draw | SMFIX-P2 | |
| SMB-007 | P2 | CONFIRMED | Failed UI save not retried | SMFIX-P2 | |
| SMB-008 | P2 | PLAUSIBLE | Restart Try again sent twice | SMFIX-P2 | |
| SMB-009 | P2 | CONFIRMED | Repo scope reverted after Save as default | SMFIX-P2 (trivial) | |
| SMB-010 | P2 | PLAUSIBLE | Toast cap evicts Undo | SMFIX-P2 (trivial) | |
| SMB-011 | P1 | CONFIRMED | Desk Cancel drops start/clean-up/save failure | SMFIX-P1 | |
| SMB-012 | P1 | CONFIRMED | Channel Desk close sheet stuck | SMFIX-P1 | |
| SMB-013 | P2 | PLAUSIBLE | End-for-good flag survives Open | MACHINEARCHIVE 0.10.2 | |
| SMB-014 | P2 | PLAUSIBLE | Close-tab flow acts on whichever session is open | MACHINEARCHIVE 0.10.2 | |
| SMB-015 | P2 | CONFIRMED | Second agent tab overwrites pending prompt | SMFIX-P2 | |
| SMB-016 | P2 | CONFIRMED | Opening prompt dropped silently | SMFIX-P2 | |
| SMB-017 | P2 | PLAUSIBLE | Late rename clears another draft | SMFIX-P2 | |
| SMB-018 | P2 | CONFIRMED | Empty "Show later" hides the button for good | SMFIX-P2 | |
| SMB-019 | P2 | PLAUSIBLE | Stale conversation reply clears loading | SMFIX-P2 | |
| SMB-020 | P2 | CONFIRMED | Add machine stream ends with no retry | SMFIX-P2 | |
| SMB-021 | P2 | PLAUSIBLE | Add machine login stuck finishing | SMFIX-P2 | |
| SMB-022 | P2 | PLAUSIBLE | Hosts stopped check reads stale workspace | SMFIX-P2 | |
| SMB-023 | P2 | CONFIRMED | Mass delete plan fills the next sheet | SMFIX-P2 | |
| SMB-024 | P2 | CONFIRMED | PR Submit plan applies to the re-opened session | SMFIX-P2 | |
| SMB-025 | P2 | PLAUSIBLE | To-do overlapping writes show a stale copy | SMFIX-P2 | |
| SMB-026 | P2 | CONFIRMED | Refused Keep hides the view | SMFIX-P2 (trivial) | |
| SMB-027 | P2 | CONFIRMED | Repositories busy cleared during Remove | SMFIX-P2 | |
| SMB-028 | P2 | PLAUSIBLE | Providers import Cancel ignored | SMFIX-P2 (trivial) | |
| SMB-029 | P2 | PLAUSIBLE | Files listings out of order | SMFIX-P2 | |
| SMB-030 | P1 | CONFIRMED | Machine page shows archived sessions as ordinary rows | MACHINEARCHIVE 0.10.2 | |
| SMB-031 | P1 | CONFIRMED | End for good on Machine page ends nothing | MACHINEARCHIVE 0.10.2 | |
| SMB-032 | P1 | CONFIRMED | To-do and Pinned feeds die after daemon restart | SMFIX-P1 | |
| SMB-033 | P1 | PLAUSIBLE | Second subscriber gets no stream | SMFIX-P1 | |
| SMB-034 | P2 | PLAUSIBLE | Restart report shows again after failed read | SMFIX-P2 | |
| SMB-035 | P2 | PLAUSIBLE | Partial mass delete loses its count | SMFIX-P2 | |
| SMB-036 | P2 | PLAUSIBLE | Inbox cache keeps a stale pending request | SMFIX-P2 | |
| SMB-037 | P2 | PLAUSIBLE | Tab-list refreshes out of order | SMFIX-P2 | |
| SMB-038 | P2 | PLAUSIBLE | Typing a line waits forever | SMFIX-P2 (trivial) | |
| SMB-039 | P2 | PLAUSIBLE | Finished move still shown Moving | SMFIX-P2 | |
| SMB-040 | — | IN FLIGHT | Events subscribe Resync | FUSEFSWIRE 0.10.0 (in flight) | |
| SMB-041 | P2 | CONFIRMED | Exited agent's terminal never ended | MACHINEARCHIVE 0.10.2 | |
| SMB-042 | P2 | CONFIRMED | Archive refuses stale Launching/Resuming | MACHINEARCHIVE 0.10.2 | |
| SMB-043 | P2 | CONFIRMED | Folder free never retried | MACHINEARCHIVE 0.10.2 | |
| SMB-044 | P2 | PLAUSIBLE | Two archives of one session both run | MACHINEARCHIVE 0.10.2 | |
| SMB-045 | P1 | PLAUSIBLE | Restore races a free of the shared folder | SMFIX-P1 | |
| SMB-046 | P1 | PLAUSIBLE | Failed Start fresh leaves "being resumed" | SMFIX-P1 | |
| SMB-047 | P2 | CONFIRMED | Retried Stop withdraws no requests | SMFIX-P2 (trivial) | |
| SMB-048 | P2 | PLAUSIBLE | Two mass deletes with one token | SMFIX-P2 | |
| SMB-049 | P2 | PLAUSIBLE | Crash makes a second trash item | SMFIX-P2 | |
| SMB-050 | P2 | PLAUSIBLE | Rebuild switch-over not one transaction | SMFIX-P2 | |
| SMB-051 | P1 | PLAUSIBLE | Host drops a remote Desk start whose answer was lost | SMFIX-P1 | |
| SMB-052 | P2 | CONFIRMED | Desk close records Stopped though process survived | SMFIX-P2 | |
| SMB-053 | P2 | CONFIRMED | Remote clean-up reports failure after deleting | SMFIX-P2 | |
| SMB-054 | P2 | CONFIRMED | Desk start fails the wrong step | SMFIX-P2 | |
| SMB-055 | P2 | PLAUSIBLE | Remote Desk start writes three rows untransacted | SMFIX-P2 | |
| SMB-056 | P1 | PLAUSIBLE | Local daemon Migrate stuck at Migrating | SMFIX-P1 | |
| SMB-057 | P2 | CONFIRMED | Holders move failure loses its reason | SMFIX-P2 | |
| SMB-058 | P2 | PLAUSIBLE | Slow remote Migrate shows Failed | SMFIX-P2 | |
| SMB-059 | P2 | PLAUSIBLE | Peer reporter stops for the link | SMFIX-P2 | |
| SMB-060 | P2 | PLAUSIBLE | Upload key checked and stored under two locks | SMFIX-P2 (trivial) | |
| SMB-061 | P2 | PLAUSIBLE | Unanswered forget records trimmed | SMFIX-P2 (trivial) | |
| SMB-062 | P2 | PLAUSIBLE | Transfers keep old link generation | SMFIX-P2 | |
| SMB-063 | P2 | PLAUSIBLE | Change root saves in the wrong order | SMFIX-P2 | |
| SMB-064 | P0 | CONFIRMED | Damaged agents.toml turns agents back on | SMFIX-P0 | |
| SMB-065 | P0 | PLAUSIBLE | Rename recovery leaves agents stopped | SMFIX-P0 | |
| SMB-066 | P1 | PLAUSIBLE | Uplink journal never sent after failed ack | SMFIX-P1 | |
| SMB-067 | P2 | PLAUSIBLE | Board delete leaves asks Pending | SMFIX-P2 (trivial) | |
| SMB-068 | P2 | PLAUSIBLE | Queued archive/restore jobs not run after restart | SMFIX-P2 | |
| SMB-069 | P2 | PLAUSIBLE | Two settles run one queued job | SMFIX-P2 | |
| SMB-070 | P2 | PLAUSIBLE | Close saves column before fields | SMFIX-P2 | |
| SMB-071 | P2 | PLAUSIBLE | Channel delete fails on a vanished folder | SMFIX-P2 (trivial) | |
| SMB-072 | P0 | PLAUSIBLE | Freeze/thaw race leaves agent frozen | SMFIX-P0 | |
| SMB-073 | P1 | CONFIRMED | Output saving stops for good | SMFIX-P1 | |
| SMB-074 | P2 | PLAUSIBLE | Failed shell Stop leaves the stopped mark | SMFIX-P2 | |
| SMB-075 | P2 | PLAUSIBLE | Dead-holder Stop leaves program running | SMFIX-P2 | |
| SMB-076 | P2 | PLAUSIBLE | Stale handoff marker hides a session | SMFIX-P2 | |
| SMB-077 | P2 | CONFIRMED | Multi-file Bin move loses moved ids | SMFIX-P2 | |
| SMB-078 | P1 | PLAUSIBLE | Composed Claude homes drop keychain setting | SMFIX-P1 | |
| SMB-079 | P2 | CONFIRMED | Config sync re-composes archived homes | SMFIX-P2 | |
| SMB-080 | P2 | PLAUSIBLE | History follower skips on lag | SMFIX-P2 (trivial) | |
| SMB-081 | P2 | PLAUSIBLE | Failed Settings save still changed sync | SMFIX-P2 | |
| SMB-082 | P2 | PLAUSIBLE | Agent watch doesn't reattach | SMFIX-P2 | |
| SMB-083 | P2 | PLAUSIBLE | Sends blocked after denied start card | SMFIX-P2 | |
| SMB-084 | — | IN FLIGHT | FUSEFS helper death | FUSEFSWIRE 0.10.0 (in flight) | |
| SMB-085 | — | IN FLIGHT | FUSEFS push `live` flag | FUSEFSWIRE 0.10.0 (in flight) | |
| SMB-086 | — | IN FLIGHT | FUSEFS toggle race | FUSEFSWIRE 0.10.0 (in flight) | |
| SMB-087 | — | IN FLIGHT | Settings shows only the saved FUSEFS flag | FUSEFSWIRE 0.10.0 (in flight) | |

**Ordering notes.**
- SMB-033 and the subscriber leak in SMB-003 touch `clients/live_daemon/src/events.rs` `subscribe`, which FUSEFSWIRE is changing (SMB-040). Land them on top of it.
- SMB-078, SMB-079 and SMB-081 touch `contained.rs`, `config_compose`, `config_sync.rs` and `settings_service.rs`, which FUSEFSWIRE has open. Land them after it closes.
- SMB-004 touches `new_channel.rs`; land it after NEWCHCLOSE (SMB-001).
- SMB-025, SMB-032 and SMB-068 are on the To-do path that WEBTODO serves on the web. SMB-032 is in SMFIX-P1 already; consider pulling SMB-025 and SMB-068 forward too.
- SMB-006 is in the store bridge the GPUI work will touch; fix it there.

---

## 5. Rejected

| Bug | Where | Why rejected |
|---|---|---|
| Channel delete: a stale DeleteFinished closes another channel's dialog | `features/app/src/lib.rs:1415-1427` | `CancelDelete` refuses while deleting, and the modal covers the window, so a second delete can't open during the first. |
| Archive plan opens the confirm for a session that is no longer open | `features/session/src/lib.rs:1997-2007` | The confirm carries and shows its own session; archiving a session other than the open one is the intended Machine/Files/PR path. |
| Files: the low-disk dialog reappears after Dismiss | `features/files/src/manager/edit.rs:140-154` | `low_disk` is set only by `LowDiskReady`, so there is no dialog to dismiss while the storage read runs. |
| Continue-from turn list error goes to the page banner | `features/session/src/continue_in.rs:77-84` | The continue dialog does draw `session.error` (`ui/overlays/src/overlay_continue.rs:133-139`). |
| A retry that succeeds is reported as a failure after an app restart | `clients/live_daemon/src/daemon.rs:747-766` | Setups reach the app only through this run's `watch_setup`, so after an app restart there is nothing to retry. |
| Watched move keeps `moving` set (as the finder described it) | `clients/live_core/src/map.rs:451` | The daemon publishes the session after the move ends and the mapping clears it; the narrower real race is SMB-039. |
| Bind before fallible startup leaves the socket | `daemon/humatad/src/serve.rs:354` | The early bind is deliberate, and the next start's bind removes the stale socket. |
| Upgrade or relaunch skipped when serve errors | `daemon/humatad/src/serve.rs:1474-1500` | The non-zero exit makes launchd KeepAlive start the already-swapped binary. |
| Claude login refused needlessly for composed homes | `daemon/humatad/src/contained.rs:329-331` | The version check still matters for composed homes; the real bug is the missing env setting, SMB-078. |
| Stale Migrate timer stalls a newer attempt | `daemon/svc_upgrade/src/items.rs:773-810` | Approve refuses an item already Migrating and the Err branch starts no timer, so a second attempt can't start before the first timer fires. |
| Idempotency key returns a dead request forever | `daemon/svc_inbox/src/inbox/deferred.rs:548-557` | Each agent call gets a new key; returning the same result for the same key is correct. |
| Browser "Allow 30 min" stored as a one-time approval | `daemon/svc_inbox/src/inbox/browser.rs:40-49` | No browser card can exist yet (`browser_approval` is unimplemented); this is unbuilt BROWSER-lane work. |
| A thaw that fails after a failed freeze leaves the agent paused with no retry | `daemon/humatad/src/session_service/move_session.rs:941-950` | The move's failure runs undo, which thaws again and, if that fails, journals and retries once linked. |
| Desk move recovery drops the journal without unstaging when the channel differs | `daemon/svc_desk/src/desk_service/move_to_channel.rs:1139-1142` | A mismatch means the move already finished and released, and recovery runs before anything is served. |
| A remote start answer with no folder is saved without one | `daemon/svc_desk/src/desk_service/remote.rs:263-271` | The machine builds the answer with `view::desk_pb`, which always sets the folder. |

---

## 6. How to work this list

1. **Failing-first test per item.** Write the test named in the entry before touching the code, run it through `tools/bq/bq --fast <target>`, and see it fail for the reason the entry gives. If it passes, the bug isn't what the entry says: write what you found under the entry and move it to section 5 instead of fixing it. PLAUSIBLE items especially may turn out not to reproduce.
2. **Fix in the leaf crate.** Put the fix and its test in the crate that owns the state (the feature reducer, the client, the service), not in the app shell or a daemon-wide test. Prefer in-process `rust_test` targets over spawning a daemon, and pick the narrowest daemon test profile when you must spawn one (see AGENTS.md).
3. **Tick the item with its commit.** Tick the entry's box in section 3, put the commit hash in **Fixed in** there and in the section 4 table, and add the GAPS_REVIEW.md line as usual. One commit can close several items; list each.
4. **Keep the Slot column current.** If an item moves (a P2 that turns out trivial and goes early, a P1 pulled into a milestone, a PLAUSIBLE that is rejected), change its Slot in both the entry and the table in the same commit.
5. **Order.** FUSEFSWIRE finishes first. Then MACHINEARCHIVE and NEWCHCLOSE ship as 0.10.2. SMFIX-P0 and SMFIX-P1 go before WEBTODO. SMFIX-P2 waits until after WEBTODO, except the ones marked trivial, which can ride along with any nearby fix.

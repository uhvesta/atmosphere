# NEWCHCLOSE: Pressing Create channel closes the sheet at once and shows the new channel

- **Status:** queued — opens after FUSEFSWIRE closes (owner, 2026-11-20); ships as 0.10.2, after PROVIDERCLI (0.10.1)
- **Depends on:** FUSEFSWIRE (owner's order)
- **Unblocks:** none

## Bug (owner report, 2026-11-20, desktop app)
After I press "Create channel", the new channel appears in the left sidebar, but the New channel sheet stays open. It only closes about 10 seconds later, after something else happens.

**Expected:** the sheet closes as soon as the channel is created (the sidebar row appearing is the signal), and the clone progress (frame 45) carries on without blocking it.

## Exit criterion
- Root cause named (likely: the sheet waits for a later event, such as clone completion or the next refresh, instead of the create reply).
- A failing-first behavioural test: the TCA store test for the New channel feature, and a native test if the close is UI-driven, showing that the sheet closes on the create reply without waiting on later events.
- Fix lands; main's batched gate passes; shipped in the next patch release.

## Read these (pointers, nothing else)
- NEWCH.md (frames 44/45), features/ new-channel reducer, ui/ new-channel sheet, clients create-channel call.

## Out of scope
- Redesigning the sheet or the clone progress.

## Likely cause (state machine sweep finder 03, 2026-11-20; not yet verified)
- **Why it waits:** `clients/live_daemon/src/daemon.rs:247-301`. The live `create_channel` waits through `watch_setup`/`finish` for every clone step before it returns the id. The sidebar shows the channel earlier, after the `record` step arrives through a Resync. So the sheet stays open for the whole clone, after the channel is already listed.
  - Fix idea: return the id once `record` lands and let the setup progress carry on separately, or close the sheet as soon as the new channel appears in the sidebar list.
- **A second bug that makes it worse:** `features/app/src/new_channel.rs:290`. `heard_setup` takes setup events only when `sheet.folder() == setup.name`. But `folder()` returns the slug and `setup.name` is the name as typed, so a channel named something like "Q4 Billing" never gets progress or the early close.
  - The test name "billing-exports" is already a slug, which is why the test doesn't catch it. Add a test with a name that has spaces and capitals.

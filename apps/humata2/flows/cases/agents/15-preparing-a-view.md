# A prepared view is a draft until it is marked ready, then waits for Open or Later, and a restart keeps it

Layer: agents. Platforms: macOS for the offer bar and the view tab (the app); Linux where the host runs. The daemon side is the same on both. The offer bar and the Open and Later actions are not wired to the daemon yet (see Gaps), so those steps are shown with what the code does today.

## Starting state

- This Mac runs the host daemon. The machine is `this-mac`, and `box-1` is linked and online.
- The channel `acme` has the repo `acme/api`. The session `migration` runs on `this-mac` with an agent tab. The agent holds its session token.
- The agent has finished a database migration in `acme/api`. It wants to show me three files in order: `migrations/0042.sql`, `db/refund.rs` and `db/refund_test.rs`.
- No view exists yet for `migration`. The app is open on the `acme` channel, and the Journal of `migration` is empty.
- I have no open tab for a prepared view.

## Steps

1. **I do:** nothing in the app. The agent calls the MCP tool `view_prepare` with the title "Migration: read these three files", a summary "Read in this order", and a review walkthrough of three steps. Each step has a repo, a path, lines and a note.
   **I see:** the agent gets back a view id and the state `draft`. The app shows no offer, and the Journal stays empty.
   **State and events:** one draft view exists on the daemon. Only the agent can see it. Its revision is 1.
   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::an_agent_prepares_revises_and_offers_a_view` (state `draft`), and `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` (the user's list is empty for a draft).

2. **I do:** the agent calls `view_revise` with the new title "Migration, step by step" and `expected_revision` 1.
   **I see:** the agent gets back the new title, the same summary, and revision 2.
   **State and events:** the revision is 2. The parts not named keep their values.
   **Status:** Works (real daemon). `daemon/humatad/tests/mcp.rs::an_agent_prepares_revises_and_offers_a_view` and `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it`.

3. **I do:** the agent calls `view_mark_ready` on the view.
   **I see:** the agent gets the state `waiting`. The user's event feed gets the offer. The user's feed never carries the draft.
   **State and events:** the view is offered. It is now the user's to answer. Each feed gets one change. A repeated mark-ready changes nothing, and nobody is told again.
   **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only`. The MCP result is in `daemon/humatad/tests/mcp.rs::an_agent_prepares_revises_and_offers_a_view`.

4. **I do:** look at the `migration` session's page in the app.
   **I see (expected, VW-2):** an offer bar reads "agent in “migration” prepared a view for you: Migration, step by step", with the summary, and the buttons Open it, Later and Not now.
   **State and events:** the app shows one offer for this view, taken from the daemon.
   **Status:** Not built (VIEWS milestone, planning). The app's view offers come from a fixture keyed by title. No app crate uses the live views client `clients/live/src/views.rs`. The bar is covered only by the fake in `desktop/tests/native/session.rs::prepared_views_wait_for_open_and_later_returns_on_the_next_visit`.

5. **I do:** click Later.
   **I see (expected):** the bar goes away. The view waits for my next visit to the session.
   **State and events:** the view's state is set to Later on the daemon. The daemon keeps that state across a restart.
   **Status:** Fakes only for the app: `features/views/tests/offers.rs::returning_shows_a_deferred_offer_but_never_a_dismissed_one` (the Later set is in memory, keyed by title). The daemon's Later is Works (real daemon): `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it`.

6. **I do:** click the `rail-home` item, then the `migration` session again.
   **I see (expected):** the offer bar is back, with Open it, Later and Not now.
   **State and events:** the view is offered again on the return. Its state is still Later on the daemon until I answer.
   **Status:** Fakes only: `desktop/tests/native/session.rs::prepared_views_wait_for_open_and_later_returns_on_the_next_visit` and `features/views/tests/offers.rs::returning_shows_a_deferred_offer_but_never_a_dismissed_one`. Live, Not built (VIEWS).

7. **I do:** click Open it.
   **I see (expected, VW-6):** the view opens as a tab in the session's tab row. The left list shows the three steps, and the main area shows the first file with the agent's note above it.
   **State and events:** the view's state is set to Opened on the daemon. The agent is told of the change on its feed, and the user is told too.
   **Status:** Works (real daemon) for the state change and feeds: `daemon/humatad/tests/review.rs::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only` (Opened reaches the agent's feed and the user's). The tab and its contents are Fakes only: `desktop/tests/native/session.rs::prepared_views_wait_for_open_and_later_returns_on_the_next_visit` checks the tab appears, not the steps.

8. **I do:** restart the host daemon while the view is Later, then open the session.
   **I see:** the offer is still there after the restart, with the same title and summary.
   **State and events:** the view and its state survive the restart. Its id is unchanged. A repeated idempotency key after the restart makes no second view.
   **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it`.

9. **I do:** in a plain terminal, run `humata views` (no session named).
   **I see:** the list shows every offered view on this Mac, newest first, with its title and state. Drafts do not appear.
   **State and events:** a read only. No state changes.
   **Status:** Works (real daemon) for the order and the drafts: `daemon/humatad/tests/review.rs::the_user_lists_every_offered_view_newest_first_without_naming_a_session`. The CLI output itself is Fakes only: `cli/tests/views.rs::views_with_no_session_asks_for_every_session_and_prints_them`.

10. **I do:** run `humata view not-now <view id>` (the same as clicking Not now).
    **I see:** the command prints the view with the state Not now and exits 0.
    **State and events:** the view's state is Not now on the daemon.
    **Status:** Code, no test for the daemon side of Not now. The state is accepted by `daemon/humatad/src/review/views.rs::set_state`, and no daemon test sets it. The CLI's mapping is Fakes only: `cli/tests/views.rs::each_view_action_sets_its_state_once`.

11. **I do:** read the agent's event feed after the steps above.
    **I see (expected):** the agent's feed has its draft as created, then the offer, then my answer (Opened). The user's feed has the offer and my answer, and never the draft.
    **State and events:** no change.
    **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only`.

## Branches

### A stale revision is refused

- **When:** the agent revises with `expected_revision` 1 after a revision already made it 2.
- **I do:** the agent calls `view_revise` with the old revision.
- **I see:** the call fails with the kind revision conflict (gRPC Aborted). The MCP reply is an error.
- **Expected outcome:** the draft is unchanged, and the agent knows to read it again.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` (the stale call is Aborted) and `daemon/humatad/tests/mcp.rs::an_agent_prepares_revises_and_offers_a_view` (isError).

### An offered view cannot be revised

- **When:** the agent revises a view after it is marked ready.
- **I do:** the agent calls `view_revise`.
- **I see:** the call fails with InvalidArgument: an offered view can't be revised.
- **Expected outcome:** the offer is unchanged.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it`.

### The user answers a draft

- **When:** I try to set a state on a view that is still a draft.
- **I do:** the app or `humata view open <id>` on the draft.
- **I see:** the answer is refused: the draft isn't offered yet.
- **Expected outcome:** nothing changes. The draft is still the agent's.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` (InvalidArgument), and the review's Draft check at `daemon/humatad/src/review/views.rs:284-291`.

### The agent tries to set an answer on its own offer

- **When:** the agent calls `set_view_state` (Kept) on a view it has marked ready.
- **I do:** the agent makes the call.
- **I see:** the call is refused. Only the user answers an offer.
- **Expected outcome:** the state is unchanged.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it`.

### A draft fails its checks and nothing is kept

- **When:** the agent prepares a view that breaks a rule: no title, a walkthrough with no step, a pull request view with no repo, a board in another channel, a plan that names a file its spec does not show, or a board of the global scope.
- **I do:** the agent calls `view_prepare` with the bad content.
- **I see:** each call fails with the kind of error that names what is wrong (InvalidArgument for a bad field, FailedPrecondition for a file the spec does not show). The list of the agent's views is still empty.
- **Expected outcome:** nothing refused is kept.
- **Status:** Works (real daemon). `daemon/humatad/tests/review.rs::a_draft_view_is_checked_before_it_is_kept`. A board in a second session of the same channel is allowed, and the same test checks that too.

### A draft while the host is away

- **When:** this machine's host is away, and the agent prepares a board view on this machine.
- **I do:** the agent calls `view_prepare` for a board in its channel.
- **I see:** the call fails with the kind `host_unreachable`. Nothing is written for the host.
- **Expected outcome:** the draft is refused, and the host's journal stays empty.
- **Status:** Works (real daemon, away harness). `daemon/humatad/tests/away_board_journal.rs::a_view_draft_while_away_is_host_unreachable_and_nothing_is_kept`. Only the board scope is tested. A walkthrough draft while away is not tested.

### The user has said Not now to a view

- **When:** I click Not now, or run `humata view not-now`.
- **I do:** click Not now on an offer.
- **I see:** the offer goes away for good. The view stays in the Journal.
- **Expected outcome:** the offer does not come back on a return, and the view stays visible in the session's Journal.
- **Status:** Fakes only for the offer going away: `features/views/tests/offers.rs::returning_shows_a_deferred_offer_but_never_a_dismissed_one`. The daemon's cross-session list still returns a Not now view, which is a Likely bug (see below).

### The app's own view state is asked for by a call

- **When:** the app or a caller uses the view state service, for the app's own saved state.
- **I do:** a caller opens a view-state key.
- **I see:** the call returns Unimplemented with a message that the method is not built.
- **Expected outcome:** the app keeps its own state elsewhere until the service is built.
- **Status:** Not built (no milestone). `daemon/humatad/src/not_built.rs` answers every method of `ViewStateService`. `VIEWS.md` P4 names a view-state key for later.

### The "Prepare views for you" rule is off

- **When:** the rule for preparing views is Never or Ask for this channel.
- **I do:** the agent calls `view_prepare`.
- **I see:** the call is refused or asks the user first, by the channel rule.
- **Expected outcome:** the rule is applied to the call before it reaches the view code.
- **Status:** Code, no test found. The rule is `Capability::ViewsPrepare` (`daemon/access/src/capability.rs`), checked at `daemon/humatad/src/guard.rs:379` and `daemon/humatad/src/access_service.rs:765`. A grep of the daemon, access and CLI tests for `ViewsPrepare` or `views_prepare` finds nothing.

## Expected outcomes

- [ ] A draft is visible to its agent only, and not to the user, the feed or the Journal.
- [ ] A draft can be revised as often as the agent likes, with a matching revision.
- [ ] A stale revision is refused and the draft is unchanged.
- [ ] Marking a draft ready makes one offer. A repeated mark-ready tells nobody again.
- [ ] A draft that breaks a rule is refused, and nothing refused is kept.
- [ ] An offered view can't be revised, and the agent can't answer its own offer.
- [ ] Later is kept on the daemon, and the offer comes back on the user's next visit.
- [ ] Open puts the view in the Journal as Opened, and the feed tells both parties.
- [ ] A restart keeps the view, its state and its id, and a repeated key makes no second view.
- [ ] The user's list of offered views is newest first and names no session.
- [ ] A dismissed (Not now) view does not come back as an offer.
- [ ] A draft while the host is away is refused as host unreachable, and nothing is kept for the host.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. `view_prepare` makes a draft | Works (real daemon) | `mcp.rs::an_agent_prepares_revises_and_offers_a_view`; `review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` |
| 2. `view_revise` with a matching revision | Works (real daemon) | same two tests |
| 3. `view_mark_ready` offers it | Works (real daemon) | `review.rs::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only` |
| 4. Offer bar in the app | Not built (VIEWS) | fixture only: `desktop/tests/native/session.rs::prepared_views_wait_for_open_and_later_returns_on_the_next_visit` |
| 5. Later hides the offer | Fakes only (app); daemon Works | `features/views/tests/offers.rs::returning_shows_a_deferred_offer_but_never_a_dismissed_one` |
| 6. Offer returns on the next visit | Fakes only | `desktop/tests/native/session.rs::prepared_views_wait_for_open_and_later_returns_on_the_next_visit` |
| 7. Open it opens the tab | Works (real daemon) for the state and feeds; Fakes only for the tab | `review.rs::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only` |
| 8. A restart keeps the offer | Works (real daemon) | `review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` |
| 9. `humata views` lists newest first | Works (real daemon); CLI output Fakes only | `review.rs::the_user_lists_every_offered_view_newest_first_without_naming_a_session`; `cli/tests/views.rs::views_with_no_session_asks_for_every_session_and_prints_them` |
| 10. Not now (daemon) | Code, no test | `review/views.rs::set_state`; CLI Fakes only: `cli/tests/views.rs::each_view_action_sets_its_state_once` |
| 11. The agent's feed | Works (real daemon) | `review.rs::the_feed_tells_the_agent_of_its_drafts_and_the_user_of_offers_only` |
| Branch: stale revision | Works (real daemon) | `review.rs::a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it` |
| Branch: offered view not revisable | Works (real daemon) | same test |
| Branch: user answers a draft | Works (real daemon) | same test |
| Branch: agent answers its own offer | Works (real daemon) | same test |
| Branch: draft fails its checks | Works (real daemon) | `review.rs::a_draft_view_is_checked_before_it_is_kept` |
| Branch: draft while host away | Works (real daemon, away harness) | `away_board_journal.rs::a_view_draft_while_away_is_host_unreachable_and_nothing_is_kept` |
| Branch: Not now hides for good | Fakes only; list is a Likely bug | `features/views/tests/offers.rs`; `review/views.rs:249-255` |
| Branch: view state service | Not built (no milestone) | `daemon/humatad/src/not_built.rs` |
| Branch: "Prepare views for you" rule off | Code, no test | `guard.rs:379`, `access_service.rs:765` |

## Gaps and likely bugs

- **Not built (VIEWS milestone, planning):** the app's offer bar, Open it, Later and Not now are not wired to the daemon. `features/views` is a fixture keyed by title, and no app crate uses `clients/live/src/views.rs`. Revert (VW-9) and Keep are missing. The view tab's contents (VW-6) are missing. The Journal states do not match VW-10. VW-12, where a remote session's offers reach the app, is not met: views live on the session's own daemon. The milestone file is `workflow/milestones/VIEWS.md`, status planning.
- **Not built (no milestone):** `ViewStateService` answers every method as not built (`daemon/humatad/src/not_built.rs`). The app's own state for views is not stored there yet.
- **Likely bug:** `daemon/humatad/src/review/views.rs:249-255` (`every_offered`) filters out only Draft views. A Not now view is still returned by the user's cross-session list, and so by `humata views` (`cli/src/commands/boards.rs:155-170`, which adds no filter). VW-5 says Not now hides the offer for good. To confirm: prepare a view, mark it ready, run `humata view not-now <id>`, then run `humata views`. Expect the view listed.
- **Test gap:** no daemon test sets Not now or checks that a Not now view leaves the offer list.
- **Test gap:** the "Prepare views for you" rule has no test that refuses `view_prepare` when the rule is off.
- **Corrections to the brief:** the operation is `view_prepare` (singular), and there is no `ViewsPrepare` operation. `ViewsPrepare` is a capability, the rule "Prepare views for you" (`daemon/access/src/capability.rs`).

# A remembered sentence is found in last week's conversation, opened at its turn, pinned, and a file is found by name

Layer: macro. Platforms: macOS first; Linux and the web build share the same UI, and the host search is the same on every platform.

## Starting state

- Humata 2 is open on this Mac. The host (humatad) runs on this-mac. box-1 is online and has its own sessions.
- Last week, an agent in the acme channel said "the retry budget is 3" in session "api rate limiter" on this-mac. The mirror of that conversation is on the host.
- The sidebar shows the acme channel and its sessions. Nothing is pinned. The palette is closed.
- The session page for "api rate limiter" is open on its agent tab. The agent is not running.
- The session "api rate limiter" has a clone of acme/api with a file `src/client.rs`.
- I hold nothing but the memory of the sentence and the word "budget".

## Steps

1. **I do:** press ⌘K on the session page.
   **I see:** the palette opens over the session. The search box has focus and is empty. The entries list shows the commands and the sessions, with the session I am in ranked first (KP-3).
   **State and events:** the palette opens. Nothing is sent to the agent.
   **Status:** Works (real daemon). `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` presses ⌘K on a session page and waits until the palette shows with nothing typed. The same test is the only live check of the palette on a real host.

2. **I do:** type "retry budget".
   **I see:** the entries narrow to the matching commands and sessions. Under them, a section named **Conversations** lists the hits. Each row shows the session title, the channel, the machine and the turn. Words that match are emphasised. The first hit has the cursor.
   **State and events:** the app sends one search to the host with the two words. The host returns hits where every word matches and the last word may be the start of a word. Hits from box-1 would name box-1.
   **Status:** Works (real daemon) for the host's matching rule: `daemon/humatad/tests/history.rs::every_word_must_match_and_only_the_last_may_be_the_start_of_one` and the app's search over the live client: `clients/live/tests/history.rs::a_search_finds_the_turn_that_has_the_words`. Fakes only for the rows and the cursor: `features/find/tests/search.rs::a_query_loads_its_hits` and `features/palette/tests/hits_cursor.rs::hits_arriving_leave_a_cursor_that_still_fits`. The emphasis of matched words is a pure function in `ui/src/palette_hits.rs`, proven by its unit tests `every_word_is_emphasised_wherever_it_starts_whatever_its_case` and `a_word_that_is_not_there_emphasises_nothing` (no shell, no daemon).

3. **I do:** type a single letter, then delete it, and type "retry budget" again.
   **I see:** with one character the Conversations section goes away and no search is made. With the two words the hits come back.
   **State and events:** a query under two characters clears the hits and asks the host for nothing. The next query with two characters asks again.
   **Status:** Fakes only. `features/find/tests/search.rs::a_query_under_two_characters_clears_the_hits_and_asks_nothing` and `features/find/tests/search.rs::spaces_do_not_count_towards_the_two_characters`. The host side is not asked for anything, so no real-daemon test is needed.

4. **I do:** run the same search from a shell: `humata sessions search "retry budget" --since 2026-11-08T00:00:00Z` (the time is RFC 3339, as `--since` requires).
   **I see:** one line per hit: the session, the channel, the machine, the agent, the turn number and a short text. Exit code 0.
   **State and events:** the CLI calls SearchSessions on the host. The CLI does not filter by channel or machine (see the branch "A channel or machine filter").
   **Status:** Fakes only. `cli/tests/search.rs::search_sends_the_words_joined_and_every_filter` and `cli/tests/search.rs::a_person_gets_the_hits_as_text` run the CLI against a fake daemon. The host answer behind the command is proven by `daemon/humatad/tests/history_tools.rs::the_tool_finds_what_search_sessions_finds`, which compares the MCP tool with the same search over a real daemon. The CLI binary itself is not run against a real daemon.

5. **I do:** click the first hit, or press Return on it.
   **I see:** the palette closes. A **Conversation** tab opens in the focused group. It is read-only. It shows the turns around the hit, and the hit's entry is in view. The tab is in front.
   **State and events:** the app asks the host for the hit's turns (ReadTurns, a window around the hit's turn). The tab records the hit's turn and entry sequence (D186). Nothing is sent to the agent.
   **Status:** Works (real daemon). `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (step 2): the host search finds the word, the hit opens the Conversation tab in front, and the tab shows the host's entries for the turns it read. Fakes only for the tab's details: `features/app/tests/find_conversation.rs::picking_a_search_hit_opens_a_read_only_conversation_on_its_entry` and `features/palette/tests/hits_cursor.rs::running_a_hit_closes_the_palette_and_hands_its_place_to_the_app`.

6. **I do:** in the Conversation tab, press "show earlier turns" a few times, then "show later turns".
   **I see:** five turns are added each time, in order, above or below what I see. At turn 0 the earlier control goes quiet. The later control goes quiet once a read comes back empty.
   **State and events:** each click asks for the next five turns from the host. A failed read shows an error and Retry reads again.
   **Status:** Fakes only for the tab: `features/session/tests/conversation.rs::show_earlier_turns_adds_five_before_in_order_until_turn_zero`, `features/session/tests/conversation.rs::show_later_turns_adds_five_after_until_a_read_comes_back_empty`, `features/session/tests/conversation.rs::a_failed_read_shows_an_error_and_retry_reads_again`. The host paging is Works (real daemon): `daemon/humatad/tests/history.rs::a_read_pages_through_a_session` and `clients/live/tests/history.rs::read_returns_the_turns_in_order_with_who_wrote_them`.

7. **I do:** run `humata session read "api rate limiter" --from 40 --to 42` in a shell.
   **I see:** the three turns, in order, with who wrote each one. Exit code 0.
   **State and events:** the CLI calls ReadTurns on the host. Nothing is written.
   **Status:** Fakes only for the CLI: `cli/tests/search.rs::read_sends_the_session_and_turn_range` and `cli/tests/search.rs::read_resolves_a_session_by_title`. The host read is Works (real daemon): `daemon/humatad/tests/history.rs::a_read_returns_the_turns_asked_for_in_order`.

8. **I do:** press ⌘K, type "pin", and run **Pin** on the session "api rate limiter".
   **I see:** the palette shows **Pin api rate limiter** with the pin icon. After Enter the palette closes. The Pinned section at the top of the sidebar gets one row for the session, at the end of the list, with its title.
   **State and events:** the app sends a set-mark change to the host (MarkService, user only). The host stores the pin in `marks.db` and publishes the change on the marks topic. The app takes the new list from the host.
   **Status:** Fakes only for the palette entry and the app's handoff: `features/palette/tests/marks.rs::typing_finds_pin_and_running_it_hands_the_change_to_the_app` and `features/palette/tests/marks.rs::an_unpinned_session_offers_pin_and_vip`. The Pinned row on a fake client: `features/app/tests/find.rs::a_pin_made_on_another_client_shows_in_the_pinned_section`. The host's storage and order are Works (real daemon): `daemon/humatad/tests/marks.rs::a_new_pin_goes_at_the_end` and `daemon/humatad/tests/marks.rs::a_pinned_session_is_listed_with_its_title`. The pin reaches the app and a second client on a real daemon: `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` (the pin step, through the live marks client).

9. **I do:** close Humata, restart the Mac, and open Humata again.
   **I see:** the Pinned section shows "api rate limiter" at the top, in the same place. The Conversation tab I had open is back and reads its turns again.
   **State and events:** the pin and its order live in `<state>/marks.db` on the host, so they are read back from there. The restored Conversation tab asks for its turns again; it does not keep old text.
   **Status:** Works (real daemon). `daemon/humatad/tests/marks.rs::marks_and_their_order_survive_a_restart` and `clients/live/tests/marks.rs::pins_and_vip_marks_survive_a_restart`. The restored tab on fakes is `features/app/tests/find_conversation.rs::a_saved_conversation_tab_comes_back_after_a_restart_and_reads_again`. The history of the mirror across a host restart is `daemon/humatad/tests/history.rs::a_restart_keeps_what_was_mirrored`.

10. **I do:** press ⌘K and type "client.rs".
    **I see:** a **Files** group lists `api/src/client.rs` under the session it lives in, with the session's title as the hint. Enter opens the Files panel of that session with the file selected and open.
    **State and events:** the palette builds its file entries from the workspace the app holds (`session_files`). Picking one navigates to the session, selects the file and opens it.
    **Status:** Works (real daemon): `desktop/tests/live_flows/find.rs::a_file_in_a_sessions_clone_is_found_from_the_palette` types `README` into the palette on a real host and the Files group lists the session clone's README with its session. Opening the palette fetches each session's files once (`features/app/tests/palette_files.rs::opening_the_palette_fetches_each_sessions_files_once`); the palette's ranking is `features/palette/tests/search.rs::file_search_and_parent_hints_keep_their_session_context`. (Was a likely bug: the live workspace held no files; fixed by BUGS.)

11. **I do:** in a shell, run `humata files find "api rate limiter" client` (the session's title, then the query).
    **I see:** one line per matching path, across the session's repos, in the session's order, with its size, kind and where the clone is. Exit code 0.
    **State and events:** the CLI calls FindFiles on the host, which lists the tracked and untracked files of each repo clone and skips ignored files and git's own folders. Nothing is changed.
    **Status:** Code, no test for the CLI command: `cli/src/commands/files.rs:34` sends FindFiles, but `cli/tests/files.rs` has no find test and `daemon/humatad/tests/cli_files.rs` runs no find. The host's FindFiles is Works (real daemon): `daemon/humatad/tests/find_files.rs::a_query_finds_matching_paths_in_each_repo_in_the_sessions_order` and `daemon/humatad/tests/find_files.rs::ignored_files_and_git_internals_never_show_but_untracked_files_do`.

12. **I do:** let an agent on box-1 ask for the same file through MCP: `files_find` with the query "client.rs".
    **I see:** the agent gets an error that the tool is not built, not a list of files.
    **State and events:** the MCP call is refused with the not-built status. Nothing is recorded.
    **Status:** Not built (no milestone). The tool is defined at `daemon/mcp/src/tools.rs:945` (Op::FilesFind) and listed in `daemon/mcp/tests/tools.txt`, but the host's MCP dispatch in `daemon/humatad/src/mcp.rs` has no arm for it, so it falls through to `other => Err(not_built(other))` at `daemon/humatad/src/mcp.rs:1064`. The gRPC FindFiles that it should call exists (step 11).

## Branches

### The turn is on a machine that is offline

- **When:** box-1 is asleep or its link is down when I search. It had sessions that talked about the retry budget.
- **I do:** type "retry budget" in the palette, then pick a hit from box-1.
- **I see:** the hits from box-1 still show, with the text "as of" and the time of box-1's last sync. Picking the hit opens the Conversation tab with the turns the host kept. Nothing from box-1 that happened after its last sync appears. A search that runs while box-1 is offline still answers.
- **State and events:** the host answers from the mirror it holds (LD-23). Nothing crosses from box-1 while it is offline. The hit's "as of" time does not change while it is offline.
- **Expected outcome:** the search answers, the hit opens, the "as of" label shows, and what I read is what the host held when box-1 went away.
- **Status:** Works (real daemon) for the host side: `daemon/humatad/tests/remote_history.rs::a_machine_that_was_offline_catches_up_when_its_link_returns` (the host answers while the machine is offline with the same as-of time, and a word written only on the machine is not found until the link returns). Fakes only for the app: `features/app/tests/find_conversation.rs::a_hit_on_an_offline_machine_opens_the_same_conversation`. The "as of" time text is unit-tested (`ui/src/palette_hits.rs::as_of_reads_as_a_day_and_a_time`). The label is drawn at `ui/src/palette_hits.rs:198-204` from the caller's offline flag, and no test checks that the flag is set when a machine is offline, so that part is Code, no test.

### The search finds nothing

- **When:** I type a phrase that is not in any mirror, such as "retry quota".
- **I do:** type it in the palette.
- **I see:** the entries narrow to nothing, and the Conversations section is gone. There is no line that says nothing matched.
- **State and events:** the host answers with zero hits and no next page. The app hides the section (no hits, no error, no next page).
- **Expected outcome:** the palette says that nothing matched, in words, so I know the search ran and found nothing. It does not look like the search did not run.
- **Status:** Works (real daemon): `desktop/tests/live_flows/find.rs::a_search_that_finds_nothing_says_so`. A search nothing matches lists nothing and says `Nothing matches “…”. Try fewer letters or fewer words.` (`ui/src/overlay_palette.rs`, the one empty-state line). (Was a likely bug; the line was added by BUGS.)

### The search fails

- **When:** the host does not answer, or answers with an error (for example, a bad page token).
- **I do:** type "retry budget".
- **I see:** a red line under Conversations: "Search didn't answer: <error>. Edit the query to try again." Editing the query tries again.
- **State and events:** the search state holds the error. The next query clears it.
- **Expected outcome:** the message says what failed and how to try again; the entries are still there.
- **Status:** Fakes only. `features/find/tests/search.rs::a_failed_search_shows_and_the_next_query_clears_it` and `features/find/tests/search.rs::a_short_query_clears_the_error_too`. The real host error path is Works (real daemon) at `daemon/humatad/tests/history.rs::a_search_for_nothing_is_refused` and `daemon/humatad/tests/history.rs::a_page_token_the_daemon_never_gave_is_refused` (the host refuses; the app's rendering of it is Fakes only).

### Many hits

- **When:** the phrase matches more hits than one page.
- **I do:** scroll to the "More conversations" row and press Return on it, or type a word that makes many hits.
- **I see:** the next page of hits is added after the ones shown. The palette stays open. After the last page, the row goes away.
- **State and events:** the app asks for the next page with the page token the host gave. A new query drops the old next page.
- **Expected outcome:** every hit appears once, in the order the host sent them.
- **Status:** Works (real daemon) for the host's paging: `daemon/humatad/tests/history.rs::a_search_pages_through_its_hits` and `clients/live/tests/history.rs::a_search_pages_through_every_hit`. Fakes only for the palette row: `features/find/tests/search_more.rs::more_adds_the_next_page_after_the_hits_shown` and `features/find/tests/search_more.rs::more_after_the_last_page_asks_nothing`.

### A channel or machine filter

- **When:** I want only the acme channel's hits, or only box-1's.
- **I do:** run `humata sessions search "retry budget" --channel acme`.
- **I see:** the CLI refuses the flag as unknown. The palette has no channel or machine filter either.
- **State and events:** nothing is sent.
- **Expected outcome:** the plan in `api/CLI.md:200` says the command takes `--channel` and `--machine`, and the host's SearchSessions can take them. The CLI should accept them, or say they are not there yet.
- **Status:** Not built. `api/CLI.md:200` plans `--channel` and `--machine`. The built command takes only words, `--agent`, `--since` and `--until` (`cli/src/args.rs` around line 333). The FIND milestone (T5a) says the CLI is done, and does not list these filters. The host side can filter: `daemon/humatad/tests/history.rs::the_user_searches_every_channel_or_the_one_named` (real daemon, through the gRPC request, not the CLI).

### A pinned session is deleted

- **When:** I delete "api rate limiter" (through the trash, LD-5) while it is pinned.
- **I do:** delete the session, then open the Pinned section and run `humata pinned`.
- **I see:** the expected result is that the pin is gone from both. Today the pin stays, and its row shows the session's id instead of its title, because the title lookup finds nothing.
- **State and events:** the spec (NV-26) says a deleted or forgotten thing loses its pin and VIP mark; archiving keeps both. Nothing removes the mark.
- **Expected outcome:** after the delete, the Pinned section and `humata pinned` no longer list the session. After an archive they still do.
- **Status:** Works (real daemon): `daemon/humatad/tests/marks.rs::a_deleted_sessions_pin_is_gone_from_the_pinned_list`. Deleting a session forgets its pin and VIP mark (`daemon/humatad/src/mark_service.rs` calls `Marks::forget`). (Was a likely bug; fixed by BUGS.)

### Archiving a pinned session

- **When:** I archive "api rate limiter" while it is pinned.
- **I do:** archive it from the session menu or the palette's Archive.
- **I see:** the session leaves the sidebar's main list and its pin stays in the Pinned section.
- **State and events:** the archive keeps the pin and the VIP mark (NV-26).
- **Expected outcome:** the pin stays.
- **Status:** Code, no test for the pin. The archive is `features/app/tests/every_action.rs` (palette action, fakes); the pin surviving an archive is not tested. Nothing in `daemon/humatad/src/mark_service.rs` or `daemon/humatad/src/session_service.rs` removes a mark on archive, so the code matches NV-26 by omission.

### §26.22 steps the FIND milestone still owes

- **When:** I run the palette flow from SPEC §26.22 end to end: ⌘K in the agent terminal, Move session, paste a PR link.
- **I do:** press ⌘K inside the focused `claude` terminal tab, then pick Move session and a machine, then paste a PR URL.
- **I see:** the palette opens over the terminal and the keystroke does not reach the agent (step 1). The move asks for a machine inline and starts (step 3). A pasted PR URL offers Review this PR with the PR filled in (step 4).
- **State and events:** see each part below.
- **Expected outcome:** all three work as §26.22 says.
- **Status:** per part.
  - **Step 1 (⌘K in a focused agent terminal, KP-6):** Not built for the proof. No test covers KP-6 (grep finds no `KP-6` in the tests). The terminal's part is C1's (FIND.md, "Main's ruling"). The live test presses ⌘K on the session page, not in the terminal.
  - **Step 3 (Move session from the palette):** the app side is Fakes only (`features/app/tests/move_from_palette.rs`). The daemon's MoveSession is built (`daemon/humatad/src/session_service/move_session.rs`), and the Works (real daemon) test is `daemon/humatad/tests/move_session.rs`. The live rig has one machine (`desktop/tests/live_flows/world.rs`), so the live test stops at the inline ask and Escape. FIND.md says the daemon MoveSession is not built (MV-2); the code says it is built, so FIND.md is stale on this point.
  - **Step 4 (Review this PR):** Not built. The Review a pull request dialog belongs to the PR milestone (PR-1). Until then, a pasted PR gets Review this PR only when a session here already reviews it (D197; the gate is in `features/palette/src/lib.rs` around line 509). The live test checks that the entry is hidden.

## Expected outcomes

- [ ] ⌘K opens the palette from a session page and nothing reaches the agent.
- [ ] Typing "retry budget" shows the matching hits under Conversations, with the newest page first from the host.
- [ ] One character does not search; two do.
- [ ] Picking a hit opens a read-only Conversation tab on that turn, in front.
- [ ] Turns can be paged earlier and later, in order.
- [ ] `humata session read` prints the same turns the tab shows.
- [ ] A pin appears at the end of the Pinned section and holds across a restart, in the same order.
- [ ] A hit from an offline machine answers from the host and says "as of" its last sync.
- [ ] A search with no matches says so in words.
- [ ] A file found by name opens its session's file (live).
- [ ] `humata files find` lists matches across the session's repos.
- [ ] A deleted pinned session loses its pin.

## Status summary

| Step or branch | Status | Evidence |
|---|---|---|
| 1. ⌘K opens the palette on the session page | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` |
| 2. Type "retry budget"; hits list under Conversations | Works (real daemon) for matching; Fakes only for rows and emphasis | `daemon/humatad/tests/history.rs::every_word_must_match_and_only_the_last_may_be_the_start_of_one`, `clients/live/tests/history.rs::a_search_finds_the_turn_that_has_the_words`, `features/find/tests/search.rs::a_query_loads_its_hits`, `ui/src/palette_hits.rs::every_word_is_emphasised_wherever_it_starts_whatever_its_case` |
| 3. One character does not search | Fakes only | `features/find/tests/search.rs::a_query_under_two_characters_clears_the_hits_and_asks_nothing` |
| 4. `humata sessions search` | Fakes only (CLI); host answer Works (real daemon) via MCP | `cli/tests/search.rs::search_sends_the_words_joined_and_every_filter`, `daemon/humatad/tests/history_tools.rs::the_tool_finds_what_search_sessions_finds` |
| 5. Pick a hit; Conversation tab opens at the turn | Works (real daemon) | `desktop/tests/live_flows/find.rs::from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds` |
| 6. Show earlier / later turns | Fakes only (tab); Works (real daemon) for host paging | `features/session/tests/conversation.rs::show_earlier_turns_adds_five_before_in_order_until_turn_zero`, `daemon/humatad/tests/history.rs::a_read_pages_through_a_session` |
| 7. `humata session read` | Fakes only (CLI); host Works (real daemon) | `cli/tests/search.rs::read_sends_the_session_and_turn_range`, `daemon/humatad/tests/history.rs::a_read_returns_the_turns_asked_for_in_order` |
| 8. Pin a session; it shows at the end of Pinned | Fakes only (palette, app); Works (real daemon) for host order | `features/palette/tests/marks.rs::typing_finds_pin_and_running_it_hands_the_change_to_the_app`, `daemon/humatad/tests/marks.rs::a_new_pin_goes_at_the_end` |
| 9. Restart keeps the pin and order | Works (real daemon) | `daemon/humatad/tests/marks.rs::marks_and_their_order_survive_a_restart`, `clients/live/tests/marks.rs::pins_and_vip_marks_survive_a_restart` |
| 10. Palette file search opens a file | Works (real daemon) | `desktop/tests/live_flows/find.rs::a_file_in_a_sessions_clone_is_found_from_the_palette`; `features/app/tests/palette_files.rs::opening_the_palette_fetches_each_sessions_files_once`; `features/palette/tests/search.rs::file_search_and_parent_hints_keep_their_session_context` |
| 11. `humata files find` | Code, no test (CLI); host Works (real daemon) | `cli/src/commands/files.rs:34`; `daemon/humatad/tests/find_files.rs::a_query_finds_matching_paths_in_each_repo_in_the_sessions_order` |
| 12. Agent calls `files_find` over MCP | Not built (no milestone) | `daemon/humatad/src/mcp.rs:1064` |
| B1. Turn on an offline machine | Works (real daemon) for host; Fakes only for app | `daemon/humatad/tests/remote_history.rs::a_machine_that_was_offline_catches_up_when_its_link_returns`; `features/app/tests/find_conversation.rs::a_hit_on_an_offline_machine_opens_the_same_conversation` |
| B2. No results | Works (real daemon) | `desktop/tests/live_flows/find.rs::a_search_that_finds_nothing_says_so` |
| B3. Search fails | Fakes only (app); Works (real daemon) for host refusal | `features/find/tests/search.rs::a_failed_search_shows_and_the_next_query_clears_it` |
| B4. Many hits | Works (real daemon) for host; Fakes only for row | `daemon/humatad/tests/history.rs::a_search_pages_through_its_hits`, `features/find/tests/search_more.rs::more_adds_the_next_page_after_the_hits_shown` |
| B5. Channel or machine filter | Not built | `api/CLI.md:200`; `cli/src/args.rs` around line 333 |
| B6. Pinned session deleted | Works (real daemon) | `daemon/humatad/tests/marks.rs::a_deleted_sessions_pin_is_gone_from_the_pinned_list` |
| B7. Archive keeps the pin | Code, no test | `daemon/humatad/src/mark_service.rs` (no removal on archive) |
| B8. §26.22 step 1 (⌘K in terminal, KP-6) | Not built (C1 owes it) | FIND.md; no `KP-6` test |
| B8. §26.22 step 3 (Move from palette) | Fakes only (app); Works (real daemon) for MoveSession | `features/app/tests/move_from_palette.rs`; `daemon/humatad/tests/move_session.rs` |
| B8. §26.22 step 4 (Review this PR) | Not built (PR milestone, PR-1) | D197; `features/palette/src/lib.rs` around line 509 |

## Gaps and likely bugs

**Not built**
- `files_find` over MCP has no dispatch arm in the host (`daemon/humatad/src/mcp.rs:1064`). No milestone.
- `humata sessions search` has no `--channel` or `--machine` filter (`api/CLI.md:200`; `cli/src/args.rs` around line 333). FIND T5a.
- The session menu's Pin and VIP entries (NV-20, the ⋯ menu) are not built: `ui/src/channel.rs:179` says the menu has no items yet, and only the channel's Pin icon and the palette pin sessions. No milestone found.
- §26.22 step 1 (⌘K from a focused agent terminal, KP-6): owed by C1 (FIND.md).
- §26.22 step 4 (Review this PR with the PR filled in): owed by the PR milestone (PR-1; D197).
- §26.22 step 3 live: the multi-machine rig is not in `desktop/tests/live_flows/world.rs` (FIND.md). The daemon's MoveSession is built, so the note in FIND.md that it is not built is stale.

**Likely bugs (all fixed)**
- Fixed (was a likely bug): a search with no matches says so (`desktop/tests/live_flows/find.rs::a_search_that_finds_nothing_says_so`).
- Fixed (was a likely bug): a deleted session's pin is gone (`daemon/humatad/tests/marks.rs::a_deleted_sessions_pin_is_gone_from_the_pinned_list`).
- Fixed (was a likely bug): the palette's Files group lists the clones' files on a real host (`desktop/tests/live_flows/find.rs::a_file_in_a_sessions_clone_is_found_from_the_palette`).

**Corrections to the brief**
- The brief's list of owed §26.22 steps (1, 3, 4) is right, but FIND.md says the daemon's MoveSession is not built (MV-2). The code has `daemon/humatad/src/session_service/move_session.rs`, so only the live rig is owed for step 3.
- "Palette search modes" is not a separate mode in the code. The palette has one query box; it searches commands, sessions, channels, files, pulls and, from two characters, conversations (`features/palette/src/lib.rs` and `features/find/src/search.rs`).
- "`humata sessions search`" is the CLI name, and `files_find` is the MCP name. The CLI for files is `humata files find`, a different command; it has no test.

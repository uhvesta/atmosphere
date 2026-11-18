# A teammate's question becomes a Desk session, its doc is copied to the channel, and the session closes

Layer: macro. Platforms: macOS first; Linux where the host runs; the web build shares the same UI.

## Starting state

- Humata 2 is open on this Mac. The host (humatad) runs on this-mac, and devbox-1 is online.
- A channel named acme exists, with the repo acme/api and a shared folder. This Mac is in it.
- The Desk page has two older sessions: one open (plan-q4) and one closed.
- The sidebar shows Desk under Board. Nothing else is open.
- Outside Humata, a teammate's message is in my chat app: "can you look at whether we could batch the export job?". I have copied it.
- The agent is the normal agent for this Mac. Tests use a fake agent that prints "agent up" and echoes what it is told.

## Steps

1. **I do:** click **Desk** in the sidebar.
   **I see:** the Desk page lists open sessions (status, title) and closed ones in two groups. Each row has Open or Reopen.
   **State and events:** the page reads the Desk list from the host. Nothing changes.
   **Status:** Works (real daemon). `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` opens the page in the headless app against a real `humatad2`. Fake-client version: `desktop/tests/native/desk.rs::the_desk_page_lists_open_documents_and_closed`. The host's list of open and closed Desk sessions is covered by the real-daemon tests that close and reopen sessions (`daemon/humatad/tests/desk.rs::closing_stops_the_agent_and_keeps_the_folder_for_seven_days`).

2. **I do:** click **New desk session**. Type the title "batch export idea", pick this-mac as the machine, pick Agent. The form has no folder to pick. Turn the two access switches as I want them. Click Start.
   **I see:** the dialog shows no path; a line says Humata makes the folder (DK-14). After Start, the session opens on its agent tab, and the folder bar shows the folder and a "scratch" chip.
   **State and events:** one new Desk session is started on this-mac. It is filed under the desk channel. The route becomes the new session. The form closes.
   **Status:** Works (real daemon). `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` types the title, clicks Start, and sees the session page open with the folder bar and its scratch chip. Fake-client version: `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens`. The form with no folder to pick and no path is Fakes only: the frame 40 snapshot in `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens`. The form no longer draws the path. It still works out where the folder will be, and that the daemon makes that folder Works (real daemon): `desktop/tests/live_flows/desk_form.rs::the_new_desk_session_form_previews_the_folder_the_daemon_makes_and_clones_its_repo_chip` and `::a_taken_title_previews_the_suffixed_folder_the_daemon_then_makes`. The real start is `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title`.

3. **I do:** nothing more in the form. I look at the folder bar.
   **I see:** the folder is `desk/batch-export-idea` under the machine's Humata root. It is not named by me, and it has no name field (DK-14).
   **State and events:** the folder is created on this-mac. The session's folder is recorded as a scratch folder, not kept, not picked.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title` (asserts the folder is `desk/plan-q4` for the title "Plan Q4"; same rule). The name rules are in `daemon/humatad/tests/desk.rs::a_taken_name_gets_the_next_suffix` (Works (real daemon)) and `daemon/humatad/tests/desk.rs::an_untitled_session_gets_the_fallback_name` (Works (real daemon); the test checks only that the folder sits in the Desk root).

4. **I do:** in the form, add the repo acme/api before Start (optional). Start.
   **I see:** the folder contains an `api/` folder with the repo's default branch after the start finishes.
   **State and events:** the repo is cloned into the session's folder. The Desk session lists it.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_start_with_repos_clones_each_into_the_folder` (asserts `api/README.md` and the repo list). The form's repo chip Works (real daemon): `desktop/tests/live_flows/desk_form.rs::the_new_desk_session_form_previews_the_folder_the_daemon_makes_and_clones_its_repo_chip` adds it in the form and finds the clone in the folder. Add repo in the folder bar, with its chip, Works (real daemon): `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`.

5. **I do:** paste the teammate's message as the first prompt. The Desk form has no field for it (DK-14 asks for a title, an agent, a machine, a folder, repos and access switches, and no first message). So I paste it into the agent's terminal once the agent is up.
   **I see:** the message appears at the agent's prompt, and the agent starts on it.
   **State and events:** the pasted text reaches the agent's PTY. A multi-line paste waits for my confirmation, and a one-line paste is sent as is.
   **Status:** Fakes only for the paste. `terminal/pane/tests/pane.rs::a_one_line_paste_is_sent_and_bracketed_when_asked` and `terminal/pane/tests/pane.rs::a_multi_line_paste_waits_for_the_user` run a real emulator with a recording PTY, not a real agent. Typing into a running agent through the host is Works (real daemon): the test helper `say` in `daemon/humatad/tests/desk.rs` types a line into the fake agent and waits for its echo. The Desk form having no first-prompt field is a spec fact (DK-14), not a gap in the test.

6. **I do:** wait for the agent. Read what it finds.
   **I see:** the agent's terminal shows "agent up" and then its reply.
   **State and events:** the Desk session is Working. Its agent tab is attached.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title` waits for "agent up --session-id". The agent is a fake. A real agent writing a doc is not tested live.

7. **I do:** see the markdown file the agent wrote in the folder.
   **I see:** the Desk session's documents list shows the markdown file, and non-markdown files are left out.
   **State and events:** the host lists the folder's markdown files on the session row.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder`. The test writes the file by hand; it does not check that an agent wrote it.

8. **I do:** keep the document, then copy it to the acme channel's shared folder.
   **I see:** Keep folder marks the folder as kept, and the delete time goes away. The copy appears in acme's shared folder, and the Desk's own copy stays.
   **State and events:** the copy goes to `shared/` of acme on this-mac. The folder keeps its days cleared.
   **Status:** Works (real daemon) for the copy and the keep: `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` and `daemon/humatad/tests/desk.rs::keeping_a_folder_takes_away_its_delete_time`. The copy dialog in the UI Works (real daemon): `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` (the test writes the file by hand). Fake-client version: `desktop/tests/native/desk.rs::a_document_is_copied_into_a_channels_shared_folder`. Keep isn't clicked in the app live.

9. **I do:** try to copy the same file again into acme (or a file with the same name that acme already has).
   **I see:** the copy is refused. acme's file is unchanged.
   **State and events:** no write happens.
   **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_copy_never_overwrites_the_channels_file`.

10. **I do:** close the session (the session menu's Close session, which is the Desk's Archive per DK-11).
    **I see:** the session moves to Closed. Its agent stops. The folder stays, and it shows that it will be deleted in seven days unless I keep it.
    **State and events:** the agent is stopped and saved stopped. `closed_at` and `delete_at` are set seven days out (the test build uses 600 s). The folder is untouched.
    **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::closing_stops_the_agent_and_keeps_the_folder_for_seven_days`. The stop is checked only as "status is not Working"; the test does not check the process is gone.

11. **I do:** open the Closed session and choose Reopen (Open).
    **I see:** the agent resumes with its conversation, in the same folder, and the session is Open again.
    **State and events:** the agent restarts with `--resume`. The folder path is unchanged. `closed_at` and `delete_at` are cleared.
    **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::reopening_resumes_the_conversation_in_the_same_folder` (checks "agent up --resume" and the same folder path).

## Branches

### The clone fails

- **When:** the repo I added in the form cannot be cloned (for example its name is wrong or the remote is unreachable).
- **I do:** click Start.
- **I see:** the form stays open with an error that says what to check. No session appears, and no folder is left behind.
- **Expected outcome:** nothing starts, and no folder is created, not even a partial one.
- **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_start_whose_clone_fails_starts_nothing_and_leaves_no_folder`. The error text is `daemon/humatad/src/desk_service.rs:528-531` ("check the repository's name and that this Mac can reach its remote, then try again"). The form keeping its error is `features/desk/tests/start.rs::a_failed_start_keeps_the_form_with_the_error` (Fakes only).

### I pick an existing folder instead of a scratch one

- **When:** a client starts a session with a picked folder; the New desk session form no longer offers one (DK-14).
- **I do:** start it with a folder that exists and sits outside Humata, through the CLI or the API.
- **I see:** the session runs in that folder. The folder bar shows it with no "scratch" chip.
- **Expected outcome:** the folder is used where it is. A path that is missing, relative, a file, inside Humata's channels, or the channels root is refused with an invalid-argument error, and nothing starts.
- **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_picked_folder_is_used_where_it_is` and `daemon/humatad/tests/desk.rs::a_picked_folder_must_exist_and_sit_outside_humata`. A picked folder is never deleted: `daemon/humatad/tests/desk.rs::a_picked_folder_is_never_deleted` (Works (real daemon)). The dialog also Works (real daemon): `desktop/tests/live_flows/desk.rs::a_picked_folder_is_used_where_it_is_and_survives_close_and_clean_up`.

### Another machine is refused for a Desk session

- **When:** I pick devbox-1 in the New desk session dialog, or send a start to it.
- **I do:** choose devbox-1 and Start.
- **I see:** the start is refused, and the message says Desk sessions start only on this machine until other machines are built. Nothing starts.
- **Expected outcome:** no session and no folder on either machine.
- **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::another_machine_is_refused_and_starts_nothing` (FailedPrecondition). The message is `daemon/humatad/src/desk_service/start.rs:43-44`. The UI side is `features/desk/tests/start.rs::with_no_online_machine_start_is_an_error` (Fakes only).

### The retried start

- **When:** the Start click is sent twice (a slow network, or I click again).
- **I do:** click Start twice.
- **I see:** one session and one folder. No "plan-q4-2" appears.
- **Expected outcome:** the same key answers with the first result.
- **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::a_retried_start_makes_one_session_and_one_folder`.

### Work not pushed holds the folder past seven days

- **When:** I close the session while a repo in its folder has commits or changes not pushed.
- **I do:** close, then wait past the seven days.
- **I see:** the session says what work it holds. The folder is not deleted.
- **Expected outcome:** the folder stays while work is held, and the delete time is unset.
- **Status:** Works (real daemon). `daemon/humatad/tests/desk.rs::work_not_pushed_holds_the_folder_and_says_why`. The rule is `daemon/humatad/src/desk_service/view.rs:37` (`expires_at(...).filter(|_| row.held_work.is_empty())`).

### Move the Desk session to box-1 from its bar

- **When:** I want the agent on devbox-1 instead.
- **I do:** pick devbox-1 in the folder bar's machine picker.
- **I see (expected, DK-3):** the folder and conversation are carried to devbox-1 and the agent restarts there.
- **I see (code):** the live client's `move_to` returns "not in this release" (`clients/live/src/desk.rs:453-455`). The daemon refuses a Move for a Desk session (`daemon/humatad/src/session_service.rs:2235`, `refuse_desk`).
- **Expected outcome:** the session moves.
- **Status:** Not built (DK-3, the DESK milestone's live part; DAG.md says DESK's live client is "not served yet, D134" for some calls). The native test `desktop/tests/native/desk.rs::a_desk_session_moves_to_another_machine_from_its_bar` passes only against the fake daemon.

### The machine restarts while the session is open

- **When:** this Mac reboots while the Desk session's agent is running.
- **I do:** log in again and open Humata 2.
- **I see (expected):** the Desk session comes back Open, with its agent resumed in the same folder.
- **I see (code):** the Desk service says its closed sessions' agents are saved stopped and that restart needs no filter for Desk sessions (`daemon/humatad/src/desk_service.rs:10-13`). The Desk tests restart the daemon only to check access switches (`daemon/humatad/tests/desk.rs::the_access_switches_are_kept_across_a_restart`) and the days left (`daemon/humatad/tests/desk.rs::the_days_left_are_counted_across_a_restart`).
- **Expected outcome:** the open Desk session resumes after the reboot with its conversation.
- **Status:** Code, no test. No test restarts the daemon while a Desk session is open and checks that its agent resumes.

## Expected outcomes

- [ ] The Desk page shows open sessions and closed ones.
- [ ] A new desk session starts on this Mac, with the title's folder name, and the session opens on its agent.
- [ ] A taken folder name gets the next suffix; an untitled session gets the fallback name.
- [ ] Repos chosen in the form are cloned into the folder before the agent runs.
- [ ] The teammate's message reaches the agent when pasted into its terminal (paste is tested with a fake PTY only).
- [ ] The agent runs in the folder, and a markdown file in the folder is listed.
- [ ] Keeping the folder clears its delete time, and copying the file into acme's shared folder leaves the Desk's copy in place.
- [ ] A copy onto an existing file in acme's shared folder is refused, and that file is unchanged.
- [ ] Closing stops the agent, keeps the folder, and sets a delete time seven days out.
- [ ] Reopening resumes the same conversation in the same folder.
- [ ] A failed clone starts nothing and leaves no folder.
- [ ] A picked folder must exist and sit outside Humata, and is never deleted.
- [ ] A start on another machine is refused.
- [ ] A retried start makes one session and one folder.
- [ ] Unpushed work holds the folder past seven days.
- [ ] Moving the Desk session to devbox-1 works (not built).
- [ ] An open Desk session resumes after a reboot (not tested).

## Status summary

| Step or branch | Status | Evidence |
| --- | --- | --- |
| 1. Desk page lists open and closed | Works (real daemon) | `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`; fakes: `desktop/tests/native/desk.rs::the_desk_page_lists_open_documents_and_closed` |
| 2. New desk session form and start | Works (real daemon); no folder and no path on the form: Fakes only (DK-14) | `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel`; today's preview and chip: `desktop/tests/live_flows/desk_form.rs::the_new_desk_session_form_previews_the_folder_the_daemon_makes_and_clones_its_repo_chip`; fakes: `desktop/tests/native/desk.rs::a_new_desk_session_starts_from_the_form_and_opens` |
| 3. Folder named from the title | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title` |
| 4. Repo cloned into the folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_start_with_repos_clones_each_into_the_folder`; the chip: `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` |
| 5. Paste the message into the agent | Fakes only | `terminal/pane/tests/pane.rs::a_one_line_paste_is_sent_and_bracketed_when_asked` |
| 6. Agent runs (fake agent) | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title` |
| 7. Markdown listed in the folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder` |
| 8. Keep and copy to acme's shared folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::keeping_a_folder_takes_away_its_delete_time`; the copy dialog: `desktop/tests/live_flows/desk.rs::a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel` |
| 9. Copy never overwrites | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_copy_never_overwrites_the_channels_file` |
| 10. Close stops the agent, keeps seven days | Works (real daemon) | `daemon/humatad/tests/desk.rs::closing_stops_the_agent_and_keeps_the_folder_for_seven_days` |
| 11. Reopen resumes in the same folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::reopening_resumes_the_conversation_in_the_same_folder` |
| Branch: the clone fails | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_start_whose_clone_fails_starts_nothing_and_leaves_no_folder` |
| Branch: pick an existing folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_picked_folder_must_exist_and_sit_outside_humata` |
| Branch: another machine refused | Works (real daemon) | `daemon/humatad/tests/desk.rs::another_machine_is_refused_and_starts_nothing` |
| Branch: retried start | Works (real daemon) | `daemon/humatad/tests/desk.rs::a_retried_start_makes_one_session_and_one_folder` |
| Branch: unpushed work holds the folder | Works (real daemon) | `daemon/humatad/tests/desk.rs::work_not_pushed_holds_the_folder_and_says_why` |
| Branch: move Desk session to box-1 | Not built | DK-3; `clients/live/src/desk.rs:453-455`; `daemon/humatad/src/session_service.rs:2235` |
| Branch: machine restarts while open | Code, no test | `daemon/humatad/src/desk_service.rs:10-13` |

## Gaps and likely bugs

- Not built: moving a Desk session to another machine (DK-3). The native test for the move passes against the fake only, so the Desk bar looks as if it works while the live client refuses it.
- Not built: a first-prompt field on the Desk form. The spec (DK-14) does not ask for one, so the paste into the agent terminal is the path.
- Code, no test: a Desk session open through a reboot resumes its agent.
- The page, the form's start and repo chip, Add repo and the copy dialog now run against a real daemon (`desktop/tests/live_flows/desk.rs`, `desk_form.rs`). The form with no folder and no path (DK-14) is proven on the fake clients only, by the frame 40 snapshot.
- Stale doc: `daemon/humatad/tests/desk.rs` (its header) says the tests are "parked until DeskService is served". The service is served (`daemon/humatad/src/serve.rs:736`), so the header is out of date.
- Not proven live: a real agent writing a markdown file into the folder. The tests use a fake agent and write the file by hand.

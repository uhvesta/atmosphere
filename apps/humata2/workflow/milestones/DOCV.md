# DOCV: A proposal doc keeps versions I can step back and forward through, in a folder Humata makes for me

**Owner asks (2026-11-16):**
1. "We should add versioning support for proposal doc aka flow 4. I want to be able to go back to previous versions of the doc and future versions of the doc. If that makes creating a git repo for the doc as a one off that is fine."
2. Picks, answered by the owner:
   - **Restore adds a version.** Restoring v3 saves it as a new newest version; v4 and later stay, and nothing is lost.
   - **Saving.** "Agent can save via git if it wants, I can also save one via a button." A version is a git commit in the doc's folder. The agent commits with plain git. The doc tab has a **Save version** button. There is no automatic snapshot per turn.
3. "Desks should automatically get a folder if I want to do spec stuff, I don't want to pick a folder I want it to make one for me and make it a git repo for the MD file."

**Shape main proposed (no objection beyond the picks above):**
- A History strip on a Markdown tab: back and forward arrows; a list of versions (time, who: agent or you, message, lines changed); **Compare with current**, using the existing diff view.
- An old version opens read-only and shows its comments as they were.
- MCP tools let the agent list and read versions.
- History moves with the session (MV-13) and survives restarts.

**Exit:**
- SPEC rules with new IDs. They cover:
  - the made-for-me git folder option in DK-2/DK-14 (kept, never auto-deleted like scratch);
  - the History strip;
  - Save version;
  - restore-as-new-version;
  - comments on old versions;
  - MCP version tools;
  - how this works for a picked folder that is or isn't a git repo.
- Flow 4 (flows/04-proposal-doc.md) gains its steps, with Status tags.
- The new frames are listed for the goldens flow. They go to the owner before any golden changes.
- Behaviour tests on fakes and the live daemon, then the UI.

**Rules:**
- AGENTS.md.
- Spec and frames come before UI.
- Bazel only.
- Never touch the user's own git repos or global git config. The made folder's repo uses its own local config.

## Lane
Manager: minion DOCV (Opus 5.5). Scripts in the session scratchpad `docv/`; main applies.

### Scope changes from main (2026-11-16)
- No picked folders: New desk session has no folder choice and shows no path; every Desk folder is made by Humata (D451). Old picked-folder sessions keep working. "Picked folder that is or isn't a git repo" is out of scope.
- Desks are global and per channel: the doc folder lives under `<root>/desk/` or `<root>/<channel>/desk/` (DV-13, D452).
- NOPATH owns file moves, roots in Settings and the rest of the spec's path audit; DOCV owns DK-2, DK-3, DK-14 and DV-*.

### Batch 1 (spec): done as scripts
`01_spec.py` (DK-2, DK-3, DK-14 rewrites; DK-11, CD-1, CD-4, MV-13 pointers; §11 "Document versions" DV-1–DV-13), `02_mcp_doc.py` (api/MCP.md rows), `03_decisions.py` (D450–D458), `04_flows.py` (flows 4 and 1), `05_lane.py` (this).

### Design notes (batch 2 on)
- Daemon, git: new `daemon/sessions/src/doc_repo.rs`: init (no commit, local identity, `info/exclude` per clone), `versions(path)`, `read(path, commit)`, `save(message)`, `restore(path, commit)`, `has_commit`. Uses `crate::git::git`; Humata's own commits use `-c core.hooksPath=/dev/null -c commit.gpgsign=false`. Explicit `DocRepoError` enum.
- Daemon, Desk: `start_desk` and channel Desk start init the repo; Add repo appends the clone to `info/exclude`; close / clean-up / channel-desk close treat "has a commit" as kept (DV-4, DV-13).
- Daemon, records: comment records per (session, commit, path) in the store (SEAM: store schema), written by a HEAD watcher in new `daemon/humatad/src/doc_service/`; carried on moves as records (SEAM: carry record list).
- API: new `api/proto/humata/v1/docs.proto` with ListDocVersions, ReadDocVersion, SaveDocVersion, RestoreDocVersion (SEAM: proto, serve.rs, BUILD). MCP tools `doc_versions_list`, `doc_version_read` in new `daemon/humatad/src/mcp/docs.rs` (SEAM: tool registry). desk.proto: the picked folder stays on the wire for old sessions; the app stops sending it.
- App: `clients/src/docs.rs` trait + fake, `clients/live/src/docs.rs`; history state and old-version mode in `features/session` (new `history.rs`); New desk session form drops the folder field (`features/desk`); History strip in new `ui/src/doc_history.rs`, hosted by `ui/src/document_tab.rs` (one hunk).

### Tasks (tests first)
- T1 (Opus writes tests, Haiku implements): `doc_repo.rs` unit tests: init writes only the repo's config (a temp `GIT_CONFIG_GLOBAL` file is byte-identical after); clones are never tracked; versions are per file with who from the trailer; save commits every change outside clones and is refused with none; restore saves unsaved first and only adds commits (every old id still reachable).
- T2: `daemon/humatad/tests/doc_versions.rs`: a new Desk folder is a repo and not kept; a plain `git commit` keeps it (close puts no clock on it); list and read over gRPC; restore as new; a commit records the file's comments and an old version returns them; a move to another machine (fake ssh, as `desk_move.rs`) keeps versions and records; a kept channel Desk folder survives Close.
- T3: MCP tests for the two tools: own folder needs no rule; another session's folder follows FILES_READ_OTHER_SESSION.
- T4: fakes + TestStore tests: stepping ◀ ▶, Save version disabled with no change, old version read-only, restore adds a version; New desk session has no folder field.
- T5: UI behaviour tests in new `desktop/tests/native/doc_versions.rs`, after the frames are signed off.

### Claims (proposed; main to enter in WORKLOG)
New: `daemon/sessions/src/doc_repo.rs`, `daemon/humatad/src/doc_service.rs` + `doc_service/`, `daemon/humatad/src/mcp/docs.rs`, `daemon/humatad/tests/doc_versions.rs`, `api/proto/humata/v1/docs.proto`, `clients/src/docs.rs`, `clients/src/daemon/fake/docs.rs`, `clients/live/src/docs.rs`, `features/session/src/history.rs`, `features/session/tests/history.rs`, `ui/src/doc_history.rs`, `desktop/tests/native/doc_versions.rs`. Hunks: `daemon/sessions/src/desk.rs`, `daemon/humatad/src/desk_service/{start,close,clean_up,view,channel_desk,channel_desk_close,repos}.rs`, `features/desk` (new-session form), `ui/src/document_tab.rs` (host the strip). SEAMs: protos, `serve.rs`, BUILD files, store schema, carry records, MCP registry.

### Frames for the owner (before any golden changes)
Numbers are main's to assign (98 is VIEWS'; 96–97 ROAM's).
1. New desk session (re-draw of 40): no folder choice, no path.
2. Desk session folder bar (re-draw of 37): "Not kept · goes 7 days after close" and "Kept · 4 versions" chips.
3. Markdown tab in Preview with the History strip at "Current · unsaved changes" and the Save version box (message field).
4. History ▾ open: v4…v1 with time, You/claude, message, +/− lines.
5. An old version, read-only, Side by side, with its comments as they were and the version bar (Compare, Restore as new version, Back to current).
6. Compare with current: the diff view with v1 as base.
7. Close confirmation for an unkept folder that holds files: Save version and close / Close.
8. A channel's Desk tab with a kept doc folder under Closed and Documents.
9. Desk page (re-draw of 36): Closed rows showing Kept vs days left.

### Workers
- Batch 2, 2026-11-16 (scripts 10-14; the manager wrote 10 and 13, the tests):
  - Haiku high wrote 11, the doc_repo.rs bodies (T1).
  - Sonnet wrote 12, the New desk session form without a folder choice (DK-14).
  - Sonnet wrote 14, the daemon wiring: init at start, clones excluded, a version keeps the folder,
    and a channel Desk folder stays on Close (T2 phase A). The manager fixed the order of the
    `close_locked` parameters.
- Batch 3, 2026-11-16 (scripts 20-25; the manager wrote 20, the proto, and 21, the tests):
  - Sonnet wrote 22: List, Read, Save and Restore doc versions on DeskService, and `doc_head`
    and `has_versions` on DeskSession.
  - Sonnet wrote 23: a channel Desk session closed in place lists as kept and reopens in its
    folder; the folder repo's identity is the agent's person name, not its id.
  - Sonnet wrote 24: the VIP "Migrate Desk folders" item (D458) for folders made before DOCV.

### Changes and follow-ups (batch 3)
- The version RPCs went on DeskService in desk.proto, not a new docs.proto: no serve.rs or BUILD
  change, and only a Desk folder has versions.
- Desk folders on other machines can wait for SAFE: their versions RPCs refuse with "on another
  machine" (not relayed), and Migrate Desk folders covers this machine only.
- Agent names have two tables. Versions' `who` uses fork_card's ("Codex CLI"). The repo identity
  uses `AgentKind::person_name` ("Codex"). The migration's init still passes the agent id.
  Make all three use one helper.
- `doc_head` only moves on the next list. An agent's plain `git commit` sends no Updated yet:
  the HEAD watcher (with comment records, DV-12) is next.
- Still to come in phase B: comment records (store SEAM), the move test on the rig
  (RIG_TESTS SEAM), records carried on moves (SEAM), and the MCP tools (registry SEAM).

### Batch 4, 2026-11-16 (scripts 30-34, all Sonnet)
- 30: comment records per (session, commit, path) in the store (schema step 31, carried and
  copied with the session); the HEAD watcher (1 s stat poll) records them and sends Updated.
- 31: tests showing a move to another machine or into a channel keeps the repo and its records.
  The move code needed no change.
- 32: the MCP tools `doc_versions_list` and `doc_version_read`, which call DeskService with the
  agent's token under file_read's rule.
- 33: one agent-name helper, `Agents::person_name`, used for versions, repo identity, the
  migration and the fork card.
- Follow-ups: OpenCode now shows as "opencode" (its kind has no display name). Versions and
  the MCP tools for a session on another machine are refused until SAFE. Commits made while
  the daemon was down get no comment record (no backfill).

### Batch 5, 2026-11-16 (scripts 40-45; T4 and T5 on the draft frames)
- 40 (Sonnet): the HEAD watcher pushes through `notify` with no poll, follows session events,
  and backfills records on start. This replaces 30's poll and its no-backfill rule.
- 41 (Haiku): AgentKind gets a display name, and OpenCode shows as "OpenCode" again.
- 42 (Sonnet, T4): DeskClient version calls, the fake and live clients, `DocHistory` in
  features/session, and TestStore tests.
- 43 (Sonnet, T5 A): the History strip, old versions shown read-only with their recorded
  comments, and Compare with current (a diff on the client in the Changes panel).
- 44 (Sonnet, T5 B): the Kept and Not kept chips, the close confirmation (Save version and
  close / Close), and Kept on Closed rows.
- Left after the owner reviews frames 99-107:
  - pixel tuning;
  - "Kept · N versions", which needs a version count on DeskSession (proto SEAM);
  - frame 8, the channel Desk tab, which waits on ROAM's UI;
  - Compare through a daemon `DiffBase::Version`;
  - reducer-level read-only gating of comments;
  - a daemon signal for "holds files outside its clones".
- The native goldens 36, 37, 38 and 40 differ until they are re-drawn.

### Batch 6, 2026-11-16 (scripts 60-63, all Sonnet)
- 60: DeskSession gets `version_count` (13) and `holds_files` (14), a desk.proto SEAM. They
  are filled here, in channel Desks, and on a box's view. The chip reads "Kept · N versions",
  and the close question asks only when the folder holds files outside its clones and .git.
- 61: Compare with current shows the daemon's diff. This is DeskService `DiffDocVersion`, not a
  DiffBase, because DiffBase is per clone with branches and reviewed snapshots and a Desk
  folder has none of them. desk.proto and relay.rs are SEAMs. The diff is relayed to the
  session's machine.
- 62: the app reducer refuses comment actions on an old version, with a toast; nothing else
  edits a Desk file. Side by side, the old version's page follows its source.
- Frame 8: ROAM hosts the channel Desk tab (R2-DESK, `Daemon::channel_desk`), and DOCV adds
  the History strip to its row slot once r2_desk is queued.

### Batch 7, 2026-11-16 (PLACES scripts 70-71, manager-written)
- Already built when PLACES took over, so nothing was redone: the HEAD watcher with
  comment records (DV-12, `doc_comments.rs`, which also covers versions made while the
  daemon was stopped) and the OpenCode display name.
- 70: since DV-13, the channel Desk tab's footnote and Close session no longer say that a
  folder holding a version goes to the trash. In that case the close sheet says the folder
  stays under Closed (`channel-desk-close-kept`) and drops the 30-days-in-the-trash line.
  Two native tests cover this.
- Left on purpose: the design drawing code (`desktop/tests/design/roam.rs:467`) still has
  the old footnote. It is golden code, so it is for main to re-draw.

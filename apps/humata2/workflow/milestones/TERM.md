# TERM: Session tabs run on the reusable terminal in the desktop app

- **Status:** open (2026-11-16, the owner: Codex C1 finished the reusable terminal and left a handoff)
- **Depends on:** FL (the app on the real daemon); DESK* and REVIEW* (they share ui/ and look_test)
- **Unblocks:** V1
- **Layer:** L4 · **Lane:** ui + clients + daemon seams

## Outcome
Every terminal in the app (a session's tab, Open a shell, a desk terminal) is the `terminal_replica` component C1 built: copy/paste, Tab, links, find, resize, scroll and memory saver work as in C1's demo, the terminal survives a viewer refresh or app upgrade without losing its holder, and input is never replayed after a reconnect. The app keeps its own chrome; the component draws only the terminal.

## Exit criterion
- Desktop session tabs, Open a shell and desk terminals render through `terminal_replica` over our daemon's attach transport, on fakes and live.
- A viewer reconnect or app restart reattaches the same holder (same pid/UUID), with output caught up and uncertain input disabled until resynced.
- Native, look and design tests stay green (terminal frames 01–09 within budget); the terminal component's own tests stay green.
- No regression in agent recovery (FORK, SAFE) or STOP.

## Read these (pointers, nothing else)
- `terminal/README.md` (component contract), `terminal/client/README.md`, `terminal/lab/HANDOFF.md`, `terminal/host/{README,TRANSPORT,AGENT_RECOVERY}.md`.
- App today: `ui/src/terminals.rs` (uses `terminal/pane` + `terminal/view` directly), `ui/BUILD.bazel`, the desktop attach path in `clients/live/`, `daemon/holder/` and the daemon's attach service.
- Decisions on terminals: grep DECISIONS.md for holder, attach, terminal.

## Out of scope
Changing `terminal/` itself (C1's; ask main to relay to C1 if the component needs a change). Neovim clipboard. Mosh.
Cut on 2026-11-16 by the owner ("i want this app to be usable"; the app is macOS desktop only): TERM's browser half, ANYHOST and LINUX, so Linux VM runs too (D485).

## Lane (written by the manager)

Manager: minion TERM (Opus 5.5), 2026-11-16. Phase 1 plan; no code yet. Picks came from an Opus 5.5 high advisor (Fable is rate-limited); they go to DECISIONS via main.

### Structure (plain words)
- **One source of truth stays our daemon.** humatad keeps the holders, the store, revive, FORK, SAFE and STOP. C1's `terminal_host` is not embedded and not run beside it. C1 owns its own store and resume adapters, so either option would give two owners of the same holders.
- **New: an authoritative screen per running tab inside humatad.** It is a new crate `daemon/vt_authority`, written on `ghostty_vt`, which is already visible to `apps/humata2` and is `Send`. holders.rs already keeps one link per tab, and that link feeds it. It answers VT queries even when nobody is watching. It records a reply as pending before writing it and clears it on the holder's `InputWritten`. It resizes its own screen when it sends the resize to the holder, and broadcasts `Resized` in order. It debounces checkpoint encoding off the attach task. Scrollback is capped, and the screen is dropped once an exited tab's final checkpoint is taken. Attaches subscribe to this one screen, not to the holder.
- **Wire: the same gRPC Attach, with optional fields added.** The fields are a Ghostty-checkpoint snapshot format, Control, Resized, Detached, ResyncRequired, and InputWritten in place of InputAccepted. The remote relay forwards them untouched. The CLI's raw attach keeps working. A peer that does not send the checkpoint format means resync with input disabled; raw bytes are never replayed into the replica.
- **Client: a per-tab terminal link instead of bytes in the TCA store.** `Terminals::link(session, tab)` gives an ordered stream of `terminal_wire::ServerFrame` and a `ClientFrame` sink. A new single-job crate maps gRPC to and from `terminal_wire` and adapts it to `ReplicaSink`. It is wasm-clean (no tokio) so ANYHOST can reuse it. The fake builds a real Ghostty checkpoint from the fixture bytes at the golden's grid size, sends `Control{controller:true}`, and answers each resize with `Resized`.
- **UI:** `ui/src/terminals.rs` holds one `TerminalReplica` per (session, tab), along with its holder generation, and replaces the replica when the generation changes. The app keeps its chrome, notice, aria text and colours. `terminal_bytes` leaves `features/session` and `clients/workspace`; the store keeps only status, exited, input-uncertain, title and cwd.
- **Writers:** keep D14. Every non-read-only attach is a controller, which keeps goldens 01–09 off the pan container. Only the focused, visible replica sends fit requests.
- **Identity:** replica UUID = UUIDv5(tab id, holder generation). `TabRecord.holder_generation` is new, and revive and FORK move bump it.
- **Recovery:** daemon/revive stays the only recovery authority. C1's AGENT_RECOVERY and FAILURE_MATRIX are used only as a test checklist.
- **Memory saver:** on by default, with a 30 s delay for tabs not visible in any pane. It pins tabs with unflushed or uncertain input and gets a Settings switch. If restoring a 10k-line tab takes more than about 50 ms at p95, raise the delay.
- **Scope:** desktop only. The browser half and ANYHOST are cut (2026-11-16, D485). The mapper crate stays wasm-clean, so a browser
  can still reuse it if the cut is reversed.

### What stays
The holder binary and protocol, revive, the store (except one TabRecord field), the remote relay's shape, the CLI's raw attach, the app's chrome, and the goldens.

### Tasks (failing tests first; smart agents write the tests, Haiku/Sonnet workers implement)
1. A tab's screen in the daemon matches what the holder printed, answers DA/DSR queries with nobody attached, and never resends a reply after a restart (crate tests on `vt_authority`, new crate, no seams).
2. An attach gets a Ghostty checkpoint the replica can decode, then ordered Output and Resized, and input is acknowledged as InputWritten (humatad attach tests; proto fields and serve wiring go to main as seams).
3. A revived or moved tab gets a new holder generation, and a stale replica is replaced, never fed the new process's offsets (store and revive tests; schema and records lines via main).
4. The fake and the live client give each tab an ordered frame link, and a reconnect never replays unacknowledged input (clients and clients/live tests, plus a new mapper crate with its own tests).
5. Session tabs, Open a shell and desk terminals draw through `terminal_replica`, and goldens 01–09 stay within budget (desktop native, look and design tests on fakes; ui seams via main).
6. Bytes leave the TCA store, and features/session, desktop native and live_flows tests move to the link (features/session tests updated, not weakened).
7. `humata attach` in the CLI does not double-answer terminal queries: the CLI filters reply sequences out of raw-attach input (cli tests).
8. FORK, SAFE and STOP stay green: the STOP sweep drops screens without waiting on a pending reply, and SAFE's power-cut suite covers the pending-reply record (existing suites plus one new case each).
9. A live run on the real daemon reattaches the same holder after an app restart, catches up the output, and keeps input disabled until resync (live_flows, headless).

### Risks
Daemon memory and CPU with one screen per running tab (capped scrollback; checkpoints off the hot path). Golden pixels: cursor and blink state in checkpoints, and the first fit before Resized. Mixed daemon versions over the remote relay. The CLI's double replies. The pending-reply record under SAFE: checkpoints are not persisted at first, and after a daemon restart the screen is rebuilt from the holder's kept bytes, or ResyncRequired is sent.

### Requests for C1 (via main; none of them block us)
1. Widen the visibility of `terminal_replica` to `//apps/humata2/ui` and the new mapper crate, and of `terminal_wire` to the mapper crate, `clients/...` and `daemon/vt_authority`.
2. Confirm that `controller=true` renders pixel-identical to a bare `TerminalPane`.
3. Make the replica send fit requests only when it has focus, or make that an option.
4. Say whether a new UUID under the same key needs a new entity or can use a `rebind`.
5. Let ReplicaConfig take default terminal colours that are reapplied on each Snapshot install.
6. Optional: extract the reply-watermark state machine into a pure library.

### Claims (asked of main)
New `daemon/vt_authority/`, new `clients/terminal_link/` (the mapper crate), `daemon/humatad/src/attach.rs`, `daemon/humatad/src/holders.rs` (REVIEW batch 2 held it; it needs main's OK), `clients/live/src/terminals.rs`, `clients/src/daemon/fake/terminals.rs`, `ui/src/terminals.rs` (LOOK1 held it), `features/session/tests/terminal.rs`, and this Lane. Seams go to main: `terminal.proto`, `serve.rs`, `store/{schema,records}.rs`, revive, `clients/src/{lib,daemon,model,workspace}.rs`, `features/session/src/lib.rs`, `ui/src/{lib,shell,session_tabs}.rs`, `desktop/BUILD.bazel`, and the CLI attach.

### Batch 1 (2026-11-16; main approved the plan, picks logged as D271)
- Same-version peers (owner, via main): the mixed-version fallback is dropped; UPGRADE force-upgrades peers. A peer whose snapshot has no checkpoint is refused cleanly (`LinkError::PeerVersionUnsupported`, "being upgraded"), never replayed.
- Task 1: new `daemon/vt_authority` (crate `humata2_vt_authority`, private), with `tests/authority.rs` written first. It is fed by output offsets and refuses gaps. It answers DSR/DA with each reply numbered by sequence. `ReplyRecord{answered_through, pending, uncertain}` is what SAFE must save before a reply is written. `resume` answers only queries that were never answered. It also covers resize-at-offset, the changed flag and Ghostty checkpoints.
- Task 4a: new `clients/terminal_link` (crate `humata2_terminal_link`, private and `manual` until C1's wire visibility lands), with `tests/link.rs` written first. It maps the attach to `terminal_wire` frames. State that comes before the screen is held back until the screen is installed. TerminalState's size becomes Resized, Writable becomes Control/Exited, InputAccepted becomes InputWritten, SnapshotRequest means reopen from 0, and Detach closes. It is wasm-clean (prost + wire only).
- Seam S1 (main): additive proto fields, with no new oneof arms. They are `AttachOpen.checkpoint_engine=9`, `TerminalResize.cell_width/height=3/4`, `TerminalSnapshot.engine=9, terminal_uuid=10` and `TerminalState.cols/rows/cell_width/cell_height=5..8`. It also fixes the 10 struct literals that have no `..`.
- Scripts: scratchpad `term/b1/b1_new_crates.py` and `term/b1/s1_proto_seam.py` (both `--check`).
- Fallbacks if C1 requests 3 and 5 don't land: the ui's sink adapter forwards Resize frames only from the focused, visible replica, keeping the latest. The ui reapplies `set_default_colors` on the pane whenever the replica reaches Live after a Snapshot.
- Next, batch 2: humatad wiring of the authority into holders.rs and attach.rs (task 2, with SAFE/STOP seams via main), and `Terminals::link` plus the fake that builds checkpoints (task 4b).

### Batch 2 (2026-11-16)
- Task 2: `holders.rs` now keeps one `vt_authority` screen per linked tab and writes query answers back as input with their own seq. Attaches with a `checkpoint_engine` get a Ghostty checkpoint, and TerminalState carries the size. A resize goes authority -> holder -> every attach. A foreign engine gets `PeerVersionUnsupported`. New suite `daemon/humatad/tests/checkpoint.rs`.
- Task 3: `TabRecord.holder_generation`, store migration 21. Revive bumps it whenever the holder pid changes. Moves already mint new tab ids. `terminal_uuid(tab, generation)` uses SHA-256 with v8 bits, because `uuid` has no v5 feature (seam request: enable `v5`).
- Task 4b (deferred until C1's wire visibility lands): `Terminals::link` (seam), a fake link from a real checkpoint, and a live link whose gate refuses input across reattaches.
- Open: the attach still sends an empty `terminal_uuid` (batch 3 wires `terminal_uuid`). The ReplyRecord is not persisted. A screen starts at the first link, not at tab open.

### Batch 3 (2026-11-16)
- The attach and checkpoints carry `terminal_uuid`. The reply record is saved on the tab (migration 22) before an answer is sent, and a screen starts at boot and at tab open (D292).
- The CLI's `humata attach` filters out query answers.
- `stop` waits for a pending reply.
- The ui draws each tab through `terminal_replica` fed by `Terminals::link`, with `FitPolicy::Controlling` (revisit in ANYHOST). A tab's replica is replaced when its link ends, and the live link ends when the terminal UUID changes.
- Terminal bytes no longer go through the TCA store: `terminal_bytes`, `TerminalData`, `Terminals::input` and `Terminals::resize` are gone. Typing goes over the link.
- Holder shutdown gives up after 3 s. Every humatad stop path (desk close, tab close, session stop, archive and move, channel delete, carry release) then ends the holder by pid through `holder_end::shut_or_end`.
- Revive and launch save a fresh reply record with each new holder generation, in the same transaction as the tab.
- The live link admits typing only once the screen has arrived. A real-daemon test covers an app restart: same holder, output caught up, typing after the screen.
- Memory saver: hidden terminals free their screen after 30 s, on by default, with a switch in Settings > General (P8).
- Open:
  - revive/reconcile.rs and sessions/own_copy.rs ignore shutdown errors and never end the holder by pid.
  - On resume or restart, the holder is spawned before the tab is saved.
  - Title and Pwd steps are ignored.
  - Checkpoints over 3 MiB fail with Internal.
  - No test covers the lagged-subscriber resync.
  - An attach keeps a stale `Arc<Link>`.
  - Seam request: enable `uuid`'s `v5` feature.

### Batch 4: gate-20 terminal failures (2026-11-16, script `term/s1/apply.py`)
- Typing clears "Reconnected": keys go through the replica's PTY writer, which never emits `InputQueued`, so only pastes cleared the line. Now the ui `Sink` clears it when an Input frame leaves, and the pump redraws the shell on the next `InputWritten`.
- The paste test's "here" tab is `refund` tab 0. The design fixture now has `refund` on this Mac and `rate` on devbox-1 (SEAM, test only).
- Resize after find: not diagnosed without a run. The test now asserts that Escape closed the find bar, and prints both sizes (SEAM, test only). The next run says whether the find bar or the refit is at fault. Sibling `a_terminal_sizes_its_program_only_while_it_has_focus` passes, so the Shown/focus path works without a tab switch plus find.
- `session::prepared_views_wait_for_open_and_later_returns_on_the_next_visit` (session.rs:423) fails before any terminal step (`prepared-view-content` missing after Open it). It is VIEWS', not TERM's.
- Next: the live terminal harness over a real holder (WIRE-A `term_rows.tsv` rows: typing at the shell first, then reading output).

### Batch 5: typing at a real shell, live (2026-11-16, scripts `term/t1/apply.py`, `term/t1/retag.py`)

- New `desktop/tests/live_flows/typing.rs` (SEAM: one `mod typing;` line in `live_flows.rs`). It runs four tests in the headless app against a real `humatad2`, holder and `zsh -f`. Keys are pressed as the window gets them, and the shell's own answer is read off the drawn grid: `| tr a-z A-Z` capitals, which the typed line never holds.
  - Editing keys (micro 01 steps 2–13, micro 03 ⌥←/⌥→/⌥⌫).
  - History keys (micro 05 ⌃R, ⌃P/⌃N).
  - Control keys (micro 08 ⌃C flood, ⌃D on `cat` and on a typed line, ⌃Z/`fg`, ⌃\).
  - vim and less (micro 15 Esc, ⌃Z/`fg`, `:wq`, less `/` and `q`).
- These tests are failing-first: they have not run yet. Any key the app takes before the terminal does (⌃A, ⌃L, Esc and the like) shows up as a timeout, and gets fixed in t2.
- `retag.py` sets 28 term_rows to Works (real shell), in both the step line and the summary row. Run it only after the four tests pass. `--only NN:STEP` retags a subset. It leaves the README counts alone.
- Next: the rows that need a second viewer, a link drop, paste or the mouse on the live rig.

### Batch 6: gate fixes and the screen of a real shell (2026-11-16, scripts `term/s3/apply.py`, `term/t2/apply.py`, `term/t2/retag.py`)

- s3 (batch-3 list, queued for round 23): a holder that is spawned and then fails is ended (`ReviveError::EndHolder`, `OwnCopyError::EndHolder`); the holders.rs link cache keys on socket and pid; a reader that fell behind catches up from the scrollback. Title and Pwd stay dropped until s4.
- D480 and D481 are provisionally yes (owner's next wave). s4 builds both: `program_title` and `pwd` on the tab record (migration 23; the user's tab name is never overwritten), and an over-limit checkpoint trims the oldest history with a `Trimmed` event.
- t2: new `desktop/tests/live_flows/shell_screen.rs` (SEAM: one `mod shell_screen;` line). It shares t1's zsh rig, so t1 goes first. Three failing-first live tests:
  - pastes: a hidden end marker, the multi-line prompt's Cancel and Paste, and a 3 MB paste refused whole (micro 06, 07);
  - history: wheel, hold while output streams, ⌘Home/⌘End, ⌘F count, ⌘G/⇧⌘G, a broken regex, Esc, and ⌘K into the palette (micro 02, 10, 21);
  - mouse: drag and double-click copy, ⌘C with nothing selected, SGR click and wheel reports, ⇧ drag, and the wheel in `less` (micro 11, 14).
- `t2/retag.py` sets 21 rows to Works (real shell) and rewrites the Gaps lines they settle. `--only NN:STEP`, `NN:B<n>` for branches, and `NN:gaps`.
- Next: link drops, two viewers, then IME. E2E-BUGS handed 17 Likely bug rows here; they're checked in the same pass.

### Batch 7: the E2E-BUGS rows (2026-11-16, script `term/t3/retag.py`)

- E2E-BUGS handed 17 Likely bug rows. TERM read each fix in the code and checked the tests it cites. No gate is needed: every cited test is already in the tree.
- 15 rows go to Fakes only:
  - 02/4 ⌘A, 11/9 Edit > Copy, and 06/5 Edit > Paste (an extra row);
  - 12/1 and 12/4 links;
  - 06 exited-shell branch and 19/1;
  - 14/5 ⇧ wheel;
  - 13, composing while disconnected, and 13, the candidate list;
  - 10, ⌘PgUp in a full-screen program;
  - 17 B1, typing during catch-up;
  - 20/3, the writers count (app side).
- Two rows become designed behaviour, with new decisions (Provisional, recommendation: keep):
  - D482: `:` stays a word boundary (12/3).
  - D483: a killed program's mouse mode stays on for the shell (14 branch).
- 14 Gaps lines are rewritten.
- Held back: micro/26 and macro/07 step 10 wait for their gate-20 failures to go green. micro/22 belongs to WIRE-A.
- s4 is delivered (`term/s4/apply.py`, 27 changes):
  - D480: migration 32 adds a `tab_program` table (title and pwd from OSC 2 and OSC 7), proto Tab fields 8 and 9, and a revived shell starts in the saved folder.
  - D481: `Terminal::checkpoint_trimmed` and `Snapshot.trimmed_rows`.
  - Not built yet. The first gate check is `checkpoint_trim_test`: does lowering the history limit evict at once?
  - Follow-ups: the UI subtitle; new shell and move using the saved folder; whether the last_good and upgrade table lists need `tab_program`.
- Next: link drops, two viewers (that live test promotes 20/3), then IME.

### Batch 8: link drops, two viewers and input methods, live (2026-11-16, scripts `term/t4`, `term/t5`, `term/t6`, each `apply.py` then `retag.py`)

- t4 (applied by main): `live_flows/link_drop.rs` runs on a host linked to devbox-1. It cuts and restores the link. While the link is down, keys raise one "not sent" toast and never arrive. Once it is back, output printed meanwhile shows and the same shell takes typing. The retag sets micro/16 steps 3, 4, 5, 7 and 8 to Works (real shell) and waits for `link_drop::` to pass.
- t5: `live_flows/two_viewers.rs` opens two headless apps on one real shell tab. Both type, and both show both answers. While the session works (`humata turn working`), each bar says "1 other can type here". When B closes, A's bar drops the line. Needs t4 for `shells_of`. The retag sets micro/20 steps 1, 2 and 3 and B2 to Works (real shell). Run it after t3's retag: t3 refuses once t5 has changed its 20/3 line.
- t6: `live_flows/ime.rs` composes `にほんご`, sends Enter mid-composition and commits `日本語`. It also pastes the family emoji and a flag, types `سلام`, and has `zsh -f` read the bytes back with `xxd -p`. It then checks that what the shell prints from octal escapes is drawn. Composition goes through the new `Shell::terminal_input` (D484, Provisional, recommendation: keep). The retag sets micro/13 steps 1 to 7 to Works (real shell), with no real input method.
- Watch at the gate:
  - t5: whether dropping viewer B's headless context closes its link (else A's count stays at 1), and whether `SelectTab(n)` focuses B's pane for typing.
  - t6: whether `en_US.UTF-8` makes `zsh -f` take multibyte input, and whether a wide letter shows as its own text run.

### Batch 9: what was left for the desktop (2026-11-16, scripts `term/t7plus`)

- Already built before this batch:
  - link drops (t4, applied);
  - two viewers and input methods (t5 and t6, round 24);
  - FitPolicy (`const FIT: FitPolicy = FitPolicy::Shown` in `ui/src/terminals.rs`);
  - checkpoint trim (s4, D481);
  - the 17 Likely bug rows (t3: 15 set to Fakes only, D482 and D483).
- `find_escape.py` (round 25, SEAM in `ui/src/terminals.rs`): Escape in the find bar now closes it. The app's `Dismiss` took every Escape first, the text box included. The terminal body now closes find on `Dismiss` and passes any other Escape on.
- `snapshot_tab_program.py`: dropped, because it duplicated main's `fix23/fM_store_tab_program.py`.
- `bug_rows.py`: three live tests in `live_flows/shell_screen.rs`:
  - ⌘A and Edit > Select All/Copy/Paste
  - an exited shell
  - the ⇧ wheel and ⌘PgUp in `less`
- `retag_live.py` sets 02/4, 06/5, the 06 exited-shell branch, the 10 ⌘PgUp branch, 11/9, 14/5 and 19/1 to Works (real shell). Apply it once `shell_screen::` passes.
- Links (12/1, 12/4) and 17 B1 stay Fakes only: a headless run cannot see `open_url`.
- Watch:
  - Escape reaching vim. `Dismiss` may take it before the terminal's key handler, the same cause as the find bar. Main runs `typing::vim_and_less_take_their_keys_from_the_terminal` after round 25.
  - If that test fails, apply `vim_escape.py`. An Escape that finds nothing to dismiss is counted (`AppState::unused_escapes`), and the Shell then passes it on to what has focus. The native test is `escape_with_nothing_open_reaches_the_terminal`.
- The browser half is cut (D485), and so is the LINUX list below.

### LINUX batch list (deferred: LINUX is cut, D485; kept for if it comes back)

- micro 01 and 03: the same editing keys in bash; ⌥ as Alt.
- micro 02 Ctrl+U branch; micro 06 step 1 Ctrl+Shift+V; micro 10 ⇧PgUp/⇧Home/⇧End and Ctrl+Shift+F; micro 11 Ctrl+Shift+C and Ctrl+C with a selection; micro 21 Linux Ctrl+K.
- micro 05 and 08: ⌃R/⌃P/⌃N and the control keys in bash.

## C1's answers (C1 takeover, 2026-11-16)

Codex C1 stopped, so a takeover agent answered through main. The changes are in the C1 takeover scripts that main applies (`c1_02_visibility.py`, `c1_03_replica_options.py`).

1. **Visibility: done.** `terminal/wire:wire` is now visible to `clients`, `clients/live`, `clients/terminal_link` and `ui`. `terminal_replica` and `terminal_replica_fast` are visible to `terminal/web` and `ui`. Drop `manual` from `clients/terminal_link`. `daemon/vt_authority` uses only `ghostty_vt`, so it got nothing; ask again if it starts using the wire.
2. **`controller=true` draws the same pixels as a bare `TerminalPane`, once the grid has the fitted size.** With control and a fresh screen, the replica returns the pane straight inside a plain full-size div. That div paints nothing and has no pan container; the offline banner is off and shortcuts are actions only. The component test `a_controlling_replica_draws_the_same_pixels_as_a_bare_pane` compares every pixel with a bare pane in the same 800x600 headless window. The condition: the replica keeps the checkpoint's grid size until the host's `Resized` arrives, because it never resizes its own grid. Before that it can differ from a bare pane, which resizes itself. The fake must build checkpoints at the golden's grid size or answer the first Resize before a frame is captured, as the plan already says. A replica without control, or with a stale screen, uses the scrollable pan container on purpose.
3. **Option: `ReplicaConfig::fit: FitPolicy`.** `Controlling`, the default, keeps today's behaviour: it sends Resize whenever the replica holds control. `Focused` sends Resize only while the replica holds control and has focus, and asks for a fresh fit when it gains focus. A hidden replica never sends input or Resize either way. Use `Focused` for the app's tabs; the sink-adapter fallback is then not needed. With `Focused`, a terminal nobody has focused keeps the holder's size; focus the tab that opens.
4. **A new UUID needs a new entity; there is no rebind.** `bind_connection` with another UUID on the same route unbinds and returns `ReplicaError::WrongIdentity`. Checkpoints carry and check the UUID. Drop the old entity and create a new one, as `terminal/client`'s workspace does when a route's UUID changes. The test is `reused_route_cannot_bind_another_uuid_or_restore_its_authority`. So a new holder generation means a new `TerminalReplica`, which matches the plan.
5. **Done: `ReplicaConfig::default_colors: Option<ghostty_vt::Colors>`.** The replica applies them to every checkpoint it installs, both host Snapshots and `restore_checkpoint`. `TerminalReplica::set_default_colors(Option<Colors>, cx)` changes them for the screen showing and for later checkpoints, for an appearance change. A refusal is `ReplicaError::Colors` and changes nothing. Colors a program sets (OSC 4, 10, 11, 12) still win. The ui fallback of reapplying them after each Snapshot is not needed.
6. **Not extracted.** `daemon/vt_authority`'s `ReplyRecord` is the only user, since a replica never answers queries (its `reply` drops engine answers). Extract it when a second user appears.
- **Holder protocol:** it stays at `VERSION` 1. The keyed-input additions are additive, and this pass changed nothing in `daemon/holder`.
- **`ESC[c` with nobody attached:** no device-attributes callback is needed. libghostty-vt 1.0.1 answers `CSI c` with `CSI ? 62 ; 22 c` through the write-PTY callback that `terminal/vt` installs, and `vt_authority`'s `device_attributes_are_answered_with_nobody_watching` passes.

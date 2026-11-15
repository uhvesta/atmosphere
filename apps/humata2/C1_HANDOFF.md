# Humata 2 native UI handoff (minion C1)

## Current State Summary

Wrapped at the owner's explicit request on 2026-11-15. The original native-app/frame-matching goal is **unfinished**. The checkpoint preserves review comments in Humata's outbound buffer, with repository/base/quoted-line metadata, retry, removal back to drafts and restart compatibility. The UI now says **Waiting to send**, referring only to Humata-owned input that has not reached the terminal. It cannot observe the CLI's own queue.

**Latest owner direction is documented but partly unimplemented:** allow individual-comment Send as well as Send N comments, and let explicit Send write terminal input without mandatory turn-end detection. Current Review Send is still batch-only and retains the previous hook-end hold policy. Reconcile that first; do not describe the checkpoint as the finished send UX.

Checkpoint commit: `9e185a5b`; the accompanying hash-fill commit names it in GAPS_REVIEW. Publish destination is shared remote main; verify ancestry when resuming. Previous published C1 foundation: `491a1cdd` and `7c5de8fe`. This handoff is under the app directory because the task prohibits touching the shared checkout's .claude directory. The session-handoff scaffold was generated in a private temporary directory only.

## Important Context

- Owner explicitly requested terminal mouse interactions and keyboard interaction **only while that terminal is focused**. SPEC SE-10 and DECISIONS D126 record this. Existing TerminalPane handles click-to-focus, selection, scrolling and application-requested mouse reports. Shift preserves local selection. A new native regression verifies palette keystrokes cannot reach the terminal and clicking back restores input; pane tests also cover find-field focus and focus reporting.
- The app embeds each agent's terminal/TUI or a shell. New session's redundant model/Opus field was removed in published work; agent defaults apply. Do not reintroduce a model control without new owner direction.
- The owner approves **all publishing** in this chat. Do not ask again for each pair. No AI attribution or Co-Authored-By/Claude trailers. Only the task's `(minion C1)` attribution is used.
- Build, test, bench, lint and formatting use Bazel only. No cargo, dependency repins, new lockfiles, bazel clean or shutdown. Headless UI only; never open real windows.
- This is a shared worktree. Other agents have uncommitted daemon, CLI, prototype, workflow and design work. Preserve it. Never stage broad directories or assume a dirty tree belongs to C1.
- Direct owner direction takes precedence over frames; frames govern appearance, SPEC governs behavior otherwise. Whole-prompt fidelity remains the priority. Half/full resizing is secondary.

## Immediate Next Steps

1. Read this file, OVERVIEW's **Pick up here (minion C1)** and latest C1 GAPS_REVIEW entries. Inspect status/log before editing; do not rerun old temporary commit scripts.
2. Add an individual-comment Send action/control. Retain unsent newer drafts when transferring only the chosen comment. Keep **Send N comments** as the batch choice. Implement explicit Send as terminal delivery without requiring a turn-end hook; optional wait mode must be an explicit choice. Remove the checkpoint's mandatory hold assumptions from reducer/UI/tests while preserving payload retention and recovery.
3. Prove individual and batch behavior, focused keyboard routing, error/retry without loss, origin-session/repository scoping, navigation during send and restart compatibility. Keep the app buffer separate from CLI processing claims. Record headless native/panels/shell references, inspect light/dark and run the full Bazel gate below.
4. Continue prepared-view Keep/Revert/Showing/walkthrough work, header actions and the per-frame audit. New D100 Desk/empty/markdown screens34–43 are not implemented in the real app. Check current approved scope: concurrent design owners added44–55; C1 has not established those approvals and must not alter their files.
5. Publish verified C1 work in the required code/log pair and update OVERVIEW immediately. Do not mark the larger goal done merely because the gate passes.

## Architecture Overview

The desktop is the real GPUI shell on fake clients; UI lives in ui, framework-free reducers in features, and fake data/transport in clients. TerminalPane renders ANSI/VT output and encodes raw input through the fake terminal client. No real PTY or agent process is launched by this desktop integration yet. Real daemon/holder work belongs to another lane.

Review captures saved drafts. App transfers a submitted batch into Session's saved outbox and removes only that batch from Review in the same parent reduction. Session keeps payloads until client acknowledgement, pauses automatic retry after failure, and restores removed comment batches to Review. Successful fake-client delivery writes formatted comment input and updates scoped sent metadata.

## Critical Files

| File | Role |
|------|------|
| apps/humata2/OVERVIEW.md | Authoritative C1 pickup and detailed01–33 frame audit; preserve other agents' pickup blocks. |
| apps/humata2/features/review/src/lib.rs | Saved draft capture, repo/base/new-line quote and batch Send. First place to add individual send. |
| apps/humata2/features/app/src/messages.rs | Parent-owned atomic Review → Session transfer; refuses missing origin sessions. |
| apps/humata2/features/session/src/message.rs | Untagged saved text/comment message; old string queues still deserialize. |
| apps/humata2/features/session/src/lib.rs | Saved outbox, delivery acknowledgement, hold policy, failure and draft-return delegates. |
| apps/humata2/ui/src/panels.rs | Diff comment cards, batch footer and waiting comment display. |
| apps/humata2/ui/src/session_tabs.rs | Waiting-message bar/list, counts, per-message Send now/Remove. |
| apps/humata2/ui/src/terminals.rs | Cached panes, raw input/resize dispatch and terminal focus targets. |
| apps/humata2/terminal/view/src/view.rs | Grid-scoped key/input handlers, mouse and focus tracking. |
| apps/humata2/terminal/pane/src/host.rs | Terminal input encoding, mouse protocols, paste and focus reporting. |
| apps/humata2/features/app/tests/messages.rs | Nine behavioral transfer/restart/failure/scoping cases. |
| apps/humata2/desktop/tests/native.rs | Real shell headless flows including palette input isolation. |

## Files Modified

The checkpoint changes the files above except the terminal view/host and ui terminals files, whose support already existed. Additional changes:

- clients model: optional serde-default CommentContext and whole-batch formatting.
- fake review client: scoped comment metadata and atomic rejection of ambiguous legacy paths across repos; unchanged retry cannot fix missing repository context.
- fixture design_session/diffs: old comments explicitly have no context.
- clients fake_daemon tests: failed ambiguous batch leaves the complete workspace unchanged; scoped retry succeeds only in its intended repo.
- Session queue tests and desktop panel/native flows adapt to typed retained messages.
- Native PNG/view references refreshed for Draft/Waiting labels, singular counts and outbox helper; four new review queued-light/queued-dark/retry/sent captures.
- SPEC, DECISIONS, OVERVIEW, GAPS_REVIEW and this handoff record the owner requirements and implementation gap. No design sources/PNGs or real daemon files belong to this checkpoint.

## Decisions Made

- Keep structured comment payloads until delivery acknowledgement so failed sends can retry and removal can restore editable drafts. Saved text queues retain their old representation.
- Capture repo/base/quoted source when the comment is saved, so later navigation does not silently retarget it. Current anchors cover new-side single lines only.
- Use **Waiting to send** for Humata's own outbound buffer; count comments in a comments-only batch rather than counting one opaque message. Never infer agent processing from transport success.
- D126 records owner terminal focus/mouse and individual/batch delivery direction. The exact expanded outbox appearance stays Provisional D114; Fork layout stays Provisional D102.

## Verification

Final command passed **all23 test targets (52 total targets)** on 2026-11-15, including native36, panels12, shell2, App messages9, Session queue4 and standalone pane tests:

```sh
export PATH=$PWD/.tools/bin:$PATH
bazel test //apps/humata2/desktop/... //apps/humata2/ui/... //apps/humata2/features/... //apps/humata2/clients/... //apps/humata2/terminal/pane:pane_test --test_output=errors
```

Full native35/panels12/shell2 records passed after Waiting to send wording. Then the added focus-isolation test passed both alone through native_test_record and in the final native36 gate. Touched Rust was formatted through Bazel. No real windows opened.

```sh
bazel run //apps/humata2/desktop:native_test_record
bazel run //apps/humata2/desktop:panels_test_record
bazel run //apps/humata2/desktop:shell_test_record
```

Inspect actual captures in desktop/tests/__snapshots__/native alongside approved design references. C1 inspected the updated waiting-comment light/dark, expanded outbox, retry/sent and relevant01/03/11 appearances. Remaining fake terminal tones/prompt, sample counts, controls and spacing differ; see the full audit in OVERVIEW. Passing snapshot tests does not mean every approved frame matches.

## Remaining Work and Boundaries

- First reconcile individual/batch direct Send per the latest owner direction. Current native captures visibly retain “after this turn”; that is checkpoint behavior, not the final requirement.
- Prepared views currently show a plain body tab. Keep/Revert/Showing/walkthrough are missing; titles are used globally as identity, risking cross-session collisions. Use scoped stable identity and restore saved presentation without replacing new queues/work.
- Inbox answer state cannot distinguish still-waiting agent calls from already-pending calls; do not invent always-queued approval lines.
- Header ellipsis currently opens the palette. Actual Rename/Move/Stop/PR/Archive menu is missing.
- Full review/composer, old-side/range anchors, editor/markdown/PR comment producers, general re-anchoring and broader Desk/markdown UI remain.
- Real PTY/agent transport, durable deduplication, bracketed-paste serialization among writers and presence events are outside the fake integration. Fake raw history remains unbounded; OSC title/pwd/clipboard-read/file/image actions need work.
- Never edit real daemon/store/revive/machines/MCP/CLI/API/proto, MODULE/lockfiles/rustfmt config, design sources/PNGs, ui-prototype, prototype index or .claude. Fake clients under clients/src/daemon/fake are in scope.

## Assumptions Made

The owner requested wrapping now so another agent can continue. Therefore preserve and publish the tested foundation, explicitly document the newest send behavior as pending, and stop this session without claiming the original goal complete. No separate owner approval is needed for continuing the recorded work or publishing verified changes.

## Potential Gotchas

- Headless windows begin inactive, which suppresses focus observers. Focus tests activate only the headless window; never a real app window. Resize also needs bounds_changed so GPUI uses the new viewport.
- Parent child reductions happen synchronously. Expect the incoming delegate and inspect resulting state rather than waiting for an emitted child action that does not exist.
- A missing concurrent44 design reference caused an earlier test failure; its owner supplied it and the final gate passed. Never record design_test from this lane.
- The P0 refactor moved App helpers, fake clients, fixtures and overlays into modules. Avoid recreating old monolithic paths/imports.
- Stage shared markdown from HEAD plus only C1's transformation using hash-object/update-index, then apply the same edit to the working copy. Preserve other agents' changes. Check the index is empty first, stage explicit owned paths, inspect the staged diff and use the next free decision number.
- Use git's temporary LFS storage override `/tmp/humata2-minion-c-lfs`. Push with the existing uhvesta identity and existing credential-helper invocation; never print credentials, switch global config or force-push. Verify remote main after each pair. A concurrent push may already include C1 commits; inspect ancestry before retrying.

## Environment State

Worktree: /Users/avestabarzegar/oss/atmosphere-khodr; branch atmosphere-khodr. PATH must include this checkout's .tools/bin. Shared Bazel server remains available; do not stop it. No C1 test process remains running after the completed final gate. Other agents may still be active. Token variable names only: GH_TOKEN is set transiently for the authorized push, never stored in docs.

## Related Resources

Read SPEC SE-10/SE-23/SM-9/SM-10, DECISIONS D76/D100/D102/D114/D126, LAYOUT and TESTING. OVERVIEW retains the detailed per-frame matches/partial/not-started list; GAPS_REVIEW holds each implementation commit and its verification. The original task's C1 scope and conventions remain in force.

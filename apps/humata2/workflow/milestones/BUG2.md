# BUG2: Report, annotate and draw from anywhere

- **Status:** open (lane BUG2, 2026-11-15)
- **Depends on:** BUG (done, ed5882ed); LOOK2* (shares ui files: palette, overlays; claim around LOOK2's claims)
- **Unblocks:** LINUX (its suites join the batch)
- **Layer:** L2 · **Lane:** BUG2

## Outcome (owner request 2026-11-15)
From the command palette the person can start/stop a recording, take a snapshot, open the bug report form and open the last report. On a snapshot they can annotate (comments pinned to elements) and draw freehand marks over the window. An agent reading the report sees the annotations and drawings as text: each stroke as a path in window points plus the elements it covers (from the view tree), each comment with its element path. Recordings also carry the daemon RPCs the app makes on the live client.

## Exit criterion
- Palette commands exist (features/palette) and act like the menu items; TestStore tests.
- Draw mode on a snapshot: pen, undo, clear, done; strokes saved in the report folder as JSON and summarised in the copied prompt ("a circle around `Shell › Sidebar › Row[Refund retries]`"); headless flow test that draws, copies and checks the prompt names the element.
- Annotations: add/edit/remove comments on pinned elements from the form; in the prompt.
- Live client RPCs recorded (method, timing, error kind; bodies scrubbed) and replay against fakes still reaches the same end state.
- **Images and video (owner, 2026-11-15, supersedes D31's no-pictures rule):** a snapshot also saves a PNG of the window rendered by the engine itself (the same offscreen path the native snapshot tests use, so no Screen Recording permission), and drawings/annotations as an SVG overlay that references it (plus the text summary). A recording can save a video made from engine-rendered frames (frame sampling at a fixed rate while recording; encoded in-process, no OS capture APIs). The copied prompt names the files. Scrubbing applies: frames are rendered with seeded secrets masked.

## Read these
- Decisions: D31, D47, D133 (grep `^## D<n>:` in DECISIONS.md); BUG.md › Lane for the layout.
- Code: `report/src/{recorder,replay,scrub,prompt,remote}.rs`, `features/app/src/report.rs`, `ui/src/{record,report_view,keys}.rs`, `features/palette/src/`, `clients/live/src/`, `desktop/tests/report.rs`.
- Design: no frame; follow the quiet chrome in desktop/tests/__snapshots__/design; ask a Fable 5.2 high advisor for the draw-mode look and log it as a D-number.

## Out of scope
OS screen capture (ScreenCaptureKit etc.). Changing golden frames.

## Lane (written by the manager)

Manager: minion BUG2 (Opus 5.5). Design picks: D159 (Fable 5.2 high advisor).

### Design notes
- **Palette (features/palette, seam: LOOK2's file).** `Command::Record` becomes the start/stop toggle ("Start recording a bug report" ⇧⌘R / "Stop recording"). New `Command::Snapshot` ("Take a snapshot for a bug report" ⇧⌘S, same as the key), `Command::ReportBug` ("Report a bug…": stops a running recording, else takes a snapshot; the form opens either way) and `Command::OpenLastReport` ("Open the last bug report", listed only when there is one). `PaletteState.report: ReportStatus { recording, last }`, set by `open_palette` before `Open`.
- **Reducer (features/app).** `ReportState` gains `last: Option<ReportForm>` (set on Saved, updated on Close) and `request: Option<ReportRequest { Toggle, Snapshot, ReportBug }>`. `commands.rs` (seam) maps the palette commands to `ReportAction::Request(..)` / `ReportAction::OpenLast`. The ui sees `request`, waits two frames so the palette is gone from the snapshot, acts, and sends `ReportAction::Handled`.
- **Palette gestures stay out of replays.** `Recording::mark()` / `rewind(mark)` in `report/`; the ui recorder marks when the palette opens while recording, and a report command run from it rewinds the palette steps.
- **Draw mode (features/app + ui).** `ReportForm` gains `drawing`, `strokes: Vec<Stroke { shape: Loop|Line|Dot, points, covers: Vec<PinTarget>, more }>`, `places` (each pin target's rect at save time, from the snapshot's layout) and `window`. Actions `Draw`, `Stroke(points)`, `UndoStroke`, `ClearStrokes`, `RemoveStroke(i)`, `DrawDone`; Escape leaves draw mode first. Shape and covers are pure geometry in `features/app/src/report_draw.rs`. The ui paints strokes with gpui paths on a full-window layer that takes every pointer event.
- **Report folder (report/).** `Form.drawing: Drawing { window, strokes }`; `fill_form` writes `drawing.json` when there are strokes (removes it when none); `report.md` gets a `## Drawings` section and a Summary line only when there are strokes, so existing folders read the same. `copy_prompt` now carries the Comments and Drawings sections. Stroke elements are scrubbed like comment elements. The element text keeps the existing `Role "label" in Role "label"` form (the milestone's `›` example is illustrative).
- **RPCs (clients/live, inert file + seams to FL's files).** `clients/live/src/recorded.rs`: a tower layer over the Channel that reports method, duration to response headers and status name (`ok`, or the gRPC code's snake_case name; a transport error is `unavailable`), never bodies. `LiveOptions::observe_calls(..)`. The ui recorder exposes a Send+Sync `RpcFeed` drained into `Recording::rpc` while recording. Desktop wires it (seam in FL's `desktop/src/{main,live}.rs`). Replay on fakes ignores Rpc events, so it reaches the same end state.

### Tasks
- **T1 report crate** (`report/src/{form,write,prompt,scrub,recording,lib}.rs`, `report/tests/`): Drawing types, drawing.json, Drawings section, prompt sections, scrub, mark/rewind. Tests: folder with strokes has drawing.json and the prompt names the covered element; no strokes leaves the folder as before; rewind drops the steps after the mark; scrubbed secrets in a stroke's element.
- **T2 reducer** (`features/app/src/{report,report_draw,overlay}.rs`, `features/app/tests/report.rs`): requests, last report, draw actions, geometry. TestStore tests.
- **T3 live RPC layer** (`clients/live/src/recorded.rs` new, inert; its tests as `#[cfg(test)]` in the file): service_fn mock tests for ok, grpc-status error and transport error.
- **T4 ui** (`ui/src/{record,report_view}.rs`, new `ui/src/report_draw.rs`): request handling, mark/rewind, RpcFeed, draw layer and toolbar (D159), Marks list, copy with the drawing.
- **T5 desktop tests** (`desktop/tests/report.rs`): palette flows, draw → copy → prompt names the element, RPC feed while recording replays to the same state.
- **Seams (main applies):** scratchpad `bug2/seams.py --check`: `features/palette/src/lib.rs`, `features/palette/tests/search.rs` (new tests), `features/app/src/{lib,commands}.rs`, `clients/live/src/{lib,connection}.rs`, `desktop/src/{main,live}.rs`.

### Workers
| Worker | Task | Files | Status |
|---|---|---|---|
| w-t1 (Sonnet) | T1 report crate | `report/src`, `report/tests` | done, checked |
| w-t2 (Sonnet) | T2 reducer + geometry | `features/app/src/{report,report_draw,overlay}.rs`, `features/app/tests/report.rs` | done, checked |
| w-t3 (Sonnet) | T3 live RPC layer | new `clients/live/src/recorded.rs` | done, checked |
| w-t4a (Sonnet) | T4a ui recorder: RpcFeed, palette rewind, places, copy with drawing | `ui/src/record.rs` | done, checked |
| w-t4b (Sonnet) | T4b draw layer, toolbar, Marks, palette requests | `ui/src/report_view.rs`, new `ui/src/report_draw.rs` | done, checked |
| w-t5 (Sonnet) | T5 desktop flows | `desktop/tests/report.rs` | done, checked |
| w-a (Sonnet) | W-A engine PNG of the window, secrets masked in pixels, marks.svg overlay (owner scope change 2026-11-15) | `ui/src/record.rs`, `report/src/*`, `report/tests/*`, `report/BUILD.bazel` | done (record.rs edits went in live before the seams rule; video wiring in seams2.py) |
| w-b (Sonnet) | W-B video: APNG pipeline + sampler (D160) | new `report/src/video.rs`, `report/tests/video.rs`, `ui/src/record_video.rs` | done (wiring in seams2.py) |

### Close check (minion BUG2-LOOK2, 2026-11-15, after g74)
- Every seam (seams.py to seams4.py) is in the tree; g74 passed all of BUG2's suites (`report:*`, `features/app:report_test`, `features/palette:report_commands_test`, `clients/live:live_unit_test`, `ui:ui_unit_test`, `desktop:report_test`).
- Found and fixed: the report named a recording's video by placeholder names (`recording.mp4`, `.webm`, `.gif`) the app never writes, so `report.md` left out `video.png`/`video.jsonl` and the prompt listed them with no description. `report/src/write.rs` now lists `video.png` and `video.jsonl` (names from `report/src/video.rs`) with what they hold. Test first: `report/tests/picture.rs` `a_recordings_video_is_named_with_what_it_holds_in_the_summary_and_the_prompt` (replaces the placeholder `recording.mp4` test). `report:{picture,prompt,video}_test` pass.
- **GAPS_REVIEW note:** BUG2 close pass: the report and the copied prompt name the recording's real video files (`report/src/{write,video}.rs`, `report/tests/picture.rs`).
- **Left, outside the exit criterion:** round stroke caps, RPC status sent in trailers after a stream starts, a `humata report draw` CLI (OVERVIEW › Pick up here (minion BUG2)).
- **Ready to commit:** yes.

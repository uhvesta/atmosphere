# TIDY: No leftovers from dropped designs in the code or API

- **Status:** done (commit: this one; batch 2 4a636632)
- **Depends on:**
  - W1: deleting an API and folders can't be undone; it waits for the owner to confirm D104 and D9
- **Unblocks:** V1
- **Layer:** L1 · **Lane:** daemon

## Outcome
An agent or CLI user is never offered spec mode, and a reader of the tree finds no dead Swift-era app.

## Exit criterion
`SetSpecMode` and `spec_mode` are gone from `sessions.proto`; the MCP tool `session_spec_mode` and `humata session spec` are gone; `api/MCP.md`, `api/CLI.md` and the SM-1, SM-2, SM-5 rows of `api/TRACE.md` are updated; boundary suites pass. If the owner says so under D9, `app/`, `bindgen/`, `core/` and `rehearsal/` are deleted in their own commit. A grep for wording the tidy retired ("Always let", "Widen", "Approve" as a button) in `api/` and user-facing strings comes back clean.

## Read these (pointers, nothing else)
- SPEC: SM-3, SM-4, SM-6 (what replaced spec mode); the dropped IDs are SM-1, SM-2, SM-5. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: none
- Decisions: D104, D9 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `api/proto/humata/v1/sessions.proto`, `daemon/mcp/src/tools.rs`, `cli/src/commands/mod.rs`, `cli/src/commands/sessions.rs`, `cli/src/daemon.rs`, `api/MCP.md`, `api/CLI.md`, `api/TRACE.md`, `app`, `bindgen`, `core`, `rehearsal`
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
Markdown review itself (REVIEW). `prototype/` and `ui-prototype/` (never touched).

## Lane (written by the manager)
- **Goal / exit:** as above. Owner settled D104 and D9 on 2026-11-15, so both halves run.
- **Design notes:**
  - Batch 1 (spec mode, D104): drop `SetSpecMode` + `SetSpecModeRequest`; drop `Session.spec_mode` (13) and `Session.spec_files` (14) and `StartSessionRequest.spec_files` (10), since SM-2 (a session's spec files) is dropped too. Removed fields are `reserved` by number and name (TESTING §4.8). The MCP op `session_spec_mode`, the CLI `humata session spec`, its rpc constant and allowlist entry, and `spec_mode` in MCP `session_info` go. Docs: MCP.md, CLI.md, TRACE.md (SM-1/SM-2 row removed; SM-3..SM-5 → SM-3, SM-4).
  - Retired wording ("Always let" → "Always allow"; "Widen" → "Full screen"; the Approve button → "Allow", D112, D104 labels): proto comments in api/, user-facing daemon messages, TESTING.md button labels. Kept on purpose: the CLI verb `humata inbox approve`, the proto message `Approve` and `VERDICT_APPROVE` (wire names, and a PR verdict is "Approve" per SPEC), "widen" as a security verb ("an agent can't widen its own grant"), the internal action name `ToggleWiden`.
  - Batch 2 (D9): delete the four folders; root `Cargo.toml` loses the three members and the workspace deps only they used (uniffi, uniffi_bindgen, camino); main repins (`CARGO_BAZEL_REPIN=1 bazel mod tidy`) so Cargo.lock and cargo-bazel-lock.json drop them. LAYOUT.md line 5 updated. No BUILD or MODULE file outside the folders references them.
- **Claims:** see WORKLOG Claims row TIDY.
- **Tasks:**
  - [x] T1 proto: sessions.proto (rpc, message, fields reserved), capabilities/access/inbox/common.proto comments · manager · done
  - [x] T2 MCP: daemon/mcp/src/tools.rs op + definition, tests/tools.txt · manager · done
  - [x] T3 CLI: args.rs Spec, commands/{mod,sessions}.rs, daemon.rs rpc constant + list · manager · done
  - [x] T4 docs: api/{MCP,CLI,TRACE}.md, TESTING.md labels · manager · done
  - [x] T5 wording: daemon/access/src/{error,verdict}.rs, daemon/humatad/src/grant_target.rs · manager · done
  - [ ] T6 seams in ASK's files (main applies): session_service.rs `set_spec_mode`, mcp.rs `spec_mode` line, inbox.rs two strings · main
  - [x] T7 batch 2: delete app/, bindgen/, core/, rehearsal/; root Cargo.toml; LAYOUT.md · manager · done
- **T6 seams (ASK's files, main applies with batch 1):**
  1. `daemon/humatad/src/session_service.rs`: delete the whole `async fn set_spec_mode(...)` (returns `not_built("humata.v1.SessionService.SetSpecMode")`); required, the trait method is gone.
  2. `daemon/humatad/src/mcp.rs` `session_json`: delete the line `"spec_mode": session.spec_mode,`; required, the field is gone.
  3. `daemon/humatad/src/inbox.rs:1023`: `format!("Always let “{}” {target_label}", agent.title)` → `format!("Always allow “{}” to {target_label}", agent.title)`; line 391 `"approve it, approve it with Always let, or deny it"` → `"allow it, allow it with Always allow, or deny it"`; doc comments at lines 3 and 497 and `access_service.rs:1`: "Always let …" → "Always allow …".
- **Left for others:** `clients/src/model.rs:454` doc comment "Always let …" (LOOK1, not user-facing). `api/TRACE.md` has no rows for SM-17–SM-21 (pre-existing trace gap). `apps/humata/SPEC.md` (Humata 1) still names the deleted folders as history. `HUMATA2_BUNDLE_ID` in `apps/lifr/identity.bzl` is now unused; kept for the GPUI app's future bundle. OVERVIEW line 28 (D9 open item) and line 425 (D104 follow-up) close with these commits; D17's "waits on D9" note is now moot.
- **Waiting on:** main: T6 seams, repin, gate.
- **Build gates:** none yet
- **Ready to commit:** no (waiting on the gate)

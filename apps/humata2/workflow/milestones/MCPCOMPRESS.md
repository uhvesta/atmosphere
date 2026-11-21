# MCPCOMPRESS: Progressive disclosure for Humata 2 MCP (`humata_list`, `humata_describe`, `humata_call`)

- **Status:** queued — **0.10.2** (or **0.10.1** after `SETTINGSFIX` & `PROVIDERCLI`)
- **Outcome sentence:** Humata's MCP server exposes 3 progressive-disclosure meta-tools (`humata_list`, `humata_describe`, `humata_call`) instead of 68+ up-front schemas, cutting the initial `tools/list` footprint by ~98% (~21k prompt / ~52k wire tokens → ~473 prompt / ~1,023 wire tokens) while keeping 1-turn self-correcting validation and all existing allowlists/transcript scrubbers intact.

## Measured footprint (`tools/list`)
- **Before (`GrpcDaemon`, 68 tools):** 209,105 B wire (~52k–58k tokens) / 84,053 B without `outputSchema` (~21k–23k prompt tokens).
- **After (3 meta-tools: `humata_list`, `humata_describe`, `humata_call`):** 4,093 B wire (~1,023 tokens) / 1,891 B without `outputSchema` (~473 prompt tokens) — **-98.0% wire / -97.8% prompt**.

## Design
1. **`humata_list({ category?, query? })`:** compact 1-line `<name> [<category>,<effect>](<req*>; <opt>): <summary>` entries, dynamically filtered by `serves(op)` (including `browser_offered`, `delete_many_offered`, and fixing the 6 missing `GrpcDaemon::serves` arms + `InboxWait`). Audit underlying operation names for clean, logical `<area>_<verb>` consistency (owner, 2026-11-20).
2. **`humata_describe({ names: [..] })`:** batchable (`minItems: 1`, `maxItems: 20`) returning full `inputSchema`, annotations, and validated examples (`Schema::minimal_example()` + curated complex examples), plus structured `unavailable` entries for user-only/unoffered/unknown names.
3. **`humata_call({ name, arguments? })`:** dispatches to existing `Operation` + validator; fixes `Kind::OneOf` closest-shape scoring so discriminator-matched branches win ties; on `-32602` validation error returns the exact violation AND compact inline schema (`Schema::to_compact_focused`) in both `error.message` and `error.data.schema` for 1-turn self-correction; preserves `user_only` refusals, `_meta.progressToken`, `notifications/cancelled`, `inbox_wait`, and idempotency keys.
4. **Hooks & skills:** unwrap `humata_call` `tool_input.name` in `hooks.rs::only_files_an_inbox_request` (for `provider_import`), keep `NO_MOUNT` `"files_list"` token and D743 `INSTRUCTIONS` rules, and update `api/MCP.md`, `skills/shared/basics.md`, and standalone review skills.

## Disjoint worker plan
- **T0 (`daemon/mcp` core + tests):** `schema.rs`, `tools.rs`, `server.rs`, `lib.rs`, `daemon/mcp/tests/**`.
- **T1 (`svc_mcp` + `hooks`):** `daemon/svc_mcp/src/mcp.rs`, `daemon/humatad/src/session_service/hooks.rs`, `daemon/humatad/tests/hooks.rs`.
- **T2 (`humatad` MCP tests):** `daemon/humatad/tests/common/mcp_client.rs`, `daemon/humatad/tests/{mcp,browser_mcp,board_tools,comment_tools,history_tools,repo_tools,matrix,mcp_desk_folder}.rs`.
- **T3 (skills & docs):** `api/MCP.md`, `TESTING.md`, `skills/**`.

## Exit criteria
- `tools/list` returns exactly `["humata_call", "humata_describe", "humata_list"]` in `<= 5_000` bytes wire (`<= 2_500` bytes without `outputSchema`).
- Every example from `humata_describe` across `Operation::ALL` validates against `input_schema(op)`.
- `OneOf` closest-shape regression, 1-turn self-correction, `user_only` refusals, dynamic `browser_offered`/`delete_many_offered`, and `humata_call` hook unwrapping all pass fast behavioural tests.

# Brief agents-09: Moving a session
Output: `apps/humata2/flows/cases/agents/09-moving-a-session.md`

Scenario: the user (or an agent, if allowed) moves a session from this Mac to box-1, then box-1 to box-2. Includes the user doing it with ⌥⌘M / the palette and an agent asking via MCP `session_move` (check whether built).

Steps: plan; terminal read-only while moving; id, branch, work and conversation kept; resumes on target; the Mac's folder freed only after it runs on box-1; a moved shell tab comes back with its output; conversation mirrored once.

Branches: copy fails → stays on source, names the step; target offline or outside the channel → refused, nothing changes; same move asked again → same operation; cancel before agent stops → undone; after → refused; cuts at each step; a stopped session stays stopped; MCP SessionMove (likely not in the dispatch → not_built).

Pointers: daemon/humatad/tests/move_session.rs (18), features/palette/tests/act.rs (move), features/app/tests/move_from_palette.rs, desktop/tests/native/fork_move.rs, desktop/tests/live_flows/find.rs (a session moves from the palette; real daemon), daemon/humatad/src/mcp.rs dispatch, daemon/humatad/src/session_service/move_session.rs.

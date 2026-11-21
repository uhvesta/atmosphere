# INTENTSPEC — Implementation Intent Spec (`humata.intent/v1`)

Approved render: `/Users/avestabarzegar/Humata/atmosphere/desk/taste-review/renders/07-intent-spec/01-desk-intent-spec.png`

## Outcome
In any Channel Desk (or Desk) session, the owner can say *"hey I want to make an intent spec"* (or `/intent-spec`). The agent uses the packaged `humata-intent-spec` skill to inspect channel repos read-only, clarify key tradeoffs, and write a schema-validated `intent.yaml` (`humata.intent/v1`) in the session folder. Humata versions it via existing `DOCV` (`doc_history`), renders it in a **Structured | YAML | Diff** split pane beside the terminal with inline comments, and offers **`Approve & Start Milestone`** to pin the approved version, create/link a channel To-do milestone from `steps:`, and launch a normal implementation session pre-filled with the approved intent brief.

## Schema (`humata.intent/v1`)
- `schema: humata.intent/v1`
- `metadata`: `id`, `title`, `status` (`draft | review | approved | implementing | verified`), `channel`, `todo_key?`, `repos[]`, `approved_doc_version?`
- `outcome`: declarative "What & Why" (1–2 sentences)
- `non_goals[]`: explicit out-of-scope fences
- `invariants[]`: unbreakable system/data/API guarantees
- `constraints`: `forbidden_actions[]`, `approval_boundaries[]`
- `touch_points[]`: scoped repos/paths/contracts to read or modify
- `steps[]`: `{ id, title, outcome, depends_on[], crates[], verify }` (maps 1:1 to Humata To-do milestone steps)
- `acceptance[]`: `{ id, given, when, then }` (feeds TDD loop)
- `open_questions[]`: `{ id, question, default_choice?, blocking: bool, resolved: bool }` (must have 0 blocking unresolved before approval)

## Disjoint Implementation Lanes
1. **Lane A (`intent-core`):** `apps/humata2/intent_spec/**` (new wasm-safe crate: strict `serde_yaml` parser, semantic validator + cycle check, `IntentDiff`, `to_milestone_brief(version_id)`) + `apps/humata2/skills/humata-intent-spec/{SKILL.md,intent-spec.md,schema.yaml}`.
2. **Lane B (`intent-docv`):** `apps/humata2/features/session/src/{document.rs,history.rs}` — generalize `is_markdown` → `is_versioned_doc` (`*.md`, `*.mdx`, `intent.yaml`, `*.intent.yaml`) so existing `DOCV` versioning and inline comments work with zero new proto/storage tables.
3. **Lane C (`intent-ui`):** `apps/humata2/ui/changes/src/{intent_tab.rs,doc_history.rs,document_tab.rs}` + `features/app/src/new_session.rs` — **Structured | YAML | Diff** viewer, schema validity pill, version stepper `‹ vN of M ›`, inline comment cards, and **`Approve & Start Milestone`** CTA.

## TDD Exit Criteria (Full Lint, no `--fast`)
- `//apps/humata2/intent_spec:intent_spec_unit_test` (parse/reject, cycle detection, `IntentDiff`, `to_milestone_brief`)
- `//apps/humata2/daemon/machines:own_skills_test` (`humata-intent-spec` bundled & installed)
- `//apps/humata2/features/session:history_test` (`intent.yaml` save/view/compare/restore via `DOCV`)
- `//apps/humata2/ui/changes:changes_unit_test` (Structured/YAML/Diff rendering, validation errors, approval gate)
- `//apps/humata2/features/app:board_start_test` (`Approve & Start Milestone` pins version, creates/attaches milestone, pre-fills `first_message`)

## Next Phase — Board "Intent Mode" ([INTENTMODE](INTENTMODE.md))
- Extends `INTENTSPEC` from a single-session spec tab into **Intent Mode on the Board**: intents replace granular TODOs as living, `DOCV`-versioned packages (`DRAFT`, `TODO`, `IN PROGRESS`, `IN REVIEW`, `BLOCKED`, `DONE` + due dates) rendered with `intent_tab.rs`, spawning multiple tagged channel sessions that continuously write back what is done and what remains, and notifying the owner on `IN REVIEW`, design questions, edge cases, or unexpected blockers. See [INTENTMODE.md](INTENTMODE.md).

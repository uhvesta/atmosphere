# Authoring an Implementation Intent Spec (`humata.intent/v1`)

An Implementation Intent Spec (`intent.yaml` or `<name>.intent.yaml`) captures
the declarative **What & Why**, hard constraints, verification plan, and
acceptance criteria in the session folder before implementation begins.

Humata versions `intent.yaml` automatically through document history (`DOCV`)
and renders it in a **Structured | YAML | Diff** pane beside the terminal,
where the owner can leave inline comments and click **Approve & Start Milestone**.

## Workflow

1. **Inspect read-only first.**
   - Run `humata whoami` to learn your session, channel, and repos.
   - Read relevant files, tests, and build targets in the channel's repos
     without editing code or running destructive commands.
2. **Clarify tradeoffs.**
   - If scope, invariants, or breaking changes are ambiguous, ask focused
     questions or record them under `open_questions:` with `blocking: true`
     and a `default_choice:`.
   - Approval is blocked until every `blocking: true` question has
     `resolved: true`.
3. **Write `intent.yaml` in the session folder.**
   - Follow the exact `humata.intent/v1` schema in `schema.yaml`. Unknown fields
     and duplicate keys are rejected at parse time.
   - Keep `steps:` acyclic (`depends_on` must reference existing step IDs with
     no cycles) and give every step a concrete `verify:` command.
   - Write concrete `Given / When / Then` entries in `acceptance:` to feed the
     TDD loop.
4. **Iterate on owner feedback.**
   - Address inline comments left on `intent.yaml`, resolve blocking
     `open_questions`, and update `intent.yaml` cleanly so `DOCV` captures each
     revision for side-by-side semantic diffing.

## Schema Summary (`humata.intent/v1`)

- `schema`: must be `humata.intent/v1`.
- `metadata`:
  - `id`: short stable identifier (e.g. `PAY-14`, `intent-001`).
  - `title`: concise title for the milestone and implementation session.
  - `status`: `draft | review | approved | implementing | verified`.
  - `channel`: target channel name or ID.
  - `todo_key` (optional): existing board/To-do item key to link.
  - `repos` (optional list): repositories in scope.
  - `approved_doc_version` (optional): pinned version ID set on approval.
- `outcome`: 1–2 sentences stating what changes and why.
- `non_goals`: explicit out-of-scope boundaries.
- `invariants`: unbreakable system, data, or API guarantees.
- `constraints`:
  - `forbidden_actions`: actions the implementing agent must never take.
  - `approval_boundaries`: actions that require owner Inbox approval.
- `touch_points`: `{ repo, path, mode: read | modify, note? }`.
- `steps`: `{ id, title, outcome, depends_on[], crates[], verify }`.
- `acceptance`: `{ id, given, when, then }`.
- `open_questions`: `{ id, question, default_choice?, blocking, resolved }`.

## What needs the owner

- **Approving and launching the milestone** is owner-only: the owner reviews the
  spec in Humata's Intent Spec pane and clicks **Approve & Start Milestone**
  once schema validation passes and all blocking open questions are resolved.
- Never start implementing code edits during the intent-spec phase unless the
  owner explicitly asks you to.

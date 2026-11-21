# INTENTMODE: Board "Intent Mode" — High-Level Living Intent Packages Replace To-do

- **Status:** planning (owner + agent co-design, 2026-11-21)
- **Depends on:** [INTENTSPEC](INTENTSPEC.md) (`Structured | YAML | Diff` viewer in `ui/changes/src/intent_tab.rs`, `DOCV` versioning & inline comments); `BOARDSIMPLE` / `svc_board` (due dates, multi-session attachment, comments, audit history); `NOTIFYAUDIT` (macOS notifications, Deck toasts, Dock badge); `REVIEW_UI_AND_WEB` (`Reviews` inbox handoff).
- **Mockups:** [mockups/intentmode/FRAMES.md](mockups/intentmode/FRAMES.md) (`im-1-board-open.png` .. `im-6-settings-toggle.png`)
- **Unblocks:** retiring heavy `TODO2` micro-task management wherever Intent Mode is enabled.

## Locked-In Owner Decisions (2026-11-21)

1. **Global App Toggle that Replaces To-do:**
   - **Intent mode** is a global switch in `Settings › General`. When enabled, **Intents replace To-dos everywhere** across the app (sidebar `Intents` entry, Channel Desk pane, session panel, Web UI, and agent CLI/MCP surface) — you never juggle both TODOs and Intents at the same time. Switching modes is non-destructive.
2. **Author With or Without an LLM:**
   - Users can create and edit any Intent **directly on the board without an LLM** (`+ New intent` → lightweight direct form for Title, Channel, Due date, Outcome, optional Non-goals / Guardrails / Deliverables, or raw `YAML`), *or* co-author / refine it conversationally with an LLM (`✦ Draft with AI` or in Channel Desk).
3. **High-Level Upfront — Zero Premature Implementation Micromanagement:**
   - Before coding agents start cooking (`DRAFT` / `TODO`), an Intent captures **Outcome, Due date, Non-goals, and Hard-Fence Guardrails** — *not* speculative file lists, crate breakdowns, or micromanaged implementation checklists. `steps` starts empty unless a coarse human/rollout milestone is already known.
3. **Deliverable-Only Steps Populated Live as Agents Cook:**
   - Once sessions start cooking (`IN PROGRESS`), tagged agents add and update `steps` representing **real, human-trackable outcomes only**:
     - **`pr`**: a real PR or code deliverable (e.g. `acme/api#489 — Opaque cursor codec & handler`)
     - **`human`**: a manual human action (e.g. `Enable users.keyset_v2 feature flag in LaunchDarkly`, `Rotate secret in 1Password`)
     - **`deploy`**: a deployment, migration, or rollout gate (e.g. `Backfill 3.1M staging rows`, `Prod rollout & drop legacy endpoint`)
   - **Explicitly forbidden filler:** steps must **never** be process chores like *"review PR"*, *"write unit tests"*, *"fix lint"*, or *"iterate until CI is green"*. Skill instructions and schema validation rules reject/discourage filler steps.
4. **First-Class Due Dates (`metadata.due: YYYY-MM-DD`):**
   - Every intent supports a firm `due` date (`Day` / `parse_day`: `"tomorrow"`, `"fri"`, `"next week"`, `"nov 28"`, `"2026-11-30"`), stored in both the board store and `intent.yaml`, rendered on Board rows and in `intent_tab.rs`'s `meta_strip`, and used by authoring LLMs to right-size scope (`non_goals`). Approved due dates are owner-locked (agents flag risk rather than silently pushing dates).

## Outcome

With **Intent Mode** turned on globally, the owner co-authors lightweight, high-level **Intent Packages** with an LLM (Outcome + Due date + Guardrails), tracks their state (`DRAFT`, `TODO`, `IN PROGRESS`, `IN REVIEW`, `BLOCKED`, `DONE`) on one calm board, and spawns multiple channel sessions against an intent. Every spawned session tags the intent (`HUMATA_INTENT_KEY`), continuously updates **what has been done**, **what it believes is left**, and **real deliverable steps** (`PR` / `Human` / `Deploy`) into `DOCV`-versioned `intent.yaml` rendered by `Structured | YAML | Diff`, transitions `IN PROGRESS` ↔ `IN REVIEW` / `BLOCKED`, and notifies the owner whenever an intent needs human action (PR review, manual `[HUMAN]` step ready, design question, edge case, or deadline risk) — while keeping comments and run/audit logs.

## Schema Evolution (`humata.intent/v2`)

- **Set upfront & owner-locked after approval (changes require owner sign-off):**
  - `metadata`: `{ id, title, state, channel, due?: YYYY-MM-DD, approved_doc_version? }`
  - `outcome`: declarative What & Why (high-level, implementation-agnostic)
  - `non_goals[]`: explicit scope fences (informed by `due`)
  - `invariants[]` & `constraints` (`forbidden_actions[]`, `approval_boundaries[]` — rendered as `HARD FENCE`)
  - `acceptance[]`: optional high-level `Given / When / Then`
- **Living sections continuously updated by tagged sessions via atomic CLI/MCP ops (`DOCV` snapshots `v2..vN`):**
  - `progress`: `{ done: String/Vec, left: String/Vec }` — plain-language summary of completed work and current remaining outlook
  - `steps[]` (*optional at `DRAFT`/`TODO`; populated while cooking*):
    - `{ id, kind: pr | human | deploy, title, status: todo | in_progress | in_review | blocked | done, ref?: String, session?: String }`
  - `open_questions[]`: `{ id, question, recommended?, asked_by_session?, blocking: bool, resolved: bool }`
  - `state`: `draft | todo | in_progress | in_review | blocked | done`

## Core Workflows & Simplifications

1. **Starting Dedicated Sessions OR Tagging Already-Spawned Sessions:**
   - **Start a new session (`Start session`):** Opens `NewSession` (`SessionOptions.item = Some(intent_key)`) pre-filled with the intent's channel, host/agent, and brief; the new session is dedicated to that intent from birth (`HUMATA_INTENT_KEY=<key>`) and transitions `TODO → IN PROGRESS`. Repeat `Start session` anytime to run parallel sessions on the same intent.
   - **Tag an already-spawned session (`Tag session…` / `humata intent tag <key>`):** Attach any running session from the UI picker (on the intent or session header) or let a running agent call `humata intent tag <key>` via CLI/MCP, which links `session_id ↔ intent_key` in `svc_board` and returns the intent's outcome, guardrails, and current `Done / Left` state.
   - Bidirectional link: `intent.sessions[]` + `HUMATA_INTENT_KEY=<key>` + session header chip + `Runs N · $X.XX` cost/audit log.
2. **Concurrent Atomic Write-Back → `DOCV` History:**
   - Sessions call structured MCP/CLI tools (`humata intent progress`, `humata intent step`, `humata intent ask`, `humata intent review`) so N parallel sessions never clobber raw YAML, while `humatad` emits each canonical `intent.yaml` revision into `DOCV` (`‹ vN of M ›`) for semantic `Diff` inspection.
3. **High-Signal Notifications (`NOTIFYAUDIT` + `Reviews`):**
   - Notify owner only on: (a) `IN PROGRESS` → `IN REVIEW` (linked to `Reviews`), (b) a `[HUMAN]` step becoming actionable, (c) a blocking design question / edge case / unexpected blocker (`BLOCKED`), or (d) an intent due within 24h that is still `TODO` or `BLOCKED`.
4. **What Intent Mode Eliminates (vs. `TODO2`):**
   - Cuts `Effort × Impact × Need` scoring, `1.1×` blocker inheritance, `HeldBackBy` popovers, `SplitTodo` / `FoldTodoIntoStep` / `From`–`Became` lineage, `FuzzyDue`, and `WithContext → Defined → Done` checkpoints.
   - Keeps inline & item comments, `DOCV` revision history + semantic diffs, due dates, tagged session logs, and `Reviews` handoff.

## Exit Criterion

1. Flipping global **Intent mode** ON in `Settings › General` switches sidebar/Desk/session panels from `To-do` to `Intents`.
2. Owner drafts a high-level Intent Package (`steps: []`, `due` set) with an LLM, approves it to `TODO`, and spawns two parallel channel sessions tagged with the intent.
3. Sessions update `progress.{done, left}` and add deliverable steps (`pr`, `human`, `deploy`) concurrently via CLI/MCP without conflict, producing inspectable `DOCV` versions in `Structured | YAML | Diff`.
4. Raising a blocking edge-case question or reaching an actionable `[HUMAN]` step / `IN REVIEW` transition notifies the owner and updates the board group; requesting changes in review returns the intent to `IN PROGRESS`.
5. Focused unit, `humatad2_test_away_board`, and headless GPUI tests pass under `tools/bq/bq` without `--fast`.

## Read These (Pointers, Nothing Else)

- Milestones & Mockups: [INTENTSPEC.md](INTENTSPEC.md), [mockups/intentmode/FRAMES.md](mockups/intentmode/FRAMES.md), [BOARDSIMPLE.md](BOARDSIMPLE.md), [NOTIFYAUDIT.md](NOTIFYAUDIT.md), [REVIEW_UI_AND_WEB.md](REVIEW_UI_AND_WEB.md)
- Code:
  - `apps/humata2/intent_spec/` (`IntentSpec`, `IntentDiff`, `validate`, `brief`)
  - `apps/humata2/ui/changes/src/intent_tab.rs` (`Structured | YAML | Diff` viewer, `DOCV` stepper, inline comments)
  - `apps/humata2/skills/humata-intent-spec/`
  - `apps/humata2/api/proto/humata/v1/boards.proto` & `apps/humata2/daemon/svc_board/`
  - `apps/humata2/ui/workspace/src/todo.rs`, `apps/humata2/ui/settings/src/settings.rs`

## Lane (written by the manager)

- Design notes:
- Claims:
- Tasks:
- Waiting on: owner review of `mockups/intentmode/im-1..4.png`
- Build gates:
- Ready to commit: no

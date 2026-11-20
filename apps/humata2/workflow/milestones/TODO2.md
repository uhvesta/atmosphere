# TODO2: Smarter To-do ranking, fast triage, structured agent collaboration, and two-way PR/session sync

- **Status:** planning (scheduled before `FUSEFS`)
- **Depends on:** `BOARDSIMPLE` (single ranked To-do list, `features/todo`, `ui/workspace/src/todo.rs`, `Needs attention`); `INBOXHOOKS` (separate Agent inbox vs. quiet To-do inbox); `REVIEWUI` / `STACKVIEW` / `DIFFVIEW` (inline diff & markdown comments, PR reviews, PR stacks); `AGENTS2` (structured agent sessions and turn-end signals).
- **Unblocks:** `FUSEFS` (scheduled immediately after `TODO2` so To-do/session/PR lifecycle semantics are settled before filesystem cutover).

## Outcome

Humata 2's ranked To-do list stays automatically in sync with attached sessions, PRs, and PR stacks (defaulting to quiet 1-click prompts, with opt-in Auto), ranks items using Need, Impact, fuzzy vs. firm due dates, and transitive blocker inheritance, lets the user triage and bulk-act at keyboard speed, and supports rich structured agent questions, action/review requests, run/cost ledgers, and checkpoint handoffs.

## Exit criterion

1. **Two-way lifecycle sync & PR watchers:** linking a PR or PR stack to a To-do puts it in quiet *Watching PR* without holding an agent turn; CI failure, review comments, merge conflicts, or an upstream stack merge wakes the session or surfaces a 1-click prompt in `Needs attention`; PR merge prompts (or auto-closes when opted in) the To-do and offers clean-session archival with Undo; archiving the last session on an `In progress` item prompts to close or return it to `Open`.
2. **Diff & Markdown integration:** inline Diff and Markdown review comments offer **Add to steps** and **Spin off To-do** (`From: <key>` + `file:line` anchor), and the drawer / close requests display a compact close-readiness summary.
3. **Ranking, structure & fast triage:** transitive `1.1×` blocker priority inheritance with write-time cycle rejection, *Held back by* rank popover explanations, Need vs. Impact with need-scaled aging, fuzzy vs. firm due dates, temporary sort-by switcher, `Won't do` / `Became` states with *Blocker gone* recovery, splitting/folding with bidirectional `From`/`Became` links, free-text external block reasons, drop/restore checklist steps with Markdown context, append-only comments, word-level history diffs, Undo toast with conditional *Start next*, drawer `↑`/`↓` triage + list shortcuts, bulk selection with partial-failure retention, clickable tag & removable filter chips, and search including `Hidden until` items.
4. **Structured agent collaboration:** free-text & single/multi-choice Questions, Action requests (`Done`/`Can't be done`), multi-round Review requests (`Approved`/`Changes asked`), stuck-agent recovery card (`Retry`/`Take back`), explicit ownership + *Take back* confirmation, folded `Runs N · $X.XX` ledger, and `With context → Defined → Done` checkpoint handoffs.
5. **Automated tests:** leaf-crate unit tests in `daemon/svc_board` (or `daemon/todo`) and `features/todo`, integration tests in `daemon/humatad` + `desktop/tests/live_flows/todo_asks.rs`, and native/design UI frames in `desktop/tests/native/` + `desktop/tests/design/todo*.rs` all pass under `tools/bq/bq`.

## Read these (pointers, nothing else)

- Milestones: `apps/humata2/workflow/milestones/BOARDSIMPLE.md`, `INBOXHOOKS.md`, `STACKVIEW.md`, `FUSEFS.md`
- Decisions: `D830`–`D874` (`BOARDSIMPLE` / `TODOLIVE`), `D845`–`D847` (`INBOXHOOKS`), `D881`–`D886` (`FUSEFS`)
- Flows: `apps/humata2/flows/cases/board/README.md` and `01`–`14`
- Code:
  - `apps/humata2/api/proto/humata/v1/board.proto` (To-do RPCs & models)
  - `apps/humata2/daemon/svc_board/` & `apps/humata2/daemon/humatad/src/board_service.rs`
  - `apps/humata2/features/todo/`
  - `apps/humata2/ui/workspace/src/todo.rs` & `apps/humata2/ui/changes/src/panel_changes/`
  - `apps/humata2/desktop/tests/live_flows/todo_asks.rs`, `desktop/tests/native/todo_asks.rs`, `desktop/tests/design/todo.rs`, `desktop/tests/design/todo_more.rs`

## Phased sub-tasks (deliverable in clean slices)

### Slice 1 — Ranking, Priority Intelligence & Item Structure
- [x] **T1 — Priority scoring & Need/Impact model:** separate **Need** (*Idea / Nice to have / Expected / Critical*) from **Impact**, add need-scaled aging, loose/fuzzy due dates (*In a few days*, *Next week*, *Next month* — never red/overdue) alongside exact deadlines, transitive `1.1×` blocker priority inheritance with write-time cycle detection, and *Held back by* explanation payload.
- [ ] **T2 — Lifecycle states, splitting, lineage & rich steps:** add terminal states `Won't do` and `Became`, *Blocker gone* recovery cards in `Needs attention`, split-into-tasks (keep age vs. fresh) and fold-into-step with bidirectional `From`/`Became` links, free-text external block reasons, per-step optional Markdown context + reorder + soft Drop/Restore, append-only item comments, and word-level diffs in item History.
  - Status 2026-11-20: built: Won't do / Became, Blocker gone cards, split and fold with From/Became links, external block reasons, step Drop/Restore/Move, comments and word-diff history (`daemon/svc_board/src/todo.rs`, `todo_fields.rs`, `history.rs`, `features/todo/src/lib.rs`, `ui/workspace/src/todo.rs`); missing: a UI control to edit a step's Markdown context (`TodoAction::SetStepMarkdown` exists but the drawer only displays it).

### Slice 2 — Fast Triage & List UX
- [x] **T3 — Keyboard triage, sort switcher & chips:** temporary sort-by switcher (*Priority* default, *Due soonest*, *Most needed*, *Smallest effort*, *Largest impact*), drawer triage stepping (`↑`/`↓`) + list shortcuts (`↑`/`↓`, `Enter`, `s`, `x`, `/`, `N`), Undo toast with conditional `Start next: <top item>` when nothing else is `In progress`, clickable tag chips + removable active-filter chips bar, and search including `Hidden until` items.
- [x] **T4 — Bulk selection & partial-failure handling:** Shift-click / `Shift+↑`/`↓` multi-select for `Done`, `Won't do`, `Start`, and `Move channel`, reporting per-item refusals and keeping only refused items selected; *Held back by* rank popover + subtle unset-field highlights in the drawer.

### Slice 3 — Structured Agent Collaboration
- [ ] **T5 — Structured asks, reviews & ownership:** free-text and single/multi-choice Questions (optional step anchor, short text + expandable details, editable while open), Action requests (`Done`/`Can't be done`), multi-round Review requests (`Approved`/`Changes asked`) distinct from `Ready to close`, stuck-agent recovery card (`Retry`/`Take back`), and explicit agent ownership with *Take back* confirmation.
  - Status 2026-11-20: built: Question/Action/Review asks and multi-choice (`daemon/svc_board/src/asks.rs`, `api/proto/humata/v1/inbox.proto`, `ui/workspace/src/todo.rs`), ownership and Take back (`daemon/svc_board/src/ownership.rs`), stuck-agent card with Retry/Take back (`ui/workspace/src/todo.rs`, `features/todo/src/lib.rs`); missing: nothing in the daemon ever sets `stuck_agent`, so the recovery card never appears for a real stuck agent.
- [ ] **T6 — Runs & Cost ledger + checkpoint handoffs:** folded `Runs N · $X.XX` drawer section (tool, model, outcome, links, cost) and `With context → Defined → Done` stable-state handoffs with `Changes asked…` bounce-back.
  - Status 2026-11-20: built: `Runs N · $X.XX` ledger section, `todo_runs` store table and `ListTodoRuns` (`daemon/store/src/todo_runs.rs`, `daemon/svc_board/src/todo.rs`), checkpoint With context/Defined/Done in `ui/workspace/src/todo.rs`; missing: nothing records a run from a real session (`record_todo_run` is only called from tests), so the ledger stays empty.

### Slice 4 — Two-Way Lifecycle Sync, PR Watchers & Review Integration
- [ ] **T7 — PR ↔ To-do ↔ Session two-way sync & live signals:** PR/stack merge → close prompt (or opt-in Auto); PR closed without merge → `Won't do` vs. keep open prompt; To-do close → clean-session archive offer (or opt-in Auto with Undo) with explicit reasons when kept open; last session archived → prompt to close or return `In progress` item to `Open`; quiet live session signals on `In progress` rows + soft *Idle Nd — resume or pause?* nudge; Settings / channel config toggles (Ask default, opt-in Auto).
  - Status 2026-11-20: built: Settings `TodoAutomationSettings` (Ask/Auto) in `api/proto/humata/v1/settings.proto`, prompt kinds for PR merged/closed, session idle and last session archived in `daemon/svc_board/src/pr_watch.rs`, Idle Nd nudge and prompt resolution in `ui/workspace/src/todo.rs`; missing: the settings are never read (`todo_automation: None` in `daemon/humatad/src/settings_service.rs`), `evaluate_session_idle` and `evaluate_last_session_archived` are only called from tests, no auto-close or archive-sessions-on-close offer, no live PR/session event feed.
- [ ] **T8 — Event-driven PR/CI watchers & Diff/Markdown comment hooks:** quiet *Watching PR* state that wakes sessions (or surfaces 1-click *Send comments to session* in `Needs attention`) on CI failure, new review comments, merge conflicts, or upstream stack merge rebase; **Add to steps** and **Spin off To-do** (`From: <key>` + `file:line`) from inline Diff and Markdown review comments; compact close-readiness line in drawer and close requests (`Steps X/Y · Z unresolved comments · CI … · N commits`).
  - Status 2026-11-20: built: Add to steps and Spin off To-do from review comments (`ui/changes/src/comment_cards.rs`, `features/todo/src/lib.rs` SpinOffFromComment), close-readiness line (`daemon/svc_board/src/pr_watch.rs`, `ui/workspace/src/todo.rs`), prompt kinds for CI failure, comments, conflicts and stack rebase (`pr_watch.rs`); missing: no watcher polls or subscribes to PR/CI state (prompts only evaluate when a client writes a watched PR), no wake of the session, no Send comments to session action wired in the To-do UI.

## Out of scope

- Multi-tenant teams, separate heaps, or admin-configured weighted custom parameters.
- Standalone disk-cleanup sweeps (`MASSDELETE` owns bulk disk/session deletion).
- Anything owned by `FUSEFS` (virtual filesystem / cross-machine mount).

## Lane (written by the manager)

- Design notes:
- Claims:
- Tasks:
- Waiting on:
- Build gates:
- Ready to commit: no

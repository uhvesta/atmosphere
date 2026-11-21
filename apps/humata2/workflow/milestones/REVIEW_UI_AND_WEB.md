# REVIEW_UI_AND_WEB — Unified Desktop & Web Reviews Inbox + Stack Layer Navigation

Covers: `LOCALREVIEW` (`ATM-73`, `ATM-74`), `PRREVIEW` (`ATM-58`, `ATM-59`, `ATM-60`, `ATM-61`), `STACKIMPORT` UI (`ATM-36`), `PRFEEDBACK` UI (`ATM-38`), and `WEBREVIEWS` (`web_todo` minimal Chrome `/reviews` + stack layer navigation).

## Authoritative Decisions & Renders
- Locked-in rules: `/Users/avestabarzegar/Humata/atmosphere/desk/taste-review/DECISIONS.md` (§1 Reviews, §6 Minimal Chrome / Web UI)
- Approved Desktop renders:
  - `renders/01-review/01-reviews-inbox.png`
  - `renders/01-review/02-stack-review-revised.png`
  - `renders/01-review/03-github-feedback-revised.png`
- Approved Web renders:
  - `renders/06-web-minimal/01-minimal-web-ui.png`
  - `renders/06-web-minimal/02-web-reviews-inbox.png`
  - `renders/06-web-minimal/03-web-layer-navigation.png`

## Desktop Requirements (ONE Unified Implementation for Local & Remote)
1. **Dedicated `Reviews` sidebar entry** right under `Agent inbox` (keeps `Agent inbox` 100% pure for blocked agents; drops duplicate `Pinned` PR slot).
   - Groups: **Local · ready to push** and **On GitHub · feedback & review requests** (incl. manually watched PR URLs + `+ Watch PR URL…`).
   - Stripped row noise: no duplicate author names, no `"no To-do, not your work"`, no `"1 layer"` on single PRs, concise chips (`Fixes pushed`), no competing solid-blue `Review` button per row.
2. **Unified Full-Screen Review (`ui/changes/src/panel_stack.rs`, `panel_changes/`):**
   - Replace `New agent tab ⌄` with single **`Agent · N ⌘J`** toggle in title bar (`+` inside dock adds agents under flat cap `enabled kinds + 2`, max 2 per kind).
   - Collapse `Expand` + `Unfold` + `✦ Drafted by agent` into single **`Details ⤢`** button (Markdown + Mermaid).
   - Unified / Split (`⌥⌘S`) + GitHub-style in-place context expand (`20 lines` / `Expand all`) + `Open file ↗` read-only window.
   - Separate **`Layer looks good`** (local progress; preserves `Looked at` with `Rebased · diff unchanged` after unchanged restack) from **`Push N PRs…`**, which opens a light anchored confirmation popover listing exact PRs, unreviewed-layer warnings, and **Draft PRs / Ready for review** toggle (default: **Draft PRs**). Retires standalone `ATM-55` Submit dialog.
   - Returning GitHub feedback: `Checked Nm ago · Refresh` in left rail (not title bar), inline drafts queue (`Edit · Discard`) and submit via single footer CTA (`Post N replies to GitHub`).
3. **Stack Import Dialog (`ATM-36`, `features/pr_review/src/dialog.rs`, `ui/overlays/src/overlay_pr_review.rs`):**
   - Display detected layers bottom-to-top; offer `Import whole stack` and `Review single PR`.
4. **On-Demand Pull PR Feedback (`ATM-38`):**
   - Inline reviewer badge, status, PR link affordance, selectable feedback + `Send to agent`.

## Minimal Web UI (`apps/humata2/web_todo/{server,web,wire}`)
1. Single 46px top bar (`Humata` · `To-do` · `Reviews` · filter pills · `⌘K Search` / `+ Watch PR URL…`), zero `• Live` badge or heavy sidebar.
2. Full `/reviews` inbox alongside `/todo`: Local stacks, GitHub feedback, teammates' review requests, and manually watched PRs (`+ Watch PR URL…`).
3. `/reviews/<id>?layer=N` with left-rail layer list (`Rebased · diff unchanged`), top layer breadcrumb chain (`1 → 2 → 3`), `‹ Prev layer / Next layer ›` (`⌥⌘↑ / ⌥⌘↓`), and `Open in Desktop (with Agent) ↗` handoff (read-only for Inbox approvals).

## Planned Disjoint Claims (Ready to Launch as Wave 2 UI Lane)
- `apps/humata2/web_todo/**`
- `apps/humata2/ui/changes/**`
- `apps/humata2/features/pr_review/**`
- `apps/humata2/features/review/**`
- `apps/humata2/ui/overlays/src/overlay_pr_review.rs`
- Shared seams (applied by coordinator): `ui/workspace/src/sidebar.rs` (`Reviews` entry), `features/app/src/lib.rs`, `clients/src/model.rs`.

## Exit Criteria
- All focused unit tests, headless GPUI interaction tests, and `web_todo` tests pass without `--fast` (full lint).
- Re-recorded UI goldens judged against `renders/01-review/` and `renders/06-web-minimal/` by a `gemini-4-neon-high` judge subagent.

# Milestone: REVIEW-DATA (`STACKIMPORT` ATM-23/37, `PRFEEDBACK` ATM-22/27, `LOCALREVIEW` backend ATM-56/57/71/72/75/76)

## Scope & Locked-In Decisions
Implements the backend data model, stack import, restack `Looked at` preservation, GitHub PR feedback/polling, and local review requested + deterministic PR→To-do auto-attach rules per `DECISIONS.md` (§1 Reviews, §3 Auto-created PR To-dos) and the board spec.

---

## Implemented Components

### 1. Stack Import (`ATM-23`, `ATM-37`) — `apps/humata2/daemon/svc_review`
- **`src/pull_requests/resolve.rs`**:
  - Keeps existing `PullRequests::resolve` (`pb::PullRequestInfo`) unchanged.
  - Adds `StackSource` (`NativeGhStack`, `BaseChain`, `Single`), `ResolvedStack`, `parse_gh_stack_numbers`, `detect_base_chain`, `resolve_stack_from_candidates`, and `resolve_stack(target)`.
  - Detects native `gh stack` metadata and base-chained PR stacks bottom-to-top from any PR URL/number in the chain, with cycle protection and single-PR fallback.
- **`src/pull_requests/start.rs`**:
  - Removes `StartReviewError::RepoTwice` so connected multi-PR stacks in one repository can start in a single review session.
  - Adds `StartReviewError::DuplicatePr`, `collect_review_prs`, and `dedupe_wanted` so each connected repository is cloned/registered **once**.
- **`src/pull_requests/heads.rs` & `src/pull_requests/commits.rs`**:
  - Adds `pr_ref_name`, `StackLayerRange`, `fetch_stack_layer_refs_blocking`, and `stack_layer_ranges_blocking` to fetch `refs/humata/pr-<n>` per layer and build ordered bottom-to-top `(base_sha, head_sha, ref_name)` ranges in a single clone.
  - Updates `commits.rs` (`head_of_in_stack`) to resolve per-layer commit ranges via `refs/humata/pr-<n>` (falling back to `origin/<base_branch>` and `HEAD`).

### 2. Restack Behavior (Owner Locked-In) — `apps/humata2/daemon/review_diff`
- **`src/stack.rs`**:
  - Adds `REBASED_DIFF_UNCHANGED_NOTE` (`"Rebased · diff unchanged"`), `RestackStatus` (`Unchanged`, `RebasedDiffUnchanged`, `DiffChanged`, `Missing`), `LayerRestack`, and `CheckedStack::restack_status_blocking` comparing layer diffs via `git diff --no-color --no-ext-diff <base>..<head>` piped through `git patch-id --stable`.
- **`src/looks.rs` & `src/lib.rs`**:
  - Adds `LayerLook` (`rebased_diff_unchanged: bool`), `reconcile_looks_after_restack`, and `Checkout::reconcile_looks_blocking`: keeps `Looked at` intact and quietly marks `"Rebased · diff unchanged"` when a layer's diff is unchanged after restack; invalidates looks when the diff changed. Preserves 100% backward-compatible `[path, fingerprint]` JSON format.
- **`tests/restack_looks.rs`**:
  - Real-git behavioural tests covering 2-layer rebase with unchanged diffs, modified lower layer + unchanged upper layer, unchanged stack, and missing layer.

### 3. GitHub Feedback & Polling (`ATM-22`, `ATM-27`, `ATM-75`, `ATM-76` + Manual Watch) — `apps/humata2/daemon/svc_review` & `apps/humata2/daemon/svc_board`
- **`svc_review/src/pull_requests/feedback.rs`**:
  - Single GraphQL call per PR (`PR_FEEDBACK_QUERY`) requesting `reviewThreads` + top-level `reviews` + PR state/url/headRefOid.
  - Filters `<!-- stack-history -->` comments completely.
  - Collapses bot comments into one quiet summary (`CollapsedBots`, e.g., `"2 bot comments collapsed"`).
  - Deduplicates by `(pr_key, thread_id)`, preserving reviewer, status, file path, line, body, and link metadata.
- **`svc_review/src/pull_requests/inbox_poll.rs`**:
  - `ReviewInboxPoller` / `InboxPollState` with 15-minute default cadence (`DEFAULT_POLL_INTERVAL`), `poll_on_focus`, and `poll_manual` (bypassing staleness).
  - Conditional/ETag + seen-watermark dedup; retires merged/closed PRs from polling.
  - Direct review requests only by default (`opted_in_teams` empty by default).
  - **Owner addition:** `watch_pr_url` / `unwatch_pr` / `watched_prs` parses any remote GitHub PR URL (`https://github.com/owner/repo/pull/N` or `owner/repo#N`) and polls it on the same cycle.
- **`svc_board/src/pr_reviews_gh.rs` & `svc_board/src/pr_reviews.rs`**:
  - Extends `review-requested:@me` GraphQL query with `viewer { login }` and `reviewRequests` (`User` vs `Team`) so direct review requests are included by default and team-wide blasts are ignored unless opted in.
  - Adds durable `ManualWatchedPr` storage (`watch_remote_pr_url`, `unwatch_remote_pr`, `manual_watched_prs`) without auto-creating To-dos.

### 4. Local Review Requested & Deterministic PR→To-do Auto-Attach (`ATM-56`, `ATM-71`, `ATM-72`) — `apps/humata2/daemon/svc_board`
- **`src/todo_fields.rs`**:
  - Adds additive `#[serde(default)]` `local_review: Option<LocalReviewRow>` (`LocalReviewState`: `Draft`, `LocalReviewRequested`, `ChangesRequested`, `Submitted`, `Approved`, `Done`, plus `layers`, `summary_what/why/details`, `owner_approved_push`, `fixes_pushed_since_review`, `attachment_reason`, `auto_attached`).
- **`src/local_review.rs`**:
  - State transitions enforcing `Draft → LocalReviewRequested ⇄ ChangesRequested → Submitted → Approved → Done`.
  - `request_local_review` never blocks in `Agent inbox`; returns a buttonless review-inbox notice (`"PAY-3 · 3 layers ready for your review"`).
  - `mark_local_review_submitted` refuses with `LocalReviewError::PushNotApproved` unless owner `Looks good` (`approve_local_review_for_push`) has been recorded.
  - `attach_or_create_todo_for_local_review` attaches to an existing To-do **only on unique deterministic signals** (linked session or branch ancestry `head_branch == incoming.base_branch`); otherwise always creates a new To-do with a declarative outcome title (never fuzzy-matches silently).
  - `move_local_review_to_own_todo` provides the single `Move to its own To-do` escape action with `from_keys` lineage.

---

## Additive Proto Seams (For Coordinator)

### `apps/humata2/api/proto/humata/v1/review.proto`
1. **Stack resolution (`ATM-23`):**
   ```protobuf
   enum StackSource {
     STACK_SOURCE_UNSPECIFIED = 0;
     STACK_SOURCE_NATIVE_GH_STACK = 1;
     STACK_SOURCE_BASE_CHAIN = 2;
     STACK_SOURCE_SINGLE = 3;
   }
   // Additive fields on PullRequestInfo (next free: 12):
   repeated PullRequestInfo stack_layers = 12;
   StackSource stack_source = 13;
   ```
2. **Restack unchanged drift (`DECISIONS.md` §1):**
   ```protobuf
   // Additive enum value on ReviewLayerResolved.Drift (next free: 5):
   DRIFT_REBASED_UNCHANGED = 5;  // commits moved by rebase, layer diff unchanged; keep Looked at
   ```
3. **PR feedback & manual watched PR URLs (`ATM-22`, `ATM-27`, `ATM-75`, `ATM-76`):**
   ```protobuf
   message WatchPullRequestRequest { string url = 1; }
   message UnwatchPullRequestRequest { PullRequestRef pr = 1; }
   message ListWatchedPullRequestsResponse { repeated PullRequestRef prs = 1; }
   message CollapsedBots { uint32 count = 1; repeated string authors = 2; string label = 3; }
   message PrFeedback {
     PullRequestRef pr = 1;
     repeated FeedbackThread threads = 2;
     repeated TopLevelReview reviews = 3;
     CollapsedBots collapsed_bots = 4;
     string etag = 5;
   }
   ```

### `apps/humata2/api/proto/humata/v1/boards.proto`
1. **Local review state (`ATM-56`, `ATM-71`, `ATM-72`):**
   ```protobuf
   enum LocalReviewState {
     LOCAL_REVIEW_STATE_UNSPECIFIED = 0;
     LOCAL_REVIEW_STATE_DRAFT = 1;
     LOCAL_REVIEW_STATE_LOCAL_REVIEW_REQUESTED = 2;
     LOCAL_REVIEW_STATE_CHANGES_REQUESTED = 3;
     LOCAL_REVIEW_STATE_SUBMITTED = 4;
     LOCAL_REVIEW_STATE_APPROVED = 5;
     LOCAL_REVIEW_STATE_DONE = 6;
   }
   message LocalReviewLayer {
     string repo = 1;
     string base_branch = 2;
     string head_branch = 3;
     string title = 4;
     optional uint32 pr_number = 5;
   }
   message LocalReview {
     LocalReviewState state = 1;
     repeated LocalReviewLayer layers = 2;
     string summary_what = 3;
     string summary_why = 4;
     string summary_details = 5;
     bool owner_approved_push = 6;
     bool fixes_pushed_since_review = 7;
     optional string attachment_reason = 8;
     bool auto_attached = 9;
   }
   // Additive field on TodoFields (next free: 32):
   LocalReview local_review = 32;
   ```
2. **Manual watched PRs (`ATM-76`):**
   ```protobuf
   message ManualWatchedPr {
     string owner = 1;
     string name = 2;
     uint32 number = 3;
     string url = 4;
     int64 added_at_ms = 5;
   }
   ```

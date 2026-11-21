# Milestone PERMAUDIT — Permission Audit & Smarter Local Defaults

Covers **ATM-62** (Packaged hooks never gate generic agent permissions) and **ATM-28..35** (Capability inventory, proposal, new scoped defaults, migration, Settings › Agent access declutter, docs/skills sync, and CI tests).

## 1. Packaged Hooks Boundary (ATM-62)

- **Boundary:** Generic agent operations (`Bash`, `git status`/`diff`/local CLI, and file edits inside the session workspace) belong to each agent's own permission engine. Humata approval gates apply only to Humata-owned MCP/CLI operations and external/destructive side effects.
- **Packaged configs:**
  - Copilot (`daemon/agent_hooks::copilot_plugin`): `permissionRequest` hook removed from packaged `hooks.json`. Turn, `ask_user` (`preToolUse`), and conversation tracking hooks remain.
  - Claude (`daemon/humatad/src/session_service/turns.rs` seam): remove `PermissionRequest` from packaged Claude settings JSON (seam reported to coordinator).
  - Codex: runtime configuration (`codex_hooks`) verified separately; `codex_trust` flow remains intact.
- **Verification & CI:**
  - `apps/humata2/daemon/agent_hooks/verify_packaged_hooks.sh --verify <dir>` checks packaged hook configs and exits non-zero if `permissionRequest` or `PermissionRequest` is present.
  - `apps/humata2/daemon/agent_hooks/tests/packaged_hooks.rs` runs in CI to assert `copilot_plugin` never reintroduces `permissionRequest`, `claude_settings_has_permission_hook` flags any reintroduced `PermissionRequest`, and `verify_packaged_hooks.sh --verify` passes on clean configs and fails visibly on dirty configs.

## 2. Capability Inventory & Proposal (ATM-28, ATM-29, ATM-30, ATM-32)

All 58 capabilities in `Capability::ALL` (`daemon/access/src/capability.rs`):

### 2.1 Promoted from `AskMe` → `DefaultAllow` (10 safe local same-channel capabilities)

| # | Capability | Old Tier | New Tier | Scope & Narrowing | Why |
| --- | --- | --- | --- | --- | --- |
| 1 | `FilesReadOtherSession` | AskMe | DefaultAllow | Same machine + same channel (`read_place`) | Read-only, local, reversible |
| 2 | `FilesCopyIn` | AskMe | DefaultAllow | Same machine + same channel (`copy_files`); cross-machine routes to `FilesReadOtherMachine` (Ask) | Reversible copy into own clone/`shared/` |
| 3 | `SessionStartSameMachine` | AskMe | DefaultAllow | Same machine + same channel (`use_need`); top-level Desk asks unless owner sets Allow | Local session start/fork within channel |
| 4 | `SessionNote` | AskMe | DefaultAllow | Same channel (`use_need`) | Non-destructive inter-session note/question |
| 5 | `OwnSessionManage` | AskMe | DefaultAllow | Own session only (`session.id == me.id`) | Rename/repo-adjust own session; reversible |
| 6 | `OwnSessionEnd` | AskMe | DefaultAllow | Own session only (`session.id == me.id`) | Stop/archive own session |
| 7 | `PrReviewStart` | AskMe | DefaultAllow | Own channel (`in_channel`) | Starts read-only local review session |
| 8 | `TodoClose` | AskMe | DefaultAllow | Own session (`session.id == me.id`) | Reversible board state transition |
| 9 | `ChannelBoardEditOthers` | AskMe | DefaultAllow | Agent-created items (`created_by: Some(_)`); owner-created (`created_by: None`) still asks unless owner sets Allow or grants | Agents coordinate on agent-created To-dos without touching owner-created ones silently |
| 10 | `TrashRestore` | AskMe | DefaultAllow | Own channel (`in_channel`) | Restoring from trash is non-destructive |

### 2.2 Owner's Push Rule (`GitPush`, D920)

- **Push its own branch (no open PR · non-force, non-default branch):** `Target::PushBranch` → **Allow** by default (still refuses if `GitPush == Never` or `Review { push_allowed: false }`; default branches `main`/`master`/`HEAD` still Ask).
- **Open a PR, or push to a branch with an open PR:** `Target::Push` → **Ask** by default (`GitPush` stays `Tier::AskMe`), silent when approved in Review or explicitly told.

### 2.3 Remaining 48 Capabilities (unchanged tiers)

- **Already `DefaultAllow` (16):** `OwnBoardWrite`, `TodoSuggest`, `ChannelRead`, `ChannelBoardAdd`, `ChannelBoardEditOwn`, `SharedWrite`, `FilesFetchLocal`, `ViewsPrepare`, `ReviewDraft`, `InboxAsk`, `ReviewFocus`, `ReviewNote`, `ReviewOrder`, `CommentReply`, `CommentResolve`, `GlobalRead`.
- **Stay `AskMe` (10):** `GlobalBoardWrite`, `FilesReadOtherMachine`, `SessionType`, `SessionStartOtherMachine`, `SessionMove`, `GitPush`, `MachineProvision`, `ChannelCreate`, `ChannelRepoAdd`, `SessionStopOther`.
- **Stay `AskNoAllow` (4):** `NetworkReach`, `DeskMoveToChannel`, `ProviderImport`, `SettingsChange`.
- **Stay `NeverByDefault` (6):** `SessionDeleteOther`, `SessionsDeleteMany`, `MachineSessionsStop`, `ChannelChange`, `MachineDestroy`, `MachineScriptRun`.
- **Stay `UserOnly` (10):** `InboxAnswer`, `AccessChange`, `MarksChange`, `PrSubmit`, `UserReviewState`, `TrashEmpty`, `HostSecrets`, `ChannelDelete`, `MachineSettings`, `TerminalInput`.
- **Stay `Peer` (2):** `PeerReport`, `HostControl`.

(Total: 10 promoted + 16 existing DefaultAllow + 10 AskMe + 4 AskNoAllow + 6 NeverByDefault + 10 UserOnly + 2 Peer = **58**.)

### 2.4 Non-table checks

- `AwayRule::AllowLowRisk` (`policy.rs`): gated by `Capability::may_be_low_risk()` (never allows `AskNoAllow`, `NeverByDefault`, `needs_host()`, `SessionType`, or `SessionStopOther`).

## 3. Migration of Existing Channels (ATM-33)

- `ChannelRules` tracks `owner_set: BTreeSet<Capability>` whenever `Policy::set_rule` is called (without altering `RulesState` wire shape).
- `Policy::plan_permaudit_migration` (pure dry run) and `Policy::apply_permaudit_migration` carry existing channels and `new_channels` from the old `Ask` default to `Allow` for `Capability::permaudit_promoted()`:
  - Rules at `Ask` not in `owner_set` → `Allow`.
  - Rules in `owner_set` (even at `Ask`) → untouched.
  - Rules at `Never` → **never loosened**, even without an `owner_set` mark.
  - Rules outside the 10 promoted capabilities → untouched.

## 4. Settings › Agent Access Declutter (ATM-31)

Matches `DECISIONS.md` §4 and `01-agent-access-cleaned.png`:

- 2 primary groups: **LOCAL & SAME-CHANNEL** (7 rows with neutral inline scope chips) and **LEAVES THIS MACHINE OR AFFECTS OTHERS** (5 rows).
- `"Runs automatically within scope · asks in Agent inbox outside scope"` stated once in the section header; repeated right-hand note column removed.
- No permanent `"New default"` pills and no permanent Inbox receipt banner.
- 6 rarer rules tucked behind `Show 6 advanced rules ⌄` in the footer bar.

## 5. Required Coordinator Seams (outside PERMAUDIT claims)

1. **`apps/humata2/daemon/humatad/src/session_service/turns.rs:850-855,884`**: remove `PermissionRequest` entry from Claude's packaged `hooks_json` (keep `Notification`, `SessionStart`, `UserPromptSubmit`, `PostToolBatch`, `Stop`, `StopFailure`, `PreToolUse` AskUserQuestion, `PermissionDenied`, `Elicitation`, `ElicitationResult`).
2. **`apps/humata2/daemon/humatad/src/rules_kept.rs:127`**: optionally call `restored.apply_permaudit_migration(&BTreeMap::new(), &BTreeSet::new())` (or wire persisted `owner_set` column) on startup restore so pre-PERMAUDIT stores carry existing channels forward.
3. **`apps/humata2/clients/src/fixture/rules.rs`**: update the 10 promoted capabilities' fixture defaults from `Some(Ask)` to `Some(Allow)` when re-recording design goldens.

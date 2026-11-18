# OWNERFIX: the owner's frame notes hold in the frames and the app

- **Status:** open (2026-11-16)
- **Depends on:** VIEWS (Changes panel, review daemon), ROAM (desk move frames), PR (archive drafts)
- **Layer:** L3 · **Lane:** frames + UI + review wire

## Outcome
Every note the owner left on frames 76, 88–93, 98 and 113, and D513, shows the same in the frame drawings and in the app.

## Exit criterion
- 76: a selected sidebar row's "offline" is in the selected text colour.
- 88, 89: no Folder path row; on 88 it sits behind Details (D546).
- 90: the title reads "Plan Q4 board".
- 91: the Target cell ellipsizes with a gap before Agent; This Mac and devbox-1 are drawn alike; the footnote is the desk-session sentence, in the frame and the app.
- 93: a blocked switch is dimmed, on or off, in the frame and the app.
- 98: Humata never tells the person to run git. Find the default branch runs `git remote set-head origin --auto` in that clone on its machine; the repo's "against" chip is the other way; Try again shows only after a failed find, with its reason. Wired through UI, reducer, client, fake, live client and daemon, with tests.
- 113: the first summary line wraps inside its box.
- D513: archiving keeps unsubmitted pull request drafts; restore brings them back; only delete removes them. The dialog says they are kept; an archive-then-restore test pins it (fake and daemon).

## Lane (written by the manager)

Manager: OWNERFIX (Opus 5.5), opened 2026-11-16. Delivery is anchor-only `--check` scripts in the session scratchpad `ownerfix/`; main applies them in order.

### Claims
- desktop/tests/design/{sidebar,roam,diff_targets,fork_move}.rs (frames 76, 88–91, 93, 98, 113)
- ui/src/{sidebar,channel_desk,desk_info,overlay_continue,overlay_archive}.rs
- daemon/humatad/src/review/default_branch.rs (new), features/review/tests/default_branch.rs (new), clients/tests/archive_drafts.rs (new)

### Seams (files other lanes own)
- VIEWS: desktop/tests/design/diff_targets.rs, daemon/humatad/src/review.rs, review/bases.rs, daemon/review_diff/src/error.rs, daemon/humatad/tests/review_per_repo.rs, features/review/src/lib.rs, ui/src/panel_changes.rs
- AGENTSET: ui/src/overlay_desk_channel.rs (passes the blocked flag)
- PR: daemon/humatad/tests/pr_review.rs
- Shared: api/proto/humata/v1/{errors,review,sessions}.proto, cli/src/exit.rs, cli/tests/exit_codes.rs, daemon/humatad/src/mcp.rs, daemon/mcp/src/daemon.rs, api/GRPC.md, SPEC.md (PR-17), clients/src/daemon.rs, clients/src/daemon/fake.rs, clients/live/src/{daemon,diff,connection}.rs

### Decisions
- Frame 98 wording (Fable): button "Find the default branch"; line "ledger has no default branch to compare with."; hint "Or pick a branch to compare against." beside the chip; busy "Finding the default branch…"; failure "Couldn't find ledger's default branch." with the reason, Try again and the hint.
- Error kind `no_default_branch` = 62 (proposed; main assigns kind numbers), FAILED_PRECONDITION, CLI exit 7.
- D513 needed no daemon code: only deleting a review's session removes its comments.

### Scripts (apply order)
| Script | What | State |
|---|---|---|
| 01_frames.py | frames 76, 88, 89, 90, 91, 93, 98, 113 | sent |
| 02_ui.py | app 76, 91, 93, 113; D513 dialog line | sent |
| 03_d513.py | D513 tests (fake + daemon), docs, SPEC PR-17, proto comment | sent |
| 04_default_branch_seams.py | NoDefaultBranch kind, FindDefaultBranch rpc, kind lists | sent |
| 04_default_branch.py | frame 98 wiring: daemon, clients, fake, live, reducer, UI, tests | sent |
| 05_milestone.py | this file | sent |

### Follow-ups
- The live client sends a per-repo DefaultBranch in a Spec as `origin/HEAD`; with no origin/HEAD that may surface as a git error, not NoDefaultBranch.
- relay.rs's allowlist relays GetDiff but not ListBases or FindDefaultBranch; sessions on other machines need both relayed.
- DECISIONS.md D513/PR-17 entry (line ~3164) still quotes "archiving drops them"; main owns DECISIONS.md.

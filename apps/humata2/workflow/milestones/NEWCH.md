# NEWCH: New channel lets me pick repos from gh and machines, as drawn

- **Status:** done (this commit, 2026-11-16; found by the POLISH coach comparing frame 44): batches 1 and 2 green at gate_b3 and gate_b5; picks in D272–D280
- **Depends on:** MACH (machines and their online state), FL (channel creation and clone progress), REVIEW* (shares ui/src/ and the look_test budget; can't close until REVIEW's frames are green)
- **Unblocks:** V1
- **Layer:** L4 · **Lane:** UI + clients (+ a daemon read if gh repos aren't served yet)

## Outcome
I open New channel and see the sheet in frame 44: labels on the left (Name, Repos, Machines), a "Folder: ~/Humata/<name>/" line under Name, a repo search over the repos `gh` sees on This Mac with checkboxes, a machine checklist where offline machines show but can't be picked, and a primary button that says what it will do ("Create channel and clone 2 repos"). Creating it clones the picked repos on the picked machines (frame 45 progress, already built in FL).

## Exit criterion
- Frame 44 rendered natively in its default state by a native test, and look_test holds it under budget against `design/44-new-channel.png`.
- Repos come from the host's gh (reuse the saved-repositories source if one exists; never write the GitHub token to disk or send it to clients).
- Offline machines are listed and disabled (CH-4). Zero repos is valid (CH-10).
- Fakes and live both drive it; headless only.

## Read these (pointers)
- SPEC CH-1 to CH-5, CH-10 (grep `\*\*CH-` in apps/humata2/SPEC.md).
- Design code: `desktop/tests/design/new_channel.rs` (frames 44, 45). Golden `desktop/tests/__snapshots__/design/44-new-channel.png` (never edit goldens).
- Code: `ui/src/overlay_new_channel.rs`, `ui/src/overlays.rs`, clients model/fake for create_channel, `daemon/humatad/src/channel_service*`.
- POLISH.md's frame-44 table (what differs today).

## Out of scope
Adding a repo to an existing channel (CH-6, CH-18), the channel page.

## Lane (written by the manager)

Manager: minion NEWCH (Opus 5.5), started 2026-11-16. Scratch:
`scratchpad/newch/` (seam batches in `scratchpad/newch/batches/`).

### Goal and exit
Frame 44 drawn natively by a new native test (`44-new-channel`) and held by
look_test under budget; repos come from the host's `gh` through the daemon's
SearchGitHubRepos (today a `not_built` stub; the token never leaves `gh`);
offline machines listed and disabled (CH-4); zero repos valid (CH-10); fakes
and live both drive it.

### Design notes
- **Daemon:** serve `ChannelService.SearchGitHubRepos` from a new
  `channel_service/gh_repos.rs`: runs the host's `gh` (`HUMATA2_GH` in admin
  builds, as `gh_login.rs`) to list the repos the signed-in user can see,
  most recently pushed first, fuzzy-filtered by the query, paged. Peers are
  refused; other callers pass the GlobalRead guard like ListChannels. No new
  RPC, no token handling (gh keeps its own login).
- **Clients:** `Daemon::github_repos(query) -> Reply<Vec<String>>`
  (`owner/name`), default NotServedYet; fake lists a fixed set (acme/ledger,
  api, web, infra, docs and two non-acme repos) with `set_github_repos` and
  `Operation::GitHubRepos` for `fail_next`; live calls the RPC. A shared
  `clients::github::matches(query, repo)` fuzzy rule (case-insensitive
  subsequence) for the fake and the reducer.
- **Reducer (`features/app/src/new_channel.rs`):** the sheet keeps the name,
  the search query, the listed repos (asked once when the sheet opens,
  filtered locally as typed), ticked repos, ticked machines (This Mac ticked
  when online). New actions `Query`, `ToggleRepo`, `ToggleMachine`,
  `ReposListed`; `Repos(String)` goes. Offline machines don't toggle;
  Create refuses no machines; sends the ticked repos and machines. Helpers
  `folder()` (CH-2, same rule as `humata2_sessions::channel_folder`),
  `shown_repos()`, `create_label()` ("Create channel and clone 2 repos").
- **View (`ui/src/overlay_new_channel.rs`):** frame 44's layout: 560 pt card,
  right-aligned 110 pt labels (Name, Repos, Machines), the folder line, the
  search field with its icon, "From gh on This Mac. Zero or more.", repo
  and machine checkbox rows (offline at half opacity, not clickable), Cancel
  and the primary button with `create_label()`. Frame 45 progress unchanged.

### Tasks
1. **The daemon lists the repos gh on the host sees, fuzzy-matched (CH-3).**
   New `daemon/humatad/src/channel_service/gh_repos.rs`, new
   `daemon/humatad/tests/gh_repos.rs`; seam into `channel_service.rs`.
2. **The app's clients can ask for those repos, on fakes and live.**
   New `clients/src/github.rs`, `clients/src/daemon/fake/channels.rs`, new
   `clients/tests/github_repos.rs`; seams into `clients/src/{lib,daemon}.rs`,
   `clients/src/daemon/fake.rs`, `clients/live/src/daemon.rs`.
3. **The New channel sheet keeps ticked repos and machines and creates with
   them (CH-1 to CH-4, CH-10).** `features/app/src/new_channel.rs`, new
   `features/app/tests/new_channel.rs`, `features/app/tests/new_channel_setup.rs`.
4. **New channel is drawn as frame 44.** `ui/src/overlay_new_channel.rs`,
   new `desktop/tests/native/new_channel.rs`; seams into `ui/src/overlays.rs`
   (Query action, placeholder, 50 pt top), `desktop/tests/native.rs` (mod),
   `desktop/tests/look.rs` (frame 44 entry).

Tests first: the manager writes the failing tests for 1–4, then workers
implement.

### Claims
`ui/src/overlay_new_channel.rs`, `features/app/src/new_channel.rs`,
`features/app/tests/{new_channel,new_channel_setup}.rs`,
`clients/src/github.rs` (new), `clients/src/daemon/fake/channels.rs`,
`clients/tests/github_repos.rs` (new),
`daemon/humatad/src/channel_service/gh_repos.rs` (new),
`daemon/humatad/tests/gh_repos.rs` (new),
`desktop/tests/native/new_channel.rs` (new). Labelled seams to main:
`clients/src/{lib,daemon}.rs`, `clients/src/daemon/fake.rs`,
`clients/live/src/daemon.rs`, `daemon/humatad/src/channel_service.rs`,
`ui/src/overlays.rs`, `desktop/tests/{native,look}.rs`.

### Workers
| Worker | Model | Task | Files | Status |
|---|---|---|---|---|
| T1 | Haiku 5.5 high | daemon SearchGitHubRepos over gh | channel_service.rs, channel_service/gh_repos.rs | done (uncompiled) |
| T2 | Haiku 5.5 high | clients github_repos (trait, fake, live, matching) | clients/src/github.rs, daemon.rs, fake.rs, fake/channels.rs, lib.rs, live daemon.rs | done (uncompiled) |
| T3 | Haiku 5.5 high | New channel reducer: ticks, query, create | features/app/src/new_channel.rs | done (uncompiled) |
| T4 | Sonnet 5.5 medium | frame-44 sheet view | ui/src/overlay_new_channel.rs, ui/src/overlays.rs | done (uncompiled) |

### Picks (for DECISIONS.md)
- The daemon lists repos with `gh api --paginate "user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member"` cut by `--jq` to the proto fields, at most 1000; past that the person types to narrow.
- The app asks once when the sheet opens (empty query) and filters locally; the daemon still honours query and paging for CLI and MCP.
- Matching is case-insensitive subsequence over owner/name in two tiers: whole-substring matches first, then scattered ones; gh's pushed order within a tier.
- The list scrolls about 8 rows and shows the first 50 matches, then a faint "and N more; type to narrow".
- This Mac starts ticked when online, else the first online machine.
- Ticked repos always show first and stay ticked whether or not they match.
- Peers are refused; others pass the GlobalRead guard like ListChannels (an Ask waits in the Inbox).
- While listing the sheet says "Listing repos from gh…"; failures say what to do (install gh, or run `gh auth login`) with a Retry; Create needs no repo.
- The daemon keeps a successful listing in memory for 60 s, never on disk; failures are not kept.

### Log
- 2026-11-16: plan written.
- 2026-11-16: failing tests written (app, clients, humatad, native frame 44); T1–T4 spawned on scratch copies.
- 2026-11-16: T1–T4 done; batch 1 (scratchpad/newch/batches/batch1.py, 12 changed / 38 hunks, 6 new) checked against the repo and sent to main for bazel.
- 2026-11-16: batch 1 gated (gate_b3); look_test 44 at 2.815%. Batch 2 sent: title 15 semibold, hints/notes 11.5 secondary, checkbox gap per design, unused overlays::field removed.
- 2026-11-16: batch 2 green at gate_b5 (look_test 44 within budget; overlays.rs `field` warning gone; main dropped the then-unused `widgets::self`). Closed.

### Close check (2026-11-16)
- Frame 44 natively, look_test under budget: `desktop:native_test` (`new_channel_frame_44`) and `desktop:look_test` (`44-new-channel`), green at gate_b5.
- Repos from the host's gh: humatad serves SearchGitHubRepos (`channel_service/gh_repos.rs`) by running `gh api --paginate user/repos`; gh keeps its own login and the token is never read, stored or sent. No saved-repositories source existed, so none was reused. `humatad:gh_repos_test`.
- CH-1: name, zero or more repos, one or more machines in any mix; a channel without This Mac is made (`features/app:new_channel_test`).
- CH-2: the folder is shown live under Name with the daemon's hyphenate rule (`NewChannel::folder`).
- CH-3: gh's repos with fuzzy search and checkboxes; Create says how many repos it clones. "Repos already in the channel are left out" applies to adding repos to an existing channel (CH-6), out of scope here.
- CH-4: offline machines are listed, dimmed and can't be ticked (reducer and native test).
- CH-5: unchanged from FL (frame 45 progress); `features/app:new_channel_setup_test` passes with the new ticks.
- CH-10: Create with no repos makes and opens the channel (reducer and native test); sessions in an empty folder are FL's, unchanged.
- Fakes and live: the fake daemon drives every test above. The live client calls SearchGitHubRepos 100 at a time and follows page tokens (`clients/live/src/daemon.rs`); `clients/live:github_repos_live_test` runs it against a real humatad2 with a fake gh (251 repos over three pages, in order, and a query).
- Not done (outside the exit): page tokens are list offsets, not the pinned snapshots `common.proto` promises (a cache refresh between pages can shift items).

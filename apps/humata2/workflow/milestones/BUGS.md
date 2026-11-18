# BUGS: The likely bugs the use cases found are confirmed and fixed, each with a test

- **Status:** in progress (lane BUGS)
- **Depends on:** the use cases (`flows/cases/`), which found the bugs
- **Unblocks:** none

## Outcome
Each likely bug in `flows/cases/README.md` › "Likely bugs" that this lane
owns is either confirmed and fixed with a behavioural test, or dropped with
the line that shows it is not a bug.

## Exit criterion
Every owned item has a verdict below; each fixed one names its failing-first
test and its bazel target, and main's batched build and tests pass for them.

## Read these (pointers, nothing else)
- `flows/cases/README.md` › "Likely bugs", and the use case each item cites
- SPEC rules each item names (grep by ID)

## Out of scope
- Terminal bugs 1–17 (separate lane).
- 30 and 31, moving-session state (ROAM).
- 40 and 41 (PR).
- `move_session.rs`, carry, `review/pull_requests`: written up for main.
- 13, 65, 66: the owner decides; listed with a recommendation in the report.

## Lane (written by the manager)
- Design notes: no builds or git in this lane. Every change is a python
  apply script in the session scratchpad `bugs/` (sha256 guard per file,
  anchored hunks, `--check`); SEAM hunks are marked. Workers write
  `hunks_<group>_<n>.py`, which builds `<group>_<n>.py` against the current
  tree, so a script whose file moved on is rebuilt by re-running its hunks
  file.
- Claims: hunks only, anchored, in the files each item names (listed per
  script when it is sent to main).
- Tasks (worker groups; scripts in the scratchpad `bugs/`, each built by
  its `hunks_<group>_<n>.py`):
  - [x] keys (manager): `keys_1.py` (18, 19, 20, 21, 24, 27); 22 dropped
  - [x] nav: `nav_1.py` (25, 26, 28), `nav_2.py` (23, held: cost)
  - [x] sess: `sess_1.py` (29, 32, 34, 38), `sess_2.py` (33, 35, 37); 36 written up
  - [x] inbox: `inbox_1.py` (39, 45, 46, 47), `inbox_2.py` (43, 44); 42 written up
  - [x] agent: `agent_1.py` (48, 49, 50), `agent_2.py` (52, 57, 64)
  - [x] cli: `cli_1.py` (53, 55, 67), `cli_2.py` (54, 56)
  - [x] queue: `queue_1.py` (51 docs, 59, 61), `queue_2.py` (60, 62, 63); 58 written up
  - [x] DECISIONS D320-D334: `decisions_1.py`
  - [x] follow-ups from main (built against the chain's result, `post/`):
    `q63_1.py` (63 on the clock client), `r39_1.py` (39 InputState reply),
    `files23_1.py` (23 lazy, replaces nav_2), `stop_1.py` (36),
    `stop_2.py` (32 leftover), `rev42_1.py` (42), `labels_1.py` (18 labels
    and Linux keys in desktop tests), `files23_2.py` (23 pushed `FileChanged`
    events, after files23_1 and rev42_1), `files23_3.py` (diff only the paths
    notify names; full relist only on rescan); DECISIONS D340-D344: `decisions_2.py`
- Verdicts:
  - Fixed with a test: 18, 19, 20, 21, 24, 25, 26, 27, 28, 32, 33, 34 (files
    half), 38, 43, 44, 46, 47, 48, 49 (fork reason), 50, 52, 53, 54, 55, 56,
    57, 59, 60, 61, 62, 63, 64, 67, 23 (lazy), 36, 42, 32 leftover.
  - Fixed, no behavioural test possible (wording or view only): 29, 35
    (part), 37, 39, 45, 51 (docs: raw keystrokes are right).
  - Dropped: 22 (`ui/src/overlay_palette.rs:181-199` already says "Nothing
    matches" when no command or hit matches).
  - Written up, not fixed: 34 moves half (`move_session.rs:162` needs
    `machine_ready`; main), 36 (needs `STOPPED_BY_KIND_SCRIPT` proto and
    model seams; now `stop_1.py`), 42 (now `rev42_1.py`),
    49 AddRepo reason (with building `channel_add_repo`), 58 (no verified
    Codex rollout fixture).
  - For the owner: 13, 65, 66 (recommendations in the report).
- Waiting on: main's apply, build and tests, in the order of
  `batch.py --check nav_1.py keys_1.py queue_1.py queue_2.py agent_1.py
  agent_2.py cli_1.py cli_2.py sess_1.py sess_2.py inbox_1.py inbox_2.py
  decisions_1.py`, then the follow-ups in any order (no shared files), then `files23_2.py`, `files23_3.py`
  (path-only diffs), then
  `decisions_2.py`. nav_2 is held and replaced by `files23_1.py`.
- Build gates: none run yet (no builds in this lane).
- Ready to commit: no

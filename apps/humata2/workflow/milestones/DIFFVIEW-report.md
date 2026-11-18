# DIFFVIEW live test report (2026-11-18)

**Setup:** the real app and daemon with an isolated home (/tmp/h2diff.Bvj5). One session holds 4 local repos:
- **large:** a 60k-line file with 20k changed lines, plus binaries.
- **many:** 177 files, then 586 with bulk untracked folders.
- **medium:** a copy of a real Rust source file.
- **small:** the stress cases.

Screenshots are in /tmp/h2diff.Bvj5/shots/. The PR view wasn't run live: it needs a signed-in gh and a real PR.

## Findings

1. **The UI looks broken** (medium).
   - Repo names show as "/large" because `clients/live/src/map.rs:64` `repo_text` always formats `{owner}/{name}`, and the owner is empty for local clones. Print the name alone when the owner is empty.
   - The per-repo base chip shows only a chevron. In `ui/src/panel_changes.rs:~1462-1476`, the ellipsized text has min_w_0 next to a flex_1 spacer, so Taffy shrinks it to 0 (repo_head, :1383-1393).
   - The base menu popover is clipped at the panel's right edge.
   - "Push & PR" shows on every repo even with nothing to push (`push_control`, :1393/1416).
   - In narrow mode, the list paints over the "0 draft comments" footer.
   - Binary files show "+0 −0", an empty body and no "binary" note. Cause: the client model `clients/src/model.rs:1108` `FileDiff { path, added, removed, lines }` drops the proto FileSummary fields old_path, change, binary, similarity and mode (`api/proto/humata/v1/review.proto:262-275`), and `clients/live/src/diff.rs:405-485` maps binary files to empty lines.
   - Hunk headers have no counts: "@@ -1 +1 @@" over a 3-line hunk. The client's hunk mapping in clients/live/src/diff.rs drops them.

2. **Long content can't expand** (high). With the base "origin/main...HEAD + staged + unstaged + untracked", the Expand and Close panel buttons sit past the panel's right edge.
   - `ui/src/panels.rs:122-171` `header()` lays out title + extra + a flex_1 spacer + the buttons, with nothing shrinking.
   - The base chip at `panel_changes.rs:589-612` is flex_shrink_0, with the full "Against {base}" label.
   - Fix: let the chip shrink (min_w_0 plus ellipsis) and keep the buttons flex_shrink_0.

3. **Clicking a file shows no diff text** (critical, narrow/default panel). The code view gets 0 px height.
   - `panel_changes.rs:645-670`: the body is a flex column holding the list and then the view.
   - The list (:403-415) isn't a scroll container unless `inline_handle` (review inline mode) is set, and it's only `w(300).h_full()` when full && !inline. So the list takes its full natural height and the code view (:466-565, flex_1 min_h_0) gets 0.
   - Fix: wrap the list in an overflow_y_scroll container with a bounded basis or height, in both modes.

4. **The list can't scroll in expanded mode either** (high). With 586 files, the later repos can't be reached. The cause is the same as 3. The repo chevron (:1375) also has no click handler, so a repo can't collapse.

5. **Big diffs make the app unusable** (critical). big.txt (+20000 −20000):
   - first open takes ~70 s; reselecting takes ~25 s at 100% CPU;
   - RSS grows from 0.5 to 4.8 GB;
   - scrolling runs at 72–99% CPU.
   - The time goes to Taffy layout of every row. `code_rows` (panel_changes.rs:~2018) builds a div per line for the whole file: use gpui `uniform_list`/`list` (virtualized).
   - `code_rows_focused` (~2085) clones file.lines every frame.
   - `.test_support()` wraps every code element (:550).
   - A 194k-char minified line is fine: it renders wrapped in about 2 s.

6. **Loading is slow, shows no indicator and doesn't refresh** (high).
   - 177 files take ~30 s; a base switch takes 30–60 s+, and the chip shows the OLD base until the load finishes; toggling the panel refetches everything.
   - **Client:** `clients/live/src/diff.rs:430-463` calls get_file_diff for each file and page in sequence, all-or-nothing (`?`).
   - **Daemon:** `daemon/humatad/src/review.rs:607-690` get_file_diff re-runs default_bases (634), pull_request_bases (637) and a full `checkout.diff(...)` (655) on every call. That is a whole snapshot (add -u, ls-files --others, update-index, write-tree, diff-tree -M) even when the request carries trees, so the cost is O(N × repo).
   - **Reducer:** the chip label comes from the fetched diffs, not the picked target (`features/review/src/lib.rs:699-710` PickBase, 489-506 fetch). There's no loading state.
   - **Fix:** snapshot once per get_diff and reuse the trees (or batch), show a loading state with the picked target, and make per-file errors partial.

7. **A file deleted during a refresh fails the whole diff** (high). The banner reads: "git update-index --add -z --stdin failed … does not exist and --remove not passed".
   - Cause: `daemon/review_diff/src/checkout.rs:183-202` runs ls-files --others and then update-index --add without --remove or --ignore-missing.

## Stress cases

- **Long paths (207 chars, a deep rename):**
  - List rows clip with no ellipsis or tooltip, and two different long paths read the same: the filename is lost. Use middle or start ellipsis that keeps the filename.
  - The code header path is clipped at the panel edge.
- **Long repo name:** repo_head (:1377/1413) uses text(diff.repo) with no min_w_0 or ellipsis.
- **Long branch name:** repo_head shows the session's branch (:320-325, :1379), not each clone's own branch. The PR header branch (:983-1011) has no ellipsis.
- **Binary and 12 MB files:** "+0 −0", with no binary or size note.
- **No trailing newline:** no "\ No newline at end of file" marker.
- **Mode-only change:** "+0 −0", with no "mode 100644 → 100755" note.
- **Rename:** one row under the new path, "+0 −0", with no "renamed from" line. old_path and similarity are dropped.
- **Unicode and emoji names, CRLF, and minified lines:** OK.

## What works

- A small or medium file in expanded mode shows a correct diff.
- Pixel scrolling a medium file is smooth.
- The wrap toggle, the base picker list and "Try again" all work.
- Idle CPU is about 0%.

## Suggested order

1. A list scroll container in both modes (3, 4).
2. A header chip that shrinks (2).
3. Virtualized code rows (5).
4. The daemon snapshots once, plus --ignore-missing (6, 7).
5. Binary, old_path, change kind and mode carried through FileDiff, with notes (stress cases).
6. Names: owner-less repos, ellipsis, each clone's branch, chip width, hunk counts (1).

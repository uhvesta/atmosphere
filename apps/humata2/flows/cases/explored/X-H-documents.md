# Documents: a Desk document tab, comments, the version history, and the code editor

Layer: explored · macOS · harness: `//apps/humata2/desktop:native_test` (document::, doc_versions::, editor::, editor_comments::) and `//apps/humata2/desktop:live_flows_test` (document::, desk_document::)

Phase 1 (map from code and snapshots, no Bazel run). Every step marked PENDING RUN is filled in phase 2.

## Setup

- Native: the design workspace with the Desk, channels folded (`doc_app()` in `desktop/tests/native/document.rs`; `versioned_app()` in `desktop/tests/native/doc_versions.rs`). The versioned app has BRIN rollout RFC (`desk-brin-rfc`) with `rfc-brin.md` committed twice by the agent ("First draft" V1, "Add two" V2), and `notes/bench-results.md` saved in the same folder before the tab opens.
- Live: a real `humatad2`; the Desk session's document is in its own folder (no repo), and the channel-session version of the same script is `desktop/tests/live_flows/document.rs`.
- Snapshots used: `desktop/tests/__snapshots__/native/39-desk-document.view.txt` (and PNG). The side-by-side and comment frames (41 to 43) are not among the native snapshots; the design goldens under `desktop/tests/__snapshots__/design/` have 39-desk-document.png only.

## Steps

### 1. Open a Desk Markdown document as a tab (Preview)
- **Screen:** the session's tab row shows "claude", "zsh", "rfc-brin.md" (selected, with its Close tab). A radio group "Show the file as": Preview (selected), Source, Side by side. The page: heading "BRIN index rollout", "Draft · written with Claude Code · rfc-brin.md", sections Problem, Proposal (a list of three items with PAY-13 and PAY-14 references), Bench (a code block). The Files panel is open on the left with the session folder.
- **Offered:** the Preview, Source and Side by side radio buttons; Close tab; the file rows in the Files panel; Close panel.
- **Action:** Open `rfc-brin.md` from the Files panel (`SelectFile` then `OpenFile`).
- **Result:** The document opens as a tab beside the Files panel, in Preview.
- **Evidence:** `desktop/tests/__snapshots__/native/39-desk-document.view.txt` (and `39-desk-document.png`).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=document::` (includes `a_desk_document_opens_as_a_page_beside_the_files_panel`); expected: pass.

### 2. Switch the mode: Preview, Source, Side by side
- **Screen:** the same radio group. Source shows numbered lines (`doc-line-1` and up); Side by side shows the source and the page together.
- **Offered:** the three modes; Link scrolling (in Source and Side by side); the Settings › Long lines choice (wrap or scroll) affects the open source.
- **Action:** Choose Source, then Side by side.
- **Result:** The drawn view changes. In Source the line ids exist; in Preview alone nothing is linked, so the wheel changes nothing.
- **Evidence:** none in snapshots. The Side by side frame (42) is not in the native snapshot set.
- **Replay check:** `--test_filter=document::` (includes `the_mode_switch_changes_what_is_drawn`, `the_terminal_the_source_and_the_page_fit_side_by_side`, `the_page_marks_where_a_comment_is`); expected: pass.

### 3. Link scrolling
- **Screen:** the page and the source scroll together while linked.
- **Offered:** the Link scrolling tick.
- **Action:** Scroll the page by hand.
- **Result:** Scrolling the page by hand unticks Link scrolling. Ticking it again links them.
- **Evidence:** none.
- **Replay check:** `--test_filter=document::` (includes `link_scrolling_unticks_and_ticks_again`, `scrolling_the_page_by_hand_unticks_link_scrolling`); expected: pass.

### 4. A plain file has no mode switch
- **Screen:** a non-Markdown file opens with no radio group.
- **Offered:** the file only.
- **Action:** Open a plain text file as a tab.
- **Result:** No mode switch.
- **Evidence:** none.
- **Replay check:** `--test_filter=document::` (includes `a_plain_file_has_no_mode_switch`); expected: pass.

### 5. Comment on a source line (click, type, Comment)
- **Screen:** in Source, each line takes a click. The comment box opens under the line with a text field and the buttons Cancel and Comment.
- **Offered:** click a line (opens the box), shift-click a second line (stretches the box to a range, e.g. lines 13 to 14), type, Comment, Cancel.
- **Action:** Click line 7, type a comment, press Comment.
- **Result:** The comment is sent to the agent at once. Its line shows the sent comment (status "Sent" in the recorded-comment row). Cancel closes the box and leaves no comment.
- **Evidence:** none for the box (the frame 42 PNG is not in the native set).
- **Replay check:** `--test_filter=document::` (includes `clicking_a_source_line_opens_the_comment_box_on_it`, `shift_clicking_a_second_line_stretches_the_box_to_a_range`, `typing_in_the_box_and_pressing_comment_sends_it_to_the_agent`, `cancel_closes_the_box_and_leaves_no_comment`); expected: pass.

### 6. Drafts wait until sent together
- **Screen:** a draft box in the link bar; drafts show as "Draft" or "Waiting to send" until sent; the footer or bar gives "Send N comments".
- **Offered:** Send N comments, the draft box, Comment.
- **Action:** Write two drafts on different lines, then send.
- **Result:** Drafts are typed to the agent together when Send is pressed. Until then the page lights a draft and the box being written, never a sent comment.
- **Evidence:** none.
- **Replay check:** `--test_filter=document::` (includes `drafts_wait_in_the_link_bar_until_send_types_them_together`); expected: pass. Live: `--test_filter=desk_document::` (the full click-through, including Send N comments).

### 7. The rewrite: Resolve and Re-attach (live)
- **Screen:** after the agent rewrites the file, sent comments show their status (Addressed, Resolved), and a comment whose line moved can be re-attached.
- **Offered:** Resolve, Re-attach.
- **Action:** The test writes the rewrite itself (the agent stand-in), then clicks Resolve and Re-attach.
- **Result:** Status changes; a re-attached comment points at its new line.
- **Evidence:** none.
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:live_flows_test --test_filter=desk_document::`; expected: pass.

### 8. The version history strip (Desk Markdown tab with versions only)
- **Screen (from `ui/src/doc_history.rs`):** a strip above the page with a position label ("Current", "v2 of 2", "No versions yet", "Current · unsaved changes"), buttons "Older version" and "Newer version", a "History ▾" menu (disabled while there are no versions), and "Save version". A "What changed? (optional)" field sits by Save version. The Save button is disabled with a reason: "No changes since v2" or "No changes to save".
- **Offered:** Older, Newer, the History menu (version list with +/− counts), Save version, the optional message, and on an old version Compare with current, Restore as new version, Back to current.
- **Action:** Open `rfc-brin.md` (two agent versions saved before).
- **Result:** The strip shows with "v2 of 2"-style labels. It does not show on a picked folder, which keeps no versions.
- **Evidence:** none. No doc-history snapshot exists in the native set.
- **Replay check:** `--test_filter=doc_versions::` (includes `the_strip_shows_on_a_desk_markdown_tab_with_versions`, `the_strip_is_not_on_a_folder_that_keeps_no_versions`); expected: pass.

### 9. Step back to an old version (read-only)
- **Screen:** an old-version bar (`doc-version-bar`, "Old version") with "v{n} · {lines} · {date}", a "read-only" tag, and "Compare with current", "Restore as new version", "Back to current". The page shows the old text with old-line ids; no line takes a comment, and the note "Comments weren't recorded for this version" shows.
- **Offered:** Older (back), Newer (forward), Compare with current, Restore as new version, Back to current.
- **Action:** Older (back) on `rfc-brin.md` from V2.
- **Result:** The view shows v2 (the newest saved version, one step back from Current), read-only; no comment box can open on it.
- **Evidence:** none.
- **Replay check:** `--test_filter=doc_versions::` (includes `stepping_back_shows_an_old_version_that_takes_no_comment`); expected: pass.

### 10. Save version needs a change
- **Screen:** Save version is disabled with the reason "No changes to save" or "No changes since v2".
- **Offered:** Save version (disabled until the file changed since the newest version).
- **Action:** Click Save version with no change (nothing happens); then the agent edits the file (the test writes a third line), reload, Save version.
- **Result:** With no change the version count stays at 2. After the edit and Save, the count is 3 and the newest version is "by You" (VersionBy::You), and Save is disabled again.
- **Evidence:** none.
- **Replay check:** `--test_filter=doc_versions::` (includes `save_version_needs_a_change_since_the_newest_version`); expected: pass.

### 11. Restore an old version adds a version
- **Screen:** on the old version, "Restore as new version" (enabled when ready).
- **Offered:** Restore as new version.
- **Action:** Back twice to v1 (the first draft), then Restore as new version.
- **Result:** A new version is added (3 versions, the newest "by You"), and the view returns to Current. History is not rewritten; the old text comes back as a new version.
- **Evidence:** none.
- **Replay check:** `--test_filter=doc_versions::` (includes `restoring_an_old_version_adds_a_version_and_returns_to_current`); expected: pass.

### 12. Compare with current
- **Screen:** the comparison view titled "<path> · v{n} against the current file" (`ui/src/doc_history.rs`, the compare card with "doc-compare-close").
- **Offered:** Compare with current, Close.
- **Action:** Back twice to v1, then Compare with current.
- **Result:** A comparison is asked of the diff view for the shown version (session, path, version id match).
- **Evidence:** none.
- **Replay check:** `--test_filter=doc_versions::` (includes `compare_with_current_asks_the_diff_view_for_the_shown_version`); expected: pass.

### 13. Side by side on an old version scrolls both sides together
- **Screen:** an old version in Side by side, both columns scroll.
- **Offered:** scroll.
- **Action:** Scroll a long Markdown text on an old version in Side by side.
- **Result:** Both sides scroll together.
- **Evidence:** none.
- **Replay check:** `--test_filter=doc_versions::` (includes `side_by_side_scrolls_both_sides_together_on_an_old_version`); expected: pass.

### 14. Code editor: a changed file opens inline, side by side or in full
- **Screen (snapshots `editor-inline`, `editor-side-by-side`, `editor-full-file`):** the session's Files panel on the left (Session files, with "ledger", "ledger/migrations" and the migration files; "A" for added). The file tab shows Inline, Side by side or Full file, with folds for unchanged stretches and a change counter for ↑/↓.
- **Offered:** the mode choice, unchanged-stretch folds (open and stay open), ↑ and ↓ (step through changes, wrap around), Show changes (turns the changes off in Full file).
- **Action:** Open a changed code file; switch modes; step with ↑ and ↓.
- **Result:** The mode and the folds change as shown; an unchanged file shows as a plain file with no modes.
- **Evidence:** `desktop/tests/__snapshots__/native/editor-inline.view.txt`, `editor-side-by-side.view.txt`, `editor-full-file.view.txt` (and PNGs).
- **Replay check:** `bazel test --jobs=4 //apps/humata2/desktop:native_test --test_filter=editor` (covers `a_changed_file_opens_inline_with_folds_that_open_and_stay_open`, `up_and_down_step_through_the_changes_and_wrap_around`, `side_by_side_puts_removed_lines_beside_the_added_ones`, `full_file_shows_everything_and_show_changes_turns_the_changes_off`, `an_unchanged_file_shows_as_a_plain_file_without_modes`); expected: pass.

### 15. Comments in the code editor share the diff's drafts
- **Screen:** a footer shows the drafts with "Send N comments". A comment is on the new-side line number; in Side by side, comments go to the new column only.
- **Offered:** write a comment on a line, send them together.
- **Action:** Draft on a line in the editor, then in the diff, then send.
- **Result:** The editor and the diff share one draft list and send together.
- **Evidence:** none.
- **Replay check:** `--test_filter=editor` (covers `a_comment_written_in_the_editor_is_a_draft_on_its_new_line`, `side_by_side_comments_on_the_new_column_only`, `the_editor_and_the_diff_share_drafts_and_send_them_together`); expected: pass.

## Not reachable headless

- Live Save version from the Close question of a Desk session: covered by the Desk page record (`X-H-desk-page.md`, step 8), not by the strip.
- The doc-history strip on a live Desk session with real agent commits: the native test commits the versions through the fake daemon (`commit_as_agent`), so the live path is not driven here.

## Surprises

- **Restore is an add, not a revert.** "Restore as new version" adds a version by "You" and returns to Current. The label says so; the history keeps the newer text. A person may expect Restore to change the file back in place. Facts only.
- **The optional message exists only on the strip.** The close question's "Save version and close" saves with an empty message. A version saved there has no note, and nothing on the strip shows that.
- **No strip on a picked folder.** A folder the person picked (e.g. `~/notes`) keeps no versions, so its Markdown files have no History and no Save version. The page does not say why.
- **Old versions take no comments, and the note says so only once you are on one.** The strip has no hint before stepping back.
- **Save version can be pressed only with a change, and the reason is in a small label.** "No changes since v2" sits next to a disabled button; nothing explains how to make a change.
- **No doc-history snapshot.** The native set has the page (39) and editor frames, not the strip, the old-version bar, or the compare card. Those are checked by the tests' asserts only.

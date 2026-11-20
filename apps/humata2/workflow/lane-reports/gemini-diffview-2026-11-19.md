warning: --mode auto is not available for this account; continuing in the default mode.
### Audit summary (DV-10–DV-19 & RESUME.md §1 GOLDENFIX in `ui/changes`)

Checked every item against the current tree before editing:
- **Already landed in tree:** DV-10–DV-19 core behavior (`repo_label`, `split_path`, name-first rows, `repo_head` clone branch + collapse + pinned totals, D868 per-repo bases, D869 PR repo chevron, DV-11 modes/`LineKind::Note`, DV-12 `file_note`/marks/loading, DV-13 `diff_stress.rs`, DV-15 file bar/stepping/list keys, DV-16 footer/push menu, DV-17 two number columns/sign column/hunk band/word highlight/Wrap switch, DV-18 single full-screen bar/Back/keys, DV-19 Compare with…/per-repo errors); plus GOLDENFIX items: repo row ellipsis, file header shrink, binary/mode-only single note, `ArrowUpRight` icon asset, file bar ellipsis, PR `+/−` spacing, 720 pt comment card cap (`comment_cards.rs` & `diff_body.rs`), and `"The changed lines aren't available"` fallback.
- **Remaining gaps implemented in this batch:**
  1. **DV-5 step 5 / DS-29 / GOLDENFIX `review_code` virtualization + `review_repo` `bottom_up` cache:** `review_code` now virtualizes files over `REVIEW_VIRTUAL_FROM` (1,500 lines) inside a bounded box (~60% of panel body height, clamped 160–720 pt) using `gpui_kit::list` with `reveal_virtual_focus` (`scroll_to_reveal_item`); files ≤ 1,500 lines keep frame 68's exact inline blocks. `review_repo` now caches `bottom_up` order per `(session, repo)` diff identity (`review_order`) and skips joining for ≤ 1 file.
  2. **DS-24 in inline PR review:** `review_code` renders `change-file-note` on coded PR files that have a note (rename/mode/error) and `change-file-empty` on PR files with no code lines (binary/mode-only/empty/error); normal coded files return the bare code element (frame 68 tree unchanged).
  3. **DS-9 in inline PR review:** `changes` shows `file_filter_field` above 20 files in inline PR reviews as well; `review_repo` skips filtered-out files and folded `review_repo_head` picks the first visible file.
  4. **DS-36 in `clients/live_diff`:** `read_diffs` captures per-file `GetFileDiff` failures (`Refused`, `NotFound`, `Conflict`) into `FileDiff.error` instead of aborting the whole repository read; connection/version/config errors (`Offline`, `NoDefaultBranch`, `MigrationNeeded`, `NotServedYet`, `Scripted`) still abort the read.
  5. **DS-45 / GOLDENFIX polish:** added `.flex_shrink_0().whitespace_nowrap()` on file-row `stat_word` / `+N` / `−N`, `.text_ellipsis()` on PR review footer `review-drafts-*`, and ellipsis on `plan_repo_line`.

---

### Files changed, new and deleted

**Changed:**
- [diff_body.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/changes/src/panel_changes/diff_body.rs) — added `REVIEW_VIRTUAL_FROM = 1500`.
- [review_header.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/changes/src/panel_changes/review_header.rs) — `review_code` virtualization (> 1,500 lines) with bounded box & `reveal_virtual_focus`, `review_order` cache for `bottom_up`, DS-24 `with_review_note` & empty-file body in `review_code`, DS-9 filtering in `review_repo` & `review_repo_head`, and unit tests (`review_box_height_is_about_sixty_percent_of_panel_and_bounded`, `review_order_puts_used_files_first_and_updates_when_files_change`, `review_order_handles_empty_and_single_file_diffs`).
- [mod.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/changes/src/panel_changes/mod.rs) — passed viewport height to `review_repo`, enabled `file_filter_field` in inline PR reviews (DS-9), added `.text_ellipsis()` on `review-drafts-*`.
- [file_list.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/changes/src/panel_changes/file_list.rs) — added `.flex_shrink_0().whitespace_nowrap()` on `stat_word`/`+{added}`/`−{removed}` (DS-45) and unit tests (`repo_label_strips_one_leading_slash`, `split_path_separates_directory_and_file_name`, `file_note_covers_binary_rename_mode_empty_deleted_and_error`).
- [plan.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/ui/changes/src/panel_changes/plan.rs) — added `min_w_0` + ellipsis on `plan_repo_line`.
- [diff.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/clients/live_diff/src/diff.rs) — captured per-file `GetFileDiff` failures into `FileDiff.error` (`file_read_error`) and added unit test `per_file_errors_are_captured_while_connection_errors_abort`.
- [file_filter.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/features/review/tests/file_filter.rs) — added `a_pull_request_review_with_more_than_twenty_files_filters_like_plain_diff`.
- [DIFFVIEW.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/workflow/milestones/DIFFVIEW.md) — updated `## Lane`.

**New:** none.  
**Deleted:** none.

---

### Seam lines for the main agent

1. **Native E2E tests for DS-29, DS-24, and DS-9 in PR review** — append to [pr_session.rs](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/desktop/tests/native/pr_session.rs#L718-L723) right after the closing `}` of `scroll_in_a_review_keeps_the_gutter_still` (line 723):

```rust
/// DV-5 step 5 / DS-29: a pull request file over 1,500 lines draws in its own
/// bounded box with a virtual list, scrolls to its end, and scrolls to a
/// distant agent focus inside that box.
#[test]
fn a_huge_file_in_a_pull_request_review_scrolls_in_its_own_box() {
    let huge: Vec<DiffLine> = (1..=4_000)
        .map(|n| line(LineKind::Added, None, Some(n), &format!("    let row_{n} = {n};")))
        .collect();
    let mut app = review_with(vec![file("src/huge.rs", huge)]);
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(ReviewAction::SelectFile {
        repo: "acme/api".to_owned(),
        path: "src/huge.rs".to_owned(),
    }));
    app.draw();
    app.draw();

    const CODE: &str = "change-code acme/api/src/huge.rs";
    assert!(app.contains("change-line-0"), "first lines are drawn");
    assert!(!app.contains("change-line-3999"), "distant lines are virtualized out");
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let code = window.find(CODE).bounds();
            assert!(code.size.height > px(100.), "box is usable");
            assert!(code.size.height < panel.size.height * 0.85, "box is bounded below panel height");
        })
        .expect("window stays open");

    // Agent focus far down scrolls inside the file's own virtual list.
    agent_focuses(&mut app, 1, "src/huge.rs", 2_500);
    for _ in 0..3 {
        app.draw();
    }
    assert!(app.contains("review-focus-2500"), "distant focused line comes into view");

    // Scrolling the box reaches the last line and drops the first.
    for _ in 0..3 {
        scroll_over(&mut app, CODE, 0., -10_000_000.);
    }
    assert!(app.contains("change-line-3999"), "last line is drawn");
    assert!(!app.contains("change-line-0"), "first line scrolled out");
}

/// DS-24: a binary file in an inline pull request review shows its note as its
/// body instead of an empty code area.
#[test]
fn a_binary_file_in_a_pull_request_review_says_binary() {
    let binary = FileDiff {
        path: "assets/logo.png".to_owned(),
        binary: true,
        ..FileDiff::default()
    };
    let mut app = review_with(vec![binary]);
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(ReviewAction::SelectFile {
        repo: "acme/api".to_owned(),
        path: "assets/logo.png".to_owned(),
    }));
    assert!(app.contains("change-file-empty"));
    assert!(!app.contains("change-file-note"));
}

/// DS-9: a pull request review with more than 20 changed files shows the
/// "Filter files" field.
#[test]
fn a_pull_request_review_with_more_than_twenty_files_shows_a_filter() {
    let more: Vec<FileDiff> = (0..22)
        .map(|n| file(&format!("src/extra_{n:02}.rs"), vec![line(LineKind::Added, None, Some(1), "let x = 1;")]))
        .collect();
    let mut app = review_with(more);
    shown(&mut app);
    assert!(app.contains("changes-file-filter"));
}
```

2. **Provisional decision D877** — append to [DECISIONS.md](file:///Users/avestabarzegar/oss/atmosphere-khodr/apps/humata2/DECISIONS.md) after `## D876`:

```markdown
## D877: Inline PR review virtualization box height and per-file diff errors (DIFFVIEW Gemini lane, 2026-11-19). Provisional

- **PR review virtual box (DS-29):** inline PR review files above 1,500 lines (`REVIEW_VIRTUAL_FROM`) render in a virtual `gpui_kit::list` box of height `((viewport_height - 104) * 0.6).clamp(160, 720)` with no invented border (matching frame 68's code block styling). Files at or under 1,500 lines keep inline blocks unchanged.
- **Per-file errors in `clients/live_diff` (DS-36):** after `GetDiff` succeeds for a repository, a file-scoped `GetFileDiff` failure (`Refused`, `NotFound`, `Conflict`) records `FileDiff.error` and clears that file's lines so the rest of the repository still loads; transport/version/config errors (`Offline`, `NoDefaultBranch`, `MigrationNeeded`, `NotServedYet`, `Scripted`) still abort the read.
- **Reverse:** restore unbounded inline rendering in `review_code` and restore `?` propagation on `get_file_diff` in `clients/live_diff/src/diff.rs`.
```

---

### Focused Bazel targets to build and test

```text
//apps/humata2/ui/changes:changes_unit_test
//apps/humata2/clients/live_diff:live_diff_unit_test
//apps/humata2/clients/live_diff:diffs_live_test
//apps/humata2/features/review:file_filter_test
//apps/humata2/features/review/...
//apps/humata2/desktop:native_test --test_arg=diff_stress:: --test_arg=pr_session:: --test_arg=diff_view:: --test_arg=diff_big:: --test_arg=diff_keys:: --test_arg=review_ui::
//apps/humata2/desktop:panels_test
//apps/humata2/desktop:design_test
```

---

### Goldens that need re-recording and frames Fable must judge

- **`diff-stress-pr-huge-file`** (`desktop/tests/__snapshots__/native/diff-stress-pr-huge-file.{png,view.txt}`): **expected to change** because `review_code` now renders the 40,000-line PR file inside its ~60% bounded virtual box (dropping runtime from ~68 s to sub-second) instead of rendering all 40,000 lines inline. Re-record and send to Fable to judge the virtualized PR box look (and confirm whether a subtle 1 px border/radius is wanted or frameless matches frame 68).
- **`68-pr-review-session`, `68b-pr-review-full-screen`, `68c-pr-review-after-show-terminal`, `69-pr-submit-review`, `11-panel-full-screen-diff`, `41-review-beside-terminal`, `changes-pushed`, `changes-wrapped-long-line*`:** **expected unchanged** (all ≤ 1,500 lines, coded, no file note, ≤ 20 files).

---

### What is still open

- **Design question for Fable:** should the virtualized > 1,500-line box inside an inline PR review (`diff-stress-pr-huge-file`) stay borderless like frame 68's inline code blocks (current implementation), or carry a subtle 1 px hairline border to cue its independent scroll?
- **Still open outside this lane / deferred by spec:**
  - `DS-24` `↗` Open in editor remains inert (no editor client exists; no open task).
  - `DS-28` per-file lazy loading ("Loading big.txt…") stays deferred pending DV-14 live rerun (P-5 586-file check).
  - `DV-14` live rerun (4-repo session + `pr_demo` measurements) and `RV-8a` (P-3 scroll CPU split) remain for main.
  - `RESUME.md §1` design-mock items outside `ui/changes` (`desktop/tests/design/{session,diff_targets,comments}.rs` narrow footer / mock hunk header) and `DIFFVIEW-spec.md` status-column updates remain outside this lane's claims.
agy exit 0 for diffview

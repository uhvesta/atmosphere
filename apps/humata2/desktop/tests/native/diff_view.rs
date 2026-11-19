//! The Diff panel's file list and header in a narrow and an expanded panel
//! (DV-1): the list scrolls, the code keeps room, and the header buttons
//! stay inside the panel.
use super::*;
use humata2_clients::model::{DiffLine, FileChange, FileDiff, LineKind};
use humata2_review::ReviewAction as R;

/// The "ledger" session's `api` repository holds 200 files, `src/client.rs`
/// among them, and `src/client.rs` is the selected file.
fn many_files_app() -> App {
    let mut workspace = fixture::design();
    let repo = workspace
        .diffs
        .iter_mut()
        .find(|d| d.repo == "api")
        .unwrap();
    let client: FileDiff = repo
        .files
        .iter()
        .find(|f| f.path == "src/client.rs")
        .unwrap()
        .clone();
    repo.files = std::iter::once(client.clone())
        .chain((0..199).map(|n| FileDiff {
            path: format!("src/file_{n:03}.rs"),
            ..client.clone()
        }))
        .collect();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app
}

/// In a narrow panel the file list takes at most 220 pt and the code keeps
/// the rest, above the footer.
#[test]
fn a_long_file_list_leaves_room_for_the_code() {
    let mut app = many_files_app();
    // A draft puts the footer's Send button in the footer, the edge to
    // measure the code against.
    app.write_comment("ledger", "api", 43, "Keep these writes in one transaction.");
    // With a file picked the list is folded into the file bar: open it.
    app.send(AppAction::Review(R::ToggleFileList));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let files = window.find("changes-files").bounds();
            let code = window.find("change-code api/src/client.rs").bounds();
            // The Send button sits inside the footer's padding, so its top
            // is just under the footer's top edge.
            let send = window.find("send-comments").bounds();
            assert!(f32::from(files.size.height) <= 220.);
            assert!(f32::from(code.size.height) > 100.);
            assert!(code.origin.y + code.size.height <= send.origin.y + px(0.5));
        })
        .unwrap();
}

/// Expanded, the file list takes the panel's full height and scrolls
/// inside it.
#[test]
fn the_file_list_scrolls_in_the_expanded_panel() {
    let mut app = many_files_app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let files = window.find("changes-files").bounds();
            let window_height = window.viewport_size().height;
            assert!(f32::from(files.size.height) > 220.);
            assert!(files.origin.y + files.size.height <= window_height + px(0.5));
        })
        .unwrap();
}

/// A base too long for the chip ellipsizes inside the header, and Expand
/// and Close stay inside the narrow panel.
#[test]
fn a_long_base_keeps_expand_and_close_inside_the_panel() {
    let mut workspace = fixture::design();
    for diff in workspace
        .diffs
        .iter_mut()
        .filter(|d| d.session == SessionId::new("ledger"))
    {
        diff.base = "origin/main...HEAD + staged + unstaged + untracked".into();
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let right = panel.origin.x + panel.size.width;
            let expand = window.find("expand-panel").bounds();
            let close = window.find("close-panel").bounds();
            assert!(expand.origin.x + expand.size.width <= right + px(0.5));
            assert!(close.origin.x + close.size.width <= right + px(0.5));
        })
        .unwrap();
}

/// The "ledger" session's repositories, in the order the panel draws them.
fn repos_of(workspace: &humata2_clients::Workspace) -> Vec<String> {
    workspace
        .diffs
        .iter()
        .filter(|d| d.session == SessionId::new("ledger"))
        .map(|d| d.repo.clone())
        .collect()
}

/// The `x` of the left and right edges of an element, in points.
fn span_of(window: &gpui_kit::Window, id: &str) -> (f32, f32) {
    let bounds = window.find(gpui_kit::SharedString::from(id.to_owned())).bounds();
    (
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.x + bounds.size.width),
    )
}

/// Two long paths that differ only in the file name both keep the name
/// inside their row: the directory gives way first.
#[test]
fn two_long_paths_that_differ_in_the_file_name_both_show_it() {
    let mut workspace = fixture::design();
    let directory = format!("{}/", "deep_directory_name/".repeat(10).trim_end_matches('/'));
    let repo = workspace.diffs.iter_mut().find(|d| d.repo == "api").unwrap();
    let template: FileDiff = repo.files[0].clone();
    for name in ["alpha.rs", "omega.rs"] {
        repo.files.push(FileDiff {
            path: format!("{directory}{name}"),
            ..template.clone()
        });
    }
    let paths: Vec<String> = ["alpha.rs", "omega.rs"]
        .iter()
        .map(|name| format!("api/{directory}{name}"))
        .collect();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            for path in &paths {
                let (name_left, name_right) = span_of(window, &format!("change-file-name {path}"));
                let (row_left, row_right) = span_of(window, &format!("change-file {path}"));
                assert!(name_right - name_left > 0., "{path}: the name has width");
                assert!(name_left >= row_left - 0.5, "{path}: the name starts inside its row");
                assert!(name_right <= row_right + 0.5, "{path}: the name ends inside its row");
            }
        })
        .unwrap();
}

/// A repo name and a branch far too long for the row stay inside the panel.
#[test]
fn a_long_repo_and_branch_stay_inside_the_repo_row() {
    let mut workspace = fixture::design();
    let long_repo = "r".repeat(120);
    for diff in workspace.diffs.iter_mut().filter(|d| d.repo == "api") {
        diff.repo = long_repo.clone();
    }
    let ledger = SessionId::new("ledger");
    let session = workspace.sessions.iter_mut().find(|s| s.id == ledger).unwrap();
    session.clones = vec![humata2_clients::model::SessionClone {
        repo: long_repo.clone(),
        branch: "b".repeat(120),
        path: "/work/long".into(),
    }];
    let index = repos_of(&workspace).iter().position(|r| *r == long_repo).unwrap();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let right = f32::from(panel.origin.x + panel.size.width);
            let (toggle_left, toggle_right) = span_of(window, &format!("repo-toggle-{index}"));
            let (_, branch_right) = span_of(window, &format!("repo-branch-{index}"));
            assert!(toggle_left >= f32::from(panel.origin.x) - 0.5);
            assert!(toggle_right <= right + 0.5);
            assert!(branch_right <= right + 0.5, "the branch ends inside the panel");
        })
        .unwrap();
}

/// A repo whose clone is on another branch shows it; one on the session's
/// branch shows nothing.
#[test]
fn each_repo_shows_its_own_clones_branch() {
    let mut workspace = fixture::design();
    let ledger = SessionId::new("ledger");
    let repos = repos_of(&workspace);
    let session = workspace.sessions.iter_mut().find(|s| s.id == ledger).unwrap();
    let on_session = session.branch.clone();
    session.clones = vec![
        humata2_clients::model::SessionClone {
            repo: repos[0].clone(),
            branch: format!("{on_session}-other"),
            path: "/work/a".into(),
        },
        humata2_clients::model::SessionClone {
            repo: repos[1].clone(),
            branch: on_session,
            path: "/work/b".into(),
        },
    ];
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    assert!(app.contains("repo-branch-0"));
    assert!(!app.contains("repo-branch-1"));
}

/// The chevron folds a repo's files and opens them again; the other repo is
/// left as it was.
#[test]
fn a_collapsed_repo_hides_its_files_and_opens_again() {
    let workspace = fixture::design();
    let repos = repos_of(&workspace);
    let files_of = |repo: &str| -> Vec<String> {
        workspace
            .diffs
            .iter()
            .find(|d| d.session == SessionId::new("ledger") && d.repo == repo)
            .unwrap()
            .files
            .iter()
            .map(|f| format!("change-file {repo}/{}", f.path))
            .collect()
    };
    let (first, second) = (files_of(&repos[0]), files_of(&repos[1]));
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    assert!(app.contains(&first[0]));
    app.click("repo-toggle-0");
    for id in &first {
        assert!(!app.contains(id), "{id} is folded away");
    }
    for id in &second {
        assert!(app.contains(id), "{id} stays");
    }
    app.click("repo-toggle-0");
    for id in first.iter().chain(&second) {
        assert!(app.contains(id), "{id} is back");
    }
}

/// A local repository reads "large", not "/large".
#[test]
fn a_local_repo_reads_without_a_leading_slash() {
    let mut workspace = fixture::design();
    for diff in workspace.diffs.iter_mut().filter(|d| d.repo == "api") {
        diff.repo = "/large".into();
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.draw();
    let tree = app
        .cx
        .update_window(app.window, |_, window, _| view::capture(window))
        .expect("window")
        .expect("view");
    assert!(tree.contains("text \"large\""));
    assert!(!tree.contains("text \"/large\""));
}

/// Push & PR… opens one menu with a push per repo; its scrim closes it.
#[test]
fn push_and_pr_opens_one_menu_listing_every_repo() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    assert!(app.contains("changes-push-menu"));
    assert!(!app.contains("changes-push-api"));
    app.click("changes-push-menu");
    assert!(app.contains("changes-push-api"));
    assert!(app.contains("changes-push-ledger"));
    app.click("changes-push-scrim");
    assert!(!app.contains("changes-push-api"));
}

fn diff_line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

/// The "ledger" session with `file` as the only file of its `api`
/// repository, picked and shown in the Diff panel.
fn app_showing(file: FileDiff) -> App {
    let mut workspace = fixture::design();
    let path = file.path.clone();
    for diff in workspace.diffs.iter_mut().filter(|d| d.repo == "api") {
        diff.files = vec![file.clone()];
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path,
    }));
    app
}

/// A binary file has no lines to draw: the body says so, once (the note
/// line is left out so the words aren't doubled).
#[test]
fn a_binary_file_says_binary_instead_of_an_empty_diff() {
    let mut app = app_showing(FileDiff {
        path: "assets/logo.png".into(),
        binary: true,
        ..FileDiff::default()
    });
    assert!(!app.contains("change-file-note"));
    assert!(app.contains("change-file-empty"));
    assert!(!app.contains("change-line-0"));
}

/// A rename names where the file came from, and its row carries the mark.
#[test]
fn a_rename_names_where_it_came_from() {
    let mut app = app_showing(FileDiff {
        path: "src/http.rs".into(),
        change: FileChange::Renamed,
        old_path: Some("src/net.rs".into()),
        similarity: 92,
        added: 1,
        lines: vec![
            diff_line(LineKind::Hunk, None, None, "@@ -1 +1 @@"),
            diff_line(LineKind::Added, None, Some(1), "fn main() {}"),
        ],
        ..FileDiff::default()
    });
    // The row is in the list, which a picked file folds into the file bar.
    app.send(AppAction::Review(R::ToggleFileList));
    assert!(app.contains("change-file-note"));
    assert!(app.contains("change-file-mark api/src/http.rs"));
    assert!(!app.contains("change-file-empty"));
}

/// A change of mode alone has no lines; its body says what the modes were.
#[test]
fn a_mode_only_change_shows_both_modes() {
    let mut app = app_showing(FileDiff {
        path: "scripts/run.sh".into(),
        old_mode: 100_644,
        new_mode: 100_755,
        ..FileDiff::default()
    });
    assert!(!app.contains("change-file-note"));
    assert!(app.contains("change-file-empty"));
}

/// "No newline at end of file" is a row with no number to click.
#[test]
fn a_missing_final_newline_is_a_note_row_without_a_number() {
    let mut app = app_showing(FileDiff {
        path: "src/lib.rs".into(),
        added: 1,
        lines: vec![
            diff_line(LineKind::Hunk, None, None, "@@ -0,0 +1 @@"),
            diff_line(LineKind::Added, None, Some(1), "fn a() {}"),
            diff_line(LineKind::Note, None, None, "\\ No newline at end of file"),
        ],
        ..FileDiff::default()
    });
    assert!(app.contains("change-note-2"));
    assert!(!app.contains("change-gutter-2"));
    assert!(app.contains("change-gutter-1"));
}

/// Picking a base shows Loading… beside the chip until the diff arrives.
#[test]
fn picking_a_base_shows_loading_until_the_diff_arrives() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    assert!(!app.contains("changes-loading"));
    // The read is held in flight until the test lets it answer.
    app.fakes.daemon.hold(humata2_clients::Operation::Diffs);
    app.send(AppAction::Review(R::PickBase(humata2_clients::model::DiffBase::SinceFork)));
    assert!(app.contains("changes-loading"));
    app.fakes.daemon.release(humata2_clients::Operation::Diffs);
    app.settle();
    assert!(!app.contains("changes-loading"));
}

/// A session with nothing changed offers Compare with…, which opens the
/// base menu.
#[test]
fn an_empty_diff_offers_compare_with() {
    let mut workspace = fixture::design();
    workspace
        .diffs
        .retain(|d| d.session != SessionId::new("ledger"));
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    assert!(app.contains("diff-compare-with"));
    assert!(!app.contains("diff-base-menu"));
    app.click("diff-compare-with");
    assert!(app.contains("diff-base-menu"));
}

/// A pull request review's footer carries Submit review…, beside the
/// terminal as well as in full screen.
#[test]
fn a_pull_request_review_footer_has_submit_review() {
    let mut app = App::new(super::pr_session::review_workspace(Vec::new()));
    app.open("se_pr_1", Panel::Changes);
    assert!(app.contains("diff-submit-review"));
}

/// A file of `hunks` hunks, each a header and `per_hunk` added lines.
fn hunked(path: &str, hunks: u32, per_hunk: u32) -> FileDiff {
    let mut lines = Vec::new();
    let mut number = 1;
    for hunk in 0..hunks {
        lines.push(diff_line(LineKind::Hunk, None, None, &format!("@@ hunk {hunk} @@")));
        for _ in 0..per_hunk {
            lines.push(diff_line(LineKind::Added, None, Some(number), "let x = 1;"));
            number += 1;
        }
    }
    FileDiff {
        path: path.into(),
        added: hunks * per_hunk,
        lines,
        ..FileDiff::default()
    }
}

/// The file the "ledger" session has open, as the review page holds it.
fn open_file(app: &App) -> Option<(String, String)> {
    app.ui
        .read(|state| state.review.saved.selected.get(&SessionId::new("ledger")).cloned())
}

/// With a file picked, the narrow list folds into one file bar.
#[test]
fn with_a_file_open_the_list_folds_into_the_file_bar() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    assert!(app.contains("diff-file-bar"));
    assert!(app.contains("diff-file-count"));
    assert!(app.contains("diff-prev-file"));
    assert!(app.contains("diff-next-file"));
    assert!(app.contains("change-open-editor"));
    assert!(!app.contains("changes-files"));
}

/// Clicking the bar opens the list under it; picking a file folds it again.
#[test]
fn clicking_the_file_bar_opens_the_list_and_picking_a_file_folds_it() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.click("diff-file-bar");
    assert!(app.contains("changes-files"));
    assert!(app.contains("diff-file-bar"));
    app.click("change-file api/src/client.rs");
    assert!(!app.contains("changes-files"));
    assert!(app.contains("diff-file-bar"));
}

/// Next file steps through every file of a repo, then into the next repo.
#[test]
fn next_file_steps_across_repos() {
    let workspace = fixture::design();
    let ledger = SessionId::new("ledger");
    let diffs: Vec<_> = workspace.diffs.iter().filter(|d| d.session == ledger).collect();
    assert!(diffs.len() >= 2, "the design session has two repositories");
    let first_repo = diffs[0].repo.clone();
    let first_file = diffs[0].files[0].path.clone();
    let steps = diffs[0].files.len();
    let second_repo = diffs[1].repo.clone();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: first_repo.clone(),
        path: first_file,
    }));
    for _ in 0..steps {
        app.click("diff-next-file");
    }
    let (repo, _) = open_file(&app).expect("a file is open");
    assert_eq!(repo, second_repo);
    // A step does not open the list.
    assert!(!app.contains("changes-files"));
}

/// The step buttons and the count stay inside a narrow panel.
#[test]
fn the_file_bar_controls_stay_inside_a_narrow_panel() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let right = panel.origin.x + panel.size.width;
            for id in ["diff-next-file", "diff-prev-file", "change-open-editor"] {
                let bounds = window.find(id).bounds();
                assert!(bounds.origin.x >= panel.origin.x - px(0.5), "{id} starts inside");
                assert!(bounds.origin.x + bounds.size.width <= right + px(0.5), "{id} ends inside");
            }
        })
        .unwrap();
}

/// A long file with several hunks offers its change stepper; a short one
/// does not.
#[test]
fn a_long_file_shows_its_change_stepper() {
    let mut long = app_showing(hunked("src/big.rs", 3, 40));
    assert!(long.contains("diff-change-count"));
    assert!(long.contains("diff-prev-change"));
    assert!(long.contains("diff-next-change"));
    long.click("diff-next-change");
    assert!(long.contains("diff-change-count"));

    let mut short = app_showing(hunked("src/small.rs", 3, 2));
    assert!(!short.contains("diff-change-count"));
}

/// The repo row keeps its totals inside it, on one line, at any width.
#[test]
fn repo_totals_stay_inside_their_row_at_any_width() {
    let workspace = fixture::design();
    let repos = repos_of(&workspace);
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    // A branch of its own makes the per-repo base chip show.
    app.send(AppAction::Review(R::PickRepoBase {
        session: SessionId::new("ledger"),
        repo: repos[0].clone(),
        base: humata2_clients::model::RepoBase::Branch("release/2.4".into()),
    }));
    for width in [300., 220.] {
        app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, width)));
        app.draw();
        app.cx
            .update_window(app.window, |_, window, _| {
                for index in 0..repos.len() {
                    let row = window.find(gpui_kit::SharedString::from(format!("repo-row-{index}"))).bounds();
                    let totals = window.find(gpui_kit::SharedString::from(format!("repo-totals-{index}"))).bounds();
                    assert!(
                        totals.origin.x + totals.size.width <= row.origin.x + row.size.width + px(0.5),
                        "repo {index} at {width}: the totals end inside the row"
                    );
                    assert!(f32::from(row.size.height) <= 27., "repo {index} at {width}: one line");
                }
            })
            .unwrap();
    }
}

/// A file with a hunk that has a section, then a context, a removed and an
/// added line (indexes 1, 2 and 3).
fn mixed_file() -> FileDiff {
    FileDiff {
        path: "src/mixed.rs".into(),
        added: 1,
        removed: 1,
        lines: vec![
            diff_line(LineKind::Hunk, None, None, "@@ -40,7 +40,12 @@ impl Client {"),
            diff_line(LineKind::Context, Some(40), Some(40), "    let a = 1;"),
            diff_line(LineKind::Removed, Some(41), None, "    let b = 2;"),
            diff_line(LineKind::Added, None, Some(41), "    let b = 3;"),
        ],
        ..FileDiff::default()
    }
}

/// Full screen draws an old and a new number column on every code line;
/// beside the terminal there is one.
#[test]
fn full_screen_has_two_number_columns() {
    let mut app = app_showing(mixed_file());
    // Beside the terminal: one column, the clickable one.
    assert!(app.contains("change-gutter-1"));
    assert!(app.contains("change-gutter-2"));
    assert!(app.contains("change-gutter-3"));
    for id in ["change-gutter-old-1", "change-gutter-old-3", "change-gutter-new-2"] {
        assert!(!app.contains(id), "{id} is not drawn beside the terminal");
    }
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    // A context and an added line: the new number is the target, the old
    // column is the other one.
    for id in ["change-gutter-old-1", "change-gutter-1", "change-gutter-old-3", "change-gutter-3"] {
        assert!(app.contains(id), "{id} is drawn in full screen");
    }
    // A removed line: the old number is the target.
    assert!(app.contains("change-gutter-2"));
    assert!(app.contains("change-gutter-new-2"));
}

/// An added and a removed line start their code at the same edge, beside
/// the terminal and in full screen.
#[test]
fn added_and_removed_code_starts_at_one_edge() {
    let mut app = app_showing(mixed_file());
    for full in [false, true] {
        if full {
            app.send(AppAction::Session(SessionAction::ToggleWiden));
        }
        app.draw();
        app.cx
            .update_window(app.window, |_, window, _| {
                let context = window.find("change-code-1-0").bounds();
                let removed = window.find("change-code-2-0").bounds();
                let added = window.find("change-code-3-0").bounds();
                assert!(
                    (f32::from(removed.origin.x - added.origin.x)).abs() < 0.5,
                    "full {full}: removed and added start together"
                );
                assert!(
                    (f32::from(context.origin.x - added.origin.x)).abs() < 0.5,
                    "full {full}: context starts there too"
                );
            })
            .unwrap();
    }
}

/// A hunk is a 24 pt band with only its section text, or a thin line when
/// it has none; the `@@` numbers are never drawn.
#[test]
fn a_hunk_header_draws_no_at_numbers() {
    let mut app = app_showing(mixed_file());
    app.draw();
    let tree = app
        .cx
        .update_window(app.window, |_, window, _| view::capture(window))
        .expect("window")
        .expect("view");
    assert!(!tree.contains("@@"), "no hunk numbers are drawn");
    app.cx
        .update_window(app.window, |_, window, _| {
            let band = window.find("change-hunk-0").bounds();
            assert!((f32::from(band.size.height) - 24.).abs() < 0.5);
        })
        .unwrap();

    let mut plain = app_showing(hunked("src/plain.rs", 1, 2));
    plain.draw();
    plain
        .cx
        .update_window(plain.window, |_, window, _| {
            let band = window.find("change-hunk-0").bounds();
            assert!(f32::from(band.size.height) <= 2., "a hunk without a section is a thin line");
        })
        .unwrap();
}

/// The file header's Wrap / Scroll switch flips the app-wide preference
/// Settings › Long lines sets, and back.
#[test]
fn the_wrap_switch_flips_long_lines() {
    let mut app = app_showing(hunked("src/wrap.rs", 1, 2));
    let wrapping = |app: &App| {
        app.ui
            .read(|state| (state.settings.preferences.wrap_diffs, state.review.wrap))
    };
    let (first, shown) = wrapping(&app);
    assert_eq!(first, shown, "the diff follows the preference");
    app.click("diff-wrap");
    assert_eq!(wrapping(&app), (!first, !first));
    app.click("diff-wrap");
    assert_eq!(wrapping(&app), (first, first));
}

//! The owner's diff stress cases, each a frame (DV-13): a huge file, a
//! minified line, many files, long names, odd bytes and empty or mode-only
//! changes. Each case checks that the panel, its header buttons and its code
//! stay inside the panel, then records its frame.
use super::*;
use gpui_kit::{Bounds, Pixels};
use humata2_clients::model::{DiffLine, FileChange, FileDiff, LineKind, RepoDiff, SessionClone};
use humata2_review::ReviewAction as R;

const REVIEW: &str = "se_pr_1";

fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

fn hunk(text: &str) -> DiffLine {
    line(LineKind::Hunk, None, None, text)
}

/// A file with `lines`, its counts taken from them.
fn file(path: &str, lines: Vec<DiffLine>) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        added: lines.iter().filter(|entry| entry.kind == LineKind::Added).count() as u32,
        removed: lines.iter().filter(|entry| entry.kind == LineKind::Removed).count() as u32,
        lines,
        ..FileDiff::default()
    }
}

/// A file with one hunk and one added line.
fn small_file(path: &str) -> FileDiff {
    file(
        path,
        vec![
            hunk("@@ -0,0 +1 @@"),
            line(LineKind::Added, None, Some(1), "fn main() {}"),
        ],
    )
}

/// `count` small files named `src/<repo_dir>_NNN.rs`.
fn numbered_files(repo_dir: &str, count: u32) -> Vec<FileDiff> {
    (0..count)
        .map(|n| small_file(&format!("src/{repo_dir}_{n:03}.rs")))
        .collect()
}

/// 40,000 lines in 800 hunks of 50: a header and 49 added lines each.
fn huge_file(path: &str) -> FileDiff {
    let lines: Vec<DiffLine> = (0..800u32)
        .flat_map(|index| {
            let first = index * 49 + 1;
            std::iter::once(hunk(&format!("@@ -{first},0 +{first},49 @@"))).chain(
                (first..first + 49).map(|n| {
                    line(LineKind::Added, None, Some(n), &format!("let value_{n} = {n};"))
                }),
            )
        })
        .collect();
    file(path, lines)
}

/// One added line of 100,000 characters, as a minified bundle has.
fn minified_file(path: &str) -> FileDiff {
    file(
        path,
        vec![
            hunk("@@ -0,0 +1 @@"),
            line(LineKind::Added, None, Some(1), &"x=1;".repeat(25_000)),
        ],
    )
}

/// A 200-byte path: six directories and a file name.
fn long_path() -> String {
    let dirs = "a_directory_name/".repeat(6);
    format!("{dirs}{}.rs", "n".repeat(200 - dirs.len() - ".rs".len()))
}

/// Puts `repos` into `session`'s diff in place of its own repositories,
/// opens the Changes panel and selects `picked` (repo, then path).
fn app_with(
    mut workspace: Workspace,
    session: &str,
    repos: Vec<(&str, Vec<FileDiff>)>,
    picked: Option<(&str, &str)>,
) -> App {
    let id = SessionId::new(session);
    workspace.diffs.retain(|diff| diff.session != id);
    workspace
        .diffs
        .extend(repos.into_iter().map(|(repo, files)| RepoDiff {
            session: id.clone(),
            repo: repo.to_owned(),
            base: "main".to_owned(),
            files,
            comments: Vec::new(),
        }));
    let mut app = App::new(workspace);
    app.open(session, Panel::Changes);
    if let Some((repo, path)) = picked {
        app.send(AppAction::Review(R::SelectFile {
            repo: repo.into(),
            path: path.into(),
        }));
        // Open the folded list so each frame shows the rows under stress too.
        app.send(AppAction::Review(R::ToggleFileList));
    }
    app.draw();
    app
}

/// Sets the Diff panel's Wrap on or off, as its Wrap toggle does.
fn set_wrap(app: &mut App, on: bool) {
    if app.ui.read(|state| state.review.wrap) != on {
        app.send(AppAction::Review(R::ToggleWrap));
    }
}

/// Whether `inner` lies inside `outer`, with half a point of slack.
fn within(inner: &Bounds<Pixels>, outer: &Bounds<Pixels>) -> bool {
    let slack = px(0.5);
    inner.origin.x >= outer.origin.x - slack
        && inner.origin.y >= outer.origin.y - slack
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width + slack
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height + slack
}

/// The panel and its header buttons stay inside the panel, and the code
/// view of the file in view is taller than 100 pt (or the file's empty
/// note, for a file with no lines). With no file picked, only the panel
/// and its buttons are checked.
fn stays_inside(app: &mut App) {
    app.draw();
    // The one session each case picks a file in.
    let picked = app
        .ui
        .read(|state| state.review.saved.selected.values().next().cloned());
    let full = app
        .ui
        .read(|state| state.full_screen() == Some(FullScreen::Panel));
    let frame = if full { "panel-full-screen" } else { "side-panel" };
    let buttons: Vec<&str> = if full {
        vec!["show-terminal"]
    } else {
        vec!["expand-panel", "close-panel"]
    };
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find(frame).bounds();
            for id in buttons {
                let button = window.find(id).bounds();
                assert!(within(&button, &panel), "{id} lies inside the panel");
            }
            let Some((repo, path)) = &picked else {
                return;
            };
            let code = format!("change-code {repo}/{path}");
            let body = if window
                .try_find(gpui_kit::SharedString::from(code.clone()))
                .is_some()
            {
                code
            } else {
                "change-file-empty".to_owned()
            };
            let height = window
                .try_find(gpui_kit::SharedString::from(body.clone()))
                .expect("the code view or the empty body is drawn")
                .bounds()
                .size
                .height;
            assert!(f32::from(height) > 100., "{body} is taller than 100 pt");
        })
        .unwrap();
}

/// Owner case huge-file: one file of 40,000 lines in hunks of 50.
#[test]
fn a_huge_file_stays_inside_the_panel() {
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![huge_file("src/client.rs")])],
        Some(("api", "src/client.rs")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-huge-file");
}

/// Owner case minified-line: one 100,000-character line, with Wrap on.
#[test]
fn a_minified_line_wrapped_stays_inside_the_panel() {
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("web", vec![minified_file("dist/app.min.js")])],
        Some(("web", "dist/app.min.js")),
    );
    set_wrap(&mut app, true);
    stays_inside(&mut app);
    app.snapshot("diff-stress-minified-line-wrap-on");
}

/// Owner case minified-line: one 100,000-character line, with Wrap off.
#[test]
fn a_minified_line_unwrapped_stays_inside_the_panel() {
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("web", vec![minified_file("dist/app.min.js")])],
        Some(("web", "dist/app.min.js")),
    );
    set_wrap(&mut app, false);
    stays_inside(&mut app);
    app.snapshot("diff-stress-minified-line-wrap-off");
}

/// Owner case many-files: 500 files over three repositories, with the list
/// scrolled so the last repository's last file is in view.
#[test]
fn many_files_scroll_to_the_last_repositorys_last_file() {
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![
            ("api", numbered_files("api", 200)),
            ("web", numbered_files("web", 200)),
            ("docs", numbered_files("docs", 100)),
        ],
        Some(("docs", "src/docs_099.rs")),
    );
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll(
                "changes-files",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-10_000_000.))),
                cx,
            );
        })
        .unwrap();
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let list = window.find("changes-files").bounds();
            let row = window.find("change-file docs/src/docs_099.rs").bounds();
            assert!(within(&row, &list), "the last file's row is in the list's view");
        })
        .unwrap();
    stays_inside(&mut app);
    app.snapshot("diff-stress-many-files");
}

/// Owner case long-paths: a 200-byte path stays inside the panel.
#[test]
fn a_200_byte_path_stays_inside_the_panel() {
    let path = long_path();
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![small_file(&path)])],
        Some(("api", path.as_str())),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-long-paths");
}

/// Owner case long-repo-and-branch: a 120-character repository and branch
/// stay inside the panel.
#[test]
fn a_long_repo_and_branch_stay_inside_the_panel() {
    let repo = "r".repeat(120);
    let mut workspace = fixture::design();
    let ledger = SessionId::new("ledger");
    let session = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == ledger)
        .unwrap();
    session.clones = vec![SessionClone {
        repo: repo.clone(),
        branch: "b".repeat(120),
        path: "/work/long".to_owned(),
    }];
    let mut app = app_with(
        workspace,
        "ledger",
        vec![(repo.as_str(), vec![small_file("src/client.rs")])],
        Some((repo.as_str(), "src/client.rs")),
    );
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            assert!(within(&window.find("repo-toggle-0").bounds(), &panel));
            assert!(within(&window.find("repo-branch-0").bounds(), &panel));
        })
        .unwrap();
    stays_inside(&mut app);
    app.snapshot("diff-stress-long-repo-and-branch");
}

/// Owner case binary: a binary file has no lines and its note stays inside.
#[test]
fn a_binary_file_stays_inside_the_panel() {
    let binary = FileDiff {
        path: "assets/logo.png".to_owned(),
        binary: true,
        ..FileDiff::default()
    };
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![binary])],
        Some(("api", "assets/logo.png")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-binary");
}

/// Owner case unicode-names: accented, Japanese and emoji paths stay inside.
#[test]
fn unicode_paths_stay_inside_the_panel() {
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![(
            "api",
            vec![
                small_file("src/café.rs"),
                small_file("src/日本語/模块.rs"),
                small_file("docs/🦀 notes.md"),
            ],
        )],
        Some(("api", "src/日本語/模块.rs")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-unicode-names");
}

/// Owner case crlf: lines that end in a carriage return stay inside.
#[test]
fn crlf_lines_stay_inside_the_panel() {
    let crlf = file(
        "src/crlf.rs",
        vec![
            hunk("@@ -1 +1 @@"),
            line(LineKind::Removed, Some(1), None, "let crlf = false;\r"),
            line(LineKind::Added, None, Some(1), "let crlf = true;\r"),
        ],
    );
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![crlf])],
        Some(("api", "src/crlf.rs")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-crlf");
}

/// Owner case no-trailing-newline: the last added lines are followed by the
/// "no newline" note, which stays inside.
#[test]
fn a_missing_final_newline_stays_inside_the_panel() {
    let lib = file(
        "src/lib.rs",
        vec![
            hunk("@@ -0,0 +1,2 @@"),
            line(LineKind::Added, None, Some(1), "fn a() {}"),
            line(LineKind::Added, None, Some(2), "fn b() {}"),
            line(LineKind::Note, None, None, "\\ No newline at end of file"),
        ],
    );
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![lib])],
        Some(("api", "src/lib.rs")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-no-trailing-newline");
}

/// Owner case long-rename: a rename from a 200-byte path stays inside.
#[test]
fn a_long_rename_stays_inside_the_panel() {
    let renamed = FileDiff {
        change: FileChange::Renamed,
        old_path: Some(long_path()),
        similarity: 87,
        ..file(
            "src/renamed.rs",
            vec![
                hunk("@@ -1 +1 @@"),
                line(LineKind::Added, None, Some(1), "fn renamed() {}"),
            ],
        )
    };
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![renamed])],
        Some(("api", "src/renamed.rs")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-long-rename");
}

/// Owner case mode-only: a mode change with no lines stays inside.
#[test]
fn a_mode_only_change_stays_inside_the_panel() {
    let script = FileDiff {
        path: "scripts/run.sh".to_owned(),
        old_mode: 100_644,
        new_mode: 100_755,
        ..FileDiff::default()
    };
    let mut app = app_with(
        fixture::design(),
        "ledger",
        vec![("api", vec![script])],
        Some(("api", "scripts/run.sh")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-mode-only");
}

/// Owner case empty (DV-13): a session with no changed files stays inside.
#[test]
fn an_empty_session_stays_inside_the_panel() {
    let mut app = app_with(fixture::design(), "ledger", Vec::new(), None);
    stays_inside(&mut app);
    app.snapshot("diff-stress-empty");
}

/// Owner case huge-file in a pull request review: 40,000 lines stay inside.
#[test]
fn a_pull_request_huge_file_stays_inside_the_panel() {
    let mut app = app_with(
        super::pr_session::review_workspace(Vec::new()),
        REVIEW,
        vec![("acme/api", vec![huge_file("src/refund.rs")])],
        Some(("acme/api", "src/refund.rs")),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-pr-huge-file");
}

/// Owner case long-paths in a pull request review: a 200-byte path stays
/// inside.
#[test]
fn a_pull_request_long_path_stays_inside_the_panel() {
    let path = long_path();
    let mut app = app_with(
        super::pr_session::review_workspace(Vec::new()),
        REVIEW,
        vec![("acme/api", vec![small_file(&path)])],
        Some(("acme/api", path.as_str())),
    );
    stays_inside(&mut app);
    app.snapshot("diff-stress-pr-long-paths");
}

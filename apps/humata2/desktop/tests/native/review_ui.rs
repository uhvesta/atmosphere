//! The Diff panel's code scroll is kept per session and file (DS-73), and
//! opening, closing and widening the panel reads the diff only once.
use super::*;
use humata2_clients::comments::{Anchor, Side};
use humata2_clients::model::{DiffLine, FileDiff, LineKind};
use humata2_review::ReviewAction as R;
use humata2_review::comments::CommentsAction as C;

const FILE: &str = "src/client.rs";
const VIEW: &str = "change-code api/src/client.rs";
/// The file's lines: the hunk header at line 0, then 399 added lines.
const LINES: u32 = 400;

/// The "ledger" and "partial-brin" sessions, which share one working tree,
/// both show `api/src/client.rs` as a 400-line file.
fn long_file_app() -> App {
    let mut workspace = fixture::design();
    for diff in workspace.diffs.iter_mut().filter(|d| {
        d.repo == "api"
            && (d.session == SessionId::new("ledger") || d.session == SessionId::new("partial-brin"))
    }) {
        let file = diff
            .files
            .iter_mut()
            .find(|f| f.path == FILE)
            .expect("the fixture has src/client.rs");
        let mut lines = vec![DiffLine {
            kind: LineKind::Hunk,
            old: None,
            new: None,
            text: format!("@@ -1,{LINES} +1,{LINES} @@"),
        }];
        lines.extend((1..LINES).map(|n| DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(n),
            text: format!("let line_{n} = {n};"),
        }));
        *file = FileDiff {
            added: LINES - 1,
            removed: 0,
            lines,
            ..file.clone()
        };
    }
    App::new(workspace)
}

/// Opens the Diff panel of `session` with the long file picked.
fn open_long_file(app: &mut App, session: &str) {
    app.open(session, Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: FILE.into(),
    }));
}

/// The first line of the code view that is drawn, and where it starts
/// relative to the view's top: how far the code is scrolled.
fn top_line(app: &mut App) -> (u32, f32) {
    // A second frame lets the list settle the heights it measured on the first.
    app.draw();
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let view = window.find(VIEW).bounds();
            (0..LINES)
                .find_map(|n| {
                    window
                        .try_find(gpui_kit::SharedString::from(format!("change-line-{n}")))
                        .map(|line| {
                            let top = f32::from(line.bounds().origin.y - view.origin.y);
                            (n, top)
                        })
                })
                .expect("a line of the code view is drawn")
        })
        .expect("window stays open")
}

/// Scrolls the code view down by `pixels`.
fn scroll_code(app: &mut App, pixels: f32) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll(
                VIEW,
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-pixels))),
                cx,
            );
        })
        .expect("window stays open");
    app.draw();
    app.draw();
}

/// Opens the comment box on `line` of the picked file and types `body` into
/// it, leaving the box open.
fn open_box(app: &mut App, session: &str, line: u32, body: &str) {
    let id = SessionId::new(session);
    let anchor = Anchor {
        repo: "api".into(),
        path: FILE.into(),
        side: Side::New,
        first: line,
        last: line,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    };
    app.send(AppAction::Comments(C::OpenBox(id.clone(), anchor)));
    app.send(AppAction::Comments(C::Typed(id, body.into())));
}

/// How many diffs the daemon has been asked for in "ledger".
fn diff_reads(app: &App) -> usize {
    app.fakes
        .daemon
        .calls()
        .iter()
        .filter(|call| call.starts_with("diffs ledger "))
        .count()
}

/// Two sessions on one file keep their own scroll: each session's code view
/// returns to the place that session left it, not where the other one did.
#[test]
fn two_sessions_on_one_file_keep_their_own_scroll() {
    let mut app = long_file_app();
    open_long_file(&mut app, "ledger");
    let start = top_line(&mut app);
    scroll_code(&mut app, 600.);
    let a = top_line(&mut app);
    assert!(a.1 < start.1 - 100., "session A's code scrolled down");

    open_long_file(&mut app, "partial-brin");
    scroll_code(&mut app, 3000.);
    let b = top_line(&mut app);
    assert!(
        (b.0, b.1.round()) != (a.0, a.1.round()),
        "session B scrolled somewhere else"
    );

    open_long_file(&mut app, "ledger");
    let back = top_line(&mut app);
    assert_eq!(back.0, a.0, "session A shows its own place, not session B's");
    assert!(
        (back.1 - a.1).abs() < 0.5,
        "session A keeps its scroll to the pixel: {back:?} vs {a:?}"
    );

    open_long_file(&mut app, "partial-brin");
    let again = top_line(&mut app);
    assert_eq!(again.0, b.0, "session B shows its own place");
    assert!((again.1 - b.1).abs() < 0.5, "session B keeps its scroll to the pixel");
}

/// Show the terminal and back keeps the file, its scroll, the draft and the
/// open comment box.
#[test]
fn show_the_terminal_keeps_the_line_scroll_drafts_and_open_box() {
    let mut app = long_file_app();
    open_long_file(&mut app, "ledger");
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.write_comment("ledger", "api", 20, "Keep this write in the transaction.");
    scroll_code(&mut app, 600.);
    open_box(&mut app, "ledger", 45, "Second thought on this.");
    assert!(app.contains("doc-comment-box"));
    let scrolled = top_line(&mut app);
    assert!(scrolled.0 > 0, "the code scrolled down");
    let drafts = app
        .ui
        .read(|state| state.comments.sendable(&SessionId::new("ledger")).len());
    assert!(drafts > 0, "the draft is written");

    app.click("show-terminal");
    assert!(app.contains("expand-panel"));
    assert!(app.contains(VIEW), "the same file is shown");
    let back = top_line(&mut app);
    assert_eq!(back.0, scrolled.0, "the scroll line is kept");
    assert!(
        (back.1 - scrolled.1).abs() < 0.5,
        "the scroll offset is kept"
    );
    let after = app
        .ui
        .read(|state| state.comments.sendable(&SessionId::new("ledger")).len());
    assert_eq!(after, drafts, "the draft is still there");
    assert!(app.contains("doc-comment-box"), "the comment box is still open");
    let text = app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&SessionId::new("ledger"))
            .and_then(|held| held.editing.as_ref())
            .map(|box_| box_.text.clone())
    });
    assert_eq!(text.as_deref(), Some("Second thought on this."));
}

/// Closing and reopening the Diff panel reads the diff from the daemon no
/// more than opening it did.
#[test]
fn closing_and_reopening_the_panel_sends_no_fetch() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    let reads = diff_reads(&app);
    assert!(reads > 0, "opening the panel reads the diff");

    app.send(AppAction::Session(SessionAction::SetPanel(None)));
    app.send(AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))));
    assert_eq!(diff_reads(&app), reads, "closing and reopening read the diff again");
}

/// Full screen and back reads the diff from the daemon no more than opening
/// the panel did.
#[test]
fn full_screen_and_back_sends_no_fetch() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    let reads = diff_reads(&app);
    assert!(reads > 0, "opening the panel reads the diff");

    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert_eq!(diff_reads(&app), reads, "full screen and back read the diff again");
}


/// The comment box starts at the code column, not under the line numbers
/// (DS-15).
#[test]
fn the_comment_box_starts_at_the_code_column() {
    let mut app = long_file_app();
    open_long_file(&mut app, "ledger");
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    open_box(&mut app, "ledger", 10, "A thought on this line.");
    app.draw();
    app.draw();
    let (code, card) = app
        .cx
        .update_window(app.window, |_, window, _| {
            let code = f32::from(window.find("change-code-10-0").bounds().origin.x);
            let card = f32::from(window.find("doc-comment-box").bounds().origin.x);
            (code, card)
        })
        .expect("window stays open");
    assert!(
        (card - code).abs() < 1.,
        "the box starts at the code column: box at {card}, code at {code}"
    );
}

fn press(app: &mut App, key: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.press(key, cx))
        .expect("window stays open");
    app.settle();
}

fn wrap(app: &App) -> bool {
    app.ui.read(|state| state.review.wrap)
}

fn box_text(app: &App) -> Option<String> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&SessionId::new("ledger"))
            .and_then(|held| held.editing.as_ref())
            .map(|box_| box_.text.clone())
    })
}

/// ⌥Z flips Wrap only from the diff: typed in the comment box it is text,
/// and Wrap stays as it was (RV-5, DS-40).
#[test]
fn option_z_in_the_comment_box_types_and_does_not_flip_wrap() {
    let mut app = long_file_app();
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("window stays open");
    open_long_file(&mut app, "ledger");
    open_box(&mut app, "ledger", 10, "Note");
    app.draw();
    press(&mut app, "q");
    assert!(
        box_text(&app).is_some_and(|text| text.contains('q')),
        "the box has the keys"
    );
    let before = wrap(&app);
    press(&mut app, "alt-z");
    assert_eq!(wrap(&app), before, "⌥Z in the box leaves Wrap alone");

    app.click("change-code-10-0");
    press(&mut app, "alt-z");
    assert_ne!(wrap(&app), before, "⌥Z in the code flips Wrap");
}

/// After Show the terminal, the keys go back to the terminal that had them
/// before full screen (DS-83).
#[test]
fn show_the_terminal_returns_focus_to_the_terminal() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("window stays open");
    app.settle();
    app.open("ledger", Panel::Changes);
    app.click("terminal");
    press(&mut app, "ctrl-cmd-enter");
    assert!(app.contains("show-terminal"), "the diff is full screen");
    app.click("show-terminal");
    assert!(app.contains("expand-panel"), "the terminal shows again");
    press(&mut app, "x");
    let typed = app
        .fakes
        .daemon
        .workspace()
        .terminal_bytes
        .iter()
        .any(|((session, _), bytes)| session == &SessionId::new("ledger") && bytes.ends_with(b"x"));
    assert!(typed, "the key reached the terminal");
}

/// A repository's base menu opens inside a narrow panel: its right edge
/// lines up with the chip, never past the panel (RV-7, DS-50).
#[test]
fn the_base_menu_stays_inside_a_narrow_panel() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.click("repo-base-1");
    assert!(app.contains("repo-base-menu"));
    let (menu, panel) = app
        .cx
        .update_window(app.window, |_, window, _| {
            (window.find("repo-base-menu").bounds(), window.find("side-panel").bounds())
        })
        .expect("window stays open");
    assert!(
        menu.right() <= panel.right() + px(0.5),
        "the menu ends at {:?}, the panel at {:?}",
        menu.right(),
        panel.right()
    );
    let window = app
        .cx
        .update_window(app.window, |_, window, _| window.bounds().size.width)
        .expect("window stays open");
    // Anchored right, a menu wider than the room left of the chip may reach
    // over the terminal, but never past the window.
    assert!(menu.origin.x >= px(0.), "the menu starts inside the window");
    assert!(menu.right() <= window, "the menu ends inside the window");
}

/// Above twenty changed files a "Filter files" field sits over the list; at
/// twenty or fewer it is not drawn (RV-6, DS-9).
#[test]
fn a_filter_shows_above_twenty_files() {
    let with_files = |count: usize| {
        let mut workspace = fixture::design();
        workspace.diffs.retain(|d| d.session != SessionId::new("ledger"));
        let files = (0..count)
            .map(|n| FileDiff {
                path: format!("src/file_{n}.rs"),
                added: 1,
                ..FileDiff::default()
            })
            .collect();
        workspace.diffs.push(humata2_clients::model::RepoDiff {
            session: SessionId::new("ledger"),
            repo: "api".to_owned(),
            base: "origin/main".to_owned(),
            files,
            comments: Vec::new(),
        });
        let mut app = App::new(workspace);
        app.open("ledger", Panel::Changes);
        app
    };
    assert!(with_files(21).contains("changes-file-filter"));
    assert!(!with_files(20).contains("changes-file-filter"));
}

/// A name the Diff panel draws shortened still carries all of it: a repo
/// whose clone sits on a long branch exposes the whole branch as the label of
/// its shortened chip, which is also what its hover shows (DS-80).
#[test]
fn a_shortened_path_shows_in_full_on_hover() {
    const BRANCH: &str = "avesta/a-branch-name-long-enough-to-be-cut-short-in-a-narrow-panel";
    let mut workspace = fixture::design();
    let ledger = SessionId::new("ledger");
    let repo = workspace
        .diffs
        .iter()
        .find(|diff| diff.session == ledger)
        .map(|diff| diff.repo.clone())
        .expect("ledger has a diff");
    let session = workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == ledger)
        .expect("the fixture has ledger");
    session.clones = vec![humata2_clients::model::SessionClone {
        repo,
        branch: BRANCH.into(),
        path: "/work/a".into(),
    }];
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    let label = app
        .cx
        .update_window(app.window, |_, window, _| {
            window.find("repo-branch-0").label().map(str::to_owned)
        })
        .expect("window stays open");
    assert_eq!(label.as_deref(), Some(BRANCH));
}

/// RV-8: the Diff panel's own cost, measured headless, for spec §5 P-1 to P-6.
/// Not part of the normal run (`#[ignore]`) and with no timing asserts,
/// because times vary by machine; each test prints its number beside the
/// proposed budget. The daemon's side of P-5 and P-7 (git, the wire) is
/// measured live in DV-14's rerun. Run on a release build:
///
/// `bazel test -c opt //apps/humata2/desktop:native_test --test_output=streamed
/// --test_arg=--ignored --test_arg=--nocapture --test_arg=review_ui::budgets::
/// --test_arg=--test-threads=1`
mod budgets {
    use super::*;
    use std::time::{Duration, Instant};

    /// Prints one measurement: its spec id, what was measured, the number and
    /// the proposed budget it is read against.
    fn report(id: &str, what: &str, took: Duration, budget: Duration) {
        let verdict = if took <= budget { "within" } else { "OVER" };
        println!(
            "RV-8 {id}: {what}: {:.1} ms (proposed budget {:.0} ms, {verdict})",
            took.as_secs_f64() * 1000.,
            budget.as_secs_f64() * 1000.
        );
    }

    /// This process's resident memory in MB, read with `ps`; `None` when
    /// `ps` can't be run.
    fn rss_mb() -> Option<f64> {
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kb: f64 = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
        Some(kb / 1024.)
    }

    /// The 40,000-line file of P-1: one hunk, 20,000 lines removed and
    /// 20,000 added.
    fn forty_thousand() -> Vec<DiffLine> {
        let mut lines = vec![DiffLine {
            kind: LineKind::Hunk,
            old: None,
            new: None,
            text: "@@ -1,20000 +1,20000 @@".into(),
        }];
        lines.extend((1..=20_000).map(|n| DiffLine {
            kind: LineKind::Removed,
            old: Some(n),
            new: None,
            text: format!("let old_{n} = compute({n}, \"some text to wrap\");"),
        }));
        lines.extend((1..=20_000).map(|n| DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(n),
            text: format!("let new_{n} = compute({n}, \"some other text\");"),
        }));
        lines
    }

    /// The design fixture with `api/src/client.rs` of "ledger" replaced by
    /// `lines`, and the Diff panel open with nothing picked.
    fn app_with(lines: Vec<DiffLine>) -> App {
        let mut workspace = fixture::design();
        for diff in workspace
            .diffs
            .iter_mut()
            .filter(|d| d.repo == "api" && d.session == SessionId::new("ledger"))
        {
            if let Some(file) = diff.files.iter_mut().find(|f| f.path == FILE) {
                *file = FileDiff {
                    added: lines.iter().filter(|l| l.kind == LineKind::Added).count() as u32,
                    removed: lines.iter().filter(|l| l.kind == LineKind::Removed).count() as u32,
                    lines: lines.clone(),
                    ..file.clone()
                };
            }
        }
        let mut app = App::new(workspace);
        app.open("ledger", Panel::Changes);
        app.draw();
        app
    }

    fn select(app: &mut App, path: &str) {
        app.send(AppAction::Review(R::SelectFile {
            repo: "api".into(),
            path: path.into(),
        }));
        app.draw();
    }

    /// Another file of "ledger"'s `api`, to step away to and back from.
    fn other_file(app: &App) -> String {
        app.ui
            .read(|state| {
                state
                    .workspace
                    .diffs
                    .iter()
                    .filter(|d| d.repo == "api" && d.session == SessionId::new("ledger"))
                    .flat_map(|d| d.files.iter())
                    .map(|f| f.path.clone())
                    .find(|path| path != FILE)
            })
            .expect("ledger's api has a second file")
    }

    /// P-1, P-2 and P-4: the first open of the 40,000-line file, reselecting
    /// it, and the memory it holds while open.
    #[test]
    #[ignore = "RV-8 measurement; run by hand on a release build"]
    fn a_forty_thousand_line_file() {
        let mut app = app_with(forty_thousand());
        let other = other_file(&app);
        let before = rss_mb();
        let start = Instant::now();
        select(&mut app, FILE);
        let took = start.elapsed();
        report("P-1", "first open of a 40,000-line file", took, Duration::from_millis(300));
        let after = rss_mb();
        select(&mut app, &other);
        let start = Instant::now();
        select(&mut app, FILE);
        report("P-2", "reselecting that file", start.elapsed(), Duration::from_millis(100));
        match (before, after) {
            (Some(before), Some(after)) => println!(
                "RV-8 P-4: memory with that file open: {:.0} MB above the panel \
                 before it (proposed budget 300 MB)",
                after - before
            ),
            _ => println!("RV-8 P-4: memory not read: /bin/ps could not run here"),
        }
    }

    /// P-3: scrolling that file, timed per frame against a 60 Hz frame
    /// (16.7 ms); the share of the frame spent is the CPU it takes.
    #[test]
    #[ignore = "RV-8 measurement; run by hand on a release build"]
    fn scrolling_a_forty_thousand_line_file() {
        let mut app = app_with(forty_thousand());
        select(&mut app, FILE);
        let frames = 120;
        let mut slowest = Duration::ZERO;
        let start = Instant::now();
        for _ in 0..frames {
            let frame = Instant::now();
            app.cx
                .update_window(app.window, |_, window, cx| {
                    window.scroll(
                        VIEW,
                        gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-120.))),
                        cx,
                    );
                    window.render_frame(cx);
                })
                .expect("window stays open");
            slowest = slowest.max(frame.elapsed());
        }
        let mean = start.elapsed() / frames;
        report("P-3", "mean frame while scrolling", mean, Duration::from_micros(16_667 * 30 / 100));
        report("P-3", "slowest frame while scrolling", slowest, Duration::from_micros(16_667));
    }

    /// P-5, the panel's side: drawing "ledger" once its diff holds 586 files
    /// over 4 repositories.
    #[test]
    #[ignore = "RV-8 measurement; run by hand on a release build"]
    fn five_hundred_eighty_six_files_over_four_repos() {
        let mut workspace = fixture::design();
        let ledger = SessionId::new("ledger");
        let template = workspace
            .diffs
            .iter()
            .find(|d| d.session == ledger)
            .cloned()
            .expect("ledger has a diff");
        let file = template.files.first().cloned().expect("the diff has a file");
        workspace.diffs.retain(|d| d.session != ledger);
        for repo in 0..4 {
            let count = if repo == 0 { 586 - 3 * 146 } else { 146 };
            workspace.diffs.push(humata2_clients::model::RepoDiff {
                repo: format!("repo-{repo}"),
                files: (0..count)
                    .map(|n| FileDiff {
                        path: format!("src/dir_{}/file_{n}.rs", n % 12),
                        ..file.clone()
                    })
                    .collect(),
                ..template.clone()
            });
        }
        let mut app = App::new(workspace);
        app.send(AppAction::Navigate(Route::Session(ledger)));
        let start = Instant::now();
        app.send(AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))));
        app.draw();
        let took = start.elapsed();
        report("P-5", "opening the panel on 586 files (panel only)", took, Duration::from_secs(5));
    }

    /// P-6: drawing a 194,000-character line with Wrap on.
    #[test]
    #[ignore = "RV-8 measurement; run by hand on a release build"]
    fn wrapping_a_194_000_character_line() {
        let mut lines = forty_thousand();
        lines.truncate(1_500);
        lines[1].text = "x".repeat(194_000);
        let mut app = app_with(lines);
        if !wrap(&app) {
            app.send(AppAction::Review(R::ToggleWrap));
        }
        let start = Instant::now();
        select(&mut app, FILE);
        let took = start.elapsed();
        report("P-6", "wrapping a 194,000-character line", took, Duration::from_millis(500));
    }
}

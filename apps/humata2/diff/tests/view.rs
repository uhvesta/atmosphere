//! The embedded diff driven through its public API and real input, on
//! each engine. No window is shown and no daemon is needed.
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Entity, Focusable as _, Point, Subscription, TestAppContext,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use humata2_diff_view::{
    CommentOnSelection, DiffEvent, DiffSpec, DiffView, Include, LineRange, LineTarget, Review,
    Side, fixture, key_bindings,
};

struct Pane {
    view: Entity<DiffView>,
    events: Vec<DiffEvent>,
    _events: Subscription,
}

impl Render for Pane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

fn open(cx: &mut TestAppContext, review: Review) -> (AnyWindowHandle, Entity<Pane>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| cx.bind_keys(key_bindings()));
    let (window, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    show: false,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1280.), px(1000.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let view = cx.new(|cx| DiffView::new(review, cx));
                        window.focus(&view.focus_handle(cx), cx);
                        let events = cx.subscribe(&view, |pane: &mut Pane, _, event, _| {
                            pane.events.push(event.clone())
                        });
                        Pane {
                            view,
                            events: Vec::new(),
                            _events: events,
                        }
                    })
                },
            )
        })
        .expect("headless pane opens");
    cx.run_until_parked();
    draw(cx, window);
    (window, pane)
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("window stays open");
}

fn view(cx: &TestAppContext, pane: &Entity<Pane>) -> Entity<DiffView> {
    pane.read_with(cx, |pane, _| pane.view.clone())
}

/// RV-31: editing a spec asks the embedder for the chosen comparison and
/// keeps that choice while the old review is still being refreshed.
#[gpui_kit::test]
fn editing_a_spec_requests_and_keeps_the_new_comparison(cx: &mut TestAppContext) {
    let (window, pane) = open(cx, fixture::small());
    cx.update_window(window, |_, window, cx| {
        window.click("spec atmosphere", cx);
        window.click("spec atmosphere include 0", cx);
    })
    .expect("spec choices are drawn");
    cx.run_until_parked();
    let expected = DiffSpec {
        include: Include::Commits,
        ..DiffSpec::default()
    };
    pane.read_with(cx, |pane, _| {
        assert!(pane.events.contains(&DiffEvent::SpecChanged {
            repo: "atmosphere".into(),
            spec: expected.clone(),
        }))
    });
    let view = view(cx, &pane);
    view.update(cx, |view, cx| view.set_review(fixture::small(), cx));
    assert_eq!(
        view.read_with(cx, |view, _| view.spec("atmosphere").cloned()),
        Some(expected.clone())
    );
    let mut refreshed = fixture::small();
    refreshed.repos[0].spec = expected.clone();
    view.update(cx, |view, cx| view.set_review(refreshed, cx));
    assert_eq!(
        view.read_with(cx, |view, _| view.spec("atmosphere").cloned()),
        Some(expected)
    );
}

/// RV-4, RV-27: requesting a comment carries the selected side, lines,
/// quoted code, spec and base commit to the embedder.
#[gpui_kit::test]
fn a_comment_request_carries_the_selected_code_and_comparison(cx: &mut TestAppContext) {
    let (window, pane) = open(cx, fixture::small());
    let view = view(cx, &pane);
    let target = LineTarget {
        repo: "atmosphere".into(),
        path: "src/ledger.rs".into(),
        side: Side::New,
        lines: LineRange::single(3),
    };
    view.update(cx, |view, cx| assert!(view.select(&target, cx)));
    draw(cx, window);
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(CommentOnSelection), cx)
    })
    .expect("the focused diff takes comment actions");
    cx.run_until_parked();
    pane.read_with(cx, |pane, _| {
        let anchor = pane
            .events
            .iter()
            .find_map(|event| match event {
                DiffEvent::CommentRequested(anchor) => Some(anchor),
                _ => None,
            })
            .expect("the pane receives a comment request");
        assert_eq!(anchor.target, target);
        assert_eq!(anchor.quoted, "/// A balance in cents.");
        assert_eq!(anchor.base_commit.as_deref(), Some("4f2a9c1"));
        assert_eq!(anchor.spec, DiffSpec::default());
    });
}

/// RV-18: a fold without context requests it and opens when the text arrives.
#[gpui_kit::test]
fn opening_a_fold_requests_missing_context_and_uses_the_response(cx: &mut TestAppContext) {
    let mut review = fixture::small();
    review.repos.truncate(1);
    review.repos[0].files.truncate(1);
    review.repos[0].files[0].new_lines = None;
    let (window, pane) = open(cx, review.clone());
    let mut driver = humata2_flow::GpuiDriver::new(cx, window);
    use humata2_flow::Driver as _;
    driver
        .perform(&humata2_flow::Action::Click(humata2_flow::Target::new(
            gpui_kit::Role::Button,
            "Show 5 unchanged lines, atmosphere/src/ledger.rs 7–11",
        )))
        .expect("a fold can be opened");
    pane.read_with(cx, |pane, _| {
        assert!(pane.events.contains(&DiffEvent::ContextNeeded {
            repo: "atmosphere".into(),
            path: "src/ledger.rs".into(),
        }))
    });
    let view = view(cx, &pane);
    let target = LineTarget {
        repo: "atmosphere".into(),
        path: "src/ledger.rs".into(),
        side: Side::New,
        lines: LineRange::single(9),
    };
    view.update(cx, |view, cx| assert!(!view.select(&target, cx)));
    review.repos[0].files[0].new_lines = fixture::small().repos[0].files[0].new_lines.clone();
    view.update(cx, |view, cx| view.set_review(review, cx));
    draw(cx, window);
    view.update(cx, |view, cx| assert!(view.select(&target, cx)));
    assert_eq!(
        view.read_with(cx, |view, cx| view.selection(cx)),
        Some(target)
    );
}

/// RV-29: viewed folds unchanged content, survives refresh, and clears
/// when the file's content changes.
#[gpui_kit::test]
fn viewed_files_fold_until_the_content_changes(cx: &mut TestAppContext) {
    let (window, pane) = open(cx, fixture::small());
    cx.update_window(window, |_, window, cx| {
        window.click("viewed atmosphere/src/ledger.rs", cx)
    })
    .expect("the viewed control is shown");
    cx.run_until_parked();
    let view = view(cx, &pane);
    assert!(view.read_with(cx, |view, cx| view.is_file_collapsed(
        "atmosphere",
        "src/ledger.rs",
        cx
    )));
    pane.read_with(cx, |pane, _| {
        assert!(pane.events.contains(&DiffEvent::ViewedChanged {
            repo: "atmosphere".into(),
            path: "src/ledger.rs".into(),
            viewed: true,
            content_id: Some("blob:ledger-2".into()),
        }))
    });
    view.update(cx, |view, cx| view.set_review(fixture::small(), cx));
    assert!(view.read_with(cx, |view, cx| view.is_file_collapsed(
        "atmosphere",
        "src/ledger.rs",
        cx
    )));
    let mut changed = fixture::small();
    changed.repos[0].files[0].content_id = Some("blob:ledger-3".into());
    view.update(cx, |view, cx| view.set_review(changed, cx));
    assert!(!view.read_with(cx, |view, _| view.is_viewed("atmosphere", "src/ledger.rs")));
    assert!(!view.read_with(cx, |view, cx| view.is_file_collapsed(
        "atmosphere",
        "src/ledger.rs",
        cx
    )));
}

/// RV-19, RV-23: the counter follows replacement reviews, including an
/// empty comparison with no change to navigate to.
#[gpui_kit::test]
fn the_counter_follows_the_comparison_and_handles_no_changes(cx: &mut TestAppContext) {
    let (window, pane) = open(cx, fixture::small());
    let view = view(cx, &pane);
    assert_eq!(view.read_with(cx, |view, cx| view.change_position(cx).1), 5);
    view.update(cx, |view, cx| view.set_review(Review::default(), cx));
    draw(cx, window);
    assert_eq!(
        view.read_with(cx, |view, cx| view.change_position(cx)),
        (None, 0)
    );
    view.update(cx, |view, cx| view.next_change(cx));
    draw(cx, window);
    assert_eq!(
        view.read_with(cx, |view, cx| view.change_position(cx)),
        (None, 0)
    );
}

/// HIGHLIGHT: DiffView passes the shared detector's language to gpui-component's
/// DiffFile for every file, including Starlark (`BUILD.bazel`, `.bzl`), TOML,
/// YAML, Go, Rust, Java, Dockerfile, Terraform and shebang scripts.
#[gpui_kit::test]
fn diff_view_passes_detected_language_to_diff_files(cx: &mut TestAppContext) {
    use humata2_diff_view::{DiffLine, FileDiff, FileStatus, Hunk, LineKind, RepoDiff};

    let make_file = |path: &str, line: &str| FileDiff {
        path: path.to_owned(),
        status: FileStatus::Added,
        hunks: vec![Hunk {
            old_start: 0,
            new_start: 1,
            section: String::new(),
            lines: vec![DiffLine::new(LineKind::Added, line)],
        }],
        ..FileDiff::default()
    };

    let review = Review {
        repos: vec![RepoDiff {
            name: "repo".to_owned(),
            branch: "main".to_owned(),
            files: vec![
                make_file("src/lib.rs", "pub fn main() {}"),
                make_file("cmd/main.go", "package main"),
                make_file("Cargo.toml", "[package]"),
                make_file("ci.yaml", "name: ci"),
                make_file("App.java", "public class App {}"),
                make_file("BUILD.bazel", "load(\"@rules_rust//rust:defs.bzl\", \"rust_library\")"),
                make_file("bazel/defs.bzl", "def my_rule(name): pass"),
                make_file("infra/main.tf", "resource \"null_resource\" \"x\" {}"),
                make_file("Dockerfile", "FROM rust:1.80"),
                make_file("bin/tool", "#!/usr/bin/env python3"),
            ],
            ..RepoDiff::default()
        }],
        guide: None,
    };

    let (_window, pane) = open(cx, review);
    let diff_view = view(cx, &pane);
    let languages: Vec<(String, String)> = diff_view.read_with(cx, |view, cx| {
        view.diff_state()
            .read(cx)
            .files()
            .iter()
            .filter(|f| f.path().starts_with("repo/"))
            .map(|f| (f.path().to_string(), f.language().to_string()))
            .collect()
    });
    for (path, expected) in [
        ("repo/src/lib.rs", "rust"),
        ("repo/cmd/main.go", "go"),
        ("repo/Cargo.toml", "toml"),
        ("repo/ci.yaml", "yaml"),
        ("repo/App.java", "java"),
        ("repo/BUILD.bazel", "starlark"),
        ("repo/bazel/defs.bzl", "starlark"),
        ("repo/infra/main.tf", "terraform"),
        ("repo/Dockerfile", "dockerfile"),
        ("repo/bin/tool", "python"),
    ] {
        assert!(
            languages.iter().any(|(p, l)| p == path && l == expected),
            "expected ({path}, {expected}) in {languages:?}"
        );
    }
}

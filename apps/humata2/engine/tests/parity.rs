//! Natural frames, observation and parity, on whichever engine this test is
//! built with. Each test opens headless windows; nothing is shown.
//!
//! The fixtures are small on purpose: a board of cards that are each their
//! own view, so GPUI Fast can draw the untouched ones again from the last
//! frame, and variants with one defect each that the checks must catch.

#![cfg(target_os = "macos")]

use std::borrow::Cow;

// `gpui_kit::*` exports a `test` attribute; keep `#[test]` the standard one.
use ::std::prelude::v1::test;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::prelude::*;
use gpui_kit::*;
use humata2_engine::frame::{self, FrameError};
use humata2_engine::parity::{Comparison, Mismatch, Pair};
use humata2_engine::{Engine, Resources};

const DOT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" fill="black"/></svg>"#;

/// One icon, `icons/dot.svg`; every other path is missing.
struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok((path == "icons/dot.svg").then_some(Cow::Borrowed(DOT)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec!["icons/dot.svg".into()])
    }
}

/// What is wrong with a fixture, if anything.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Defect {
    None,
    /// A card's count changes without `cx.notify()`.
    ForgetsToNotify,
    /// A card's text is drawn fully transparent.
    InvisibleText,
    /// A card's icon names an asset that isn't there.
    MissingIcon,
}

struct Card {
    name: &'static str,
    count: usize,
    defect: Defect,
}

impl Render for Card {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = format!("{} {}", self.name, self.count);
        let defect = self.defect;
        let icon = if defect == Defect::MissingIcon {
            "icons/missing.svg"
        } else {
            "icons/dot.svg"
        };
        let text = if defect == Defect::InvisibleText {
            hsla(0., 0., 0., 0.)
        } else {
            hsla(0., 0., 1., 1.)
        };
        div()
            .id(self.name)
            .test_support()
            .role(Role::Button)
            .aria_label(label.clone())
            .flex()
            .gap_2()
            .items_center()
            .w(px(220.))
            .h(px(32.))
            .px_2()
            .bg(rgb(0x2f4a72))
            .text_color(text)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.count += 1;
                if defect != Defect::ForgetsToNotify {
                    cx.notify();
                }
            }))
            .child(
                svg()
                    .path(icon)
                    .size(px(16.))
                    .text_color(rgb(0xf0c040)),
            )
            .child(div().child(label.clone()).test_text(label))
    }
}

struct Board {
    cards: Vec<Entity<Card>>,
    notes: bool,
    rows: usize,
}

impl Board {
    fn new(defect: Defect, cx: &mut Context<Self>) -> Self {
        let cards = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|name| {
                cx.new(|_| Card {
                    name,
                    count: 0,
                    defect,
                })
            })
            .collect();
        Self {
            cards,
            notes: true,
            rows: 0,
        }
    }
}

impl Render for Board {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(rgb(0x111318))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .id("toggle-notes")
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Toggle notes")
                    .w(px(140.))
                    .h(px(24.))
                    .bg(rgb(0x50545c))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.notes = !this.notes;
                        cx.notify();
                    }))
                    .child(div().child("Toggle notes").test_text("Toggle notes")),
            )
            .when(self.notes, |board| {
                board.child(
                    div()
                        .id("notes")
                        .test_support()
                        .role(Role::Group)
                        .aria_label("Notes")
                        .h(px(24.))
                        .bg(rgb(0x3a2f20))
                        .child(div().child("Remember the milk").test_text("Remember the milk")),
                )
            })
            .children(self.cards.iter().cloned())
            .when(self.rows > 0, |board| {
                board.child(
                    div()
                        .id("rows")
                        .test_support()
                        .role(Role::List)
                        .aria_label("Rows")
                        .h(px(120.))
                        .overflow_y_scroll()
                        .bg(rgb(0x1c2029))
                        .children((0..self.rows).map(|row| {
                            let text = format!("row {row}");
                            div()
                                .id(("row", row))
                                .test_support()
                                .role(Role::ListItem)
                                .aria_label(text.clone())
                                .h(px(20.))
                                .when(row % 2 == 0, |row| row.bg(rgb(0x262b36)))
                                .child(div().child(text.clone()).test_text(text))
                        })),
                )
            })
    }
}

fn context() -> HeadlessAppContext {
    humata2_engine::headless(&Resources::new(Icons)).expect("the headless context starts")
}

fn open(cx: &mut HeadlessAppContext, defect: Defect, rows: usize) -> AnyWindowHandle {
    open_board(cx, defect, rows).0
}

fn open_board(
    cx: &mut HeadlessAppContext,
    defect: Defect,
    rows: usize,
) -> (AnyWindowHandle, Entity<Board>) {
    let opened = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(400.), px(360.)),
            })),
            show: false,
            focus: false,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| {
                let mut board = Board::new(defect, cx);
                board.rows = rows;
                board
            })
        })
        .expect("the headless window opens")
    });
    cx.run_until_parked();
    opened
}

fn pair(cx: &mut HeadlessAppContext, defect: Defect) -> Pair {
    let subject = open(cx, defect, 0);
    let reference = open(cx, defect, 0);
    Pair::new(cx, subject, reference, false).expect("the pair draws")
}

/// Clicks the middle of the visible element `id`, found in each window's
/// last frame.
fn click(id: &'static str) -> impl FnMut(&mut Window, &mut App) -> Result<(), FrameError> {
    move |window, cx| {
        let at = frame::find(window, id)
            .unwrap_or_else(|| panic!("{id} is in the last frame"))
            .bounds()
            .center();
        frame::click(window, at, MouseButton::Left, cx);
        Ok(())
    }
}

#[test]
fn natural_frames_match_a_fresh_window_and_reuse_untouched_views() {
    let mut cx = context();
    let pair = pair(&mut cx, Defect::None);
    pair.input(&mut cx, click("beta")).expect("the click lands");
    pair.input(&mut cx, click("beta")).expect("the click lands");
    pair.settle(&mut cx).expect("the windows settle");

    let report = pair.check(&mut cx, 2).expect("both windows draw");
    assert!(report.matches(), "{report}");
    let shown = cx
        .update_window(pair.subject, |_, window, _| {
            frame::find(window, "beta").and_then(|beta| beta.label().map(str::to_owned))
        })
        .expect("the subject is open");
    assert_eq!(shown.as_deref(), Some("beta 2"));
    if Engine::CURRENT.retains_views() {
        let counts = report.subject_counts.expect("GPUI Fast counts frames");
        assert!(counts.views_reused > 0, "no view was reused: {counts:?}");
    } else {
        assert!(report.subject_counts.is_none());
    }
}

/// Bumps the alpha card's count from outside any input, as a daemon event
/// would, notifying unless the card has [`Defect::ForgetsToNotify`].
fn bump_alpha(cx: &mut HeadlessAppContext, board: &Entity<Board>) {
    cx.update(|cx| {
        let alpha = board.read(cx).cards[0].clone();
        alpha.update(cx, |card, cx| {
            card.count += 1;
            if card.defect != Defect::ForgetsToNotify {
                cx.notify();
            }
        });
    });
}

/// A pair whose boards the test can change directly.
fn pair_of_boards(cx: &mut HeadlessAppContext, defect: Defect) -> (Pair, [Entity<Board>; 2]) {
    let (subject, subject_board) = open_board(cx, defect, 0);
    let (reference, reference_board) = open_board(cx, defect, 0);
    let pair = Pair::new(cx, subject, reference, false).expect("the pair draws");
    (pair, [subject_board, reference_board])
}

#[test]
fn an_update_that_notifies_draws_in_the_next_natural_frame() {
    let mut cx = context();
    let (pair, boards) = pair_of_boards(&mut cx, Defect::None);
    for board in &boards {
        bump_alpha(&mut cx, board);
    }
    pair.settle(&mut cx).expect("the windows settle");
    let report = pair.check(&mut cx, 2).expect("both windows draw");
    assert!(report.matches(), "{report}");
}

#[test]
fn a_view_that_forgets_to_notify_is_caught_where_views_are_retained() {
    let mut cx = context();
    let (pair, boards) = pair_of_boards(&mut cx, Defect::ForgetsToNotify);
    for board in &boards {
        bump_alpha(&mut cx, board);
    }
    pair.settle(&mut cx).expect("the windows settle");

    let report = pair.check(&mut cx, 2).expect("both windows draw");
    if Engine::CURRENT.retains_views() {
        // The subject still shows "alpha 0"; the fresh reference "alpha 1".
        assert!(
            report.mismatches.iter().any(|mismatch| matches!(
                mismatch,
                Mismatch::Element { path, .. } if path.contains("alpha")
            )),
            "{report}"
        );
        assert!(
            report
                .mismatches
                .iter()
                .any(|mismatch| matches!(mismatch, Mismatch::Pixels { .. })),
            "{report}"
        );
    } else {
        // A natural frame here always draws (GPUI has no public "is anything
        // dirty"; see humata2_engine::frame), and ordinary GPUI builds every
        // uncached view on each draw, so the forgotten notify can't be seen.
        // The Fast run of this test is the one that catches it.
        assert!(report.matches(), "{report}");
    }
}

#[test]
fn a_removed_control_is_not_in_the_frame() {
    let mut cx = context();
    let pair = pair(&mut cx, Defect::None);
    let shows_notes = |cx: &mut HeadlessAppContext| {
        cx.update_window(pair.subject, |_, window, _| frame::find(window, "notes").is_some())
            .expect("the subject is open")
    };
    assert!(shows_notes(&mut cx));

    pair.input(&mut cx, click("toggle-notes")).expect("the click lands");
    pair.settle(&mut cx).expect("the windows settle");
    assert!(!shows_notes(&mut cx));
    let report = pair.check(&mut cx, 2).expect("both windows draw");
    assert!(report.matches(), "{report}");

    pair.input(&mut cx, click("toggle-notes")).expect("the click lands");
    pair.settle(&mut cx).expect("the windows settle");
    assert!(shows_notes(&mut cx));
}

/// Draws a fixture alone and observes it.
fn drawn(defect: Defect) -> (frame::Frame, frame::Picture) {
    let mut cx = context();
    let window = open(&mut cx, defect, 0);
    frame::settle(&mut cx, window, |cx| cx.run_until_parked(), 12).expect("it settles");
    cx.update_window(window, |_, window, _| {
        (
            frame::observe(window),
            frame::pixels(window).expect("Metal draws off screen"),
        )
    })
    .expect("the window is open")
}

fn semantic(mismatch: &Mismatch) -> bool {
    matches!(mismatch, Mismatch::Element { .. } | Mismatch::Text { .. })
}

#[test]
fn pixels_catch_invisible_text_and_a_missing_icon_that_semantics_miss() {
    let (good, good_picture) = drawn(Defect::None);
    for defect in [Defect::InvisibleText, Defect::MissingIcon] {
        let (bad, bad_picture) = drawn(defect);
        let report = humata2_engine::parity::compare(
            &bad,
            &good,
            &bad_picture,
            &good_picture,
            Comparison {
                channel_tolerance: 2,
                painted: true,
            },
        );
        assert!(
            !report.mismatches.iter().any(semantic),
            "the defect changed semantics, so it doesn't test pixels: {report}"
        );
        assert!(
            report
                .mismatches
                .iter()
                .any(|mismatch| matches!(mismatch, Mismatch::Pixels { .. })),
            "the pixels missed the defect: {report}"
        );
    }
}

#[test]
fn settling_gives_up_on_a_window_that_never_stops_changing() {
    struct Ticker(usize);
    impl Render for Ticker {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0 += 1;
            window.request_animation_frame();
            let text = format!("tick {}", self.0);
            div().child(text.clone()).test_text(text)
        }
    }

    let mut cx = context();
    let window = cx.update(|cx| {
        let options = WindowOptions {
            show: false,
            focus: false,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Ticker(0)))
            .expect("the headless window opens")
            .0
    });
    let settled = frame::settle(&mut cx, window, |cx| cx.run_until_parked(), 5);
    assert!(
        matches!(settled, Err(FrameError::Unsettled { frames: 5 })),
        "{settled:?}"
    );
}

/// Scrolling, with GPUI Fast's scroll layers on where the engine has them.
#[test]
fn scrolling_matches_a_fresh_window() {
    let mut cx = context();
    let subject = open(&mut cx, Defect::None, 40);
    let reference = open(&mut cx, Defect::None, 40);
    let layers = Engine::CURRENT.retains_views();
    let pair = Pair::new(&mut cx, subject, reference, layers).expect("the pair draws");
    for _ in 0..3 {
        pair.input(&mut cx, |window, cx| {
            let at = frame::find(window, "rows")
                .expect("the rows are drawn")
                .bounds()
                .center();
            frame::scroll(window, at, point(px(0.), px(-30.)), cx);
            Ok(())
        })
        .expect("the scroll lands");
    }
    pair.settle(&mut cx).expect("the windows settle");
    let report = pair.check(&mut cx, 2).expect("both windows draw");
    assert!(report.matches(), "{report}");
    let mut shows = |id| {
        cx.update_window(subject, |_, window, _| frame::find(window, id).is_some())
            .expect("the subject is open")
    };
    // Scrolled by 90 pixels, rows of 20: row 0 is out of view, row 6 in it.
    assert!(!shows("row-0"), "the first row should have scrolled out");
    assert!(shows("row-6"), "row 6 should have scrolled in");
}

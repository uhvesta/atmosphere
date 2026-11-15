//! The sidebar tree in a headless window, driven through real input: the
//! arrow keys and the Expand all and Collapse all buttons change the
//! reducer's state, and the rows drawn follow it.

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Point, Role, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use humata2_gui_spike::tree::{CHANNELS, SESSIONS_PER_CHANNEL, TreeRow, TreeScreen, TreeStore};

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TreeScreen>) {
    cx.update(|cx| {
        humata2_gui_spike::init(cx);
        let store = TreeStore::new().expect("the tree's store starts");
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(480.), px(600.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| TreeScreen::new(store, window, cx))
        })
        .expect("the test window opens")
    })
}

/// The labels of the tree rows on screen, top to bottom.
fn drawn_labels(window: &Window) -> Vec<String> {
    let mut rows: Vec<_> = window
        .find_all_by_role(Role::TreeItem)
        .into_iter()
        .filter(|row| row.visible())
        .collect();
    rows.sort_by(|a, b| {
        a.bounds()
            .top()
            .partial_cmp(&b.bounds().top())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.iter()
        .filter_map(|row| row.label().map(str::to_owned))
        .collect()
}

/// The labels the state says come first, as many as `count`.
fn expected_labels(tree: &TreeScreen, from: usize, count: usize) -> Vec<String> {
    tree.with_state(|state| {
        state
            .visible_rows()
            .iter()
            .skip(from)
            .take(count)
            .filter_map(|row| state.outline().label(*row).map(|label| label.to_string()))
            .collect()
    })
}

fn selected(tree: &Entity<TreeScreen>, cx: &mut gpui_kit::App) -> Option<TreeRow> {
    tree.read(cx).with_state(|state| state.selected())
}

#[gpui_kit::test]
fn arrow_keys_move_the_selection_and_the_drawn_row_shows_it(cx: &mut TestAppContext) {
    let (handle, tree) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = drawn_labels(window);
        assert!(!first.is_empty());
        assert_eq!(first, expected_labels(tree.read(cx), 0, first.len()));

        window.press("down", cx);
        window.press("down", cx);
        window.press("up", cx);
        assert_eq!(selected(&tree, cx), Some(TreeRow::Channel(0)));

        window.press("down", cx);
        assert_eq!(selected(&tree, cx), Some(TreeRow::Channel(1)));
        let row = window.find(("tree-row", TreeRow::Channel(1).key()));
        assert_eq!(row.selected(), Some(true));
        let other = window.find(("tree-row", TreeRow::Channel(0).key()));
        assert_eq!(other.selected(), Some(false));
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn expand_all_opens_every_channel_and_draws_its_sessions(cx: &mut TestAppContext) {
    let (handle, tree) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("expand-all", cx);
        window.render_frame(cx);

        let (open, rows) = tree.read(cx).with_state(|state| {
            (state.expanded().len(), state.visible_rows().len())
        });
        assert_eq!(open, CHANNELS);
        assert_eq!(rows, CHANNELS * (1 + SESSIONS_PER_CHANNEL));

        let drawn = drawn_labels(window);
        assert!(drawn.len() > 1);
        assert_eq!(drawn, expected_labels(tree.read(cx), 0, drawn.len()));
        let channel = window.find(("tree-row", TreeRow::Channel(0).key()));
        assert_eq!(channel.expanded(), Some(true));
        assert!(
            window
                .try_find(("tree-row", TreeRow::Session { channel: 0, session: 0 }.key()))
                .is_some()
        );
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn moving_past_the_bottom_scrolls_the_selection_into_view(cx: &mut TestAppContext) {
    let (handle, tree) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("expand-all", cx);
        for _ in 0..40 {
            window.press("down", cx);
        }
        window.render_frame(cx);

        let target = TreeRow::Session { channel: 1, session: 13 };
        assert_eq!(selected(&tree, cx), Some(target));
        let row = window.find(("tree-row", target.key()));
        assert!(row.visible());
        assert_eq!(row.selected(), Some(true));
        let top = window
            .try_find(("tree-row", TreeRow::Channel(0).key()))
            .filter(|row| row.visible());
        assert!(top.is_none());

        let drawn = drawn_labels(window);
        let first = tree.read(cx).with_state(|state| {
            state
                .visible_rows()
                .iter()
                .position(|row| state.outline().label(*row).map(|l| l.as_ref()) == drawn.first().map(String::as_str))
        });
        let first = first.expect("the top drawn row is in the state");
        assert_eq!(drawn, expected_labels(tree.read(cx), first, drawn.len()));
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn collapse_all_moves_a_session_selection_onto_its_channel(cx: &mut TestAppContext) {
    let (handle, tree) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("expand-all", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        assert_eq!(
            selected(&tree, cx),
            Some(TreeRow::Session { channel: 0, session: 1 })
        );

        window.click("collapse-all", cx);
        window.render_frame(cx);
        assert_eq!(selected(&tree, cx), Some(TreeRow::Channel(0)));
        assert!(tree.read(cx).with_state(|state| state.expanded().is_empty()));
        assert_eq!(
            window.find(("tree-row", TreeRow::Channel(0).key())).selected(),
            Some(true)
        );

        // The keys still reach the tree after a button took the click.
        window.press("down", cx);
        assert_eq!(selected(&tree, cx), Some(TreeRow::Channel(1)));
    })
    .expect("the test window is open");
}

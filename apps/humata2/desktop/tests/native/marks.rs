//! Controls that act: the channel page's Pin (NV-20, frames 18 and 35)
//! pins and unpins its channel on the host, and the arrow keys move the
//! palette's cursor (KP-2).
use super::*;
use humata2_clients::marks::{Call, Thing};
use humata2_clients::model::ChannelId;

#[test]
fn the_channel_pages_pin_pins_and_unpins_its_channel() {
    let mut app = App::new(fixture::design());
    let channel = ChannelId::new("payments-rewrite");
    app.send(AppAction::Switch(Route::Channel(channel.clone())));
    app.click("channel-pin");
    let thing = Thing::Channel(channel);
    assert!(
        app.fakes
            .marks
            .calls()
            .contains(&Call::Set(thing.clone(), Some(true), None)),
        "Pin asks the host to pin the channel: {:?}",
        app.fakes.marks.calls()
    );
    assert!(
        app.ui
            .read(|s| s.pinned.list.items.iter().any(|p| p.thing == thing)),
        "the Pinned section shows the channel"
    );
    app.click("channel-pin");
    assert!(
        app.fakes
            .marks
            .calls()
            .contains(&Call::Set(thing.clone(), Some(false), None)),
        "a second click unpins it"
    );
    assert!(
        app.ui
            .read(|s| s.pinned.list.items.iter().all(|p| p.thing != thing)),
        "the Pinned section drops it"
    );
}

#[test]
fn arrow_keys_move_the_palette_cursor() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.send(AppAction::OpenPalette);
    let rows = app.ui.read(|s| s.palette.entries.len());
    assert!(rows > 2, "the palette lists several rows");
    assert_eq!(app.ui.read(|s| s.palette.cursor), 0);
    let press = |app: &mut App, key: &str| {
        app.cx
            .update_window(app.window, |_, window, cx| window.press(key, cx))
            .unwrap();
        app.settle();
    };
    press(&mut app, "down");
    assert_eq!(app.ui.read(|s| s.palette.cursor), 1, "down moves to the next row");
    press(&mut app, "down");
    assert_eq!(app.ui.read(|s| s.palette.cursor), 2);
    press(&mut app, "up");
    assert_eq!(app.ui.read(|s| s.palette.cursor), 1, "up moves back");
    press(&mut app, "up");
    press(&mut app, "up");
    assert_eq!(
        app.ui.read(|s| s.palette.cursor),
        rows - 1,
        "up from the first row wraps to the last"
    );
}

//! SPEC flow 03 (on call) on a real daemon: a channel opens from the
//! palette, its Pin is kept on the host and opens it again from the sidebar,
//! and its Sessions tab lists a running session with its state.
//!
//! The target's deps hold no palette crate, so entries are told apart by
//! their `Debug` names, as in `find.rs`.
use gpui_kit::Role;
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::marks::Thing;
use humata2_flow::Action;

use super::world::World;
use super::{App, channel_with_repo, running_session, runtime, session_in};

const CHANNEL: &str = "oncall";

/// How the view writes an element's line: its role, then its label.
fn heading(role: Role, label: &str) -> String {
    format!("{role:?} {label:?}")
}

/// `text` as the view writes it inside a line.
fn written(text: &str) -> String {
    let quoted = format!("{text:?}");
    quoted[1..quoted.len() - 1].to_owned()
}

/// The lines of `view` drawn inside the element `role` `label`; `None` when
/// no such element shows.
fn inside(view: &str, role: Role, label: &str) -> Option<String> {
    let heading = heading(role, label);
    let mut lines = view.lines();
    let depth = loop {
        let line = lines.next()?;
        let rest = line.trim_start();
        let states = rest.strip_prefix(heading.as_str());
        if states.is_some_and(|states| states.is_empty() || states.starts_with(' ')) {
            break line.len() - rest.len();
        }
    };
    Some(
        lines
            .take_while(|line| line.len() - line.trim_start().len() > depth)
            .map(|line| format!("{line}\n"))
            .collect(),
    )
}

fn palette_open(app: &App) -> bool {
    app.ui
        .read(|state| state.palette.open && state.overlays.last() == Some(&Overlay::Palette))
}

fn pinned(app: &App, thing: &Thing) -> bool {
    app.ui
        .read(|state| state.pinned.list.items.iter().any(|pin| &pin.thing == thing))
}

#[test]
fn the_palette_opens_a_channel_and_its_pin_is_kept_on_the_host() {
    let runtime = runtime();
    let (world, clients, channel) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, repo) = channel_with_repo(&world, &clients, CHANNEL).await;
        // A channel offers Pin once it has a session (frames 18 and 35).
        session_in(&world, &clients, &channel, repo, "Page triage", "triage the pages").await;
        (world, clients, channel)
    });
    let marks = clients.marks.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the channel to load", |app| {
        app.ui
            .read(|state| state.workspace.channel(&channel).is_some())
    });
    let thing = Thing::Channel(channel.clone());

    // 1. ⌘K, the channel's name, Return: its page opens on Sessions.
    app.play(&Action::Key(super::app_key("k")));
    app.wait_until("the palette to open", |app| palette_open(app));
    app.type_text(CHANNEL);
    app.wait_until("the palette to take what was typed", |app| {
        app.ui.read(|state| state.palette.query == CHANNEL)
    });
    let first = app.ui.read(|state| {
        state
            .palette
            .entries
            .first()
            .map(|entry| format!("{:?}", entry.command))
    });
    assert!(
        first.as_deref().is_some_and(|name| name.starts_with("Channel(")),
        "the channel's own entry is first for its name: {first:?}"
    );
    app.play(&Action::Key("enter".to_owned()));
    app.wait_until("the channel page to open", |app| {
        app.ui
            .read(|state| state.route == Route::Channel(channel.clone()))
    });
    assert!(!palette_open(&app), "the palette closes when its entry runs");

    // 2. Pin: the host keeps it and the sidebar's Pinned section lists it.
    assert!(!pinned(&app, &thing), "nothing is pinned yet");
    app.click(Role::Button, "Pin this channel");
    app.wait_until("the host to keep the pin", |app| {
        app.runtime
            .block_on(marks.pinned())
            .is_ok_and(|list| list.items.iter().any(|pin| pin.thing == thing))
    });
    app.wait_until("the Pinned section to list the channel", |app| {
        pinned(app, &thing)
    });
    let title = app
        .ui
        .read(|state| {
            state
                .pinned
                .list
                .items
                .iter()
                .find(|pin| pin.thing == thing)
                .map(|pin| pin.title.clone())
        })
        .expect("the pin is listed");

    // 3. From another page, its Pinned row opens the channel.
    app.click(Role::Link, "Home");
    app.wait_until("Home to show", |app| {
        app.ui.read(|state| state.route == Route::Home)
    });
    app.click(Role::ListItem, &title);
    app.wait_until("the pinned channel to open", |app| {
        app.ui
            .read(|state| state.route == Route::Channel(channel.clone()))
    });

    drop(app);
    drop(world);
}

#[test]
fn a_channels_sessions_tab_lists_its_session_with_its_state() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "reconcile the invoices").await;
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui
            .read(|state| state.workspace.session(&id).is_some())
    });
    let channel = app
        .ui
        .read(|state| state.workspace.session(&id).map(|s| s.channel.clone()))
        .expect("the session is loaded");

    // The channel page opens on its Sessions tab (a Route, so the app's own
    // navigation is used rather than a sidebar click on a channel name).
    app.cx
        .update(|cx| {
            app.ui
                .send(AppAction::Navigate(Route::Channel(channel.clone())), cx)
        });
    app.settle();
    app.wait_until("the channel's Sessions tab to list the session", |app| {
        let status = app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                .map(|session| session.status.label())
        });
        let Some(status) = status else {
            return false;
        };
        inside(&app.view(), Role::Link, "Fix the ledger")
            .is_some_and(|row| row.contains(&written(&status)))
    });

    drop(app);
    drop(world);
}

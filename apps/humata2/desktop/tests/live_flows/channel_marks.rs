//! SPEC flow 03 (on call) on a real daemon: step 8, a session marked VIP
//! and pinned from the palette, kept on the host across a restart; and
//! step 10, the channel's Sessions tab with a session unread after a real
//! turn, its filter, its fold of a fork, and a row that opens its session.
//!
//! The target's deps hold no palette crate, so entries are told apart by
//! their `Debug` names, as in `find.rs`.

use gpui_kit::Role;
use humata2_app::channel::Action as ChannelAction;
use humata2_app::{AppAction, Route};
use humata2_clients::{Clients, Event};
use humata2_clients::marks::Thing;
use humata2_clients::model::{Agent, ChannelId, SessionId, SessionOptions, Turn};
use humata2_flow::Action;
use humata2_session::SessionAction;

use super::world::World;
use super::{App, channel_with_repo, running_session, runtime};

/// A prompt as Claude writes it to its conversation file, so the session
/// has a conversation a fork can carry.
fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "isSidechain": false,
        "message": {"role": "user", "content": text},
        "timestamp": "2026-11-15T09:00:00.000Z",
    })
    .to_string()
}

fn command(entry: &impl std::fmt::Debug) -> String {
    format!("{entry:?}")
}

fn palette_open(app: &App) -> bool {
    app.ui.read(|state| {
        state.palette.open && state.overlays.last() == Some(&humata2_app::Overlay::Palette)
    })
}

/// ⌘K, then waits until the palette shows with nothing typed.
fn open_palette(app: &mut App) {
    app.play(&Action::Key(super::app_key("k")));
    app.wait_until("the palette to open", |app| {
        palette_open(app) && app.ui.read(|state| state.palette.query.is_empty())
    });
}

/// Types `text` into the palette and waits until it has it.
fn type_query(app: &mut App, text: &str) {
    app.type_text(text);
    app.wait_until("the palette to take what was typed", |app| {
        app.ui.read(|state| state.palette.query == text)
    });
}

/// The title of the entry whose command is named `name` and says `on`.
fn mark_entry(app: &App, name: &str, on: bool) -> Option<String> {
    let says = format!("on: {on}");
    app.ui.read(|state| {
        state
            .palette
            .entries
            .iter()
            .find(|entry| {
                let named = command(&entry.command);
                named.starts_with(name) && named.contains(&says)
            })
            .map(|entry| entry.title.clone())
    })
}

/// Types `query`, waits for the mark entry, and clicks it; the palette
/// closes when its entry runs.
fn pick_mark(app: &mut App, query: &str, name: &str, on: bool) {
    open_palette(app);
    type_query(app, query);
    app.wait_until("the palette to offer the mark", |app| {
        mark_entry(app, name, on).is_some()
    });
    let title = mark_entry(app, name, on).expect("offered after the wait");
    app.click(Role::ListBoxOption, &title);
    app.wait_until("the palette to close", |app| !palette_open(app));
}

/// The host's pin of `thing`, as the marks client reads it now.
fn host_pin(app: &App, marks: &dyn humata2_clients::marks::MarksClient, thing: &Thing) -> Option<bool> {
    app.runtime
        .block_on(marks.pinned())
        .ok()
        .and_then(|list| list.items.iter().find(|pin| &pin.thing == thing).map(|pin| pin.vip))
}

/// Flow 03 step 8: ⌘K finds the session, its VIP entry marks it, its Pin
/// entry pins it; the host lists the pin with the VIP mark, the app's own
/// pins agree, the palette then offers to clear both, and a restart of the
/// daemon keeps them.
#[test]
fn the_palette_marks_a_session_vip_and_pins_it_and_the_host_keeps_both() {
    const TITLE: &str = "Checkout fix";
    let runtime = runtime();
    let (mut world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, TITLE, "reconcile the invoices").await;
        (world, clients, id)
    });
    let marks = clients.marks.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    let thing = Thing::Session(id.clone());

    // 1. ⌘K, the session's title, Return: its page opens.
    open_palette(&mut app);
    type_query(&mut app, TITLE);
    app.wait_until("the session to be the first entry", |app| {
        app.ui.read(|state| {
            state
                .palette
                .entries
                .first()
                .is_some_and(|entry| command(&entry.command).starts_with("Session("))
        })
    });
    app.play(&Action::Key("enter".to_owned()));
    app.wait_until("the session page to open", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });

    // 2. Over the session, VIP is offered to set. Picking it hands the
    // mark to the host. An unpinned thing isn't listed, so the host shows
    // the mark once the session is pinned in the next step.
    assert_eq!(host_pin(&app, &*marks, &thing), None, "nothing is pinned yet");
    pick_mark(&mut app, "vip", "Vip", true);
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Session(id.clone()));

    // 3. Pin: the host lists the session, with its VIP mark; so does the app.
    pick_mark(&mut app, "pin", "Pin", true);
    app.wait_until("the host to keep the pin with its VIP mark", |app| {
        host_pin(app, &*marks, &thing) == Some(true)
    });
    app.wait_until("the app's pins to hold the VIP pin", |app| {
        app.ui.read(|state| {
            state
                .pinned
                .list
                .items
                .iter()
                .any(|pin| pin.thing == thing && pin.vip)
        })
    });

    // 4. The palette now offers to clear what it set.
    open_palette(&mut app);
    type_query(&mut app, "vip");
    app.wait_until("the palette to offer clearing VIP", |app| {
        mark_entry(app, "Vip", false).is_some() && mark_entry(app, "Vip", true).is_none()
    });
    app.play(&Action::Key("escape".to_owned()));
    app.wait_until("the palette to close", |app| !palette_open(app));
    open_palette(&mut app);
    type_query(&mut app, "pin");
    app.wait_until("the palette to offer unpinning", |app| {
        mark_entry(app, "Pin", false).is_some() && mark_entry(app, "Pin", true).is_none()
    });
    app.play(&Action::Key("escape".to_owned()));
    app.wait_until("the palette to close", |app| !palette_open(app));
    drop(app);

    // 5. The marks live on the host: they survive its restart.
    let clients = runtime.block_on(async {
        world.reboot().await.expect("reboot");
        world.clients().await.expect("connect again")
    });
    let list = runtime
        .block_on(clients.marks.pinned())
        .expect("the host lists its pins");
    assert!(
        list.items.iter().any(|pin| pin.thing == thing && pin.vip),
        "the pin and its VIP mark survive a restart: {:?}",
        list.items
    );
    drop(world);
}

/// Starts a session in `channel` through the daemon and waits until the
/// agent has taken its first message: on each turn end the daemon tells of
/// that session, the conversation is read once (D389). The world must
/// report turns ([`World::start_with_turn_hooks`]).
async fn start_in(
    world: &World,
    clients: &Clients,
    channel: &ChannelId,
    repo: &str,
    title: &str,
    first: &str,
) -> SessionId {
    let mac = World::this_mac(clients)
        .await
        .expect("the workspace has This Mac");
    // Followed before the session exists, so no turn end is missed; what
    // comes before the id is known waits in the stream.
    let mut events = clients.daemon.events().await.expect("follow the daemon");
    let id = clients
        .daemon
        .create_session(
            channel,
            Agent::Claude,
            &mac,
            title,
            SessionOptions {
                repos: vec![repo.to_owned()],
                first_message: first.to_owned(),
                ..SessionOptions::default()
            },
        )
        .await
        .expect("create the session");
    let took = tokio::time::timeout(super::DEADLINE, async {
        while let Some(event) = events.recv().await {
            if let Ok(Event::Session(session)) = event
                && session.id == id
                && session.turn == Turn::Ended
                && world.conversation_text().contains(first)
            {
                return true;
            }
        }
        false
    })
    .await;
    assert!(
        matches!(took, Ok(true)),
        "the agent never ended a turn on its first message"
    );
    id
}

/// Whether the channel page shows a row for the session titled `title`
/// (the rows are links; the sidebar's session rows are tree items).
fn row_shown(view: &str, title: &str) -> bool {
    let heading = format!("{:?} {title:?}", Role::Link);
    view.lines().any(|line| {
        line.trim_start()
            .strip_prefix(heading.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// How many turns of unread the host holds for `id`.
fn daemon_unread(app: &App, id: &SessionId) -> Option<u32> {
    let daemon = app.daemon.clone();
    app.runtime
        .block_on(daemon.load())
        .ok()?
        .sessions
        .iter()
        .find(|session| &session.id == id)
        .map(|session| session.unread)
}

fn send(app: &mut App, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// Flow 03 step 10: a session that answered while nobody viewed it is
/// unread on the host and in the app, and turns unread again after a real
/// second turn; the channel's Sessions tab lists it beside a session and its
/// fork, the fork's fold hides and shows the fork, the filter keeps
/// matches and their ancestors, and a row opens its session, which marks it
/// read on the host.
#[test]
fn a_channels_sessions_tab_shows_unread_turns_and_filters_and_folds_forks() {
    const PARENT: &str = "Write path";
    const OTHER: &str = "Refund retries";
    let runtime = runtime();
    let (world, clients, parent, other) = runtime.block_on(async {
        // The agent reports its turns (D389), so `start_in` waits on them.
        let world = World::start_with_turn_hooks().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, repo) = channel_with_repo(&world, &clients, "payments").await;
        let parent = start_in(
            &world,
            &clients,
            &channel,
            &repo,
            PARENT,
            &prompt("write the ledger path"),
        )
        .await;
        let other = start_in(&world, &clients, &channel, &repo, OTHER, "refund the invoices").await;
        (world, clients, parent, other)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("both sessions to load", |app| {
        app.ui.read(|state| {
            state.workspace.session(&parent).is_some() && state.workspace.session(&other).is_some()
        })
    });
    let channel = app
        .ui
        .read(|state| state.workspace.session(&parent).map(|s| s.channel.clone()))
        .expect("the parent is loaded");

    // The agent answered the first message while nobody viewed the session:
    // the host marks it unread and the app is told.
    app.wait_until("the unviewed session to show unread", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&other)
                .is_some_and(|session| session.unread > 0)
        })
    });
    assert!(
        daemon_unread(&app, &other).is_some_and(|unread| unread > 0),
        "the host holds the unread mark"
    );

    // A fork of the parent, from its page.
    app.show_session(&parent);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(parent.clone()))
    });
    app.click(Role::Button, "Fork");
    assert!(
        app.ui.read(|state| state.session.fork.is_some()),
        "Fork opens the dialog"
    );
    // The dialog's own Fork button shares its label with the page's, so the
    // driver can't pick one; the confirm is sent as the button sends it.
    send(&mut app, AppAction::Session(SessionAction::ConfirmFork));
    app.wait_until("the fork to be listed under its parent", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .any(|session| session.parent.as_ref() == Some(&parent))
        })
    });
    let child = app
        .ui
        .read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .find(|session| session.parent.as_ref() == Some(&parent))
                .map(|session| session.title.clone())
        })
        .expect("the fork is listed");
    // The fork is listed as soon as it is recorded, but the app opens it
    // only when the fork's operation finishes, once its agent has started.
    // Wait for that, or the late open takes the app off the channel page
    // further down.
    app.wait_until("the finished fork to open", |app| {
        app.ui.read(|state| {
            !state.session.forking
                && matches!(&state.route, Route::Session(id) if state
                    .workspace
                    .session(id)
                    .is_some_and(|session| session.parent.as_ref() == Some(&parent)))
        })
    });

    // A real turn end: read it, give the agent another line, and the
    // answer marks it unread again.
    runtime
        .block_on(app.daemon.clone().mark_read(&other))
        .expect("mark it read");
    // mark_read also views the session (CH-17, `Daemon::mark_read`), and the
    // app shares this client: hand the view back to the page the app shows,
    // or `other` stays viewed and its answer marks nothing.
    let shown = app.ui.read(|state| state.session.open.clone());
    runtime
        .block_on(app.daemon.clone().view(shown.as_ref()))
        .expect("view the app's page again");
    assert_eq!(daemon_unread(&app, &other), Some(0), "the host cleared the mark");
    runtime
        .block_on(app.daemon.clone().send(&other, "second turn"))
        .expect("type the second turn");
    app.wait_until("the agent to take the second turn", |_| {
        world.conversation_text().contains("second turn")
    });
    app.wait_until("the answer to mark the session unread again", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&other)
                .is_some_and(|session| session.unread > 0)
        })
    });
    assert!(
        daemon_unread(&app, &other).is_some_and(|unread| unread > 0),
        "the host holds the new unread mark"
    );

    // The channel page opens on Sessions: all three rows, the fork nested.
    send(&mut app, AppAction::Navigate(Route::Channel(channel.clone())));
    app.wait_until("the Sessions tab to list the three sessions", |app| {
        let view = app.view();
        row_shown(&view, PARENT) && row_shown(&view, &child) && row_shown(&view, OTHER)
    });

    // Fold: the parent's chevron hides its fork; Expand all shows it again.
    app.click(Role::Button, "Fold session forks");
    app.wait_until("the fork to fold away", |app| {
        let view = app.view();
        row_shown(&view, PARENT) && !row_shown(&view, &child) && row_shown(&view, OTHER)
    });
    app.click(Role::Button, "Expand all");
    app.wait_until("the fork to show again", |app| row_shown(&app.view(), &child));

    // Filter. The filter's text box has no role or label to click, so the
    // text goes to the page as the box sends it.
    send(&mut app, AppAction::Channel(ChannelAction::Filter("refund".to_owned())));
    app.wait_until("the filter to keep only the matching session", |app| {
        let view = app.view();
        row_shown(&view, OTHER) && !row_shown(&view, PARENT) && !row_shown(&view, &child)
    });
    send(&mut app, AppAction::Channel(ChannelAction::Filter(child.clone())));
    app.wait_until("the filter to keep the fork and its parent", |app| {
        let view = app.view();
        row_shown(&view, PARENT) && row_shown(&view, &child) && !row_shown(&view, OTHER)
    });
    send(&mut app, AppAction::Channel(ChannelAction::Filter(String::new())));
    app.wait_until("the filter to clear", |app| row_shown(&app.view(), OTHER));

    // A row opens its session; viewing it marks it read on the host.
    app.click(Role::Link, OTHER);
    app.wait_until("the session to open", |app| {
        app.ui.read(|state| state.route == Route::Session(other.clone()))
    });
    app.wait_until("the host to mark the opened session read", |app| {
        daemon_unread(app, &other) == Some(0)
    });

    drop(app);
    drop(world);
}

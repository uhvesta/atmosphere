//! The ⌥⌘M key (macro case 05 step 11a) on a real daemon: pressed with a
//! session open it asks for a machine in the palette, as the fake-side
//! Move tests expect; pressed with no session open it moves nothing and
//! says why. The key is pressed as the app receives it (ui/src/keys.rs:109).
//!
//! Only the key's reach is proven here. The move itself, and its dialog's
//! plan, are proven by the daemon's move tests and the palette test in
//! `find.rs`.
use humata2_app::{AppAction, Overlay, Route};
use humata2_flow::Action;

use super::world::World;
use super::{App, running_session, runtime};

/// ⌥⌘M, as GPUI writes it. Off a Mac the app moves the chord (D318), so
/// this rig, which only runs on a Mac, names the Mac one.
const MOVE_KEY: &str = "alt-cmd-m";

fn palette_open(app: &App) -> bool {
    app.ui.read(|state| {
        state.palette.open && state.overlays.last() == Some(&Overlay::Palette)
    })
}

fn asking(app: &App) -> bool {
    app.ui.read(|state| state.palette.asking.is_some())
}

#[test]
fn the_move_key_asks_for_a_machine_in_a_session_and_says_why_nothing_moves_outside_one() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before the move").await;
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });

    // No session open: the key moves nothing and a note says why.
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Home), cx));
    app.settle();
    let toasts = app.ui.read(|state| state.toasts.len());
    app.play(&Action::Key(MOVE_KEY.to_owned()));
    app.wait_until("a note that there is nothing to move", |app| {
        app.ui.read(|state| state.toasts.len() > toasts)
    });
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Home);
    assert!(!palette_open(&app), "no machine is asked for");

    // A session open: the key opens the palette already asking for a
    // machine to move that session to.
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    app.play(&Action::Key(MOVE_KEY.to_owned()));
    app.wait_until("the palette to ask for a machine", |app| {
        palette_open(app) && asking(app)
    });
    assert_eq!(
        app.ui.read(|state| state.palette.context.clone()),
        Some(id.clone()),
        "it is asking about the open session"
    );

    // Escape steps back from the question, then closes the palette; the
    // session stays where it runs.
    app.play(&Action::Key("escape".to_owned()));
    app.wait_until("Escape to step back from the question", |app| !asking(app));
    assert!(palette_open(&app), "the first Escape only steps back");
    app.play(&Action::Key("escape".to_owned()));
    app.wait_until("the palette to close", |app| !palette_open(app));
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Session(id.clone()));
    drop(app);
    drop(world);
}

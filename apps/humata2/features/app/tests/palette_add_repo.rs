//! RL-5, KP-4: running "Add <repo> to this channel…" from the palette opens
//! the channel's Add a GitHub repo sheet with the repo already in its field.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Overlay, Route};
use humata2_clients::model::ChannelId;
use humata2_clients::{Clients, fixture};
use humata2_palette::PaletteAction as P;
use tca::{TestClock, TestStore};

fn on_channel() -> (TestStore<App>, ChannelId) {
    let workspace = fixture::design();
    let Some(channel) = workspace.channels.first().map(|channel| channel.id.clone()) else {
        panic!("the design workspace has a channel");
    };
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(channel.clone());
    (TestStore::new(state, App::new(clients)).non_exhaustive(), channel)
}

fn settle(store: &mut TestStore<App>) {
    for _ in 0..4 {
        store.skip_received();
    }
}

#[test]
fn a_pasted_repo_opens_the_channels_add_sheet_prefilled() {
    let (mut store, channel) = on_channel();
    store.send(A::OpenPalette, |_| {});
    settle(&mut store);
    store.send(A::Palette(P::Query("https://github.com/acme/ledger".into())), |_| {});
    store.send(A::Palette(P::Run), |_| {});
    settle(&mut store);
    match store.state().overlay() {
        Some(Overlay::ChannelAddRepo(sheet)) => {
            assert_eq!(sheet.channel, channel);
            assert_eq!(sheet.query, "acme/ledger");
        }
        other => panic!("the Add a GitHub repo sheet is open, not {other:?}"),
    }
}

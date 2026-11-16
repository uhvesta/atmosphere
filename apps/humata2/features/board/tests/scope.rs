//! BD-5–BD-8 with session boards (D216): what each board scope shows, where
//! New item goes, and what the session strip's Board panel lists.
use humata2_board::{session_panel, Board, BoardAction as A, BoardState, ChannelView};
use humata2_clients::{
    Clients, Workspace, fixture,
    model::{ChannelId, Item, ItemId, ItemKind, ItemStatus, Scope, Session, SessionId},
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

const CHANNEL: &str = "payments-rewrite";
const OTHER: &str = "infra";

fn sid(id: &str) -> SessionId {
    SessionId::new(id)
}

fn on_session(id: &str) -> Scope {
    Scope::Session {
        channel: ChannelId::new(CHANNEL),
        session: sid(id),
    }
}

fn item(id: &str, scope: Scope) -> Item {
    Item {
        id: ItemId::new(id),
        scope,
        kind: ItemKind::Task,
        title: id.to_owned(),
        status: ItemStatus::Todo,
        depends_on: Vec::new(),
        session: None,
        parent: None,
    }
}

/// The channel has `root` → `kid` → `grandkid` (forks by parent) and `solo`,
/// plus an archived `old`. Each has one item on its own board, beside a
/// global item, a channel item and another channel's item.
fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    let template = workspace
        .sessions
        .iter()
        .find(|s| s.channel == ChannelId::new(CHANNEL))
        .cloned()
        .expect("the design fixture has a payments session");
    let session = |id: &str, parent: Option<&str>, archived: bool| Session {
        id: sid(id),
        parent: parent.map(sid),
        archived,
        ..template.clone()
    };
    workspace.sessions = vec![
        session("root", None, false),
        session("kid", Some("root"), false),
        session("grandkid", Some("kid"), false),
        session("solo", None, false),
        session("old", None, true),
    ];
    workspace.items = vec![
        item("GLOBAL", Scope::Global),
        item("CHAN", Scope::Channel(ChannelId::new(CHANNEL))),
        item("ELSEWHERE", Scope::Channel(ChannelId::new(OTHER))),
        item("ROOT", on_session("root")),
        item("KID", on_session("kid")),
        item("GRANDKID", on_session("grandkid")),
        item("SOLO", on_session("solo")),
    ];
    workspace
}

fn state(scope: Scope, view: ChannelView) -> BoardState {
    let mut state = BoardState {
        workspace: Arc::new(workspace()),
        scope,
        ..Default::default()
    };
    state.view.channel_view = view;
    state
}

fn shown(state: &BoardState) -> Vec<String> {
    let mut ids: Vec<String> = state
        .column(ItemStatus::Todo)
        .iter()
        .map(|item| item.id.to_string())
        .collect();
    ids.sort();
    ids
}

fn ids(list: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = list.iter().map(|id| (*id).to_owned()).collect();
    ids.sort();
    ids
}

fn channel() -> Scope {
    Scope::Channel(ChannelId::new(CHANNEL))
}

#[test]
fn the_global_board_shows_global_and_channel_items_but_no_session_items() {
    let all = state(Scope::Global, ChannelView::Channel);
    assert_eq!(shown(&all), ids(&["GLOBAL", "CHAN", "ELSEWHERE"]));
    let mut only = all;
    only.view.global_only = true;
    assert_eq!(shown(&only), ids(&["GLOBAL"]));
}

#[test]
fn each_channel_board_scope_shows_its_own_set() {
    assert_eq!(shown(&state(channel(), ChannelView::Channel)), ids(&["CHAN"]));
    assert_eq!(
        shown(&state(channel(), ChannelView::WithSessions)),
        ids(&["CHAN", "ROOT", "KID", "GRANDKID", "SOLO"])
    );
    assert_eq!(
        shown(&state(channel(), ChannelView::SessionAndForks(sid("root")))),
        ids(&["ROOT", "KID", "GRANDKID"])
    );
    assert_eq!(
        shown(&state(channel(), ChannelView::SessionAndForks(sid("kid")))),
        ids(&["KID", "GRANDKID"])
    );
    assert_eq!(
        shown(&state(channel(), ChannelView::OneSession(sid("kid")))),
        ids(&["KID"])
    );
}

#[test]
fn a_saved_scope_whose_session_is_gone_or_archived_reads_as_the_channel_board() {
    for gone in ["nobody", "old"] {
        let state = state(channel(), ChannelView::OneSession(sid(gone)));
        assert_eq!(state.channel_view(), ChannelView::Channel, "{gone}");
        assert_eq!(shown(&state), ids(&["CHAN"]), "{gone}");
        // The save itself is kept, so the session coming back restores it.
        assert_eq!(state.view.channel_view, ChannelView::OneSession(sid(gone)));
    }
    // A session of another channel doesn't scope this channel's board.
    let elsewhere = state(Scope::Channel(ChannelId::new(OTHER)), ChannelView::OneSession(sid("kid")));
    assert_eq!(elsewhere.channel_view(), ChannelView::Channel);
}

#[test]
fn the_scope_picker_offers_forks_only_for_sessions_that_have_them() {
    let state = state(channel(), ChannelView::Channel);
    let offered = state.channel_views();
    assert!(offered.contains(&ChannelView::Channel));
    assert!(offered.contains(&ChannelView::WithSessions));
    assert!(offered.contains(&ChannelView::SessionAndForks(sid("root"))));
    assert!(offered.contains(&ChannelView::SessionAndForks(sid("kid"))));
    assert!(!offered.contains(&ChannelView::SessionAndForks(sid("grandkid"))));
    assert!(!offered.contains(&ChannelView::SessionAndForks(sid("solo"))));
    for live in ["root", "kid", "grandkid", "solo"] {
        assert!(offered.contains(&ChannelView::OneSession(sid(live))), "{live}");
    }
    assert!(!offered.contains(&ChannelView::OneSession(sid("old"))));
}

#[test]
fn picking_a_channel_view_saves_it_and_closes_the_menu() {
    let workspace = Arc::new(workspace());
    let (clients, _) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(state(channel(), ChannelView::Channel), Board::new(clients));
    store.send(A::ScopeMenu, |s| s.scope_menu = true);
    store.send(A::SetChannelView(ChannelView::OneSession(sid("solo"))), |s| {
        s.scope_menu = false;
        s.view.channel_view = ChannelView::OneSession(sid("solo"));
    });
    assert_eq!(shown(store.state()), ids(&["SOLO"]));
    store.finish();
}

fn create_in(view: ChannelView) -> Scope {
    let workspace = Arc::new(workspace());
    let (clients, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(state(channel(), view), Board::new(clients));
    store.send(A::StartNew, |s| s.new_title = Some(String::new()));
    store.send(A::NewTitle("fresh".into()), |s| s.new_title = Some("fresh".into()));
    store.send(A::CreateNew, |s| s.new_title = None);
    store.receive_matching("the create", |a| matches!(a, A::Done(Ok(()))), |_| {});
    store.finish();
    fakes
        .daemon
        .workspace()
        .items
        .into_iter()
        .find(|item| item.title == "fresh")
        .expect("the item was made")
        .scope
}

#[test]
fn new_item_goes_on_the_session_board_only_in_one_sessions_scope() {
    assert_eq!(create_in(ChannelView::OneSession(sid("kid"))), on_session("kid"));
    assert_eq!(create_in(ChannelView::WithSessions), channel());
    assert_eq!(create_in(ChannelView::SessionAndForks(sid("root"))), channel());
    // A gone session's scope reads as the channel board, so the item does too.
    assert_eq!(create_in(ChannelView::OneSession(sid("old"))), channel());
}

#[test]
fn the_session_panel_shows_its_channel_board_with_its_own_items_first() {
    let workspace = workspace();
    let kid = workspace.session(&sid("kid")).expect("kid").clone();
    let panel: Vec<String> = session_panel(&workspace, &kid)
        .iter()
        .map(|item| item.id.to_string())
        .collect();
    assert_eq!(panel, vec!["KID".to_owned(), "CHAN".to_owned()]);
}

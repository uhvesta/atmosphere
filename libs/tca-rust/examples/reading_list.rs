//! A reading list whose toolbar holds two small features: a sort menu and
//! a sync button. Each is a child reducer that runs on one field of the
//! list's state, and the list lifts the child's effects into its own
//! actions with `Effect::map`.
//!
//! Patterns: a child reducer on `&mut state.field`, and `Effect::map`.

use std::sync::Arc;
use std::time::Duration;

use tca::{Clock, Effect, Reducer, TestClock, TestStore};

// The sort menu. It has no effects, only state.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SortOrder {
    #[default]
    Added,
    Title,
    Author,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SortMenu {
    open: bool,
    order: SortOrder,
}

#[derive(Debug, PartialEq)]
enum SortAction {
    Toggle,
    Pick(SortOrder),
}

struct Sorting;

impl Reducer for Sorting {
    type State = SortMenu;
    type Action = SortAction;

    fn reduce(&self, menu: &mut SortMenu, action: SortAction) -> Effect<SortAction> {
        match action {
            SortAction::Toggle => menu.open = !menu.open,
            SortAction::Pick(order) => {
                menu.order = order;
                // Picking closes the menu, as a menu does.
                menu.open = false;
            }
        }
        Effect::none()
    }
}

// The sync button. Its whole state is one enum, and it owns the sync.

#[derive(Clone, Debug, Default, PartialEq)]
enum SyncStatus {
    #[default]
    Idle,
    Syncing,
    /// When it last finished, on the reducer's clock, for "synced 2m ago".
    Synced { at: Duration },
}

#[derive(Debug, PartialEq)]
enum SyncAction {
    Tapped,
    Finished,
}

/// Holds its clock, so a test moves time instead of waiting.
struct Syncing {
    clock: Arc<dyn Clock>,
}

impl Reducer for Syncing {
    type State = SyncStatus;
    type Action = SyncAction;

    fn reduce(&self, status: &mut SyncStatus, action: SyncAction) -> Effect<SyncAction> {
        match action {
            SyncAction::Tapped => {
                // A second tap while syncing starts nothing: the state says
                // a sync is under way, so the reducer needs no cancel id.
                if *status == SyncStatus::Syncing {
                    return Effect::none();
                }
                *status = SyncStatus::Syncing;
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(2)).await;
                    SyncAction::Finished
                })
            }
            SyncAction::Finished => {
                *status = SyncStatus::Synced {
                    at: self.clock.now(),
                };
                Effect::none()
            }
        }
    }
}

// The list, which holds both children's state as plain fields.

#[derive(Clone, Debug, PartialEq)]
struct Book {
    title: String,
    author: String,
}

fn book(title: &str, author: &str) -> Book {
    Book {
        title: title.into(),
        author: author.into(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ReadingList {
    /// In the order they were added.
    books: Vec<Book>,
    sort: SortMenu,
    sync: SyncStatus,
}

impl ReadingList {
    /// The titles as the view lists them. Derived from the state when
    /// asked rather than kept in it, so the order can never go stale.
    fn shown(&self) -> Vec<&str> {
        let mut books: Vec<&Book> = self.books.iter().collect();
        match self.sort.order {
            SortOrder::Added => {}
            SortOrder::Title => books.sort_by(|a, b| a.title.cmp(&b.title)),
            SortOrder::Author => books.sort_by(|a, b| a.author.cmp(&b.author)),
        }
        books.into_iter().map(|book| book.title.as_str()).collect()
    }
}

/// Each child's actions wrapped in one of the list's, so a child's effect
/// can be sent back to the list's store and find its way to the child.
#[derive(Debug, PartialEq)]
enum ListAction {
    Sort(SortAction),
    Sync(SyncAction),
    Add(Book),
}

/// The list's reducer holds its children's reducers, and with them their
/// dependencies.
struct Listing {
    sorting: Sorting,
    syncing: Syncing,
}

impl Reducer for Listing {
    type State = ReadingList;
    type Action = ListAction;

    fn reduce(&self, list: &mut ReadingList, action: ListAction) -> Effect<ListAction> {
        match action {
            // The child sees only its own field. `map` turns its effect's
            // actions into the list's, even though the sort menu has none
            // today, so giving it one later changes nothing here.
            ListAction::Sort(action) => self
                .sorting
                .reduce(&mut list.sort, action)
                .map(ListAction::Sort),
            ListAction::Sync(action) => self
                .syncing
                .reduce(&mut list.sync, action)
                .map(ListAction::Sync),
            ListAction::Add(book) => {
                list.books.push(book);
                Effect::none()
            }
        }
    }
}

fn reading_list() -> (TestStore<Listing>, TestClock) {
    let clock = TestClock::new();
    let state = ReadingList {
        books: vec![
            book("Middlemarch", "Eliot"),
            book("Bleak House", "Dickens"),
        ],
        ..ReadingList::default()
    };
    let reducer = Listing {
        sorting: Sorting,
        syncing: Syncing {
            clock: Arc::new(clock.clone()),
        },
    };
    (TestStore::new(state, reducer), clock)
}

#[test]
fn picking_an_order_closes_the_menu_and_sorts_what_is_shown() {
    let (mut store, _clock) = reading_list();
    store.send(ListAction::Sort(SortAction::Toggle), |s| s.sort.open = true);
    store.send(ListAction::Add(book("Anna Karenina", "Tolstoy")), |s| {
        s.books.push(book("Anna Karenina", "Tolstoy"));
    });
    store.send(ListAction::Sort(SortAction::Pick(SortOrder::Title)), |s| {
        s.sort = SortMenu {
            open: false,
            order: SortOrder::Title,
        };
    });
    assert_eq!(
        store.state().shown(),
        ["Anna Karenina", "Bleak House", "Middlemarch"]
    );
    store.send(ListAction::Sort(SortAction::Pick(SortOrder::Author)), |s| {
        s.sort.order = SortOrder::Author;
    });
    assert_eq!(
        store.state().shown(),
        ["Bleak House", "Middlemarch", "Anna Karenina"]
    );
}

#[test]
fn the_sync_buttons_effect_comes_back_as_the_lists_action() {
    let (mut store, clock) = reading_list();
    store.send(ListAction::Sync(SyncAction::Tapped), |s| {
        s.sync = SyncStatus::Syncing;
    });
    clock.advance(Duration::from_secs(2));
    // The child's `Finished`, wrapped by `map` on its way back.
    store.receive(ListAction::Sync(SyncAction::Finished), |s| {
        s.sync = SyncStatus::Synced {
            at: Duration::from_secs(2),
        };
    });
}

#[test]
fn tapping_sync_while_syncing_starts_no_second_sync() {
    let (mut store, clock) = reading_list();
    store.send(ListAction::Sync(SyncAction::Tapped), |s| {
        s.sync = SyncStatus::Syncing;
    });
    clock.advance(Duration::from_secs(1));
    store.send(ListAction::Sync(SyncAction::Tapped), |_| {});
    clock.advance(Duration::from_secs(1));
    store.receive(ListAction::Sync(SyncAction::Finished), |s| {
        s.sync = SyncStatus::Synced {
            at: Duration::from_secs(2),
        };
    });
    // Finishing checks no second `Finished` comes and nothing still runs.
    clock.advance(Duration::from_secs(5));
    store.finish();
}

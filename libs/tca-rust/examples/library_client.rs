//! A bookshelf that loads from a library client and marks books read. The
//! client is a trait with two implementations: the live one, which reads
//! the shelf from a folder on disk, and a fake that keeps books in memory
//! and records what it was asked. The shelf's reducer holds the client
//! itself and is built from `Dependencies`, so the app hands it the live
//! client and a test hands it the fake. Marking a book read shows at once
//! and is put back if the library refuses.
//!
//! Patterns: a client trait with a live and a fake implementation; the
//! client held on the reducer, built from `Dependencies`; tests on the
//! fake; an optimistic change rolled back on failure.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tca::{BoxFuture, Dependencies, DependencyError, Effect, Reducer, TestStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BookId(u32);

#[derive(Clone, Debug, PartialEq)]
struct Book {
    id: BookId,
    title: String,
    read: bool,
}

/// What the library can fail with. It's part of the actions, so it's
/// compared in tests and must be `Clone` and `PartialEq`: it keeps what
/// went wrong as data rather than holding the source error.
#[derive(Clone, Debug, PartialEq)]
enum LibraryError {
    /// The shelf couldn't be read at all.
    Unreachable { reason: String },
    NoSuchBook(BookId),
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable { reason } => write!(
                f,
                "couldn't reach the library: {reason}. Check you're online, then refresh."
            ),
            Self::NoSuchBook(id) => write!(
                f,
                "the library has no book {}: it may have been removed. Refresh the shelf.",
                id.0
            ),
        }
    }
}

impl std::error::Error for LibraryError {}

/// The client. Its calls return futures the reducer's effects await, so
/// the reducer never blocks.
trait Library: Send + Sync {
    fn books(&self) -> BoxFuture<Result<Vec<Book>, LibraryError>>;
    fn mark_read(&self, id: BookId) -> BoxFuture<Result<(), LibraryError>>;
}

/// The live client: the shelf is a file in a folder, a book a line, as
/// `id<TAB>title<TAB>read`. A real one would talk to a server; the shape
/// is the same.
struct ShelfOnDisk {
    folder: PathBuf,
}

impl Library for ShelfOnDisk {
    fn books(&self) -> BoxFuture<Result<Vec<Book>, LibraryError>> {
        let path = self.folder.join("shelf.tsv");
        Box::pin(async move {
            let text = std::fs::read_to_string(&path).map_err(|error| {
                LibraryError::Unreachable {
                    reason: format!("{}: {error}", path.display()),
                }
            })?;
            Ok(text
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split('\t');
                    let id = fields.next()?.parse().ok()?;
                    let title = fields.next()?.to_owned();
                    let read = fields.next() == Some("read");
                    Some(Book {
                        id: BookId(id),
                        title,
                        read,
                    })
                })
                .collect())
        })
    }

    fn mark_read(&self, id: BookId) -> BoxFuture<Result<(), LibraryError>> {
        // Left out here: this example is about the fake.
        Box::pin(async move { Err(LibraryError::NoSuchBook(id)) })
    }
}

/// The fake: books in memory, a switch to go offline, and a record of what
/// was marked, for tests to check.
#[derive(Default)]
struct FakeLibrary {
    books: Mutex<Vec<Book>>,
    offline: Mutex<bool>,
    marked: Mutex<Vec<BookId>>,
}

impl FakeLibrary {
    fn with(books: Vec<Book>) -> Arc<Self> {
        let fake = Self::default();
        *fake.books.lock().unwrap() = books;
        Arc::new(fake)
    }
}

/// So a test keeps one handle to inspect and gives the reducer another.
impl Library for Arc<FakeLibrary> {
    fn books(&self) -> BoxFuture<Result<Vec<Book>, LibraryError>> {
        let fake = self.clone();
        Box::pin(async move {
            if *fake.offline.lock().unwrap() {
                return Err(LibraryError::Unreachable {
                    reason: "offline".into(),
                });
            }
            Ok(fake.books.lock().unwrap().clone())
        })
    }

    fn mark_read(&self, id: BookId) -> BoxFuture<Result<(), LibraryError>> {
        let fake = self.clone();
        Box::pin(async move {
            let mut books = fake.books.lock().unwrap();
            let Some(book) = books.iter_mut().find(|book| book.id == id) else {
                return Err(LibraryError::NoSuchBook(id));
            };
            book.read = true;
            fake.marked.lock().unwrap().push(id);
            Ok(())
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Shelf {
    books: Vec<Book>,
    loading: bool,
    /// The last thing that went wrong, shown until the next try.
    error: Option<LibraryError>,
}

#[derive(Debug, PartialEq)]
enum ShelfAction {
    Refresh,
    Loaded(Result<Vec<Book>, LibraryError>),
    MarkRead(BookId),
    Marked(BookId, Result<(), LibraryError>),
}

/// The shelf's reducer, holding its client. Asked for as the trait object,
/// the type the app and the tests both give it under.
struct Shelving {
    library: Arc<dyn Library>,
}

impl Shelving {
    /// Takes what it needs from the dependencies when it's made, so a
    /// missing client fails here, at startup, not when the person first
    /// taps refresh.
    fn new(dependencies: &Dependencies) -> Result<Self, DependencyError> {
        Ok(Self {
            library: dependencies.get()?,
        })
    }
}

impl Reducer for Shelving {
    type State = Shelf;
    type Action = ShelfAction;

    fn reduce(&self, shelf: &mut Shelf, action: ShelfAction) -> Effect<ShelfAction> {
        match action {
            ShelfAction::Refresh => {
                shelf.loading = true;
                shelf.error = None;
                let books = self.library.books();
                Effect::future(async move { ShelfAction::Loaded(books.await) })
            }
            ShelfAction::Loaded(Ok(books)) => {
                shelf.loading = false;
                shelf.books = books;
                Effect::none()
            }
            ShelfAction::Loaded(Err(error)) => {
                // What was on the shelf stays: better stale than empty.
                shelf.loading = false;
                shelf.error = Some(error);
                Effect::none()
            }
            ShelfAction::MarkRead(id) => {
                let Some(book) = shelf.books.iter_mut().find(|book| book.id == id) else {
                    return Effect::none();
                };
                // Shown at once; put back below if the library refuses.
                book.read = true;
                let marked = self.library.mark_read(id);
                Effect::future(async move { ShelfAction::Marked(id, marked.await) })
            }
            ShelfAction::Marked(_, Ok(())) => Effect::none(),
            ShelfAction::Marked(id, Err(error)) => {
                if let Some(book) = shelf.books.iter_mut().find(|book| book.id == id) {
                    book.read = false;
                }
                shelf.error = Some(error);
                Effect::none()
            }
        }
    }
}

/// What the app builds once, with its live clients.
fn live() -> Dependencies {
    Dependencies::new().with::<Arc<dyn Library>>(Arc::new(ShelfOnDisk {
        folder: PathBuf::from("/nonexistent/library"),
    }))
}

fn book(id: u32, title: &str) -> Book {
    Book {
        id: BookId(id),
        title: title.into(),
        read: false,
    }
}

/// A shelf built the way the app builds it, with the library swapped for
/// the fake.
fn shelf(fake: &Arc<FakeLibrary>) -> Result<TestStore<Shelving>, DependencyError> {
    let dependencies = live().with::<Arc<dyn Library>>(Arc::new(fake.clone()));
    Ok(TestStore::new(Shelf::default(), Shelving::new(&dependencies)?))
}

#[test]
fn refreshing_loads_the_shelf_from_the_fake_never_the_live_client() -> Result<(), DependencyError> {
    let fake = FakeLibrary::with(vec![book(1, "Persuasion"), book(2, "Emma")]);
    let mut store = shelf(&fake)?;
    store.send(ShelfAction::Refresh, |s| s.loading = true);
    // Had the live client been used, this would be an Unreachable error.
    store.receive(
        ShelfAction::Loaded(Ok(vec![book(1, "Persuasion"), book(2, "Emma")])),
        |s| {
            s.loading = false;
            s.books = vec![book(1, "Persuasion"), book(2, "Emma")];
        },
    );
    Ok(())
}

#[test]
fn a_failed_refresh_keeps_the_books_already_shown() -> Result<(), DependencyError> {
    let fake = FakeLibrary::with(vec![book(1, "Persuasion")]);
    let mut store = shelf(&fake)?;
    store.send(ShelfAction::Refresh, |s| s.loading = true);
    store.receive(ShelfAction::Loaded(Ok(vec![book(1, "Persuasion")])), |s| {
        s.loading = false;
        s.books = vec![book(1, "Persuasion")];
    });
    *fake.offline.lock().unwrap() = true;
    store.send(ShelfAction::Refresh, |s| s.loading = true);
    let offline = LibraryError::Unreachable {
        reason: "offline".into(),
    };
    store.receive(ShelfAction::Loaded(Err(offline.clone())), |s| {
        s.loading = false;
        s.error = Some(offline);
    });
    assert_eq!(store.state().books, [book(1, "Persuasion")]);
    Ok(())
}

#[test]
fn marking_a_book_read_shows_at_once_and_reaches_the_library() -> Result<(), DependencyError> {
    let fake = FakeLibrary::with(vec![book(1, "Persuasion")]);
    let mut store = shelf(&fake)?;
    store.send(ShelfAction::Refresh, |s| s.loading = true);
    store.receive(ShelfAction::Loaded(Ok(vec![book(1, "Persuasion")])), |s| {
        s.loading = false;
        s.books = vec![book(1, "Persuasion")];
    });
    store.send(ShelfAction::MarkRead(BookId(1)), |s| s.books[0].read = true);
    store.receive(ShelfAction::Marked(BookId(1), Ok(())), |_| {});
    assert_eq!(*fake.marked.lock().unwrap(), [BookId(1)]);
    Ok(())
}

#[test]
fn a_book_the_library_refuses_to_mark_is_put_back() -> Result<(), DependencyError> {
    let fake = FakeLibrary::with(vec![book(1, "Persuasion"), book(2, "Emma")]);
    let mut store = shelf(&fake)?;
    store.send(ShelfAction::Refresh, |s| s.loading = true);
    store.receive(
        ShelfAction::Loaded(Ok(vec![book(1, "Persuasion"), book(2, "Emma")])),
        |s| {
            s.loading = false;
            s.books = vec![book(1, "Persuasion"), book(2, "Emma")];
        },
    );
    // Removed from the library since the shelf loaded.
    fake.books.lock().unwrap().retain(|book| book.id != BookId(2));
    store.send(ShelfAction::MarkRead(BookId(2)), |s| s.books[1].read = true);
    store.receive(
        ShelfAction::Marked(BookId(2), Err(LibraryError::NoSuchBook(BookId(2)))),
        |s| {
            s.books[1].read = false;
            s.error = Some(LibraryError::NoSuchBook(BookId(2)));
        },
    );
    assert!(fake.marked.lock().unwrap().is_empty());
    Ok(())
}

#[test]
fn a_shelf_without_a_library_fails_when_it_is_made() {
    let made = Shelving::new(&Dependencies::new());
    assert!(matches!(made, Err(DependencyError::Missing { .. })));
}

#[test]
fn the_live_client_reports_a_missing_shelf_as_unreachable() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let live: Arc<dyn Library> = live().get().unwrap();
    let books = runtime.block_on(live.books());
    assert!(matches!(books, Err(LibraryError::Unreachable { .. })));
}

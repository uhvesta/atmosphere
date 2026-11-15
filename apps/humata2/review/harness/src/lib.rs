//! Fixtures for the review pane's GUI tests: a fake agent terminal, a fake
//! store, reviews to show, and a test window to show them in.
//!
//! The terminal and the store write to one shared log, in the order things
//! happened, so a test can stop the app at any point of it and start again
//! from what was saved by then. The daemon's review history (reviewed
//! snapshots, changes since one) and the review chat's MCP tool calls are
//! faked here too.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Focusable as _, Point, TestAppContext,
    WindowBounds, WindowOptions, px, size,
};
use humata2_review_view::model::{
    AgentAction, Base, ChangeKind, Content, DiffLine, FileChange, FileKey, HighlightAt, Hunk,
    ImageBlob, ImageChange, LineRange, NoteAt, PreparedGroup, PreparedNote, PreparedReview,
    PullRequest, Review, ReviewSnapshot, Reviewed, SessionId, Side, Target, TextDiff, Tool, Typed,
};
use humata2_review_view::{
    CommentSink, DeliveryError, HistoryError, HistoryFuture, PaneEvent, ReviewHistory, ReviewPane,
    ReviewStore, StoreError, TypeFuture, TypeRequest,
};
use image::{ImageFormat, Rgba, RgbaImage};

/// Something that happened outside the pane, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A snapshot reached the disk (as JSON, the way a real store keeps it).
    Saved { session: SessionId, json: String },
    /// The terminal typed a message.
    Typed { key: String, text: String },
}

#[derive(Default)]
struct World {
    log: Vec<Event>,
    /// The newest saved snapshot of each session, as JSON.
    disk: HashMap<SessionId, (u64, String)>,
    /// Keys the terminal has typed; it types each once.
    typed: HashSet<String>,
    /// What the terminal answers instead of typing, while set.
    refuse: Option<DeliveryError>,
    /// Saves fail while set.
    disk_full: bool,
    /// Sessions whose saved review doesn't read.
    corrupt: HashSet<SessionId>,
    /// Times the terminal was asked to type, counting repeats.
    asked: usize,
    /// Reviewed snapshots, oldest first, with the review each one saw.
    reviewed: Vec<(Reviewed, Review)>,
    /// What the session shows now, which Mark reviewed records.
    now: Option<Review>,
}

/// The agent's terminal and the disk, faked together.
#[derive(Clone, Default)]
pub struct Fakes {
    world: Arc<Mutex<World>>,
}

impl Fakes {
    pub fn new() -> Self {
        Self::default()
    }

    fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn sink(&self) -> Arc<dyn CommentSink> {
        Arc::new(self.clone())
    }

    pub fn store(&self) -> Arc<dyn ReviewStore> {
        Arc::new(self.clone())
    }

    pub fn history(&self) -> Arc<dyn ReviewHistory> {
        Arc::new(self.clone())
    }

    /// What the session shows now: what Mark reviewed records, and what
    /// changes against a review are compared with.
    pub fn set_now(&self, review: Review) {
        self.world().now = Some(review);
    }

    /// Adds a reviewed snapshot that saw `review`, as if marked earlier.
    pub fn add_reviewed(&self, when: &str, review: Review) -> u32 {
        let mut world = self.world();
        let number = world.reviewed.len() as u32 + 1;
        world.reviewed.push((
            Reviewed {
                number,
                when: when.to_owned(),
            },
            review,
        ));
        number
    }

    /// Everything that happened, in order.
    pub fn log(&self) -> Vec<Event> {
        self.world().log.clone()
    }

    /// The messages the agent got, in order.
    pub fn typed(&self) -> Vec<String> {
        self.world()
            .log
            .iter()
            .filter_map(|event| match event {
                Event::Typed { text, .. } => Some(text.clone()),
                Event::Saved { .. } => None,
            })
            .collect()
    }

    /// Times the terminal was asked to type anything, counting repeats.
    pub fn asked(&self) -> usize {
        self.world().asked
    }

    /// The newest saved snapshot of `session`.
    pub fn saved(&self, session: &SessionId) -> Option<ReviewSnapshot> {
        let world = self.world();
        let (_, json) = world.disk.get(session)?;
        serde_json::from_str(json).ok()
    }

    /// The terminal answers `error` instead of typing until cleared.
    pub fn refuse(&self, error: Option<DeliveryError>) {
        self.world().refuse = error;
    }

    pub fn set_disk_full(&self, full: bool) {
        self.world().disk_full = full;
    }

    pub fn corrupt(&self, session: &SessionId) {
        self.world().corrupt.insert(session.clone());
    }

    /// The world as it was after the first `events` events: what an app
    /// killed at that instant left on disk and in the terminal.
    pub fn killed_at(&self, events: usize) -> Fakes {
        let world = self.world();
        let mut next = World::default();
        for event in world.log.iter().take(events) {
            next.log.push(event.clone());
            match event {
                Event::Saved { session, json } => {
                    let revision = serde_json::from_str::<ReviewSnapshot>(json)
                        .map_or(0, |snapshot| snapshot.revision);
                    next.disk.insert(session.clone(), (revision, json.clone()));
                }
                Event::Typed { key, .. } => {
                    next.typed.insert(key.clone());
                }
            }
        }
        next.reviewed = world.reviewed.clone();
        next.now = world.now.clone();
        Fakes {
            world: Arc::new(Mutex::new(next)),
        }
    }
}

impl CommentSink for Fakes {
    fn type_text(&self, request: TypeRequest) -> TypeFuture {
        let mut world = self.world();
        world.asked += 1;
        let result = if let Some(error) = world.refuse.clone() {
            Err(error)
        } else if world.typed.insert(request.key.clone()) {
            world.log.push(Event::Typed {
                key: request.key,
                text: request.text,
            });
            Ok(Typed::Typed)
        } else {
            Ok(Typed::AlreadyTyped)
        };
        Box::pin(async move { result })
    }
}

impl ReviewStore for Fakes {
    fn save(&self, snapshot: &ReviewSnapshot) -> Result<(), StoreError> {
        let mut world = self.world();
        let session = snapshot.session().clone();
        if world.disk_full {
            return Err(StoreError::Write {
                session,
                source: std::io::Error::new(std::io::ErrorKind::StorageFull, "no space left on device"),
            });
        }
        if world
            .disk
            .get(&session)
            .is_some_and(|(revision, _)| *revision >= snapshot.revision)
        {
            return Ok(());
        }
        let json = serde_json::to_string(snapshot).map_err(|error| StoreError::Write {
            session: session.clone(),
            source: std::io::Error::other(error),
        })?;
        world.disk.insert(session.clone(), (snapshot.revision, json.clone()));
        world.log.push(Event::Saved { session, json });
        Ok(())
    }

    fn load(&self, session: &SessionId) -> Result<Option<ReviewSnapshot>, StoreError> {
        let world = self.world();
        if world.corrupt.contains(session) {
            return Err(StoreError::Corrupt {
                session: session.clone(),
                reason: "it isn't valid JSON".into(),
            });
        }
        match world.disk.get(session) {
            None => Ok(None),
            Some((_, json)) => serde_json::from_str(json).map(Some).map_err(|error| StoreError::Corrupt {
                session: session.clone(),
                reason: error.to_string(),
            }),
        }
    }
}

/// The fake history compares by file: a file whose blob is the same in the
/// reviewed snapshot and now isn't listed, the way the daemon's
/// `ReviewService.Changes` leaves out what the user already saw (RV-30).
impl ReviewHistory for Fakes {
    fn reviewed(&self, _session: &SessionId) -> HistoryFuture<Vec<Reviewed>> {
        let list: Vec<Reviewed> = self.world().reviewed.iter().rev().map(|(r, _)| r.clone()).collect();
        Box::pin(async move { Ok(list) })
    }

    fn mark_reviewed(&self, session: &SessionId) -> HistoryFuture<Reviewed> {
        let mut world = self.world();
        let result = match world.now.clone() {
            None => Err(HistoryError::Mark {
                session: session.clone(),
                reason: "the session's machine is offline".into(),
            }),
            Some(now) => {
                let reviewed = Reviewed {
                    number: world.reviewed.len() as u32 + 1,
                    when: "Today 14:02".into(),
                };
                world.reviewed.push((reviewed.clone(), now));
                Ok(reviewed)
            }
        };
        Box::pin(async move { result })
    }

    fn changes(&self, session: &SessionId, base: Base) -> HistoryFuture<Review> {
        let world = self.world();
        let now = world.now.clone().unwrap_or_else(|| Review::new(Vec::new()));
        let result = match base {
            Base::Session => Ok(now),
            Base::Since(number) => match world.reviewed.iter().find(|(r, _)| r.number == number) {
                None => Err(HistoryError::Gone {
                    session: session.clone(),
                    number,
                }),
                Some((_, then)) => {
                    let seen: HashSet<(FileKey, String)> =
                        then.files().iter().map(|f| (f.key.clone(), f.blob.clone())).collect();
                    let files = now
                        .files()
                        .iter()
                        .filter(|f| !seen.contains(&(f.key.clone(), f.blob.clone())))
                        .cloned()
                        .collect();
                    Ok(Review::new(files).with_pulls(now.pulls().to_vec()))
                }
            },
        };
        Box::pin(async move { result })
    }
}

/// Everything the pane told its window, in order.
pub fn record_events(cx: &mut TestAppContext, pane: &Entity<ReviewPane>) -> Rc<RefCell<Vec<PaneEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(pane, move |_, event: &PaneEvent, _| sink.borrow_mut().push(event.clone()))
            .detach();
    });
    events
}

// Reviews.

/// A PNG of `width` by `height`, coloured by `paint`.
pub fn png(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 4]) -> Arc<[u8]> {
    let image = RgbaImage::from_fn(width, height, |x, y| Rgba(paint(x, y)));
    let mut bytes = std::io::Cursor::new(Vec::new());
    if image.write_to(&mut bytes, ImageFormat::Png).is_err() {
        return Arc::from(Vec::new());
    }
    Arc::from(bytes.into_inner())
}

fn blob(sha: &str, bytes: Arc<[u8]>) -> ImageBlob {
    ImageBlob {
        sha: sha.to_owned(),
        bytes,
    }
}

/// A logo: a blue square on grey, with the after side's square moved and a
/// red bar added, so every image mode has something to show.
pub fn logo(after: bool) -> Arc<[u8]> {
    png(96, 64, move |x, y| {
        let (left, top) = if after { (40, 20) } else { (24, 16) };
        if after && y >= 52 && x < 60 {
            [220, 60, 60, 255]
        } else if (left..left + 32).contains(&x) && (top..top + 24).contains(&y) {
            [60, 120, 230, 255]
        } else {
            [200, 200, 200, 255]
        }
    })
}

/// A hunk of `count` lines starting at `start` on both sides: context, with
/// every fourth line changed (removed and added).
pub fn hunk(start: u32, count: u32, label: &str) -> Hunk {
    let mut lines = Vec::new();
    let (mut old, mut new) = (start, start);
    for index in 0..count {
        if index % 4 == 2 {
            lines.push(DiffLine::removed(old, format!("    let {label}_{index} = old();")));
            lines.push(DiffLine::added(new, format!("    let {label}_{index} = new();")));
        } else {
            lines.push(DiffLine::context(old, new, format!("    step_{label}({index});")));
        }
        old += 1;
        new += 1;
    }
    Hunk {
        header: format!("@@ -{start},{count} +{start},{count} @@ fn {label}()"),
        lines,
    }
}

/// The new side's text of [`hunk`]s, for anchoring.
fn text_of(hunks: &[Hunk]) -> TextDiff {
    TextDiff {
        hunks: hunks.to_vec(),
        old_text: None,
        new_text: None,
    }
}

pub const README_OLD: &str = "# Humata\n\nA terminal for agents.\n\n## Install\n\nRun the installer.\n\n- macOS\n- Linux\n";
pub const README_NEW: &str = "# Humata\n\nA terminal for agents.\n\n## Install\n\nRun the installer, then sign in.\n\n- macOS\n- Linux\n- Windows, later\n";

/// The markdown diff between [`README_OLD`] and [`README_NEW`], with both
/// sides whole, as the daemon sends it for markdown.
pub fn readme() -> FileChange {
    let lines = vec![
        DiffLine::context(5, 5, "## Install"),
        DiffLine::context(6, 6, ""),
        DiffLine::removed(7, "Run the installer."),
        DiffLine::added(7, "Run the installer, then sign in."),
        DiffLine::context(8, 8, ""),
        DiffLine::context(9, 9, "- macOS"),
        DiffLine::context(10, 10, "- Linux"),
        DiffLine::added(11, "- Windows, later"),
    ];
    FileChange {
        key: FileKey::new("app", "README.md"),
        change: ChangeKind::Modified,
        base: "4f1c2e9a0b".into(),
        blob: "readme-2".into(),
        content: Content::Markdown(TextDiff {
            hunks: vec![Hunk {
                header: "@@ -5,6 +5,7 @@".into(),
                lines,
            }],
            old_text: Some(Arc::from(README_OLD)),
            new_text: Some(Arc::from(README_NEW)),
        }),
    }
}

/// `src/lib.rs` with one small hunk: what most flows comment on.
pub fn lib_rs(blob: &str) -> FileChange {
    FileChange {
        key: FileKey::new("app", "src/lib.rs"),
        change: ChangeKind::Modified,
        base: "4f1c2e9a0b".into(),
        blob: blob.into(),
        content: Content::Text(text_of(&[Hunk {
            header: "@@ -1,6 +1,7 @@ pub fn start()".into(),
            lines: vec![
                DiffLine::context(1, 1, "pub fn start() {"),
                DiffLine::context(2, 2, "    let config = load();"),
                DiffLine::removed(3, "    run(config);"),
                DiffLine::added(3, "    let server = Server::new(config);"),
                DiffLine::added(4, "    server.run();"),
                DiffLine::context(4, 5, "}"),
            ],
        }])),
    }
}

/// One session's review across two repos: code, markdown, images that were
/// changed, added and broken, and a binary.
pub fn small_review() -> Review {
    let image = |path: &str, change: ChangeKind, old: Option<ImageBlob>, new: Option<ImageBlob>| FileChange {
        key: FileKey::new("app", path),
        change,
        base: "4f1c2e9a0b".into(),
        blob: format!("{path}-blob"),
        content: Content::Image(ImageChange { old, new }),
    };
    Review::new(vec![
        lib_rs("lib-2"),
        FileChange {
            key: FileKey::new("app", "src/server.rs"),
            change: ChangeKind::Added,
            base: "4f1c2e9a0b".into(),
            blob: "server-1".into(),
            content: Content::Text(text_of(&[hunk(1, 8, "serve")])),
        },
        readme(),
        image(
            "assets/logo.png",
            ChangeKind::Modified,
            Some(blob("logo-1", logo(false))),
            Some(blob("logo-2", logo(true))),
        ),
        image("assets/icon.png", ChangeKind::Added, None, Some(blob("icon-1", png(32, 32, |x, _| [x as u8 * 8, 90, 160, 255])))),
        image(
            "assets/broken.png",
            ChangeKind::Modified,
            Some(blob("broken-1", logo(false))),
            Some(blob("broken-2", Arc::from(&b"\x89PNG\r\n\x1a\nnot really"[..]))),
        ),
        FileChange {
            key: FileKey::new("app", "assets/font.woff2"),
            change: ChangeKind::Modified,
            base: "4f1c2e9a0b".into(),
            blob: "font-2".into(),
            content: Content::Binary {
                old_size: Some(48_120),
                new_size: Some(51_004),
            },
        },
        FileChange {
            key: FileKey::new("docs", "guide.txt"),
            change: ChangeKind::Modified,
            base: "77aa01c3".into(),
            blob: "guide-2".into(),
            content: Content::Text(text_of(&[hunk(10, 5, "guide")])),
        },
    ])
}

/// `src/lib.rs` in `app` and `guide.txt` in `docs`: two repos, small
/// enough that every line is on screen.
pub fn two_repos() -> Review {
    Review::new(vec![
        lib_rs("lib-2"),
        FileChange {
            key: FileKey::new("docs", "guide.txt"),
            change: ChangeKind::Modified,
            base: "77aa01c3".into(),
            blob: "guide-2".into(),
            content: Content::Text(text_of(&[hunk(10, 5, "guide")])),
        },
    ])
}

/// A review of `files` files across three repos, `lines` diff lines in all,
/// with an image every 25th file and markdown every 10th.
pub fn large_review(files: usize, lines: usize) -> Review {
    let image = |index: usize| index % 25 == 7;
    let markdown = |index: usize| index % 10 == 3;
    let text_files = (0..files).filter(|&i| !image(i) && !markdown(i)).count();
    let per_file = (lines / text_files.max(1)).max(1) as u32;
    let mut out = Vec::with_capacity(files);
    for index in 0..files {
        let repo = ["app", "lib", "docs"][index % 3];
        let base = "9d8c7b6a5f".to_owned();
        let change = if image(index) {
            let sha = |side: u8| format!("img-{index}-{side}");
            FileChange {
                key: FileKey::new(repo, format!("assets/img_{index:03}.png")),
                change: ChangeKind::Modified,
                base,
                blob: sha(2),
                content: Content::Image(ImageChange {
                    old: Some(blob(&sha(1), logo(false))),
                    new: Some(blob(&sha(2), logo(true))),
                }),
            }
        } else if markdown(index) {
            let mut readme = readme();
            readme.key = FileKey::new(repo, format!("notes/note_{index:03}.md"));
            readme.blob = format!("note-{index}");
            readme
        } else {
            let mut hunks = Vec::new();
            let mut left = per_file;
            let mut start = 1;
            while left > 0 {
                // Each changed line adds one row, so a hunk of n shows ~n*1.25.
                let count = left.min(40);
                hunks.push(hunk(start, count * 4 / 5, &format!("f{index}")));
                start += count + 20;
                left -= count;
            }
            FileChange {
                key: FileKey::new(repo, format!("src/module_{index:03}.rs")),
                change: ChangeKind::Modified,
                base,
                blob: format!("blob-{index}"),
                content: Content::Text(text_of(&hunks)),
            }
        };
        out.push(change);
    }
    Review::new(out)
}

/// A PR review: `refund.rs` and `ledger.rs` of PR 7, read with the review
/// chat (PR-22).
pub fn pr_review() -> Review {
    let file = |path: &str, blob: &str, label: &str, count: u32| FileChange {
        key: FileKey::new("app", path),
        change: ChangeKind::Modified,
        base: "4f1c2e9a0b".into(),
        blob: blob.into(),
        content: Content::Text(text_of(&[hunk(1, count, label)])),
    };
    Review::new(vec![
        file("src/refund.rs", "refund-2", "refund", 12),
        file("src/ledger.rs", "ledger-2", "ledger", 8),
        file("src/util.rs", "util-2", "util", 4),
    ])
    .with_pulls(vec![PullRequest {
        repo: "app".into(),
        number: 7,
        title: "Refunds go back to the card".into(),
    }])
}

/// The review chat's agent's tool calls, as the MCP relay hands them over.
pub fn action(id: &str, tool: Tool) -> AgentAction {
    AgentAction {
        id: id.into(),
        agent: "Claude Code".into(),
        tool,
    }
}

/// `review_focus` on new-side lines `first`–`last` of `path`, selected and
/// highlighted with `label`.
pub fn review_focus(id: &str, path: &str, first: u32, last: u32, label: &str) -> AgentAction {
    let target = Target::lines(FileKey::new("app", path), Side::New, LineRange::new(first, last));
    action(
        id,
        Tool::Focus {
            target: target.clone(),
            select: true,
            highlights: vec![HighlightAt {
                target,
                label: label.into(),
            }],
        },
    )
}

/// `review_note` on new-side line `line` of `path`.
pub fn review_note(id: &str, path: &str, line: u32, text: &str) -> AgentAction {
    action(
        id,
        Tool::Note {
            target: Target::lines(FileKey::new("app", path), Side::New, LineRange::line(line)),
            text: text.into(),
        },
    )
}

/// An agent-prepared review of [`pr_review`]: ledger first, then refunds,
/// with a note on the refund's first change.
pub fn prepared() -> PreparedReview {
    PreparedReview {
        id: "view-1".into(),
        title: "Money moves first".into(),
        agent: "Claude Code".into(),
        summary: "The ledger writes, then the refund that calls them.".into(),
        groups: vec![
            PreparedGroup {
                title: "Ledger".into(),
                files: vec![FileKey::new("app", "src/ledger.rs")],
            },
            PreparedGroup {
                title: "Refunds".into(),
                files: vec![FileKey::new("app", "src/refund.rs")],
            },
        ],
        notes: vec![PreparedNote {
            file: FileKey::new("app", "src/refund.rs"),
            at: Some(NoteAt {
                side: Side::New,
                lines: LineRange::line(3),
                quoted: Vec::new(),
            }),
            text: "This is the new path.".into(),
            outdated: false,
        }],
    }
}

// Windows.

pub fn options(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(width), px(height)),
        })),
        focus: false,
        show: false,
        ..Default::default()
    }
}

/// Opens a pane on `fakes` in a test window that is never shown, focused.
pub fn open_pane(cx: &mut TestAppContext, fakes: &Fakes, width: f32, height: f32) -> (AnyWindowHandle, Entity<ReviewPane>) {
    cx.update(gpui_kit::init);
    let sink = fakes.sink();
    let store = fakes.store();
    let history = fakes.history();
    let (window, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(options(width, height), cx, |_, cx| {
                cx.new(|cx| ReviewPane::new(sink, store, cx).with_history(history))
            })
        })
        .unwrap_or_else(|error| panic!("the test window opens: {error}"));
    focus(cx, window, &pane);
    (window, pane)
}

/// Gives the pane keyboard focus.
pub fn focus(cx: &mut TestAppContext, window: AnyWindowHandle, pane: &Entity<ReviewPane>) {
    let pane = pane.clone();
    cx.update_window(window, |_, window, cx| {
        let handle = pane.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    })
    .unwrap_or_else(|error| panic!("the window is open: {error}"));
    cx.run_until_parked();
}

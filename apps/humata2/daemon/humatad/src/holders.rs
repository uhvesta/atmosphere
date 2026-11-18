//! One long-lived connection to each tab's holder, shared by every call
//! that only needs to know where the tab stands or to type into it. A
//! connection opens on first use and stays until the holder hangs up or
//! the tab is closed, so listing tabs or typing costs a message on an open
//! socket, not a connect and a welcome per tab per call. Attaches still
//! open their own: each streams from an offset of its own.
//!
//! The link also keeps the tab's authoritative screen (D271): it parses
//! everything the holder prints, answers the program's terminal queries
//! with nobody attached, and hands attaches checkpoints and size changes.
//! The holder's protocol has no control-only connection, so the link hears
//! all of its tab's output. That costs a read per chunk the shell prints,
//! and nothing while it's quiet: no timers run on a link.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_holder::{Attachment, Control, Event, Events, HolderError, Welcome};
use humata2_store::{Store, StoreError, TabReplies};
use humata2_vt_authority::{Authority, AuthorityError, Config, ReplyRecord, Resized, Snapshot, Step};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

/// Rows of history the screen keeps.
const SCROLLBACK_LINES: usize = 10_000;

/// The most a checkpoint carries, envelope included: what the app's terminal
/// installs (terminal_wire's `MAX_RAW_CHECKPOINT_BYTES`). One bigger than a
/// gRPC message goes in pieces to a client that lists a codec; one that
/// lists none is refused it.
const CHECKPOINT_LIMIT: usize = 16 << 20;

/// Size changes waiting for a slow attach before it is told to resync.
const CHANGES_QUEUE: usize = 16;

/// Size of a screen the holder didn't say.
const DEFAULT_SIZE: (u16, u16) = (80, 24);

/// Where a tab stands, as its link last heard. It can trail output still
/// on its way from the holder, by as much as is in flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Offsets {
    /// The oldest byte the holder still keeps, or a later offset when
    /// what it kept had gaps; never one it no longer has, unless it was
    /// started with a smaller scrollback than this daemon's.
    pub(crate) earliest: u64,
    pub(crate) end: u64,
    pub(crate) exited: Option<i32>,
    /// The holder hung up, so a new link is needed.
    closed: bool,
}

/// Why typing through a link failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TypeFailed {
    #[error("the connection to the tab's holder failed")]
    Lost(#[source] HolderError),
    #[error("the tab's holder refused the typing: {message}")]
    Refused { message: String },
    #[error("the tab's shell exited before the text reached it")]
    Exited,
}

/// Why a link couldn't give or change its tab's screen.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ScreenFailed {
    #[error(
        "humatad2 has no screen for this tab right now, so it can't draw it. Close the tab's view and open it again; if it keeps failing, restart humatad2 (the tab's shell keeps running in its holder)."
    )]
    Unavailable,
    #[error(
        "humatad2 couldn't read or resize this tab's screen. The shell is unaffected: close the tab's view and open it again."
    )]
    Engine(#[source] AuthorityError),
    #[error("a screen task of humatad2 stopped before it finished. Open the tab's view again.")]
    Stopped(#[source] tokio::task::JoinError),
}

/// The tab's screen as a link keeps it.
struct Screen {
    authority: Authority,
    /// A cell's size in pixels, which the engine needs to be nonzero.
    cell: (u32, u32),
}

/// The links, by tab id.
pub(crate) struct Holders {
    links: Mutex<HashMap<String, Arc<Link>>>,
    /// One turn per tab to start a link, so two calls that find none don't
    /// each start a screen: both would answer the program's queries.
    starting: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The scrollback each holder was started with, to work out what it
    /// drops as output arrives.
    scrollback_bytes: u64,
    /// Where each screen keeps its reply record.
    store: Arc<Store>,
    /// Changes after each write the store commits, so a link waiting for
    /// its tab to be saved with a new holder is told, not left to look
    /// again (D672).
    commits: watch::Receiver<u64>,
}

impl Holders {
    pub(crate) fn new(scrollback_bytes: usize, store: Arc<Store>) -> Self {
        let (committed, commits) = watch::channel(0u64);
        // Kept by the store for as long as it lives; a send never fails, so
        // a write with no link waiting costs a counter bump (TR-7).
        store.on_commit(move || committed.send_modify(|count| *count = count.wrapping_add(1)));
        Self {
            links: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            scrollback_bytes: u64::try_from(scrollback_bytes.max(1)).unwrap_or(u64::MAX),
            store,
            commits,
        }
    }

    /// The open link to `tab`'s holder at `socket`, connecting if there is
    /// none or the last one was hung up on.
    pub(crate) async fn link(&self, tab: &str, socket: &Path) -> Result<Arc<Link>, HolderError> {
        if let Some(link) = self.open_link(tab, socket) {
            return Ok(link);
        }
        let turn = lock(&self.starting).entry(tab.to_owned()).or_default().clone();
        let _turn = turn.lock().await;
        // Another call may have started it while this one waited.
        if let Some(link) = self.open_link(tab, socket) {
            return Ok(link);
        }
        // From the start, so the screen can be rebuilt from what the
        // holder kept.
        let attachment = humata2_holder::attach(socket, 0).await?;
        let saving = Saving {
            store: self.store.clone(),
            tab: tab.to_owned(),
        };
        let link = Arc::new(
            Link::start(attachment, socket, self.scrollback_bytes, saving, self.commits.clone()).await?,
        );
        lock(&self.links).insert(tab.to_owned(), link.clone());
        Ok(link)
    }

    /// The cached link of `tab` when it still serves the holder answering
    /// on `socket`. One that hung up, or that is on another socket or an
    /// earlier holder process, is dropped, so the caller connects anew
    /// rather than type into or read the screen of a holder that is gone.
    fn open_link(&self, tab: &str, socket: &Path) -> Option<Arc<Link>> {
        let answering = humata2_holder::holder_pid(socket);
        let mut links = lock(&self.links);
        match links.get(tab) {
            Some(link) if link.serves(socket, answering) => Some(link.clone()),
            Some(_) => {
                links.remove(tab);
                None
            }
            None => None,
        }
    }

    /// Drops `tab`'s link and its screen, closing its connection once no
    /// call holds it. Never waits on an answer the screen still owes the
    /// program.
    pub(crate) fn forget(&self, tab: &str) {
        lock(&self.links).remove(tab);
        lock(&self.starting).remove(tab);
    }
}

/// The tab's screen at an offset, and the size changes after it.
pub(crate) struct Checkpoint {
    pub(crate) snapshot: Snapshot,
    /// A cell's size in pixels.
    pub(crate) cell: (u32, u32),
    /// Size changes from the moment of the snapshot on. A `Lagged` error
    /// means some were missed: ask for a new checkpoint.
    pub(crate) changes: broadcast::Receiver<Resized>,
}

/// An open connection to one holder.
pub(crate) struct Link {
    /// The holder this link was made to: its socket, and the process
    /// answering on it then, if it said.
    socket: PathBuf,
    holder: Option<u32>,
    offsets: watch::Receiver<Offsets>,
    typing: mpsc::Sender<Typing>,
    reader: JoinHandle<()>,
    screen: Arc<Mutex<Option<Screen>>>,
    changes: broadcast::Sender<Resized>,
}

struct Typing {
    text: Vec<u8>,
    done: oneshot::Sender<Result<(), TypeFailed>>,
}

/// An answer to a terminal query, for the program to read as input.
struct Reply {
    seq: u64,
    bytes: Vec<u8>,
    /// The screen's record with this answer pending: saved before the
    /// answer is sent, so a crash in between leaves it pending.
    record: ReplyRecord,
}

/// Where a link keeps its screen's reply record.
struct Saving {
    store: Arc<Store>,
    tab: String,
}

struct Save {
    record: ReplyRecord,
    /// Told once the record is on disk.
    done: Option<oneshot::Sender<()>>,
}

/// Saves a screen's reply records, one at a time and in the order given,
/// off the reader's path. Records that pile up while one is being saved
/// are saved as their latest.
#[derive(Clone)]
struct Saver {
    queue: mpsc::UnboundedSender<Save>,
}

impl Saver {
    fn spawn(saving: Saving, generation: u64) -> Self {
        let (queue, saves) = mpsc::unbounded_channel();
        tokio::spawn(save_all(saving, generation, saves));
        Self { queue }
    }

    /// Saves `record` when it can; nothing waits for it.
    fn later(&self, record: ReplyRecord) {
        let _ = self.queue.send(Save { record, done: None });
    }

    /// Waits until `record` is saved, or saving it failed (and was said).
    async fn now(&self, record: ReplyRecord) {
        let (done, saved) = oneshot::channel();
        let sent = Save {
            record,
            done: Some(done),
        };
        if self.queue.send(sent).is_ok() {
            let _ = saved.await;
        }
    }
}

async fn save_all(saving: Saving, generation: u64, mut saves: mpsc::UnboundedReceiver<Save>) {
    while let Some(first) = saves.recv().await {
        let mut latest = first.record;
        let mut waiting: Vec<oneshot::Sender<()>> = first.done.into_iter().collect();
        while let Ok(next) = saves.try_recv() {
            latest = next.record;
            waiting.extend(next.done);
        }
        let (store, tab) = (saving.store.clone(), saving.tab.clone());
        let saved = tokio::task::spawn_blocking(move || {
            store.put_tab_replies(&tab, &replies_of(&latest, generation))
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                crate::log::say!("humatad2: couldn't save which of a tab's terminal queries were answered: {error}");
            }
            Err(error) => crate::log::say!("humatad2: a background task failed: {error}"),
        }
        for done in waiting {
            let _ = done.send(());
        }
    }
}

fn replies_of(record: &ReplyRecord, generation: u64) -> TabReplies {
    TabReplies {
        generation,
        answered_through: record.answered_through,
        pending: record.pending.clone(),
        uncertain: record.uncertain,
    }
}

/// What is saved for a tab's screen: the holder its tab names (none when
/// the tab isn't saved or couldn't be read), the tab's holder generation,
/// and the record saved for its screen, if any.
struct SavedTab {
    holder_pid: Option<u32>,
    generation: u64,
    replies: Option<TabReplies>,
}

async fn saved_record(saving: &Saving) -> SavedTab {
    let (store, tab) = (saving.store.clone(), saving.tab.clone());
    let read = tokio::task::spawn_blocking(move || -> Result<_, StoreError> {
        let saved = store.tab(&tab)?;
        Ok(SavedTab {
            holder_pid: saved.as_ref().map(|tab| tab.holder_pid),
            generation: saved.map_or(0, |tab| tab.holder_generation),
            replies: store.tab_replies(&tab)?,
        })
    })
    .await;
    match read {
        Ok(Ok(found)) => found,
        Ok(Err(error)) => {
            crate::log::say!("humatad2: couldn't read which of a tab's terminal queries were answered: {error}");
            SavedTab { holder_pid: None, generation: 0, replies: None }
        }
        Err(error) => {
            crate::log::say!("humatad2: a background task failed: {error}");
            SavedTab { holder_pid: None, generation: 0, replies: None }
        }
    }
}

/// Whether a link may be made to the holder `answering` on a tab's socket,
/// given the holder pid the tab is saved with. Reviving a tab starts its
/// new holder before saving the tab with it, and the tab's generation and
/// reply record are those of the holder it names: a link made in between
/// would run the new holder's screen on the old one's identity. So a tab
/// that names a holder is linked only to that holder; a tab not saved yet
/// (or unreadable) has nothing to contradict.
fn linkable(saved: Option<u32>, answering: Option<u32>) -> bool {
    match saved {
        None => true,
        Some(saved) => answering == Some(saved),
    }
}

/// The longest a link waits for a resumed tab to be saved with its new
/// holder (D672).
const SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Reads with `read` until `ready` accepts what it read, reading again only
/// when `commits` says the store wrote something, and for at most `within`.
/// Gives back the last thing read when it never became ready in time, or
/// when nothing can say of a write any more.
async fn wait_until<T, F, Fut>(
    mut read: F,
    ready: impl Fn(&T) -> bool,
    mut commits: watch::Receiver<u64>,
    within: std::time::Duration,
) -> Result<T, T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let deadline = tokio::time::Instant::now() + within;
    loop {
        // Before the read: a write that lands during it wakes the wait.
        commits.borrow_and_update();
        let last = read().await;
        if ready(&last) {
            return Ok(last);
        }
        match tokio::time::timeout_at(deadline, commits.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return Err(last),
        }
    }
}

/// Whether a link made to `linked` (a holder's socket and the process that
/// answered on it) is still for `wanted`. A holder that was replaced answers
/// as another process; a pid that either side doesn't know is no evidence
/// of a change.
fn same_holder(linked: (&Path, Option<u32>), wanted: (&Path, Option<u32>)) -> bool {
    linked.0 == wanted.0
        && match (linked.1, wanted.1) {
            (Some(was), Some(now)) => was == now,
            _ => true,
        }
}

/// Folds a title or folder step into what is waiting to be saved. A folder
/// that isn't one (OSC 7 gone wrong) leaves the last good one.
fn fold_program(step: Step, title: &mut Option<String>, pwd: &mut Option<String>) {
    match step {
        Step::Title(text) => *title = Some(text),
        Step::Pwd(reported) => {
            if let Some(path) = humata2_store::pwd_path(&reported) {
                *pwd = Some(path.to_string_lossy().into_owned());
            }
        }
        Step::Reply { .. } => {}
    }
}

/// Saves what a tab's program reports about itself (D480), off the
/// reader's path. Reports that pile up while one is being saved are saved
/// as their latest. The tab's own name is never touched.
async fn save_program(store: Arc<Store>, tab: String, mut steps: mpsc::UnboundedReceiver<Step>) {
    while let Some(first) = steps.recv().await {
        let (mut title, mut pwd) = (None, None);
        fold_program(first, &mut title, &mut pwd);
        while let Ok(next) = steps.try_recv() {
            fold_program(next, &mut title, &mut pwd);
        }
        if title.is_none() && pwd.is_none() {
            continue;
        }
        let (store, tab) = (store.clone(), tab.clone());
        let saved = tokio::task::spawn_blocking(move || {
            store.put_tab_program(&tab, title.as_deref(), pwd.as_deref())
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                crate::log::say!("humatad2: couldn't save what a tab's program reported about itself: {error}");
            }
            Err(error) => crate::log::say!("humatad2: a background task failed: {error}"),
        }
    }
}

/// What a screen resumes from, given the record saved for the tab (if
/// any), the tab's holder generation now, and where the holder's output
/// ends. A record of this holder is used as it is. A record of an earlier
/// holder is stale, and the new one has answered nothing: no answer goes
/// out before its record is saved. With no record, or one that doesn't
/// fit the holder, whatever is in the output already was answered or is
/// lost with the daemon that missed it, and is never answered now.
fn starting_record(saved: Option<TabReplies>, generation: u64, end: u64) -> ReplyRecord {
    match saved {
        Some(saved) if saved.generation == generation && saved.answered_through <= end => ReplyRecord {
            answered_through: saved.answered_through,
            pending: saved.pending,
            uncertain: saved.uncertain,
        },
        Some(saved) if saved.generation < generation => ReplyRecord::default(),
        _ => ReplyRecord {
            answered_through: end,
            ..ReplyRecord::default()
        },
    }
}

/// Hands the answers the screen raised to the writer, each with the
/// record to save before it goes out.
fn queue_replies(replies: &mpsc::UnboundedSender<Reply>, record: &ReplyRecord, steps: Vec<Step>) {
    for step in steps {
        if let Step::Reply { seq, bytes } = step {
            let _ = replies.send(Reply {
                seq,
                bytes,
                record: record.clone(),
            });
        }
    }
}

/// What the holder kept of a tab's output when a link started.
struct Kept {
    from: u64,
    bytes: Vec<u8>,
    /// What came after the kept output, in order.
    after: Vec<Event>,
}

impl Link {
    async fn start(
        attachment: Attachment,
        socket: &Path,
        limit: u64,
        saving: Saving,
        commits: watch::Receiver<u64>,
    ) -> Result<Self, HolderError> {
        let Attachment {
            welcome,
            mut events,
            control,
        } = attachment;
        let kept = read_kept(&welcome, &mut events).await?;
        let start = Offsets {
            earliest: welcome.earliest,
            end: welcome.end,
            exited: welcome.exited,
            closed: false,
        };
        // D672: a resume starts its holder a moment before saving the tab
        // with it, so a link that finds the tab naming another holder
        // waits for the store to say of a write, then re-checks, for at
        // most `SAVE_WAIT`. A holder still not the tab's then is refused.
        let saving_ref = &saving;
        let settled = wait_until(
            || async move {
                let tab = saved_record(saving_ref).await;
                // After the tab is read: a holder that replaced the one
                // the tab names can only make these differ, never agree
                // by chance.
                let answering = humata2_holder::holder_pid(socket);
                (tab, answering)
            },
            |(tab, answering)| linkable(tab.holder_pid, *answering),
            commits,
            SAVE_WAIT,
        )
        .await;
        let Ok((
            SavedTab {
                generation,
                replies: saved,
                ..
            },
            answering,
        )) = settled
        else {
            return Err(HolderError::NotSaved {
                socket: socket.to_owned(),
            });
        };
        let record = starting_record(saved, generation, welcome.end);
        let (screen, owed) = screen_of(&welcome, &kept, limit, &record);
        let (program_tx, program_rx) = mpsc::unbounded_channel();
        tokio::spawn(save_program(saving.store.clone(), saving.tab.clone(), program_rx));
        let saver = Saver::spawn(saving, generation);
        // Neither can block the reader: it must keep draining the holder.
        let (replies_tx, replies) = mpsc::unbounded_channel();
        let (heard_tx, heard) = mpsc::unbounded_channel();
        if let Some(screen) = &screen {
            // A record always stands for the holder this link is on, and
            // answers to queries that never had one go out after it.
            let record = screen.authority.record();
            saver.later(record.clone());
            queue_replies(&replies_tx, &record, owed);
        }
        let screen = Arc::new(Mutex::new(screen));
        let (offsets_tx, offsets) = watch::channel(start);
        let (changes, _) = broadcast::channel(CHANGES_QUEUE);
        let reader = Reader {
            offsets: offsets_tx,
            limit,
            screen: screen.clone(),
            replies: replies_tx,
            heard: heard_tx,
            program: program_tx,
        };
        let reader = tokio::spawn(reader.run(events, kept.after));
        let (typing, requests) = mpsc::channel(16);
        tokio::spawn(write(control, requests, replies, heard, screen.clone(), saver));
        Ok(Self {
            socket: socket.to_owned(),
            holder: answering,
            offsets,
            typing,
            reader,
            screen,
            changes,
        })
    }

    /// Whether the holder is still connected and typing still goes
    /// through.
    fn open(&self) -> bool {
        !self.offsets.borrow().closed && !self.typing.is_closed()
    }

    /// Whether this link is open and still the one for the holder
    /// answering on `socket` (process `answering`, if it says).
    fn serves(&self, socket: &Path, answering: Option<u32>) -> bool {
        self.open() && same_holder((&self.socket, self.holder), (socket, answering))
    }

    pub(crate) fn offsets(&self) -> Offsets {
        *self.offsets.borrow()
    }

    /// Types `text` and waits until all of it reached the terminal. Calls
    /// take turns. A failure ends the link, so the next call reconnects
    /// rather than read an answer meant for this one.
    pub(crate) async fn type_text(&self, text: &[u8]) -> Result<(), TypeFailed> {
        let (done, answer) = oneshot::channel();
        let typing = Typing {
            text: text.to_vec(),
            done,
        };
        if self.typing.send(typing).await.is_err() {
            return Err(TypeFailed::Exited);
        }
        answer.await.unwrap_or(Err(TypeFailed::Exited))
    }

    /// The screen as it is now, encoded off the runtime's threads, with
    /// the size changes that come after it.
    pub(crate) async fn checkpoint(&self) -> Result<Checkpoint, ScreenFailed> {
        let screen = self.screen.clone();
        let changes = self.changes.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = lock(&screen);
            let screen = guard.as_mut().ok_or(ScreenFailed::Unavailable)?;
            // Under the lock, so no size change falls between the two.
            let receiver = changes.subscribe();
            let snapshot = screen.authority.snapshot().map_err(ScreenFailed::Engine)?;
            if snapshot.trimmed_rows > 0 {
                crate::log::say!(
                    "humatad2: a tab's screen was over the {CHECKPOINT_LIMIT}-byte checkpoint limit, so its oldest {} history rows were dropped from the screen (the holder still keeps the output)",
                    snapshot.trimmed_rows
                );
            }
            Ok(Checkpoint {
                snapshot,
                cell: screen.cell,
                changes: receiver,
            })
        })
        .await
        .map_err(ScreenFailed::Stopped)?
    }

    /// Resizes the screen and tells every attach of the tab. A cell size
    /// of 0 keeps the one the screen has. Resize the holder after this, so
    /// the output that follows the new size is parsed at it.
    pub(crate) fn resize(
        &self,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<(), ScreenFailed> {
        let mut guard = lock(&self.screen);
        let screen = guard.as_mut().ok_or(ScreenFailed::Unavailable)?;
        let cell = (
            if cell_width == 0 { screen.cell.0 } else { cell_width },
            if cell_height == 0 { screen.cell.1 } else { cell_height },
        );
        let resized = screen
            .authority
            .resize(cols, rows, cell.0, cell.1)
            .map_err(ScreenFailed::Engine)?;
        screen.cell = cell;
        // No attach listening is fine.
        let _ = self.changes.send(resized);
        Ok(())
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Why [`screen_at`] couldn't read a holder's screen.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ScreenUnread {
    #[error("humatad2 couldn't reach the tab's holder to read its screen")]
    Attach(#[source] HolderError),
    #[error("humatad2 couldn't rebuild the tab's screen from its output")]
    Engine(#[source] AuthorityError),
    #[error("the tab's holder didn't send its screen's output in time")]
    TimedOut,
}

/// D762: the text the holder at `socket` shows on its screen now, rebuilt
/// from the output it kept. A screen of its own, not a link's, and only
/// read: the queries in the output get no answer from it.
pub(crate) async fn screen_at(socket: &Path) -> Result<String, ScreenUnread> {
    let Attachment {
        welcome, mut events, ..
    } = humata2_holder::attach(socket, 0).await.map_err(ScreenUnread::Attach)?;
    let kept = read_kept(&welcome, &mut events).await.map_err(ScreenUnread::Attach)?;
    let (cols, rows) = match (welcome.cols, welcome.rows) {
        (0, _) | (_, 0) => DEFAULT_SIZE,
        size => size,
    };
    let config = Config {
        cols,
        rows,
        scrollback_lines: SCROLLBACK_LINES,
        scrollback_bytes: kept.bytes.len().max(1),
        checkpoint_limit: CHECKPOINT_LIMIT,
    };
    // Every query counts as answered, so none is owed.
    let record = ReplyRecord {
        answered_through: welcome.end,
        ..ReplyRecord::default()
    };
    let (authority, _) =
        Authority::resume(&config, kept.from, &kept.bytes, &record).map_err(ScreenUnread::Engine)?;
    authority.screen_text().map_err(ScreenUnread::Engine)
}

/// Reads the output the holder keeps, which it sends first.
async fn read_kept(welcome: &Welcome, events: &mut Events) -> Result<Kept, HolderError> {
    let mut kept = Kept {
        from: welcome.earliest,
        bytes: Vec::new(),
        after: Vec::new(),
    };
    let mut at = welcome.earliest;
    while at < welcome.end {
        let Some(event) = events.next().await? else {
            break;
        };
        match event {
            Event::Output { offset, data } => {
                if kept.bytes.is_empty() || offset != at {
                    kept.bytes.clear();
                    kept.from = offset;
                }
                let split = usize::try_from(welcome.end.saturating_sub(offset))
                    .unwrap_or(usize::MAX)
                    .min(data.len());
                let (inside, beyond) = data.split_at(split);
                kept.bytes.extend_from_slice(inside);
                if !beyond.is_empty() {
                    kept.after.push(Event::Output {
                        offset: welcome.end,
                        data: beyond.to_vec(),
                    });
                }
                at = offset + data.len() as u64;
            }
            // Said before the kept output when it starts after the ask.
            Event::Trimmed { resume_at } => {
                at = at.max(resume_at);
                kept.bytes.clear();
                kept.from = at;
            }
            other => kept.after.push(other),
        }
    }
    Ok(kept)
}

/// The screen for a link that starts now, from the record it resumes
/// with, and the answers owed to queries in the kept output that the
/// record doesn't say were answered. Queries it says were answered are
/// never answered again.
fn screen_of(
    welcome: &Welcome,
    kept: &Kept,
    limit: u64,
    record: &ReplyRecord,
) -> (Option<Screen>, Vec<Step>) {
    let (cols, rows) = match (welcome.cols, welcome.rows) {
        (0, _) | (_, 0) => DEFAULT_SIZE,
        size => size,
    };
    let config = Config {
        cols,
        rows,
        scrollback_lines: SCROLLBACK_LINES,
        scrollback_bytes: usize::try_from(limit).unwrap_or(usize::MAX),
        checkpoint_limit: CHECKPOINT_LIMIT,
    };
    match Authority::resume(&config, kept.from, &kept.bytes, record) {
        Ok((authority, owed)) => (
            Some(Screen {
                authority,
                cell: (1, 1),
            }),
            owed,
        ),
        Err(error) => {
            crate::log::say!("humatad2: a tab's screen couldn't be built from its output: {error}");
            (None, Vec::new())
        }
    }
}

/// What the link's reader task needs while it reads.
struct Reader {
    offsets: watch::Sender<Offsets>,
    limit: u64,
    screen: Arc<Mutex<Option<Screen>>>,
    replies: mpsc::UnboundedSender<Reply>,
    heard: mpsc::UnboundedSender<Event>,
    /// The title and folder steps, for the store (D480).
    program: mpsc::UnboundedSender<Step>,
}

impl Reader {
    /// Keeps the offsets and the screen as output arrives and hands what
    /// the writer waits for on.
    async fn run(self, mut events: Events, first: Vec<Event>) {
        for event in first {
            self.handle(event);
        }
        loop {
            match events.next().await {
                Ok(Some(event)) => self.handle(event),
                Ok(None) | Err(_) => break,
            }
        }
        self.offsets.send_modify(|now| now.closed = true);
    }

    fn handle(&self, event: Event) {
        match event {
            Event::Output { offset, data } => {
                let end = offset + data.len() as u64;
                self.feed(offset, &data);
                self.offsets
                    .send_if_modified(|now| advance(now, end, self.limit));
            }
            Event::Trimmed { resume_at } => {
                self.offsets.send_if_modified(|now| {
                    let moved = advance(now, resume_at, self.limit);
                    let trimmed = now.earliest < resume_at;
                    now.earliest = now.earliest.max(resume_at);
                    moved || trimmed
                });
            }
            Event::Exited { code } => {
                self.offsets.send_modify(|now| now.exited = Some(code));
                // Typing waiting on an answer hears it ended.
                let _ = self.heard.send(Event::Exited { code });
            }
            answer @ (Event::InputWritten { .. } | Event::Refused { .. }) => {
                let _ = self.heard.send(answer);
            }
        }
    }

    /// Parses output on the screen and queues the answers it raises.
    fn feed(&self, offset: u64, data: &[u8]) {
        let (steps, record) = {
            let mut guard = lock(&self.screen);
            let Some(screen) = guard.as_mut() else {
                return;
            };
            match screen.authority.feed(offset, data) {
                Ok(steps) => (steps, screen.authority.record()),
                Err(error) => {
                    // The screen would be wrong from here on: better none
                    // than a wrong one. The next link rebuilds it.
                    crate::log::say!("humatad2: dropping a tab's screen: {error}");
                    *guard = None;
                    return;
                }
            }
        };
        // The writer saves the record before it sends an answer. The title
        // and folder steps go to the store when it wants them.
        for step in &steps {
            if matches!(step, Step::Title(_) | Step::Pwd(_)) {
                let _ = self.program.send(step.clone());
            }
        }
        queue_replies(&self.replies, &record, steps);
    }
}

/// Moves the end on to `end`, and the earliest kept byte with it once more
/// than `limit` bytes are past it. Says whether anything moved.
fn advance(now: &mut Offsets, end: u64, limit: u64) -> bool {
    if end <= now.end {
        return false;
    }
    now.end = end;
    now.earliest = now.earliest.max(end.saturating_sub(limit));
    true
}

/// Input sent to the holder and not yet confirmed written.
enum Sent {
    Typed(oneshot::Sender<Result<(), TypeFailed>>),
    /// The screen's answer with this sequence number.
    Answer(u64),
}

/// Sends typing and the screen's answers to the program, each with a
/// sequence number of its own on this connection, and settles them from
/// what the holder says. An answer never waits for the typing before it
/// to be confirmed, so a program stopped reading its input can't keep a
/// query from being answered behind it.
async fn write(
    mut control: Control,
    mut requests: mpsc::Receiver<Typing>,
    mut replies: mpsc::UnboundedReceiver<Reply>,
    mut heard: mpsc::UnboundedReceiver<Event>,
    screen: Arc<Mutex<Option<Screen>>>,
    saver: Saver,
) {
    let mut seq = 0;
    let mut sent: VecDeque<(u64, Sent)> = VecDeque::new();
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(Typing { text, done }) = request else { break };
                seq += 1;
                match control.input(seq, &text).await {
                    Ok(()) => sent.push_back((seq, Sent::Typed(done))),
                    Err(error) => {
                        let _ = done.send(Err(TypeFailed::Lost(error)));
                        break;
                    }
                }
            }
            Some(Reply { seq: answer, bytes, record }) = replies.recv() => {
                // On disk first: a crash after the send leaves the answer
                // pending, which a restart flags rather than sends again.
                saver.now(record).await;
                seq += 1;
                if control.input(seq, &bytes).await.is_err() {
                    break;
                }
                sent.push_back((seq, Sent::Answer(answer)));
            }
            event = heard.recv() => match event {
                Some(Event::InputWritten { seq: written }) => settle(&mut sent, written, &screen, &saver),
                Some(Event::Refused { message }) => {
                    if refuse(&mut sent, message) {
                        break;
                    }
                }
                Some(Event::Exited { .. }) => {
                    if end_typing(&mut sent) {
                        break;
                    }
                }
                Some(_) => {}
                None => break,
            },
        }
    }
    // Typing still waiting is dropped, and its callers hear it ended.
}

/// The holder wrote input `written`: its typing is done, or its answer is
/// no longer pending on the screen.
fn settle(
    sent: &mut VecDeque<(u64, Sent)>,
    written: u64,
    screen: &Mutex<Option<Screen>>,
    saver: &Saver,
) {
    let Some(index) = sent.iter().position(|(seq, _)| *seq == written) else {
        return;
    };
    match sent.remove(index) {
        Some((_, Sent::Typed(done))) => {
            let _ = done.send(Ok(()));
        }
        Some((_, Sent::Answer(answer))) => {
            if let Some(screen) = lock(screen).as_mut() {
                screen.authority.written(answer);
                // Queued under the screen's lock, so records are saved in
                // the order the screen had them.
                saver.later(screen.authority.record());
            }
        }
        None => {}
    }
}

/// The holder refused the oldest input sent. Says whether that was typing,
/// which ends the link.
fn refuse(sent: &mut VecDeque<(u64, Sent)>, message: String) -> bool {
    match sent.pop_front() {
        Some((_, Sent::Typed(done))) => {
            let _ = done.send(Err(TypeFailed::Refused { message }));
            true
        }
        Some((_, Sent::Answer(_))) => {
            // It stays pending on the screen: it is unknown whether the
            // program got it.
            crate::log::say!("humatad2: a tab's holder refused an answer to a query: {message}");
            false
        }
        None => false,
    }
}

/// The shell exited: typing still waiting never reaches it. Says whether
/// any was waiting.
fn end_typing(sent: &mut VecDeque<(u64, Sent)>) -> bool {
    let mut any = false;
    for (_, input) in sent.drain(..) {
        if let Sent::Typed(done) = input {
            any = true;
            let _ = done.send(Err(TypeFailed::Exited));
        }
    }
    any
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(earliest: u64, end: u64) -> Offsets {
        Offsets {
            earliest,
            end,
            ..Offsets::default()
        }
    }

    #[test]
    fn output_within_the_scrollback_keeps_the_earliest_byte() {
        let mut now = at(0, 10);
        assert!(advance(&mut now, 60, 100));
        assert_eq!(now, at(0, 60));
    }

    #[test]
    fn output_past_the_scrollback_moves_the_earliest_byte_on() {
        let mut now = at(0, 90);
        assert!(advance(&mut now, 150, 100));
        assert_eq!(now, at(50, 150));
    }

    #[test]
    fn a_tab_is_linked_only_to_the_holder_it_names() {
        assert!(linkable(Some(7), Some(7)));
        assert!(!linkable(Some(7), Some(8)), "a new holder the tab isn't saved with yet");
        assert!(!linkable(Some(7), None), "a holder that hasn't said who it is");
        assert!(!linkable(Some(0), Some(8)), "a tab saved before any holder started");
    }

    /// The tab as a test saves it: the holder pid it names, and the
    /// store's commit notice.
    fn saved_tab(pid: u32) -> (Arc<std::sync::atomic::AtomicU32>, watch::Sender<u64>, watch::Receiver<u64>) {
        let (committed, commits) = watch::channel(0u64);
        (Arc::new(std::sync::atomic::AtomicU32::new(pid)), committed, commits)
    }

    fn save(tab: &std::sync::atomic::AtomicU32, committed: &watch::Sender<u64>, pid: u32) {
        tab.store(pid, std::sync::atomic::Ordering::SeqCst);
        committed.send_modify(|count| *count += 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_waits_for_the_tab_to_be_saved_with_the_new_holder() {
        let (tab, committed, commits) = saved_tab(7);
        let saving = tab.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            // A write that doesn't name the new holder yet, then the save.
            save(&saving, &committed, 7);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            save(&saving, &committed, 8);
            // Kept open, so the wait can only end on the save or its bound.
            tokio::time::sleep(SAVE_WAIT * 2).await;
            drop(committed);
        });
        let started = tokio::time::Instant::now();
        let found = wait_until(
            || {
                let saved = tab.load(std::sync::atomic::Ordering::SeqCst);
                async move { saved }
            },
            |saved| linkable(Some(*saved), Some(8)),
            commits,
            SAVE_WAIT,
        )
        .await;
        assert_eq!(found, Ok(8));
        assert!(started.elapsed() < SAVE_WAIT, "woken by the save, not by the bound");
    }

    #[tokio::test(start_paused = true)]
    async fn a_holder_that_never_becomes_the_tabs_is_refused_after_the_bound() {
        let (tab, committed, commits) = saved_tab(7);
        let started = tokio::time::Instant::now();
        let found = wait_until(
            || {
                let saved = tab.load(std::sync::atomic::Ordering::SeqCst);
                async move { saved }
            },
            |saved| linkable(Some(*saved), Some(8)),
            commits,
            SAVE_WAIT,
        )
        .await;
        assert_eq!(found, Err(7));
        assert!(started.elapsed() >= SAVE_WAIT);
        drop(committed);
    }

    #[tokio::test]
    async fn a_tab_already_saved_with_the_holder_links_at_once() {
        let (tab, _committed, commits) = saved_tab(8);
        let found = wait_until(
            || {
                let saved = tab.load(std::sync::atomic::Ordering::SeqCst);
                async move { saved }
            },
            |saved| linkable(Some(*saved), Some(8)),
            commits,
            SAVE_WAIT,
        )
        .await;
        assert_eq!(found, Ok(8));
    }

    #[test]
    fn a_tab_not_saved_has_nothing_to_contradict_a_link() {
        assert!(linkable(None, Some(8)));
        assert!(linkable(None, None));
    }

    #[test]
    fn a_link_is_kept_for_the_holder_it_was_made_to() {
        let socket = Path::new("/tmp/a.sock");
        assert!(same_holder((socket, Some(7)), (socket, Some(7))));
    }

    #[test]
    fn a_link_is_dropped_once_another_process_answers_on_the_socket() {
        let socket = Path::new("/tmp/a.sock");
        assert!(!same_holder((socket, Some(7)), (socket, Some(8))));
    }

    #[test]
    fn a_link_is_dropped_for_another_socket() {
        assert!(!same_holder(
            (Path::new("/tmp/a.sock"), Some(7)),
            (Path::new("/tmp/b.sock"), Some(7))
        ));
    }

    #[test]
    fn an_unknown_pid_is_no_reason_to_drop_a_link() {
        let socket = Path::new("/tmp/a.sock");
        assert!(same_holder((socket, None), (socket, Some(8))));
        assert!(same_holder((socket, Some(7)), (socket, None)));
    }

    #[test]
    fn the_latest_title_and_a_valid_folder_win_and_a_bad_folder_changes_nothing() {
        let (mut title, mut pwd) = (None, None);
        fold_program(Step::Title("one".to_owned()), &mut title, &mut pwd);
        fold_program(Step::Pwd("file://host/tmp/a".to_owned()), &mut title, &mut pwd);
        fold_program(Step::Title("two".to_owned()), &mut title, &mut pwd);
        fold_program(Step::Pwd("not a folder".to_owned()), &mut title, &mut pwd);
        assert_eq!(title.as_deref(), Some("two"));
        assert_eq!(pwd.as_deref(), Some("/tmp/a"));
    }

    fn saved(generation: u64, answered_through: u64, pending: &[u64]) -> TabReplies {
        TabReplies {
            generation,
            answered_through,
            pending: pending.to_vec(),
            uncertain: false,
        }
    }

    #[test]
    fn a_record_of_this_holder_is_resumed_from() {
        let record = starting_record(Some(saved(2, 40, &[3])), 2, 100);
        assert_eq!(record.answered_through, 40);
        assert_eq!(record.pending, vec![3]);
    }

    #[test]
    fn a_record_of_an_earlier_holder_is_stale_and_the_new_holder_has_answered_nothing() {
        let record = starting_record(Some(saved(1, 5000, &[3])), 2, 100);
        assert_eq!(record, ReplyRecord::default());
    }

    #[test]
    fn no_record_means_what_the_output_already_holds_is_never_answered() {
        let record = starting_record(None, 0, 100);
        assert_eq!(record.answered_through, 100);
        assert!(record.pending.is_empty());
    }

    #[test]
    fn a_record_past_the_holders_output_does_not_fit_it() {
        let record = starting_record(Some(saved(2, 900, &[])), 2, 100);
        assert_eq!(record.answered_through, 100);
    }

    #[test]
    fn output_already_counted_changes_nothing() {
        let mut now = at(5, 90);
        assert!(!advance(&mut now, 80, 100));
        assert_eq!(now, at(5, 90));
    }
}

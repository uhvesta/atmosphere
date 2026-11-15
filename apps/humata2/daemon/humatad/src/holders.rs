//! One long-lived connection to each tab's holder, shared by every call
//! that only needs to know where the tab stands or to type into it. A
//! connection opens on first use and stays until the holder hangs up or
//! the tab is closed, so listing tabs or typing costs a message on an open
//! socket, not a connect and a welcome per tab per call. Attaches still
//! open their own: each streams from an offset of its own.
//!
//! The holder's protocol has no control-only connection, so the link
//! hears all of its tab's output and keeps only the offsets. That costs a
//! read per chunk the shell prints, and nothing while it's quiet: no
//! timers run on a link.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_holder::{Attachment, Control, Event, Events, HolderError};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

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

/// The links, by tab id.
pub(crate) struct Holders {
    links: Mutex<HashMap<String, Arc<Link>>>,
    /// The scrollback each holder was started with, to work out what it
    /// drops as output arrives.
    scrollback_bytes: u64,
}

impl Holders {
    pub(crate) fn new(scrollback_bytes: usize) -> Self {
        Self {
            links: Mutex::new(HashMap::new()),
            scrollback_bytes: u64::try_from(scrollback_bytes.max(1)).unwrap_or(u64::MAX),
        }
    }

    /// The open link to `tab`'s holder at `socket`, connecting if there is
    /// none or the last one was hung up on.
    pub(crate) async fn link(&self, tab: &str, socket: &Path) -> Result<Arc<Link>, HolderError> {
        if let Some(link) = lock(&self.links).get(tab)
            && link.open()
        {
            return Ok(link.clone());
        }
        let attachment = humata2_holder::attach(socket, u64::MAX).await?;
        let link = Arc::new(Link::start(attachment, self.scrollback_bytes));
        lock(&self.links).insert(tab.to_owned(), link.clone());
        Ok(link)
    }

    /// Drops `tab`'s link, closing its connection once no call holds it.
    pub(crate) fn forget(&self, tab: &str) {
        lock(&self.links).remove(tab);
    }
}

/// An open connection to one holder.
pub(crate) struct Link {
    offsets: watch::Receiver<Offsets>,
    typing: mpsc::Sender<Typing>,
    reader: JoinHandle<()>,
}

struct Typing {
    text: Vec<u8>,
    done: oneshot::Sender<Result<(), TypeFailed>>,
}

impl Link {
    fn start(attachment: Attachment, limit: u64) -> Self {
        let Attachment {
            welcome,
            events,
            control,
        } = attachment;
        let start = Offsets {
            earliest: welcome.earliest,
            end: welcome.end,
            exited: welcome.exited,
            closed: false,
        };
        let (offsets_tx, offsets) = watch::channel(start);
        // At most one input is in flight, so one answer at a time.
        let (answers_tx, answers) = mpsc::channel(4);
        let reader = tokio::spawn(read(events, offsets_tx, answers_tx, limit));
        let (typing, requests) = mpsc::channel(16);
        tokio::spawn(write(control, requests, answers));
        Self {
            offsets,
            typing,
            reader,
        }
    }

    /// Whether the holder is still connected and typing still goes
    /// through.
    fn open(&self) -> bool {
        !self.offsets.borrow().closed && !self.typing.is_closed()
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
}

impl Drop for Link {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Keeps the offsets as output arrives and hands answers to typing on.
async fn read(
    mut events: Events,
    offsets: watch::Sender<Offsets>,
    answers: mpsc::Sender<Event>,
    limit: u64,
) {
    loop {
        let event = match events.next().await {
            Ok(Some(event)) => event,
            Ok(None) | Err(_) => break,
        };
        match event {
            Event::Output { offset, data } => {
                let end = offset + data.len() as u64;
                offsets.send_if_modified(|now| advance(now, end, limit));
            }
            Event::Trimmed { resume_at } => {
                offsets.send_if_modified(|now| {
                    let moved = advance(now, resume_at, limit);
                    let trimmed = now.earliest < resume_at;
                    now.earliest = now.earliest.max(resume_at);
                    moved || trimmed
                });
            }
            Event::Exited { code } => {
                offsets.send_modify(|now| now.exited = Some(code));
                // Typing waiting on an answer hears it ended.
                let _ = answers.try_send(Event::Exited { code });
            }
            answer @ (Event::InputWritten { .. } | Event::Refused { .. }) => {
                if answers.send(answer).await.is_err() {
                    break;
                }
            }
        }
    }
    offsets.send_modify(|now| now.closed = true);
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

/// Sends typing one call at a time, each with a sequence number of its
/// own, and answers it from what the holder says.
async fn write(
    mut control: Control,
    mut requests: mpsc::Receiver<Typing>,
    mut answers: mpsc::Receiver<Event>,
) {
    let mut seq = 0;
    while let Some(Typing { text, done }) = requests.recv().await {
        seq += 1;
        let result = type_one(&mut control, &mut answers, seq, &text).await;
        let failed = result.is_err();
        let _ = done.send(result);
        if failed {
            break;
        }
    }
}

async fn type_one(
    control: &mut Control,
    answers: &mut mpsc::Receiver<Event>,
    seq: u64,
    text: &[u8],
) -> Result<(), TypeFailed> {
    control.input(seq, text).await.map_err(TypeFailed::Lost)?;
    loop {
        match answers.recv().await {
            Some(Event::InputWritten { seq: written }) if written == seq => return Ok(()),
            Some(Event::Refused { message }) => return Err(TypeFailed::Refused { message }),
            Some(Event::Exited { .. }) | None => return Err(TypeFailed::Exited),
            Some(_) => {}
        }
    }
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
    fn output_already_counted_changes_nothing() {
        let mut now = at(5, 90);
        assert!(!advance(&mut now, 80, 100));
        assert_eq!(now, at(5, 90));
    }
}

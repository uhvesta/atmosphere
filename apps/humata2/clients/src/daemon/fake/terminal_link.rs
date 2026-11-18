//! The fake's terminal link: a real Ghostty checkpoint of the tab's screen,
//! then Control, then what the tab prints from then on. What the view sends
//! reaches the same fake program the older calls reach: typing is echoed,
//! and a new size is answered with Resized and, for an agent, a redraw.

use std::sync::{Arc, Mutex};

use ghostty_vt::Terminal;
use terminal_wire::{ClientFrame, MAX_SNAPSHOT_BYTES, SNAPSHOT_ENGINE, ServerFrame};
use tokio::sync::mpsc;

use super::{FakeDaemon, Inner, terminals};
use crate::daemon::{DaemonError, LinkSink, Reply, SendError, TerminalLink, TerminalNotice};
use crate::model::{SessionId, Status};

/// The grid a tab has before its view reports its own size.
const DEFAULT_SIZE: (u16, u16) = (100, 30);
const SCROLLBACK: usize = 1000;
const QUEUE: usize = 256;

/// A view that is following a tab: where its frames go and how far along
/// the tab's output it is.
pub(super) struct LinkTap {
    key: (SessionId, usize),
    route: u64,
    frames: mpsc::Sender<ServerFrame>,
    notices: mpsc::Sender<TerminalNotice>,
    offset: u64,
}

/// Hands what a tab printed to the views following it, in order.
pub(super) fn feed(links: &mut Vec<LinkTap>, session: &SessionId, tab: usize, bytes: &[u8]) {
    links.retain_mut(|tap| {
        if tap.frames.is_closed() {
            return false;
        }
        if tap.key.0 != *session || tap.key.1 != tab {
            return true;
        }
        let frame = ServerFrame::Output {
            id: tap.route,
            offset: tap.offset,
            bytes: bytes.to_vec(),
        };
        tap.offset += bytes.len() as u64;
        !matches!(
            tap.frames.try_send(frame),
            Err(mpsc::error::TrySendError::Closed(_))
        )
    });
}

/// Tells the views following a tab that its program exited with `code`.
pub(super) fn exit(links: &[LinkTap], session: &SessionId, tab: usize, code: i32) {
    for tap in links
        .iter()
        .filter(|tap| tap.key.0 == *session && tap.key.1 == tab)
    {
        let _ = tap.frames.try_send(ServerFrame::Exited {
            id: tap.route,
            code: Some(code),
        });
    }
}

/// Ends the links of the views following a tab, as the daemon's do when the
/// tab's program is stopped: each view sees its link close.
pub(super) fn end(links: &mut Vec<LinkTap>, session: &SessionId, tab: usize) {
    links.retain(|tap| tap.key.0 != *session || tap.key.1 != tab);
}

/// Hands `notice` to the views following a tab.
pub(super) fn notice(links: &[LinkTap], session: &SessionId, tab: usize, notice: TerminalNotice) {
    for tap in links
        .iter()
        .filter(|tap| tap.key.0 == *session && tap.key.1 == tab)
    {
        let _ = tap.notices.try_send(notice.clone());
    }
}

pub(super) fn link(daemon: &FakeDaemon, session: &SessionId, tab: usize) -> Reply<TerminalLink> {
    let id = session.clone();
    let shared = daemon.shared.clone();
    daemon.reply(
        None,
        format!("terminal link {session} {tab}"),
        move |inner| {
            let found = inner.session_mut(&id)?;
            if tab > found.shells.len() {
                return Err(DaemonError::NotFound {
                    what: format!("Terminal {tab} of {id}"),
                });
            }
            let exited = match found.status {
                Status::Exited { code } if tab == 0 => Some(code),
                _ => None,
            };
            let key = (id.clone(), tab);
            let exited = exited.or_else(|| inner.exited_tabs.get(&key).copied());
            let size = inner
                .workspace
                .terminal_sizes
                .get(&key)
                .copied()
                .unwrap_or(DEFAULT_SIZE);
            let screen = inner
                .workspace
                .terminal_bytes
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let (link, tap) = start(&shared, &screen, size, exited, key)?;
            inner.links.push(tap);
            Ok(link)
        },
    )
}

fn refused(reason: String) -> DaemonError {
    DaemonError::Refused {
        what: "The terminal's screen".into(),
        reason,
    }
}

fn locked(shared: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    match shared.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The frames a view gets first, the sink that answers what it sends, and
/// the tap that keeps it following the tab.
fn start(
    shared: &Arc<Mutex<Inner>>,
    screen: &[u8],
    (cols, rows): (u16, u16),
    exited: Option<i32>,
    key: (SessionId, usize),
) -> Result<(TerminalLink, LinkTap), DaemonError> {
    let broken = |error: ghostty_vt::Error| {
        refused(format!(
            "it could not be built ({error}). Reload the app; if it keeps failing, update Humata"
        ))
    };
    let route = key.1 as u64;
    let mut terminal = Terminal::new(cols, rows, SCROLLBACK).map_err(broken)?;
    terminal.feed(screen);
    let bytes = terminal.checkpoint(MAX_SNAPSHOT_BYTES).map_err(broken)?;
    let (frames, received) = mpsc::channel(QUEUE);
    let (notices, notices_received) = mpsc::channel(QUEUE);
    let first = [
        ServerFrame::Snapshot {
            id: route,
            engine: SNAPSHOT_ENGINE.to_owned(),
            bytes,
            offset: screen.len() as u64,
            cols,
            rows,
            live: exited.is_none(),
            exited,
        },
        ServerFrame::Control {
            id: route,
            controller: true,
        },
    ];
    for frame in first {
        frames
            .try_send(frame)
            .map_err(|error| refused(format!("its first frames were not queued ({error})")))?;
    }
    let tap = LinkTap {
        key: key.clone(),
        route,
        frames: frames.clone(),
        notices,
        offset: screen.len() as u64,
    };
    let shared = shared.clone();
    // Only the tap keeps the link open, so ending the tap ends the link.
    let frames = frames.downgrade();
    let sink = LinkSink::new(move |frame| {
        let Some(frames) = frames.upgrade().filter(|frames| !frames.is_closed()) else {
            return Err(SendError::Closed);
        };
        match frame {
            ClientFrame::Resize {
                cols,
                rows,
                cell_width,
                cell_height,
                ..
            } => {
                let mut inner = locked(&shared);
                frames
                    .try_send(ServerFrame::Resized {
                        id: route,
                        cols,
                        rows,
                        cell_width,
                        cell_height,
                    })
                    .map_err(SendError::from)?;
                // A refused size changes nothing; the view keeps its grid.
                let _ = terminals::apply_resize(&mut inner, &key.0, key.1, cols, rows);
                Ok(())
            }
            ClientFrame::Input { seq, bytes, .. } => {
                let mut inner = locked(&shared);
                let answer = match terminals::apply_input(&mut inner, &key.0, key.1, bytes) {
                    Ok(()) => ServerFrame::InputWritten { id: route, seq },
                    Err(error) => ServerFrame::Error {
                        id: Some(route),
                        code: "input_refused".to_owned(),
                        message: error.to_string(),
                    },
                };
                frames.try_send(answer).map_err(SendError::from)
            }
            _ => Ok(()),
        }
    });
    Ok((
        TerminalLink {
            frames: received,
            notices: notices_received,
            sink,
        },
        tap,
    ))
}

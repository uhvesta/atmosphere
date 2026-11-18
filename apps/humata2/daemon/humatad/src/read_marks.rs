//! Read marks (CH-16, CH-17; D392): a session whose agent printed something
//! while no client was viewing it is unread, shown bold, until it is viewed.
//!
//! A client views a session by holding MarkService.ViewSession open: that
//! clears the mark, and output while any client holds one marks nothing.
//! The marks are kept in `humata2_marks`'s file, `<state>/marks.db`, so a
//! restart keeps them, and also here, so building a session never reads
//! the disk. Every change is sent on [`changes`], for the feed.
//!
//! A session's pin and VIP mark (NV-20; D387) are kept here the same way,
//! read from the same file at start and set by MarkService, so building a
//! session never reads the disk for them either. A change to either is sent
//! on [`changes`] too: the feed tells the session again.
//!
//! One set for the process, as `turns` keeps one registry: `session_pb`
//! builds a session from anywhere and only needs to ask.
//!
//! A redraw is not new output (D393): when the daemon resizes a session's
//! agent tab, the program repaints its screen, and that must not turn the
//! session unread. After a resize the session is "redrawing" until
//! something is typed to the agent, or the agent prints a word it hadn't
//! printed lately (see [`Printed`]); output while redrawing marks only in
//! the second case.
//!
//! Only output this daemon follows marks a session: its own sessions'
//! agent tabs. A session running on another machine isn't marked on the
//! host yet.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};

use humata2_marks::{Marks, MarksError};
use humata2_store::{TabKind, TabRecord};
use tokio::sync::broadcast;

mod printed;

pub(crate) use printed::Printed;

/// How many changes a slow follower may fall behind by.
const CHANGES: usize = 256;

static READS: LazyLock<Reads> = LazyLock::new(|| Reads {
    state: Mutex::default(),
    path: OnceLock::new(),
    file: OnceLock::new(),
    writing: Mutex::new(()),
    changes: broadcast::channel(CHANGES).0,
});

struct Reads {
    state: Mutex<State>,
    /// `<state>/marks.db`, once the daemon named it.
    path: OnceLock<PathBuf>,
    /// The file, once something needed it.
    file: OnceLock<Marks>,
    /// Held while a mark is written, so the file ends as memory is.
    writing: Mutex<()>,
    /// Each session whose mark changed.
    changes: broadcast::Sender<String>,
}

#[derive(Default)]
struct State {
    unread: HashSet<String>,
    /// Sessions the user pinned (D387).
    pinned: HashSet<String>,
    /// Sessions the user marked VIP (D387).
    vip: HashSet<String>,
    /// How many views each viewed session has open.
    viewers: HashMap<String, usize>,
    /// Sessions whose agent tab was resized and not typed to since: their
    /// output may be the program repainting.
    redrawing: HashSet<String>,
}

fn state() -> MutexGuard<'static, State> {
    READS.state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps the marks in the file at `path` from now on, starting from what it
/// holds. The daemon calls this once as it starts; a later call changes
/// nothing. A file that isn't there yet is made at the first mark.
pub(crate) fn open(path: &Path) -> Result<(), MarksError> {
    if READS.path.set(path.to_owned()).is_err() || !path.exists() {
        return Ok(());
    }
    let unread = file()?.map(|marks| marks.unread()).transpose()?.unwrap_or_default();
    let Some(marks) = file()? else {
        return Ok(());
    };
    let (pinned, _) = marks.pinned()?;
    let vips = marks.vips()?;
    let mut state = state();
    state.unread.extend(unread);
    state.pinned.extend(pinned.into_iter().filter_map(|mark| session_of(mark.thing)));
    state.vip.extend(vips.into_iter().filter_map(session_of));
    Ok(())
}

fn file() -> Result<Option<Marks>, MarksError> {
    if let Some(marks) = READS.file.get() {
        return Ok(Some(marks.clone()));
    }
    let Some(path) = READS.path.get() else {
        return Ok(None);
    };
    let opened = Marks::open(path)?;
    // Another thread may have opened it first; either copy will do.
    Ok(Some(READS.file.get_or_init(|| opened).clone()))
}

fn session_of(thing: humata2_marks::Thing) -> Option<String> {
    match thing {
        humata2_marks::Thing::Session(id) => Some(id),
        _ => None,
    }
}

/// Whether `session` is pinned (D387).
pub(crate) fn is_pinned(session: &str) -> bool {
    state().pinned.contains(session)
}

/// Whether `session` is marked VIP (D387).
pub(crate) fn is_vip(session: &str) -> bool {
    state().vip.contains(session)
}

/// MarkService saved `session`'s pin and VIP mark as these (D387). Tells
/// [`changes`] if either differs from what is held. Writes nothing: the
/// caller already saved them.
pub(crate) fn flags_changed(session: &str, pinned: bool, vip: bool) -> bool {
    let differs = {
        let mut state = state();
        let a = if pinned { state.pinned.insert(session.to_owned()) } else { state.pinned.remove(session) };
        let b = if vip { state.vip.insert(session.to_owned()) } else { state.vip.remove(session) };
        a || b
    };
    if differs {
        let _ = READS.changes.send(session.to_owned());
    }
    differs
}

/// Whether `session` is unread.
pub(crate) fn is_unread(session: &str) -> bool {
    state().unread.contains(session)
}

/// Each session whose read mark changes from now on.
pub(crate) fn changes() -> broadcast::Receiver<String> {
    READS.changes.subscribe()
}

/// The daemon resized (or a client attached to, at a size) `session`'s
/// agent tab, so the program is about to repaint (D393).
pub(crate) fn resized(session: &str) {
    state().redrawing.insert(session.to_owned());
}

/// Something was typed to `session`'s agent: what it prints next is its
/// answer, not a repaint.
pub(crate) fn typed(session: &str) {
    let mut state = state();
    if !state.redrawing.is_empty() {
        state.redrawing.remove(session);
    }
}

/// [`resized`], for a tab: only an agent tab of a session counts.
pub(crate) fn tab_resized(tab: &TabRecord) {
    if let (TabKind::Agent, Some(session)) = (tab.kind, &tab.session) {
        resized(session);
    }
}

/// [`typed`], for a tab.
pub(crate) fn tab_typed(tab: &TabRecord) {
    if let (TabKind::Agent, Some(session)) = (tab.kind, &tab.session) {
        typed(session);
    }
}

/// Whether output on `session` now would mark it: cheap, for the loop that
/// follows the agent, which calls [`heard_output`] only then.
/// `shows_new_text` says the output had a word the session hadn't printed
/// lately; output that doesn't, while the session is redrawing, marks
/// nothing. New text ends the redrawing.
pub(crate) fn would_mark(session: &str, shows_new_text: bool) -> bool {
    let mut state = state();
    if state.redrawing.contains(session) {
        if !shows_new_text {
            return false;
        }
        state.redrawing.remove(session);
    }
    !state.viewers.contains_key(session) && !state.unread.contains(session)
}

/// The agent of `session` printed something: unread unless a client views
/// it. Says whether that changed it. Writes the file, so call it off the
/// async threads.
pub(crate) fn heard_output(session: &str) -> bool {
    // The file first, then memory: a session never shows as unread before
    // its mark is kept, so a restart right after keeps it.
    let _writing = READS.writing.lock().unwrap_or_else(PoisonError::into_inner);
    {
        let state = state();
        if state.viewers.contains_key(session) || state.unread.contains(session) {
            return false;
        }
    }
    save(session, true);
    let kept = {
        let mut state = state();
        !state.viewers.contains_key(session) && state.unread.insert(session.to_owned())
    };
    if !kept {
        // A client began viewing the session meanwhile: it is read.
        save(session, false);
        return false;
    }
    let _ = READS.changes.send(session.to_owned());
    true
}

/// A client views `session` until the returned [`Viewing`] is dropped.
/// Clears its mark first and says whether it had one. Writes the file, so
/// call it off the async threads.
pub(crate) fn view(session: &str) -> (Viewing, bool) {
    let was_unread = {
        let mut state = state();
        *state.viewers.entry(session.to_owned()).or_default() += 1;
        state.unread.remove(session)
    };
    if was_unread {
        changed(session);
    }
    (
        Viewing {
            session: session.to_owned(),
        },
        was_unread,
    )
}

/// Whether a client views `session` now (D784: "open in a tab").
pub(crate) fn is_viewed(session: &str) -> bool {
    state().viewers.contains_key(session)
}

/// A view of a session, held while a client holds its ViewSession stream.
pub(crate) struct Viewing {
    session: String,
}

impl Drop for Viewing {
    fn drop(&mut self) {
        let mut state = state();
        if let Some(count) = state.viewers.get_mut(&self.session) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.viewers.remove(&self.session);
            }
        }
    }
}

/// Saves `session`'s mark as memory has it now, then tells [`changes`]. A
/// failed write is logged: the mark is right until humatad2 restarts.
fn changed(session: &str) {
    {
        let _writing = READS.writing.lock().unwrap_or_else(PoisonError::into_inner);
        save(session, is_unread(session));
    }
    // Nobody may be following yet; the mark is kept all the same.
    let _ = READS.changes.send(session.to_owned());
}

/// Writes `session`'s mark as `unread` to the file; the caller holds the
/// write lock. A failed write is logged.
fn save(session: &str, unread: bool) {
    let saved = file().and_then(|marks| match marks {
        Some(marks) => marks.set_unread(session, unread).map(|_| ()),
        None => Ok(()),
    });
    if let Err(error) = saved {
        crate::log::say!(
            "humatad2: session {session} shows as {}, but saving that failed, so it may show otherwise after humatad2 restarts: {error}",
            if unread { "unread" } else { "read" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The set is the process's, so each test uses sessions of its own. No
    // file is named here, so nothing is written.

    #[test]
    fn output_marks_a_session_nobody_views_once() {
        assert!(would_mark("se_rm_once", true));
        assert!(heard_output("se_rm_once"));
        assert!(is_unread("se_rm_once"));
        assert!(!would_mark("se_rm_once", true));
        assert!(!heard_output("se_rm_once"), "a second output changes nothing");
    }

    #[test]
    fn a_view_clears_the_mark_and_keeps_the_session_read_while_it_is_held() {
        heard_output("se_rm_view");
        let (viewing, was_unread) = view("se_rm_view");
        assert!(was_unread);
        assert!(!is_unread("se_rm_view"));
        assert!(!would_mark("se_rm_view", true));
        assert!(!heard_output("se_rm_view"), "output while viewed marks nothing");
        assert!(!is_unread("se_rm_view"));
        drop(viewing);
        assert!(heard_output("se_rm_view"), "output after the view ended marks it");
    }

    #[test]
    fn pin_and_vip_flags_are_kept_per_session_and_changes_are_told() {
        let mut told = changes();
        flags_changed("se_rm_flags", true, false);
        assert!(is_pinned("se_rm_flags") && !is_vip("se_rm_flags"));
        flags_changed("se_rm_flags", true, true);
        assert!(is_vip("se_rm_flags"));
        flags_changed("se_rm_flags", true, true);
        flags_changed("se_rm_flags", false, false);
        assert!(!is_pinned("se_rm_flags") && !is_vip("se_rm_flags"));
        assert!(!is_pinned("se_rm_flags_other"));
        let mut count = 0;
        while let Ok(session) = told.try_recv() {
            if session == "se_rm_flags" {
                count += 1;
            }
        }
        assert_eq!(count, 3, "only real changes are told");
    }

    #[test]
    fn a_session_stays_viewed_while_any_view_is_held() {
        let (first, _) = view("se_rm_two");
        let (second, was_unread) = view("se_rm_two");
        assert!(!was_unread);
        drop(first);
        assert!(!heard_output("se_rm_two"));
        drop(second);
        assert!(heard_output("se_rm_two"));
    }

    #[test]
    fn a_repaint_after_a_resize_marks_nothing_and_new_text_ends_the_redrawing() {
        resized("se_rm_redraw");
        assert!(!would_mark("se_rm_redraw", false));
        assert!(!would_mark("se_rm_redraw", false), "still redrawing");
        assert!(would_mark("se_rm_redraw", true), "new words mark");
        assert!(would_mark("se_rm_redraw", false), "and the redrawing is over");
    }

    #[test]
    fn typing_ends_the_redrawing_so_old_words_mark_again() {
        resized("se_rm_typed");
        typed("se_rm_typed");
        assert!(would_mark("se_rm_typed", false));
    }

    #[test]
    fn a_resize_of_one_session_leaves_the_others_marking() {
        resized("se_rm_other_a");
        assert!(would_mark("se_rm_other_b", false));
    }
}

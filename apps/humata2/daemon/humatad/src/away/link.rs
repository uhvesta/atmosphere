//! Whether the Mac can be reached from this daemon. A machine's daemon is
//! away until a `PeerUplink` is open; the host's own daemon is always
//! reachable to itself (D213).

use std::sync::Arc;

use humata2_access::{Context, MachineId};
use tokio::sync::watch;

/// Whether the host is reachable, and which open made it so. Each open
/// takes the next generation, so an older open can tell it was replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LinkState {
    reachable: bool,
    generation: u64,
}

/// The host link's state, shared by the uplinks and the guard.
pub(crate) struct HostLink {
    on_host: bool,
    state: watch::Sender<LinkState>,
    /// Counts the journal entries written, so an open uplink sends them as
    /// they come.
    journal: watch::Sender<u64>,
}

impl HostLink {
    /// The host's daemon starts reachable. A machine's starts away until an
    /// uplink opens.
    pub(crate) fn new(on_host: bool) -> Self {
        let (state, _) = watch::channel(LinkState {
            reachable: on_host,
            generation: 0,
        });
        let (journal, _) = watch::channel(0);
        Self { on_host, state, journal }
    }

    pub(crate) fn on_host(&self) -> bool {
        self.on_host
    }

    /// Says an entry was added to the journal.
    pub(crate) fn journal_written(&self) {
        self.journal.send_modify(|written| *written = written.wrapping_add(1));
    }

    /// Changes each time an entry is added to the journal.
    pub(crate) fn journal_writes(&self) -> watch::Receiver<u64> {
        self.journal.subscribe()
    }

    pub(crate) fn reachable(&self) -> bool {
        self.state.borrow().reachable
    }

    /// Where a check made from this daemon judges the host: here on the
    /// host, and reachable or away from a machine.
    pub(crate) fn context(&self, here: &MachineId) -> Context {
        if self.on_host {
            Context::on_host(here.clone())
        } else {
            Context::machine(here.clone(), self.reachable())
        }
    }

    /// Resolves once the host is reachable, at once if it already is.
    pub(crate) async fn until_reachable(&self) {
        let mut state = self.state.subscribe();
        let _ = state.wait_for(|state| state.reachable).await;
    }

    /// Opens a link, making the host reachable. A link already open is
    /// replaced: its [`Opened::replaced`] resolves.
    pub(crate) fn open(self: &Arc<Self>) -> Opened {
        let mut generation = 0;
        self.state.send_modify(|state| {
            state.generation = state.generation.wrapping_add(1);
            state.reachable = true;
            generation = state.generation;
        });
        Opened {
            link: Arc::clone(self),
            generation,
            done: false,
        }
    }
}

/// An open link. Closing it, or dropping it, ends it: unless a newer open
/// has replaced it, the host goes away.
pub(crate) struct Opened {
    link: Arc<HostLink>,
    generation: u64,
    /// Set once the link has ended, so it ends at most once.
    done: bool,
}

impl Opened {
    /// Resolves once a newer open has replaced this one.
    pub(crate) async fn replaced(&self) {
        let generation = self.generation;
        let mut state = self.link.state.subscribe();
        let _ = state
            .wait_for(|state| state.generation != generation)
            .await;
    }

    /// Ends this link. Returns true when that took the host away: no newer
    /// open has replaced it. A host's daemon never goes away, so there it
    /// returns false.
    pub(crate) fn close(mut self) -> bool {
        self.end()
    }

    fn end(&mut self) -> bool {
        if std::mem::replace(&mut self.done, true) {
            return false;
        }
        let generation = self.generation;
        let on_host = self.link.on_host;
        let mut went_away = false;
        self.link.state.send_modify(|state| {
            if state.generation == generation {
                state.reachable = on_host;
                went_away = !on_host;
            }
        });
        went_away
    }
}

impl Drop for Opened {
    /// A task that drops its link without closing it still leaves the host
    /// away, never reachable.
    fn drop(&mut self) {
        let _ = self.end();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    /// How long something that must not happen yet is given to happen.
    const SHORT: Duration = Duration::from_millis(100);
    /// The bound on something that must happen.
    const BOUND: Duration = Duration::from_secs(5);

    #[test]
    fn a_machine_starts_away() {
        let link = Arc::new(HostLink::new(false));
        assert!(!link.reachable());
    }

    #[test]
    fn an_open_makes_the_machine_reachable() {
        let link = Arc::new(HostLink::new(false));
        let _opened = link.open();
        assert!(link.reachable());
    }

    #[test]
    fn a_close_makes_the_machine_away_again() {
        let link = Arc::new(HostLink::new(false));
        let opened = link.open();
        assert!(opened.close());
        assert!(!link.reachable());
    }

    #[tokio::test]
    async fn a_newer_open_replaces_the_older_one() {
        let link = Arc::new(HostLink::new(false));
        let first = link.open();
        let second = link.open();
        tokio::time::timeout(BOUND, first.replaced())
            .await
            .expect("the newer open should replace the first one");
        assert!(!first.close(), "a replaced link takes nothing away");
        assert!(link.reachable(), "the newer open still holds the host");
        assert!(second.close());
        assert!(!link.reachable());
    }

    #[test]
    fn dropping_an_open_link_makes_the_machine_away() {
        let link = Arc::new(HostLink::new(false));
        let opened = link.open();
        drop(opened);
        assert!(!link.reachable());
    }

    #[test]
    fn a_host_is_reachable_before_and_after_an_open_and_close() {
        let link = Arc::new(HostLink::new(true));
        assert!(link.reachable());
        let opened = link.open();
        assert!(link.reachable());
        assert!(!opened.close(), "the host never goes away");
        assert!(link.reachable());
    }

    #[tokio::test]
    async fn until_reachable_waits_for_an_open() {
        let link = Arc::new(HostLink::new(false));
        assert!(
            tokio::time::timeout(SHORT, link.until_reachable())
                .await
                .is_err(),
            "a machine that is away keeps waiting until an open"
        );
        let _opened = link.open();
        tokio::time::timeout(BOUND, link.until_reachable())
            .await
            .expect("the open should make the host reachable");
    }
}

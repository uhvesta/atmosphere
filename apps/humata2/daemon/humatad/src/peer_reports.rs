//! What each machine reports on its Uplink, as the host keeps it (D139,
//! SL-3, TR-8, HO-5).
//!
//! The link feeds every `MachineToHost` message to [`PeerReports::accept`].
//! Status and pointer reports are kept in memory as the latest per session
//! per machine, and pushed to subscribers. A store table can be added behind
//! [`PeerReports`] later without changing its callers.
//!
//! Snapshots (D139: "every new link starts from a full snapshot"): today's
//! proto has no end-of-snapshot marker. The smallest workable rule is that
//! the reports received after [`PeerReports::link_up`] are the snapshot, and
//! it is complete at the first message of any other kind (an ack, journal
//! entry, mirror chunk...) or when the link calls
//! [`PeerReports::snapshot_complete`]. A completed snapshot replaces the
//! machine's earlier sessions: those it doesn't mention are reported gone.
//! A machine that sends no other message and has no caller of
//! `snapshot_complete` keeps its old sessions until it does. Wanted in the
//! proto: a `bool snapshot_complete` on `UplinkAck`, or a `SnapshotEnd`
//! message in `MachineToHost`, sent after the last report of the first pass.
//!
//! While a machine's link is down its last reports stay readable, marked
//! stale with when the link was last up (HO-5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use humata2_grpc::humata::v1 as pb;
use tokio::sync::broadcast;

use crate::machine_service::lock;

/// How many updates a slow subscriber may miss before it must re-read the
/// state with [`PeerReports::machine`] (TR-8: latest wins).
const UPDATE_BACKLOG: usize = 256;

/// The latest the machine said about one session.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SessionReport {
    pub(crate) status: Option<pb::SessionStatusReport>,
    pub(crate) pointers: Vec<pb::Pointer>,
    pub(crate) pointers_at: Option<humata2_grpc::Timestamp>,
}

/// Whether what is held was read over a live link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Freshness {
    Live,
    /// The link is down; `last_up` is when it was last known up.
    Stale { last_up: Option<SystemTime> },
}

/// A machine's sessions as last reported.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MachineReports {
    pub(crate) freshness: Freshness,
    pub(crate) sessions: BTreeMap<String, SessionReport>,
}

/// One change, pushed to subscribers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReportUpdate {
    Session {
        machine: String,
        session: String,
        report: SessionReport,
    },
    /// The machine no longer has the session (absent from a new snapshot).
    Gone { machine: String, session: String },
    /// The machine's link went up or down; held reports changed freshness.
    Freshness { machine: String, freshness: Freshness },
}

#[derive(Default)]
struct Machine {
    live: bool,
    last_up: Option<SystemTime>,
    sessions: BTreeMap<String, SessionReport>,
    /// While a snapshot is being received: the sessions it has named.
    snapshot: Option<BTreeSet<String>>,
}

impl Machine {
    fn freshness(&self) -> Freshness {
        if self.live {
            Freshness::Live
        } else {
            Freshness::Stale { last_up: self.last_up }
        }
    }
}

struct Inner {
    machines: Mutex<BTreeMap<String, Machine>>,
    updates: broadcast::Sender<Arc<ReportUpdate>>,
}

/// The host's copy of what machines report. Cheap to clone.
#[derive(Clone)]
pub(crate) struct PeerReports {
    inner: Arc<Inner>,
}

impl Default for PeerReports {
    fn default() -> Self {
        let (updates, _) = broadcast::channel(UPDATE_BACKLOG);
        PeerReports {
            inner: Arc::new(Inner {
                machines: Mutex::new(BTreeMap::new()),
                updates,
            }),
        }
    }
}

impl PeerReports {
    /// A new link is up: what arrives now is the machine's full snapshot.
    pub(crate) fn link_up(&self, machine: &str) {
        let mut machines = lock(&self.inner.machines);
        let held = machines.entry(machine.to_owned()).or_default();
        held.live = true;
        held.snapshot = Some(BTreeSet::new());
        self.push(ReportUpdate::Freshness {
            machine: machine.to_owned(),
            freshness: Freshness::Live,
        });
    }

    /// The link is gone at `at`. Held reports stay readable, stale. A
    /// snapshot cut short is dropped: nothing is reported gone from it.
    pub(crate) fn link_down(&self, machine: &str, at: SystemTime) {
        let mut machines = lock(&self.inner.machines);
        let held = machines.entry(machine.to_owned()).or_default();
        held.live = false;
        held.last_up = Some(at);
        held.snapshot = None;
        self.push(ReportUpdate::Freshness {
            machine: machine.to_owned(),
            freshness: held.freshness(),
        });
    }

    /// One message read from `machine`'s Uplink. Kinds this sink doesn't
    /// keep are ignored; any other kind ends a snapshot in progress.
    pub(crate) fn accept(&self, machine: &str, message: pb::MachineToHost) {
        use pb::machine_to_host::Message;
        let mut machines = lock(&self.inner.machines);
        let held = machines.entry(machine.to_owned()).or_default();
        match message.message {
            Some(Message::Status(report)) => {
                let Some(session) = report.session.as_ref().map(|id| id.value.clone()) else {
                    return;
                };
                let entry = held.sessions.entry(session.clone()).or_default();
                entry.status = Some(report);
                self.changed(machine, held, session);
            }
            Some(Message::Pointers(report)) => {
                let Some(session) = report.session.as_ref().map(|id| id.value.clone()) else {
                    return;
                };
                let entry = held.sessions.entry(session.clone()).or_default();
                entry.pointers = report.pointers;
                entry.pointers_at = report.at;
                self.changed(machine, held, session);
            }
            Some(_) => self.finish(machine, held),
            None => {}
        }
    }

    /// The machine's first pass of reports is over (see the module doc).
    pub(crate) fn snapshot_complete(&self, machine: &str) {
        let mut machines = lock(&self.inner.machines);
        if let Some(held) = machines.get_mut(machine) {
            self.finish(machine, held);
        }
    }

    /// Everything held for `machine`, with whether it is live.
    pub(crate) fn machine(&self, machine: &str) -> Option<MachineReports> {
        lock(&self.inner.machines).get(machine).map(|held| MachineReports {
            freshness: held.freshness(),
            sessions: held.sessions.clone(),
        })
    }

    /// One session's latest reports and whether they are live.
    pub(crate) fn session(&self, machine: &str, session: &str) -> Option<(SessionReport, Freshness)> {
        let machines = lock(&self.inner.machines);
        let held = machines.get(machine)?;
        Some((held.sessions.get(session)?.clone(), held.freshness()))
    }

    /// Changes as they happen. A receiver that lags has missed some: it
    /// reads [`PeerReports::machine`] again, which holds the latest.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<ReportUpdate>> {
        self.inner.updates.subscribe()
    }

    /// `machine` was removed: its reports go.
    pub(crate) fn forget(&self, machine: &str) {
        let removed = lock(&self.inner.machines).remove(machine);
        for session in removed.into_iter().flat_map(|held| held.sessions.into_keys()) {
            self.push(ReportUpdate::Gone {
                machine: machine.to_owned(),
                session,
            });
        }
    }

    fn changed(&self, machine: &str, held: &mut Machine, session: String) {
        if let Some(named) = held.snapshot.as_mut() {
            named.insert(session.clone());
        }
        if let Some(report) = held.sessions.get(&session) {
            self.push(ReportUpdate::Session {
                machine: machine.to_owned(),
                session,
                report: report.clone(),
            });
        }
    }

    fn finish(&self, machine: &str, held: &mut Machine) {
        let Some(named) = held.snapshot.take() else {
            return;
        };
        let gone: Vec<String> = held.sessions.keys().filter(|session| !named.contains(*session)).cloned().collect();
        for session in gone {
            held.sessions.remove(&session);
            self.push(ReportUpdate::Gone {
                machine: machine.to_owned(),
                session,
            });
        }
    }

    fn push(&self, update: ReportUpdate) {
        let _ = self.inner.updates.send(Arc::new(update));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const MACHINE: &str = "devbox";

    fn id(value: &str) -> Option<pb::SessionId> {
        Some(pb::SessionId { value: value.to_owned() })
    }

    fn status(session: &str, status: pb::SessionStatus) -> pb::MachineToHost {
        pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Status(pb::SessionStatusReport {
                session: id(session),
                status: status as i32,
                ..Default::default()
            })),
        }
    }

    fn pointers(session: &str, branch: &str) -> pb::MachineToHost {
        pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Pointers(pb::PointersReport {
                session: id(session),
                pointers: vec![pb::Pointer {
                    branch: branch.to_owned(),
                    ..Default::default()
                }],
                at: None,
            })),
        }
    }

    fn ack() -> pb::MachineToHost {
        pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Ack(pb::UplinkAck::default())),
        }
    }

    fn held_status(reports: &PeerReports, session: &str) -> Option<i32> {
        let (report, _) = reports.session(MACHINE, session)?;
        report.status.map(|status| status.status)
    }

    #[test]
    fn the_latest_report_per_session_wins() {
        let reports = PeerReports::default();
        reports.link_up(MACHINE);
        reports.accept(MACHINE, status("se_1", pb::SessionStatus::Working));
        reports.accept(MACHINE, status("se_1", pb::SessionStatus::Idle));
        reports.accept(MACHINE, pointers("se_1", "one"));
        reports.accept(MACHINE, pointers("se_1", "two"));

        let (report, freshness) = reports.session(MACHINE, "se_1").unwrap();

        assert_eq!(held_status(&reports, "se_1"), Some(pb::SessionStatus::Idle as i32));
        assert_eq!(report.pointers.len(), 1);
        assert_eq!(report.pointers[0].branch, "two");
        assert_eq!(freshness, Freshness::Live);
    }

    #[test]
    fn a_new_links_snapshot_replaces_the_old_one_and_reports_the_missing_session_gone() {
        let reports = PeerReports::default();
        reports.link_up(MACHINE);
        reports.accept(MACHINE, status("se_1", pb::SessionStatus::Working));
        reports.accept(MACHINE, status("se_2", pb::SessionStatus::Working));
        reports.snapshot_complete(MACHINE);
        reports.link_down(MACHINE, SystemTime::UNIX_EPOCH);
        let mut updates = reports.subscribe();

        reports.link_up(MACHINE);
        reports.accept(MACHINE, status("se_2", pb::SessionStatus::Idle));
        // Still held until the snapshot is complete.
        assert!(reports.session(MACHINE, "se_1").is_some());
        // Any other kind of message ends the snapshot.
        reports.accept(MACHINE, ack());

        assert!(reports.session(MACHINE, "se_1").is_none());
        assert_eq!(held_status(&reports, "se_2"), Some(pb::SessionStatus::Idle as i32));
        let mut gone = Vec::new();
        while let Ok(update) = updates.try_recv() {
            if let ReportUpdate::Gone { session, .. } = &*update {
                gone.push(session.clone());
            }
        }
        assert_eq!(gone, vec!["se_1".to_owned()]);
    }

    #[test]
    fn a_link_that_drops_mid_snapshot_removes_nothing() {
        let reports = PeerReports::default();
        reports.link_up(MACHINE);
        reports.accept(MACHINE, status("se_1", pb::SessionStatus::Working));
        reports.snapshot_complete(MACHINE);
        reports.link_down(MACHINE, SystemTime::UNIX_EPOCH);
        reports.link_up(MACHINE);
        reports.link_down(MACHINE, SystemTime::UNIX_EPOCH + Duration::from_secs(5));

        reports.snapshot_complete(MACHINE);

        assert!(reports.session(MACHINE, "se_1").is_some());
    }

    #[test]
    fn reports_stay_readable_but_stale_after_the_link_goes_down() {
        let reports = PeerReports::default();
        reports.link_up(MACHINE);
        reports.accept(MACHINE, status("se_1", pb::SessionStatus::Working));
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(42);

        reports.link_down(MACHINE, at);

        let held = reports.machine(MACHINE).unwrap();
        assert_eq!(held.freshness, Freshness::Stale { last_up: Some(at) });
        assert!(held.sessions.contains_key("se_1"));
        reports.link_up(MACHINE);
        assert_eq!(reports.machine(MACHINE).unwrap().freshness, Freshness::Live);
    }

    #[test]
    fn other_kinds_and_reports_without_a_session_are_ignored() {
        let reports = PeerReports::default();
        reports.link_up(MACHINE);

        reports.accept(MACHINE, pb::MachineToHost { message: None });
        reports.accept(
            MACHINE,
            pb::MachineToHost {
                message: Some(pb::machine_to_host::Message::Status(pb::SessionStatusReport::default())),
            },
        );

        assert!(reports.machine(MACHINE).unwrap().sessions.is_empty());
    }

    #[test]
    fn subscribers_are_told_of_each_report() {
        let reports = PeerReports::default();
        let mut updates = reports.subscribe();
        reports.link_up(MACHINE);

        reports.accept(MACHINE, status("se_1", pb::SessionStatus::Working));

        let mut sessions = Vec::new();
        while let Ok(update) = updates.try_recv() {
            if let ReportUpdate::Session { session, .. } = &*update {
                sessions.push(session.clone());
            }
        }
        assert_eq!(sessions, vec!["se_1".to_owned()]);
    }
}

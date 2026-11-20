//! Select mode: which rows are ticked, which forks go along, the anchor for
//! ranges and the counts the action bar shows (D875).

use std::collections::{BTreeMap, BTreeSet};

use humata2_clients::model::{ChannelId, MachineId, SessionId};

use crate::MassDeleteState;

/// Where select mode was entered. Leaving it exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The sidebar.
    Sidebar,
    /// Home.
    Home,
    /// A machine's page.
    Machine(MachineId),
}

/// One session as the lists show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The session.
    pub id: SessionId,
    /// Its channel.
    pub channel: ChannelId,
    /// Its host.
    pub machine: MachineId,
    /// Set on a context-only fork: the parent whose folder it works in.
    pub context_fork_of: Option<SessionId>,
    /// Whether it is archived.
    pub archived: bool,
    /// Whether it is live (not archived).
    pub live: bool,
    /// Whether it is a Desk session: those are never picked.
    pub desk: bool,
}

/// What select mode ticks from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Every session known.
    pub rows: Vec<Row>,
    /// Machines that can't be reached: they can't be ticked as a whole.
    pub offline: BTreeSet<MachineId>,
    /// The names people see for machines, for the toast; an id stands in
    /// for a machine with no name.
    pub machine_names: BTreeMap<MachineId, String>,
}

impl Listing {
    fn row(&self, id: &SessionId) -> Option<&Row> {
        self.rows.iter().find(|row| &row.id == id)
    }

    /// The name people see for a machine.
    #[must_use]
    pub fn machine_name(&self, machine: &MachineId) -> String {
        self.machine_names
            .get(machine)
            .cloned()
            .unwrap_or_else(|| machine.to_string())
    }
}

impl MassDeleteState {
    /// Whether select mode is on.
    #[must_use]
    pub fn selecting(&self) -> bool {
        self.surface.is_some()
    }

    /// The context forks that go along with picked sessions: ticked for the
    /// person, and not clearable.
    #[must_use]
    pub fn forks_along(&self) -> BTreeSet<SessionId> {
        let mut along: BTreeSet<SessionId> = BTreeSet::new();
        loop {
            let before = along.len();
            for row in &self.listing.rows {
                let Some(parent) = &row.context_fork_of else {
                    continue;
                };
                if !row.desk && (self.picked.contains(parent) || along.contains(parent)) {
                    along.insert(row.id.clone());
                }
            }
            if along.len() == before {
                return along;
            }
        }
    }

    /// "N selected": the picks and the forks that go along.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.picked.len() + self.forks_along().len()
    }

    /// "p picked".
    #[must_use]
    pub fn picked_count(&self) -> usize {
        self.picked.len()
    }

    /// "k forks go along".
    #[must_use]
    pub fn fork_count(&self) -> usize {
        self.forks_along().len()
    }

    /// Whether a row shows a tick, picked or along.
    #[must_use]
    pub fn is_ticked(&self, id: &SessionId) -> bool {
        self.picked.contains(id) || self.forks_along().contains(id)
    }

    /// Whether a row's tick is the paler one that can't be cleared.
    #[must_use]
    pub fn is_along(&self, id: &SessionId) -> bool {
        self.forks_along().contains(id)
    }

    /// Whether a row can be ticked at all: Desk sessions can't.
    #[must_use]
    pub fn can_pick(&self, id: &SessionId) -> bool {
        self.listing.row(id).is_some_and(|row| !row.desk)
    }

    /// Whether a machine can be ticked as a whole: it is reachable and has
    /// live sessions.
    #[must_use]
    pub fn can_pick_machine(&self, machine: &MachineId) -> bool {
        !self.listing.offline.contains(machine)
            && self.listing.rows.iter().any(|row| self.bulk_row(row) && &row.machine == machine)
    }

    /// Whether a channel can be ticked as a whole: it has live sessions on
    /// reachable machines.
    #[must_use]
    pub fn can_pick_channel(&self, channel: &ChannelId) -> bool {
        self.listing.rows.iter().any(|row| self.bulk_row(row) && &row.channel == channel)
    }

    /// A row that channel, machine and "all" picks take: live, not Desk, on
    /// a reachable machine.
    fn bulk_row(&self, row: &Row) -> bool {
        row.live && !row.archived && !row.desk && !self.listing.offline.contains(&row.machine)
    }

    fn pick(&mut self, id: &SessionId) {
        if self.can_pick(id) && !self.is_along(id) {
            self.picked.insert(id.clone());
        }
    }

    /// Drops picks the forks that go along now cover.
    fn settle(&mut self) {
        let along = self.forks_along();
        self.picked.retain(|id| !along.contains(id));
    }

    pub(crate) fn show(&mut self, listing: Listing) {
        self.listing = listing;
        let known: BTreeSet<SessionId> =
            self.listing.rows.iter().map(|row| row.id.clone()).collect();
        self.picked.retain(|id| known.contains(id));
        if self.anchor.as_ref().is_some_and(|anchor| !known.contains(anchor)) {
            self.anchor = None;
        }
        self.settle();
    }

    pub(crate) fn enter(&mut self, surface: Surface, first: SessionId) {
        self.surface = Some(surface);
        self.picked.clear();
        self.anchor = None;
        if self.can_pick(&first) {
            self.picked.insert(first.clone());
            self.anchor = Some(first);
        }
    }

    pub(crate) fn enter_empty(&mut self, surface: Surface) {
        self.surface = Some(surface);
        self.picked.clear();
        self.anchor = None;
    }

    pub(crate) fn toggle(&mut self, id: &SessionId) {
        if !self.selecting() || !self.can_pick(id) || self.is_along(id) {
            return;
        }
        if !self.picked.remove(id) {
            self.picked.insert(id.clone());
            self.settle();
        }
        self.anchor = Some(id.clone());
    }

    pub(crate) fn range_to(&mut self, to: &SessionId, visible: &[SessionId]) {
        if !self.selecting() {
            return;
        }
        let end = visible.iter().position(|id| id == to);
        let start = self
            .anchor
            .as_ref()
            .and_then(|anchor| visible.iter().position(|id| id == anchor));
        let (Some(end), Some(start)) = (end, start) else {
            self.toggle(to);
            return;
        };
        let (low, high) = if start <= end { (start, end) } else { (end, start) };
        for id in &visible[low..=high] {
            self.pick(id);
        }
        self.settle();
    }

    pub(crate) fn select_visible(&mut self, visible: &[SessionId]) {
        if !self.selecting() {
            return;
        }
        let rows: Vec<SessionId> = visible
            .iter()
            .filter(|id| self.listing.row(id).is_some_and(|row| self.bulk_row(row)))
            .cloned()
            .collect();
        for id in &rows {
            self.pick(id);
        }
        self.settle();
    }

    pub(crate) fn select_channel(&mut self, channel: &ChannelId) {
        if !self.selecting() {
            return;
        }
        let ids: Vec<SessionId> = self
            .listing
            .rows
            .iter()
            .filter(|row| self.bulk_row(row) && &row.channel == channel)
            .map(|row| row.id.clone())
            .collect();
        for id in &ids {
            self.pick(id);
        }
        self.settle();
    }

    pub(crate) fn select_machine(&mut self, machine: &MachineId) {
        if !self.selecting() || !self.can_pick_machine(machine) {
            return;
        }
        let ids: Vec<SessionId> = self
            .listing
            .rows
            .iter()
            .filter(|row| self.bulk_row(row) && &row.machine == machine)
            .map(|row| row.id.clone())
            .collect();
        for id in &ids {
            self.pick(id);
        }
        self.settle();
    }

    pub(crate) fn select_nothing(&mut self) {
        self.picked.clear();
        self.anchor = None;
    }

    pub(crate) fn exit(&mut self) {
        self.surface = None;
        self.picked.clear();
        self.anchor = None;
    }
}

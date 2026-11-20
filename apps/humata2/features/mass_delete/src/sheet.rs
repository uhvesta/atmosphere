//! The delete sheet: the plan, the per-session "Delete that work too" ticks,
//! the typed count and the stale notice (D875, NV-27).

use std::collections::BTreeSet;

use humata2_clients::DaemonError;
use humata2_clients::mass_delete::{
    DeleteScope, DeleteSessionsOutcome, DeleteSessionsPlan, SessionToDelete,
};
use humata2_clients::model::{MachineId, SessionId};
use tca::Effect;

use crate::{Delegate, MassDeleteAction, MassDeleteState};

/// What changed under a plan the person had reviewed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stale {
    /// One line per change, in words.
    pub changes: Vec<String>,
    /// The sessions that are new or differ: tagged "changed".
    pub changed: BTreeSet<SessionId>,
}

/// Which line sits beside the footer's buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FooterHint {
    /// Tick "Delete that work too" for this one session.
    TickOne(SessionId),
    /// Tick it for this many sessions.
    TickMany(usize),
    /// Tick it for this many sessions and type the count.
    TickAndType {
        /// Sessions still unticked.
        sessions: usize,
        /// The number to type.
        count: u32,
    },
    /// Type the count.
    Type(u32),
    /// Nothing left to do: Return deletes.
    Ready,
    /// No session can be reached: nothing can be deleted.
    NothingReachable,
}

/// The sheet, open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteSheet {
    /// What it covers.
    pub scope: DeleteScope,
    /// The plan, once loaded.
    pub plan: Option<DeleteSessionsPlan>,
    /// The plan is being loaded.
    pub loading: bool,
    /// Sessions whose work the person agreed to delete.
    pub work_ticks: BTreeSet<SessionId>,
    /// What is typed in the count field: digits only.
    pub typed: String,
    /// Set when the plan changed under the person.
    pub stale: Option<Stale>,
    /// The delete is in flight.
    pub deleting: bool,
    /// Why the plan or the delete failed.
    pub error: Option<String>,
}

impl DeleteSheet {
    pub(crate) fn loading(scope: DeleteScope) -> Self {
        Self {
            scope,
            plan: None,
            loading: true,
            work_ticks: BTreeSet::new(),
            typed: String::new(),
            stale: None,
            deleting: false,
            error: None,
        }
    }

    /// How many sessions the plan deletes; unreachable ones are not counted.
    #[must_use]
    pub fn planned_count(&self) -> u32 {
        let count = self.plan.as_ref().map_or(0, |plan| plan.sessions.len());
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Whether the person types the count to delete.
    #[must_use]
    pub fn needs_count(&self) -> bool {
        matches!(self.scope, DeleteScope::All) || self.plan.as_ref().is_some_and(|plan| plan.needs_count)
    }

    /// The number typed, if it is one.
    #[must_use]
    pub fn typed_count(&self) -> Option<u32> {
        self.typed.parse().ok()
    }

    /// The planned sessions that hold work that exists only there.
    #[must_use]
    pub fn sessions_with_work(&self) -> Vec<&SessionToDelete> {
        self.plan
            .iter()
            .flat_map(|plan| plan.sessions.iter())
            .filter(|session| !session.unpushed.is_empty())
            .collect()
    }

    /// The sessions with work whose box is not yet ticked.
    #[must_use]
    pub fn missing_work_ticks(&self) -> Vec<SessionId> {
        self.sessions_with_work()
            .into_iter()
            .filter(|session| !self.work_ticks.contains(&session.session))
            .map(|session| session.session.clone())
            .collect()
    }

    fn count_ok(&self) -> bool {
        !self.needs_count() || self.typed_count() == Some(self.planned_count())
    }

    /// Whether Delete is on.
    #[must_use]
    pub fn can_confirm(&self) -> bool {
        self.plan.is_some()
            && !self.loading
            && !self.deleting
            && self.planned_count() > 0
            && self.missing_work_ticks().is_empty()
            && self.count_ok()
    }

    /// Whether the delete says to delete unpushed work too: every session
    /// with work is ticked.
    #[must_use]
    pub fn delete_unpushed_work(&self) -> bool {
        !self.sessions_with_work().is_empty() && self.missing_work_ticks().is_empty()
    }

    /// The line beside the buttons; `None` while the plan loads.
    #[must_use]
    pub fn hint(&self) -> Option<FooterHint> {
        let plan = self.plan.as_ref()?;
        if plan.sessions.is_empty() {
            return Some(FooterHint::NothingReachable);
        }
        let missing = self.missing_work_ticks();
        let count_ok = self.count_ok();
        Some(match (missing.len(), count_ok) {
            (0, true) => FooterHint::Ready,
            (0, false) => FooterHint::Type(self.planned_count()),
            (1, true) => FooterHint::TickOne(missing[0].clone()),
            (k, true) => FooterHint::TickMany(k),
            (k, false) => FooterHint::TickAndType {
                sessions: k,
                count: self.planned_count(),
            },
        })
    }

    /// Whether a session carries the "changed" tag.
    #[must_use]
    pub fn is_changed(&self, session: &SessionId) -> bool {
        self.stale.as_ref().is_some_and(|stale| stale.changed.contains(session))
    }
}

fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

impl MassDeleteState {
    pub(crate) fn planned(&mut self, result: Result<DeleteSessionsPlan, DaemonError>) {
        let Some(sheet) = self.sheet.as_mut() else {
            return;
        };
        sheet.loading = false;
        match result {
            Ok(plan) => sheet.plan = Some(plan),
            Err(error) => sheet.error = Some(error.to_string()),
        }
    }

    pub(crate) fn tick_work(&mut self, id: &SessionId) {
        let Some(sheet) = self.sheet.as_mut() else {
            return;
        };
        if sheet.deleting || !sheet.sessions_with_work().iter().any(|s| &s.session == id) {
            return;
        }
        if !sheet.work_ticks.remove(id) {
            sheet.work_ticks.insert(id.clone());
        }
    }

    pub(crate) fn typed(&mut self, text: &str) {
        let Some(sheet) = self.sheet.as_mut() else {
            return;
        };
        if !sheet.deleting {
            sheet.typed = text.chars().filter(char::is_ascii_digit).take(9).collect();
        }
    }

    pub(crate) fn deleted(
        &mut self,
        result: Result<DeleteSessionsOutcome, DaemonError>,
    ) -> Effect<MassDeleteAction> {
        let Some(sheet) = self.sheet.as_mut() else {
            return Effect::none();
        };
        match result {
            Ok(DeleteSessionsOutcome::Deleted { sessions }) => {
                let toast = self.toast(sessions);
                self.sheet = None;
                self.exit();
                Effect::send(MassDeleteAction::Delegate(Delegate::Toast(toast)))
            }
            Ok(DeleteSessionsOutcome::Changed { plan, changes }) => {
                let changed = changed_sessions(sheet.plan.as_ref(), &plan);
                let planned_before = sheet.planned_count();
                sheet.work_ticks.retain(|id| {
                    !changed.contains(id) && plan.sessions.iter().any(|s| &s.session == id)
                });
                sheet.plan = Some(plan);
                if sheet.planned_count() != planned_before {
                    sheet.typed.clear();
                }
                sheet.stale = Some(Stale { changes, changed });
                sheet.deleting = false;
                sheet.error = None;
                Effect::none()
            }
            Err(error) => {
                sheet.deleting = false;
                sheet.error = Some(error.to_string());
                Effect::none()
            }
        }
    }

    /// "Deleted N sessions. They are in the trash on {machines} for 30 days."
    fn toast(&self, deleted: u32) -> String {
        let Some(sheet) = self.sheet.as_ref() else {
            return String::new();
        };
        let mut machines: Vec<MachineId> = Vec::new();
        let mut left_alone = Vec::new();
        if let Some(plan) = sheet.plan.as_ref() {
            for session in &plan.sessions {
                if !machines.contains(&session.machine) {
                    machines.push(session.machine.clone());
                }
            }
            for left in &plan.left_alone {
                left_alone.push((self.listing.machine_name(&left.machine), left.sessions.len()));
            }
        }
        let names: Vec<String> = machines.iter().map(|m| self.listing.machine_name(m)).collect();
        let noun = if deleted == 1 { "session" } else { "sessions" };
        let verb = if deleted == 1 { "It is" } else { "They are" };
        let mut toast = format!(
            "Deleted {deleted} {noun}. {verb} in the trash on {} for 30 days.",
            join_names(&names)
        );
        for (machine, count) in left_alone {
            let were = if count == 1 { "was" } else { "were" };
            toast = format!("{toast} {count} on {machine} {were} left alone.");
        }
        toast
    }
}

/// The sessions in the new plan that are not in the old one or differ.
fn changed_sessions(old: Option<&DeleteSessionsPlan>, new: &DeleteSessionsPlan) -> BTreeSet<SessionId> {
    new.sessions
        .iter()
        .filter(|session| {
            old.and_then(|plan| plan.sessions.iter().find(|s| s.session == session.session))
                != Some(*session)
        })
        .map(|session| session.session.clone())
        .collect()
}

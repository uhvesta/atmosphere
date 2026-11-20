//! Deleting many sessions (MASSDELETE, D875): select mode on the sidebar,
//! Home or a machine's page, and the one sheet that shows the plan and
//! deletes. State, actions and effects only.

mod selection;
mod sheet;

use std::sync::Arc;

use humata2_clients::mass_delete::{DeleteScope, DeleteSessionsOutcome, DeleteSessionsPlan};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{Daemon, DaemonError};
use tca::{Effect, Reducer};

pub use selection::{Listing, Row, Surface};
pub use sheet::{DeleteSheet, FooterHint, Stale};

/// What this feature asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Say this in a toast.
    Toast(String),
}

/// Select mode and the delete sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MassDeleteState {
    /// The sessions the lists show, for ticking.
    pub listing: Listing,
    /// The surface select mode was entered on; `None` when not selecting.
    pub surface: Option<Surface>,
    /// The sessions the person ticked, without the forks that go along.
    pub picked: std::collections::BTreeSet<SessionId>,
    /// The row a shift-click range starts from.
    pub anchor: Option<SessionId>,
    /// The sheet, while it is open.
    pub sheet: Option<DeleteSheet>,
}

/// What happens in select mode and on its sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum MassDeleteAction {
    /// The lists changed.
    Show(Listing),
    /// Enter select mode on a surface with a first tick.
    Enter {
        /// Where.
        surface: Surface,
        /// The row that was clicked.
        first: SessionId,
    },
    /// Enter select mode on a surface with nothing ticked.
    EnterEmpty(Surface),
    /// Tick or untick one row.
    Toggle(SessionId),
    /// Shift-click: tick the rows between the anchor and this one, in the
    /// order the surface shows them.
    RangeTo {
        /// The row clicked.
        to: SessionId,
        /// The rows as shown, top to bottom.
        visible: Vec<SessionId>,
    },
    /// Tick every live row shown.
    SelectAllVisible(Vec<SessionId>),
    /// Tick a channel's live sessions.
    SelectChannel(ChannelId),
    /// Tick a machine's live sessions.
    SelectMachine(MachineId),
    /// Clear the ticks and stay in select mode.
    SelectNothing,
    /// The person went to this surface: leaving the one select mode was
    /// entered on exits it.
    SurfaceChanged(Surface),
    /// Leave select mode.
    Exit,
    /// Escape: closes the sheet if open, else leaves select mode.
    Escape,
    /// Open the sheet for the picked sessions.
    OpenPicked,
    /// Open the sheet for every session ("Delete all sessions…").
    OpenAll,
    /// The plan came back.
    Planned(Result<DeleteSessionsPlan, DaemonError>),
    /// Tick or untick "Delete that work too" for one session.
    TickWork(SessionId),
    /// The count field changed.
    Typed(String),
    /// Delete.
    Confirm,
    /// The delete answered.
    Deleted(Result<DeleteSessionsOutcome, DaemonError>),
    /// Close the sheet, keeping the ticks.
    Cancel,
    /// For the parent.
    Delegate(Delegate),
}

/// The reducer.
#[derive(Clone)]
pub struct MassDelete {
    daemon: Arc<dyn Daemon>,
}

impl MassDelete {
    /// A reducer that plans and deletes with `daemon`.
    #[must_use]
    pub fn new(daemon: Arc<dyn Daemon>) -> Self {
        Self { daemon }
    }

    fn open(&self, state: &mut MassDeleteState, scope: DeleteScope) -> Effect<MassDeleteAction> {
        if state.sheet.is_some() {
            return Effect::none();
        }
        let reply = self.daemon.plan_delete_sessions(&scope);
        state.sheet = Some(DeleteSheet::loading(scope));
        Effect::future(async move { MassDeleteAction::Planned(reply.await) })
    }
}

impl Reducer for MassDelete {
    type State = MassDeleteState;
    type Action = MassDeleteAction;

    fn reduce(&self, state: &mut MassDeleteState, action: MassDeleteAction) -> Effect<MassDeleteAction> {
        use MassDeleteAction as A;
        match action {
            A::Show(listing) => state.show(listing),
            A::Enter { surface, first } => state.enter(surface, first),
            A::EnterEmpty(surface) => state.enter_empty(surface),
            A::Toggle(id) => state.toggle(&id),
            A::RangeTo { to, visible } => state.range_to(&to, &visible),
            A::SelectAllVisible(visible) => state.select_visible(&visible),
            A::SelectChannel(channel) => state.select_channel(&channel),
            A::SelectMachine(machine) => state.select_machine(&machine),
            A::SelectNothing => state.select_nothing(),
            A::SurfaceChanged(surface) => {
                if state.surface.as_ref().is_some_and(|entered| *entered != surface) {
                    state.exit();
                }
            }
            A::Exit => state.exit(),
            A::Escape => {
                if state.sheet.as_ref().is_some_and(|sheet| !sheet.deleting) {
                    state.sheet = None;
                } else if state.sheet.is_none() {
                    state.exit();
                }
            }
            A::OpenPicked => {
                if state.surface.is_none() || state.picked.is_empty() {
                    return Effect::none();
                }
                let ids = state.picked.iter().cloned().collect();
                return self.open(state, DeleteScope::Picked(ids));
            }
            A::OpenAll => return self.open(state, DeleteScope::All),
            A::Planned(result) => state.planned(result),
            A::TickWork(id) => state.tick_work(&id),
            A::Typed(text) => state.typed(&text),
            A::Confirm => return self.confirm(state),
            A::Deleted(result) => return state.deleted(result),
            A::Cancel => {
                if state.sheet.as_ref().is_some_and(|sheet| !sheet.deleting) {
                    state.sheet = None;
                }
            }
            A::Delegate(_) => {}
        }
        Effect::none()
    }
}

impl MassDelete {
    fn confirm(&self, state: &mut MassDeleteState) -> Effect<MassDeleteAction> {
        let Some(sheet) = state.sheet.as_mut() else {
            return Effect::none();
        };
        if !sheet.can_confirm() {
            return Effect::none();
        }
        let Some(plan) = sheet.plan.as_ref() else {
            return Effect::none();
        };
        let reply = self.daemon.delete_sessions(
            &plan.token,
            sheet.delete_unpushed_work(),
            sheet.typed_count().unwrap_or(0),
        );
        sheet.deleting = true;
        sheet.error = None;
        Effect::future(async move { MassDeleteAction::Deleted(reply.await) })
    }
}

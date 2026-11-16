//! The Pinned section (NV-20, NV-21, NV-24, NV-25): the pins in the
//! person's order, kept on the host and followed as other clients change
//! them.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::marks::{MarksClient, PinnedList, Thing};
use tca::{Effect, Reducer};

/// The id of the effect that follows the host's pins.
const WATCH: &str = "pinned-watch";

/// The Pinned section's state.
#[derive(Clone, Debug, PartialEq)]
pub struct PinnedState {
    /// The pins, as the host last gave them or as a move just left them.
    pub list: PinnedList,
    /// Whether the sidebar shows the section (Settings › Sidebar shows
    /// Pinned, ST-18). The parent sets it.
    pub shown: bool,
    /// What went wrong with the last change or load.
    pub error: Option<String>,
}

impl Default for PinnedState {
    fn default() -> Self {
        Self {
            list: PinnedList::default(),
            shown: true,
            error: None,
        }
    }
}

/// What the Pinned section asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinnedDelegate {
    /// Open a pinned thing.
    Open(Thing),
}

/// What happens in the Pinned section.
#[derive(Clone, Debug, PartialEq)]
pub enum PinnedAction {
    /// The section appeared: load the pins and follow their changes.
    Start,
    /// The host answered a load or a change with the pins, or didn't. A
    /// list replaces the state's and clears the error, unless it is older
    /// (a lower revision) than the one there; a failure shows as the error
    /// and leaves the list.
    Loaded(Result<PinnedList, DaemonError>),
    /// The pins changed, on this client or another. Taken as a list from
    /// [`PinnedAction::Loaded`] is.
    Changed(PinnedList),
    /// Pin or unpin a thing. The list changes when the host answers.
    Toggle {
        /// The thing.
        thing: Thing,
        /// Pin it, or unpin it.
        pin: bool,
    },
    /// Mark a thing VIP, or not. The list changes when the host answers.
    Vip {
        /// The thing.
        thing: Thing,
        /// VIP, or not.
        on: bool,
    },
    /// Move the pin at `from` so it ends at `to`, both places in the list.
    /// The list changes at once and the host is asked for the new order,
    /// with the revision the list had. When the host refuses, because the
    /// pins changed elsewhere or for any other reason, the pins are loaded
    /// again so the list is the host's. The same place twice, or a place
    /// past the end, does nothing and asks nothing.
    Move {
        /// Where the pin is.
        from: usize,
        /// Where it goes.
        to: usize,
    },
    /// The pin at this place was picked: it opens. Past the end, nothing
    /// happens.
    Open(usize),
    /// For the parent.
    Delegate(PinnedDelegate),
}

/// The Pinned section's reducer.
#[derive(Clone)]
pub struct PinnedSection {
    marks: Arc<dyn MarksClient>,
}

impl PinnedSection {
    /// A Pinned section over the host's marks.
    #[must_use]
    pub fn new(marks: Arc<dyn MarksClient>) -> Self {
        Self { marks }
    }
}

impl Reducer for PinnedSection {
    type State = PinnedState;
    type Action = PinnedAction;

    fn reduce(&self, state: &mut PinnedState, action: PinnedAction) -> Effect<PinnedAction> {
        match action {
            PinnedAction::Start => {
                let load = self.marks.pinned();
                let marks = self.marks.clone();
                Effect::merge([
                    Effect::future(async move { PinnedAction::Loaded(load.await) }),
                    Effect::run(move |send| async move {
                        let mut changes = match marks.watch().await {
                            Ok(changes) => changes,
                            Err(error) => {
                                send.send(PinnedAction::Loaded(Err(error)));
                                return;
                            }
                        };
                        loop {
                            send.ready().await;
                            match changes.recv().await {
                                Some(list) => send.send(PinnedAction::Changed(list)),
                                None => return,
                            }
                        }
                    })
                    .cancellable(WATCH, true),
                ])
            }
            PinnedAction::Loaded(Ok(list)) | PinnedAction::Changed(list) => {
                if list.revision >= state.list.revision {
                    state.list = list;
                    state.error = None;
                }
                Effect::none()
            }
            PinnedAction::Loaded(Err(error)) => {
                state.error = Some(error.to_string());
                Effect::none()
            }
            PinnedAction::Toggle { thing, pin } => {
                let reply = self.marks.set(&thing, Some(pin), None);
                Effect::future(async move { PinnedAction::Loaded(reply.await) })
            }
            PinnedAction::Vip { thing, on } => {
                let reply = self.marks.set(&thing, None, Some(on));
                Effect::future(async move { PinnedAction::Loaded(reply.await) })
            }
            PinnedAction::Move { from, to } => {
                let len = state.list.items.len();
                if from == to || from >= len || to >= len {
                    return Effect::none();
                }
                let revision = state.list.revision;
                let item = state.list.items.remove(from);
                state.list.items.insert(to, item);
                let order: Vec<Thing> = state.list.items.iter().map(|item| item.thing.clone()).collect();
                let marks = self.marks.clone();
                Effect::future(async move {
                    match marks.reorder(&order, revision).await {
                        Ok(list) => PinnedAction::Loaded(Ok(list)),
                        // Refused, because the pins changed or otherwise:
                        // the host's list is the one to show.
                        Err(_) => PinnedAction::Loaded(marks.pinned().await),
                    }
                })
            }
            PinnedAction::Open(index) => match state.list.items.get(index) {
                Some(pin) => Effect::send(PinnedAction::Delegate(PinnedDelegate::Open(pin.thing.clone()))),
                None => Effect::none(),
            },
            PinnedAction::Delegate(_) => Effect::none(),
        }
    }
}

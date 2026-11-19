//! Pins and VIP marks, as the host keeps them (NV-20, NV-21, NV-24 to
//! NV-26).
//!
//! The host owns the marks and their order, so the app and `humata pinned`
//! show the same items in the same order, and two apps open at once follow
//! each other through [`MarksClient::watch`]. [`FakeMarks`] is the only
//! implementation until the live client serves it.

use tokio::sync::mpsc;

use crate::daemon::Reply;
use crate::model::{ChannelId, MachineId, SessionId};



/// Something that can be pinned or marked VIP (NV-20).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Thing {
    /// A session.
    Session(SessionId),
    /// A channel.
    Channel(ChannelId),
    /// A machine.
    Machine(MachineId),
    /// A PR review, by its session.
    PrReview(SessionId),
    /// A board item, by its id.
    BoardItem(String),
}

/// A pinned thing as the Pinned section shows it (NV-21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinned {
    /// What is pinned.
    pub thing: Thing,
    /// Its name.
    pub title: String,
    /// The event it shows, such as "needs input"; empty when there is none.
    pub event: String,
    /// Whether it is VIP too.
    pub vip: bool,
}

/// The pins in the person's order (NV-25).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinnedList {
    /// The pinned things, first to last.
    pub items: Vec<Pinned>,
    /// Goes up with every change to a pin or a VIP mark, from any client.
    pub revision: u64,
}

/// The stream [`MarksClient::watch`] returns.
pub type PinnedLists = mpsc::Receiver<PinnedList>;

/// What the app asks the host about pins and VIP marks.
pub trait MarksClient: Send + Sync + 'static {
    /// The pins.
    fn pinned(&self) -> Reply<PinnedList>;

    /// Pins or unpins `thing`, marks it VIP or not; `None` leaves that mark
    /// as it is. A new pin goes at the end (NV-25). Answers with the pins
    /// after the change.
    fn set(&self, thing: &Thing, pinned: Option<bool>, vip: Option<bool>) -> Reply<PinnedList>;

    /// Puts the pins in `order`, which names each pinned thing once.
    /// `revision` is the list's revision the order was made from: when the
    /// pins have changed since, on any client, the host refuses and nothing
    /// changes, so ask for the pins again.
    fn reorder(&self, order: &[Thing], revision: u64) -> Reply<PinnedList>;

    /// Follows the pins from now on: the new list after each change, from
    /// any client, this one included.
    fn watch(&self) -> Reply<PinnedLists>;
}

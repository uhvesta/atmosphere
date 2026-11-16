//! Finding things: search over every session's conversation (LD-23,
//! LD-24, the palette of KP-1 and KP-2), and the Pinned section's pins and
//! VIP marks (NV-20 to NV-26). State, actions and effects; the views are
//! in the UI crate.

mod pinned;
mod search;

pub use pinned::{PinnedAction, PinnedDelegate, PinnedSection, PinnedState};
pub use search::{Find, FindAction, FindDelegate, FindState};

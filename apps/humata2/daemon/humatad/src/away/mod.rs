//! The machine keeps working with the host closed (AW-*); this module holds
//! the host link state, which says whether the Mac is reachable from here,
//! and the approval passphrase a machine checks while it isn't.

mod link;
mod passphrase;
mod uplink;

pub(crate) use link::HostLink;
pub(crate) use passphrase::set_approval_passphrase;
pub(crate) use uplink::{UplinkStream, Uplinks};

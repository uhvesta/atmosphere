//! The machine keeps working with the host closed (AW-*); this module holds
//! the host link state, which says whether the Mac is reachable from here,
//! the approval passphrase a machine checks while it isn't, and the copy of
//! the host's policy it judges calls by meanwhile.

mod link;
mod lockout;
mod passphrase;
mod policy_copy;
mod uplink;

pub(crate) use link::HostLink;
pub(crate) use lockout::{CheckError, Checked, check_passphrase};
pub(crate) use passphrase::{set_approval_passphrase, verifier};
pub(crate) use policy_copy::{PolicySource, kept_stamp as kept_policy_stamp, restore as restore_policy_copy};
pub(crate) use uplink::{UplinkStream, Uplinks};

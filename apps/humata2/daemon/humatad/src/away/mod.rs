//! The machine keeps working with the host closed (AW-*); this module holds
//! the host link state, which says whether the Mac is reachable from here,
//! the approval passphrase a machine checks while it isn't, and the copy of
//! the host's policy it judges calls by meanwhile.

mod journal;
mod link;
mod lockout;
mod passphrase;
mod policy_copy;
mod registry;
pub(crate) mod stale;
mod up;
mod uplink;

pub(crate) use journal::{
    ANSWER as JOURNAL_ANSWER, REQUEST as JOURNAL_REQUEST, WITHDRAWN as JOURNAL_WITHDRAWN, record as record_journal,
    record_board as record_board_journal, record_now as record_journal_now, record_view as record_view_journal,
    VIEW_OFFERED as JOURNAL_VIEW_OFFERED,
};
pub(crate) use link::HostLink;
pub(crate) use lockout::{CheckError, Checked, check_passphrase};
pub(crate) use passphrase::{set_approval_passphrase, verifier};
pub(crate) use policy_copy::{
    PolicySource, kept_stamp as kept_policy_stamp, owned_item, restore as restore_policy_copy,
};
pub(crate) use registry::{Forwarder, Method, Placement, Row, by_name, by_operation, placed_on};
pub(crate) use up::{agent_session, ask_host, names_another, on_link, unnamed_here};
pub(crate) use uplink::{UplinkStream, Uplinks};

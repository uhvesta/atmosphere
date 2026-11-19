//! The machine keeps working with the host closed (AW-*); this crate holds
//! the host link state, which says whether the Mac is reachable from here,
//! the approval passphrase a machine checks while it isn't, and the copy of
//! the host's policy it judges calls by meanwhile.

pub mod journal;
pub mod link;

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_events::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
    pub use humata2_grpc_review::humata::v1::*;
}
pub mod lockout;
pub mod passphrase;
pub mod policy_copy;
pub mod registry;
pub mod stale;
pub mod up;
pub mod uplink;

pub use journal::{
    ANSWER as JOURNAL_ANSWER, JournalError, REQUEST as JOURNAL_REQUEST,
    VIEW_OFFERED as JOURNAL_VIEW_OFFERED, WITHDRAWN as JOURNAL_WITHDRAWN, acked as ack_journal,
    entries_after as journal_entries_after, record as record_journal,
    record_board as record_board_journal, record_now as record_journal_now,
    record_view as record_view_journal,
};
pub use link::{HostLink, Opened};
pub use lockout::{CheckError, Checked, check_passphrase};
pub use passphrase::{set_approval_passphrase, verifier};
pub use policy_copy::{
    CopyError, Delivered, Linked, PolicyGuard, PolicySource, kept_stamp as kept_policy_stamp,
    owned_item, restore as restore_policy_copy,
};
pub use registry::{Forwarder, Method, Placement, Row, by_name, by_operation, placed_on};
pub use up::{agent_session, ask_host, names_another, on_link, unnamed_here};
pub use uplink::{EventSink, UplinkInbox, UplinkStream, Uplinks};

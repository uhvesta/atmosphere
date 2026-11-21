//! `DeskService` (`humata2_svc_desk`): Desk sessions, channel desks, document
//! watching/versions, and moving Desk sessions into channels.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_desk::humata::v1::*;
    pub use humata2_grpc_events::humata::v1::*;
    pub use humata2_grpc_files::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
    pub use humata2_grpc_review::humata::v1::*;
    pub use humata2_grpc_sessions::humata::v1::*;
}

pub mod ports;
pub mod desk_service;

pub use desk_service::{DeskDeps, DeskSvc, Locks, server};

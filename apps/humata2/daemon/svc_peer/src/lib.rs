//! Peer, relay, host_relay, remote_end, and remote_work service crate (`humata2_svc_peer`).

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_board::humata::v1::*;
    pub use humata2_grpc_catalog::humata::v1::*;
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_desk::humata::v1::*;
    pub use humata2_grpc_events::humata::v1::*;
    pub use humata2_grpc_files::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_machines::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
    pub use humata2_grpc_review::humata::v1::*;
    pub use humata2_grpc_sessions::humata::v1::*;
    pub use humata2_grpc_terminal::humata::v1::*;
}

pub mod host_relay;
pub mod peer;
pub mod peer_client;
pub mod peer_link;
pub mod peer_reports;
pub mod peer_skills;
pub mod ports;
pub mod relay;
pub mod relay_auth;
pub mod remote_end;
pub mod remote_work;

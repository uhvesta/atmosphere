//! `humata2_svc_stop_restart`: stop/start sessions, restart recovery,
//! restart doctor, and `DaemonService` for `humatad2`.
//!
//! Extracted from `humatad` as a standalone crate (mandatory split of
//! `svc_lifecycle` to stay <=12k LOC).

pub mod daemon;
pub mod doctor;
pub mod ports;
pub mod restart;
pub mod stop;

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
}

//! Shared transport, connection facts, operation progress polling, time
//! formatting, call recording, and pure proto-to-model mapping for the live
//! daemon clients.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod humata2_grpc {
    pub use humata2_grpc_core::{Duration, Timestamp};
    pub mod humata {
        pub mod v1 {
            pub use humata2_grpc_core::humata::v1::*;
            pub use humata2_grpc_machines::humata::v1::*;
            pub use humata2_grpc_peer::humata::v1::*;
            pub use humata2_grpc_review::humata::v1::*;
            pub use humata2_grpc_sessions::humata::v1::*;
            pub use humata2_grpc_terminal::humata::v1::*;
        }
    }
}

pub mod connection;
pub mod local_time;
pub mod map;
pub mod recorded;

pub use connection::{
    ConnectError, Credentials, DaemonFacts, LiveConnection, LiveOptions, MESSAGE_MAX_BYTES, Wire,
    bad_repo, daemon_error, fresh_key, key_of, not_in_this_release, said, status_error,
};
pub use recorded::{Call, Observer, Recorded};

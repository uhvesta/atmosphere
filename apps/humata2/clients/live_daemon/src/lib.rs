//! Live [`humata2_clients::Daemon`], [`humata2_clients::Terminals`],
//! [`humata2_clients::history::HistoryClient`], and
//! [`humata2_clients::marks::MarksClient`] over [`LiveConnection`].

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod humata2_grpc {
    pub use humata2_grpc_core::{Duration, Timestamp};
    pub mod humata {
        pub mod v1 {
            pub use humata2_grpc_board::humata::v1::*;
            pub use humata2_grpc_catalog::humata::v1::*;
            pub use humata2_grpc_core::humata::v1::*;
            pub use humata2_grpc_events::humata::v1::*;
            pub use humata2_grpc_inbox::humata::v1::*;
            pub use humata2_grpc_machines::humata::v1::*;
            pub use humata2_grpc_peer::humata::v1::*;
            pub use humata2_grpc_sessions::humata::v1::*;
            pub use humata2_grpc_terminal::humata::v1::*;
        }
    }
}

mod agent_switches;
mod boards;
mod channel_repos;
mod cli_auth;
mod daemon;
mod events;
pub mod history;
mod inbox;
mod link;
pub mod marks;
mod mass_delete;
mod message;
mod read_marks;
mod repos;
mod running;
mod session_repos;
mod settings;
mod setup;
mod state;
mod terminals;
pub mod todo;

pub(crate) use humata2_clients_live_core::{local_time, map};
pub(crate) use state as connection;

pub use state::{ConnectError, DaemonFacts, LiveConnection, LiveDaemon, LiveOptions};

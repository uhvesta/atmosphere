//! [`LiveMachines`] implements the app's `Machines` trait over the daemon's
//! machine, provider, operation, and skills gRPC services.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod humata2_grpc {
    pub use humata2_grpc_core::{Duration, Timestamp};
    pub mod humata {
        pub mod v1 {
            pub use humata2_grpc_core::humata::v1::*;
            pub use humata2_grpc_machines::humata::v1::*;
            pub use humata2_grpc_peer::humata::v1::*;
            pub use humata2_grpc_sessions::humata::v1::*;
        }
    }
}

mod machines;

pub use machines::LiveMachines;

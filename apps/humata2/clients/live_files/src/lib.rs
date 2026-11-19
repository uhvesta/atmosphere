//! [`LiveFileManager`] implements the app's `FileManager` trait over the
//! daemon's file, transfer, trash, and machine gRPC services.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod humata2_grpc {
    pub use humata2_grpc_core::{Duration, Timestamp};
    pub mod humata {
        pub mod v1 {
            pub use humata2_grpc_core::humata::v1::*;
            pub use humata2_grpc_files::humata::v1::*;
            pub use humata2_grpc_machines::humata::v1::*;
        }
    }
}

mod file_manager;

pub use file_manager::LiveFileManager;

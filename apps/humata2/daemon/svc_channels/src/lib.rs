//! Channel service domain logic (`humata2_svc_channels`).

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_sessions::humata::v1::*;
}

pub mod gh_repos;
pub mod move_folder;
pub mod ports;
pub mod rename_alias;

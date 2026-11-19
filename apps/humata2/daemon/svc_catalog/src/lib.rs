//! Catalog leaf services for `humatad2` (SPLIT §3.2).

pub(crate) mod pb {
    #![allow(unused_imports)]
    pub use humata2_grpc_catalog::humata::v1::*;
    pub use humata2_grpc_core::humata::v1::*;
}

pub mod agent_gate;
pub mod agent_service;

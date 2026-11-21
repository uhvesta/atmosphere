//! `humatad2 mcp` gRPC-to-MCP bridge service crate.

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

pub mod mcp;
pub mod ports;

pub use mcp::{McpArgs, run_on_socket};

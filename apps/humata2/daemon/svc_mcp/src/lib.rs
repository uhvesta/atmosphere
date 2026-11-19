//! `humatad2 mcp` gRPC-to-MCP bridge service crate.

pub mod mcp;
pub mod ports;

pub use mcp::{McpArgs, run_on_socket};

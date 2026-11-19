//! Adapter wiring `humata2_svc_mcp` into `humatad2` (`Command::Mcp`).
#![cfg(feature = "svc_mcp")]

use humatad_core::error::DaemonError;
use humatad_core::serve;

pub(crate) use humata2_svc_mcp::McpArgs;

pub(crate) async fn run(args: McpArgs) -> Result<(), DaemonError> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => serve::default_socket()?,
    };
    humata2_svc_mcp::run_on_socket(socket)
        .await
        .map_err(|source| DaemonError::Mcp { source })
}

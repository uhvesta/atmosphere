//! The daemon tests' harness and multi-machine rig as a crate, so another
//! package's tests (the desktop's live flows) can start a host with boxes.
//! The rig names the harness as `crate::common`, as it does in a test.

#[path = "../common/mod.rs"]
pub mod common;
#[path = "../common/remote_rig.rs"]
pub mod remote_rig;
/// The fake GitHub and the pull requests flow 26.17 reviews (acme/api#482,
/// acme/ledger#91), with an agent that speaks MCP.
#[path = "../common/mcp_client.rs"]
pub mod mcp_client;
#[path = "../common/pr_github.rs"]
pub mod pr_github;

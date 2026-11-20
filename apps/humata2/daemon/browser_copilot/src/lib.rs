//! Humata 2 Native Browser Co-Pilot (`0.7.0 BROWSER`).
//!
//! Connects coding agents (`claude`, `codex`, `copilot`, `agy`) to the user's
//! existing native desktop browsers via WebExtension Native Messaging, with an
//! Agent Inbox permission gate that grants 30-minute sessions for read actions
//! and enforces strict per-action approval for every write/guide action.

pub mod detect;
pub mod manifest;
pub mod mcp_tools;
pub mod policy;
pub mod protocol;

pub use detect::{BrowserDetector, BrowserFamily, BrowserKind, DetectedBrowser, Platform};
pub use manifest::{
    DEFAULT_FIREFOX_EXTENSION_ID, ManifestConfig, ManifestError, ManifestInstaller,
    NATIVE_HOST_DESCRIPTION, NATIVE_HOST_NAME,
};
pub use mcp_tools::{
    BrowserTransport, DispatchError, DispatchOutcome, FakeBrowserTransport, McpDispatcher,
    McpToolSchema, TransportError, tool_schemas,
};
pub use policy::{
    ActionClass, ApprovalOutcome, BrowserInboxAsk, Decision, GateCheck, PermissionGate,
    PolicyError, READ_GRANT_TTL_MS, SupportedAgent,
};
pub use protocol::{
    BrowserAction, BrowserReply, BrowserReplyPayload, BrowserRequest, GuideHighlight,
    InteractiveElement, MAX_NATIVE_MESSAGE_BYTES, PageSnapshot, ProtocolError, TabInfo, read_frame,
    write_frame,
};

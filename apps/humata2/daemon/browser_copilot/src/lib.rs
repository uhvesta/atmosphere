//! Humata 2 Native Browser Co-Pilot (`0.7.0 BROWSER`).
//!
//! Connects coding agents (`claude`, `codex`, `copilot`, `agy`) to the user's
//! existing native desktop browsers via WebExtension Native Messaging for
//! strictly read-only inspection (`list_tabs`, `read_tab`, `screenshot`),
//! gated by an Agent Inbox permission policy with 30-minute read session
//! grants. Never clicks, types, navigates, mutates the DOM, or executes code.

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
    BrowserAction, BrowserReply, BrowserReplyPayload, BrowserRequest, InteractiveElement,
    MAX_NATIVE_MESSAGE_BYTES, PageScreenshot, PageSnapshot, ProtocolError, TabInfo, read_frame,
    write_frame,
};

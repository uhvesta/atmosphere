//! Transport-neutral protocol between terminal viewers and their persistent host.
//! No process, filesystem, socket, SSH, runtime, or GUI dependencies.
mod binary;
mod snapshot;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 3;
pub const MAX_INPUT_BYTES: usize = 256 << 10;
pub const MAX_OUTPUT_BYTES: usize = 4 << 20;
/// Compressed checkpoint bound; its base64 carrier fits the 32 MiB frame cap.
pub const MAX_SNAPSHOT_BYTES: usize = 4 << 20;
/// Maximum decoded checkpoint, checked before and during decompression.
pub const MAX_RAW_CHECKPOINT_BYTES: usize = 16 << 20;
pub const SNAPSHOT_ENGINE: &str = "libghostty-vt/1.0.1;gvtc/1";

/// Explicit recovery hints supplied by a launcher/provider integration.
/// These are identifiers and paths, never inferred from terminal text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryHint {
    pub provider: String,
    pub session_id: String,
    pub working_directory: String,
    #[serde(default)]
    pub state_paths: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryPolicy {
    Off,
    Recent {
        max_lines: u32,
        max_bytes: u64,
        interval_seconds: u64,
    },
}
impl Default for HistoryPolicy {
    fn default() -> Self {
        Self::Recent {
            max_lines: 10_000,
            max_bytes: 4 * 1024 * 1024,
            interval_seconds: 30,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionState {
    Running,
    Exited { code: Option<i32> },
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameSource {
    Manual,
    Programmatic,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionNameOverride {
    pub title: String,
    pub source: NameSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    Raw,
    Paste,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InjectionOptions {
    pub mode: InjectionMode,
    pub submit: bool,
}
impl InjectionOptions {
    pub const fn raw() -> Self {
        Self {
            mode: InjectionMode::Raw,
            submit: false,
        }
    }
    pub const fn paste() -> Self {
        Self {
            mode: InjectionMode::Paste,
            submit: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputTicket {
    pub terminal_uuid: String,
    pub connection_epoch: u64,
    pub seq: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInfo {
    pub id: u64,
    pub uuid: String,
    /// Effective display title: explicit override, or the automatic title.
    pub title: String,
    pub automatic_title: String,
    pub name_override: Option<SessionNameOverride>,
    pub attributes: BTreeMap<String, String>,
    pub cwd: String,
    /// Sanitized program label, not argv or environment.
    pub program: String,
    pub state: SessionState,
    pub recovery_available: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInfo {
    pub label: String,
    pub os: String,
    pub arch: String,
    pub protocol_version: u32,
    pub engine: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientFrame {
    Authenticate {
        protocol: u32,
        token: String,
    },
    List,
    Open {
        id: u64,
    },
    /// Unsubscribe this viewer and release its lease; the process and host VT
    /// remain alive. A later Open obtains a fresh authoritative checkpoint.
    Detach {
        id: u64,
    },
    /// Explicitly take the input/resize lease for this shared session.
    TakeControl {
        id: u64,
    },
    /// Raw terminal input; a written acknowledgement is not agent execution.
    Input {
        id: u64,
        seq: u64,
        #[serde(with = "crate::binary::input")]
        bytes: Vec<u8>,
    },
    Resize {
        id: u64,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Ack {
        id: u64,
        offset: u64,
    },
    SnapshotRequest {
        id: u64,
    },
    Stop {
        id: u64,
    },
    /// Only configured trusted profiles can start a program.
    Create {
        profile: String,
    },
    SetRecovery {
        id: u64,
        hint: Option<RecoveryHint>,
    },
    SetHistory {
        id: u64,
        policy: HistoryPolicy,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerFrame {
    /// Ordered barrier: prior view frames have drained; PTY and host VT remain alive.
    Detached {
        id: u64,
    },
    Sessions {
        host: HostInfo,
        sessions: Vec<SessionInfo>,
    },
    /// Opaque versioned Ghostty snapshot, not ANSI text and not a framebuffer.
    Snapshot {
        id: u64,
        engine: String,
        // Rust consumers hold native checkpoint bytes; the wire uses a bounded
        // compressed envelope so dense scrollback does not become JSON bloat.
        #[serde(with = "crate::snapshot")]
        bytes: Vec<u8>,
        offset: u64,
        cols: u16,
        rows: u16,
        live: bool,
        exited: Option<i32>,
    },
    /// Per-viewer input/resize lease; process availability remains Snapshot.live.
    Control {
        id: u64,
        controller: bool,
    },
    Output {
        id: u64,
        offset: u64,
        #[serde(with = "crate::binary::output")]
        bytes: Vec<u8>,
    },
    Resized {
        id: u64,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    InputWritten {
        id: u64,
        seq: u64,
    },
    Exited {
        id: u64,
        code: Option<i32>,
    },
    /// A slow viewer must request a fresh snapshot; never silently skip bytes.
    ResyncRequired {
        id: u64,
        reason: String,
    },
    Error {
        id: Option<u64>,
        code: String,
        message: String,
    },
}

//! Shared JSON wire protocol between the host daemon's To-do web server
//! (`humata2_web_todo_server`) and the GPUI Wasm To-do browser client
//! (`humata2_web_todo_web`) — WEBTODO (D905).
//!
//! Pure data types with no IO or clock dependencies so the crate compiles
//! identically on native hosts and `wasm32-unknown-unknown`.

use humata2_todo::{Day, Todo};
use serde::{Deserialize, Serialize};

/// Protocol version spoken over `/ws`.
pub const PROTOCOL_VERSION: u32 = 1;

/// A channel's display identity for filtering and row pills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
}

/// A session's display identity for session chips on To-do rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    /// RGB hex color for the status dot (e.g. `0x4ade80`).
    pub dot: u32,
}

/// A full point-in-time snapshot of Humata's To-do board pushed by the host
/// daemon on connect and on every board/channel/session change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardSnapshot {
    /// Host civil day (`YYYY-MM-DD`) used by `humata2_todo::rank_with` and due chips.
    pub today: Day,
    /// Every non-deleted to-do item across all channels.
    pub items: Vec<Todo>,
    /// Channels for filter pills and display names.
    pub channels: Vec<ChannelInfo>,
    /// Sessions for session chips on rows.
    pub sessions: Vec<SessionInfo>,
    /// Monotonic snapshot revision counter from the server.
    #[serde(default)]
    pub revision: u64,
}

/// Messages pushed from the host daemon to the browser over `/ws`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Complete board snapshot pushed on connect and on every live change.
    Snapshot(BoardSnapshot),
    /// Server-side error notice (non-fatal unless the socket closes).
    Error { message: String },
}

/// Messages sent from the browser to the host daemon over `/ws`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Client handshake identifying protocol version.
    Hello { protocol: u32 },
    /// Optional keepalive ping.
    Ping,
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_todo::{Effort, Impact, Need, State};

    #[test]
    fn snapshot_round_trips_as_json() {
        let today = Day::from_ymd(2026, 11, 20).expect("valid day");
        let mut item = Todo::new("PAY-1", "Fix checkout retry", today);
        item.state = State::InProgress;
        item.channel = Some("ch-pay".to_owned());
        item.effort = Some(Effort::M);
        item.impact = Some(Impact::High);
        item.need = Some(Need::Critical);
        item.due = Some(today);

        let msg = ServerMessage::Snapshot(BoardSnapshot {
            today,
            items: vec![item],
            channels: vec![ChannelInfo {
                id: "ch-pay".to_owned(),
                name: "payments".to_owned(),
            }],
            sessions: vec![SessionInfo {
                id: "s-1".to_owned(),
                name: "retry-fix".to_owned(),
                dot: 0x4ade80,
            }],
            revision: 7,
        });

        let json = serde_json::to_string(&msg).expect("serialize");
        let back: ServerMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, msg);
    }

    #[test]
    fn client_messages_round_trip() {
        let hello = ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
        };
        let encoded = serde_json::to_string(&hello).expect("serialize");
        let decoded: ClientMessage = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, hello);
    }
}

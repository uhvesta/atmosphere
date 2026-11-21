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

/// Category of an item in the Reviews inbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    Local,
    GithubFeedback,
    ReviewRequested,
    Watched,
}

/// Summary row for the `/reviews` inbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewInboxItem {
    pub id: String,
    pub kind: ReviewKind,
    pub title: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub layers_summary: Option<String>,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub pr_refs: Vec<String>,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub chip_label: String,
    #[serde(default)]
    pub chip_tone: String,
    #[serde(default)]
    pub age_label: String,
    #[serde(default)]
    pub added: u32,
    #[serde(default)]
    pub removed: u32,
    #[serde(default)]
    pub layer_count: u32,
}

/// Manually watched GitHub PR entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedPrEntry {
    pub url: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
}

/// Review progress state for a single stack layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerStatus {
    LookedAt,
    FixesPushed,
    NotLookedAt,
}

/// A single diff line in a reviewed file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDiffLine {
    /// `"ctx"`, `"add"`, or `"del"`.
    pub kind: String,
    #[serde(default)]
    pub old_line: Option<u32>,
    #[serde(default)]
    pub new_line: Option<u32>,
    pub text: String,
}

/// Inline review comment attached to a file or line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewInlineComment {
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub age: String,
}

/// A file changed in a review layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDiffFile {
    pub path: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub added: u32,
    #[serde(default)]
    pub removed: u32,
    #[serde(default)]
    pub looked: bool,
    #[serde(default)]
    pub lines: Vec<ReviewDiffLine>,
    #[serde(default)]
    pub comments: Vec<ReviewInlineComment>,
}

/// One layer in a stacked review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewLayer {
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub base_branch: String,
    #[serde(default)]
    pub repos: Vec<String>,
    pub status: LayerStatus,
    #[serde(default)]
    pub rebased_diff_unchanged: bool,
    #[serde(default)]
    pub files: Vec<ReviewDiffFile>,
}

/// Full detail payload for `/reviews/<id>?layer=N`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDetail {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub is_local: bool,
    #[serde(default)]
    pub layers: Vec<ReviewLayer>,
    #[serde(default)]
    pub summary_what: String,
    #[serde(default)]
    pub summary_why: String,
    #[serde(default)]
    pub summary_details: String,
    #[serde(default)]
    pub pr_count: u32,
    #[serde(default)]
    pub open_notes: u32,
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
    /// Reviews inbox rows (local stacks, GitHub feedback, review requests, watched PRs).
    #[serde(default)]
    pub reviews: Vec<ReviewInboxItem>,
    /// Manually watched GitHub PRs.
    #[serde(default)]
    pub watched_prs: Vec<WatchedPrEntry>,
    /// Seconds since GitHub PRs were last checked, if known.
    #[serde(default)]
    pub github_checked_ago_secs: Option<u64>,
    /// Full layer + diff details for reviews.
    #[serde(default)]
    pub review_details: Vec<ReviewDetail>,
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
    /// Watch a remote GitHub PR URL in the Reviews inbox.
    WatchPrUrl { url: String },
    /// Request an immediate refresh of GitHub reviews.
    RefreshReviews,
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_todo::{Effort, Impact, Need, State};

    #[test]
    fn legacy_snapshot_deserializes_with_default_review_fields() {
        let today = Day::from_ymd(2026, 11, 20).expect("valid day");
        let legacy_json = serde_json::json!({
            "kind": "snapshot",
            "today": today,
            "items": [],
            "channels": [],
            "sessions": [],
            "revision": 3
        })
        .to_string();
        let decoded: ServerMessage = serde_json::from_str(&legacy_json).expect("decode legacy");
        let ServerMessage::Snapshot(snap) = decoded else {
            panic!("expected snapshot");
        };
        assert_eq!(snap.revision, 3);
        assert!(snap.reviews.is_empty());
        assert!(snap.watched_prs.is_empty());
        assert_eq!(snap.github_checked_ago_secs, None);
        assert!(snap.review_details.is_empty());
    }

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
            reviews: vec![ReviewInboxItem {
                id: "PAY-3".to_owned(),
                kind: ReviewKind::Local,
                title: "Idempotent webhook delivery".to_owned(),
                channel: Some("payments".to_owned()),
                agent: Some("claude".to_owned()),
                layers_summary: Some("3 layers".to_owned()),
                repos: vec!["payments-api".to_owned()],
                pr_refs: Vec::new(),
                subtitle: "payments · 3 layers".to_owned(),
                chip_label: "Fixes pushed".to_owned(),
                chip_tone: "amber".to_owned(),
                age_label: "4m ago".to_owned(),
                added: 142,
                removed: 18,
                layer_count: 3,
            }],
            watched_prs: vec![WatchedPrEntry {
                url: "https://github.com/acme/payments-api/pull/412".to_owned(),
                repo: "acme/payments-api".to_owned(),
                number: 412,
                title: "Retry queue metrics".to_owned(),
            }],
            github_checked_ago_secs: Some(120),
            review_details: vec![ReviewDetail {
                id: "PAY-3".to_owned(),
                title: "Idempotent webhook delivery".to_owned(),
                is_local: true,
                layers: vec![ReviewLayer {
                    number: 1,
                    title: "schema".to_owned(),
                    branch: "pay-3/schema".to_owned(),
                    base_branch: "main".to_owned(),
                    repos: vec!["payments-api".to_owned()],
                    status: LayerStatus::LookedAt,
                    rebased_diff_unchanged: true,
                    files: vec![ReviewDiffFile {
                        path: "migrations/0042_webhooks.sql".to_owned(),
                        repo: "payments-api".to_owned(),
                        added: 24,
                        removed: 0,
                        looked: true,
                        lines: vec![ReviewDiffLine {
                            kind: "add".to_owned(),
                            old_line: None,
                            new_line: Some(1),
                            text: "+CREATE TABLE webhook_events (id TEXT PRIMARY KEY);".to_owned(),
                        }],
                        comments: vec![ReviewInlineComment {
                            author: "maya".to_owned(),
                            body: "Looks clean.".to_owned(),
                            line: Some(1),
                            age: "2m ago".to_owned(),
                        }],
                    }],
                }],
                summary_what: "Adds idempotency keys to webhook ingestion.".to_owned(),
                summary_why: "Prevents duplicate charges on retry.".to_owned(),
                summary_details: "3 stacked layers: schema, dual write, reads.".to_owned(),
                pr_count: 3,
                open_notes: 1,
            }],
        });

        let json = serde_json::to_string(&msg).expect("serialize");
        let back: ServerMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, msg);
        if let ServerMessage::Snapshot(snap) = back {
            assert!(snap.review_details[0].layers[0].rebased_diff_unchanged);
        }
    }

    #[test]
    fn client_messages_round_trip() {
        for msg in [
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
            },
            ClientMessage::Ping,
            ClientMessage::WatchPrUrl {
                url: "https://github.com/acme/repo/pull/99".to_owned(),
            },
            ClientMessage::RefreshReviews,
        ] {
            let encoded = serde_json::to_string(&msg).expect("serialize");
            let decoded: ClientMessage = serde_json::from_str(&encoded).expect("deserialize");
            assert_eq!(decoded, msg);
        }
    }
}

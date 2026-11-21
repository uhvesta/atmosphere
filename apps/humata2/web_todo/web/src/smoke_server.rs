//! Smoke server binary for headless browser verification of WEBTODO.
//!
//! Starts the real `humata2_web_todo_server` on `127.0.0.1:0` serving the
//! compiled GPUI Wasm bundle, prints its URL as JSON on stdout, and pushes a
//! live board update when a line is written to stdin so the headless browser
//! test can verify initial load + live push rendering.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use humata2_todo::{Day, Effort, Impact, Need, State, Todo};
use humata2_web_todo_server::{Assets, BoardSource, serve_loopback};
use humata2_web_todo_wire::{
    BoardSnapshot, ChannelInfo, LayerStatus, ReviewDetail, ReviewDiffFile, ReviewDiffLine,
    ReviewInboxItem, ReviewInlineComment, ReviewKind, ReviewLayer, SessionInfo, WatchedPrEntry,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

struct SmokeSource {
    snapshot: Mutex<BoardSnapshot>,
    updates: broadcast::Sender<()>,
}

impl SmokeSource {
    fn new() -> Self {
        let (updates, _) = broadcast::channel(16);
        let today = Day::from_ymd(2026, 11, 20).expect("day");

        let mut first = Todo::new("PAY-1", "Fix checkout retry backoff", today);
        first.state = State::InProgress;
        first.channel = Some("ch-pay".to_owned());
        first.effort = Some(Effort::M);
        first.impact = Some(Impact::High);
        first.need = Some(Need::Critical);
        first.due = Some(today);
        first.sessions = vec!["s-1".to_owned()];

        let mut second = Todo::new("OPS-2", "Verify host daemon restart recovery", today);
        second.state = State::Open;
        second.channel = Some("ch-ops".to_owned());
        second.effort = Some(Effort::S);

        Self {
            snapshot: Mutex::new(BoardSnapshot {
                today,
                items: vec![first, second],
                channels: vec![
                    ChannelInfo {
                        id: "ch-pay".to_owned(),
                        name: "payments".to_owned(),
                    },
                    ChannelInfo {
                        id: "ch-ops".to_owned(),
                        name: "ops".to_owned(),
                    },
                ],
                sessions: vec![SessionInfo {
                    id: "s-1".to_owned(),
                    name: "retry-fix".to_owned(),
                    dot: 0x4ade80,
                }],
                revision: 1,
                reviews: vec![
                    ReviewInboxItem {
                        id: "PAY-3".to_owned(),
                        kind: ReviewKind::Local,
                        title: "Idempotent webhook delivery".to_owned(),
                        channel: Some("payments".to_owned()),
                        agent: Some("claude".to_owned()),
                        layers_summary: Some("3 layers".to_owned()),
                        repos: vec!["payments-api".to_owned()],
                        pr_refs: Vec::new(),
                        subtitle: "payments · 3 layers · payments-api".to_owned(),
                        chip_label: "Fixes pushed".to_owned(),
                        chip_tone: "amber".to_owned(),
                        age_label: "4m ago".to_owned(),
                        added: 142,
                        removed: 18,
                        layer_count: 3,
                    },
                    ReviewInboxItem {
                        id: "PR-412".to_owned(),
                        kind: ReviewKind::GithubFeedback,
                        title: "Retry queue metrics".to_owned(),
                        channel: Some("payments".to_owned()),
                        agent: None,
                        layers_summary: None,
                        repos: vec!["acme/payments-api".to_owned()],
                        pr_refs: vec!["#412".to_owned()],
                        subtitle: "acme/payments-api#412 · 2 comments".to_owned(),
                        chip_label: "New feedback".to_owned(),
                        chip_tone: "accent".to_owned(),
                        age_label: "12m ago".to_owned(),
                        added: 54,
                        removed: 11,
                        layer_count: 1,
                    },
                ],
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
                    layers: vec![
                        ReviewLayer {
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
                                lines: vec![
                                    ReviewDiffLine {
                                        kind: "add".to_owned(),
                                        old_line: None,
                                        new_line: Some(1),
                                        text: "+CREATE TABLE webhook_events (id TEXT PRIMARY KEY);"
                                            .to_owned(),
                                    },
                                ],
                                comments: vec![ReviewInlineComment {
                                    author: "maya".to_owned(),
                                    body: "Index on created_at looks good.".to_owned(),
                                    line: Some(1),
                                    age: "2m ago".to_owned(),
                                }],
                            }],
                        },
                        ReviewLayer {
                            number: 2,
                            title: "dual write".to_owned(),
                            branch: "pay-3/dual-write".to_owned(),
                            base_branch: "pay-3/schema".to_owned(),
                            repos: vec!["payments-api".to_owned()],
                            status: LayerStatus::FixesPushed,
                            rebased_diff_unchanged: false,
                            files: vec![ReviewDiffFile {
                                path: "src/webhooks/ingest.rs".to_owned(),
                                repo: "payments-api".to_owned(),
                                added: 68,
                                removed: 12,
                                looked: false,
                                lines: vec![
                                    ReviewDiffLine {
                                        kind: "ctx".to_owned(),
                                        old_line: Some(41),
                                        new_line: Some(41),
                                        text: " pub fn ingest(ev: WebhookEvent) -> Result<()> {"
                                            .to_owned(),
                                    },
                                    ReviewDiffLine {
                                        kind: "add".to_owned(),
                                        old_line: None,
                                        new_line: Some(42),
                                        text: "+    store.insert_idempotent(&ev)?;".to_owned(),
                                    },
                                ],
                                comments: Vec::new(),
                            }],
                        },
                        ReviewLayer {
                            number: 3,
                            title: "reads".to_owned(),
                            branch: "pay-3/reads".to_owned(),
                            base_branch: "pay-3/dual-write".to_owned(),
                            repos: vec!["payments-api".to_owned()],
                            status: LayerStatus::NotLookedAt,
                            rebased_diff_unchanged: false,
                            files: vec![ReviewDiffFile {
                                path: "src/webhooks/query.rs".to_owned(),
                                repo: "payments-api".to_owned(),
                                added: 50,
                                removed: 6,
                                looked: false,
                                lines: Vec::new(),
                                comments: Vec::new(),
                            }],
                        },
                    ],
                    summary_what: "Adds idempotency keys to webhook ingestion.".to_owned(),
                    summary_why: "Prevents duplicate charges on retry.".to_owned(),
                    summary_details: "3 stacked layers: schema, dual write, reads.".to_owned(),
                    pr_count: 3,
                    open_notes: 1,
                }],
            }),
            updates,
        }
    }

    fn push_live_item(&self, key: &str, title: &str) {
        let mut snap = self.snapshot.lock().unwrap();
        let mut item = Todo::new(key, title, snap.today);
        item.state = State::Open;
        item.channel = Some("ch-pay".to_owned());
        snap.items.push(item);
        snap.revision += 1;
        let _ = self.updates.send(());
    }
}

impl BoardSource for SmokeSource {
    fn snapshot(&self) -> BoardSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    fn subscribe(&self) -> broadcast::Receiver<()> {
        self.updates.subscribe()
    }

    fn watch_pr_url(&self, url: &str) -> Result<(), String> {
        let trimmed = url.trim();
        if !trimmed.starts_with("https://github.com/") || !trimmed.contains("/pull/") {
            return Err("Enter a full GitHub PR URL (https://github.com/org/repo/pull/123)".to_owned());
        }
        let mut snap = self.snapshot.lock().unwrap();
        snap.watched_prs.push(WatchedPrEntry {
            url: trimmed.to_owned(),
            repo: "github".to_owned(),
            number: 0,
            title: trimmed.to_owned(),
        });
        snap.revision += 1;
        let _ = self.updates.send(());
        Ok(())
    }

    fn refresh_reviews(&self) {
        let mut snap = self.snapshot.lock().unwrap();
        snap.github_checked_ago_secs = Some(0);
        snap.revision += 1;
        let _ = self.updates.send(());
    }
}

#[tokio::main]
async fn main() {
    let bundle_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: smoke_server <bundle_dir>");
    let assets = Assets::from_dir(&bundle_dir).expect("load wasm bundle");
    let source = Arc::new(SmokeSource::new());
    let server = serve_loopback(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        assets,
        Arc::clone(&source) as Arc<dyn BoardSource>,
    )
    .await
    .expect("start smoke server");

    println!("{}", server.url());

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut counter = 3u32;
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed == "quit" {
            break;
        }
        if trimmed == "push" {
            source.push_live_item(&format!("PAY-{counter}"), "Live pushed item");
            counter += 1;
        }
    }
    server.stop().await;
}

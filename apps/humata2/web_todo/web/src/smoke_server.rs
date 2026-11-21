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
use humata2_web_todo_wire::{BoardSnapshot, ChannelInfo, SessionInfo};
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

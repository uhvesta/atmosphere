use super::{diffs::lines, machines, requests::request, sessions::session};
use crate::model::{
    Agent, Channel, ChannelId, Comment, FileDiff, ForkKind, Item, ItemId, ItemKind, ItemStatus,
    Line, LineKind, Machine, MachineId, MachineKind, PreparedView, Preview, PullRequest, RepoDiff,
    RequestId, RequestKind, Scope, SessionFile, SessionId, Speaker, Status,
};
use crate::workspace::Workspace;

/// The same approved workspace with its open Ledger session in one lifecycle state.
#[must_use]
pub fn design_session(ledger_status: Status) -> Workspace {
    let mut workspace = Workspace::default();
    workspace
        .terminal_peers
        .insert((SessionId::new("ledger"), 0), 1);
    workspace.machines = vec![
        machines().remove(0),
        Machine {
            id: MachineId::new("devbox-1"),
            name: "devbox-1".into(),
            kind: MachineKind::Ssh {
                address: "dev@devbox-1".into(),
            },
            online: true,
            detail: "Ubuntu 24.04".into(),
            last_seen: "now".into(),
            lost_at: (ledger_status == Status::Offline).then(|| "14:02".into()),
        },
        Machine {
            id: MachineId::new("gpu-box"),
            name: "gpu-box".into(),
            kind: MachineKind::Ssh {
                address: "dev@gpu-box".into(),
            },
            online: false,
            detail: "Ubuntu 24.04 · RTX 4090 · 64 GB".into(),
            last_seen: "12 min ago".into(),
            lost_at: None,
        },
    ];
    workspace.machines.push(Machine {
        id: MachineId::new("cloud-vm-3"),
        name: "cloud-vm-3".into(),
        kind: MachineKind::Provider {
            provider: "cloud".into(),
        },
        online: false,
        detail: "Ubuntu 24.04 · provider".into(),
        last_seen: "stopped".into(),
        lost_at: None,
    });
    for (id, system, used, free, latency, stopped) in [
        ("this-mac", "macOS 26.7 · arm64", 42, 210, None, false),
        ("devbox-1", "Ubuntu 24.04 · ssh", 88, 11, Some(18), false),
        ("gpu-box", "Debian 13 · ssh", 30, 140, None, false),
        ("cloud-vm-3", "Ubuntu 24.04 · provider", 10, 90, None, true),
    ] {
        workspace.machine_health.insert(
            MachineId::new(id),
            crate::model::MachineHealth {
                system: system.into(),
                daemon: "2.0.3".into(),
                uptime: "6 days".into(),
                tools: vec!["gh 2.83".into(), "claude 2.4".into(), "codex 0.98".into()],
                disk_used_percent: used,
                disk_free_gb: free,
                latency_ms: latency,
                stopped,
            },
        );
    }
    for (name, repos) in [
        ("payments-rewrite", vec!["acme/ledger", "acme/api"]),
        ("infra", vec!["acme/infra"]),
        ("pr-reviews", vec!["acme/api"]),
        ("web", vec!["acme/web"]),
    ] {
        workspace.channels.push(Channel {
            id: ChannelId::new(name),
            name: name.into(),
            repos: repos.into_iter().map(str::to_owned).collect(),
            machines: vec![MachineId::new("this-mac"), MachineId::new("devbox-1")],
            purpose: String::new(),
        });
    }
    for (id, channel, title, agent, machine, status, parent) in [
        (
            "ledger",
            "payments-rewrite",
            "Ledger schema migration",
            Agent::Claude,
            "devbox-1",
            ledger_status,
            None,
        ),
        (
            "brin",
            "payments-rewrite",
            "Try BRIN index",
            Agent::Claude,
            "devbox-1",
            Status::NeedsYou,
            Some("ledger"),
        ),
        (
            "partial-brin",
            "payments-rewrite",
            "Partial BRIN on posted_at",
            Agent::Claude,
            "devbox-1",
            Status::Working,
            Some("brin"),
        ),
        (
            "schema-review",
            "payments-rewrite",
            "Review the schema",
            Agent::Codex,
            "this-mac",
            Status::Idle,
            Some("ledger"),
        ),
        (
            "refund",
            "payments-rewrite",
            "Refund retries",
            Agent::Codex,
            "devbox-1",
            Status::Working,
            None,
        ),
        (
            "rate",
            "payments-rewrite",
            "API rate limiter",
            Agent::Claude,
            "this-mac",
            Status::Idle,
            None,
        ),
        (
            "benchmark",
            "infra",
            "Benchmark BRIN",
            Agent::Claude,
            "devbox-1",
            Status::Working,
            None,
        ),
        (
            "postgres",
            "infra",
            "Upgrade Postgres 17",
            Agent::Codex,
            "devbox-1",
            Status::Exited { code: 1 },
            None,
        ),
        (
            "pr-482",
            "pr-reviews",
            "acme/api #482",
            Agent::Claude,
            "this-mac",
            Status::Idle,
            None,
        ),
    ] {
        let mut value = session(id, channel, title, agent, machine, status);
        value.parent = parent.map(SessionId::new);
        value.fork = parent.map(|_| ForkKind::Worktree);
        value.repo = if channel == "infra" {
            "acme/infra"
        } else {
            "ledger"
        }
        .into();
        value.branch = "ledger-schema".into();
        value.unread = u32::from(["ledger", "refund", "postgres"].contains(&id));
        // Frames 01–17 mark Ledger VIP: its row carries the inbox count (NV-22).
        value.vip = id == "ledger";
        value.shells = vec!["zsh".into()];
        let (activity, when) = match id {
            "ledger" => ("Asks to move PAY-3 to Done", "2m"),
            "refund" => ("Running cargo test -p refunds", "now"),
            "brin" => ("Writing migrations/0043_brin.sql", "now"),
            "partial-brin" => ("Comparing partial BRIN on posted_at", "now"),
            "benchmark" => ("pgbench run 2 of 4", "1m"),
            "postgres" => ("Exited with code 1 after the dump step", "40m"),
            "rate" => ("Finished: pushed and opened PR #491", "1h"),
            "pr-482" => ("3 new commits since your review", "2h"),
            _ => ("", ""),
        };
        if id == "refund" {
            value.agent = Agent::Copilot;
        }
        if id == "benchmark" {
            value.agent = Agent::Codex;
        }
        if id == "postgres" {
            value.agent = Agent::Claude;
        }
        match id {
            "ledger" => {
                value.started = "Today 11:40".into();
                value.restart_detail = Some(
                    "humatad holds this terminal on devbox-1, not the app. After a reboot it runs claude --resume 7f3a… in the same folder. 10,000 lines of scrollback kept on disk there.".into(),
                );
                value.permissions = vec![
                    "Owns this session, its board (4 items), its clones and the 2 items it created. Everything else goes to the Inbox.".into(),
                    "Not contained: it can read and write anything you can on devbox-1.".into(),
                ];
            }
            "brin" => value.started = "Today 12:05".into(),
            "partial-brin" => {
                value.agent = Agent::Codex;
                value.machine = MachineId::new("gpu-box");
                value.started = "Today 12:31".into();
                value.fork = Some(ForkKind::Shared);
            }
            "schema-review" => {
                value.agent = Agent::Claude;
                value.started = "Today 12:50".into();
                value.fork = Some(ForkKind::Conversation);
            }
            "refund" => {
                value.machine = MachineId::new("this-mac");
                value.started = "Yesterday".into();
            }
            "rate" => {
                value.agent = Agent::Codex;
                value.machine = MachineId::new("devbox-1");
                value.started = "Mon".into();
            }
            _ => {}
        }
        value.activity = activity.into();
        value.active = when.into();
        value.transcript = [
            (Speaker::System, "~/src/ledger (ledger-schema) $ claude"),
            (
                Speaker::You,
                "> Design the ledger schema from PAY-3 and move the reads over.",
            ),
            (Speaker::Agent, ""),
            (Speaker::Agent, "● Read(ledger/docs/model.md)"),
            (Speaker::System, "  ⎿ Read 212 lines"),
            (
                Speaker::Agent,
                "● Write(ledger/migrations/0042_accounts.sql)",
            ),
            (Speaker::System, "  ⎿ Wrote 41 lines"),
            (Speaker::Agent, "● Update(api/src/client.rs)"),
            (
                Speaker::System,
                "  ⎿ Updated with 62 additions and 21 removals",
            ),
            (Speaker::Agent, "● Bash(cargo test -p api ledger)"),
            (Speaker::Tool, "  ⎿ test result: ok. 14 passed; 0 failed"),
            (Speaker::Agent, ""),
            (Speaker::Agent, "● Updated plan"),
            (Speaker::Tool, "  ✓ Add accounts and entries tables"),
            (Speaker::Tool, "  ✓ Dual-write from client.rs"),
            (Speaker::Agent, "  ◐ Backfill entries from ledger_entries"),
            (Speaker::System, "    Switch reads, drop the old index"),
        ]
        .into_iter()
        .map(|(from, text)| Line {
            from,
            text: text.into(),
        })
        .collect();
        let tail: &[(Speaker, &str)] = match status {
            Status::Working | Status::Offline => &[
                (Speaker::Agent, ""),
                (Speaker::Agent, "● Bash(psql -f backfill.sql)"),
                (Speaker::System, "  ⎿ INSERT 0 48210 … (running)"),
                (Speaker::Agent, ""),
                (
                    Speaker::Agent,
                    "✻ Backfilling entries… (2m 14s · ↓ 3.1k tokens · esc to interrupt)",
                ),
            ],
            Status::NeedsYou => &[
                (Speaker::Agent, ""),
                (Speaker::Agent, "● humata board_item_move PAY-3 → Done"),
                (Speaker::System, "  ⎿ ⏳ waiting for your approval (Inbox)"),
                (Speaker::Agent, ""),
                (
                    Speaker::Agent,
                    "? Drop the old ledger_entries index before the switch, or keep\n  both until the backfill ends?",
                ),
                (Speaker::Agent, "  ❯ 1. Keep both"),
                (Speaker::System, "    2. Drop it now"),
                (Speaker::System, "    3. Type something else"),
            ],
            Status::Exited { .. } => &[
                (Speaker::Agent, ""),
                (
                    Speaker::Agent,
                    "● All four steps are done. Reads go to entries now.",
                ),
                (Speaker::Agent, ""),
                (Speaker::System, "~/src/ledger (ledger-schema) $ "),
            ],
            _ => &[],
        };
        value
            .transcript
            .extend(tail.iter().map(|(from, text)| Line {
                from: from.clone(),
                text: (*text).into(),
            }));
        workspace.sessions.push(value);
    }
    workspace.requests = vec![
        request(
            "move-pay-3",
            "ledger",
            "payments-rewrite",
            RequestKind::Command,
            "Move PAY-3 to Done",
        ),
        request(
            "review-question",
            "pr-482",
            "pr-reviews",
            RequestKind::Question,
            "Review the new commits first?",
        ),
    ];
    for (id, session, channel, title) in [
        (
            "refund-command",
            "refund",
            "payments-rewrite",
            "Run the integration tests",
        ),
        (
            "rate-command",
            "rate",
            "payments-rewrite",
            "Write the API rate limit config",
        ),
        (
            "benchmark-command",
            "benchmark",
            "infra",
            "Run the benchmark on devbox-1",
        ),
        (
            "postgres-command",
            "postgres",
            "infra",
            "Upgrade Postgres 17",
        ),
        (
            "review-command",
            "pr-482",
            "pr-reviews",
            "Post review on acme/api #482",
        ),
    ] {
        workspace
            .requests
            .push(request(id, session, channel, RequestKind::Command, title));
    }
    for request in &mut workspace.requests {
        match request.id.as_str() {
            "rate-command" => {
                request.id = RequestId::new("push-brin");
                request.session = SessionId::new("brin");
                request.title = "Push try-brin to origin".into();
                request.asked = "9m".into();
                request.preview = Preview::Command("git push origin try-brin".into());
                request.reason = "The index migration and its tests are ready to review.".into();
            }
            "refund-command" => {
                request.title = "Which currency for refunds?".into();
                request.kind = RequestKind::Question;
                request.asked = "15m".into();
                request.reason =
                    "Should refunds use the charge currency or the account currency?".into();
                request.preview = Preview::Text("Charge currency\nAccount currency".into());
            }
            "benchmark-command" => {
                request.title = "Run a command on gpu-box".into();
                request.asked = "22m".into();
                request.preview = Preview::Command("pgbench -f bench/brin.sql".into());
                request.reason =
                    "Compare index performance against the staging-sized sample.".into();
            }
            "postgres-command" => {
                request.title = "Delete the old snapshots".into();
                request.asked = "1h".into();
                request.kind = RequestKind::Destroy;
                request.undoable = false;
                request.reason = "The Postgres 17 restore has been verified.".into();
                request.preview =
                    Preview::Text("Remove the Postgres 16 snapshots on devbox-1.".into());
            }
            _ => {}
        }
    }
    if let Some(request) = workspace
        .requests
        .iter_mut()
        .find(|r| r.id.as_str() == "move-pay-3")
    {
        request.reason="The schema is in and the API tests pass (14 passed). The agent wants to mark PAY-3 done on the payments board. Moving items it did not create needs you.".into();
        request.preview = Preview::Text(
            "Item  PAY-3 Ledger schema\nMove  In progress → Done\nCreated by  You, 3 days ago"
                .into(),
        );
        request.asked = "2m".into();
        request.rule = "board_item_move".into();
    }
    // While Ledger works it has not asked to move PAY-3 yet (D128.5): its
    // family's only request is its fork's push.
    // Refund retries asks instead, so the Inbox still holds 7 (frames 01, 07).
    if ledger_status == Status::Working {
        workspace.requests.retain(|r| r.id.as_str() != "move-pay-3");
        let mut tests = request(
            "refund-tests",
            "refund",
            "payments-rewrite",
            RequestKind::Command,
            "Run the refund integration tests",
        );
        tests.asked = "4m".into();
        tests.preview = Preview::Command("make test-refunds".into());
        tests.reason = "The retry path changed; the integration suite covers it.".into();
        workspace.requests.push(tests);
    }
    workspace.file_contents.insert((SessionId::new("ledger"), "api".into(), "src/client.rs".into()),
        "use crate::ledger::{Entry, Ledger};\n\npub fn insert_entry(ledger: &mut Ledger, entry: Entry) {\n    ledger.insert(entry.clone());\n    ledger.insert_legacy(entry);\n}\n".into());
    // Ledger's views, newest first. The review is offered while Ledger
    // needs the person (frames 03/04) and was opened before otherwise.
    workspace.views.push(PreparedView {
        title: "Review: ledger schema".into(), session: SessionId::new("ledger"),
        body: "A guided walk through the 3 files, risky parts first.\n\n1. api/src/client.rs — keep the dual write in one transaction.\n2. migrations/0042_accounts.sql — check the account key.\n3. backfill.sql — batches, timings and rollback.".into(),
        accepted: ledger_status != Status::NeedsYou,
        note: "A guided walk through the 3 files, risky parts first".into(),
        prepared: "14m".into(),
    });
    workspace.views.push(PreparedView {
        title: "Backfill plan".into(),
        session: SessionId::new("ledger"),
        body: "Batches, timings and the rollback for moving ledger_entries into entries.".into(),
        accepted: true,
        note: "Batches, timings and the rollback".into(),
        prepared: "1h".into(),
    });
    workspace.views.push(PreparedView {
        title: "Index benchmark".into(),
        session: SessionId::new("ledger"),
        body: "BRIN against B-tree on posted_at over 48k rows.".into(),
        accepted: true,
        note: "BRIN vs B-tree on 48k rows".into(),
        prepared: "yesterday".into(),
    });
    workspace.pulls = vec![
        PullRequest {
            number: 482,
            title: "acme/api #482".into(),
            repo: "acme/api".into(),
            session: Some(SessionId::new("pr-482")),
            checks: "3 new".into(),
        },
        PullRequest {
            number: 491,
            title: "API rate limiter".into(),
            repo: "acme/api".into(),
            session: Some(SessionId::new("rate")),
            checks: "Passed".into(),
        },
    ];
    use crate::model::{Activity, ActivityKind as K};
    workspace.activity = [
        (
            K::Inbox,
            "Asks to move PAY-3 to Done",
            "Claude Code · Ledger schema migration",
            "2m",
            Some("ledger"),
        ),
        (
            K::Fork,
            "Forked “Try BRIN index” → “Partial BRIN on posted_at” (code only)",
            "You",
            "14m",
            Some("partial-brin"),
        ),
        (
            K::Turn,
            "Finished a turn: +184 −24 across 2 repos",
            "Claude Code · Ledger schema migration",
            "20m",
            Some("ledger"),
        ),
        (
            K::Item,
            "Created PAY-9 “Backfill progress bar”",
            "Claude Code · Ledger schema migration",
            "22m",
            Some("ledger"),
        ),
        (K::Offline, "gpu-box went offline", "Humata", "41m", None),
        (
            K::PullRequest,
            "Pushed rate-limiter and opened PR #491",
            "Codex · API rate limiter",
            "1h",
            Some("rate"),
        ),
    ]
    .into_iter()
    .map(|(kind, text, by, at, session)| Activity {
        channel: ChannelId::new("payments-rewrite"),
        session: session.map(SessionId::new),
        kind,
        text: text.into(),
        by: by.into(),
        at: at.into(),
    })
    .collect();
    workspace.items = [
        (
            "PAY-12",
            "payments-rewrite",
            "Multi-currency ledgers",
            ItemStatus::Backlog,
            None,
        ),
        (
            "INF-4",
            "infra",
            "Move CI to devbox runners",
            ItemStatus::Backlog,
            None,
        ),
        (
            "PAY-9",
            "payments-rewrite",
            "Backfill progress bar",
            ItemStatus::Todo,
            Some("ledger"),
        ),
        (
            "PAY-7",
            "payments-rewrite",
            "Refund retry with backoff",
            ItemStatus::Todo,
            None,
        ),
        (
            "PAY-3",
            "payments-rewrite",
            "Ledger schema v2",
            ItemStatus::InProgress,
            Some("ledger"),
        ),
        (
            "INF-2",
            "infra",
            "Postgres 17 upgrade",
            ItemStatus::InProgress,
            Some("postgres"),
        ),
        (
            "PAY-5",
            "payments-rewrite",
            "API rate limiter",
            ItemStatus::InReview,
            Some("rate"),
        ),
        (
            "PAY-2",
            "payments-rewrite",
            "Currency codes table",
            ItemStatus::Done,
            None,
        ),
        (
            "INF-1",
            "infra",
            "BRIN benchmark harness",
            ItemStatus::Done,
            Some("benchmark"),
        ),
    ]
    .into_iter()
    .map(|(id, channel, title, status, session)| Item {
        id: ItemId::new(id),
        scope: Scope::Channel(ChannelId::new(channel)),
        kind: ItemKind::Task,
        title: title.into(),
        status,
        depends_on: Vec::new(),
        session: session.map(SessionId::new),
        parent: None,
    })
    .collect();
    if let Some(item) = workspace
        .items
        .iter_mut()
        .find(|i| i.id.as_str() == "PAY-3")
    {
        item.depends_on.push(ItemId::new("PAY-2"));
    }
    let request = workspace
        .waiting()
        .find(|r| r.session.as_str() == "ledger" && r.title.contains("PAY-3"))
        .map(|r| r.id.clone());
    workspace.item_details.insert(ItemId::new("PAY-3"), crate::model::ItemDetails {
        request_summary: "Migration and backfill pass on staging; 3.1M rows in 4m12s.".into(),
        description: "Split postings into ledger_entries and ledger_balances, backfill from the old table in batches, and keep both in step until cut-over.".into(),
        history: vec!["Moved to In progress by Ledger schema migration · 1h".into(), "Created by you · Mon".into()],
        sessions: vec![SessionId::new("ledger"), SessionId::new("brin")], request, requested_status: Some(ItemStatus::Done),
        ..Default::default()
    });
    workspace.item_details.insert(
        ItemId::new("PAY-9"),
        crate::model::ItemDetails {
            created_by: Some(SessionId::new("ledger")),
            ..Default::default()
        },
    );
    for (id, day) in [("PAY-2", 10), ("INF-1", 13)] {
        workspace.item_details.insert(
            ItemId::new(id),
            crate::model::ItemDetails {
                done_day: Some(day),
                session_label: (id == "PAY-2").then(|| "Currency codes".into()),
                ..Default::default()
            },
        );
    }
    workspace.diffs = vec![
        RepoDiff { session: SessionId::new("ledger"), repo: "ledger".into(), base: "main".into(), files: vec![FileDiff {path: "migrations/0042_accounts.sql".into(),added:41,removed:0,lines:Vec::new()}],comments:Vec::new() },
        RepoDiff { session: SessionId::new("ledger"), repo: "api".into(), base: "main".into(), files: vec![
            FileDiff {path:"src/client.rs".into(),added:62,removed:21,lines:lines(&[
                (LineKind::Hunk,"@@ -40,7 +40,12 @@ impl Client {"),
                (LineKind::Context,"    pub async fn post(&self, entry: Entry)"),
                (LineKind::Removed,"        self.db.insert_legacy(&entry).await?;"),
                (LineKind::Added,"        let tx = self.db.begin().await?;"),
                (LineKind::Added,"        tx.insert_entry(&entry).await?;"),
                (LineKind::Added,"        tx.insert_legacy(&entry).await?;"),
                (LineKind::Added,"        tx.commit().await?;"),
            ],39)},
            FileDiff {path:"src/entry.rs".into(),added:81,removed:3,lines:Vec::new()},
        ],comments:vec![Comment {path:"src/client.rs".into(),line:41,body:"Wrap the two writes in one transaction so a crash can't leave them out of step.".into(),sent:true,context:None}] },
    ];
    for value in &workspace.sessions {
        workspace.session_files.insert(
            value.id.clone(),
            vec![
                SessionFile {
                    repo: "ledger".into(),
                    path: "migrations/0042_accounts.sql".into(),
                    folder: false,
                },
                SessionFile {
                    repo: "ledger".into(),
                    path: "migrations/0041_init.sql".into(),
                    folder: false,
                },
                SessionFile {
                    repo: "ledger".into(),
                    path: "docs".into(),
                    folder: true,
                },
                SessionFile {
                    repo: "ledger".into(),
                    path: "README.md".into(),
                    folder: false,
                },
                SessionFile {
                    repo: "api".into(),
                    path: "src/lib.rs".into(),
                    folder: false,
                },
                SessionFile {
                    repo: "api".into(),
                    path: "src/client.rs".into(),
                    folder: false,
                },
                SessionFile {
                    repo: "api".into(),
                    path: "src/entry.rs".into(),
                    folder: false,
                },
                SessionFile {
                    repo: "api".into(),
                    path: "Cargo.toml".into(),
                    folder: false,
                },
            ],
        );
    }
    for (session, files) in &workspace.session_files {
        for file in files {
            let body = match (file.repo.as_str(), file.path.as_str()) {
                ("ledger", "migrations/0042_accounts.sql") => {
                    "CREATE TABLE accounts (\n    id BIGINT PRIMARY KEY,\n    currency CHAR(3) NOT NULL,\n    balance BIGINT NOT NULL DEFAULT 0\n);\n"
                }
                ("ledger", "migrations/0041_init.sql") => {
                    "CREATE TABLE ledger_entries (\n    id BIGINT PRIMARY KEY,\n    posted_at TIMESTAMPTZ NOT NULL\n);\n"
                }
                ("api", "src/lib.rs") => "pub mod client;\npub mod entry;\n",
                ("ledger", "README.md") => {
                    "# Ledger\n\nAccounts and journal entries for payments.\n\nApply migrations in order before starting the backfill.\n"
                }
                ("api", "src/entry.rs") => {
                    "pub struct Entry {\n    pub account_id: i64,\n    pub amount: i64,\n    pub currency: String,\n}\n"
                }
                ("api", "Cargo.toml") => {
                    "[package]\nname = \"payments-api\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
                }
                _ => continue,
            };
            workspace
                .file_contents
                .entry((session.clone(), file.repo.clone(), file.path.clone()))
                .or_insert_with(|| body.into());
        }
    }
    if let Some(files) = workspace
        .session_files
        .get_mut(&SessionId::new("partial-brin"))
    {
        files.retain(|f| f.repo != "api");
    }
    workspace
}

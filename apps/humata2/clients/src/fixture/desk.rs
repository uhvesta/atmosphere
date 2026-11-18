//! The Desk's dummy data: frames 34, 36 and 55 of the design.

use crate::desk::{
    Access, ChannelDeskSession, Closed, DeskSession, Document, FOLDER, Folder, LastUsed, Switches,
};
use crate::model::{
    Agent, ChannelId, Item, ItemDetails, ItemId, ItemKind, ItemStatus, Line, Preview,
    RequestKind, Scope, Session, SessionFile, SessionId, Speaker, Status,
};
use crate::workspace::Workspace;

const MB: u64 = 1_000_000;

const RFC_BRIN: &str = "# BRIN index rollout

_Draft · written with Claude Code_

## Problem

ledger_entries is 410 GB and its btree on posted_at is 38 GB.
Range reads by day are 92% of the load.

## Proposal

- Add a BRIN index on posted_at with 64 pages per range (PAY-13).
- Keep the btree until the backfill ends, then drop it in one
  migration.
- Watch p95 for range reads on the metrics board (PAY-14).

## Bench

```
btree  38 GB   range p95 41 ms
brin   2.1 MB  range p95 57 ms
```
";

const BENCH_RESULTS: &str = "# Bench results

| index | size | range p95 |
|---|---|---|
| btree | 38 GB | 41 ms |
| brin | 2.1 MB | 57 ms |
";

const Q4_PLAN: &str = "# Q4 board plan

- Settle the three outcomes for the quarter.
- Turn each into a milestone with an owner.
- Review the plan on Friday.
";

/// The channel Desk of frames 91 and 95: Payments' Desk has two sessions,
/// one for the whole channel and one for the migration session. Not part of
/// [`design`](super::design): a test adds it.
pub fn channel_desk(workspace: &mut Workspace) {
    let channel = ChannelId::new("payments-rewrite");
    let rows = [
        (
            "cdesk-triage",
            "Triage flaky tests",
            Agent::Claude,
            "this-mac",
            Status::Working,
            None,
            "Today 13:10",
            vec!["flaky-report.md", "retry-plan.md"],
        ),
        (
            "cdesk-backfill",
            "Watch the backfill",
            Agent::Codex,
            "devbox-1",
            Status::Idle,
            Some("ledger"),
            "Today 11:52",
            Vec::new(),
        ),
    ];
    for (order, (id, title, agent, machine, status, target, label, documents)) in
        rows.into_iter().enumerate()
    {
        workspace
            .sessions
            .push(session(id, title, agent, machine, status));
        let name = title.to_lowercase().replace(' ', "-");
        workspace.desk.channel_sessions.insert(
            SessionId::new(id),
            ChannelDeskSession {
                desk: DeskSession::fresh(
                    SessionId::new(id),
                    Folder::Scratch {
                        path: format!("~/Humata/payments-rewrite/desk/{name}"),
                        kept: false,
                    },
                    LastUsed {
                        order: u64::try_from(order).unwrap_or(0) + 1,
                        label: label.to_owned(),
                    },
                ),
                channel: channel.clone(),
                target: target.map(SessionId::new),
                switches: Switches::default(),
                blocked: Vec::new(),
                target_gone: false,
                document_lines: documents
                    .iter()
                    .zip([54_u32, 23])
                    .map(|(name, lines)| ((*name).to_owned(), lines))
                    .collect(),
                documents: documents.into_iter().map(str::to_owned).collect(),
            },
        );
    }
}

/// A Desk session's [`Session`]: in the Desk's channel, with no repository
/// and no branch (DK-12).
pub(crate) fn session(
    id: &str,
    title: &str,
    agent: Agent,
    machine: &str,
    status: Status,
) -> Session {
    let mut value = super::session(id, FOLDER, title, agent, machine, status);
    value.branch = String::new();
    value.repo = String::new();
    value
}

/// What one fixture Desk session is made from.
struct Entry {
    id: &'static str,
    title: &'static str,
    agent: Agent,
    machine: &'static str,
    folder: Folder,
    repos: &'static [&'static str],
    order: u64,
    label: &'static str,
    closed: Option<Closed>,
    bytes: Option<u64>,
    unpushed: Option<&'static str>,
    status: Status,
}

fn add(workspace: &mut Workspace, entry: Entry) {
    let mut value = session(
        entry.id,
        entry.title,
        entry.agent,
        entry.machine,
        entry.status,
    );
    value.archived = entry.closed.is_some();
    let id = SessionId::new(entry.id);
    workspace.sessions.push(value);
    workspace.desk.sessions.insert(
        id.clone(),
        DeskSession {
            session: id.clone(),
            has_versions: entry.folder.is_scratch(),
            doc_head: String::new(),
            version_count: 0,
            holds_files: false,
            folder: entry.folder,
            repos: entry.repos.iter().map(|repo| (*repo).to_owned()).collect(),
            access: Access::default(),
            last_used: LastUsed {
                order: entry.order,
                label: entry.label.to_owned(),
            },
            closed: entry.closed,
            unpushed: entry.unpushed.map(str::to_owned),
        },
    );
    if let Some(bytes) = entry.bytes {
        workspace.desk.folder_bytes.insert(id, bytes);
    }
}

fn scratch(name: &str, kept: bool) -> Folder {
    Folder::Scratch {
        path: crate::desk::scratch_path(name),
        kept,
    }
}

fn file(path: &str, folder: bool) -> SessionFile {
    SessionFile {
        repo: String::new(),
        path: path.to_owned(),
        folder,
    }
}

/// Text lines for a terminal: `System` lines are drawn dim, `Agent` lines plain.
fn lines(text: &[(Speaker, &str)]) -> Vec<Line> {
    text.iter()
        .map(|(from, text)| Line {
            from: *from,
            text: (*text).to_owned(),
        })
        .collect()
}

/// What the agent's terminal of Plan Q4 board shows (frame 37).
fn plan_q4_terminal() -> Vec<Line> {
    use Speaker::{Agent as A, System as S};
    lines(&[
        (S, "plan-q4 $ claude"),
        (
            A,
            "> Plan Q4 for payments: add the backfill follow-ups, and make a",
        ),
        (A, "  channel for the web revamp."),
        (A, ""),
        (A, "● humata board_items channel=payments-rewrite"),
        (S, "  ⎿ 6 items · 1 in progress · 1 in review"),
        (
            A,
            "● humata board_item_create PAY-13 “Drop the ledger_entries index”",
        ),
        (S, "  ⎿ Added to # payments-rewrite · To do"),
        (
            A,
            "● humata board_item_create PAY-14 “Backfill metrics dashboard”",
        ),
        (
            S,
            "  ⎿ Added to # payments-rewrite · To do · depends on PAY-13",
        ),
        (A, "● humata channel_create web-revamp repos=acme/web"),
        (S, "  ⎿ ⏳ waiting for your approval (Inbox)"),
        (A, ""),
        (A, "● Write(q4-plan.md)"),
        (S, "  ⎿ Wrote 38 lines"),
        (A, ""),
        (A, "✻ Ordering the payments lanes… (48s · esc to interrupt)"),
    ])
}

/// What the `zsh` tab of BRIN rollout RFC shows (frame 38): the shell
/// transcript, ending at an empty prompt.
fn brin_rfc_shell() -> Vec<u8> {
    use Speaker::{Agent as A, System as S};
    let shown = lines(&[
        (S, "~/Humata/desk/brin-rfc $ humata desk move devbox-1"),
        (A, "Copying 3 files to devbox-1… done"),
        (A, "claude restarts on devbox-1 with its conversation."),
        (
            S,
            "~/Humata/desk/brin-rfc $ humata desk add acme/ledger acme/api",
        ),
        (A, "Cloned acme/ledger (main) into ./ledger"),
        (A, "Cloned acme/api (main) into ./api"),
        (S, "~/Humata/desk/brin-rfc $ humata desk keep"),
        (A, "This folder stays after the session closes."),
        (S, "~/Humata/desk/brin-rfc $ "),
    ]);
    let mut bytes = Vec::new();
    for line in &shown {
        bytes.extend(super::terminal_line(line));
    }
    // The last line is the prompt: the cursor stays on it.
    bytes.truncate(bytes.len().saturating_sub(2));
    bytes
}

/// The two items Plan Q4 board's agent added to the payments board, as its
/// terminal says (frame 37): both in To do and marked as `session`'s.
/// Skipped when the workspace has no payments channel, and an item the
/// workspace already has is left as it is.
fn plan_q4_items(workspace: &mut Workspace, session: &SessionId) {
    let channel = ChannelId::new("payments-rewrite");
    if workspace.channel(&channel).is_none() {
        return;
    }
    for (id, title, depends_on) in [
        ("PAY-13", "Drop the ledger_entries index", None),
        ("PAY-14", "Backfill metrics dashboard", Some("PAY-13")),
    ] {
        let id = ItemId::new(id);
        if workspace.items.iter().any(|item| item.id == id) {
            continue;
        }
        workspace.items.push(Item {
            id: id.clone(),
            scope: Scope::Channel(channel.clone()),
            kind: ItemKind::Task,
            title: title.to_owned(),
            status: ItemStatus::Todo,
            depends_on: depends_on.map(ItemId::new).into_iter().collect(),
            session: None,
            parent: None,
            revision: 0,
        });
        workspace.item_details.insert(
            id,
            ItemDetails {
                created_by: Some(session.clone()),
                ..ItemDetails::default()
            },
        );
    }
}

/// The clones `humata desk add` made in `session`'s folder (frame 38): each
/// sits on its repo's default branch.
fn clones(workspace: &mut Workspace, session: &SessionId, repos: &[&str]) {
    for repo in repos {
        workspace
            .desk
            .clone_branches
            .insert((session.clone(), (*repo).to_owned()), "main".to_owned());
    }
}

/// Adds the Desk of frame 36 to `workspace`. Its sessions were renamed after
/// they started ("Plan Q4", then "Plan Q4 board"), so their folders keep the
/// names they started with (D148). The workspace needs the machines
/// `this-mac` and `devbox-1` (such as [`super::design`]): seven sessions (four of them closed),
/// two documents, and the sizes of their scratch folders.
pub fn desk(workspace: &mut Workspace) {
    add(
        workspace,
        Entry {
            id: "desk-plan-q4",
            title: "Plan Q4 board",
            agent: Agent::Claude,
            machine: "this-mac",
            folder: scratch("plan-q4", false),
            repos: &[],
            order: 40,
            label: "now",
            closed: None,
            bytes: Some(2 * MB),
            unpushed: None,
            status: Status::Working,
        },
    );
    add(
        workspace,
        Entry {
            id: "desk-brin-rfc",
            title: "BRIN rollout RFC",
            agent: Agent::Claude,
            machine: "devbox-1",
            folder: scratch("brin-rfc", true),
            repos: &["acme/ledger", "acme/api"],
            order: 30,
            label: "Today 12:05",
            closed: None,
            bytes: Some(310 * MB),
            unpushed: None,
            status: Status::Idle,
        },
    );
    add(
        workspace,
        Entry {
            id: "desk-tidy-infra",
            title: "Tidy infra items",
            agent: Agent::Shell,
            machine: "this-mac",
            folder: Folder::Picked {
                path: "~/notes".to_owned(),
            },
            repos: &[],
            order: 10,
            label: "Nov 2",
            closed: None,
            bytes: None,
            unpushed: None,
            status: Status::Idle,
        },
    );
    add(
        workspace,
        Entry {
            id: "desk-release-checklist",
            title: "Sketch the release checklist",
            agent: Agent::Codex,
            machine: "this-mac",
            folder: scratch("release-checklist", false),
            repos: &[],
            order: 20,
            label: "Nov 14",
            closed: Some(Closed {
                when: "Nov 14".to_owned(),
                days_left: Some(6),
            }),
            bytes: Some(120 * MB),
            unpushed: None,
            status: Status::Exited { code: 0 },
        },
    );

    let closed = |when: &str| {
        Some(Closed {
            when: when.to_owned(),
            days_left: None,
        })
    };
    add(
        workspace,
        Entry {
            id: "desk-onboarding-doc",
            title: "Draft the onboarding doc",
            agent: Agent::Claude,
            machine: "this-mac",
            folder: scratch("onboarding-doc", true),
            repos: &[],
            order: 3,
            label: "Nov 3",
            closed: closed("Nov 3"),
            bytes: Some(48 * MB),
            unpushed: None,
            status: Status::Exited { code: 0 },
        },
    );
    add(
        workspace,
        Entry {
            id: "desk-port-cron",
            title: "Port the cron jobs",
            agent: Agent::Codex,
            machine: "devbox-1",
            folder: scratch("port-cron", false),
            repos: &["acme/api"],
            order: 6,
            label: "Nov 6",
            closed: closed("Nov 6"),
            bytes: Some(2_300 * MB),
            unpushed: Some("acme/api: 2 commits on port-cron never pushed"),
            status: Status::Exited { code: 0 },
        },
    );
    add(
        workspace,
        Entry {
            id: "desk-notes-cleanup",
            title: "Notes cleanup",
            agent: Agent::Shell,
            machine: "this-mac",
            folder: Folder::Picked {
                path: "~/notes".to_owned(),
            },
            repos: &[],
            order: 9,
            label: "Nov 9",
            closed: closed("Nov 9"),
            bytes: None,
            unpushed: None,
            status: Status::Exited { code: 0 },
        },
    );

    let brin = SessionId::new("desk-brin-rfc");
    let plan = SessionId::new("desk-plan-q4");
    workspace.desk.documents = vec![
        Document {
            session: brin.clone(),
            path: "rfc-brin.md".to_owned(),
        },
        Document {
            session: plan.clone(),
            path: "q4-plan.md".to_owned(),
        },
    ];
    for (session, path, text) in [
        (&brin, "rfc-brin.md", RFC_BRIN),
        (&brin, "notes/bench-results.md", BENCH_RESULTS),
        (&plan, "q4-plan.md", Q4_PLAN),
    ] {
        workspace.file_contents.insert(
            (session.clone(), String::new(), path.to_owned()),
            text.to_owned(),
        );
    }
    for document in &workspace.desk.documents {
        if let Some(record) = workspace.desk.sessions.get_mut(&document.session) {
            record.holds_files = true;
        }
    }
    // Frame 38: the shell tab of BRIN rollout RFC, and what its Files panel
    // lists: the notes and the two clones `humata desk add` made.
    for session in &mut workspace.sessions {
        match session.id.as_str() {
            "desk-plan-q4" => {
                session.transcript = plan_q4_terminal();
                // Frame 37 draws a zsh tab beside the agent's.
                session.shells = vec!["zsh".to_owned()];
            }
            "desk-brin-rfc" => session.shells = vec!["zsh".to_owned()],
            _ => {}
        }
    }
    let mut create = super::requests::request(
        "desk-create-web-revamp",
        "desk-plan-q4",
        crate::desk::channel().as_str(),
        RequestKind::Command,
        "Create channel web-revamp",
    );
    create.asked = "1m".to_owned();
    create.rule = "channel_create".to_owned();
    create.preview = Preview::Text("Channel  web-revamp\nRepos  acme/web".to_owned());
    create.reason = "Make channels and add machines is off for this session, so making a channel asks you.".to_owned();
    workspace.requests.push(create);
    clones(workspace, &brin, &["acme/ledger", "acme/api"]);
    plan_q4_items(workspace, &plan);
    workspace
        .terminal_bytes
        .insert((brin.clone(), 1), brin_rfc_shell());
    workspace.session_files.insert(
        brin,
        vec![
            file("rfc-brin.md", false),
            file("notes", true),
            file("notes/bench-results.md", false),
            file("ledger", true),
            file("api", true),
        ],
    );
    workspace
        .session_files
        .insert(plan, vec![file("q4-plan.md", false)]);
}

/// A fresh install (frame 34): only This Mac, no channels, no sessions.
#[must_use]
pub fn empty() -> Workspace {
    Workspace {
        machines: super::machines().into_iter().take(1).collect(),
        ..Workspace::default()
    }
}

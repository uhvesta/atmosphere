//! The Desk's dummy data: frames 34, 36 and 55 of the design.

use crate::desk::{Access, Closed, DeskSession, Document, FOLDER, Folder, LastUsed};
use crate::model::{Agent, Session, SessionFile, SessionId, Status};
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

/// Adds the Desk of frame 36 to `workspace`. Its sessions were renamed after
/// they started ("Plan Q4", then "Plan Q4 board"), so their folders keep the
/// names they started with (D148). The workspace needs the machines
/// `this-mac` and `devbox-1` (such as [`super::design`]): four sessions,
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
    workspace.session_files.insert(
        brin,
        vec![
            file("rfc-brin.md", false),
            file("notes", true),
            file("notes/bench-results.md", false),
        ],
    );
    workspace
        .session_files
        .insert(plan, vec![file("q4-plan.md", false)]);
}

/// Adds the Desk of frame 55: [`desk`], unless the workspace has it already,
/// and three more closed sessions, so Clean up… has something to list.
pub fn clean_up(workspace: &mut Workspace) {
    if workspace.desk.sessions.is_empty() {
        desk(workspace);
    }
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
}

/// A fresh install (frame 34): only This Mac, no channels, no sessions.
#[must_use]
pub fn empty() -> Workspace {
    Workspace {
        machines: super::machines().into_iter().take(1).collect(),
        ..Workspace::default()
    }
}

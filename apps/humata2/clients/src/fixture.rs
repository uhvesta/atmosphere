//! The dummy data the fakes start with. Deterministic: the same call gives
//! the same workspace every time, so tests and snapshots can name what they
//! see.
//!
//! [`standard`] has a few of everything: three hosts (one offline), four
//! channels, sessions in every state with forks two deep, Inbox requests of
//! every kind, diffs with comments, boards with dependency edges, files,
//! transfers, trash, pull requests, specs, a journal and prepared views.
//! [`large`] adds thousands of sessions, for the virtualized sidebar.

mod channels;
mod desk;
mod design_session;
pub(crate) mod diffs;
mod files;
mod items;
mod journal;
mod machines;
mod requests;
mod rules;
mod sessions;
mod terminal;

use std::collections::HashMap;

use crate::model::{
    Agent, Channel, ChannelId, ForkKind, MachineId, PreparedView, PullRequest, Recovery,
    SessionFile, SessionId, Skill, Spec, Status, Transfer, TransferId, TransferState, Trashed,
};
use crate::workspace::Workspace;

use channels::channels;
use diffs::diffs;
use files::files;
use items::items;
use journal::journal;
use machines::machines;
use requests::requests;
use rules::rules;
use sessions::{session, sessions};

pub use desk::{channel_desk, desk, empty};
pub use design_session::design_session;
pub use requests::{browser_read_request, browser_write_request};
pub(crate) use desk::session as desk_session;
pub(crate) use terminal::{extra_terminal, terminal_line, terminal_screen};

/// The standard workspace: a few of everything.
#[must_use]
pub fn standard() -> Workspace {
    let mut workspace = Workspace {
        machines: machines(),
        channels: channels(),
        ..Workspace::default()
    };
    workspace.sessions = sessions();
    workspace.requests = requests();
    workspace.rules = rules(
        &workspace
            .channels
            .iter()
            .map(|channel| channel.id.clone())
            .collect::<Vec<_>>(),
    );
    workspace.items = items();
    workspace.diffs = diffs();
    workspace.files = files();
    for session in &workspace.sessions {
        if session.repo.is_empty() {
            continue;
        }
        let mut files = vec![
            SessionFile {
                repo: session.repo.clone(),
                path: "README.md".into(),
                folder: false,
            },
            SessionFile {
                repo: session.repo.clone(),
                path: "src".into(),
                folder: true,
            },
            SessionFile {
                repo: session.repo.clone(),
                path: "src/lib.rs".into(),
                folder: false,
            },
        ];
        for diff in workspace
            .diffs
            .iter()
            .filter(|diff| diff.session == session.id)
        {
            for file in &diff.files {
                files.push(SessionFile {
                    repo: diff.repo.clone(),
                    path: file.path.clone(),
                    folder: false,
                });
            }
        }
        workspace.session_files.insert(session.id.clone(), files);
    }
    workspace.transfers = vec![
        Transfer {
            id: TransferId::new("t1"),
            path: "~/humata/web/build/app.tar.zst".to_owned(),
            from: MachineId::new("gpu-box"),
            to: MachineId::new("this-mac"),
            state: TransferState::Copying(42),
        },
        Transfer {
            id: TransferId::new("t2"),
            path: "~/humata/api/fixtures.sql".to_owned(),
            from: MachineId::new("this-mac"),
            to: MachineId::new("gpu-box"),
            state: TransferState::Done,
        },
        Transfer {
            id: TransferId::new("t3"),
            path: "~/humata/infra/state.tfstate".to_owned(),
            from: MachineId::new("ci-runner"),
            to: MachineId::new("this-mac"),
            state: TransferState::Failed,
        },
    ];
    workspace.trash = vec![
        Trashed {
            what: "Session “Spike: sqlite FTS”".to_owned(),
            expires: "in 29 days".to_owned(),
        },
        Trashed {
            what: "Channel #old-landing".to_owned(),
            expires: "in 12 days".to_owned(),
        },
    ];
    workspace.pulls = vec![
        PullRequest {
            number: 412,
            title: "Retry token refresh once".to_owned(),
            repo: "acme/api".to_owned(),
            session: Some(SessionId::new("s-auth")),
            checks: "3 of 4 checks passed".to_owned(),
        },
        PullRequest {
            number: 87,
            title: "Dark mode tokens".to_owned(),
            repo: "acme/web".to_owned(),
            session: Some(SessionId::new("s-theme")),
            checks: "All checks passed".to_owned(),
        },
    ];
    workspace.specs = vec![Spec {
        title: "Login v2".to_owned(),
        channel: ChannelId::new("web"),
        body: "Sign in with a passkey first; fall back to email codes.".to_owned(),
    }];
    workspace.journal = journal();
    workspace.views = vec![
        PreparedView {
            title: "Review: token refresh".to_owned(),
            session: SessionId::new("s-auth"),
            body: "Three files to read first: refresh.rs, client.rs, retry.rs.".to_owned(),
            accepted: false,
            note: "Three files to read first".to_owned(),
            prepared: "5 min ago".to_owned(),
        },
        PreparedView {
            title: "Board: login milestones".to_owned(),
            session: SessionId::new("s-login"),
            body: "The login milestone with its blocked task first.".to_owned(),
            accepted: true,
            note: "The login milestone, blocked task first".to_owned(),
            prepared: "1 h ago".to_owned(),
        },
    ];
    workspace.skills = [
        ("review", "Review the diff and leave comments"),
        ("plan", "Write a plan before changing code"),
        ("test", "Run the tests and fix what fails"),
        ("commit", "Commit what's staged with a good message"),
        ("explain", "Explain the code around the cursor"),
    ]
    .into_iter()
    .map(|(name, summary)| Skill {
        name: name.to_owned(),
        summary: summary.to_owned(),
    })
    .collect();
    workspace
}

/// The standard workspace plus `extra` sessions spread over ten more
/// channels, with forks, for the virtualized sidebar.
#[must_use]
pub fn large(extra: usize) -> Workspace {
    let mut workspace = standard();
    let channels = 10;
    for index in 0..channels {
        workspace.channels.push(Channel {
            id: ChannelId::new(format!("bulk-{index:02}")),
            name: format!("bulk-{index:02}"),
            repos: vec![format!("acme/bulk-{index:02}")],
            machines: vec![MachineId::new("this-mac")],
            purpose: "Many sessions at once".to_owned(),
            default_agent: None,
        });
    }
    let bulk: Vec<_> = (0..channels)
        .map(|index| ChannelId::new(format!("bulk-{index:02}")))
        .collect();
    workspace.rules.extend(rules(&bulk));
    for index in 0..extra {
        let channel = ChannelId::new(format!("bulk-{:02}", index % channels));
        let parent = if index % 5 == 0 || index < channels {
            None
        } else {
            Some(SessionId::new(format!("bulk-s{}", index - channels)))
        };
        let status = match index % 6 {
            0 => Status::Working,
            1 => Status::NeedsYou,
            2 => Status::Idle,
            3 => Status::ProbablyIdle,
            4 => Status::Exited { code: 0 },
            _ => Status::Idle,
        };
        let mut session = session(
            &format!("bulk-s{index}"),
            channel.as_str(),
            &format!("Task {index:04}"),
            Agent::ALL[index % 4],
            "this-mac",
            status,
        );
        session.fork = parent.as_ref().map(|_| ForkKind::Worktree);
        session.parent = parent;
        workspace.sessions.push(session);
    }
    workspace
}

/// The workspace shown in the owner's approved D76 frames. The demo and
/// full-screen comparisons share this data; `standard` keeps the broader
/// boundary fixtures stable.
#[must_use]
pub fn design() -> Workspace {
    let mut workspace = design_session(Status::NeedsYou);
    let channels: Vec<_> = workspace
        .channels
        .iter()
        .map(|channel| channel.id.clone())
        .collect();
    workspace.rules = rules(&channels);
    workspace
}

/// A fresh install with its first channel just made (frame 35): [`empty`]
/// plus `web-revamp` on This Mac, with no repos, no sessions and no board
/// items.
#[must_use]
pub fn fresh_channel() -> Workspace {
    let mut workspace = empty();
    workspace.channels.push(Channel {
        id: ChannelId::new("web-revamp"),
        name: "web-revamp".to_owned(),
        repos: Vec::new(),
        machines: vec![MachineId::new("this-mac")],
        purpose: String::new(),
        default_agent: None,
    });
    workspace
}

/// What reconnecting ci-runner does to each of its sessions: one of each
/// outcome PER-26 names.
#[must_use]
pub fn recoveries() -> HashMap<SessionId, Recovery> {
    HashMap::from([
        (SessionId::new("s-tf"), Recovery::Reattached),
        (SessionId::new("s-runner"), Recovery::Resumed),
        (
            SessionId::new("s-backup"),
            Recovery::Recoverable {
                needs: "its worktree was deleted; restore it from Trash or start it fresh"
                    .to_owned(),
            },
        ),
    ])
}

//! The dummy data the fakes start with. Deterministic: the same call gives
//! the same workspace every time, so tests and snapshots can name what they
//! see.
//!
//! [`standard`] has a few of everything: three hosts (one offline), four
//! channels, sessions in every state with forks two deep, Inbox requests of
//! every kind, diffs with comments, boards with dependency edges, files,
//! transfers, trash, pull requests, specs, a journal and prepared views.
//! [`large`] adds thousands of sessions, for the virtualized sidebar.

use std::collections::HashMap;

use crate::model::{
    Agent, Answer, Channel, ChannelId, Comment, DiffLine, FileDiff, FileEntry, ForkKind, Item,
    ItemId, ItemKind, ItemStatus, JournalEntry, Line, LineKind, Machine, MachineId, MachineKind,
    PreparedView, Preview, PullRequest, Recovery, RepoDiff, Request, RequestId, RequestKind, Scope, Session,
    SessionId, Skill, Speaker, Spec, Status, Transfer, TransferId, TransferState, Trashed, Turn,
};
use crate::workspace::Workspace;

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
    workspace.items = items();
    workspace.diffs = diffs();
    workspace.files = files();
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
        },
        PreparedView {
            title: "Board: login milestones".to_owned(),
            session: SessionId::new("s-login"),
            body: "The login milestone with its blocked task first.".to_owned(),
            accepted: true,
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
        });
    }
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

fn machines() -> Vec<Machine> {
    vec![
        Machine {
            id: MachineId::new("this-mac"),
            name: "This Mac".to_owned(),
            kind: MachineKind::ThisMac,
            online: true,
            detail: "macOS 26.7 · M4 Max · 64 GB".to_owned(),
            last_seen: "now".to_owned(),
        },
        Machine {
            id: MachineId::new("gpu-box"),
            name: "gpu-box".to_owned(),
            kind: MachineKind::Ssh {
                address: "dev@gpu-box.lan".to_owned(),
            },
            online: true,
            detail: "Ubuntu 26.04 · RTX 6090 · 128 GB".to_owned(),
            last_seen: "now".to_owned(),
        },
        Machine {
            id: MachineId::new("ci-runner"),
            name: "ci-runner".to_owned(),
            kind: MachineKind::Provider {
                provider: "Hetzner".to_owned(),
            },
            online: false,
            detail: "Debian 13 · 16 vCPU".to_owned(),
            last_seen: "14 min ago".to_owned(),
        },
    ]
}

fn channels() -> Vec<Channel> {
    let channel = |id: &str, repos: &[&str], machines: &[&str], purpose: &str| Channel {
        id: ChannelId::new(id),
        name: id.to_owned(),
        repos: repos.iter().map(|repo| (*repo).to_owned()).collect(),
        machines: machines.iter().map(|id| MachineId::new(*id)).collect(),
        purpose: purpose.to_owned(),
    };
    vec![
        channel(
            "web",
            &["acme/web", "acme/design-tokens", "acme/api"],
            &["this-mac", "gpu-box"],
            "The customer web app",
        ),
        channel(
            "api",
            &["acme/api", "acme/proto"],
            &["this-mac", "gpu-box"],
            "Public API and its clients",
        ),
        channel(
            "infra",
            &["acme/infra"],
            &["ci-runner"],
            "Terraform and CI",
        ),
        channel("scratch", &[], &["this-mac"], "No repos: notes and shells"),
    ]
}

fn transcript(title: &str, agent: Agent) -> Vec<Line> {
    let line = |from, text: &str| Line {
        from,
        text: text.to_owned(),
    };
    if agent == Agent::Shell {
        return vec![
            line(Speaker::System, "zsh 5.9 · ~/humata/scratch"),
            line(Speaker::You, "$ ls"),
            line(Speaker::Tool, "notes.md  todo.txt"),
        ];
    }
    vec![
        line(Speaker::System, &format!("{} started · {title}", agent.name())),
        line(Speaker::You, &format!("Please work on: {title}")),
        line(Speaker::Agent, "I'll read the relevant files first."),
        line(Speaker::Tool, "Read src/lib.rs (212 lines)"),
        line(Speaker::Tool, "Read src/client.rs (98 lines)"),
        line(
            Speaker::Agent,
            "The retry happens before the token is refreshed, so the second request reuses the expired token.",
        ),
        line(Speaker::Tool, "Edit src/client.rs (+12 −3)"),
        line(Speaker::Agent, "Done. The tests pass: 48 passed, 0 failed."),
    ]
}

fn session(
    id: &str,
    channel: &str,
    title: &str,
    agent: Agent,
    machine: &str,
    status: Status,
) -> Session {
    let turn = match status {
        Status::Working | Status::NeedsYou => Turn::Running,
        Status::ProbablyIdle => Turn::Uncertain,
        _ => Turn::Ended,
    };
    Session {
        id: SessionId::new(id),
        channel: ChannelId::new(channel),
        parent: None,
        fork: None,
        title: title.to_owned(),
        agent,
        machine: MachineId::new(machine),
        status,
        turn,
        pinned: false,
        vip: false,
        unread: 0,
        repo: String::new(),
        branch: format!("humata/{id}"),
        started: "2 h ago".to_owned(),
        active: "1 min ago".to_owned(),
        transcript: transcript(title, agent),
        shells: Vec::new(),
        archived: false,
        recovery: None,
    }
}

fn fork(mut child: Session, parent: &str, kind: ForkKind) -> Session {
    child.parent = Some(SessionId::new(parent));
    child.fork = Some(kind);
    child
}

fn sessions() -> Vec<Session> {
    let mut login = session(
        "s-login",
        "web",
        "Login v2",
        Agent::Claude,
        "this-mac",
        Status::Working,
    );
    login.pinned = true;
    login.vip = true;
    login.repo = "acme/web".to_owned();
    login.shells = vec!["zsh".to_owned(), "dev server".to_owned()];
    let mut theme = session(
        "s-theme",
        "web",
        "Dark mode tokens",
        Agent::Codex,
        "gpu-box",
        Status::Idle,
    );
    theme.repo = "acme/design-tokens".to_owned();
    let mut passkey = fork(
        session(
            "s-passkey",
            "web",
            "Passkey prompt",
            Agent::Claude,
            "this-mac",
            Status::NeedsYou,
        ),
        "s-login",
        ForkKind::Worktree,
    );
    passkey.unread = 3;
    passkey.repo = "acme/web".to_owned();
    let mut copy = fork(
        session(
            "s-copy",
            "web",
            "Passkey copy edits",
            Agent::Copilot,
            "this-mac",
            Status::ProbablyIdle,
        ),
        "s-passkey",
        ForkKind::Conversation,
    );
    copy.repo = "acme/web".to_owned();
    let mut email = fork(
        session(
            "s-email",
            "web",
            "Email code fallback",
            Agent::OpenCode,
            "gpu-box",
            Status::Exited { code: 1 },
        ),
        "s-login",
        ForkKind::Shared,
    );
    email.repo = "acme/web".to_owned();
    let mut auth = session(
        "s-auth",
        "api",
        "Token refresh retry",
        Agent::Claude,
        "gpu-box",
        Status::NeedsYou,
    );
    auth.pinned = true;
    auth.unread = 1;
    auth.repo = "acme/api".to_owned();
    let mut rate = session(
        "s-rate",
        "api",
        "Rate limit headers",
        Agent::Codex,
        "this-mac",
        Status::Working,
    );
    rate.repo = "acme/api".to_owned();
    let mut proto = fork(
        session(
            "s-proto",
            "api",
            "Proto field docs",
            Agent::Claude,
            "this-mac",
            Status::Idle,
        ),
        "s-rate",
        ForkKind::Worktree,
    );
    proto.repo = "acme/proto".to_owned();
    let mut terraform = session(
        "s-tf",
        "infra",
        "Terraform plan",
        Agent::Claude,
        "ci-runner",
        Status::Offline,
    );
    terraform.repo = "acme/infra".to_owned();
    let mut runner = session(
        "s-runner",
        "infra",
        "CI cache",
        Agent::Codex,
        "ci-runner",
        Status::Offline,
    );
    runner.repo = "acme/infra".to_owned();
    let mut backup = session(
        "s-backup",
        "infra",
        "Nightly backup",
        Agent::Claude,
        "ci-runner",
        Status::Offline,
    );
    backup.repo = "acme/infra".to_owned();
    let notes = session(
        "s-notes",
        "scratch",
        "Shell",
        Agent::Shell,
        "this-mac",
        Status::Idle,
    );
    let mut old = session(
        "s-old",
        "scratch",
        "Old experiment",
        Agent::Claude,
        "this-mac",
        Status::Idle,
    );
    old.archived = true;
    vec![
        login, passkey, copy, email, theme, auth, rate, proto, terraform, runner, backup, notes, old,
    ]
}

fn requests() -> Vec<Request> {
    let request = |id: &str, session: &str, channel: &str, kind, title: &str| Request {
        id: RequestId::new(id),
        session: SessionId::new(session),
        channel: ChannelId::new(channel),
        kind,
        title: title.to_owned(),
        reason: String::new(),
        preview: Preview::Text(String::new()),
        doing: String::new(),
        rule: String::new(),
        undoable: true,
        asked: "2 min ago".to_owned(),
        answer: None,
    };
    let mut command = request(
        "r-test",
        "s-auth",
        "api",
        RequestKind::Command,
        "Run the integration tests",
    );
    command.reason = "To check the retry against the staging token server.".to_owned();
    command.preview = Preview::Command("cargo test --test refresh -- --ignored".to_owned());
    command.doing = "Fixing the token refresh retry".to_owned();
    command.rule = "cargo test in acme/api".to_owned();
    let mut edit = request(
        "r-edit",
        "s-passkey",
        "web",
        RequestKind::FileEdit,
        "Edit the sign-in form",
    );
    edit.reason = "The passkey button replaces the password field.".to_owned();
    edit.preview = Preview::Diff(vec![
        DiffLine {
            kind: LineKind::Hunk,
            old: None,
            new: None,
            text: "@@ -12,3 +12,3 @@ export function SignIn()".to_owned(),
        },
        DiffLine {
            kind: LineKind::Removed,
            old: Some(13),
            new: None,
            text: "  <PasswordField />".to_owned(),
        },
        DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(13),
            text: "  <PasskeyButton />".to_owned(),
        },
    ]);
    edit.doing = "Adding the passkey prompt".to_owned();
    edit.rule = "edits under src/auth in acme/web".to_owned();
    edit.asked = "5 min ago".to_owned();
    let mut network = request(
        "r-net",
        "s-auth",
        "api",
        RequestKind::Network,
        "Reach auth.staging.acme.dev",
    );
    network.reason = "The refresh test talks to the staging token server.".to_owned();
    network.preview = Preview::Text("HTTPS to auth.staging.acme.dev:443".to_owned());
    network.doing = "Fixing the token refresh retry".to_owned();
    network.rule = "HTTPS to *.staging.acme.dev".to_owned();
    let mut question = request(
        "r-ask",
        "s-passkey",
        "web",
        RequestKind::Question,
        "Which browsers must passkeys support?",
    );
    question.reason = "Safari 17 lacks conditional mediation.".to_owned();
    question.preview =
        Preview::Text("Options: all evergreen browsers, or Safari 18 and up only.".to_owned());
    question.doing = "Adding the passkey prompt".to_owned();
    let mut merge = request(
        "r-merge",
        "s-theme",
        "web",
        RequestKind::Merge,
        "Merge #87 Dark mode tokens",
    );
    merge.reason = "All checks passed and the review is done.".to_owned();
    merge.preview = Preview::Text("acme/web #87 → main (squash)".to_owned());
    merge.doing = "Finishing dark mode".to_owned();
    merge.rule = "merges of green PRs in acme/web".to_owned();
    merge.undoable = false;
    let mut destroy = request(
        "r-rm",
        "s-rate",
        "api",
        RequestKind::Destroy,
        "Delete the old rate-limit tables",
    );
    destroy.reason = "They were replaced by the new counters.".to_owned();
    destroy.preview = Preview::Command("DROP TABLE rate_limit_v1, rate_limit_v1_audit;".to_owned());
    destroy.doing = "Rate limit headers".to_owned();
    destroy.undoable = false;
    let mut answered = request(
        "r-done",
        "s-login",
        "web",
        RequestKind::Command,
        "Install @simplewebauthn/browser",
    );
    answered.preview = Preview::Command("pnpm add @simplewebauthn/browser".to_owned());
    answered.answer = Some(Answer::Approved);
    answered.asked = "1 h ago".to_owned();
    vec![command, edit, network, question, merge, destroy, answered]
}

fn items() -> Vec<Item> {
    let item = |id: &str, scope: Scope, kind, title: &str, status, depends: &[&str]| Item {
        id: ItemId::new(id),
        scope,
        kind,
        title: title.to_owned(),
        status,
        depends_on: depends.iter().map(|id| ItemId::new(*id)).collect(),
        session: None,
        parent: None,
    };
    let web = || Scope::Channel(ChannelId::new("web"));
    let api = || Scope::Channel(ChannelId::new("api"));
    let mut items = vec![
        item(
            "W-1",
            web(),
            ItemKind::Milestone,
            "Login v2",
            ItemStatus::InProgress,
            &[],
        ),
        item(
            "W-2",
            web(),
            ItemKind::Task,
            "Passkey prompt",
            ItemStatus::InProgress,
            &[],
        ),
        item(
            "W-3",
            web(),
            ItemKind::Task,
            "Email code fallback",
            ItemStatus::Blocked,
            &["W-2"],
        ),
        item(
            "W-4",
            web(),
            ItemKind::Task,
            "Dark mode tokens",
            ItemStatus::Done,
            &[],
        ),
        item(
            "W-5",
            web(),
            ItemKind::Step,
            "Copy review",
            ItemStatus::Todo,
            &["W-2"],
        ),
        item(
            "A-1",
            api(),
            ItemKind::Task,
            "Token refresh retry",
            ItemStatus::InProgress,
            &[],
        ),
        item(
            "A-2",
            api(),
            ItemKind::Task,
            "Rate limit headers",
            ItemStatus::Todo,
            &["A-1"],
        ),
        item(
            "A-3",
            api(),
            ItemKind::Task,
            "v1 tables cleanup",
            ItemStatus::WontDo,
            &[],
        ),
        item(
            "G-1",
            Scope::Global,
            ItemKind::Milestone,
            "Ship login v2",
            ItemStatus::Todo,
            &["W-1", "A-1"],
        ),
    ];
    let link = |items: &mut Vec<Item>, id: &str, session: &str, parent: Option<&str>| {
        if let Some(item) = items.iter_mut().find(|item| item.id.as_str() == id) {
            item.session = Some(SessionId::new(session));
            item.parent = parent.map(ItemId::new);
        }
    };
    link(&mut items, "W-2", "s-passkey", Some("W-1"));
    link(&mut items, "W-3", "s-email", Some("W-1"));
    link(&mut items, "W-4", "s-theme", None);
    link(&mut items, "W-5", "s-copy", Some("W-2"));
    link(&mut items, "A-1", "s-auth", None);
    link(&mut items, "A-2", "s-rate", None);
    items
}

fn lines(spec: &[(LineKind, &str)], start: u32) -> Vec<DiffLine> {
    let mut old = start;
    let mut new = start;
    spec.iter()
        .map(|(kind, text)| {
            let (o, n) = match kind {
                LineKind::Hunk => (None, None),
                LineKind::Context => {
                    old += 1;
                    new += 1;
                    (Some(old), Some(new))
                }
                LineKind::Added => {
                    new += 1;
                    (None, Some(new))
                }
                LineKind::Removed => {
                    old += 1;
                    (Some(old), None)
                }
            };
            DiffLine {
                kind: *kind,
                old: o,
                new: n,
                text: (*text).to_owned(),
            }
        })
        .collect()
}

fn diffs() -> Vec<RepoDiff> {
    use LineKind::{Added, Context, Hunk, Removed};
    let client = FileDiff {
        path: "src/client.rs".to_owned(),
        added: 4,
        removed: 2,
        lines: lines(
            &[
                (Hunk, "@@ -40,7 +40,9 @@ impl Client {"),
                (Context, "    pub async fn send(&self, request: Request) -> Result<Response> {"),
                (Removed, "        let response = self.http.send(request.clone()).await?;"),
                (Removed, "        if response.status() == 401 { return self.retry(request).await; }"),
                (Added, "        let response = self.http.send(request.clone()).await?;"),
                (Added, "        if response.status() == 401 {"),
                (Added, "            self.tokens.refresh().await?;"),
                (Added, "            return self.http.send(request).await;"),
                (Context, "        }"),
                (Context, "        Ok(response)"),
            ],
            39,
        ),
    };
    let retry = FileDiff {
        path: "src/retry.rs".to_owned(),
        added: 2,
        removed: 0,
        lines: lines(
            &[
                (Hunk, "@@ -1,3 +1,5 @@"),
                (Added, "//! One retry, after refreshing the token."),
                (Added, ""),
                (Context, "use crate::client::Client;"),
            ],
            0,
        ),
    };
    let form = FileDiff {
        path: "src/auth/SignIn.tsx".to_owned(),
        added: 3,
        removed: 1,
        lines: lines(
            &[
                (Hunk, "@@ -10,6 +10,8 @@ export function SignIn() {"),
                (Context, "  return ("),
                (Context, "    <Form>"),
                (Removed, "      <PasswordField />"),
                (Added, "      <PasskeyButton onSuccess={finish} />"),
                (Added, "      <Divider>or</Divider>"),
                (Added, "      <EmailCodeLink />"),
                (Context, "    </Form>"),
            ],
            9,
        ),
    };
    let tokens = FileDiff {
        path: "tokens/color.json".to_owned(),
        added: 1,
        removed: 1,
        lines: lines(
            &[
                (Hunk, "@@ -3,3 +3,3 @@"),
                (Removed, "  \"surface\": \"#ffffff\","),
                (Added, "  \"surface\": { \"light\": \"#ffffff\", \"dark\": \"#1a1d21\" },"),
                (Context, "  \"text\": \"#1a1d21\""),
            ],
            2,
        ),
    };
    vec![
        RepoDiff {
            session: SessionId::new("s-auth"),
            repo: "acme/api".to_owned(),
            base: "main".to_owned(),
            files: vec![client, retry],
            comments: vec![Comment {
                path: "src/client.rs".to_owned(),
                line: 42,
                body: "Refresh once only; a second 401 should surface.".to_owned(),
                sent: true,
            }],
        },
        RepoDiff {
            session: SessionId::new("s-passkey"),
            repo: "acme/web".to_owned(),
            base: "main".to_owned(),
            files: vec![form.clone()],
            comments: Vec::new(),
        },
        RepoDiff {
            session: SessionId::new("s-login"),
            repo: "acme/web".to_owned(),
            base: "main".to_owned(),
            files: vec![form],
            comments: Vec::new(),
        },
        RepoDiff {
            session: SessionId::new("s-theme"),
            repo: "acme/design-tokens".to_owned(),
            base: "main".to_owned(),
            files: vec![tokens],
            comments: Vec::new(),
        },
    ]
}

fn files() -> Vec<FileEntry> {
    let file = |machine: &str, path: &str, size: &str, folder: bool| FileEntry {
        machine: MachineId::new(machine),
        path: path.to_owned(),
        size: size.to_owned(),
        folder,
    };
    vec![
        file("this-mac", "~/humata/web", "—", true),
        file("this-mac", "~/humata/web/README.md", "4 KB", false),
        file("this-mac", "~/humata/web/package.json", "2 KB", false),
        file("gpu-box", "~/humata/api", "—", true),
        file("gpu-box", "~/humata/api/Cargo.toml", "3 KB", false),
        file("gpu-box", "~/humata/web/build/app.tar.zst", "48 MB", false),
        file("ci-runner", "~/humata/infra/main.tf", "9 KB", false),
    ]
}

fn journal() -> Vec<JournalEntry> {
    let entry = |at: &str, session: Option<&str>, text: &str| JournalEntry {
        at: at.to_owned(),
        session: session.map(SessionId::new),
        text: text.to_owned(),
    };
    vec![
        entry("1 min ago", Some("s-auth"), "Asked to run the integration tests"),
        entry("4 min ago", Some("s-passkey"), "Forked “Login v2” → “Passkey prompt” (worktree)"),
        entry("12 min ago", Some("s-email"), "Exited with code 1"),
        entry("14 min ago", None, "ci-runner went offline"),
        entry("1 h ago", Some("s-theme"), "Opened #87 Dark mode tokens"),
    ]
}

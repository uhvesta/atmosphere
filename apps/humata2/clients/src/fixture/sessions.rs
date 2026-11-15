use crate::model::{
    Agent, ChannelId, ForkKind, Line, MachineId, Session, SessionId, Speaker, Status, Turn,
};

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
        line(
            Speaker::System,
            &format!("{} started · {title}", agent.name()),
        ),
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

pub(super) fn session(
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
        activity: String::new(),
        transcript: transcript(title, agent),
        shells: Vec::new(),
        terminal_agents: Default::default(),
        archived: false,
        recovery: None,
    }
}

fn fork(mut child: Session, parent: &str, kind: ForkKind) -> Session {
    child.parent = Some(SessionId::new(parent));
    child.fork = Some(kind);
    child
}

pub(super) fn sessions() -> Vec<Session> {
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
        login, passkey, copy, email, theme, auth, rate, proto, terraform, runner, backup, notes,
        old,
    ]
}

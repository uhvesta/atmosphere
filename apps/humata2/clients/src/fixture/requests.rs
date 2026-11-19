use crate::model::{
    Answer, AskedBy, Capability, ChannelId, RuleSetting, DiffLine, LineKind, Preview, Request, RequestId, RequestKind, SessionId,
};

pub(super) fn request(
    id: &str,
    session: &str,
    channel: &str,
    kind: RequestKind,
    title: &str,
) -> Request {
    Request {
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
        operation: String::new(),
        steps: Vec::new(),
        why_you: String::new(),
        asked_by: None,
        undo_note: None,
        choices: Vec::new(),
        on_screen: None,
        dismiss_only: false,
        facts: Vec::new(),
        affects: Vec::new(),
        undo_text: String::new(),
        working_on: String::new(),
        asked_why: None,
        rule_label: String::new(),
    }
}

fn asked_by(capability: &str, label: &str, setting: RuleSetting) -> Option<AskedBy> {
    Some(AskedBy {
        capability: Capability::new(capability),
        label: label.to_owned(),
        setting,
    })
}

fn steps(steps: &[&str]) -> Vec<String> {
    steps.iter().map(|step| (*step).to_owned()).collect()
}

pub(super) fn requests() -> Vec<Request> {
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
    command.operation = "terminal.run".to_owned();
    command.steps = steps(&[
        "Read src/refresh.rs",
        "Edited the retry to run once",
        "cargo test --test refresh -- --ignored",
    ]);
    command.why_you = "It runs a command outside its own clone's allowed list.".to_owned();
    command.asked_by = asked_by(
        "machine_script_run",
        "Run machine scripts",
        RuleSetting::Ask,
    );
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
    edit.operation = "files.write".to_owned();
    edit.steps = steps(&[
        "Read src/auth/SignIn.tsx",
        "Planned the passkey prompt",
        "Edit src/auth/SignIn.tsx",
    ]);
    edit.why_you = "The file is in another session's clone on this machine.".to_owned();
    edit.asked_by = asked_by(
        "files_read_other_session",
        "Read another session's files (same machine)",
        RuleSetting::Ask,
    );
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
    network.operation = "network.connect".to_owned();
    network.steps = steps(&[
        "Ran cargo test --test refresh",
        "The test needs the staging token server",
        "Connect to auth.staging.acme.dev:443",
    ]);
    network.why_you = "Connections through the host are not allowed without asking.".to_owned();
    network.asked_by = asked_by(
        "network_reach",
        "Open a network connection through the host",
        RuleSetting::Ask,
    );
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
    question.operation = "inbox.ask".to_owned();
    question.steps = steps(&[
        "Checked browser support tables",
        "Found Safari 17 lacks conditional mediation",
        "Ask which browsers must be supported",
    ]);
    question.why_you = "Only you can decide which browsers the product supports.".to_owned();
    question.asked_by = asked_by(
        "inbox_ask",
        "Ask you a question, or ask ahead for access",
        RuleSetting::Allow,
    );
    question.choices = vec![
        "All evergreen browsers".to_owned(),
        "Safari 18 and up only".to_owned(),
    ];
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
    merge.operation = "git.merge".to_owned();
    merge.steps = steps(&[
        "Checks passed on #87",
        "Review comments resolved",
        "Merge #87 into main (squash)",
    ]);
    merge.why_you = "Merging changes main, which only you may do.".to_owned();
    merge.asked_by = asked_by(
        "git_push",
        "Push its branch or open a pull request",
        RuleSetting::Ask,
    );
    merge.undo_note = Some("It can't be undone from Humata; reverting means a new commit on main.".to_owned());
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
    destroy.operation = "terminal.run".to_owned();
    destroy.steps = steps(&[
        "Moved reads to the new counters",
        "Checked nothing uses rate_limit_v1",
        "DROP TABLE rate_limit_v1, rate_limit_v1_audit;",
    ]);
    destroy.why_you = "It deletes data, and deleting always needs you.".to_owned();
    destroy.asked_by = asked_by(
        "own_session_manage",
        "Change its own session",
        RuleSetting::Ask,
    );
    destroy.undo_note = Some("The tables and their rows are gone; restoring needs a database backup.".to_owned());
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
    answered.operation = "terminal.run".to_owned();
    answered.steps = steps(&["Planned the sign-in change", "pnpm add @simplewebauthn/browser"]);
    answered.why_you = "Installing packages changes the dependency lock file.".to_owned();
    vec![command, edit, network, question, merge, destroy, answered]
}

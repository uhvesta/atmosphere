use crate::model::{
    Answer, ChannelId, DiffLine, LineKind, Preview, Request, RequestId, RequestKind, SessionId,
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
    }
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

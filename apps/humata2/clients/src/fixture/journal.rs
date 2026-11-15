use crate::model::{JournalEntry, SessionId};

pub(super) fn journal() -> Vec<JournalEntry> {
    let entry = |at: &str, session: Option<&str>, text: &str| JournalEntry {
        at: at.to_owned(),
        session: session.map(SessionId::new),
        text: text.to_owned(),
    };
    vec![
        entry(
            "1 min ago",
            Some("s-auth"),
            "Asked to run the integration tests",
        ),
        entry(
            "4 min ago",
            Some("s-passkey"),
            "Forked “Login v2” → “Passkey prompt” (worktree)",
        ),
        entry("12 min ago", Some("s-email"), "Exited with code 1"),
        entry("14 min ago", None, "ci-runner went offline"),
        entry("1 h ago", Some("s-theme"), "Opened #87 Dark mode tokens"),
    ]
}

//! Start fresh with a summary (PER-15, D143): a session whose resume failed
//! gets a new conversation, seeded with a plain digest of the old one. It
//! keeps its id, folder, branch and tab. The old conversation is recorded
//! and never deleted, and the new conversation id is only kept once the
//! agent started: if it doesn't, the session is left as it was.
//!
//! The digest needs no model: every user message (each capped), the last
//! few assistant text turns, no tool output, about 8 KiB in all.

use std::io::BufRead;
use std::path::Path;
use std::time::Duration;

use humata2_sessions::AgentError;
use humata2_store::{AgentRecord, AgentState, StoreError};
use humata2_svc_sessions_common::pb;
pub use humata2_svc_sessions_common::start_fresh::{
    Handover as CommonHandover, Source as CommonSource, retype_pending as common_retype_pending,
};
use serde_json::Value;

/// The most the digest holds.
pub const DIGEST_MAX: usize = 8 * 1024;
/// The most one user message keeps.
pub const USER_CAP: usize = 1000;
/// The most one assistant turn keeps.
pub const ASSISTANT_CAP: usize = 1200;
/// How many of the last assistant text turns the digest holds.
pub const ASSISTANTS_KEPT: usize = 3;
/// The most of the saved terminal output used when there is no transcript.
pub const SCROLLBACK_MAX: usize = 4 * 1024;
/// How much saved output is read for it: escape sequences take room.
pub const SCROLLBACK_READ: usize = 64 * 1024;
/// Between typing the seed and pressing return, so an agent that reads a
/// paste whole doesn't take the return as part of it.
pub const TYPE_PAUSE: Duration = Duration::from_millis(400); // D671 (Provisional)
/// BUG-T1: the seed waits, until the agent has run this long, for it to
/// read keys itself, then up to `SHOWN_WAIT` for it to show the seed.
pub const KEYS_WAIT: Duration = Duration::from_secs(5);
pub const SHOWN_WAIT: Duration = Duration::from_secs(5);
/// How long a seed waits for the input gate (D762): long enough for the
/// person to answer the agent's own question.
pub const SEED_READY_WAIT: Duration = Duration::from_secs(600);

/// How a fork's handover message starts: it tells the message from a start
/// fresh one, so a restart types it under the same tag (FK-6, D212).
pub const HANDOVER_LEAD: &str = "Humata: you are a fork of";

/// What the seed was made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The old conversation file.
    Transcript,
    /// The tail of the saved terminal output.
    Scrollback,
    /// Nothing: only the title.
    Nothing,
}

#[derive(Debug, thiserror::Error)]
pub enum StartFreshError {
    #[error(
        "session {session} can't start fresh: its agent is {state}, and only a session whose resume failed can"
    )]
    NotFailed {
        session: String,
        state: &'static str,
    },
    #[error("session {session}'s agent {agent} isn't known")]
    Agent {
        session: String,
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("couldn't {doing} for session {session}")]
    Store {
        session: String,
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("couldn't {doing} for session {session}: a background task failed ({message})")]
    Background {
        session: String,
        doing: &'static str,
        message: String,
    },
}

pub fn state_word(state: AgentState) -> &'static str {
    match state {
        AgentState::Launching => "still starting",
        AgentState::Running { .. } => "running",
        AgentState::Resuming => "being resumed",
        AgentState::Exited { .. } => "exited with its conversation intact",
        AgentState::Stopped => "stopped with its conversation intact",
        AgentState::ResumeFailed { .. } => "failed to resume",
    }
}

/// Whether the conversation file already holds the first message typed for
/// `conversation`, which names it. Blocking.
pub fn holds_seed(file: &Path, conversation: &str) -> bool {
    let Ok(bytes) = std::fs::read(file) else {
        return false;
    };
    [SeedKind::StartFresh, SeedKind::Handover]
        .iter()
        .any(|kind| {
            let tag = kind.tag(conversation);
            bytes
                .windows(tag.len())
                .any(|window| window == tag.as_bytes())
        })
}

/// What a kept first message is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedKind {
    StartFresh,
    /// A fork handed another agent's conversation (FK-6, D212).
    Handover,
}

impl SeedKind {
    pub fn of(seed: &str) -> Self {
        if seed.starts_with(HANDOVER_LEAD) {
            Self::Handover
        } else {
            Self::StartFresh
        }
    }

    /// What the first message for `conversation` starts with: it tells a
    /// conversation that had it from one that didn't.
    pub fn tag(self, conversation: &str) -> String {
        match self {
            Self::StartFresh => format!("[start fresh {conversation}]"),
            Self::Handover => format!("[handover {conversation}]"),
        }
    }
}

/// What a start fresh of `agent` could summarize, for the session shown to
/// the app: unspecified unless its resume failed. Blocking.
pub fn summary_source(agent: Option<&AgentRecord>) -> pb::session::SummarySource {
    use pb::session::SummarySource;
    let Some(agent) = agent.filter(|agent| matches!(agent.state, AgentState::ResumeFailed { .. }))
    else {
        return SummarySource::Unspecified;
    };
    match summarize(&agent.launch.conversation_file).0 {
        Source::Transcript => SummarySource::Transcript,
        Source::Scrollback => SummarySource::Scrollback,
        Source::Nothing => SummarySource::None,
    }
}

/// The digest of the old conversation and where it came from. Blocking.
pub fn summarize(file: &Path) -> (Source, Option<String>) {
    if let Ok(opened) = std::fs::File::open(file)
        && let Some(digest) = digest_of(std::io::BufReader::new(opened))
    {
        return (Source::Transcript, Some(digest));
    }
    (Source::Nothing, None)
}

/// `output` as plain text for a seed: escape sequences and control
/// characters dropped, the last `SCROLLBACK_MAX` bytes kept.
pub fn plain_tail(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output);
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: up to a final byte in @..~.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' || (next == '\u{1b}' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\t' => plain.push(c),
            c if c.is_control() => {}
            c => plain.push(c),
        }
    }
    let plain = plain.trim();
    if plain.is_empty() {
        return None;
    }
    Some(tail_chars(plain, SCROLLBACK_MAX))
}

/// What a fork handed its parent's conversation can be told, as the
/// preview describes it: what its first message would be made from.
/// `file` is the parent's conversation, None when its agent keeps none.
/// Blocking.
pub fn handover_source(file: Option<&Path>, _socket: &Path) -> Source {
    handover_digest(file, Some(u64::MAX)).0
}

/// The digest a fork's first message holds: of the parent's conversation
/// read up to `limit` bytes ([`Handover::read_to`]). Blocking.
fn handover_digest(file: Option<&Path>, limit: Option<u64>) -> (Source, Option<String>) {
    if let Some(file) = file
        && let Ok(opened) = std::fs::File::open(file)
        && let Some(limit) = limit
        && let Some(digest) =
            digest_of(std::io::BufReader::new(std::io::Read::take(opened, limit)))
    {
        return (Source::Transcript, Some(digest));
    }
    (Source::Nothing, None)
}

/// What the parent of a fork that runs another agent, or whose agent kept
/// no conversation file, hands it.
pub struct Handover<'a> {
    pub title: &'a str,
    pub agent: &'a str,
    /// The parent's conversation file, None when its agent keeps none.
    pub file: Option<&'a Path>,
    /// How much of `file` is read, in bytes: all of it (`u64::MAX`) from
    /// now, else up to the start of the turn the fork is before; None when
    /// that start can't be found, and the terminal's output stands in.
    pub read_to: Option<u64>,
    /// The parent's terminal, whose saved output stands in for a transcript.
    pub socket: &'a Path,
    /// Where the fork finds the conversation, from its own folder.
    pub handed: &'a Path,
}

/// What a fork handed its parent's conversation is told first (FK-6, D212):
/// a plain digest and where the transcript is. Blocking.
pub fn handover_seed(handover: &Handover<'_>) -> String {
    let (source, digest) = handover_digest(handover.file, handover.read_to);
    let mut seed = format!(
        "{HANDOVER_LEAD} the session \u{201c}{}\u{201d}, which ran {}.",
        handover.title, handover.agent
    );
    match (source, digest) {
        (Source::Transcript, Some(digest)) => {
            seed.push_str(
                " Its conversation was handed over to you: you know what was said and done there, but not that agent's exact internal state. Here is a plain summary:\n",
            );
            seed.push_str(&digest);
            seed.push_str(&format!(
                "\nThe full transcript is at {} in this folder (read it if you need more).",
                handover.handed.display()
            ));
        }
        (Source::Scrollback, Some(digest)) => {
            seed.push_str(
                " Its conversation couldn't be read, so this is the end of its terminal output, as plain text, not a transcript:\n",
            );
            seed.push_str(&digest);
        }
        _ => seed.push_str(" Nothing of its conversation could be handed over, only its title."),
    }
    seed.push_str("\nRun `git status` and check your work before continuing.");
    seed
}

/// What a start-fresh agent is told first.
pub fn seed_text(
    title: &str,
    source: Source,
    digest: Option<&str>,
    old_file: &Path,
    old_conversation: &str,
) -> String {
    let mut seed = String::from(
        "Humata: this session's earlier conversation couldn't be resumed, so you are starting fresh in the same folder and branch.",
    );
    match (source, digest) {
        (Source::Transcript, Some(digest)) => {
            seed.push_str(" Here is a plain summary of the earlier conversation:\n");
            seed.push_str(digest);
            seed.push_str(&format!(
                "\nThe full transcript is at {} (read it if you need more).",
                old_file.display()
            ));
        }
        (Source::Scrollback, Some(digest)) => {
            seed.push_str(&format!(
                " The earlier conversation file ({old_conversation}) was lost; this is the end of the terminal's output, as plain text, not a transcript:\n"
            ));
            seed.push_str(digest);
        }
        _ => {
            seed.push_str(&format!(
                " The earlier conversation ({old_conversation}) was lost and nothing of it could be summarized."
            ));
        }
    }
    seed.push_str(&format!("\nThe session's title is: {title}"));
    seed.push_str("\nRun `git status` and check your work before continuing.");
    seed
}

/// The digest of a conversation read from `reader`, a Claude JSONL file or
/// a Codex rollout: every user message, each capped, and the last few
/// assistant text turns, tool output left out, in about [`DIGEST_MAX`]
/// bytes. None if it holds neither.
pub fn digest_of(mut reader: impl BufRead) -> Option<String> {
    let mut users: Vec<String> = Vec::new();
    let mut assistants: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let Ok(entry) = serde_json::from_str::<Value>(text.trim()) else {
            continue;
        };
        if entry.get("isMeta").and_then(Value::as_bool) == Some(true)
            || entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some((role, said)) = said_in(&entry) else {
            continue;
        };
        let said = collapse(&said);
        if said.is_empty() {
            continue;
        }
        match role {
            "user" if !is_noise(&said) => users.push(cap_chars(&said, USER_CAP)),
            "assistant" => {
                assistants.push_back(cap_chars(&said, ASSISTANT_CAP));
                if assistants.len() > ASSISTANTS_KEPT {
                    assistants.pop_front();
                }
            }
            _ => {}
        }
    }
    if users.is_empty() && assistants.is_empty() {
        return None;
    }

    let mut tail = String::new();
    if !assistants.is_empty() {
        tail.push_str("Its last replies:\n");
        for said in &assistants {
            tail.push_str("- ");
            tail.push_str(said);
            tail.push('\n');
        }
    }
    // The newest user messages that fit, kept in the order they were said.
    let head = "What was asked, oldest first:\n";
    let note_room = 80;
    let room = DIGEST_MAX.saturating_sub(tail.len() + head.len() + note_room);
    let mut used = 0;
    let mut kept: Vec<&String> = Vec::new();
    for said in users.iter().rev() {
        let cost = said.len() + 3;
        if used + cost > room {
            break;
        }
        used += cost;
        kept.push(said);
    }
    kept.reverse();
    let left_out = users.len() - kept.len();
    let mut digest = String::new();
    if !kept.is_empty() {
        digest.push_str(head);
        if left_out > 0 {
            digest.push_str(&format!("({left_out} earlier messages left out)\n"));
        }
        for said in kept {
            digest.push_str("- ");
            digest.push_str(said);
            digest.push('\n');
        }
    }
    digest.push_str(&tail);
    Some(cap_bytes(digest.trim_end(), DIGEST_MAX))
}

fn said_in(entry: &Value) -> Option<(&str, String)> {
    if entry.get("type").and_then(Value::as_str) == Some("event_msg") {
        let payload = entry.get("payload")?;
        if payload.get("type").and_then(Value::as_str) != Some("item_completed") {
            return None;
        }
        let item = payload.get("item")?;
        let role = match item.get("type").and_then(Value::as_str)? {
            "UserMessage" => "user",
            "AgentMessage" => "assistant",
            _ => return None,
        };
        return Some((role, text_of(item.get("content")?)));
    }
    let role = entry.get("type").and_then(Value::as_str)?;
    let content = entry.get("message")?.get("content")?;
    Some((role, text_of(content)))
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| {
                block
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
            })
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn is_noise(said: &str) -> bool {
    ["<command-", "<local-command", "Caveat:", "Humata:", "[Humata]"]
        .iter()
        .any(|prefix| said.starts_with(prefix))
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cap_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn tail_chars(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

fn cap_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use humata2_store::AgentLaunch;

    use super::*;

    fn user(text: &str) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":text}}).to_string()
    }

    fn assistant(text: &str) -> String {
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":text},
            {"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}}
        ]}})
        .to_string()
    }

    fn tool_result(text: &str) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t","content":text}
        ]}})
        .to_string()
    }

    fn digest(lines: &[String]) -> Option<String> {
        digest_of(std::io::Cursor::new(lines.join("\n")))
    }

    #[test]
    fn keeps_user_messages_and_drops_tool_output() {
        let made = digest(&[
            user("fix the login redirect"),
            assistant("looking"),
            tool_result("SECRET-TOOL-OUTPUT"),
            user("and add a test"),
        ])
        .unwrap();
        assert!(made.contains("fix the login redirect"));
        assert!(made.contains("and add a test"));
        assert!(!made.contains("SECRET-TOOL-OUTPUT"));
        assert!(!made.contains("Bash"));
    }

    #[test]
    fn keeps_only_the_last_assistant_turns() {
        let mut lines = vec![user("go")];
        lines.extend((0..6).map(|n| assistant(&format!("reply-number-{n}"))));
        let made = digest(&lines).unwrap();
        assert!(!made.contains("reply-number-2"));
        assert!(made.contains("reply-number-3"));
        assert!(made.contains("reply-number-5"));
    }

    #[test]
    fn stays_under_the_cap_keeping_the_newest_messages() {
        let long = "x".repeat(5_000);
        let mut lines: Vec<String> = (0..60).map(|n| user(&format!("{n:03} {long}"))).collect();
        lines.push(user("the very last ask"));
        lines.extend((0..4).map(|_| assistant(&long)));
        let made = digest(&lines).unwrap();
        assert!(made.len() <= DIGEST_MAX);
        assert!(made.contains("the very last ask"));
        assert!(!made.contains("000 x"));
    }

    #[test]
    fn caps_each_user_message() {
        let made = digest(&[user(&"y".repeat(USER_CAP * 3))]).unwrap();
        assert!(made.matches('y').count() <= USER_CAP);
    }

    #[test]
    fn skips_lines_that_are_not_json_and_agent_notes() {
        let made = digest(&[
            "not json".to_owned(),
            user("<command-name>/clear</command-name>"),
            user("real ask"),
        ])
        .unwrap();
        assert!(made.contains("real ask"));
        assert!(!made.contains("command-name"));
    }

    #[test]
    fn an_empty_conversation_has_no_digest() {
        assert_eq!(digest(&[tool_result("only output")]), None);
        assert_eq!(digest(&[]), None);
    }

    fn codex(item: &str, block: &str, text: &str) -> String {
        serde_json::json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"t1",
            "item":{"type":item,"content":[{"type":block,"text":text}]}}})
        .to_string()
    }

    #[test]
    fn a_codex_rollout_keeps_its_prompts_and_replies_and_not_what_codex_injected() {
        let injected = serde_json::json!({"type":"response_item","payload":{"type":"message",
            "role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /work"}]}})
        .to_string();
        let made = digest(&[
            injected,
            codex("UserMessage", "text", "Fix the login redirect"),
            codex(
                "AgentMessage",
                "Text",
                "The redirect goes back to the page you came from.",
            ),
        ])
        .unwrap();
        assert!(made.contains("Fix the login redirect"), "{made}");
        assert!(made.contains("The redirect goes back"), "{made}");
        assert!(!made.contains("AGENTS.md"), "{made}");
    }

    #[test]
    fn plain_tail_drops_escape_sequences() {
        let plain = plain_tail(b"\x1b[31mred\x1b[0m text\x1b]0;title\x07 done\r\n").unwrap();
        assert_eq!(plain, "red text done");
        assert_eq!(plain_tail(b"\x1b[2J"), None);
    }

    #[test]
    fn cutting_respects_character_boundaries() {
        assert_eq!(cap_bytes("aé", 2), "a");
        assert_eq!(cap_chars("ééé", 2), "éé…");
        assert_eq!(tail_chars("éa", 2), "a");
    }

    #[test]
    fn a_handover_summarizes_the_parents_conversation_and_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.jsonl");
        std::fs::write(
            &file,
            [user("add retries"), assistant("added them")].join("\n") + "\n",
        )
        .unwrap();
        let handed = Path::new(".humata/handover-se_1.jsonl");
        let seed = handover_seed(&Handover {
            title: "Fix login",
            agent: "claude",
            file: Some(&file),
            read_to: Some(u64::MAX),
            socket: &dir.path().join("none.sock"),
            handed,
        });
        assert_eq!(SeedKind::of(&seed), SeedKind::Handover);
        assert!(
            seed.contains("add retries") && seed.contains("added them"),
            "{seed}"
        );
        assert!(seed.contains(".humata/handover-se_1.jsonl"), "{seed}");
        assert_eq!(
            handover_source(Some(&file), &dir.path().join("none.sock")),
            Source::Transcript
        );
    }

    #[test]
    fn a_handover_before_a_turn_leaves_out_what_came_from_that_turn_on() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.jsonl");
        let first = user("first ask");
        std::fs::write(&file, [first.clone(), user("second ask")].join("\n") + "\n").unwrap();
        let second_starts = u64::try_from(first.len() + 1).unwrap();
        let seed = handover_seed(&Handover {
            title: "Fix login",
            agent: "claude",
            file: Some(&file),
            read_to: Some(second_starts),
            socket: &dir.path().join("none.sock"),
            handed: Path::new("h.jsonl"),
        });
        assert!(seed.contains("first ask"), "{seed}");
        assert!(!seed.contains("second ask"), "{seed}");
    }

    #[test]
    fn a_handover_with_nothing_to_read_says_only_the_title_is_known() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("none.sock");
        assert_eq!(handover_source(None, &socket), Source::Nothing);
        let seed = handover_seed(&Handover {
            title: "Fix login",
            agent: "opencode",
            file: None,
            read_to: Some(u64::MAX),
            socket: &socket,
            handed: Path::new("h.jsonl"),
        });
        assert_eq!(SeedKind::of(&seed), SeedKind::Handover);
        assert!(
            !seed.contains("h.jsonl"),
            "no file is named when none was handed over: {seed}"
        );
    }

    #[test]
    fn a_conversation_that_had_its_handover_is_told_from_one_that_did_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.jsonl");
        std::fs::write(
            &file,
            format!("{} {HANDOVER_LEAD} x\n", SeedKind::Handover.tag("c-new")),
        )
        .unwrap();
        assert!(holds_seed(&file, "c-new"));
        assert!(!holds_seed(&file, "c-other"));
    }

    #[test]
    fn a_conversation_that_had_its_first_message_is_told_from_one_that_did_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.jsonl");
        std::fs::write(
            &file,
            user(&format!(
                "{} pick up the build",
                SeedKind::StartFresh.tag("c-new")
            )),
        )
        .unwrap();
        assert!(holds_seed(&file, "c-new"));
        assert!(!holds_seed(&file, "c-other"));
        assert!(!holds_seed(&dir.path().join("missing.jsonl"), "c-new"));
    }

    fn failed_agent(file: &Path, socket: &Path) -> AgentRecord {
        AgentRecord {
            session: "se_1".to_owned(),
            tab: "t".to_owned(),
            launch: AgentLaunch {
                agent: "claude".to_owned(),
                argv: Vec::new(),
                resume_argv: Vec::new(),
                conversation_id: "c1".to_owned(),
                conversation_file: file.to_owned(),
                holder_socket: socket.to_owned(),
                folder: std::env::temp_dir(),
                variable_names: Vec::new(),
                cols: 80,
                rows: 24,
                cli_version: None,
            },
            state: AgentState::ResumeFailed {
                reason: humata2_store::ResumeFailure::AgentExited,
                exit_code: None,
            },
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 0,
        }
    }

    #[test]
    fn the_summary_source_follows_what_is_left_of_the_conversation() {
        use pb::session::SummarySource;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.jsonl");
        let socket = dir.path().join("h.sock");
        let mut agent = failed_agent(&file, &socket);

        assert_eq!(summary_source(Some(&agent)), SummarySource::None);
        std::fs::write(&file, user("fix the login redirect")).unwrap();
        assert_eq!(summary_source(Some(&agent)), SummarySource::Transcript);
        agent.state = AgentState::Stopped;
        assert_eq!(summary_source(Some(&agent)), SummarySource::Unspecified);
        assert_eq!(summary_source(None), SummarySource::Unspecified);
    }
}

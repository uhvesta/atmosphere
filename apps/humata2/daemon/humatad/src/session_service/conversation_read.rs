//! `SessionService.ReadConversation`: a session's conversation on this
//! machine, read from the file its agent's CLI keeps, in pages.
//!
//! Claude's JSONL, Codex's rollout and Copilot's session-state events are
//! read the way the host's mirror reads them ([`humata2_mirror::read_from`]
//! tells the format by the line's shape), so `humata conversation` and
//! `humata session read` show the same entries. Text is scrubbed of this
//! machine's host variable values before it leaves.

use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{MirrorError, OwnTools, Role, Scrubber, read_from};
use humata2_sessions::{AgentError, Agents, ConversationFormat};
use humata2_store::{Store, StoreError};

/// How many entries a page holds when the request names no size.
const DEFAULT_PAGE: usize = 50;
/// The most entries a page holds.
const MAX_PAGE: usize = 500;

#[derive(Debug, thiserror::Error)]
pub(super) enum ReadError {
    #[error("couldn't read session {session}'s agent from the store; check that the state folder is readable, then try again")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("session {session} has no agent yet, so there is no conversation to read; start the session's agent first")]
    NoAgent { session: String },
    #[error("session {session} runs an agent named {agent}, which isn't in Settings > Agents; add it there or start the session again with a known agent")]
    UnknownAgent {
        session: String,
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("the agent {agent} keeps no conversation file Humata can read, so session {session}'s conversation can't be shown; read its terminal instead")]
    NoFormat { session: String, agent: String },
    #[error("session {session}'s agent hasn't written its conversation yet ({path}); send it a first message, wait for its turn to end, then read again")]
    NotWritten { session: String, path: PathBuf },
    #[error("couldn't read the conversation file {path}; check that this user can read it, then try again")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: MirrorError,
    },
    #[error("the page token {token:?} isn't one this call gave; start again from the first page, with no page token")]
    BadToken { token: String },
}

/// The page of `session`'s conversation the request asks for.
///
/// Binds a Codex session to the rollout its CLI wrote first (D670), so a
/// session whose file isn't known yet is read once the CLI has written it.
/// Blocking.
pub(super) fn read(
    store: &Store,
    agents: &Agents,
    home: &Path,
    session: &str,
    page: Option<&pb::PageRequest>,
    scrubber: &Scrubber,
) -> Result<pb::ReadConversationResponse, ReadError> {
    let stored = |source| ReadError::Store {
        session: session.to_owned(),
        source,
    };
    agents
        .learn_conversation(store, home, session)
        .map_err(stored)?;
    let Some(agent) = store.agent(session).map_err(stored)? else {
        return Err(ReadError::NoAgent {
            session: session.to_owned(),
        });
    };
    let kind = agents
        .get(&agent.launch.agent)
        .map_err(|source| ReadError::UnknownAgent {
            session: session.to_owned(),
            agent: agent.launch.agent.clone(),
            source,
        })?;
    if kind.conversation_format == ConversationFormat::None {
        return Err(ReadError::NoFormat {
            session: session.to_owned(),
            agent: agent.launch.agent,
        });
    }
    read_file(session, &agent.launch.conversation_file, page, scrubber)
}

/// The page of the conversation file `file`; [`ReadError::NotWritten`] when
/// it isn't there or is empty.
fn read_file(
    session: &str,
    file: &Path,
    page: Option<&pb::PageRequest>,
    scrubber: &Scrubber,
) -> Result<pb::ReadConversationResponse, ReadError> {
    let written = !file.as_os_str().is_empty()
        && std::fs::metadata(file).is_ok_and(|meta| meta.is_file() && meta.len() > 0);
    if !written {
        return Err(ReadError::NotWritten {
            session: session.to_owned(),
            path: file.to_owned(),
        });
    }
    let (start, size) = window(page)?;
    let read = read_from(file, 0, &mut OwnTools::default()).map_err(|source| {
        ReadError::Unreadable {
            path: file.to_owned(),
            source,
        }
    })?;
    let end = start.saturating_add(size).min(read.entries.len());
    let entries = read
        .entries
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .map(|entry| pb::TranscriptEntry {
            role: match entry.role {
                Role::User => pb::transcript_entry::Role::User,
                Role::Agent => pb::transcript_entry::Role::Agent,
                Role::Tool => pb::transcript_entry::Role::Tool,
            } as i32,
            text: scrubber.scrub(&entry.text),
            at: entry
                .at
                .as_deref()
                .and_then(crate::history_service::timestamp),
        })
        .collect();
    let next_page_token = if end < read.entries.len() {
        end.to_string()
    } else {
        String::new()
    };
    Ok(pb::ReadConversationResponse {
        entries,
        page: Some(pb::PageResponse { next_page_token }),
    })
}

/// Where the page starts and how many entries it holds.
fn window(page: Option<&pb::PageRequest>) -> Result<(usize, usize), ReadError> {
    let Some(page) = page else {
        return Ok((0, DEFAULT_PAGE));
    };
    let start = if page.page_token.is_empty() {
        0
    } else {
        page.page_token
            .parse::<usize>()
            .map_err(|_| ReadError::BadToken {
                token: page.page_token.clone(),
            })?
    };
    let size = match usize::try_from(page.page_size).unwrap_or(MAX_PAGE) {
        0 => DEFAULT_PAGE,
        size => size.min(MAX_PAGE),
    };
    Ok((start, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX: &str = concat!(
        r#"{"timestamp":"2026-11-16T21:48:57.265Z","type":"session_meta","payload":{"id":"x","cwd":"/w"}}"#, "\n",
        r#"{"timestamp":"2026-11-16T21:48:57.415Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"Fix the login redirect"}]}}}"#, "\n",
        r#"{"timestamp":"2026-11-16T21:48:57.459Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"Done, it goes back now."}]}}}"#, "\n",
        r#"{"timestamp":"2026-11-16T21:48:57.824Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"Now add a test"}]}}}"#, "\n",
    );

    const COPILOT: &str = concat!(
        r#"{"type":"session.start","data":{"sessionId":"x"},"id":"1","timestamp":"2026-11-17T02:42:43.111Z","parentId":null}"#, "\n",
        r#"{"type":"user.message","data":{"content":"Fix the failing payments test"},"id":"2","timestamp":"2026-11-17T02:42:44.000Z","parentId":"1"}"#, "\n",
        r#"{"type":"assistant.message","data":{"content":"The test needed a fixed clock.","toolRequests":[]},"id":"3","timestamp":"2026-11-17T02:42:45.000Z","parentId":"2"}"#, "\n",
    );

    fn page_of(text: &str, page: Option<&pb::PageRequest>) -> Result<pb::ReadConversationResponse, ReadError> {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let file = dir.path().join("conversation.jsonl");
        std::fs::write(&file, text).expect("written");
        read_file("se_1", &file, page, &Scrubber::new([]))
    }

    fn texts(response: &pb::ReadConversationResponse) -> Vec<(i32, &str)> {
        response
            .entries
            .iter()
            .map(|entry| (entry.role, entry.text.as_str()))
            .collect()
    }

    #[test]
    fn a_codex_rollout_gives_its_prompts_and_replies_in_order() {
        let read = page_of(CODEX, None).expect("read");
        let user = pb::transcript_entry::Role::User as i32;
        let agent = pb::transcript_entry::Role::Agent as i32;
        assert_eq!(
            texts(&read),
            vec![
                (user, "Fix the login redirect"),
                (agent, "Done, it goes back now."),
                (user, "Now add a test"),
            ]
        );
        assert!(read.entries.iter().all(|entry| entry.at.is_some()));
        assert!(read.page.is_some_and(|page| page.next_page_token.is_empty()));
    }

    #[test]
    fn a_copilot_session_gives_its_prompts_and_replies_in_order() {
        let read = page_of(COPILOT, None).expect("read");
        let user = pb::transcript_entry::Role::User as i32;
        let agent = pb::transcript_entry::Role::Agent as i32;
        assert_eq!(
            texts(&read),
            vec![
                (user, "Fix the failing payments test"),
                (agent, "The test needed a fixed clock."),
            ]
        );
    }

    #[test]
    fn pages_follow_each_other_without_gaps() {
        let ask = |token: &str| pb::PageRequest {
            page_size: 2,
            page_token: token.to_owned(),
        };
        let first = page_of(CODEX, Some(&ask(""))).expect("first");
        assert_eq!(first.entries.len(), 2);
        let token = first.page.expect("page").next_page_token;
        assert!(!token.is_empty());
        let second = page_of(CODEX, Some(&ask(&token))).expect("second");
        assert_eq!(texts(&second), vec![(pb::transcript_entry::Role::User as i32, "Now add a test")]);
        assert!(second.page.is_some_and(|page| page.next_page_token.is_empty()));
    }

    #[test]
    fn a_token_that_isnt_a_position_is_refused() {
        let ask = pb::PageRequest {
            page_size: 0,
            page_token: "later".to_owned(),
        };
        assert!(matches!(
            page_of(CODEX, Some(&ask)),
            Err(ReadError::BadToken { .. })
        ));
    }

    #[test]
    fn a_file_the_agent_hasnt_written_is_not_written_yet() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let missing = dir.path().join("rollout.jsonl");
        assert!(matches!(
            read_file("se_1", &missing, None, &Scrubber::new([])),
            Err(ReadError::NotWritten { .. })
        ));
        assert!(matches!(
            read_file("se_1", Path::new(""), None, &Scrubber::new([])),
            Err(ReadError::NotWritten { .. })
        ));
        std::fs::write(&missing, "").expect("written");
        assert!(matches!(
            read_file("se_1", &missing, None, &Scrubber::new([])),
            Err(ReadError::NotWritten { .. })
        ));
    }

    #[test]
    fn host_variable_values_are_scrubbed_from_the_text() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let file = dir.path().join("c.jsonl");
        std::fs::write(&file, COPILOT).expect("written");
        let scrubber = Scrubber::new([("CLOCK".to_owned(), "fixed clock".to_owned())]);
        let read = read_file("se_1", &file, None, &scrubber).expect("read");
        assert!(read.entries.iter().all(|entry| !entry.text.contains("fixed clock")));
    }
}

//! Code review, as the daemon keeps it: threads on lines of a session's
//! files, which follow those lines as the file changes; reviews sent to the
//! agent as one message; and the agent's replies, read back from the
//! session's history, where it answers each thread with Humata's
//! `reply_to_thread` tool or under a `[thread N]` line.

use std::collections::HashMap;

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::error::{DaemonError, StoreError};
use crate::store::{Comment, NewThread, Review, Thread};

/// Lines above a thread kept to find where it went if its own lines change.
const BEFORE: usize = 3;

impl Daemon {
    /// Sends `request` to `host` and returns its answer, or what it refused
    /// with.
    async fn ask(
        &self,
        host: &str,
        request: pb::request::Kind,
    ) -> Result<pb::response::Kind, DaemonError> {
        match self.forward(host, request).await? {
            Reply::Response(pb::response::Kind::Error(error)) => Err(DaemonError::Refused {
                code: error.code(),
                message: error.message,
            }),
            Reply::Response(response) => Ok(response),
            Reply::Stream { response, .. } => Ok(response),
        }
    }

    /// The session's changes from its host, with the files marked viewed
    /// that still have the contents they were viewed with.
    pub(super) async fn read_changes(
        &self,
        host: &str,
        read: pb::ReadChanges,
    ) -> Result<Reply, DaemonError> {
        let session = read.session_id.clone();
        let reply = self
            .forward(host, pb::request::Kind::ReadChanges(read))
            .await?;
        let Reply::Response(pb::response::Kind::Changes(mut changes)) = reply else {
            return Ok(reply);
        };
        let viewed: HashMap<(String, String), String> = self
            .store
            .viewed(&session)?
            .into_iter()
            .map(|(repository, path, blob)| ((repository, path), blob))
            .collect();
        for repository in &mut changes.repositories {
            for file in &mut repository.files {
                let key = (repository.name.clone(), file.path.clone());
                file.viewed = !file.blob.is_empty() && viewed.get(&key) == Some(&file.blob);
            }
        }
        Ok(Reply::Response(pb::response::Kind::Changes(changes)))
    }

    pub(super) fn mark_viewed(&self, mark: &pb::MarkViewed) -> Result<(), DaemonError> {
        let file = mark.file.as_ref().ok_or(DaemonError::NoFile)?;
        let blob = Some(mark.blob.as_str()).filter(|_| mark.viewed && !mark.blob.is_empty());
        self.store
            .set_viewed(&mark.session_id, &file.repository, &file.path, blob)?;
        Ok(())
    }

    /// The lines of a file as it is in `session`'s copy now.
    async fn file_lines(
        &self,
        host: &str,
        session: &str,
        file: &pb::FileRef,
    ) -> Result<Vec<String>, DaemonError> {
        let request = pb::request::Kind::ReadChanges(pb::ReadChanges {
            session_id: session.to_owned(),
            range: None,
            context: None,
            file: Some(file.clone()),
            whole_file: true,
        });
        match self.ask(host, request).await? {
            pb::response::Kind::Changes(changes) => Ok(new_lines(&changes)),
            _ => Ok(Vec::new()),
        }
    }

    pub(super) async fn create_thread(
        &self,
        create: pb::CreateThread,
    ) -> Result<pb::ReviewThread, DaemonError> {
        let file = create.file.ok_or(DaemonError::NoFile)?;
        if create.text.trim().is_empty() {
            return Err(DaemonError::NoComment);
        }
        let host = self.host_of(&create.session_id).await?;
        let lines = self.file_lines(&host, &create.session_id, &file).await?;
        let (start, end) = (create.start_line, create.end_line);
        let inside = start >= 1 && start <= end && (end as usize) <= lines.len();
        if !inside {
            return Err(DaemonError::LinesOutside {
                path: file.path,
                start,
                end,
                lines: lines.len(),
            });
        }
        let first = start as usize - 1;
        let new = NewThread {
            session: create.session_id,
            repository: file.repository,
            path: file.path,
            start_line: start,
            end_line: end,
            code: lines[first..end as usize].to_vec(),
            before: lines[first.saturating_sub(BEFORE)..first].to_vec(),
            quote: create.quote,
        };
        let thread = self.store.add_thread(&new, create.text.trim())?;
        Ok(wire_thread(&thread, &[]))
    }

    pub(super) async fn reply_to_thread(
        &self,
        reply: &pb::ReplyToThread,
    ) -> Result<pb::ReviewThread, DaemonError> {
        if reply.text.trim().is_empty() {
            return Err(DaemonError::NoComment);
        }
        let id = thread_id(&reply.thread_id)?;
        self.store.add_comment(id, reply.text.trim())?;
        self.shown_thread(id).await
    }

    pub(super) async fn resolve_thread(
        &self,
        resolve: &pb::ResolveThread,
    ) -> Result<pb::ReviewThread, DaemonError> {
        let id = thread_id(&resolve.thread_id)?;
        self.store.resolve_thread(id, resolve.resolved)?;
        self.shown_thread(id).await
    }

    pub(super) fn delete_comment(&self, delete: &pb::DeleteComment) -> Result<(), DaemonError> {
        let id = delete
            .comment_id
            .parse()
            .map_err(|_| StoreError::UnknownComment {
                id: delete.comment_id.clone(),
            })?;
        Ok(self.store.delete_comment(id)?)
    }

    async fn shown_thread(&self, id: i64) -> Result<pb::ReviewThread, DaemonError> {
        let session = self.store.thread(id)?.session;
        self.threads(&session)
            .await?
            .into_iter()
            .find(|thread| thread.id == id.to_string())
            .ok_or_else(|| StoreError::UnknownThread { id: id.to_string() }.into())
    }

    /// `session`'s threads where their lines are now, with the agent's
    /// replies. A host that can't be reached leaves them where they were
    /// last found, with the replies last read.
    pub(super) async fn threads(
        &self,
        session: &str,
    ) -> Result<Vec<pb::ReviewThread>, DaemonError> {
        let mut threads = self.store.threads(session)?;
        if threads.is_empty() {
            return Ok(Vec::new());
        }
        let host = self.host_of(session).await?;
        let mut files: HashMap<(String, String), Option<Vec<String>>> = HashMap::new();
        for thread in threads.iter_mut().filter(|thread| !thread.resolved) {
            let key = (thread.repository.clone(), thread.path.clone());
            if !files.contains_key(&key) {
                let file = pb::FileRef {
                    repository: key.0.clone(),
                    path: key.1.clone(),
                };
                let lines = self.file_lines(&host, session, &file).await.ok();
                files.insert(key.clone(), lines);
            }
            let Some(Some(lines)) = files.get(&key) else {
                continue;
            };
            let found = locate(thread, lines);
            if found != (thread.start_line, thread.end_line, thread.outdated) {
                (thread.start_line, thread.end_line, thread.outdated) = found;
                self.store
                    .move_thread(thread.id, found.0, found.1, found.2)?;
            }
        }
        let reviews = self.store.reviews(session)?;
        let replies = if reviews.is_empty() {
            Vec::new()
        } else {
            let read = pb::ReadHistory {
                session_id: session.to_owned(),
            };
            match self.history(&host, read).await {
                Ok(Reply::Response(pb::response::Kind::History(history))) => {
                    agent_replies(&history.events, &reviews)
                }
                _ => Vec::new(),
            }
        };
        Ok(threads
            .iter()
            .map(|thread| wire_thread(thread, &replies))
            .collect())
    }

    /// Sends every comment not sent yet, on threads not resolved, to the
    /// agent as one message.
    pub(super) async fn send_review(&self, session: &str) -> Result<u32, DaemonError> {
        let shown = self.threads(session).await?;
        let stored = self.store.threads(session)?;
        let mut pending: Vec<(&Thread, Vec<&Comment>)> = Vec::new();
        for thread in stored.iter().filter(|thread| !thread.resolved) {
            let unsent: Vec<&Comment> = thread
                .comments
                .iter()
                .filter(|comment| comment.review.is_none())
                .collect();
            if !unsent.is_empty() {
                pending.push((thread, unsent));
            }
        }
        if pending.is_empty() {
            return Err(DaemonError::NothingToReview);
        }
        let at: HashMap<&str, (u32, u32)> = shown
            .iter()
            .map(|thread| (thread.id.as_str(), (thread.start_line, thread.end_line)))
            .collect();
        let message = compose(&pending, |thread| {
            at.get(thread.id.to_string().as_str())
                .copied()
                .unwrap_or((thread.start_line, thread.end_line))
        });
        let host = self.host_of(session).await?;
        let request = pb::request::Kind::SendMessage(pb::SendMessage {
            session_id: session.to_owned(),
            text: message.clone(),
            mode: pb::SendMode::Enqueue.into(),
        });
        self.ask(&host, request).await?;
        let comments: Vec<i64> = pending
            .iter()
            .flat_map(|(_, comments)| comments.iter().map(|comment| comment.id))
            .collect();
        self.store.record_review(session, &message, &comments)?;
        Ok(u32::try_from(comments.len()).unwrap_or(u32::MAX))
    }
}

fn thread_id(id: &str) -> Result<i64, DaemonError> {
    id.parse()
        .map_err(|_| StoreError::UnknownThread { id: id.to_owned() }.into())
}

/// The lines of the one file a whole-file read returned, as they are at the
/// end of its range.
fn new_lines(changes: &pb::Changes) -> Vec<String> {
    changes
        .repositories
        .iter()
        .flat_map(|repository| &repository.files)
        .take(1)
        .flat_map(|file| &file.hunks)
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.new != 0)
        .map(|line| line.text.clone())
        .collect()
}

/// Where `thread`'s lines are in `lines`: the copy of them nearest to where
/// they were last; else just after the lines that were above them, as
/// outdated; else where they were, as far as the file goes, as outdated.
fn locate(thread: &Thread, lines: &[String]) -> (u32, u32, bool) {
    let last = thread.start_line.saturating_sub(1) as usize;
    let nearest = |block: &[String], wanted: usize| -> Option<usize> {
        if block.is_empty() || block.len() > lines.len() {
            return None;
        }
        (0..=lines.len() - block.len())
            .filter(|&at| lines[at..at + block.len()] == *block)
            .min_by_key(|&at| at.abs_diff(wanted))
    };
    let span = thread.code.len().max(1);
    let number = |index: usize| u32::try_from(index + 1).unwrap_or(u32::MAX);
    if let Some(at) = nearest(&thread.code, last) {
        return (number(at), number(at + span - 1), false);
    }
    if let Some(at) = nearest(&thread.before, last.saturating_sub(thread.before.len())) {
        let start = (at + thread.before.len()).min(lines.len().saturating_sub(1));
        let end = (start + span - 1)
            .min(lines.len().saturating_sub(1))
            .max(start);
        return (number(start), number(end), true);
    }
    let bottom = lines.len().saturating_sub(1);
    let start = last.min(bottom);
    let end = (thread.end_line.saturating_sub(1) as usize)
        .min(bottom)
        .max(start);
    (number(start), number(end), true)
}

/// One message asking the agent to answer each thread under its tag.
fn compose(pending: &[(&Thread, Vec<&Comment>)], at: impl Fn(&Thread) -> (u32, u32)) -> String {
    let mut text = String::from(
        "Here is my review of your changes, as comments on lines of the code. \
         Make the changes they ask for, and answer each thread on its own. \
         If you have Humata's reply_to_thread tool, answer with it; \
         otherwise start the answer with the thread's tag on a line of its own, \
         like this:\n\n[thread 1]\nWhat you did about it, or why not.\n",
    );
    for (thread, comments) in pending {
        let (start, end) = at(thread);
        let lines = if start == end {
            format!("line {start}")
        } else {
            format!("lines {start}-{end}")
        };
        text.push_str(&format!(
            "\n[thread {}] {}/{}, {lines}:\n",
            thread.id, thread.repository, thread.path
        ));
        text.push_str("```\n");
        for line in &thread.code {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str("```\n");
        if !thread.quote.is_empty() {
            text.push_str(&format!("On the text \"{}\":\n", thread.quote));
        }
        for comment in comments {
            text.push_str(comment.text.trim());
            text.push('\n');
        }
    }
    text
}

/// One of the agent's answers to a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentReply {
    review: i64,
    thread: i64,
    text: String,
    /// The turn it answered in, from 1; its changes are the fix.
    turn: u32,
    time_unix_ms: i64,
}

/// The agent's answers to `reviews`, from the session's history: each
/// review's message as you sent it, then what the agent said in that turn
/// and its `reply_to_thread` calls that succeeded.
fn agent_replies(events: &[pb::SessionEvent], reviews: &[Review]) -> Vec<AgentReply> {
    use pb::session_event::Kind;
    struct Answering<'a> {
        review: &'a Review,
        turn: Option<u32>,
        said: String,
        calls: Vec<Call>,
        time_unix_ms: i64,
    }
    /// A reply by tool, which counts once the call succeeds.
    struct Call {
        id: String,
        thread: i64,
        text: String,
        time_unix_ms: i64,
        succeeded: bool,
    }
    let finish = |answering: Answering<'_>, replies: &mut Vec<AgentReply>| {
        let Some(turn) = answering.turn else { return };
        let said_at = answering.time_unix_ms;
        let said = sections(&answering.said)
            .into_iter()
            .map(|(thread, text)| (thread, text, said_at));
        let called = answering
            .calls
            .into_iter()
            .filter(|call| call.succeeded)
            .map(|call| (call.thread, call.text, call.time_unix_ms));
        for (thread, text, time_unix_ms) in said.chain(called) {
            if answering.review.threads.contains(&thread) {
                replies.push(AgentReply {
                    review: answering.review.id,
                    thread,
                    text,
                    turn,
                    time_unix_ms,
                });
            }
        }
    };
    let mut replies = Vec::new();
    let mut next = reviews.iter().peekable();
    let mut checkpoints = 0_u32;
    let mut answering: Option<Answering<'_>> = None;
    for event in events {
        match &event.kind {
            Some(Kind::Checkpoint(_)) => checkpoints += 1,
            Some(Kind::UserMessage(message)) => {
                if let Some(review) = next.next_if(|review| review.message == message.text) {
                    if let Some(done) = answering.take() {
                        finish(done, &mut replies);
                    }
                    answering = Some(Answering {
                        review,
                        turn: None,
                        said: String::new(),
                        calls: Vec::new(),
                        time_unix_ms: event.time_unix_ms,
                    });
                }
            }
            Some(Kind::TurnStarted(_)) => {
                if let Some(answering) = answering.as_mut().filter(|a| a.turn.is_none()) {
                    // The checkpoint before the first turn counts too.
                    answering.turn = Some(checkpoints.max(1));
                }
            }
            Some(Kind::AssistantMessage(message)) => {
                if let Some(answering) = answering.as_mut().filter(|a| a.turn.is_some()) {
                    answering.said.push_str(&message.text);
                    answering.said.push('\n');
                    answering.time_unix_ms = event.time_unix_ms;
                }
            }
            Some(Kind::ToolStarted(call)) => {
                if let Some(answering) = answering.as_mut().filter(|a| a.turn.is_some())
                    && let Some((thread, text)) = tool_reply(call)
                {
                    answering.calls.push(Call {
                        id: call.call_id.clone(),
                        thread,
                        text,
                        time_unix_ms: event.time_unix_ms,
                        succeeded: false,
                    });
                }
            }
            Some(Kind::ToolCompleted(done)) => {
                if let Some(answering) = answering.as_mut() {
                    for call in &mut answering.calls {
                        if call.id == done.call_id {
                            call.succeeded = done.success;
                        }
                    }
                }
            }
            Some(Kind::TurnEnded(_)) => {
                if answering.as_ref().is_some_and(|a| a.turn.is_some())
                    && let Some(done) = answering.take()
                {
                    finish(done, &mut replies);
                }
            }
            _ => {}
        }
    }
    if let Some(done) = answering {
        finish(done, &mut replies);
    }
    replies
}

/// The thread and answer of a call to Humata's `reply_to_thread` tool, by
/// whatever name the agent's CLI gives it (`mcp__humata__reply_to_thread`
/// in Claude Code).
fn tool_reply(call: &pb::ToolStarted) -> Option<(i64, String)> {
    if !call.tool.ends_with("reply_to_thread") {
        return None;
    }
    let arguments: serde_json::Value = serde_json::from_str(&call.arguments_json).ok()?;
    let thread = match &arguments["thread_id"] {
        serde_json::Value::String(id) => id.trim().parse().ok()?,
        id => id.as_i64()?,
    };
    let text = arguments["text"].as_str()?.trim();
    (!text.is_empty()).then(|| (thread, text.to_owned()))
}

/// The parts of `text` under each `[thread N]` tag, by thread; the text
/// after the tag on its line belongs to it too. Markdown around a tag, such
/// as bold or a heading, is allowed.
fn sections(text: &str) -> Vec<(i64, String)> {
    let mut sections: Vec<(i64, String)> = Vec::new();
    for line in text.lines() {
        let bare = line.trim_start_matches(|c: char| c.is_whitespace() || "#*>_-".contains(c));
        let tagged = bare.strip_prefix("[thread ").and_then(|rest| {
            let (number, after) = rest.split_once(']')?;
            Some((number.trim().parse::<i64>().ok()?, after))
        });
        match tagged {
            Some((thread, after)) => {
                let after =
                    after.trim_start_matches(|c: char| c.is_whitespace() || "*_:-".contains(c));
                sections.push((thread, after.to_owned()));
            }
            None => {
                if let Some((_, section)) = sections.last_mut() {
                    section.push('\n');
                    section.push_str(line);
                }
            }
        }
    }
    let mut merged: Vec<(i64, String)> = Vec::new();
    for (thread, text) in sections {
        let text = text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        match merged.iter_mut().find(|(known, _)| *known == thread) {
            Some((_, kept)) => {
                kept.push_str("\n\n");
                kept.push_str(&text);
            }
            None => merged.push((thread, text)),
        }
    }
    merged
}

fn wire_thread(thread: &Thread, replies: &[AgentReply]) -> pb::ReviewThread {
    let mut comments: Vec<pb::ReviewComment> = thread
        .comments
        .iter()
        .map(|comment| pb::ReviewComment {
            id: comment.id.to_string(),
            author: pb::CommentAuthor::You.into(),
            text: comment.text.clone(),
            sent: comment.review.is_some(),
            time_unix_ms: comment.created_unix_ms,
            turn: 0,
        })
        .collect();
    comments.extend(
        replies
            .iter()
            .filter(|reply| reply.thread == thread.id)
            .map(|reply| pb::ReviewComment {
                id: format!("agent-{}-{}", reply.review, reply.thread),
                author: pb::CommentAuthor::Agent.into(),
                text: reply.text.clone(),
                sent: true,
                time_unix_ms: reply.time_unix_ms,
                turn: reply.turn,
            }),
    );
    comments.sort_by_key(|comment| comment.time_unix_ms);
    pb::ReviewThread {
        id: thread.id.to_string(),
        session_id: thread.session.clone(),
        file: Some(pb::FileRef {
            repository: thread.repository.clone(),
            path: thread.path.clone(),
        }),
        start_line: thread.start_line,
        end_line: thread.end_line,
        outdated: thread.outdated,
        code: thread.code.clone(),
        quote: thread.quote.clone(),
        resolved: thread.resolved,
        comments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn thread(start: u32, code: &str, before: &str) -> Thread {
        let code = lines(code);
        Thread {
            id: 1,
            session: "s".into(),
            repository: "app".into(),
            path: "main.rs".into(),
            start_line: start,
            end_line: start + u32::try_from(code.len()).unwrap_or(1) - 1,
            outdated: false,
            code,
            before: lines(before),
            quote: String::new(),
            resolved: false,
            comments: Vec::new(),
        }
    }

    #[test]
    fn a_thread_follows_its_lines_when_lines_are_added_above() {
        let thread = thread(2, "b\nc", "a");
        assert_eq!(locate(&thread, &lines("a\nb\nc\nd")), (2, 3, false));
        assert_eq!(locate(&thread, &lines("new\nnew\na\nb\nc")), (4, 5, false));
    }

    #[test]
    fn a_thread_picks_the_copy_of_its_lines_nearest_where_it_was() {
        let thread = thread(4, "x", "");
        assert_eq!(locate(&thread, &lines("x\n.\n.\n.\nx\n.")), (5, 5, false));
    }

    #[test]
    fn a_thread_whose_lines_changed_stays_after_the_lines_above_it_outdated() {
        let thread = thread(3, "old", "a\nb");
        assert_eq!(locate(&thread, &lines("top\na\nb\nnew\nz")), (4, 4, true));
    }

    #[test]
    fn a_thread_with_nothing_to_go_by_stays_within_the_file_outdated() {
        let thread = thread(9, "gone", "also gone");
        assert_eq!(locate(&thread, &lines("one\ntwo")), (2, 2, true));
        assert_eq!(locate(&thread, &[]), (1, 1, true));
    }

    #[test]
    fn answers_are_split_by_their_thread_tags_markdown_allowed() {
        let text = "Sure, here goes.\n\n**[thread 3]** Done: renamed it.\nAlso tidied.\n\
                    ## [thread 4]\nLeft as is.\n[thread 3]\nOne more thing.";
        assert_eq!(
            sections(text),
            [
                (
                    3,
                    "Done: renamed it.\nAlso tidied.\n\nOne more thing.".to_owned()
                ),
                (4, "Left as is.".to_owned()),
            ]
        );
    }

    fn event(time: i64, kind: pb::session_event::Kind) -> pb::SessionEvent {
        pb::SessionEvent {
            id: time.to_string(),
            time_unix_ms: time,
            kind: Some(kind),
        }
    }

    #[test]
    fn replies_come_from_the_turn_that_answered_the_review() {
        use pb::session_event::Kind;
        let review = Review {
            id: 7,
            message: "review".into(),
            threads: vec![3],
        };
        let said = |text: &str| {
            Kind::AssistantMessage(pb::AssistantMessage {
                text: text.to_owned(),
            })
        };
        let events = [
            event(1, Kind::Checkpoint(pb::Checkpoint::default())),
            event(2, Kind::UserMessage(pb::UserMessage { text: "hi".into() })),
            event(3, Kind::TurnStarted(pb::TurnStarted {})),
            event(4, said("[thread 3] not this one")),
            event(5, Kind::TurnEnded(pb::TurnEnded { aborted: false })),
            event(6, Kind::Checkpoint(pb::Checkpoint::default())),
            event(
                7,
                Kind::UserMessage(pb::UserMessage {
                    text: "review".into(),
                }),
            ),
            event(8, Kind::TurnStarted(pb::TurnStarted {})),
            event(9, said("[thread 3] fixed\n[thread 9] not asked")),
            event(10, Kind::TurnEnded(pb::TurnEnded { aborted: false })),
        ];
        assert_eq!(
            agent_replies(&events, &[review]),
            [AgentReply {
                review: 7,
                thread: 3,
                text: "fixed".into(),
                turn: 2,
                time_unix_ms: 9,
            }]
        );
    }

    #[test]
    fn replies_by_tool_count_once_the_call_succeeds() {
        use pb::session_event::Kind;
        let review = Review {
            id: 7,
            message: "review".into(),
            threads: vec![3, 4],
        };
        let call = |id: &str, thread: &str, text: &str| {
            Kind::ToolStarted(pb::ToolStarted {
                call_id: id.into(),
                tool: "mcp__humata__reply_to_thread".into(),
                arguments_json: serde_json::json!({"thread_id": thread, "text": text}).to_string(),
            })
        };
        let completed = |id: &str, success: bool| {
            Kind::ToolCompleted(pb::ToolCompleted {
                call_id: id.into(),
                success,
                output: String::new(),
            })
        };
        let events = [
            event(
                1,
                Kind::UserMessage(pb::UserMessage {
                    text: "review".into(),
                }),
            ),
            event(2, Kind::TurnStarted(pb::TurnStarted {})),
            event(3, call("a", "3", "Renamed it.")),
            event(4, completed("a", true)),
            event(5, call("b", "4", "Never mind.")),
            event(6, completed("b", false)),
            event(7, call("c", "9", "Not asked.")),
            event(8, completed("c", true)),
            event(9, Kind::TurnEnded(pb::TurnEnded { aborted: false })),
        ];
        assert_eq!(
            agent_replies(&events, &[review]),
            [AgentReply {
                review: 7,
                thread: 3,
                text: "Renamed it.".into(),
                turn: 1,
                time_unix_ms: 3,
            }]
        );
    }

    #[test]
    fn a_review_lists_each_thread_with_its_tag_lines_and_comments() {
        let mut first = thread(2, "let a = 1;", "");
        first.id = 5;
        let comment = Comment {
            id: 1,
            text: "why one?".into(),
            review: None,
            created_unix_ms: 0,
        };
        let message = compose(&[(&first, vec![&comment])], |_| (4, 4));
        let tagged = message
            .lines()
            .find(|line| line.starts_with("[thread 5]"))
            .unwrap_or_default();
        assert!(
            tagged.contains("app/main.rs") && tagged.contains('4'),
            "{message}"
        );
        assert!(message.contains("let a = 1;") && message.contains("why one?"));
        assert_eq!(sections(&message).len(), 2, "the example and the thread");
    }
}

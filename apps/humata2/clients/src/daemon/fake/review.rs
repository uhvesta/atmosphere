use super::FakeDaemon;
use crate::daemon::{DaemonError, Operation, Reply};
use crate::model::{Comment, SessionId, Speaker};

pub(super) fn send_comments(
    daemon: &FakeDaemon,
    session: &SessionId,
    comments: Vec<Comment>,
) -> Reply<()> {
    let id = session.clone();
    daemon.reply(
        Some(Operation::SendComments),
        format!("comments {session} {}", comments.len()),
        move |inner| {
            if !inner.session_mut(&id)?.status.accepts_input() {
                return Err(DaemonError::Refused {
                    what: format!("Sending comments to {id}"),
                    reason: "the session cannot take input. Reconnect its machine or resume the agent, then send again".into(),
                });
            }
            for comment in &comments {
                let matches = inner.workspace.diffs.iter().filter(|diff| diff.session == id)
                    .filter(|diff| diff.files.iter().any(|file| file.path == comment.path)).count();
                if comment.context.is_none() && matches > 1 {
                    return Err(DaemonError::Refused {
                        what: format!("Sending the saved comment on {}", comment.path),
                        reason: "the same path exists in several repositories and this older comment does not name one. Specify its repository before sending. Its text has been kept; retrying unchanged will not resolve the ambiguity".into(),
                    });
                }
            }
            let text = Comment::message(&comments);
            for diff in inner
                .workspace
                .diffs
                .iter_mut()
                .filter(|diff| diff.session == id)
            {
                for comment in &comments {
                    if comment.context.as_ref().is_none_or(|context| context.repo == diff.repo)
                        && diff.files.iter().any(|file| file.path == comment.path) {
                        let mut sent = comment.clone();
                        sent.sent = true;
                        diff.comments.push(sent);
                    }
                }
            }
            inner.output(&id, 0, Speaker::You, &text);
            inner.emit(crate::daemon::Event::Resync);
            Ok(())
        },
    )
}

pub(super) fn read_file(
    daemon: &FakeDaemon,
    session: &SessionId,
    repo: &str,
    path: &str,
) -> Reply<String> {
    let key = (session.clone(), repo.to_owned(), path.to_owned());
    daemon.reply(
        None,
        format!("read {session} {repo}/{path}"),
        move |inner| {
            inner.session_mut(&key.0)?;
            inner
                .workspace
                .file_contents
                .get(&key)
                .cloned()
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("File {}/{} in {}", key.1, key.2, key.0),
                })
        },
    )
}

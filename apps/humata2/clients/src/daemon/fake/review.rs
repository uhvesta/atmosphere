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
            inner.session_mut(&id)?;
            let mut text = format!("{} review comments:", comments.len());
            for comment in &comments {
                text.push_str(&format!(
                    " {}:{} {};",
                    comment.path, comment.line, comment.body
                ));
            }
            for diff in inner
                .workspace
                .diffs
                .iter_mut()
                .filter(|diff| diff.session == id)
            {
                for comment in &comments {
                    if diff.files.iter().any(|file| file.path == comment.path) {
                        let mut sent = comment.clone();
                        sent.sent = true;
                        diff.comments.push(sent);
                    }
                }
            }
            inner.output(&id, 0, Speaker::You, &text);
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

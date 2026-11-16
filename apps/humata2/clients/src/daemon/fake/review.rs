use super::FakeDaemon;
use crate::daemon::{DaemonError, Reply};
use crate::model::SessionId;

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

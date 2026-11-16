//! The Desk on the fake daemon (DK-1 to DK-14, SE-26).

use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Event, Operation as Call, Reply};
use crate::desk::{
    self, Access, CleanUp, CleanUpRow, Closed, DeskClient, DeskSession, Document, Folder,
    FolderChoice, LastUsed, NewDeskSession,
};
use crate::model::{
    ChannelId, FileEntry, Line, MachineId, SessionId, Speaker, Status, Trashed, Turn,
};

/// The branch the fake clones a repository on: its default branch (DK-3).
const DEFAULT_BRANCH: &str = "main";

fn call(operation: desk::Operation) -> Option<Call> {
    Some(Call::Desk(operation))
}

fn not_found(what: String) -> DaemonError {
    DaemonError::NotFound { what }
}

fn refused(what: &str, reason: &str) -> DaemonError {
    DaemonError::Refused {
        what: what.to_owned(),
        reason: reason.to_owned(),
    }
}

/// The session must be on the Desk.
fn require(inner: &Inner, id: &SessionId) -> Result<(), DaemonError> {
    let on_desk = inner.workspace.session(id).is_some_and(desk::is_desk)
        && inner.workspace.desk.sessions.contains_key(id);
    if on_desk {
        Ok(())
    } else {
        Err(not_found(format!("Desk session {id}")))
    }
}

fn record<'a>(inner: &'a mut Inner, id: &SessionId) -> Result<&'a mut DeskSession, DaemonError> {
    inner
        .workspace
        .desk
        .sessions
        .get_mut(id)
        .ok_or_else(|| not_found(format!("Desk session {id}")))
}

fn online(inner: &Inner, machine: &MachineId) -> Result<(), DaemonError> {
    match inner.workspace.machine(machine) {
        None => Err(not_found(format!("Host {machine}"))),
        Some(found) if !found.online => Err(DaemonError::Offline {
            machine: machine.clone(),
        }),
        Some(_) => Ok(()),
    }
}

fn next_order(inner: &Inner) -> u64 {
    inner
        .workspace
        .desk
        .sessions
        .values()
        .map(|record| record.last_used.order)
        .max()
        .unwrap_or(0)
        + 1
}

fn touch(inner: &mut Inner, id: &SessionId) {
    let order = next_order(inner);
    if let Some(found) = inner.workspace.desk.sessions.get_mut(id) {
        found.last_used = LastUsed {
            order,
            label: "now".to_owned(),
        };
    }
}

/// The person typed into, opened or answered a Desk session (SE-26): it
/// becomes the most recently used. Does nothing for other sessions.
pub(super) fn used(inner: &mut Inner, session: &SessionId) {
    let sessions = &inner.workspace.desk.sessions;
    let Some(found) = sessions.get(session) else {
        return;
    };
    // Typing sends this for every key: say so only when the order changes.
    let latest = sessions.values().map(|other| other.last_used.order).max();
    if Some(found.last_used.order) == latest && found.last_used.label == "now" {
        return;
    }
    touch(inner, session);
    inner.emit(Event::Resync);
}

/// The first path under the Desk's folder on `machine` that no Desk session
/// there uses, starting from `name` (FC-14).
fn free_path(inner: &Inner, machine: &MachineId, name: &str) -> String {
    let taken = |path: &str| {
        inner.workspace.desk.sessions.values().any(|found| {
            found.folder.path() == path
                && inner
                    .workspace
                    .session(&found.session)
                    .is_some_and(|session| &session.machine == machine)
        })
    };
    let mut path = desk::scratch_path(name);
    let mut suffix = 2;
    while taken(&path) {
        path = desk::scratch_path(&format!("{name}-{suffix}"));
        suffix += 1;
    }
    path
}

fn looks_like_repo(repo: &str) -> bool {
    match repo.split_once('/') {
        Some((owner, name)) => {
            !owner.is_empty()
                && !name.is_empty()
                && !name.contains('/')
                && !repo.contains(char::is_whitespace)
        }
        None => false,
    }
}

fn start_now(inner: &mut Inner, new: NewDeskSession) -> Result<SessionId, DaemonError> {
    online(inner, &new.machine)?;
    let title = new.title.trim().to_owned();
    if title.is_empty() {
        return Err(refused(
            "Starting the desk session",
            "it needs a title. Give it a title and try again",
        ));
    }
    let folder = match &new.folder {
        FolderChoice::Scratch { name } => {
            let name = name.trim();
            if name.is_empty() || name.contains('/') {
                return Err(refused(
                    "Starting the desk session",
                    "the folder name must not be empty or contain “/”. Type a plain name such as plan-q4",
                ));
            }
            let path = free_path(inner, &new.machine, name);
            Folder::Scratch { path, kept: false }
        }
        FolderChoice::Picked { path } => {
            let path = path.trim();
            if path.is_empty() {
                return Err(refused(
                    "Starting the desk session",
                    "the folder path is empty. Pick a folder and try again",
                ));
            }
            Folder::Picked { path: path.to_owned() }
        }
    };
    let mut repos: Vec<String> = Vec::new();
    for repo in &new.repos {
        if !repos.contains(repo) {
            repos.push(repo.clone());
        }
    }
    let id = SessionId::new(inner.id("desk"));
    let mut session = crate::fixture::desk_session(
        id.as_str(),
        &title,
        new.agent,
        new.machine.as_str(),
        Status::Idle,
    );
    session.started = "just now".to_owned();
    session.active = "just now".to_owned();
    session.last_used = "just now".to_owned();
    session.transcript = vec![Line {
        from: Speaker::System,
        text: format!("{} started · {title}", new.agent.name()),
    }];
    let order = next_order(inner);
    inner.workspace.sessions.push(session);
    inner.workspace.desk.sessions.insert(
        id.clone(),
        DeskSession {
            session: id.clone(),
            folder,
            repos,
            access: new.access,
            last_used: LastUsed {
                order,
                label: "now".to_owned(),
            },
            closed: None,
            unpushed: None,
        },
    );
    for repo in &new.repos {
        inner
            .workspace
            .desk
            .clone_branches
            .insert((id.clone(), repo.clone()), DEFAULT_BRANCH.to_owned());
    }
    inner.changed(&id);
    inner.emit(Event::Resync);
    Ok(id)
}

fn close_now(inner: &mut Inner, id: &SessionId) -> Result<(), DaemonError> {
    require(inner, id)?;
    if record(inner, id)?.closed.is_some() {
        return Err(refused(
            "Closing the desk session",
            "it is already closed. Reopen it first if you want to use it",
        ));
    }
    let session = inner.session_mut(id)?;
    session.status = Status::Exited { code: 0 };
    session.turn = Turn::Ended;
    session.archived = true;
    session.pinned = false;
    let found = record(inner, id)?;
    let days_left = (found.folder.expires() && found.unpushed.is_none()).then_some(7);
    found.closed = Some(Closed {
        when: "today".to_owned(),
        days_left,
    });
    inner.changed(id);
    inner.emit(Event::Resync);
    Ok(())
}

fn reopen_now(inner: &mut Inner, id: &SessionId) -> Result<(), DaemonError> {
    require(inner, id)?;
    if record(inner, id)?.closed.is_none() {
        return Err(refused(
            "Reopening the desk session",
            "it isn't closed. Nothing needs reopening",
        ));
    }
    let session = inner.session_mut(id)?;
    session.archived = false;
    session.status = Status::Idle;
    session.turn = Turn::Ended;
    record(inner, id)?.closed = None;
    touch(inner, id);
    inner.changed(id);
    inner.emit(Event::Resync);
    Ok(())
}

fn keep_now(inner: &mut Inner, id: &SessionId) -> Result<(), DaemonError> {
    require(inner, id)?;
    let found = record(inner, id)?;
    if let Folder::Scratch { kept, .. } = &mut found.folder {
        *kept = true;
        if let Some(closed) = &mut found.closed {
            closed.days_left = None;
        }
    }
    inner.emit(Event::Resync);
    Ok(())
}

fn rows(inner: &Inner) -> Vec<CleanUpRow> {
    let mut closed: Vec<&DeskSession> = inner
        .workspace
        .desk
        .sessions
        .values()
        .filter(|found| found.closed.is_some())
        .collect();
    closed.sort_by(|a, b| {
        a.last_used
            .order
            .cmp(&b.last_used.order)
            .then_with(|| a.session.cmp(&b.session))
    });
    closed
        .into_iter()
        .map(|found| CleanUpRow {
            session: found.session.clone(),
            size_bytes: found.folder.is_scratch().then(|| {
                inner
                    .workspace
                    .desk
                    .folder_bytes
                    .get(&found.session)
                    .copied()
                    .unwrap_or(0)
            }),
            unpushed: found.unpushed.clone(),
        })
        .collect()
}

fn clean_up_now(
    inner: &mut Inner,
    shown: &[CleanUpRow],
    delete: &[SessionId],
) -> Result<CleanUp, DaemonError> {
    let now = rows(inner);
    if now != shown {
        return Ok(CleanUp::Changed(now));
    }
    if let Some(missing) = delete
        .iter()
        .find(|id| !now.iter().any(|row| &row.session == *id))
    {
        return Err(not_found(format!("Closed desk session {missing}")));
    }
    let mut sessions = 0;
    let mut freed_bytes = 0;
    for row in now.iter().filter(|row| delete.contains(&row.session)) {
        let id = &row.session;
        let title = inner
            .workspace
            .session(id)
            .map_or_else(|| id.to_string(), |session| session.title.clone());
        inner.workspace.sessions.retain(|session| &session.id != id);
        inner.workspace.desk.sessions.remove(id);
        inner.workspace.desk.documents.retain(|doc| &doc.session != id);
        inner.workspace.desk.folder_bytes.remove(id);
        inner
            .workspace
            .desk
            .clone_branches
            .retain(|(session, _), _| session != id);
        inner.workspace.session_files.remove(id);
        inner
            .workspace
            .file_contents
            .retain(|(session, _, _), _| session != id);
        inner
            .workspace
            .terminal_bytes
            .retain(|(session, _), _| session != id);
        inner.workspace.trash.insert(
            0,
            Trashed {
                what: format!("Desk session “{title}”"),
                expires: "in 30 days".to_owned(),
            },
        );
        sessions += 1;
        freed_bytes += row.size_bytes.unwrap_or(0);
    }
    inner.emit(Event::Resync);
    Ok(CleanUp::Deleted {
        sessions,
        freed_bytes,
    })
}

fn copy_now(
    inner: &mut Inner,
    document: &Document,
    channel: &ChannelId,
    machine: Option<&MachineId>,
) -> Result<(), DaemonError> {
    if !inner.workspace.desk.documents.contains(document) {
        return Err(not_found(format!(
            "Document {} of {}",
            document.path, document.session
        )));
    }
    let target = inner
        .workspace
        .channel(channel)
        .ok_or_else(|| not_found(format!("Channel {channel}")))?
        .clone();
    let machine = match machine {
        None => match target.machines.as_slice() {
            [only] => only.clone(),
            _ => {
                return Err(refused(
                    "Copying the document",
                    "the channel runs on several hosts. Pick the host to copy it to",
                ));
            }
        },
        Some(chosen) if target.machines.contains(chosen) => chosen.clone(),
        Some(chosen) => {
            return Err(refused(
                "Copying the document",
                &format!(
                    "#{} doesn't run on {chosen}. Pick one of the channel's hosts",
                    target.name
                ),
            ));
        }
    };
    online(inner, &machine)?;
    let name = document.path.rsplit('/').next().unwrap_or_default();
    let path = format!("{}/{}/shared/{name}", desk::ROOT, target.name);
    let length = inner
        .workspace
        .file_contents
        .get(&(document.session.clone(), String::new(), document.path.clone()))
        .map_or(0, String::len);
    let entry = FileEntry {
        machine: machine.clone(),
        path: path.clone(),
        size: format!("{length} B"),
        folder: false,
    };
    let files = &mut inner.workspace.files;
    match files
        .iter_mut()
        .find(|file| file.machine == machine && file.path == path)
    {
        Some(existing) => *existing = entry,
        None => files.push(entry),
    }
    inner.emit(Event::Resync);
    Ok(())
}

fn add_repo_now(inner: &mut Inner, id: &SessionId, repo: &str) -> Result<(), DaemonError> {
    require(inner, id)?;
    let repo = repo.trim();
    if !looks_like_repo(repo) {
        return Err(refused(
            "Adding the repository",
            "it must be written owner/name, such as acme/api. Fix the name and try again",
        ));
    }
    let found = record(inner, id)?;
    if found.repos.iter().any(|have| have == repo) {
        return Err(refused(
            "Adding the repository",
            "it is already in this session's folder",
        ));
    }
    found.repos.push(repo.to_owned());
    inner
        .workspace
        .desk
        .clone_branches
        .insert((id.clone(), repo.to_owned()), DEFAULT_BRANCH.to_owned());
    inner.emit(Event::Resync);
    Ok(())
}

fn remove_repo_now(inner: &mut Inner, id: &SessionId, repo: &str) -> Result<(), DaemonError> {
    require(inner, id)?;
    let found = record(inner, id)?;
    let Some(at) = found.repos.iter().position(|have| have == repo) else {
        return Err(not_found(format!("Repository {repo} in {id}")));
    };
    found.repos.remove(at);
    inner
        .workspace
        .desk
        .clone_branches
        .remove(&(id.clone(), repo.to_owned()));
    inner.emit(Event::Resync);
    Ok(())
}

fn move_now(inner: &mut Inner, id: &SessionId, machine: &MachineId) -> Result<(), DaemonError> {
    require(inner, id)?;
    if inner.workspace.machine(machine).is_none() {
        return Err(not_found(format!("Host {machine}")));
    }
    let current = inner
        .workspace
        .session(id)
        .map(|session| session.machine.clone())
        .ok_or_else(|| not_found(format!("Desk session {id}")))?;
    if &current == machine {
        return Err(refused(
            "Moving the desk session",
            "it already runs on that host. Pick another host",
        ));
    }
    if record(inner, id)?.closed.is_some() {
        return Err(refused(
            "Moving the desk session",
            "it is closed. Reopen it first, then move it",
        ));
    }
    online(inner, machine)?;
    online(inner, &current)?;
    inner.session_mut(id)?.machine = machine.clone();
    inner.changed(id);
    inner.emit(Event::Resync);
    Ok(())
}

impl DeskClient for FakeDaemon {
    fn start(&self, new: NewDeskSession) -> Reply<SessionId> {
        let line = format!("desk start {:?} on {}", new.title, new.machine);
        self.reply(call(desk::Operation::Start), line, move |inner| {
            start_now(inner, new)
        })
    }

    fn close(&self, session: &SessionId) -> Reply<()> {
        let id = session.clone();
        self.reply(
            call(desk::Operation::Close),
            format!("desk close {session}"),
            move |inner| close_now(inner, &id),
        )
    }

    fn reopen(&self, session: &SessionId) -> Reply<()> {
        let id = session.clone();
        self.reply(
            call(desk::Operation::Reopen),
            format!("desk reopen {session}"),
            move |inner| reopen_now(inner, &id),
        )
    }

    fn keep(&self, session: &SessionId) -> Reply<()> {
        let id = session.clone();
        self.reply(
            call(desk::Operation::Keep),
            format!("desk keep {session}"),
            move |inner| keep_now(inner, &id),
        )
    }

    fn clean_up_list(&self) -> Reply<Vec<CleanUpRow>> {
        self.reply(
            call(desk::Operation::CleanUpList),
            "desk clean up list".to_owned(),
            |inner| Ok(rows(inner)),
        )
    }

    fn clean_up(&self, shown: Vec<CleanUpRow>, delete: Vec<SessionId>) -> Reply<CleanUp> {
        let line = format!("desk clean up {} of {}", delete.len(), shown.len());
        self.reply(call(desk::Operation::CleanUp), line, move |inner| {
            clean_up_now(inner, &shown, &delete)
        })
    }

    fn copy_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Reply<()> {
        let line = format!(
            "desk copy {} of {} to {channel} on {machine:?}",
            document.path, document.session
        );
        let document = document.clone();
        let channel = channel.clone();
        let machine = machine.cloned();
        self.reply(call(desk::Operation::CopyDocument), line, move |inner| {
            copy_now(inner, &document, &channel, machine.as_ref())
        })
    }

    fn add_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let id = session.clone();
        let repo = repo.to_owned();
        self.reply(
            call(desk::Operation::AddRepo),
            format!("desk add repo {repo} to {session}"),
            move |inner| add_repo_now(inner, &id, &repo),
        )
    }

    fn remove_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let id = session.clone();
        let repo = repo.to_owned();
        self.reply(
            call(desk::Operation::RemoveRepo),
            format!("desk remove repo {repo} from {session}"),
            move |inner| remove_repo_now(inner, &id, &repo),
        )
    }

    fn move_to(&self, session: &SessionId, machine: &MachineId) -> Reply<()> {
        let id = session.clone();
        let target = machine.clone();
        self.reply(
            call(desk::Operation::Move),
            format!("desk move {session} to {machine}"),
            move |inner| move_now(inner, &id, &target),
        )
    }

    fn set_access(&self, session: &SessionId, access: Access) -> Reply<()> {
        let id = session.clone();
        self.reply(
            call(desk::Operation::SetAccess),
            format!("desk access {session} {access:?}"),
            move |inner| {
                require(inner, &id)?;
                record(inner, &id)?.access = access;
                inner.emit(Event::Resync);
                Ok(())
            },
        )
    }
}

impl FakeDaemon {
    /// Stands in for the person typing into, opening or answering a Desk
    /// session (SE-26): it becomes the most recently used. Does nothing for
    /// a session that isn't on the Desk.
    pub fn use_desk_session(&self, session: &SessionId) {
        used(&mut self.inner(), session);
    }
}

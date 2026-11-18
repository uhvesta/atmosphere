//! The Desk on the fake daemon (DK-1 to DK-14, SE-26).

use super::repos as library;
use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Event, Operation as Call, Reply};
use crate::desk::{
    self, Access, ChannelDeskSession, CleanUp, CleanUpRow, Closed, DeskClient, DeskSession,
    Document, Folder, FolderChoice, LastUsed, NewChannelDesk, NewDeskSession, Switches,
};
use crate::model::{
    ChannelId, FileEntry, Line, MachineId, SessionId, Speaker, Status, Trashed, Turn,
};
use crate::repos::RepoPlace;

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
    let root = desk::root_of(&inner.workspace.machines, machine);
    let mut path = desk::scratch_path_in(root, name);
    let mut suffix = 2;
    while taken(&path) {
        path = desk::scratch_path_in(root, &format!("{name}-{suffix}"));
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
            has_versions: folder.is_scratch(),
            doc_head: String::new(),
            version_count: 0,
            holds_files: false,
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
    for repo in &new.repos {
        library::record(inner, repo, RepoPlace::Desk);
    }
    inner.changed(&id);
    inner.emit(Event::Resync);
    Ok(id)
}

fn close_now(inner: &mut Inner, id: &SessionId) -> Result<(), DaemonError> {
    if inner.workspace.desk.channel_sessions.contains_key(id) {
        return close_channel_now(inner, id);
    }
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
    if inner.workspace.desk.channel_sessions.contains_key(id) {
        return reopen_channel_now(inner, id);
    }
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

/// Starts a channel's own Desk session (CD-1, CD-6): a scratch folder
/// `<root>/<channel>/desk/<name>`, no clones, filed under no channel's tree.
fn start_channel_now(inner: &mut Inner, new: NewChannelDesk) -> Result<SessionId, DaemonError> {
    const WHAT: &str = "Starting the channel desk session";
    online(inner, &new.machine)?;
    let title = new.title.trim().to_owned();
    if title.is_empty() {
        return Err(refused(
            WHAT,
            "it needs a title. Give it a title and try again",
        ));
    }
    let Some(channel) = inner.workspace.channel(&new.channel).cloned() else {
        return Err(not_found(format!("Channel {}", new.channel)));
    };
    if let Some(target) = &new.target {
        let in_channel = inner
            .workspace
            .session(target)
            .is_some_and(|session| session.channel == new.channel);
        if !in_channel {
            return Err(not_found(format!(
                "Session {target} in channel {}",
                channel.name
            )));
        }
    }
    let name = match desk::folder_name(&title) {
        name if name.is_empty() => "desk-session".to_owned(),
        name => name,
    };
    let root = desk::root_of(&inner.workspace.machines, &new.machine).to_owned();
    let base = format!("{root}/{}/{}", desk::folder_name(&channel.name), desk::FOLDER);
    let taken = |inner: &Inner, path: &str| {
        inner
            .workspace
            .desk
            .channel_sessions
            .values()
            .any(|found| found.desk.folder.path() == path)
    };
    let mut path = format!("{base}/{name}");
    let mut suffix = 2;
    while taken(inner, &path) {
        path = format!("{base}/{name}-{suffix}");
        suffix += 1;
    }
    let rules = inner
        .workspace
        .rules
        .get(&new.channel)
        .cloned()
        .unwrap_or_default();
    let id = SessionId::new(inner.id("cdesk"));
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
    let order = inner
        .workspace
        .desk
        .channel_sessions
        .values()
        .map(|found| found.desk.last_used.order)
        .max()
        .unwrap_or(0)
        + 1;
    inner.workspace.sessions.push(session);
    inner.workspace.desk.channel_sessions.insert(
        id.clone(),
        ChannelDeskSession {
            desk: DeskSession::fresh(
                id.clone(),
                Folder::Scratch { path, kept: false },
                LastUsed {
                    order,
                    label: "now".to_owned(),
                },
            ),
            channel: new.channel,
            target: new.target,
            switches: new.switches,
            blocked: desk::blocked_by(&rules),
            target_gone: false,
            documents: Vec::new(),
            document_lines: std::collections::BTreeMap::new(),
        },
    );
    inner.changed(&id);
    inner.emit(Event::Resync);
    Ok(id)
}

/// A channel Desk session's folder is kept once it holds a version (DV-13).
/// A channel Desk session has no Keep folder (CD-4).
fn channel_folder_kept(found: &ChannelDeskSession) -> bool {
    found.desk.version_count > 0
}

fn next_channel_order(inner: &Inner) -> u64 {
    inner
        .workspace
        .desk
        .channel_sessions
        .values()
        .map(|found| found.desk.last_used.order)
        .max()
        .unwrap_or(0)
        + 1
}

/// Closing a channel Desk session (CD-4, DV-13): a folder that is not kept
/// goes to the trash at once, so the session is gone from the channel's
/// Desk. A kept folder stays: the session closes in place, under Closed.
/// Closing one that is already closed changes nothing.
fn close_channel_now(inner: &mut Inner, id: &SessionId) -> Result<(), DaemonError> {
    let Some(found) = inner.workspace.desk.channel_sessions.get(id) else {
        return Err(not_found(format!("Channel desk session {id}")));
    };
    if found.desk.closed.is_some() {
        return Ok(());
    }
    if !channel_folder_kept(found) {
        inner.workspace.desk.channel_sessions.remove(id);
        inner.workspace.sessions.retain(|session| &session.id != id);
        inner.emit(Event::Resync);
        return Ok(());
    }
    let order = next_channel_order(inner);
    let session = inner.session_mut(id)?;
    session.status = Status::Exited { code: 0 };
    session.turn = Turn::Ended;
    if let Some(found) = inner.workspace.desk.channel_sessions.get_mut(id) {
        found.desk.closed = Some(Closed {
            when: "today".to_owned(),
            days_left: None,
        });
        found.desk.last_used = LastUsed {
            order,
            label: "now".to_owned(),
        };
    }
    inner.changed(id);
    inner.emit(Event::Resync);
    Ok(())
}

/// Reopening a channel Desk session closed in place (DV-13): it is an open
/// row again, the most recently used. One that is open has nothing to reopen.
fn reopen_channel_now(inner: &mut Inner, id: &SessionId) -> Result<(), DaemonError> {
    let order = next_channel_order(inner);
    let Some(found) = inner.workspace.desk.channel_sessions.get_mut(id) else {
        return Err(not_found(format!("Channel desk session {id}")));
    };
    if found.desk.closed.is_none() {
        return Err(refused(
            "Reopening the channel desk session",
            "it isn't closed. Nothing needs reopening",
        ));
    }
    found.desk.closed = None;
    found.desk.last_used = LastUsed {
        order,
        label: "now".to_owned(),
    };
    let session = inner.session_mut(id)?;
    session.status = Status::Idle;
    session.turn = Turn::Ended;
    inner.changed(id);
    inner.emit(Event::Resync);
    Ok(())
}

fn set_switches_now(
    inner: &mut Inner,
    id: &SessionId,
    switches: Switches,
) -> Result<(), DaemonError> {
    let found = inner
        .workspace
        .desk
        .channel_sessions
        .get_mut(id)
        .ok_or_else(|| not_found(format!("Channel desk session {id}")))?;
    found.switches = switches;
    inner.emit(Event::Resync);
    Ok(())
}

/// Changing what a channel Desk session looks after (CD-5): an open session
/// moves to a session of its channel, or to the whole channel; "Target gone"
/// ends. The session's status stays as it was.
fn set_target_now(
    inner: &mut Inner,
    id: &SessionId,
    target: Option<SessionId>,
) -> Result<(), DaemonError> {
    let channel = inner
        .workspace
        .desk
        .channel_sessions
        .get(id)
        .filter(|found| found.desk.closed.is_none())
        .map(|found| found.channel.clone())
        .ok_or_else(|| not_found(format!("Channel desk session {id}")))?;
    if let Some(wanted) = &target {
        let there = inner
            .workspace
            .session(wanted)
            .is_some_and(|session| session.channel == channel && !session.archived);
        if !there {
            return Err(refused(
                "Changing what a desk session looks after",
                "that session is not one of the channel's any more; pick another session, or have it look after the whole channel",
            ));
        }
    }
    let mut resumes = false;
    if let Some(found) = inner.workspace.desk.channel_sessions.get_mut(id) {
        resumes = found.target_gone;
        found.target = target;
        found.target_gone = false;
    }
    // The daemon stopped it for its target; a new one runs it again.
    if resumes
        && let Some(session) = inner.workspace.sessions.iter_mut().find(|s| &s.id == id)
    {
        session.status = Status::Idle;
    }
    inner.emit(Event::Resync);
    Ok(())
}

/// A session went (deleted, or out of its channel): each open channel Desk
/// session that looked after it closes when its folder holds no Markdown,
/// else stops and shows its target gone (CD-5), as the daemon's watch does.
fn target_went_now(inner: &mut Inner, target: &SessionId) {
    let hit: Vec<SessionId> = inner
        .workspace
        .desk
        .channel_sessions
        .iter()
        .filter(|(_, found)| found.desk.closed.is_none() && found.target.as_ref() == Some(target))
        .map(|(id, _)| id.clone())
        .collect();
    for id in hit {
        let keeps = inner
            .workspace
            .desk
            .channel_sessions
            .get(&id)
            .is_some_and(|found| !found.documents.is_empty());
        if !keeps {
            // Nothing to keep: the session closes at once, as for any close.
            let _ = close_channel_now(inner, &id);
            continue;
        }
        if let Some(found) = inner.workspace.desk.channel_sessions.get_mut(&id) {
            found.target_gone = true;
        }
        if let Some(session) = inner.workspace.sessions.iter_mut().find(|s| s.id == id) {
            session.status = Status::Stopped;
        }
    }
    inner.emit(Event::Resync);
}

impl FakeDaemon {
    /// Stands in for the daemon deleting session `session`: the channel Desk
    /// sessions that looked after it close or show their target gone.
    pub fn remove_session(&self, session: &SessionId) {
        let mut inner = self.inner();
        inner.workspace.sessions.retain(|found| &found.id != session);
        target_went_now(&mut inner, session);
    }

    /// Stands in for session `session` leaving its channel for `channel`.
    pub fn relocate_session(&self, session: &SessionId, channel: &ChannelId) {
        let mut inner = self.inner();
        if let Some(found) = inner.workspace.sessions.iter_mut().find(|s| &s.id == session) {
            found.channel = channel.clone();
        }
        target_went_now(&mut inner, session);
    }
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
        inner
            .workspace
            .desk
            .documents
            .retain(|doc| &doc.session != id);
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
    let path = format!(
        "{}/{}/shared/{name}",
        desk::root_of(&inner.workspace.machines, &machine),
        target.name
    );
    let length = inner
        .workspace
        .file_contents
        .get(&(
            document.session.clone(),
            String::new(),
            document.path.clone(),
        ))
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
    library::record(inner, repo, RepoPlace::Desk);
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
    if record(inner, id)?.folder.is_scratch() {
        let name = record(inner, id)?
            .folder
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let moved = free_path(inner, machine, &name);
        if let Folder::Scratch { path, .. } = &mut record(inner, id)?.folder {
            *path = moved;
        }
    }
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

    fn list_versions(
        &self,
        session: &SessionId,
        path: &str,
    ) -> Reply<crate::docs::DocVersions> {
        super::docs::list_versions(self, session, path)
    }

    fn read_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<crate::docs::VersionText> {
        super::docs::read_version(self, session, path, id)
    }

    fn save_version(
        &self,
        session: &SessionId,
        message: &str,
    ) -> Reply<crate::docs::DocVersion> {
        super::docs::save_version(self, session, message)
    }

    fn restore_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<crate::docs::Restored> {
        super::docs::restore_version(self, session, path, id)
    }

    fn diff_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<crate::docs::VersionDiff> {
        super::docs::diff_version(self, session, path, id)
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

    fn channel_sessions(&self, channel: &ChannelId) -> Reply<Vec<ChannelDeskSession>> {
        let channel = channel.clone();
        self.reply(
            None,
            format!("desk of {channel}"),
            move |inner| {
                let mut found: Vec<ChannelDeskSession> = inner
                    .workspace
                    .desk
                    .channel_sessions
                    .values()
                    .filter(|found| found.channel == channel)
                    .cloned()
                    .collect();
                found.sort_by(|a, b| {
                    b.desk
                        .last_used
                        .order
                        .cmp(&a.desk.last_used.order)
                        .then_with(|| a.desk.session.cmp(&b.desk.session))
                });
                Ok(found)
            },
        )
    }

    fn target_sessions(&self, target: &SessionId) -> Reply<Vec<ChannelDeskSession>> {
        let target = target.clone();
        self.reply(
            None,
            format!("desk looking after {target}"),
            move |inner| {
                let mut found: Vec<ChannelDeskSession> = inner
                    .workspace
                    .desk
                    .channel_sessions
                    .values()
                    .filter(|found| found.target.as_ref() == Some(&target))
                    .cloned()
                    .collect();
                found.sort_by(|a, b| {
                    b.desk
                        .last_used
                        .order
                        .cmp(&a.desk.last_used.order)
                        .then_with(|| a.desk.session.cmp(&b.desk.session))
                });
                Ok(found)
            },
        )
    }

    fn start_channel(&self, new: NewChannelDesk) -> Reply<SessionId> {
        let line = format!(
            "desk start {:?} in {} on {}",
            new.title, new.channel, new.machine
        );
        self.reply(call(desk::Operation::StartChannel), line, move |inner| {
            start_channel_now(inner, new)
        })
    }

    fn set_switches(&self, session: &SessionId, switches: Switches) -> Reply<()> {
        let id = session.clone();
        self.reply(
            call(desk::Operation::SetSwitches),
            format!("desk switches {session} {switches:?}"),
            move |inner| set_switches_now(inner, &id, switches),
        )
    }

    fn set_target(&self, session: &SessionId, target: Option<&SessionId>) -> Reply<()> {
        let id = session.clone();
        let target = target.cloned();
        self.reply(
            call(desk::Operation::SetTarget),
            format!("desk target {session} {target:?}"),
            move |inner| set_target_now(inner, &id, target),
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

    /// Stands in for another client (the CLI, another window, another
    /// machine) changing the Desk behind the app's back: `change` edits the
    /// Desk and only Desk events follow, no reload.
    pub fn change_desk_elsewhere(&self, change: impl FnOnce(&mut desk::DeskData)) {
        let mut inner = self.inner();
        change(&mut inner.workspace.desk);
        inner.tell_desk();
    }
}

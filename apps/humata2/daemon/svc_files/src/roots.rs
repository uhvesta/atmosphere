//! What a `FileLocation` names on this machine (FC-1, FM-10): a session's
//! clone, a channel's folder, its `shared/`, a main clone, or (for the
//! user only, which the policy enforces) the home folder or what was
//! pasted into a session's terminals (D335), or the place files from
//! another machine land (PL-4), and a path
//! inside it. A path never leaves its root: not by "..", not by an
//! absolute path, and not through a symlink.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_access::{ChannelId, Entry, MachineId, Place, RepoPath, RepoRef, Root};
use crate::pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::ports::{
    CHANNELS_ROOT, FileLayout, error, failure, internal, invalid_field, missing, not_found,
    session_ref, store_error,
};

/// The folders a location can name here: the store's sessions and
/// channels, and the folder the channels live in.
pub struct Roots {
    store: Arc<Store>,
    machine: String,
    layout: FileLayout,
    /// The user's home folder on this machine (FM-10).
    home: PathBuf,
}

impl Roots {
    /// `channels` is the folder every channel's folder is in; `home` is
    /// the user's home folder here.
    pub fn new(
        store: Arc<Store>,
        machine: String,
        channels: impl Into<FileLayout>,
        home: PathBuf,
    ) -> Self {
        Self {
            store,
            machine,
            layout: channels.into(),
            home,
        }
    }

    /// The store sessions and channels are read from.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// This machine's id.
    pub fn machine(&self) -> &str {
        &self.machine
    }

    /// The main clone a path lands in, on or above, if any (FC-2, D165).
    fn main_clone_at(
        &self,
        machine: Option<&pb::MachineId>,
        folder: &Path,
        path: &RepoPath,
        entry: Entry,
    ) -> Result<Option<Root>, Status> {
        if machine.is_some_and(|named| !named.value.is_empty() && named.value != self.machine) {
            return Ok(None);
        }
        let landed = landing(&folder.join(path.as_str()));
        let channels = self
            .store
            .channels()
            .map_err(|err| store_error(err, &self.machine, "look up the channels"))?;
        for channel in channels {
            for repo in &channel.repos {
                let main = landing(&self.layout.main_clone(&channel.folder, &repo.name));
                let holds_it = entry == Entry::File && main.starts_with(&landed);
                if landed.starts_with(&main) || holds_it {
                    return Ok(Some(Root::MainClone {
                        channel: ChannelId::new(channel.id.as_str()),
                        repo: RepoRef::new(format!("{}/{}", repo.owner, repo.name)),
                    }));
                }
            }
        }
        Ok(None)
    }

    /// The entry `location` names on this machine and the place the policy
    /// judges: [`Roots::place`], then the disk.
    pub fn target(
        &self,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
    ) -> Result<Target, Status> {
        let placed = self.place(location, field, entry)?;
        if placed.machine() != self.machine {
            return Err(invalid_field(
                &format!("{field}.machine"),
                Problem::NotAllowedHere,
                format!(
                    "this is machine {}, and files on machine {} aren't relayed through it yet",
                    self.machine,
                    placed.machine()
                ),
                "ask the daemon on that machine, or leave the machine out for this one",
            ));
        }
        Ok(placed.into_target())
    }

    /// What `location` names on whichever machine it is on, and the place the
    /// policy judges, without looking at any disk.
    pub fn place(
        &self,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
    ) -> Result<Placed, Status> {
        use pb::file_location::Root as R;

        let location = location.ok_or_else(|| missing(field))?;
        let given = location
            .machine
            .as_ref()
            .map(|machine| machine.value.clone())
            .filter(|machine| !machine.is_empty());
        let named = given.clone().unwrap_or_else(|| self.machine.clone());
        if location.path.is_empty() && entry == Entry::File {
            return Err(missing(&format!("{field}.path")));
        }
        let path = RepoPath::new(location.path.as_str()).map_err(|_| outside(&location.path))?;
        let here = MachineId::new(named.as_str());
        let (root, folder, policy_path, writable, normal) = match location.root {
            Some(R::SessionClone(clone)) => {
                let id = clone
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| missing(&format!("{field}.session_clone.session")))?;
                let record = self.session_record(&id)?;
                let repo = repo_of(clone.repo, &format!("{field}.session_clone.repo"))?;
                let policy_path =
                    under(&format!("{}/{}", repo.owner, repo.name), &path, &location.path)?;
                let session = session_ref(&record, &self.machine);
                let folder = record.folder.join(&repo.name);
                let place_machine = session.machine.clone();
                let session_machine = place_machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(invalid_field(
                        &format!("{field}.machine"),
                        Problem::NotAllowedHere,
                        format!("session {id} is on machine {session_machine}, not {named}"),
                        "leave the machine out, or name the session's own",
                    ));
                }
                let normal = R::SessionClone(pb::SessionClone {
                    session: Some(pb::SessionId { value: id }),
                    repo: Some(repo),
                });
                return Ok(Placed {
                    place: Place {
                        machine: place_machine,
                        root: Root::Clone(session),
                        path: policy_path,
                        entry,
                    },
                    folder,
                    path,
                    shown: shown(&location.path),
                    location: pb::FileLocation {
                        machine: Some(pb::MachineId {
                            value: session_machine,
                        }),
                        root: Some(normal),
                        path: location.path,
                    },
                    writable: true,
                });
            }
            Some(R::SessionFolder(own)) => {
                let id = own
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| missing(&format!("{field}.session_folder.session")))?;
                let record = self.session_record(&id)?;
                let session = session_ref(&record, &self.machine);
                let place_machine = session.machine.clone();
                let session_machine = place_machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(invalid_field(
                        &format!("{field}.machine"),
                        Problem::NotAllowedHere,
                        format!("session {id} is on machine {session_machine}, not {named}"),
                        "leave the machine out, or name the session's own",
                    ));
                }
                return Ok(Placed {
                    place: Place {
                        machine: place_machine,
                        root: Root::Clone(session),
                        path: path.clone(),
                        entry,
                    },
                    folder: record.folder,
                    path,
                    shown: shown(&location.path),
                    location: pb::FileLocation {
                        machine: Some(pb::MachineId {
                            value: session_machine,
                        }),
                        root: Some(R::SessionFolder(pb::SessionFolder {
                            session: Some(pb::SessionId { value: id }),
                        })),
                        path: location.path,
                    },
                    writable: true,
                });
            }
            Some(R::SessionPastes(pastes)) => {
                let id = pastes
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| missing(&format!("{field}.session_pastes.session")))?;
                let record = self.session_record(&id)?;
                let session = session_ref(&record, &self.machine);
                let session_machine = session.machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(invalid_field(
                        &format!("{field}.machine"),
                        Problem::NotAllowedHere,
                        format!("session {id} is on machine {session_machine}, not {named}"),
                        "leave the machine out, or name the session's own",
                    ));
                }
                let folder = match session_machine == self.machine {
                    true => self.store.paste_folder(&id).map_err(|err| {
                        store_error(err, &self.machine, "make the session's paste folder")
                    })?,
                    false => PathBuf::new(),
                };
                return Ok(Placed {
                    place: Place {
                        machine: session.machine.clone(),
                        root: Root::Pastes(session),
                        path: path.clone(),
                        entry,
                    },
                    folder,
                    path,
                    shown: shown(&location.path),
                    location: pb::FileLocation {
                        machine: Some(pb::MachineId {
                            value: session_machine,
                        }),
                        root: Some(R::SessionPastes(pb::SessionPastes {
                            session: Some(pb::SessionId { value: id }),
                        })),
                        path: location.path,
                    },
                    writable: true,
                });
            }
            Some(R::SessionAgentState(agent_state)) => {
                let id = agent_state
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| missing(&format!("{field}.session_agent_state.session")))?;
                let record = self.session_record(&id)?;
                let session = session_ref(&record, &self.machine);
                let session_machine = session.machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(invalid_field(
                        &format!("{field}.machine"),
                        Problem::NotAllowedHere,
                        format!("session {id} is on machine {session_machine}, not {named}"),
                        "leave the machine out, or name the session's own",
                    ));
                }
                let conversation_id = agent_state.conversation_id;
                let folder = match session_machine == self.machine {
                    true => {
                        let Some(artifacts) = session_agent_artifacts(
                            &self.store,
                            &self.home,
                            &id,
                            &conversation_id,
                        )?
                        else {
                            return Err(not_found(
                                pb::EntityKind::RootFolder,
                                &id,
                                format!(
                                    "session {id} has no agent artifacts folder on this machine"
                                ),
                                "only supported agents with an active conversation expose agent files",
                            ));
                        };
                        artifacts.root
                    }
                    false => PathBuf::new(),
                };
                return Ok(Placed {
                    place: Place {
                        machine: session.machine.clone(),
                        root: Root::AgentState(session),
                        path: path.clone(),
                        entry,
                    },
                    folder,
                    path,
                    shown: shown(&location.path),
                    location: pb::FileLocation {
                        machine: Some(pb::MachineId {
                            value: session_machine,
                        }),
                        root: Some(R::SessionAgentState(pb::SessionAgentState {
                            session: Some(pb::SessionId { value: id }),
                            conversation_id,
                        })),
                        path: location.path,
                    },
                    writable: false,
                });
            }
            Some(R::ChannelShared(shared)) => {
                let channel =
                    self.channel(shared.channel, &format!("{field}.channel_shared.channel"))?;
                (
                    Root::Shared(ChannelId::new(channel.id.as_str())),
                    self.layout.shared(&channel.folder),
                    path.clone(),
                    true,
                    R::ChannelShared(pb::ChannelShared {
                        channel: Some(pb::ChannelId { value: channel.id }),
                    }),
                )
            }
            Some(R::ChannelFolder(whole)) => {
                let channel =
                    self.channel(whole.channel, &format!("{field}.channel_folder.channel"))?;
                (
                    Root::ChannelFolder(ChannelId::new(channel.id.as_str())),
                    self.layout.channel(&channel.folder),
                    path.clone(),
                    true,
                    R::ChannelFolder(pb::ChannelFolder {
                        channel: Some(pb::ChannelId { value: channel.id }),
                    }),
                )
            }
            Some(R::MainClone(main)) => {
                let channel = self.channel(main.channel, &format!("{field}.main_clone.channel"))?;
                let repo = repo_of(main.repo, &format!("{field}.main_clone.repo"))?;
                if !channel
                    .repos
                    .iter()
                    .any(|known| known.owner == repo.owner && known.name == repo.name)
                {
                    return Err(invalid_field(
                        &format!("{field}.main_clone.repo"),
                        Problem::NotAllowedHere,
                        format!(
                            "channel {} has no repository {}/{}",
                            channel.name, repo.owner, repo.name
                        ),
                        "name one of the channel's repositories, or add this one to the channel first",
                    ));
                }
                (
                    Root::MainClone {
                        channel: ChannelId::new(channel.id.as_str()),
                        repo: RepoRef::new(format!("{}/{}", repo.owner, repo.name)),
                    },
                    self.layout.main_clone(&channel.folder, &repo.name),
                    path.clone(),
                    false,
                    R::MainClone(pb::MainClone {
                        channel: Some(pb::ChannelId { value: channel.id }),
                        repo: Some(repo),
                    }),
                )
            }
            Some(R::Home(_)) => (
                Root::Home,
                self.home.clone(),
                path.clone(),
                true,
                R::Home(pb::MachineHome {}),
            ),
            Some(R::Received(received)) => {
                let from = received
                    .from
                    .map(|machine| machine.value)
                    .ok_or_else(|| missing(&format!("{field}.received.from")))?;
                check_machine_name(&from, &format!("{field}.received.from"))?;
                let folder = self.layout.root().join("received").join(&from);
                if named == self.machine {
                    fs::create_dir_all(&folder)
                        .map_err(|err| io_failure(&err, "make", &folder))?;
                }
                (
                    Root::Received,
                    folder,
                    path.clone(),
                    true,
                    R::Received(pb::MachineReceived {
                        from: Some(pb::MachineId { value: from }),
                    }),
                )
            }
            None => return Err(missing(&format!("{field}.root"))),
        };
        let through = matches!(root, Root::Home | Root::ChannelFolder(_));
        let landed = match through {
            true => self.main_clone_at(location.machine.as_ref(), &folder, &path, entry)?,
            false => None,
        };
        let root = landed.unwrap_or(root);
        Ok(Placed {
            place: Place {
                machine: here,
                root,
                path: policy_path,
                entry,
            },
            folder,
            path,
            shown: shown(&location.path),
            location: pb::FileLocation {
                machine: Some(pb::MachineId { value: named }),
                root: Some(normal),
                path: location.path,
            },
            writable,
        })
    }

    fn session_record(&self, id: &str) -> Result<SessionRecord, Status> {
        match self.store.session(id) {
            Ok(Some(record)) => Ok(record),
            Ok(None) => Err(not_found(
                pb::EntityKind::Session,
                id,
                format!("no session {id} on this machine"),
                "check the session id; sessions on other machines can't be named here yet",
            )),
            Err(err) => Err(store_error(err, &self.machine, "look up the session")),
        }
    }

    fn channel(&self, id: Option<pb::ChannelId>, field: &str) -> Result<ChannelRecord, Status> {
        let id = id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| missing(field))?;
        if id == CHANNELS_ROOT {
            return Ok(ChannelRecord {
                id,
                name: "Desk".to_owned(),
                folder: String::new(),
                branch_prefix: None,
                default_agent: String::new(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 0,
                created_unix_ms: 0,
            });
        }
        match self.store.channel(&id) {
            Ok(Some(channel)) => Ok(channel),
            Ok(None) => Err(not_found(
                pb::EntityKind::Channel,
                &id,
                format!("no channel {id} on this machine"),
                "check the channel id; `humata channels` lists the channels here",
            )),
            Err(err) => Err(store_error(err, &self.machine, "look up the channel")),
        }
    }
}

fn landing(path: &Path) -> PathBuf {
    let mut exists = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(exists) {
            let mut landed = real;
            landed.extend(rest.iter().rev());
            return landed;
        }
        match (exists.parent(), exists.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                exists = parent;
            }
            _ => return path.to_owned(),
        }
    }
}

fn repo_of(repo: Option<pb::RepoRef>, field: &str) -> Result<pb::RepoRef, Status> {
    let repo = repo
        .filter(|repo| !repo.owner.is_empty() && !repo.name.is_empty())
        .ok_or_else(|| missing(field))?;
    if repo.name.contains('/') || repo.name == "." || repo.name == ".." {
        return Err(invalid_field(
            &format!("{field}.name"),
            Problem::BadFormat,
            format!("{:?} isn't a repository name", repo.name),
            "give the repository's name on GitHub, like api",
        ));
    }
    Ok(repo)
}

fn check_machine_name(from: &str, field: &str) -> Result<(), Status> {
    if from.is_empty() || from.contains(['/', '\\']) || from == "." || from == ".." {
        return Err(invalid_field(
            field,
            Problem::BadFormat,
            format!("{from:?} isn't a machine id"),
            "name the machine the files came from by its id, like this-mac",
        ));
    }
    Ok(())
}

fn under(prefix: &str, path: &RepoPath, given: &str) -> Result<RepoPath, Status> {
    let joined = match path.as_str() {
        "" => prefix.to_owned(),
        path => format!("{prefix}/{path}"),
    };
    RepoPath::new(joined).map_err(|_| outside(given))
}

fn shown(path: &str) -> String {
    match path {
        "" => "the folder".to_owned(),
        path => path.to_owned(),
    }
}

/// What a location names, on whichever machine, as far as the store knows.
#[derive(Clone)]
pub struct Placed {
    pub place: Place,
    folder: PathBuf,
    path: RepoPath,
    pub shown: String,
    pub location: pb::FileLocation,
    pub writable: bool,
}

impl Placed {
    pub fn machine(&self) -> &str {
        self.place.machine.as_str()
    }

    pub fn is_root(&self) -> bool {
        self.path.as_str().is_empty()
    }

    pub fn require_writable(&self, field: &str) -> Result<(), Status> {
        read_only(self.writable, &self.place.root, field)
    }

    pub fn into_target(self) -> Target {
        Target {
            place: self.place,
            root: self.folder,
            path: self.path,
            shown: self.shown,
            location: self.location,
            writable: self.writable,
        }
    }
}

/// Resolves the agent's on-disk artifact folder for `session` and
/// `conversation_id` (empty means the session's current conversation).
pub fn session_agent_artifacts(
    store: &Store,
    home: &Path,
    session: &str,
    conversation_id: &str,
) -> Result<Option<humata2_conversation::SessionArtifacts>, Status> {
    let Some(agent) = store
        .agent(session)
        .map_err(|err| store_error(err, session, "look up the session's agent"))?
    else {
        return Ok(None);
    };
    let (conv_id, conv_file): (String, PathBuf) = if conversation_id.is_empty()
        || conversation_id == agent.launch.conversation_id
    {
        (
            agent.launch.conversation_id.clone(),
            agent.launch.conversation_file.clone(),
        )
    } else {
        let previous = store
            .previous_conversations(session)
            .map_err(|err| store_error(err, session, "look up previous conversations"))?;
        let Some(found) = previous
            .into_iter()
            .find(|prev| prev.conversation_id == conversation_id)
        else {
            return Ok(None);
        };
        (found.conversation_id, found.path)
    };
    Ok(humata2_conversation::resolve_from_launch(
        &agent.launch.agent,
        &conv_id,
        &conv_file,
        &agent.launch.folder,
        home,
        None,
    ))
}

/// Directories to watch for live file changes in `session`'s current agent
/// artifacts folder (empty if unsupported or missing).
pub fn session_agent_watch_dirs(store: &Store, home: &Path, session: &str) -> Vec<PathBuf> {
    session_agent_artifacts(store, home, session, "")
        .ok()
        .flatten()
        .map(|artifacts| artifacts.watch_dirs)
        .unwrap_or_default()
}

fn read_only(writable: bool, root: &Root, field: &str) -> Result<(), Status> {
    if writable {
        return Ok(());
    }
    let (message, fix) = match root {
        Root::AgentState(_) => (
            "agent session folders are read-only, so nothing in them can be changed here",
            "agent session files are managed by the agent CLI and can only be viewed",
        ),
        _ => (
            "a main clone is the read-only reference new clones start from, so nothing in it can be changed here",
            "make the change in a session's clone, or in the channel's shared folder",
        ),
    };
    Err(invalid_field(field, Problem::NotAllowedHere, message, fix))
}

/// A file or folder in one of this machine's roots.
pub struct Target {
    pub place: Place,
    root: PathBuf,
    path: RepoPath,
    pub shown: String,
    pub location: pb::FileLocation,
    pub writable: bool,
}

impl Target {
    pub fn key(&self) -> String {
        format!("{:?}", self.location)
    }

    pub fn is_root(&self) -> bool {
        self.path.as_str().is_empty()
    }

    pub fn child(&self, name: &str) -> pb::FileLocation {
        let mut location = self.location.clone();
        location.path = match location.path.as_str() {
            "" => name.to_owned(),
            path => format!("{path}/{name}"),
        };
        location
    }

    pub fn resolve(&self, existing: bool) -> Result<PathBuf, Status> {
        if self.is_root() {
            return Err(outside(&self.shown));
        }
        self.checked(existing)
    }

    pub fn existing(&self) -> Result<PathBuf, Status> {
        self.checked(true)
    }

    pub fn unfollowed(&self) -> Result<PathBuf, Status> {
        if self.is_root() {
            return Err(outside(&self.shown));
        }
        let root = self.canonical_root()?;
        let full = self.root.join(self.path.as_str());
        let (Some(parent), Some(name)) = (full.parent(), full.file_name()) else {
            return Err(outside(&self.shown));
        };
        let parent = fs::canonicalize(parent).map_err(|err| self.missing_or(&err, &full))?;
        if !parent.starts_with(&root) {
            return Err(outside(&self.shown));
        }
        let path = parent.join(name);
        fs::symlink_metadata(&path).map_err(|err| self.missing_or(&err, &path))?;
        Ok(path)
    }

    fn canonical_root(&self) -> Result<PathBuf, Status> {
        fs::canonicalize(&self.root).map_err(|err| match err.kind() {
            ErrorKind::NotFound => not_found(
                pb::EntityKind::RootFolder,
                &self.shown,
                format!("there is no folder at {} on this machine", self.root.display()),
                "check the session, channel and repository; the folder may not have been made yet",
            ),
            _ => io_failure(&err, "open", &self.root),
        })
    }

    fn missing_or(&self, err: &io::Error, path: &Path) -> Status {
        match err.kind() {
            ErrorKind::NotFound => not_found(
                pb::EntityKind::File,
                &self.shown,
                format!("there is no {} there", self.shown),
                "check the path; it is relative to the root the location names",
            ),
            _ => io_failure(err, "open", path),
        }
    }

    fn checked(&self, existing: bool) -> Result<PathBuf, Status> {
        let root = self.canonical_root()?;
        if self.is_root() {
            return Ok(root);
        }
        let full = self.root.join(self.path.as_str());
        let checked = if existing {
            fs::canonicalize(&full)
        } else {
            let parent = full.parent().unwrap_or(&self.root);
            if fs::symlink_metadata(&full).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(outside(&self.shown));
            }
            fs::canonicalize(parent).map(|parent| match full.file_name() {
                Some(name) => parent.join(name),
                None => parent,
            })
        };
        let checked = checked.map_err(|err| self.missing_or(&err, &full))?;
        if !checked.starts_with(&root) || checked == root {
            return Err(outside(&self.shown));
        }
        Ok(checked)
    }

    pub fn require_writable(&self, field: &str) -> Result<(), Status> {
        read_only(self.writable, &self.place.root, field)
    }
}

/// The path leaves its root.
pub fn outside(path: &str) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            format!("{path:?} leaves its folder, by \"..\", an absolute path or a symlink"),
            "name a file inside the folder, relative to its root",
            pb::error::Kind::OutsideHumataFolders(pb::OutsideHumataFolders {
                path: path.to_owned(),
            }),
        ),
    )
}

/// A file system failure the caller can't fix from the request.
pub fn io_failure(err: &io::Error, doing: &str, path: &Path) -> Status {
    internal(format!("couldn't {doing} {}: {err}", path.display()), "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_access::{
        Agent, Call, ChannelId, Context, Host, Policy, Principal, Refusal, SessionId, SessionKind,
        SessionRef, Verdict,
    };
    use humata2_store::{AgentLaunch, AgentRecord, AgentState};

    const UUID_1: &str = "11111111-1111-1111-1111-111111111111";
    const UUID_2: &str = "22222222-2222-2222-2222-222222222222";

    fn setup_roots(tmp: &Path) -> (Arc<Store>, PathBuf, Roots) {
        let store = Arc::new(Store::open(&tmp.join("store.sqlite")).unwrap());
        let home = tmp.join("home");
        let channels = tmp.join("channels");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&channels).unwrap();
        let roots = Roots::new(store.clone(), "this-mac".to_owned(), channels, home.clone());
        (store, home, roots)
    }

    fn seed_session(store: &Store, id: &str, folder: &Path) {
        fs::create_dir_all(folder).unwrap();
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: "ch1".to_owned(),
                machine: "this-mac".to_owned(),
                title: "test".to_owned(),
                agent: "claude".to_owned(),
                folder: folder.to_owned(),
                created_unix_ms: 1,
            })
            .unwrap();
    }

    fn seed_agent(store: &Store, session: &str, agent: &str, conv_id: &str, conv_file: &Path, folder: &Path) {
        store
            .put_agent(&AgentRecord {
                session: session.to_owned(),
                tab: format!("tab-{session}"),
                state: AgentState::Running { holder_pid: 1 },
                launch: AgentLaunch {
                    agent: agent.to_owned(),
                    folder: folder.to_owned(),
                    conversation_id: conv_id.to_owned(),
                    conversation_file: conv_file.to_owned(),
                    ..AgentLaunch::default()
                },
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: 1,
            })
            .unwrap();
    }

    fn agent_state_loc(session: &str, conversation_id: &str, path: &str) -> pb::FileLocation {
        pb::FileLocation {
            machine: None,
            root: Some(pb::file_location::Root::SessionAgentState(
                pb::SessionAgentState {
                    session: Some(pb::SessionId {
                        value: session.to_owned(),
                    }),
                    conversation_id: conversation_id.to_owned(),
                },
            )),
            path: path.to_owned(),
        }
    }

    #[test]
    fn resolves_per_provider_fixtures_and_rejects_agy() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, home, roots) = setup_roots(tmp.path());
        let work = tmp.path().join("work");
        seed_session(&store, "s-claude", &work);
        seed_session(&store, "s-codex", &work);
        seed_session(&store, "s-copilot", &work);
        seed_session(&store, "s-agy", &work);

        // Claude with custom config dir derived from conversation_file
        let claude_cfg = tmp.path().join("custom-claude");
        let claude_proj = claude_cfg
            .join("projects")
            .join(humata2_conversation::claude_project_name(&work));
        fs::create_dir_all(&claude_proj).unwrap();
        let claude_file = claude_proj.join(format!("{UUID_1}.jsonl"));
        fs::write(&claude_file, "{}\n").unwrap();
        seed_agent(&store, "s-claude", "claude", UUID_1, &claude_file, &work);
        let t_claude = roots
            .target(Some(agent_state_loc("s-claude", "", "")), "location", Entry::Folder)
            .unwrap();
        assert_eq!(t_claude.existing().unwrap(), fs::canonicalize(&claude_proj).unwrap());
        assert!(!t_claude.writable);

        // Codex rollout dir
        let codex_day = home.join(".codex/sessions/2026/11/21");
        fs::create_dir_all(&codex_day).unwrap();
        let codex_file = codex_day.join(format!("rollout-2026-11-21T08-00-00-{UUID_1}.jsonl"));
        fs::write(&codex_file, "{}\n").unwrap();
        seed_agent(&store, "s-codex", "codex", UUID_1, &codex_file, &work);
        let t_codex = roots
            .target(Some(agent_state_loc("s-codex", "", "")), "location", Entry::Folder)
            .unwrap();
        assert_eq!(t_codex.existing().unwrap(), fs::canonicalize(&codex_day).unwrap());

        // Copilot session-state/<id>/
        let copilot_dir = home.join(".copilot/session-state").join(UUID_1);
        fs::create_dir_all(&copilot_dir).unwrap();
        let copilot_file = copilot_dir.join("events.jsonl");
        fs::write(&copilot_file, "{}\n").unwrap();
        seed_agent(&store, "s-copilot", "copilot", UUID_1, &copilot_file, &work);
        let t_copilot = roots
            .target(Some(agent_state_loc("s-copilot", "", "")), "location", Entry::Folder)
            .unwrap();
        assert_eq!(t_copilot.existing().unwrap(), fs::canonicalize(&copilot_dir).unwrap());

        // agy is unsupported -> NotFound
        seed_agent(&store, "s-agy", "agy", UUID_1, Path::new(""), &work);
        let err = roots
            .target(Some(agent_state_loc("s-agy", "", "")), "location", Entry::Folder)
            .err()
            .unwrap();
        assert_eq!(err.code(), Code::NotFound);
    }

    #[test]
    fn agent_state_root_is_read_only_and_contained() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, home, roots) = setup_roots(tmp.path());
        let work = tmp.path().join("work");
        seed_session(&store, "s1", &work);

        let copilot_dir = home.join(".copilot/session-state").join(UUID_1);
        fs::create_dir_all(&copilot_dir).unwrap();
        let inside_file = copilot_dir.join("events.jsonl");
        fs::write(&inside_file, "hello\n").unwrap();
        seed_agent(&store, "s1", "copilot", UUID_1, &inside_file, &work);

        // Read works
        let target = roots
            .target(
                Some(agent_state_loc("s1", "", "events.jsonl")),
                "location",
                Entry::File,
            )
            .unwrap();
        assert_eq!(target.resolve(true).unwrap(), fs::canonicalize(&inside_file).unwrap());

        // Write is refused (read-only)
        let write_err = target.require_writable("location").unwrap_err();
        assert_eq!(write_err.code(), Code::InvalidArgument);
        assert!(
            write_err.message().contains("agent session folders are read-only"),
            "unexpected message: {}",
            write_err.message()
        );

        // Containment: symlink pointing outside the root is refused with PermissionDenied
        let secret = tmp.path().join("secret.txt");
        fs::write(&secret, "top-secret\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, copilot_dir.join("escape.txt")).unwrap();
            let escape = roots
                .target(
                    Some(agent_state_loc("s1", "", "escape.txt")),
                    "location",
                    Entry::File,
                )
                .unwrap();
            assert_eq!(escape.existing().unwrap_err().code(), Code::PermissionDenied);
            assert_eq!(escape.resolve(true).unwrap_err().code(), Code::PermissionDenied);
        }
    }

    #[test]
    fn previous_conversation_stays_accessible_after_rotation() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, home, roots) = setup_roots(tmp.path());
        let work = tmp.path().join("work");
        seed_session(&store, "s1", &work);

        let old_dir = home.join(".copilot/session-state").join(UUID_1);
        let new_dir = home.join(".copilot/session-state").join(UUID_2);
        fs::create_dir_all(&old_dir).unwrap();
        fs::create_dir_all(&new_dir).unwrap();
        let old_file = old_dir.join("events.jsonl");
        let new_file = new_dir.join("events.jsonl");
        fs::write(&old_file, "old\n").unwrap();
        fs::write(&new_file, "new\n").unwrap();

        store
            .push_previous_conversation("s1", UUID_1, &old_file, 100)
            .unwrap();
        seed_agent(&store, "s1", "copilot", UUID_2, &new_file, &work);

        let current = roots
            .target(Some(agent_state_loc("s1", "", "")), "location", Entry::Folder)
            .unwrap();
        assert_eq!(current.existing().unwrap(), fs::canonicalize(&new_dir).unwrap());

        let prev = roots
            .target(Some(agent_state_loc("s1", UUID_1, "")), "location", Entry::Folder)
            .unwrap();
        assert_eq!(prev.existing().unwrap(), fs::canonicalize(&old_dir).unwrap());
    }

    #[test]
    fn access_policy_allows_app_and_refuses_agent_for_agent_state() {
        let session = SessionRef {
            id: SessionId::new("s1"),
            channel: ChannelId::new("ch1"),
            machine: MachineId::new("this-mac"),
        };
        let call = Call::ReadFiles(Place {
            machine: MachineId::new("this-mac"),
            root: Root::AgentState(session.clone()),
            path: RepoPath::new("events.jsonl").unwrap(),
            entry: Entry::File,
        });
        let mut policy = Policy::new();
        policy.add_channel(ChannelId::new("ch1")).unwrap();
        let context = Context {
            here: MachineId::new("this-mac"),
            host: Host::Here,
        };

        assert!(matches!(
            policy.check(&Principal::App, &call, &context),
            Ok(Verdict::Allowed { .. })
        ));
        let agent = Principal::Agent(Agent {
            session,
            kind: SessionKind::Ordinary,
        });
        assert!(matches!(
            policy.check(&agent, &call, &context),
            Ok(Verdict::Refused(Refusal::OutsideHumataFolders { .. }))
        ));
    }
}

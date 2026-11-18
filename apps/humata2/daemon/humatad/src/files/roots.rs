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
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Layout;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::identity;
use crate::status;

/// The folders a location can name here: the store's sessions and
/// channels, and the folder the channels live in.
pub(crate) struct Roots {
    store: Arc<Store>,
    machine: String,
    layout: Layout,
    /// The user's home folder on this machine (FM-10).
    home: PathBuf,
}

impl Roots {
    /// `channels` is the folder every channel's folder is in; `home` is
    /// the user's home folder here.
    pub(crate) fn new(
        store: Arc<Store>,
        machine: String,
        channels: impl Into<Layout>,
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
    pub(crate) fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// This machine's id.
    pub(crate) fn machine(&self) -> &str {
        &self.machine
    }

    /// The main clone a path lands in, on or above, if any (FC-2, D165):
    /// `path` under `folder`, with symlinks followed in the part that
    /// exists. A main clone's own folder and everything in it always count.
    /// A folder that holds a main clone (`main/`, a channel's folder,
    /// Humata's root) counts only when the entry itself is acted on
    /// (`Entry::File`: a rename, a delete, a move away), not when it is
    /// listed or something is made in it. A location on another machine is
    /// judged there.
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
            .map_err(|error| status::store(error, &self.machine, "look up the channels"))?;
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
    /// judges: [`Roots::place`], then the disk. `field` names the location
    /// in the request, for errors ("location", "destination", "sources[0]").
    /// A file (`Entry::File`) needs a path; a folder may be the root itself,
    /// with an empty path. A location on another machine is refused: the
    /// FileService handlers serve only this machine's own files.
    pub(crate) fn target(
        &self,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
    ) -> Result<Target, Status> {
        let placed = self.place(location, field, entry)?;
        if placed.machine() != self.machine {
            return Err(status::invalid_field(
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
    /// policy judges, without looking at any disk: the session or channel is
    /// found in this daemon's store, which holds the host's record of every
    /// machine's (D139). A session says which machine it is on (a location
    /// that names another is refused); a channel's folders are on the machine
    /// the location names, this one when it names none. The location comes
    /// back with its machine filled in. A session's pastes folder is made
    /// when the session is this machine's (D335).
    pub(crate) fn place(
        &self,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
    ) -> Result<Placed, Status> {
        use pb::file_location::Root as R;

        let location = location.ok_or_else(|| status::missing(field))?;
        // The machine the location names; none or empty is this one.
        let given = location
            .machine
            .as_ref()
            .map(|machine| machine.value.clone())
            .filter(|machine| !machine.is_empty());
        let named = given.clone().unwrap_or_else(|| self.machine.clone());
        if location.path.is_empty() && entry == Entry::File {
            return Err(status::missing(&format!("{field}.path")));
        }
        let path = RepoPath::new(location.path.as_str()).map_err(|_| outside(&location.path))?;
        let here = MachineId::new(named.as_str());
        let (root, folder, policy_path, writable, normal) = match location.root {
            Some(R::SessionClone(clone)) => {
                let id = clone
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| status::missing(&format!("{field}.session_clone.session")))?;
                let record = self.session_record(&id)?;
                let repo = repo_of(clone.repo, &format!("{field}.session_clone.repo"))?;
                let policy_path = under(&format!("{}/{}", repo.owner, repo.name), &path, &location.path)?;
                let session = identity::session_ref(&record, &self.machine);
                let folder = record.folder.join(&repo.name);
                let place_machine = session.machine.clone();
                let session_machine = place_machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(status::invalid_field(
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
            // D382: a session's own folder, where a Desk session's
            // documents sit. The policy judges it as that session's clone
            // (its owner, its parent, and whoever its grants cover), with
            // the path as given; a session with no repository has no
            // other path in it.
            Some(R::SessionFolder(own)) => {
                let id = own
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| status::missing(&format!("{field}.session_folder.session")))?;
                let record = self.session_record(&id)?;
                let session = identity::session_ref(&record, &self.machine);
                let place_machine = session.machine.clone();
                let session_machine = place_machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(status::invalid_field(
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
                    .ok_or_else(|| status::missing(&format!("{field}.session_pastes.session")))?;
                let record = self.session_record(&id)?;
                let session = identity::session_ref(&record, &self.machine);
                let session_machine = session.machine.as_str().to_owned();
                if given.as_ref().is_some_and(|given| *given != session_machine) {
                    return Err(status::invalid_field(
                        &format!("{field}.machine"),
                        Problem::NotAllowedHere,
                        format!("session {id} is on machine {session_machine}, not {named}"),
                        "leave the machine out, or name the session's own",
                    ));
                }
                // Another machine's pastes are its own daemon's to make.
                let folder = match session_machine == self.machine {
                    true => self.store.paste_folder(&id).map_err(|error| {
                        status::store(error, &self.machine, "make the session's paste folder")
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
            Some(R::ChannelShared(shared)) => {
                let channel = self.channel(shared.channel, &format!("{field}.channel_shared.channel"))?;
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
                let channel = self.channel(whole.channel, &format!("{field}.channel_folder.channel"))?;
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
                    return Err(status::invalid_field(
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
                    // The read-only reference new clones start from (FC-2).
                    false,
                    R::MainClone(pb::MainClone {
                        channel: Some(pb::ChannelId { value: channel.id }),
                        repo: Some(repo),
                    }),
                )
            }
            // Only the user gets past the policy with this root.
            Some(R::Home(_)) => (
                Root::Home,
                self.home.clone(),
                path.clone(),
                true,
                R::Home(pb::MachineHome {}),
            ),
            // PL-4: where files from another machine land here. Only the
            // user gets past the policy with this root.
            Some(R::Received(received)) => {
                let from = received
                    .from
                    .map(|machine| machine.value)
                    .ok_or_else(|| status::missing(&format!("{field}.received.from")))?;
                check_machine_name(&from, &format!("{field}.received.from"))?;
                let folder = self.layout.root().join("received").join(&from);
                // Made when it is this machine's and a place is asked for,
                // so a copy can land in it and a listing shows it empty;
                // another machine's is its own daemon's to make.
                if named == self.machine {
                    fs::create_dir_all(&folder).map_err(|error| io_failure(&error, "make", &folder))?;
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
            None => return Err(status::missing(&format!("{field}.root"))),
        };
        // FC-2, D165: a change is judged by where its path lands. Through
        // the home folder or a channel's folder a path can reach a main
        // clone; to the policy it then is that main clone, whichever root
        // named it, so every caller is refused a change there. Reads and
        // copies out are judged as before.
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

    /// The session `id` as this daemon's store has it.
    fn session_record(&self, id: &str) -> Result<SessionRecord, Status> {
        match self.store.session(id) {
            Ok(Some(record)) => Ok(record),
            Ok(None) => Err(status::not_found(
                pb::EntityKind::Session,
                id,
                format!("no session {id} on this machine"),
                "check the session id; sessions on other machines can't be named here yet",
            )),
            Err(error) => Err(status::store(error, &self.machine, "look up the session")),
        }
    }

    fn channel(&self, id: Option<pb::ChannelId>, field: &str) -> Result<ChannelRecord, Status> {
        let id = id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing(field))?;
        // A Desk session moves through the channels folder itself, which no
        // agent's channel is (MV-17).
        if id == humata2_sessions::CHANNELS_ROOT {
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
            Ok(None) => Err(status::not_found(
                pb::EntityKind::Channel,
                &id,
                format!("no channel {id} on this machine"),
                "check the channel id; `humata channels` lists the channels here",
            )),
            Err(error) => Err(status::store(error, &self.machine, "look up the channel")),
        }
    }
}

/// Where `path` lands on disk: symlinks are followed in the part of it
/// that exists, and what doesn't exist yet follows as named.
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

/// The repository a location names, checked to be one folder name.
fn repo_of(repo: Option<pb::RepoRef>, field: &str) -> Result<pb::RepoRef, Status> {
    let repo = repo
        .filter(|repo| !repo.owner.is_empty() && !repo.name.is_empty())
        .ok_or_else(|| status::missing(field))?;
    if repo.name.contains('/') || repo.name == "." || repo.name == ".." {
        return Err(status::invalid_field(
            &format!("{field}.name"),
            Problem::BadFormat,
            format!("{:?} isn't a repository name", repo.name),
            "give the repository's name on GitHub, like api",
        ));
    }
    Ok(repo)
}

/// `from` checked to be one machine id, which names a folder under
/// `received/`.
fn check_machine_name(from: &str, field: &str) -> Result<(), Status> {
    if from.is_empty() || from.contains(['/', '\\']) || from == "." || from == ".." {
        return Err(status::invalid_field(
            field,
            Problem::BadFormat,
            format!("{from:?} isn't a machine id"),
            "name the machine the files came from by its id, like this-mac",
        ));
    }
    Ok(())
}

/// `path` under `prefix`, as the policy sees a path in a session's clones.
fn under(prefix: &str, path: &RepoPath, given: &str) -> Result<RepoPath, Status> {
    let joined = match path.as_str() {
        "" => prefix.to_owned(),
        path => format!("{prefix}/{path}"),
    };
    RepoPath::new(joined).map_err(|_| outside(given))
}

/// A path for messages; the root itself has none to show.
fn shown(path: &str) -> String {
    match path {
        "" => "the folder".to_owned(),
        path => path.to_owned(),
    }
}

/// What a location names, on whichever machine, as far as the store knows
/// (see [`Roots::place`]): no disk was looked at.
#[derive(Clone)]
pub(crate) struct Placed {
    /// What the policy judges.
    pub(crate) place: Place,
    /// The root's folder on disk if the entry is on this machine.
    folder: PathBuf,
    path: RepoPath,
    /// The path as the caller gave it, for messages.
    pub(crate) shown: String,
    /// The location with its machine filled in.
    pub(crate) location: pb::FileLocation,
    /// False in a main clone, the read-only reference (FC-2).
    pub(crate) writable: bool,
}

impl Placed {
    /// The machine the entry is on.
    pub(crate) fn machine(&self) -> &str {
        self.place.machine.as_str()
    }

    /// Whether this is the root itself, not something in it.
    pub(crate) fn is_root(&self) -> bool {
        self.path.as_str().is_empty()
    }

    /// Refuses a change in a main clone (FC-2).
    pub(crate) fn require_writable(&self, field: &str) -> Result<(), Status> {
        read_only(self.writable, field)
    }

    /// The entry to resolve on disk; only for one on this machine.
    pub(crate) fn into_target(self) -> Target {
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

fn read_only(writable: bool, field: &str) -> Result<(), Status> {
    if writable {
        return Ok(());
    }
    Err(status::invalid_field(
        field,
        Problem::NotAllowedHere,
        "a main clone is the read-only reference new clones start from, so nothing in it can be changed here",
        "make the change in a session's clone, or in the channel's shared folder",
    ))
}

/// A file or folder in one of this machine's roots.
pub(crate) struct Target {
    /// What the policy judges.
    pub(crate) place: Place,
    /// The root's folder on disk.
    root: PathBuf,
    path: RepoPath,
    /// The path as the caller gave it, for messages.
    pub(crate) shown: String,
    /// The location with this machine filled in.
    pub(crate) location: pb::FileLocation,
    /// False in a main clone, the read-only reference (FC-2).
    pub(crate) writable: bool,
}

impl Target {
    /// Names the entry for idempotency keys.
    pub(crate) fn key(&self) -> String {
        format!("{:?}", self.location)
    }

    /// Whether this is the root itself, not something in it.
    pub(crate) fn is_root(&self) -> bool {
        self.path.as_str().is_empty()
    }

    /// The location of `name` in this folder.
    pub(crate) fn child(&self, name: &str) -> pb::FileLocation {
        let mut location = self.location.clone();
        location.path = match location.path.as_str() {
            "" => name.to_owned(),
            path => format!("{path}/{name}"),
        };
        location
    }

    /// The path on disk of a file in the root, checked to stay in it once
    /// symlinks are followed; the root itself is refused. `existing` is
    /// false for an entry about to be made: then only its folder must
    /// exist, and a symlink already at the name is refused.
    pub(crate) fn resolve(&self, existing: bool) -> Result<PathBuf, Status> {
        if self.is_root() {
            return Err(outside(&self.shown));
        }
        self.checked(existing)
    }

    /// The path on disk of an existing entry, which may be the root.
    pub(crate) fn existing(&self) -> Result<PathBuf, Status> {
        self.checked(true)
    }

    /// The path on disk of an existing entry itself, for renaming and
    /// deleting: its folder is checked to stay in the root, and a symlink
    /// at the name is the entry, not what it points to. The root itself is
    /// refused.
    pub(crate) fn unfollowed(&self) -> Result<PathBuf, Status> {
        if self.is_root() {
            return Err(outside(&self.shown));
        }
        let root = self.canonical_root()?;
        let full = self.root.join(self.path.as_str());
        let (Some(parent), Some(name)) = (full.parent(), full.file_name()) else {
            return Err(outside(&self.shown));
        };
        let parent = fs::canonicalize(parent).map_err(|error| self.missing_or(&error, &full))?;
        if !parent.starts_with(&root) {
            return Err(outside(&self.shown));
        }
        let path = parent.join(name);
        fs::symlink_metadata(&path).map_err(|error| self.missing_or(&error, &path))?;
        Ok(path)
    }

    /// The root's folder with symlinks followed.
    fn canonical_root(&self) -> Result<PathBuf, Status> {
        fs::canonicalize(&self.root).map_err(|error| match error.kind() {
            ErrorKind::NotFound => status::not_found(
                pb::EntityKind::File,
                &self.shown,
                format!("there is no folder at {} on this machine", self.root.display()),
                "check the session, channel and repository; the folder may not have been made yet",
            ),
            _ => io_failure(&error, "open", &self.root),
        })
    }

    /// Not found for a missing entry, an internal failure otherwise.
    fn missing_or(&self, error: &io::Error, path: &Path) -> Status {
        match error.kind() {
            ErrorKind::NotFound => status::not_found(
                pb::EntityKind::File,
                &self.shown,
                format!("there is no {} there", self.shown),
                "check the path; it is relative to the root the location names",
            ),
            _ => io_failure(error, "open", path),
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
        let checked = checked.map_err(|error| self.missing_or(&error, &full))?;
        if !checked.starts_with(&root) || checked == root {
            return Err(outside(&self.shown));
        }
        Ok(checked)
    }

    /// Refuses a change in a main clone (FC-2).
    pub(crate) fn require_writable(&self, field: &str) -> Result<(), Status> {
        read_only(self.writable, field)
    }
}

/// The path leaves its root.
pub(crate) fn outside(path: &str) -> Status {
    status::failure(
        Code::PermissionDenied,
        status::error(
            format!("{path:?} leaves its folder, by \"..\", an absolute path or a symlink"),
            "name a file inside the folder, relative to its root",
            pb::error::Kind::OutsideHumataFolders(pb::OutsideHumataFolders {
                path: path.to_owned(),
            }),
        ),
    )
}

/// A file system failure the caller can't fix from the request.
pub(crate) fn io_failure(error: &io::Error, doing: &str, path: &Path) -> Status {
    status::internal(format!("couldn't {doing} {}: {error}", path.display()), "")
}

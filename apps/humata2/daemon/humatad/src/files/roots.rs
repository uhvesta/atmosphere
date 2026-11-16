//! What a `FileLocation` names on this machine (FC-1, FM-10): a session's
//! clone, a channel's folder, its `shared/`, or a main clone, and a path
//! inside it. A path never leaves its root: not by "..", not by an
//! absolute path, and not through a symlink.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_access::{ChannelId, Entry, MachineId, Place, RepoPath, RepoRef, Root};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Layout;
use humata2_store::{ChannelRecord, Store};
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
}

impl Roots {
    /// `channels` is the folder every channel's folder is in.
    pub(crate) fn new(store: Arc<Store>, machine: String, channels: PathBuf) -> Self {
        Self {
            store,
            machine,
            layout: Layout { root: channels },
        }
    }

    /// This machine's id.
    pub(crate) fn machine(&self) -> &str {
        &self.machine
    }

    /// The entry `location` names and the place the policy judges. `field`
    /// names the location in the request, for errors ("location",
    /// "destination", "sources[0]"). A file (`Entry::File`) needs a path;
    /// a folder may be the root itself, with an empty path.
    pub(crate) fn target(
        &self,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
    ) -> Result<Target, Status> {
        use pb::file_location::Root as R;

        let location = location.ok_or_else(|| status::missing(field))?;
        if let Some(machine) = location
            .machine
            .as_ref()
            .filter(|machine| !machine.value.is_empty())
            && machine.value != self.machine
        {
            return Err(status::invalid_field(
                &format!("{field}.machine"),
                Problem::NotAllowedHere,
                format!(
                    "this is machine {}, and files on machine {} aren't relayed through it yet",
                    self.machine, machine.value
                ),
                "ask the daemon on that machine, or leave the machine out for this one",
            ));
        }
        if location.path.is_empty() && entry == Entry::File {
            return Err(status::missing(&format!("{field}.path")));
        }
        let path = RepoPath::new(location.path.as_str()).map_err(|_| outside(&location.path))?;
        let here = MachineId::new(self.machine.as_str());
        let (root, folder, policy_path, writable, normal) = match location.root {
            Some(R::SessionClone(clone)) => {
                let id = clone
                    .session
                    .map(|session| session.value)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| status::missing(&format!("{field}.session_clone.session")))?;
                let record = match self.store.session(&id) {
                    Ok(Some(record)) => record,
                    Ok(None) => {
                        return Err(status::not_found(
                            pb::EntityKind::Session,
                            &id,
                            format!("no session {id} on this machine"),
                            "check the session id; sessions on other machines can't be named here yet",
                        ));
                    }
                    Err(error) => {
                        return Err(status::store(error, &self.machine, "look up the session"));
                    }
                };
                let repo = repo_of(clone.repo, &format!("{field}.session_clone.repo"))?;
                let policy_path = under(&format!("{}/{}", repo.owner, repo.name), &path, &location.path)?;
                let session = identity::session_ref(&record, &self.machine);
                let folder = record.folder.join(&repo.name);
                let place_machine = session.machine.clone();
                let normal = R::SessionClone(pb::SessionClone {
                    session: Some(pb::SessionId { value: id }),
                    repo: Some(repo),
                });
                return Ok(Target {
                    place: Place {
                        machine: place_machine,
                        root: Root::Clone(session),
                        path: policy_path,
                        entry,
                    },
                    root: folder,
                    path,
                    shown: shown(&location.path),
                    location: pb::FileLocation {
                        machine: Some(pb::MachineId {
                            value: self.machine.clone(),
                        }),
                        root: Some(normal),
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
            None => return Err(status::missing(&format!("{field}.root"))),
        };
        Ok(Target {
            place: Place {
                machine: here,
                root,
                path: policy_path,
                entry,
            },
            root: folder,
            path,
            shown: shown(&location.path),
            location: pb::FileLocation {
                machine: Some(pb::MachineId {
                    value: self.machine.clone(),
                }),
                root: Some(normal),
                path: location.path,
            },
            writable,
        })
    }

    fn channel(&self, id: Option<pb::ChannelId>, field: &str) -> Result<ChannelRecord, Status> {
        let id = id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing(field))?;
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
        if self.writable {
            return Ok(());
        }
        Err(status::invalid_field(
            field,
            Problem::NotAllowedHere,
            "a main clone is the read-only reference new clones start from, so nothing in it can be changed here",
            "make the change in a session's clone, or in the channel's shared folder",
        ))
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

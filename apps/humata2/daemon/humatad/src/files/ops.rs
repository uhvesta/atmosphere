//! The file manager's own operations on this machine's folders: list a
//! folder, stat an entry, make a folder, rename, move to the trash (FM-2,
//! FM-3, FM-10). Each one resolves its locations through [`Roots`], so none
//! can leave its root (FC-1) or change a main clone (FC-2).
//!
//! Folder reads and every change run on a blocking thread.

use std::fs::{self, Metadata};
use std::io::ErrorKind;
use std::sync::Arc;

use humata2_access::{Call, Entry};
use humata2_grpc::humata::v1 as pb;
use humata2_transfers::NewItem;
use pb::invalid_field::Problem;
use tonic::{Code, Request, Response, Status};

use super::roots::{Roots, Target, io_failure};
use crate::grant_target;
use crate::guard::Guard;
use crate::identity;
use crate::status;
use crate::trash_service::{Answers, Bin, answer_key, blocking, caller_data, now_ms};

/// Entries per page when the caller names no size.
const PAGE_DEFAULT: usize = 500;

/// The most entries in one page.
const PAGE_MAX: usize = 5000;

pub(in crate::files) struct Ops {
    roots: Arc<Roots>,
    guard: Arc<Guard>,
    bin: Arc<Bin>,
    made: Answers<pb::FileEntry>,
    renamed: Answers<pb::FileEntry>,
    trashed: Answers<Vec<pb::TrashItemId>>,
}

impl Ops {
    pub(in crate::files) fn new(roots: Arc<Roots>, guard: Arc<Guard>, bin: Arc<Bin>) -> Self {
        Self {
            roots,
            guard,
            bin,
            made: Answers::new(),
            renamed: Answers::new(),
            trashed: Answers::new(),
        }
    }

    /// The entries of a folder: folders first, then names without regard to
    /// case; transfer leftovers are hidden. A page continues after the last
    /// entry of the one before, so a folder that changes between pages loses
    /// or repeats nothing that stayed.
    pub(in crate::files) async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let target = self
            .roots
            .target(request.location, "location", Entry::Folder)?;
        self.guard
            .require(&caller, &Call::ReadFiles(target.place.clone()))?;
        let (size, token) = match request.page {
            Some(page) => (page.page_size as usize, page.page_token),
            None => (0, String::new()),
        };
        let size = match size {
            0 => PAGE_DEFAULT,
            size => size.min(PAGE_MAX),
        };
        let response = blocking(move || list(&target, size, &token)).await??;
        Ok(Response::new(response))
    }

    /// The entry itself; the root of a folder is allowed.
    pub(in crate::files) async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let target = self
            .roots
            .target(request.location, "location", Entry::Folder)?;
        self.guard
            .require(&caller, &Call::ReadFiles(target.place.clone()))?;
        let entry = blocking(move || {
            let path = target.existing()?;
            let meta = fs::metadata(&path).map_err(|error| found(&error, &target, &path))?;
            Ok::<_, Status>(entry_of(
                target.location.clone(),
                &meta,
                target.writable,
            ))
        })
        .await??;
        Ok(Response::new(entry))
    }

    /// Makes one folder whose parent exists. A name that is taken is
    /// refused, a folder included: the person asking for a new folder is
    /// told nothing was made. A retry is safe through its idempotency key,
    /// which gets the first answer back.
    pub(in crate::files) async fn create_folder(
        &self,
        request: Request<pb::CreateFolderRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let target = self
            .roots
            .target(request.location, "location", Entry::Folder)?;
        self.guard
            .require(&caller, &Call::WriteFiles(target.place.clone()))?;
        target.require_writable("location")?;
        let key = answer_key(
            "create_folder",
            &caller,
            request.idempotency_key.as_ref(),
            &target.key(),
        );
        if let Some(entry) = key.as_deref().and_then(|key| self.made.get(key)) {
            return Ok(Response::new(entry));
        }
        let entry = blocking(move || make_folder(&target)).await??;
        if let Some(key) = key {
            self.made.put(key, entry.clone());
        }
        Ok(Response::new(entry))
    }

    /// Gives an entry another name in the same folder. Never replaces
    /// something that has the name.
    pub(in crate::files) async fn rename(
        &self,
        request: Request<pb::RenameRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let new_name = request.new_name;
        let target = self
            .roots
            .target(request.location.clone(), "location", entry_for(&request.location))?;
        self.guard
            .require(&caller, &Call::WriteFiles(target.place.clone()))?;
        target.require_writable("location")?;
        not_the_root(&target, "location")?;
        valid_name(&new_name)?;
        let key = answer_key(
            "rename",
            &caller,
            request.idempotency_key.as_ref(),
            &format!("{}\0{new_name}", target.key()),
        );
        if let Some(entry) = key.as_deref().and_then(|key| self.renamed.get(key)) {
            return Ok(Response::new(entry));
        }
        let entry = blocking(move || rename(&target, &new_name)).await??;
        if let Some(key) = key {
            self.renamed.put(key, entry.clone());
        }
        Ok(Response::new(entry))
    }

    /// Moves each entry to this machine's trash (FM-3). Every location is
    /// resolved and judged before the first is moved; if a move then fails,
    /// the ones already moved are put back.
    pub(in crate::files) async fn move_to_trash(
        &self,
        request: Request<pb::MoveToTrashRequest>,
    ) -> Result<Response<pb::MoveToTrashResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        if request.locations.is_empty() {
            return Err(status::missing("locations"));
        }
        let mut targets = Vec::with_capacity(request.locations.len());
        for (at, location) in request.locations.into_iter().enumerate() {
            let field = format!("locations[{at}]");
            let entry = entry_for(&Some(location.clone()));
            let target = self.roots.target(Some(location), &field, entry)?;
            self.guard
                .require(&caller, &Call::WriteFiles(target.place.clone()))?;
            target.require_writable(&field)?;
            not_the_root(&target, &field)?;
            targets.push((field, target));
        }
        let key = answer_key(
            "move_to_trash",
            &caller,
            request.idempotency_key.as_ref(),
            &targets
                .iter()
                .map(|(_, target)| target.key())
                .collect::<Vec<_>>()
                .join("\0"),
        );
        if let Some(items) = key.as_deref().and_then(|key| self.trashed.get(key)) {
            return Ok(Response::new(pb::MoveToTrashResponse { items }));
        }
        let deleted_by = identity::actor(&caller);
        let bin = self.bin.clone();
        let items = blocking(move || {
            // Resolved all before the first moves.
            let mut found = Vec::with_capacity(targets.len());
            for (field, target) in &targets {
                // The entry itself: a link goes to the trash, not what it points to.
                let _ = field;
                let path = target.unfollowed()?;
                found.push((target, path));
            }
            let trash = bin.get()?;
            let mut moved: Vec<String> = Vec::with_capacity(found.len());
            for (target, path) in found {
                let item = NewItem {
                    description: target.shown.clone(),
                    caller_data: caller_data(&deleted_by, &was(target)),
                    now_unix_ms: now_ms(),
                };
                match trash.put(&path, item) {
                    Ok(item) => moved.push(item.id),
                    Err(error) => {
                        for id in &moved {
                            let _ = trash.restore(id);
                        }
                        return Err(status::internal(
                            format!(
                                "couldn't move {} to the trash: {}. Anything moved before it was put back",
                                target.shown,
                                status::chain(&error)
                            ),
                            "",
                        ));
                    }
                }
            }
            Ok(moved
                .into_iter()
                .map(|value| pb::TrashItemId { value })
                .collect::<Vec<_>>())
        })
        .await??;
        if let Some(key) = key {
            self.trashed.put(key, items.clone());
        }
        Ok(Response::new(pb::MoveToTrashResponse { items }))
    }
}

/// What a location is judged as: the root of a folder when it has no path,
/// otherwise an entry in it.
fn entry_for(location: &Option<pb::FileLocation>) -> Entry {
    match location {
        Some(location) if location.path.is_empty() => Entry::Folder,
        _ => Entry::File,
    }
}

fn not_the_root(target: &Target, field: &str) -> Result<(), Status> {
    if !target.is_root() {
        return Ok(());
    }
    Err(status::invalid_field(
        field,
        Problem::NotAllowedHere,
        "that is the root of a folder Humata keeps, which can't be renamed or deleted",
        "name something inside it",
    ))
}

/// What the trash records an entry was: the session or channel it was in.
fn was(target: &Target) -> pb::EntityRef {
    use pb::entity_ref::Entity;
    use pb::file_location::Root;
    let entity = match target.location.root.as_ref() {
        Some(Root::SessionClone(clone)) => clone.session.clone().map(Entity::Session),
        Some(Root::ChannelShared(shared)) => shared.channel.clone().map(Entity::Channel),
        Some(Root::ChannelFolder(folder)) => folder.channel.clone().map(Entity::Channel),
        Some(Root::MainClone(main)) => main.channel.clone().map(Entity::Channel),
        // The home folder belongs to no session or channel.
        Some(Root::Home(_)) => None,
        None => None,
    };
    pb::EntityRef { entity }
}

/// A name a file can be renamed to: one plain name, not one the transfers
/// keep for themselves.
fn valid_name(name: &str) -> Result<(), Status> {
    let bad = |message: String| {
        status::invalid_field(
            "new_name",
            Problem::BadFormat,
            message,
            "give one name, like notes.md, with no \"/\"; to move it elsewhere, use a transfer",
        )
    };
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(bad(format!("{name:?} isn't a name for a file or folder")));
    }
    if hidden(name) {
        return Err(bad(format!(
            "{name:?} is a name Humata keeps for transfers in progress"
        )));
    }
    Ok(())
}

/// Names of transfer leftovers, which listings skip.
fn hidden(name: &str) -> bool {
    name.starts_with(".humata-transfer-")
        || (name.starts_with('.') && name.contains(".humata-partial-"))
}

fn name_taken(shown: &str, name: &str) -> Status {
    status::failure(
        Code::AlreadyExists,
        status::error(
            format!("{shown} already exists"),
            "pick another name, or rename or move what is there first",
            pb::error::Kind::NameTaken(pb::NameTaken {
                kind: pb::EntityKind::File as i32,
                name: name.to_owned(),
                suggestion: String::new(),
            }),
        ),
    )
}

/// A missing path is NotFound; anything else is a failure to report.
fn found(error: &std::io::Error, target: &Target, path: &std::path::Path) -> Status {
    match error.kind() {
        ErrorKind::NotFound => status::not_found(
            pb::EntityKind::File,
            &target.shown,
            format!("there is no {} there", target.shown),
            "check the path; it is relative to the root the location names",
        ),
        _ => io_failure(error, "read", path),
    }
}

fn entry_of(location: pb::FileLocation, meta: &Metadata, writable: bool) -> pb::FileEntry {
    let kind = if meta.file_type().is_symlink() {
        pb::file_entry::Kind::Symlink
    } else if meta.is_dir() {
        pb::file_entry::Kind::Folder
    } else {
        pb::file_entry::Kind::File
    };
    pb::FileEntry {
        location: Some(location),
        kind: kind as i32,
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified_at: meta.modified().ok().map(grant_target::timestamp),
        changed: false,
        writable,
    }
}

/// Where an entry stands in a listing: folders first, then names without
/// regard to case, then as written.
type Order = (bool, String, String);

fn order(folder: bool, name: &str) -> Order {
    (!folder, name.to_lowercase(), name.to_owned())
}

/// A listing position as a page token.
fn token_of(order: &Order) -> String {
    format!("{}:{}", u8::from(order.0), order.2)
}

fn order_of(token: &str) -> Option<Order> {
    let (folder, name) = token.split_once(':')?;
    Some((folder == "1", name.to_lowercase(), name.to_owned()))
}

fn list(target: &Target, size: usize, token: &str) -> Result<pb::ListDirectoryResponse, Status> {
    let path = target.existing()?;
    let read = fs::read_dir(&path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => found(&error, target, &path),
        ErrorKind::NotADirectory => status::invalid_field(
            "location.path",
            Problem::NotAllowedHere,
            format!("{} is a file, not a folder", target.shown),
            "list the folder that holds it, or read the file",
        ),
        _ => io_failure(&error, "read the folder", &path),
    })?;
    // Names and whether each is a real folder (links aren't followed), from
    // the folder's own listing without a stat of every entry.
    let mut names: Vec<(Order, String)> = Vec::new();
    for entry in read {
        let entry = entry.map_err(|error| io_failure(&error, "read the folder", &path))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if hidden(&name) {
            continue;
        }
        let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        names.push((order(folder, &name), name));
    }
    names.sort();
    let after = match token {
        "" => None,
        token => Some(order_of(token).ok_or_else(|| {
            status::invalid_field(
                "page.page_token",
                Problem::BadFormat,
                "the page token isn't one this daemon gave",
                "start again without a page token",
            )
        })?),
    };
    let mut rest = names
        .iter()
        .filter(|(order, _)| after.as_ref().is_none_or(|after| order > after));
    let page: Vec<&(Order, String)> = rest.by_ref().take(size).collect();
    let next_page_token = match (rest.next(), page.last()) {
        (Some(_), Some((order, _))) => token_of(order),
        _ => String::new(),
    };
    let mut entries = Vec::with_capacity(page.len());
    for (_, name) in page {
        let full = path.join(name);
        // Gone since the listing: left out.
        let Ok(meta) = fs::symlink_metadata(&full) else {
            continue;
        };
        entries.push(entry_of(target.child(name), &meta, target.writable));
    }
    Ok(pb::ListDirectoryResponse {
        entries,
        page: Some(pb::PageResponse { next_page_token }),
    })
}

fn make_folder(target: &Target) -> Result<pb::FileEntry, Status> {
    if target.is_root() {
        let path = target.existing()?;
        let meta = fs::metadata(&path).map_err(|error| found(&error, target, &path))?;
        return Ok(entry_of(target.location.clone(), &meta, target.writable));
    }
    let path = target.resolve(false)?;
    match fs::symlink_metadata(&path) {
        Ok(_) => return Err(name_taken(&target.shown, &target.shown)),
        Err(error) if error.kind() == ErrorKind::NotFound => match fs::create_dir(&path) {
            Ok(()) => {}
            // Made by someone else a moment ago: the name is taken.
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return Err(name_taken(&target.shown, &target.shown));
            }
            Err(error) => return Err(io_failure(&error, "create the folder", &path)),
        },
        Err(error) => return Err(io_failure(&error, "look at", &path)),
    }
    let meta = fs::symlink_metadata(&path).map_err(|error| found(&error, target, &path))?;
    Ok(entry_of(target.location.clone(), &meta, target.writable))
}

/// Renames without following a link at the old name, and refuses a name in
/// use. The check and the rename are two steps, so something made at the
/// new name between them would be replaced; nothing in Humata makes entries
/// there but the caller's own writes.
fn rename(target: &Target, new_name: &str) -> Result<pb::FileEntry, Status> {
    let from = target.unfollowed()?;
    let to = from.with_file_name(new_name);
    if fs::symlink_metadata(&to).is_ok() {
        return Err(name_taken(new_name, new_name));
    }
    fs::rename(&from, &to).map_err(|error| io_failure(&error, "rename", &from))?;
    let meta = fs::symlink_metadata(&to).map_err(|error| io_failure(&error, "read", &to))?;
    let mut location = target.location.clone();
    location.path = match target.location.path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{new_name}"),
        None => new_name.to_owned(),
    };
    Ok(entry_of(location, &meta, target.writable))
}

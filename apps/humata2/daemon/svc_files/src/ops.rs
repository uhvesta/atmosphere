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
use crate::pb;
use pb::invalid_field::Problem;
use tonic::{Code, Request, Response, Status};

use crate::ports::{
    Answers, FileGuardPort, FileTrashPort, actor, answer_key, blocking, caller, caller_data, error,
    failure, invalid_field, missing, not_found, timestamp,
};
use crate::roots::{Roots, Target, io_failure};
use crate::{Reads, held_read};

/// Entries per page when the caller names no size.
const PAGE_DEFAULT: usize = 500;

/// The most entries in one page.
const PAGE_MAX: usize = 5000;

pub(crate) struct Ops {
    roots: Arc<Roots>,
    guard: Arc<dyn FileGuardPort>,
    reads: Reads,
    bin: Arc<dyn FileTrashPort>,
    made: Answers<pb::FileEntry>,
    renamed: Answers<pb::FileEntry>,
    trashed: Answers<Vec<pb::TrashItemId>>,
}

impl Ops {
    pub(crate) fn new(
        roots: Arc<Roots>,
        guard: Arc<dyn FileGuardPort>,
        bin: Arc<dyn FileTrashPort>,
        reads: Reads,
    ) -> Self {
        Self {
            roots,
            guard,
            reads,
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
    pub(crate) async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        let location = request.location.clone();
        let target = self
            .roots
            .target(request.location, "location", Entry::Folder)?;
        let (size, token) = match request.page {
            Some(page) => (page.page_size as usize, page.page_token),
            None => (0, String::new()),
        };
        let size = match size {
            0 => PAGE_DEFAULT,
            size => size.min(PAGE_MAX),
        };
        let (roots, held_token) = (self.roots.clone(), token.clone());
        self.reads
            .require(&caller, target.place.clone(), "list", move || {
                held_read("humata.v1.ListDirectoryResponse", async move {
                    let target = roots.target(location, "location", Entry::Folder)?;
                    blocking(move || list(&target, size, &held_token)).await?
                })
            })
            .await?;
        let response = blocking(move || list(&target, size, &token)).await??;
        Ok(Response::new(response))
    }

    /// The entry itself; the root of a folder is allowed.
    pub(crate) async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        let location = request.location.clone();
        let target = self
            .roots
            .target(request.location, "location", Entry::Folder)?;
        let roots = self.roots.clone();
        self.reads
            .require(&caller, target.place.clone(), "stat", move || {
                held_read("humata.v1.FileEntry", async move {
                    let target = roots.target(location, "location", Entry::Folder)?;
                    blocking(move || stat_of(&target)).await?
                })
            })
            .await?;
        let entry = blocking(move || stat_of(&target)).await??;
        Ok(Response::new(entry))
    }

    /// Makes one folder whose parent exists. A name that is taken is
    /// refused, a folder included: the person asking for a new folder is
    /// told nothing was made. A retry is safe through its idempotency key,
    /// which gets the first answer back.
    pub(crate) async fn create_folder(
        &self,
        request: Request<pb::CreateFolderRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = caller(&request)?;
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
    pub(crate) async fn rename(
        &self,
        request: Request<pb::RenameRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        let new_name = request.new_name;
        let target = self.roots.target(
            request.location.clone(),
            "location",
            entry_for(&request.location),
        )?;
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
    /// resolved and judged before the first is moved.
    pub(crate) async fn move_to_trash(
        &self,
        request: Request<pb::MoveToTrashRequest>,
    ) -> Result<Response<pb::MoveToTrashResponse>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        if request.locations.is_empty() {
            return Err(missing("locations"));
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
        let deleted_by = actor(&caller);
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
            let mut moved: Vec<String> = Vec::with_capacity(found.len());
            for (target, path) in found {
                let id = bin.trash_path(
                    &path,
                    target.shown.clone(),
                    caller_data(&deleted_by, &was(target)),
                )?;
                moved.push(id);
            }
            Ok::<_, Status>(moved
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
    Err(invalid_field(
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
        Some(Root::SessionPastes(pastes)) => pastes.session.clone().map(Entity::Session),
        Some(Root::SessionFolder(folder)) => folder.session.clone().map(Entity::Session),
        // The home folder belongs to no session or channel.
        Some(Root::Home(_) | Root::Received(_)) => None,
        None => None,
    };
    pb::EntityRef { entity }
}

/// A name a file can be renamed to: one plain name, not one the transfers
/// keep for themselves.
fn valid_name(name: &str) -> Result<(), Status> {
    let bad = |message: String| {
        invalid_field(
            "new_name",
            Problem::BadFormat,
            message,
            "give one name, like notes.md, with no \"/\"; to move it elsewhere, use a transfer",
        )
    };
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\0')
    {
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
    failure(
        Code::AlreadyExists,
        error(
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
fn found(err: &std::io::Error, target: &Target, path: &std::path::Path) -> Status {
    match err.kind() {
        ErrorKind::NotFound => not_found(
            pb::EntityKind::File,
            &target.shown,
            format!("there is no {} there", target.shown),
            "check the path; it is relative to the root the location names",
        ),
        _ => io_failure(err, "read", path),
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
        modified_at: meta.modified().ok().map(timestamp),
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

/// The entry `target` names, as Stat answers it.
fn stat_of(target: &Target) -> Result<pb::FileEntry, Status> {
    let path = target.existing()?;
    let meta = fs::metadata(&path).map_err(|err| found(&err, target, &path))?;
    Ok(entry_of(target.location.clone(), &meta, target.writable))
}

fn list(target: &Target, size: usize, token: &str) -> Result<pb::ListDirectoryResponse, Status> {
    let path = target.existing()?;
    let read = fs::read_dir(&path).map_err(|err| match err.kind() {
        ErrorKind::NotFound => found(&err, target, &path),
        ErrorKind::NotADirectory => invalid_field(
            "location.path",
            Problem::NotAllowedHere,
            format!("{} is a file, not a folder", target.shown),
            "list the folder that holds it, or read the file",
        ),
        _ => io_failure(&err, "read the folder", &path),
    })?;
    // Names and whether each is a real folder (links aren't followed), from
    // the folder's own listing without a stat of every entry.
    let mut names: Vec<(Order, String)> = Vec::new();
    for entry in read {
        let entry = entry.map_err(|err| io_failure(&err, "read the folder", &path))?;
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
            invalid_field(
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

/// Puts the folder's entries on disk after a make or rename in it. Best
/// effort: the change is already made, and a failure here only means a
/// power cut right now could undo it.
fn sync_parent(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::File::open(parent).and_then(|folder| folder.sync_all());
    }
}

fn make_folder(target: &Target) -> Result<pb::FileEntry, Status> {
    if target.is_root() {
        let path = target.existing()?;
        let meta = fs::metadata(&path).map_err(|err| found(&err, target, &path))?;
        return Ok(entry_of(target.location.clone(), &meta, target.writable));
    }
    let path = target.resolve(false)?;
    match fs::symlink_metadata(&path) {
        Ok(_) => return Err(name_taken(&target.shown, &target.shown)),
        Err(err) if err.kind() == ErrorKind::NotFound => match fs::create_dir(&path) {
            Ok(()) => sync_parent(&path),
            // Made by someone else a moment ago: the name is taken.
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                return Err(name_taken(&target.shown, &target.shown));
            }
            Err(err) => return Err(io_failure(&err, "create the folder", &path)),
        },
        Err(err) => return Err(io_failure(&err, "look at", &path)),
    }
    let meta = fs::symlink_metadata(&path).map_err(|err| found(&err, target, &path))?;
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
    fs::rename(&from, &to).map_err(|err| io_failure(&err, "rename", &from))?;
    sync_parent(&to);
    let meta = fs::symlink_metadata(&to).map_err(|err| io_failure(&err, "read", &to))?;
    let mut location = target.location.clone();
    location.path = match target.location.path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{new_name}"),
        None => new_name.to_owned(),
    };
    Ok(entry_of(location, &meta, target.writable))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use humata2_store::Store;

    use super::*;

    fn test_roots(dir: &tempfile::TempDir) -> Arc<Roots> {
        let store = Arc::new(Store::open(dir.path().join("state")).expect("open store"));
        let channels = dir.path().join("channels");
        let home = dir.path().join("home");
        fs::create_dir_all(&channels).unwrap();
        fs::create_dir_all(&home).unwrap();
        Arc::new(Roots::new(store, "local".to_owned(), channels, home))
    }

    fn home_loc(path: &str) -> pb::FileLocation {
        pb::FileLocation {
            machine: Some(pb::MachineId {
                value: "local".to_owned(),
            }),
            root: Some(pb::file_location::Root::Home(Default::default())),
            path: path.to_owned(),
        }
    }

    fn entry_path(entry: &pb::FileEntry) -> &str {
        entry
            .location
            .as_ref()
            .map(|l| l.path.as_str())
            .unwrap_or("")
    }

    #[test]
    fn list_directory_sorts_folders_first_then_case_insensitively_and_paginates() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(&dir);
        let home = dir.path().join("home");
        fs::create_dir(home.join("zeta_dir")).unwrap();
        fs::create_dir(home.join("Alpha_dir")).unwrap();
        fs::write(home.join("b.txt"), b"b").unwrap();
        fs::write(home.join("A.txt"), b"a").unwrap();

        let target = roots
            .target(Some(home_loc("")), "location", Entry::Folder)
            .unwrap();
        let first = list(&target, 2, "").unwrap();
        let first_names: Vec<&str> = first.entries.iter().map(entry_path).collect();
        assert_eq!(first_names, vec!["Alpha_dir", "zeta_dir"]);
        assert!(!first.page.as_ref().unwrap().next_page_token.is_empty());

        let token = first.page.unwrap().next_page_token;
        let second = list(&target, 10, &token).unwrap();
        let second_names: Vec<&str> = second.entries.iter().map(entry_path).collect();
        assert_eq!(second_names, vec!["A.txt", "b.txt"]);
        assert!(second.page.as_ref().unwrap().next_page_token.is_empty());
    }

    #[test]
    fn path_escape_and_absolute_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(&dir);

        for bad in ["../outside", "sub/../../outside", "/etc/passwd"] {
            let err = match roots.target(Some(home_loc(bad)), "location", Entry::File) {
                Err(e) => e,
                Ok(_) => panic!("expected {bad:?} to be rejected"),
            };
            assert!(
                matches!(err.code(), Code::InvalidArgument | Code::PermissionDenied),
                "path {bad:?}: got {:?}",
                err.code()
            );
        }
    }

    #[test]
    fn create_folder_and_rename_enforce_uniqueness_and_update_entries() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(&dir);

        let target = roots
            .target(Some(home_loc("notes")), "location", Entry::Folder)
            .unwrap();
        let created = make_folder(&target).unwrap();
        assert_eq!(entry_path(&created), "notes");

        // Creating the same folder again fails with AlreadyExists.
        let dup = make_folder(&target).expect_err("duplicate folder must fail");
        assert_eq!(dup.code(), Code::AlreadyExists);

        // Renaming moves it and stat reflects the new path.
        let renamed = rename(&target, "archive").unwrap();
        assert_eq!(entry_path(&renamed), "archive");
        let moved_target = roots
            .target(Some(home_loc("archive")), "location", Entry::Folder)
            .unwrap();
        let stat = stat_of(&moved_target).unwrap();
        assert_eq!(entry_path(&stat), "archive");
        assert!(PathBuf::from(dir.path().join("home/archive")).is_dir());
    }
}

//! The files of session clones, told on the feed as they come and go
//! (bug 23): the palette's Files list keeps its first fetch current from
//! `FileChanged` events instead of asking again.
//!
//! Each clone's file list is remembered in memory, filled the first time
//! it is needed and without a word, so a clone seen for the first time
//! sends nothing. A later look lists the clone again and tells what was
//! added, removed or renamed since; a changed file's text is not news, the
//! list is the same. Looks come from the user's own writes in the file
//! service and from the pointer loop's watcher, which wakes on every file
//! change in a live session's clone, the agent's edits included.
//!
//! A look never lists a clone again to find what changed: it is told the
//! paths that changed (the file service knows them; the watcher names them)
//! and stats only those, so the work follows the change and not the size of
//! the repo. A named path that now exists and is not in the list is new (a
//! new folder is listed under itself alone); one in the list that is gone
//! is removed, and a gone folder removes everything listed under it. The
//! whole clone is listed again only when the watcher says it lost events
//! (overflow, rescan or an error) or when the clone has no list yet.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_access::Entry;
use crate::pb;

use crate::find::{
    Listing, WALK_MAX, clone_dir_names, clone_listing, ignored_among, own_folder_listing,
    own_subtree_files, subtree_files,
};
use crate::ports::FileFeedPort;
use crate::roots::Roots;

/// The most events one look sends. A branch switch changes thousands of
/// files; a list that long is the checkout's news, not each file's.
const CHANGES_MAX: usize = 500;

/// What changed in a clone's list of files.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    Created(String),
    Deleted(String),
    Renamed { from: String, to: String },
}

pub struct FileChanges {
    feed: Arc<dyn FileFeedPort>,
    /// The files last seen in each clone, by session and `owner/name`.
    tracker: Mutex<Tracker>,
}

/// A session's clone of one repo (or the session's own folder when `repo` is
/// `None`), as a file call names it, and the paths the call changed in it.
#[derive(Clone)]
pub(crate) struct Spot {
    session: String,
    repo: Option<pb::RepoRef>,
    root: PathBuf,
    skip_dirs: HashSet<String>,
    /// Paths in the root, `/`-separated, that the call wrote, renamed or trashed.
    touched: Vec<String>,
}

impl Spot {
    /// The clone or session folder `location` is in, if it is on this machine.
    pub(crate) fn of(roots: &Roots, location: Option<&pb::FileLocation>) -> Option<Self> {
        let location = location?;
        let (session, repo) = match &location.root {
            Some(pb::file_location::Root::SessionClone(clone)) => {
                let session = clone
                    .session
                    .as_ref()
                    .map(|session| session.value.clone())
                    .filter(|id| !id.is_empty())?;
                let repo = clone.repo.clone()?;
                (session, Some(repo))
            }
            Some(pb::file_location::Root::SessionFolder(folder)) => {
                let session = folder
                    .session
                    .as_ref()
                    .map(|session| session.value.clone())
                    .filter(|id| !id.is_empty())?;
                (session, None)
            }
            _ => return None,
        };
        let root = pb::FileLocation {
            machine: location.machine.clone(),
            root: location.root.clone(),
            path: String::new(),
        };
        let root = roots
            .target(Some(root), "location", Entry::Folder)
            .ok()?
            .existing()
            .ok()?;
        let skip_dirs = if repo.is_none() {
            let repos = roots
                .store()
                .session_detail(&session)
                .ok()
                .flatten()
                .map(|detail| detail.repos)
                .unwrap_or_default();
            clone_dir_names(&root, &repos)
        } else {
            HashSet::new()
        };
        Some(Self {
            session,
            repo,
            root,
            skip_dirs,
            touched: vec![location.path.clone()],
        })
    }

    /// Whether `other` is the same session's clone (or folder).
    pub(crate) fn same_clone(&self, other: &Self) -> bool {
        self.session == other.session && self.repo == other.repo && self.root == other.root
    }

    /// Takes the paths of `other`, a call's other location in this clone.
    pub(crate) fn also(&mut self, other: Self) {
        self.touched.extend(other.touched);
    }

    /// Names where renaming `path` to `new_name` puts it: beside it.
    pub(crate) fn renamed(&mut self, path: &str, new_name: &str) {
        let to = match path.trim_end_matches('/').rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{new_name}"),
            None => new_name.to_owned(),
        };
        self.touched.push(to);
    }

    pub(crate) fn remember(&self, changes: &FileChanges) {
        changes.remember(&self.session, self.repo.as_ref(), &self.root, &self.skip_dirs);
    }

    pub(crate) fn look(&self, changes: &FileChanges) {
        changes.look(
            &self.session,
            self.repo.as_ref(),
            &self.root,
            &self.touched,
            false,
            &self.skip_dirs,
        );
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type Key = (String, String);

fn key_of(session: &str, repo: Option<&pb::RepoRef>) -> Key {
    let repo_key = match repo {
        Some(repo) => format!("{}/{}", repo.owner, repo.name),
        None => String::new(),
    };
    (session.to_owned(), repo_key)
}

impl FileChanges {
    pub fn new(feed: Arc<dyn FileFeedPort>) -> Self {
        Self {
            feed,
            tracker: Mutex::default(),
        }
    }

    /// Keeps the clone's (or session folder's) files if none are kept yet,
    /// saying nothing. Runs git and stats files: call it on a blocking thread.
    pub fn remember(
        &self,
        session: &str,
        repo: Option<&pb::RepoRef>,
        root: &Path,
        skip_dirs: &HashSet<String>,
    ) {
        lock(&self.tracker).sight(&key_of(session, repo), root, skip_dirs);
    }

    /// Seeds the clone's (or session folder's) baseline from an unfiltered
    /// `find_files` listing if no list is kept yet, so the first file written
    /// afterward emits `FileChanged::Created` instead of being swallowed as
    /// the initial sighting. Stats `listed` on disk: call on a blocking thread.
    pub fn seed(
        &self,
        session: &str,
        repo: Option<&pb::RepoRef>,
        root: &Path,
        listed: &[String],
    ) {
        lock(&self.tracker).seed(&key_of(session, repo), root, listed);
    }

    /// Drops every remembered file list for `session`, so archived or freed
    /// sessions do not accumulate in memory.
    pub fn forget(&self, session: &str) {
        let mut tracker = lock(&self.tracker);
        tracker.seen.retain(|(id, _), _| id != session);
        tracker.over.retain(|(id, _)| id != session);
    }

    /// Tells what happened to `named`, the paths (relative to the root) that
    /// changed. With `rescan` the root is listed again and every difference
    /// told, for when the watcher lost events. A root with no list yet only
    /// gets one. A root that can't be listed (or has more files than a list
    /// holds) is left as it was. Runs git and stats files: call it on a
    /// blocking thread.
    pub fn look(
        &self,
        session: &str,
        repo: Option<&pb::RepoRef>,
        root: &Path,
        named: &[String],
        rescan: bool,
        skip_dirs: &HashSet<String>,
    ) {
        // Held through the telling, so two looks never tell one change twice or out of order.
        let mut tracker = lock(&self.tracker);
        let changes = tracker.apply(&key_of(session, repo), root, named, rescan, skip_dirs);
        if !changes.is_empty() {
            self.tell(session, repo, changes);
        }
    }

    fn tell(&self, session: &str, repo: Option<&pb::RepoRef>, changes: Vec<Change>) {
        use pb::file_changed::Kind;
        for change in changes {
            let (kind, path, old_path) = match change {
                Change::Created(path) => (Kind::Created, path, String::new()),
                Change::Deleted(path) => (Kind::Deleted, path, String::new()),
                Change::Renamed { from, to } => (Kind::Renamed, to, from),
            };
            self.feed.send_file_changed(
                session,
                pb::FileChanged {
                    session: Some(pb::SessionId {
                        value: session.to_owned(),
                    }),
                    repo: repo.cloned(),
                    kind: kind as i32,
                    path,
                    old_path,
                    folder: false,
                },
            );
        }
    }
}

/// The remembered file lists, and how much work they took (read by tests:
/// a look of one changed path must not list the clone).
struct Tracker {
    seen: HashMap<Key, HashSet<String>>,
    /// Clones with more files than a list holds: never listed again unless
    /// the watcher says it lost events, and told nothing meanwhile.
    over: HashSet<Key>,
    /// The most files a list holds.
    cap: usize,
    /// Whole-clone listings made.
    lists: usize,
    /// Paths statted one by one.
    stats: usize,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::capped(WALK_MAX)
    }
}

impl Tracker {
    fn capped(cap: usize) -> Self {
        Self {
            seen: HashMap::new(),
            over: HashSet::new(),
            cap,
            lists: 0,
            stats: 0,
        }
    }

    /// Lists the clone (or session folder) and keeps it, unless a list is
    /// already kept or it is known to be too big for one.
    fn sight(&mut self, key: &Key, root: &Path, skip_dirs: &HashSet<String>) {
        if self.seen.contains_key(key) || self.over.contains(key) {
            return;
        }
        self.lists += 1;
        let is_session_folder = key.1.is_empty();
        match files(root, self.cap, is_session_folder, skip_dirs) {
            Some(Some(now)) => {
                self.seen.insert(key.clone(), now);
            }
            Some(None) => {
                self.over.insert(key.clone());
            }
            None => {}
        }
    }

    /// Seeds `key`'s baseline from `listed` (an unfiltered `find_files` result)
    /// unless a list is already kept or it is known to be over the cap.
    fn seed(&mut self, key: &Key, root: &Path, listed: &[String]) {
        if self.seen.contains_key(key) || self.over.contains(key) {
            return;
        }
        if listed.len() >= self.cap {
            self.over.insert(key.clone());
            return;
        }
        let now: HashSet<String> = listed
            .iter()
            .filter(|path| !partial(path))
            .filter(|path| {
                root.join(path)
                    .symlink_metadata()
                    .is_ok_and(|meta| !meta.is_dir())
            })
            .cloned()
            .collect();
        self.seen.insert(key.clone(), now);
    }

    /// Updates the clone's (or session folder's) list for `named` (or all of
    /// it on `rescan`) and returns what changed, at most [`CHANGES_MAX`] of it.
    fn apply(
        &mut self,
        key: &Key,
        root: &Path,
        named: &[String],
        rescan: bool,
        skip_dirs: &HashSet<String>,
    ) -> Vec<Change> {
        let is_session_folder = key.1.is_empty();
        if self.over.contains(key) {
            // Too big to follow: only lost events are a reason to look again.
            if rescan {
                self.over.remove(key);
                self.sight(key, root, skip_dirs);
            }
            return Vec::new();
        }
        if !self.seen.contains_key(key) {
            self.sight(key, root, skip_dirs);
            return Vec::new();
        }
        if rescan {
            self.lists += 1;
            return match files(root, self.cap, is_session_folder, skip_dirs) {
                Some(Some(now)) => match self.seen.get_mut(key) {
                    Some(before) => {
                        let changes = diff(before, &now);
                        *before = now;
                        changes
                    }
                    None => Vec::new(),
                },
                Some(None) => {
                    self.seen.remove(key);
                    self.over.insert(key.clone());
                    Vec::new()
                }
                None => Vec::new(),
            };
        }
        let Some(before) = self.seen.get_mut(key) else {
            return Vec::new();
        };
        let mut gone: BTreeSet<String> = BTreeSet::new();
        let mut new: BTreeSet<String> = BTreeSet::new();
        // Paths that may have had files under them that are gone now.
        let mut folders: HashSet<String> = HashSet::new();
        let mut loose: Vec<String> = Vec::new();
        let mut paths: Vec<String> = named.iter().filter_map(|path| tidy(path)).collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            if hidden(&path) {
                continue;
            }
            if is_session_folder
                && let Some(first) = path.split('/').next()
                && (skip_dirs.contains(first) || root.join(first).join(".git").exists())
            {
                continue;
            }
            self.stats += 1;
            let meta = root.join(&path).symlink_metadata();
            let is_dir = meta.as_ref().is_ok_and(std::fs::Metadata::is_dir);
            let was_file = before.contains(&path);
            if is_dir {
                if was_file {
                    gone.insert(path.clone());
                }
                // A folder that is here: what is in it now and not listed is new.
                let listed = if is_session_folder {
                    own_subtree_files(root, &path, skip_dirs)
                } else {
                    subtree_files(root, &path)
                };
                for file in listed.unwrap_or_default() {
                    if partial(&file) || before.contains(&file) {
                        continue;
                    }
                    self.stats += 1;
                    if root
                        .join(&file)
                        .symlink_metadata()
                        .is_ok_and(|meta| !meta.is_dir())
                    {
                        new.insert(file);
                    }
                }
                continue;
            }
            if meta.is_ok() {
                if !was_file {
                    loose.push(path.clone());
                    folders.insert(path);
                }
            } else if was_file {
                gone.insert(path);
            } else {
                folders.insert(path);
            }
        }
        // One pass over the list for every named path that may have been a folder.
        if !folders.is_empty() {
            for file in before.iter() {
                let mut end = file.len();
                while let Some(slash) = file[..end].rfind('/') {
                    end = slash;
                    if folders.contains(&file[..end]) {
                        gone.insert(file.clone());
                        break;
                    }
                }
            }
        }
        // New files git ignores are not files of the clone.
        let ignored = ignored_among(root, &loose);
        new.extend(loose.into_iter().filter(|path| !ignored.contains(path)));
        for path in &gone {
            before.remove(path);
        }
        new.retain(|path| !before.contains(path));
        before.extend(new.iter().cloned());
        changes_of(gone.into_iter().collect(), new.into_iter().collect())
    }
}

/// A path as a list entry: `/`-separated with no empty or `.` parts, or none
/// for the clone itself or a path that leaves it.
fn tidy(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Git's own files, and an upload's half-written ones, are not files of the clone.
fn hidden(path: &str) -> bool {
    path.split('/').next() == Some(".git") || partial(path)
}

/// The files of a clone (or session folder) as people see them: what git knows
/// or would add, that is on disk, without the half-written files of an upload.
///
/// `None` when the root can't be listed, `Some(None)` when it has `cap`
/// files or more (a list cut short would tell false removals).
fn files(
    root: &Path,
    cap: usize,
    is_session_folder: bool,
    skip_dirs: &HashSet<String>,
) -> Option<Option<HashSet<String>>> {
    let listing = if is_session_folder {
        own_folder_listing(root, skip_dirs, cap)
    } else {
        clone_listing(root, cap)
    };
    match listing {
        Listing::Files(listed) => Some(Some(
            listed
                .into_iter()
                .filter(|path| !partial(path))
                .filter(|path| {
                    root.join(path)
                        .symlink_metadata()
                        .is_ok_and(|meta| !meta.is_dir())
                })
                .collect(),
        )),
        Listing::TooMany => Some(None),
        Listing::Failed => None,
    }
}

/// An upload's or atomic write's hidden partial file (see `Upload::start`).
fn partial(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| {
        name.starts_with("token-tb_")
            || (name.starts_with('.')
                && (name.ends_with(".partial") || name.contains(".humata-partial-")))
    })
}

/// What the list of files lost and gained: removed first, then added, each
/// in path order. One removed and one added together are one rename.
fn diff(before: &HashSet<String>, now: &HashSet<String>) -> Vec<Change> {
    let mut created: Vec<String> = now.difference(before).cloned().collect();
    let mut deleted: Vec<String> = before.difference(now).cloned().collect();
    created.sort();
    deleted.sort();
    changes_of(deleted, created)
}

/// The changes of `deleted` and `created` (each in path order): removed
/// first, then added; one of each together is one rename.
fn changes_of(deleted: Vec<String>, created: Vec<String>) -> Vec<Change> {
    if let ([from], [to]) = (deleted.as_slice(), created.as_slice()) {
        return vec![Change::Renamed {
            from: from.clone(),
            to: to.clone(),
        }];
    }
    let mut changes: Vec<Change> = deleted.into_iter().map(Change::Deleted).collect();
    changes.extend(created.into_iter().map(Change::Created));
    changes.truncate(CHANGES_MAX);
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_new_file_is_created() {
        assert_eq!(
            diff(&set(&["a.rs"]), &set(&["a.rs", "b.rs"])),
            vec![Change::Created("b.rs".to_owned())]
        );
    }

    #[test]
    fn a_gone_file_is_deleted() {
        assert_eq!(
            diff(&set(&["a.rs", "b.rs"]), &set(&["a.rs"])),
            vec![Change::Deleted("b.rs".to_owned())]
        );
    }

    #[test]
    fn one_gone_and_one_new_file_are_a_rename() {
        assert_eq!(
            diff(&set(&["a.rs", "keep.rs"]), &set(&["b.rs", "keep.rs"])),
            vec![Change::Renamed {
                from: "a.rs".to_owned(),
                to: "b.rs".to_owned()
            }]
        );
    }

    #[test]
    fn several_changes_are_each_told_removed_first_in_path_order() {
        assert_eq!(
            diff(&set(&["a", "b", "c"]), &set(&["c", "z", "y"])),
            vec![
                Change::Deleted("a".to_owned()),
                Change::Deleted("b".to_owned()),
                Change::Created("y".to_owned()),
                Change::Created("z".to_owned()),
            ]
        );
    }

    #[test]
    fn the_same_list_changes_nothing() {
        assert!(diff(&set(&["a", "b"]), &set(&["b", "a"])).is_empty());
    }

    #[test]
    fn a_flood_of_changes_is_cut_at_the_cap() {
        let now: HashSet<String> = (0..CHANGES_MAX + 50).map(|n| format!("f{n}")).collect();
        assert_eq!(diff(&HashSet::new(), &now).len(), CHANGES_MAX);
    }

    #[test]
    fn an_uploads_partial_file_is_not_a_file() {
        assert!(partial("src/.lib.rs.humata-partial-0123"));
        assert!(partial("state/.status-since.json.33096-7.partial"));
        assert!(!partial("src/lib.rs"));
    }

    fn key() -> Key {
        ("se".to_owned(), "acme/api".to_owned())
    }

    fn named(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    fn put(root: &Path, path: &str) {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, b"x\n").unwrap();
    }

    fn apply(tracker: &mut Tracker, key: &Key, root: &Path, named: &[String], rescan: bool) -> Vec<Change> {
        tracker.apply(key, root, named, rescan, &HashSet::new())
    }

    /// A clone of `count` files in 200 folders, with its list recorded.
    fn big_clone(count: usize) -> (tempfile::TempDir, Tracker) {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..count {
            put(dir.path(), &format!("d{}/f{n}.rs", n % 200));
        }
        let mut tracker = Tracker::default();
        assert!(apply(&mut tracker, &key(), dir.path(), &[], false).is_empty());
        assert_eq!(tracker.seen[&key()].len(), count);
        (dir, tracker)
    }

    #[test]
    fn one_new_file_in_a_20k_clone_is_found_without_listing_it_again() {
        let (dir, mut tracker) = big_clone(200);
        assert_eq!(tracker.lists, 1, "the first sight lists once");
        let kept = tracker.seen.get_mut(&key()).unwrap();
        kept.extend((200..20_000).map(|n| format!("d{}/f{n}.rs", n % 200)));
        assert_eq!(kept.len(), 20_000);
        put(dir.path(), "d7/new.rs");

        let changes = apply(&mut tracker, &key(), dir.path(), &named(&["d7/new.rs"]), false);

        assert_eq!(changes, vec![Change::Created("d7/new.rs".to_owned())]);
        assert_eq!(tracker.lists, 1, "no full listing after the first");
        assert!(tracker.stats <= 4, "{} stats", tracker.stats);
        assert_eq!(tracker.seen[&key()].len(), 20_001);
    }

    #[test]
    fn a_file_that_is_already_listed_and_still_there_is_no_change() {
        let (dir, mut tracker) = big_clone(200);
        std::fs::write(dir.path().join("d3/f3.rs"), b"changed\n").unwrap();
        assert!(
            apply(&mut tracker, &key(), dir.path(), &named(&["d3/f3.rs", "./d3//f3.rs"]), false)
                .is_empty()
        );
        assert_eq!(tracker.lists, 1);
    }

    #[test]
    fn a_deleted_folder_deletes_every_file_listed_under_it() {
        let (dir, mut tracker) = big_clone(400);
        std::fs::remove_dir_all(dir.path().join("d5")).unwrap();

        let changes = apply(&mut tracker, &key(), dir.path(), &named(&["d5"]), false);

        assert_eq!(
            changes,
            vec![
                Change::Deleted("d5/f205.rs".to_owned()),
                Change::Deleted("d5/f5.rs".to_owned()),
            ]
        );
        assert_eq!(tracker.seen[&key()].len(), 398);
        assert!(
            !tracker.seen[&key()]
                .iter()
                .any(|path| path.starts_with("d5/"))
        );
        assert_eq!(tracker.lists, 1);
        assert!(tracker.stats <= 2);
    }

    #[test]
    fn a_gone_file_is_deleted_and_one_gone_with_one_new_is_a_rename() {
        let (dir, mut tracker) = big_clone(10);
        std::fs::remove_file(dir.path().join("d1/f1.rs")).unwrap();
        assert_eq!(
            apply(&mut tracker, &key(), dir.path(), &named(&["d1/f1.rs"]), false),
            vec![Change::Deleted("d1/f1.rs".to_owned())]
        );

        std::fs::rename(dir.path().join("d2/f2.rs"), dir.path().join("d2/g2.rs")).unwrap();
        assert_eq!(
            apply(&mut tracker, &key(), dir.path(), &named(&["d2/f2.rs", "d2/g2.rs"]), false),
            vec![Change::Renamed {
                from: "d2/f2.rs".to_owned(),
                to: "d2/g2.rs".to_owned()
            }]
        );
        assert!(tracker.seen[&key()].contains("d2/g2.rs"));
        assert!(!tracker.seen[&key()].contains("d2/f2.rs"));
    }

    #[test]
    fn a_new_folder_lists_only_itself() {
        let (dir, mut tracker) = big_clone(100);
        put(dir.path(), "fresh/a.rs");
        put(dir.path(), "fresh/deep/b.rs");

        let changes = apply(&mut tracker, &key(), dir.path(), &named(&["fresh"]), false);

        assert_eq!(
            changes,
            vec![
                Change::Created("fresh/a.rs".to_owned()),
                Change::Created("fresh/deep/b.rs".to_owned()),
            ]
        );
        assert_eq!(tracker.lists, 1);
    }

    #[test]
    fn a_renamed_folder_removes_the_old_files_and_adds_the_new() {
        let (dir, mut tracker) = big_clone(10);
        std::fs::rename(dir.path().join("d4"), dir.path().join("e4")).unwrap();

        let changes = apply(&mut tracker, &key(), dir.path(), &named(&["d4", "e4"]), false);

        assert_eq!(
            changes,
            vec![Change::Renamed {
                from: "d4/f4.rs".to_owned(),
                to: "e4/f4.rs".to_owned()
            }]
        );
        assert_eq!(tracker.lists, 1);
    }

    #[test]
    fn git_internals_and_partials_are_not_looked_at() {
        let (dir, mut tracker) = big_clone(10);
        put(dir.path(), ".git/objects/ab");
        put(dir.path(), "d1/.f.rs.humata-partial-1");

        let changes = apply(
            &mut tracker,
            &key(),
            dir.path(),
            &named(&[
                ".git/objects/ab",
                ".git",
                "d1/.f.rs.humata-partial-1",
                "../outside",
                "",
            ]),
            false,
        );

        assert!(changes.is_empty());
        assert_eq!(tracker.stats, 0);
        assert_eq!(tracker.seen[&key()].len(), 10);
    }

    #[test]
    fn a_rescan_lists_once_and_tells_every_difference() {
        let (dir, mut tracker) = big_clone(50);
        put(dir.path(), "n1.rs");
        put(dir.path(), "n2.rs");
        std::fs::remove_file(dir.path().join("d0/f0.rs")).unwrap();

        let changes = apply(&mut tracker, &key(), dir.path(), &[], true);

        assert_eq!(
            changes,
            vec![
                Change::Deleted("d0/f0.rs".to_owned()),
                Change::Created("n1.rs".to_owned()),
                Change::Created("n2.rs".to_owned()),
            ]
        );
        assert_eq!(tracker.lists, 2);
        assert_eq!(tracker.stats, 0);
    }

    #[test]
    fn a_clone_seen_for_the_first_time_is_recorded_and_says_nothing() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "a.rs");
        let mut tracker = Tracker::default();
        assert!(
            apply(&mut tracker, &key(), dir.path(), &named(&["a.rs"]), false).is_empty()
        );
        assert_eq!(tracker.lists, 1);
        assert!(tracker.seen[&key()].contains("a.rs"));
    }

    #[test]
    fn a_clone_over_the_cap_is_listed_once_and_then_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..10 {
            put(dir.path(), &format!("d/f{n}.rs"));
        }
        let mut tracker = Tracker::capped(5);
        assert!(apply(&mut tracker, &key(), dir.path(), &[], false).is_empty());
        assert_eq!(tracker.lists, 1);
        assert!(!tracker.seen.contains_key(&key()));

        put(dir.path(), "d/new.rs");
        std::fs::remove_file(dir.path().join("d/f1.rs")).unwrap();
        let changes = apply(
            &mut tracker,
            &key(),
            dir.path(),
            &named(&["d/new.rs", "d/f1.rs", "d"]),
            false,
        );

        assert!(changes.is_empty());
        assert_eq!((tracker.lists, tracker.stats), (1, 0));
    }

    #[test]
    fn a_rescan_tries_a_clone_over_the_cap_again() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..10 {
            put(dir.path(), &format!("d/f{n}.rs"));
        }
        let mut tracker = Tracker::capped(5);
        apply(&mut tracker, &key(), dir.path(), &[], false);
        assert!(apply(&mut tracker, &key(), dir.path(), &[], true).is_empty());
        assert_eq!(tracker.lists, 2);
        assert!(!tracker.seen.contains_key(&key()));

        for n in 0..8 {
            std::fs::remove_file(dir.path().join(format!("d/f{n}.rs"))).unwrap();
        }
        assert!(apply(&mut tracker, &key(), dir.path(), &[], true).is_empty());
        assert_eq!(tracker.lists, 3);
        assert_eq!(tracker.seen[&key()].len(), 2);
        put(dir.path(), "d/z.rs");
        assert_eq!(
            apply(&mut tracker, &key(), dir.path(), &named(&["d/z.rs"]), false),
            vec![Change::Created("d/z.rs".to_owned())]
        );
    }

    #[test]
    fn a_flood_of_new_paths_is_cut_at_the_cap_but_all_are_kept() {
        let (dir, mut tracker) = big_clone(10);
        let paths: Vec<String> = (0..CHANGES_MAX + 20)
            .map(|n| format!("bulk/f{n}.rs"))
            .collect();
        for path in &paths {
            put(dir.path(), path);
        }
        let changes = apply(&mut tracker, &key(), dir.path(), &paths, false);
        assert_eq!(changes.len(), CHANGES_MAX);
        assert_eq!(tracker.seen[&key()].len(), 10 + CHANGES_MAX + 20);
    }

    #[test]
    fn session_folder_tracks_root_files_and_skips_clones() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "api/src/lib.rs");
        let folder_key = ("se".to_owned(), String::new());
        let skip: HashSet<String> = ["api".to_owned()].into_iter().collect();
        let mut tracker = Tracker::default();
        assert!(tracker.apply(&folder_key, dir.path(), &[], false, &skip).is_empty());
        assert!(tracker.seen[&folder_key].is_empty());

        put(dir.path(), "notes.md");
        put(dir.path(), "api/src/extra.rs");
        let changes = tracker.apply(
            &folder_key,
            dir.path(),
            &named(&["notes.md", "api/src/extra.rs"]),
            false,
            &skip,
        );
        assert_eq!(changes, vec![Change::Created("notes.md".to_owned())]);
    }

    #[test]
    fn seed_primes_empty_session_folder_so_first_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let folder_key = ("se".to_owned(), String::new());
        let skip = HashSet::new();
        let mut tracker = Tracker::default();

        tracker.seed(&folder_key, dir.path(), &[]);
        assert_eq!(tracker.lists, 0);
        assert!(tracker.seen[&folder_key].is_empty());

        put(dir.path(), "CHANNEL_HOME_SPEC.md");
        let changes = tracker.apply(
            &folder_key,
            dir.path(),
            &named(&["CHANNEL_HOME_SPEC.md"]),
            false,
            &skip,
        );
        assert_eq!(
            changes,
            vec![Change::Created("CHANNEL_HOME_SPEC.md".to_owned())]
        );
        assert_eq!(tracker.lists, 0);
    }
}

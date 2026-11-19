//! FindFiles: "Go to file…" across a session's repos (RV-11, RV-9, RV-14).
//!
//! Each repo's candidates are the files git knows or would add: tracked
//! files and untracked files that aren't ignored, so `.git`, build output
//! and ignored files never show. A folder that isn't a git work tree is
//! walked instead, up to [`WALK_MAX`] files. A query matches a path when
//! its characters appear in the path in order, case-insensitively; matches
//! are ranked inside each repo, and repos keep the session's order.
//!
//! Git, the folder walk, the matching and the stats of the returned page all
//! run on a blocking thread, one repo's path list at a time. No file's
//! contents are read.
//!
//! Pages are not snapshots: the ranked list is recomputed for every call and
//! the page token is the index of the next entry in it. A file made or
//! removed between two pages can move later entries by one place.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use humata2_access::Entry;
use crate::pb;
use humata2_store::Store;
use pb::invalid_field::Problem;
use tonic::{Request, Response, Status};

use crate::Reads;
use crate::ports::{
    DESK_CHANNEL, blocking, caller, invalid_field, missing, not_found, store_error, timestamp,
};
use crate::roots::{Roots, Target, io_failure};

/// Entries per page when the caller names no size.
const PAGE_DEFAULT: usize = 200;

/// The most entries in one page.
const PAGE_MAX: usize = 1000;

/// The longest query, in bytes.
const QUERY_MAX: usize = 256;

/// The most files taken from a folder that isn't a git work tree.
pub(crate) const WALK_MAX: usize = 50_000;

/// Variables that would point git at another repository, as the review code
/// scrubs them.
const SCRUBBED: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_COMMON_DIR",
];

pub(crate) struct Find {
    roots: Arc<Roots>,
    reads: Reads,
    store: Arc<Store>,
}

impl Find {
    pub(crate) fn new(roots: Arc<Roots>, reads: Reads, store: Arc<Store>) -> Self {
        Self {
            roots,
            reads,
            store,
        }
    }

    /// The files of the session's repos whose paths match the query: repos
    /// in the session's order, ranked inside each. A session with no repos
    /// has no matches. Every repo must be one the caller may read; one that
    /// isn't fails the call before any file is listed.
    pub(crate) async fn find_files(
        &self,
        request: Request<pb::FindFilesRequest>,
    ) -> Result<Response<pb::FindFilesResponse>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        if request.query.len() > QUERY_MAX {
            return Err(invalid_field(
                "query",
                Problem::OutOfRange,
                format!(
                    "the query is {} bytes long and the longest is {QUERY_MAX}",
                    request.query.len()
                ),
                "type fewer characters of the file's path",
            ));
        }
        let session = request
            .session
            .map(|session| session.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| missing("session"))?;
        let machine = self.roots.machine();
        let record = match self.store.session(&session) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Err(not_found(
                    pb::EntityKind::Session,
                    &session,
                    format!("no session {session} on this machine"),
                    "check the session id; sessions on other machines can't be named here yet",
                ));
            }
            Err(error) => return Err(store_error(error, machine, "look up the session")),
        };
        let repos = self
            .store
            .session_detail(&session)
            .map_err(|error| store_error(error, machine, "read the session's repositories"))?
            .map(|detail| detail.repos)
            .unwrap_or_default();
        let (size, token) = match request.page {
            Some(page) => (page.page_size as usize, page.page_token),
            None => (0, String::new()),
        };
        let size = match size {
            0 => PAGE_DEFAULT,
            size => size.min(PAGE_MAX),
        };
        let offset = offset_of(&token)?;

        let mut targets = Vec::with_capacity(repos.len() + 1);
        // D382: a Desk session's documents sit in its own folder, outside
        // every clone; they come first, without the clones inside it.
        if record.channel == DESK_CHANNEL {
            let location = pb::FileLocation {
                machine: None,
                root: Some(pb::file_location::Root::SessionFolder(pb::SessionFolder {
                    session: Some(pb::SessionId {
                        value: session.clone(),
                    }),
                })),
                path: String::new(),
            };
            let target = self.roots.target(Some(location), "session", Entry::Folder)?;
            self.reads
                .require(&caller, target.place.clone(), "find", || None)
                .await?;
            targets.push((target, Scope::OwnFolder));
        }
        for repo in repos {
            let location = pb::FileLocation {
                machine: None,
                root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                    session: Some(pb::SessionId {
                        value: session.clone(),
                    }),
                    repo: Some(pb::RepoRef {
                        owner: repo.owner,
                        name: repo.name,
                    }),
                })),
                path: String::new(),
            };
            let target = self.roots.target(Some(location), "session", Entry::Folder)?;
            self.reads
                .require(&caller, target.place.clone(), "find", || None)
                .await?;
            targets.push((target, Scope::Clone));
        }

        let query = request.query;
        let (entries, next) = blocking(move || search(&targets, &query, offset, size)).await??;
        Ok(Response::new(pb::FindFilesResponse {
            entries,
            page: Some(pb::PageResponse {
                next_page_token: next.map(|next| next.to_string()).unwrap_or_default(),
            }),
        }))
    }
}

fn offset_of(token: &str) -> Result<usize, Status> {
    if token.is_empty() {
        return Ok(0);
    }
    token.parse().map_err(|_| {
        invalid_field(
            "page.page_token",
            Problem::BadFormat,
            "the page token isn't one this daemon gave",
            "start again without a page token",
        )
    })
}

/// What a target of a search holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// A repository's clone.
    Clone,
    /// A Desk session's own folder: its files, not those of the clones in
    /// it, which are searched as clones.
    OwnFolder,
}

fn search(
    targets: &[(Target, Scope)],
    query: &str,
    offset: usize,
    size: usize,
) -> Result<(Vec<pb::FileEntry>, Option<usize>), Status> {
    let mut skip = offset;
    let mut consumed = 0;
    let mut more = false;
    let mut entries = Vec::new();
    for (target, scope) in targets {
        // A clone that isn't on disk yet has nothing to find (RV-14).
        let root = match target.existing() {
            Ok(root) => root,
            Err(status) if status.code() == tonic::Code::NotFound => continue,
            Err(status) => return Err(status),
        };
        let paths = match scope {
            Scope::Clone => candidates(&root)?,
            Scope::OwnFolder => own_files(&root)?,
        };
        let matched = ranked(paths, query);
        if consumed == size {
            // Only whether anything is left matters now.
            more = !matched.is_empty();
            if more {
                break;
            }
            continue;
        }
        if skip >= matched.len() {
            skip -= matched.len();
            continue;
        }
        let rest = &matched[skip..];
        skip = 0;
        for path in rest {
            if consumed == size {
                more = true;
                break;
            }
            consumed += 1;
            // Gone since the listing, or a folder (a submodule): left out.
            let Ok(meta) = fs::symlink_metadata(root.join(path)) else {
                continue;
            };
            if meta.is_dir() {
                continue;
            }
            entries.push(entry_of(target.child(path), &meta, target.writable));
        }
        if more {
            break;
        }
    }
    Ok((entries, more.then_some(offset + consumed)))
}

fn entry_of(location: pb::FileLocation, meta: &Metadata, writable: bool) -> pb::FileEntry {
    let kind = if meta.file_type().is_symlink() {
        pb::file_entry::Kind::Symlink
    } else {
        pb::file_entry::Kind::File
    };
    pb::FileEntry {
        location: Some(location),
        kind: kind as i32,
        size: meta.len(),
        modified_at: meta.modified().ok().map(timestamp),
        changed: false,
        writable,
    }
}

/// The files of a clone, or `Listing::Failed` / `Listing::TooMany` when it
/// can't be listed or has `cap` files or more.
pub(crate) fn clone_listing(root: &Path, cap: usize) -> Listing {
    match candidates(root) {
        Ok(paths) if paths.len() < cap => Listing::Files(paths),
        Ok(_) => Listing::TooMany,
        Err(_) => Listing::Failed,
    }
}

/// What listing a clone found.
pub(crate) enum Listing {
    Files(Vec<String>),
    /// `cap` files or more.
    TooMany,
    Failed,
}

/// The paths (relative, `/`-separated) of a clone's files.
fn candidates(root: &Path) -> Result<Vec<String>, Status> {
    if root.join(".git").exists()
        && let Some(paths) = git_paths(root)
    {
        return Ok(paths);
    }
    walk(root)
}

/// The files of a Desk session's folder, leaving out every git work tree
/// in it: those are its clones, listed on their own.
fn own_files(root: &Path) -> Result<Vec<String>, Status> {
    if root.join(".git").exists()
        && let Some(paths) = git_paths(root)
    {
        return Ok(paths);
    }
    walk_from(root, "", true)
}

/// Tracked files and untracked files that aren't ignored, or `None` when git
/// can't say (not a work tree, not installed, refused): the caller then
/// walks the folder.
fn git_paths(root: &Path) -> Option<Vec<String>> {
    git_list(root, None)
}

/// A git command for `root`, with the variables that would point it elsewhere removed.
fn git_in(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(root);
    for name in SCRUBBED {
        command.env_remove(name);
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    command
}

/// [`git_paths`] limited to `only`, a path in the clone, when given.
fn git_list(root: &Path, only: Option<&str>) -> Option<Vec<String>> {
    let mut command = git_in(root);
    command.args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"]);
    if let Some(only) = only {
        command.arg("--").arg(format!(":(literal){only}"));
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            // A name that isn't text can't be named in the API.
            .filter_map(|name| String::from_utf8(name.to_vec()).ok())
            .collect(),
    )
}

/// The files under the folder `start` of a clone (a path in it, `/`-separated),
/// without listing the rest of the clone.
pub(crate) fn subtree_files(root: &Path, start: &str) -> Option<Vec<String>> {
    if root.join(".git").exists()
        && let Some(paths) = git_list(root, Some(start))
    {
        return Some(paths);
    }
    walk_from(root, start, false).ok()
}

/// Of `paths` (files of a git clone), those git ignores; none when the clone
/// isn't a git work tree or git can't say.
pub(crate) fn ignored_among(root: &Path, paths: &[String]) -> HashSet<String> {
    use std::io::Write;
    use std::process::Stdio;
    if paths.is_empty() || !root.join(".git").exists() {
        return HashSet::new();
    }
    let mut command = git_in(root);
    let Ok(mut child) = command
        .args(["check-ignore", "-z", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return HashSet::new();
    };
    let Some(mut stdin) = child.stdin.take() else {
        return HashSet::new();
    };
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    // Written on its own thread: git may answer before it has read everything.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let Ok(output) = child.wait_with_output() else {
        return HashSet::new();
    };
    let _ = writer.join();
    // Exit 0: some ignored; 1: none; anything else: git couldn't say.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return HashSet::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .filter_map(|name| String::from_utf8(name.to_vec()).ok())
        .collect()
}

/// The files under `root` without following symlinks, skipping `.git`.
fn walk(root: &Path) -> Result<Vec<String>, Status> {
    walk_from(root, "", false)
}

/// The files under the folder `start` of `root` (empty for all of it), as
/// paths from `root`. With `skip_work_trees`, a folder below `first` that
/// is a git work tree is left out whole.
fn walk_from(root: &Path, start: &str, skip_work_trees: bool) -> Result<Vec<String>, Status> {
    let first = if start.is_empty() {
        root.to_owned()
    } else {
        root.join(start)
    };
    let mut files = Vec::new();
    let mut stack = vec![(first.clone(), start.to_owned())];
    while let Some((dir, prefix)) = stack.pop() {
        let read = match fs::read_dir(&dir) {
            Ok(read) => read,
            Err(error) if dir == first => {
                return Err(io_failure(&error, "read the folder", &dir));
            }
            // A folder that can't be read is left out.
            Err(_) => continue,
        };
        for entry in read.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name == ".git" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                if skip_work_trees && entry.path().join(".git").exists() {
                    continue;
                }
                stack.push((entry.path(), relative));
            } else {
                if files.len() >= WALK_MAX {
                    return Ok(files);
                }
                files.push(relative);
            }
        }
    }
    Ok(files)
}

/// How well a path matches, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// The file's name is the query.
    Name,
    /// The file's name starts with the query.
    NamePrefix,
    /// The query is in the file's name, in one piece.
    NameContains,
    /// The query is in the path, in one piece.
    PathContains,
    /// The query's characters are in the path, in order.
    Subsequence,
}

/// How `path` matches `query` (already lower case), if it does.
fn rank(path: &str, query: &str) -> Option<Rank> {
    let path = path.to_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    if query.is_empty() {
        return Some(Rank::Subsequence);
    }
    if name == query {
        Some(Rank::Name)
    } else if name.starts_with(query) {
        Some(Rank::NamePrefix)
    } else if name.contains(query) {
        Some(Rank::NameContains)
    } else if path.contains(query) {
        Some(Rank::PathContains)
    } else if in_order(&path, query) {
        Some(Rank::Subsequence)
    } else {
        None
    }
}

/// Whether the characters of `needle` appear in `haystack` in order.
fn in_order(haystack: &str, needle: &str) -> bool {
    let mut wanted = needle.chars().peekable();
    for found in haystack.chars() {
        match wanted.peek() {
            Some(next) if *next == found => {
                wanted.next();
            }
            Some(_) => {}
            None => break,
        }
    }
    wanted.peek().is_none()
}

/// The paths that match `query`, best first: by rank, then shorter paths,
/// then by path. Each path once.
fn ranked(paths: Vec<String>, query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    let mut hits: Vec<(Rank, String)> = paths
        .into_iter()
        .filter_map(|path| rank(&path, &query).map(|rank| (rank, path)))
        .collect();
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.len().cmp(&b.1.len()))
            .then_with(|| a.1.cmp(&b.1))
    });
    hits.dedup();
    hits.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_query_matches_its_characters_in_order() {
        assert!(in_order("src/lib.rs", "srcli"));
        assert!(in_order("src/lib.rs", "lib"));
        assert!(!in_order("src/lib.rs", "bil"));
        assert!(!in_order("src/lib.rs", "libs2"));
    }

    #[test]
    fn matching_ignores_case_in_the_path_and_the_query() {
        assert_eq!(
            ranked(strings(&["src/Lib.RS"]), "SRCLI"),
            strings(&["src/Lib.RS"])
        );
        assert!(ranked(strings(&["src/lib.rs"]), "zzz").is_empty());
    }

    #[test]
    fn an_empty_query_matches_everything_shortest_path_first() {
        assert_eq!(
            ranked(strings(&["b/long/name", "z", "a"]), ""),
            strings(&["a", "z", "b/long/name"])
        );
    }

    #[test]
    fn better_matches_come_first() {
        let paths = strings(&[
            "l/i/b.rs",
            "a/liberty/x.rs",
            "src/domain_lib.rs",
            "src/library.rs",
            "deep/er/lib",
            "lib",
        ]);
        assert_eq!(
            ranked(paths, "lib"),
            strings(&[
                "lib",
                "deep/er/lib",
                "src/library.rs",
                "src/domain_lib.rs",
                "a/liberty/x.rs",
                "l/i/b.rs",
            ])
        );
    }

    #[test]
    fn ties_go_to_the_shorter_path_then_to_the_path() {
        assert_eq!(
            ranked(strings(&["b/lib.rs", "a/lib.rs", "lib.rs"]), "lib"),
            strings(&["lib.rs", "a/lib.rs", "b/lib.rs"])
        );
    }

    #[test]
    fn a_path_listed_twice_is_matched_once() {
        assert_eq!(
            ranked(strings(&["a.rs", "a.rs"]), "a"),
            strings(&["a.rs"])
        );
    }

    #[test]
    fn a_page_token_is_an_index() {
        assert_eq!(offset_of("").ok(), Some(0));
        assert_eq!(offset_of("40").ok(), Some(40));
        assert!(offset_of("next").is_err());
    }
}

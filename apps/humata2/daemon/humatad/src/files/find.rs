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

use std::fs::{self, Metadata};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use humata2_access::{Call, Entry};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::invalid_field::Problem;
use tonic::{Request, Response, Status};

use super::roots::{Roots, Target, io_failure};
use crate::grant_target;
use crate::guard::Guard;
use crate::identity;
use crate::status;
use crate::trash_service::blocking;

/// Entries per page when the caller names no size.
const PAGE_DEFAULT: usize = 200;

/// The most entries in one page.
const PAGE_MAX: usize = 1000;

/// The longest query, in bytes.
const QUERY_MAX: usize = 256;

/// The most files taken from a folder that isn't a git work tree.
const WALK_MAX: usize = 50_000;

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

pub(in crate::files) struct Find {
    roots: Arc<Roots>,
    guard: Arc<Guard>,
    store: Arc<Store>,
}

impl Find {
    pub(in crate::files) fn new(roots: Arc<Roots>, guard: Arc<Guard>, store: Arc<Store>) -> Self {
        Self {
            roots,
            guard,
            store,
        }
    }

    /// The files of the session's repos whose paths match the query: repos
    /// in the session's order, ranked inside each. A session with no repos
    /// has no matches. Every repo must be one the caller may read; one that
    /// isn't fails the call before any file is listed.
    pub(in crate::files) async fn find_files(
        &self,
        request: Request<pb::FindFilesRequest>,
    ) -> Result<Response<pb::FindFilesResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        if request.query.len() > QUERY_MAX {
            return Err(status::invalid_field(
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
            .ok_or_else(|| status::missing("session"))?;
        let machine = self.roots.machine();
        match self.store.session(&session) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(status::not_found(
                    pb::EntityKind::Session,
                    &session,
                    format!("no session {session} on this machine"),
                    "check the session id; sessions on other machines can't be named here yet",
                ));
            }
            Err(error) => return Err(status::store(error, machine, "look up the session")),
        }
        let repos = self
            .store
            .session_detail(&session)
            .map_err(|error| status::store(error, machine, "read the session's repositories"))?
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

        let mut targets = Vec::with_capacity(repos.len());
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
            self.guard
                .require(&caller, &Call::ReadFiles(target.place.clone()))?;
            targets.push(target);
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
        status::invalid_field(
            "page.page_token",
            Problem::BadFormat,
            "the page token isn't one this daemon gave",
            "start again without a page token",
        )
    })
}

/// One page of matches, and the index to continue from when there are more.
fn search(
    targets: &[Target],
    query: &str,
    offset: usize,
    size: usize,
) -> Result<(Vec<pb::FileEntry>, Option<usize>), Status> {
    let mut skip = offset;
    let mut consumed = 0;
    let mut more = false;
    let mut entries = Vec::new();
    for target in targets {
        // A clone that isn't on disk yet has nothing to find (RV-14).
        let root = match target.existing() {
            Ok(root) => root,
            Err(status) if status.code() == tonic::Code::NotFound => continue,
            Err(status) => return Err(status),
        };
        let matched = ranked(candidates(&root)?, query);
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
        modified_at: meta.modified().ok().map(grant_target::timestamp),
        changed: false,
        writable,
    }
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

/// Tracked files and untracked files that aren't ignored, or `None` when git
/// can't say (not a work tree, not installed, refused): the caller then
/// walks the folder.
fn git_paths(root: &Path) -> Option<Vec<String>> {
    let mut command = Command::new("git");
    command.current_dir(root);
    for name in SCRUBBED {
        command.env_remove(name);
    }
    let output = command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
        .output()
        .ok()?;
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

/// The files under `root` without following symlinks, skipping `.git`.
fn walk(root: &Path) -> Result<Vec<String>, Status> {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_owned(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let read = match fs::read_dir(&dir) {
            Ok(read) => read,
            Err(error) if dir == root => {
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
        assert_eq!(ranked(strings(&["src/Lib.RS"]), "SRCLI"), strings(&["src/Lib.RS"]));
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
                // The file's name is the query.
                "lib",
                "deep/er/lib",
                // The name starts with it.
                "src/library.rs",
                // It is inside the name.
                "src/domain_lib.rs",
                // It is in the path but not the name.
                "a/liberty/x.rs",
                // Only its characters in order.
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
        assert_eq!(ranked(strings(&["a.rs", "a.rs"]), "a"), strings(&["a.rs"]));
    }

    #[test]
    fn a_page_token_is_an_index() {
        assert_eq!(offset_of("").ok(), Some(0));
        assert_eq!(offset_of("40").ok(), Some(40));
        assert!(offset_of("next").is_err());
    }
}

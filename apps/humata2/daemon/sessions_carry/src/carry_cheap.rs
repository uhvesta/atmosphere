//! Cheap moves (MV-17, D313): what a move sends is what the new machine
//! lacks.
//!
//! - A clone whose history its remote has isn't copied as a `.git`. The old
//!   machine lists, in `clones.json` beside the manifest, how to rebuild it:
//!   the remote's address, the branch and the commit, a git bundle of the
//!   commits no remote has, the tracked files the work tree deleted, and the
//!   tracked files it didn't change (which the move leaves out). The new
//!   machine clones from its own main clone of the channel when it has one,
//!   else from the remote, applies the bundle, checks the commit out and lays
//!   the files the move did carry over it (the changed, untracked and ignored
//!   ones). The manifest then checks the result: every file but the `.git`.
//! - A file of a megabyte or more is rebuilt from 1 MiB chunks: those the
//!   new machine's chunk store holds and those the host sends (`carry_chunks.rs`).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use humata2_store::{ChannelRecord, SessionRecord, SessionRepo};
use serde::{Deserialize, Serialize};

use super::{CarryError, ManifestProblem, io, remove, sha_of_text, store};
use humata2_sessions_core::git::{git, sync_tree, text};
use humata2_sessions_core::layout::partial;
use humata2_sessions_core::start::Starter;

/// The staged file that says how to rebuild each clone.
pub const CLONES_FILE: &str = "clones.json";
/// The bundles of a stage, one per clone, as `clones/<name>.bundle`.
const BUNDLES: &str = "clones";
/// The extension, after the clone's name, of the bundle of its `refs/humata/…`.
const HUMATA_BUNDLE: &str = "humata.bundle";

/// How to rebuild the clones of a session's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Clones {
    pub(super) clones: Vec<Recipe>,
}

/// How to rebuild one clone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Recipe {
    /// The clone's folder, directly in the session's folder.
    pub(super) name: String,
    /// The repository's owner, used to locate `<channels_root>/.repo-cache/<owner>/<repo>.git`.
    #[serde(default)]
    pub(super) owner: String,
    /// The repository's name, which names its main clone in the channel.
    pub(super) repo: String,
    pub(super) remote_url: String,
    pub(super) branch: String,
    pub(super) head: String,
    /// The commits no remote has are in `clones/<name>.bundle`.
    pub(super) bundle: bool,
    /// Humata's own refs in the clone (`<sha> <ref>` per line, empty when it
    /// has none): the reviewed snapshots a pull request review keeps (RV-28,
    /// PR-21). Their commits are in `clones/<name>.humata.bundle`.
    pub(super) humata: String,
    /// Tracked files the work tree deleted.
    pub(super) deleted: Vec<String>,
    /// Tracked files the work tree didn't change: the move leaves them out
    /// and the checkout brings them.
    pub(super) clean: Vec<String>,
}

impl Clones {
    /// The `.git` folders the manifest leaves out, relative to the session's
    /// folder.
    pub(super) fn git_dirs(&self) -> Vec<String> {
        self.clones.iter().map(|recipe| format!("{}/.git", recipe.name)).collect()
    }

    /// `plain`, the digest of the folder's files, with what the clones'
    /// commits and deletions say, so two packs agree only when the history
    /// does too.
    pub(super) fn digest(&self, plain: String) -> String {
        if self.clones.is_empty() {
            return plain;
        }
        let facts: Vec<(&str, &str, &str, &Vec<String>)> = self
            .clones
            .iter()
            .map(|recipe| (recipe.name.as_str(), recipe.branch.as_str(), recipe.head.as_str(), &recipe.deleted))
            .collect();
        let humata: Vec<&str> = self.clones.iter().map(|recipe| recipe.humata.as_str()).collect();
        sha_of_text(&format!("{plain}\n{}", serde_json::to_string(&(facts, humata)).unwrap_or_default()))
    }
}

fn lines(output: &str) -> Vec<String> {
    output.split('\0').filter(|part| !part.is_empty()).map(str::to_owned).collect()
}

impl crate::CarryOps<'_> {
    /// The recipes of `record`'s clones, written into `stage` with the bundles
    /// of their unpushed commits. A clone git can't describe (no branch, no
    /// remote, submodules) isn't given one, and is copied as it is.
    pub(super) fn stage_clones(&self, record: &SessionRecord, stage: &Path) -> Result<Clones, CarryError> {
        let mut found = Clones::default();
        let detail = self.store.session_detail(&record.id).map_err(store("read the session's start"))?;
        for repo in detail.iter().flat_map(|detail| detail.repos.iter()) {
            let Some(name) = repo
                .path
                .strip_prefix(&record.folder)
                .ok()
                .and_then(Path::to_str)
                .filter(|name| !name.is_empty() && !name.contains('/'))
            else {
                continue;
            };
            if let Some(recipe) = recipe_of(repo, name, stage) {
                found.clones.push(recipe);
            }
        }
        if !found.clones.is_empty() {
            let path = stage.join(CLONES_FILE);
            let bytes = serde_json::to_vec_pretty(&found).map_err(|source| CarryError::Manifest {
                path: path.clone(),
                source: ManifestProblem::Parse(source),
            })?;
            fs::write(&path, bytes).map_err(io("write", &path))?;
        }
        Ok(found)
    }

    /// The recipes this machine was sent; none when the stage has none.
    pub(super) fn read_clones(&self, stage: &Path) -> Result<Clones, CarryError> {
        let path = stage.join(CLONES_FILE);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| CarryError::Manifest {
                path: path.clone(),
                source: ManifestProblem::Parse(source),
            }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Clones::default()),
            Err(source) => Err(io("read", &path)(source)),
        }
    }

    /// Makes each clone of `recipes` in `folder`: cloned from the channel's
    /// main clone or the remote, the bundle applied, the commit checked out,
    /// the deleted files removed and the files the move carried laid over it.
    pub(super) fn rebuild_clones(
        &self,
        recipes: &Clones,
        folder: &Path,
        channel: &ChannelRecord,
        stage: &Path,
    ) -> Result<(), CarryError> {
        let use_wt = humata2_sessions_core::worktree::use_worktrees_enabled(&self.store);
        for recipe in &recipes.clones {
            let dest = folder.join(&recipe.name);
            let building = partial(&dest);
            let fail = |source| CarryError::Rebuild { name: recipe.name.clone(), source };
            humata2_sessions_core::worktree::remove_worktrees_in_folder(&building);
            remove(&building)?;

            let owner = if !recipe.owner.is_empty() {
                Some(recipe.owner.as_str())
            } else {
                channel
                    .repos
                    .iter()
                    .find(|r| r.name == recipe.repo)
                    .map(|r| r.owner.as_str())
            };

            let built_as_worktree = if use_wt && let Some(owner) = owner {
                let bare = self.layout.repo_cache(owner, &recipe.repo);
                let lock = self.layout.repo_cache_lock(owner, &recipe.repo);
                let wt_res = humata2_sessions_core::worktree::with_repo_cache_lock(&lock, || {
                    // Each machine fetches directly from remote_url into its own bare cache.
                    humata2_sessions_core::worktree::ensure_bare_cache(
                        &bare,
                        recipe.remote_url.as_str(),
                        &mut |_| {},
                    )?;
                    if recipe.bundle {
                        let bundle = stage.join(BUNDLES).join(format!("{}.bundle", recipe.name));
                        let shown = text(&bundle)?;
                        let wanted = format!("refs/heads/{}", recipe.branch);
                        git(Some(bare.as_path()), &["fetch", "--no-tags", "--", shown, wanted.as_str()])?;
                    }
                    if humata2_sessions_core::worktree::has_gitmodules(&bare, recipe.head.as_str()) {
                        return Err(humata2_sessions_core::GitError::Failed {
                            args: "worktree add (submodules)".to_owned(),
                            status: "skipped".to_owned(),
                            stderr: "repository uses .gitmodules".to_owned(),
                        });
                    }
                    let detach = humata2_sessions_core::worktree::is_branch_checked_out(
                        &bare,
                        recipe.branch.as_str(),
                    );
                    let target = text(&building)?;
                    if detach {
                        git(
                            Some(bare.as_path()),
                            &["worktree", "add", "--no-checkout", "--detach", target, recipe.head.as_str()],
                        )?;
                    } else {
                        git(
                            Some(bare.as_path()),
                            &[
                                "worktree",
                                "add",
                                "--no-checkout",
                                "--no-track",
                                "-B",
                                recipe.branch.as_str(),
                                target,
                                recipe.head.as_str(),
                            ],
                        )?;
                    }
                    if !recipe.humata.is_empty() {
                        let bundle = stage.join(BUNDLES).join(format!("{}.{HUMATA_BUNDLE}", recipe.name));
                        let shown = text(&bundle)?;
                        git(
                            Some(building.as_path()),
                            &["fetch", "--no-tags", "--", shown, "+refs/humata/*:refs/humata/*"],
                        )?;
                    }
                    git(Some(building.as_path()), &["checkout", "-f", "HEAD"])?;
                    Ok(())
                });
                match wt_res {
                    Ok(()) => Some(bare),
                    Err(_) => {
                        humata2_sessions_core::worktree::remove_worktrees_in_folder(&building);
                        let _ = remove(&building);
                        None
                    }
                }
            } else {
                None
            };

            if built_as_worktree.is_none() {
                let main = self.layout.main_clone(&channel.folder, &recipe.repo);
                let from = if main.join(".git").is_dir() {
                    text(&main).map(str::to_owned)
                } else {
                    Ok(recipe.remote_url.clone())
                };
                let from = from.map_err(fail)?;
                let target = text(&building).map_err(fail)?;
                git(None, &["clone", "--no-checkout", "--", from.as_str(), target]).map_err(fail)?;
                git(Some(building.as_path()), &["remote", "set-url", "origin", recipe.remote_url.as_str()]).map_err(fail)?;
                if recipe.bundle {
                    let bundle = stage.join(BUNDLES).join(format!("{}.bundle", recipe.name));
                    let shown = text(&bundle).map_err(fail)?;
                    let wanted = format!("refs/heads/{}", recipe.branch);
                    git(Some(building.as_path()), &["fetch", "--no-tags", "--", shown, wanted.as_str()]).map_err(fail)?;
                }
                git(Some(building.as_path()), &["checkout", "-f", "--no-track", "-B", recipe.branch.as_str(), recipe.head.as_str()])
                    .map_err(fail)?;
                if !recipe.humata.is_empty() {
                    let bundle = stage.join(BUNDLES).join(format!("{}.{HUMATA_BUNDLE}", recipe.name));
                    let shown = text(&bundle).map_err(fail)?;
                    git(
                        Some(building.as_path()),
                        &["fetch", "--no-tags", "--", shown, "+refs/humata/*:refs/humata/*"],
                    )
                    .map_err(fail)?;
                }
            }

            let head = git(Some(building.as_path()), &["rev-parse", "HEAD"]).map_err(fail)?;
            if head.trim() != recipe.head {
                return Err(CarryError::Changed { item: format!("the commit of the clone {}", recipe.name) });
            }
            for rel in &recipe.deleted {
                remove(&building.join(rel))?;
            }
            if dest.is_dir() {
                lay_over(&dest, &building)?;
            }
            remove(&dest)?;
            // The rebuilt clone is on disk before it takes the clone's name,
            // and the name is on disk before the import records it: a power
            // cut after this leaves the clone, never only its `.partial`.
            sync_tree(&building).map_err(io("put on disk", &building))?;
            humata2_durable::rename(&building, &dest).map_err(io("place", &dest))?;
            humata2_durable::sync_dir(folder).map_err(io("put on disk", folder))?;
            if let Some(bare) = built_as_worktree {
                let lock = humata2_sessions_core::worktree::lock_path_for_bare(&bare);
                let _ = humata2_sessions_core::worktree::with_repo_cache_lock(&lock, || {
                    git(Some(bare.as_path()), &["worktree", "repair", text(&dest)?])?;
                    Ok(())
                });
            }
        }
        Ok(())
    }
}

/// How to rebuild the clone at `repo.path`, with the bundle of its unpushed
/// commits written into `stage`; none when git can't say.
fn recipe_of(repo: &SessionRepo, name: &str, stage: &Path) -> Option<Recipe> {
    let dir = repo.path.as_path();
    if !dir.join(".git").exists() || dir.join(".gitmodules").exists() {
        return None;
    }
    let out = |args: &[&str]| git(Some(dir), args).ok();
    let branch = out(&["symbolic-ref", "--short", "-q", "HEAD"])?.trim().to_owned();
    let head = out(&["rev-parse", "HEAD"])?.trim().to_owned();
    let remote_url = out(&["remote", "get-url", "origin"])?.trim().to_owned();
    if branch.is_empty() || head.is_empty() || remote_url.is_empty() {
        return None;
    }
    let ahead = format!("refs/heads/{branch}");
    let unpushed: u64 = out(&["rev-list", "--count", ahead.as_str(), "--not", "--remotes"])?.trim().parse().ok()?;
    let bundle = unpushed > 0;
    if bundle {
        let file = stage.join(BUNDLES).join(format!("{name}.bundle"));
        fs::create_dir_all(file.parent()?).ok()?;
        let shown = text(&file).ok()?;
        out(&["bundle", "create", shown, ahead.as_str(), "--not", "--remotes"])?;
    }
    let humata = out(&["for-each-ref", "--format=%(objectname) %(refname)", "refs/humata"])?;
    if !humata.trim().is_empty() {
        // Snapshots are commits no remote has: one bundle of all of them, so
        // the new machine's clone answers for a review's snapshots as this
        // one did (MV-13, PR-21). A bundle git can't make leaves the clone to
        // be copied whole, `.git` and all.
        let file = stage.join(BUNDLES).join(format!("{name}.{HUMATA_BUNDLE}"));
        fs::create_dir_all(file.parent()?).ok()?;
        let shown = text(&file).ok()?;
        let mut args = vec!["bundle", "create", shown];
        let names: Vec<&str> = humata.lines().filter_map(|line| line.split_once(' ').map(|(_, name)| name)).collect();
        args.extend(names.iter().copied());
        args.extend(["--not", "--remotes"]);
        out(&args)?;
    }
    let in_head = lines(&out(&["ls-tree", "-r", "-z", "--name-only", "HEAD"])?);
    let mut changed: BTreeSet<String> = lines(&out(&["diff", "--name-only", "-z", "HEAD"])?).into_iter().collect();
    changed.extend(lines(&out(&["ls-files", "-z", "--modified"])?));
    let deleted: Vec<String> = in_head
        .iter()
        .filter(|rel| fs::symlink_metadata(dir.join(rel)).is_err())
        .cloned()
        .collect();
    let gone: BTreeSet<&String> = deleted.iter().collect();
    let clean = in_head.iter().filter(|rel| !changed.contains(*rel) && !gone.contains(rel)).cloned().collect();
    Some(Recipe {
        name: name.to_owned(),
        owner: repo.owner.clone(),
        repo: repo.name.clone(),
        remote_url,
        branch,
        head,
        bundle,
        humata: humata.trim().to_owned(),
        deleted,
        clean,
    })
}

/// Moves every file and link under `from` onto the same place under `onto`,
/// making the folders it needs and replacing what is there.
fn lay_over(from: &Path, onto: &Path) -> Result<(), CarryError> {
    for entry in fs::read_dir(from).map_err(io("read the folder", from))? {
        let entry = entry.map_err(io("read the folder", from))?;
        let (source, target) = (entry.path(), onto.join(entry.file_name()));
        let meta = fs::symlink_metadata(&source).map_err(io("look at", &source))?;
        if meta.is_dir() {
            if fs::symlink_metadata(&target).is_ok_and(|found| !found.is_dir()) {
                remove(&target)?;
            }
            fs::create_dir_all(&target).map_err(io("create", &target))?;
            lay_over(&source, &target)?;
        } else {
            remove(&target)?;
            fs::rename(&source, &target).map_err(io("place", &target))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A clone of a remote with one commit, on `main`.
    fn clone_of_remote(root: &Path) -> std::path::PathBuf {
        let remote = root.join("remote");
        fs::create_dir_all(&remote).unwrap();
        run(&remote, &["init", "-q", "-b", "main"]);
        fs::write(remote.join("a.txt"), "a\n").unwrap();
        run(&remote, &["add", "."]);
        run(&remote, &["commit", "-q", "-m", "one"]);
        let clone = root.join("clone");
        run(root, &["clone", "-q", "remote", "clone"]);
        clone
    }

    fn repo(path: &Path) -> SessionRepo {
        SessionRepo { owner: "acme".to_owned(), name: "api".to_owned(), branch: "main".to_owned(), path: path.to_owned() }
    }

    #[test]
    fn a_clone_with_reviewed_snapshots_stages_them_in_a_bundle_of_their_own() {
        let dir = tempfile::tempdir().unwrap();
        let clone = clone_of_remote(dir.path());
        let snapshot = run(&clone, &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "what the user saw"]);
        run(&clone, &["update-ref", "refs/humata/reviewed/se_1/1", &snapshot]);
        let stage = dir.path().join("stage");

        let recipe = recipe_of(&repo(&clone), "api", &stage).expect("a clone of a remote has a recipe");

        assert!(recipe.humata.contains(&snapshot), "{}", recipe.humata);
        let bundle = stage.join(BUNDLES).join(format!("api.{HUMATA_BUNDLE}"));
        assert!(bundle.is_file());
        let listed = run(dir.path(), &["bundle", "list-heads", bundle.to_str().unwrap()]);
        assert!(listed.contains("refs/humata/reviewed/se_1/1"), "{listed}");
    }

    #[test]
    fn a_clone_without_snapshots_stages_no_bundle_of_them() {
        let dir = tempfile::tempdir().unwrap();
        let clone = clone_of_remote(dir.path());
        let stage = dir.path().join("stage");

        let recipe = recipe_of(&repo(&clone), "api", &stage).expect("a clone of a remote has a recipe");

        assert!(recipe.humata.is_empty());
        assert!(!stage.join(BUNDLES).join(format!("api.{HUMATA_BUNDLE}")).exists());
    }

    #[test]
    fn what_the_clones_hold_of_humata_is_part_of_their_digest() {
        let recipe = |humata: &str| Recipe {
            name: "api".to_owned(),
            owner: "acme".to_owned(),
            repo: "api".to_owned(),
            remote_url: "u".to_owned(),
            branch: "b".to_owned(),
            head: "h".to_owned(),
            bundle: false,
            humata: humata.to_owned(),
            deleted: Vec::new(),
            clean: Vec::new(),
        };
        let one = Clones { clones: vec![recipe("")] }.digest("plain".to_owned());
        let two = Clones { clones: vec![recipe("abc refs/humata/x")] }.digest("plain".to_owned());
        assert_ne!(one, two);
    }
}

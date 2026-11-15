//! Reviewed snapshots (RV-28, RV-30): what the user saw, as a commit whose
//! tree is the files as shown and whose parent is HEAD, under
//! `refs/humata/reviewed/<session>/<n>`. A pin is a second ref,
//! `refs/humata/pinned/<session>/<n>`, so pruning and pinning live in git
//! with the snapshot and move with the clone.

use std::ffi::OsStr;
use std::path::Path;

use crate::checkout::Checkout;
use crate::error::DiffError;
use crate::git::trimmed;
use crate::model::Snapshot;
use crate::spec::valid_session;

/// How many unpinned snapshots a session keeps per repo (RV-30).
pub const KEPT_SNAPSHOTS: usize = 20;

const REVIEWED: &str = "refs/humata/reviewed";
const PINNED: &str = "refs/humata/pinned";
/// Where the commit another session's clone is on is fetched to.
const OTHER: &str = "refs/humata/other";

impl Checkout {
    /// Records `shown_tree` (the new side of the diff the user was looking
    /// at, [`crate::Trees::new`]) as session's next reviewed snapshot, and
    /// drops unpinned snapshots beyond the last [`KEPT_SNAPSHOTS`].
    /// Touches nothing but objects and the session's `refs/humata/…`.
    ///
    /// # Errors
    ///
    /// [`DiffError::BadSession`] for a session id unfit for a ref name;
    /// [`DiffError`] when git fails.
    pub fn mark_reviewed_blocking(
        &self,
        session: &str,
        shown_tree: &str,
    ) -> Result<Snapshot, DiffError> {
        check_session(session)?;
        let head = self.head()?;
        let mut args = vec!["commit-tree".to_owned(), shown_tree.to_owned()];
        if let Some(head) = &head {
            args.push("-p".to_owned());
            args.push(head.clone());
        }
        args.push("-m".to_owned());
        args.push(format!("Humata: reviewed by the user in session {session}"));
        let commit = trimmed(&self.git.run(&args)?);
        // Another daemon call may take the same number at the same time;
        // creating the ref only if it doesn't exist makes one of them retry.
        let mut n = self.snapshots_blocking(session)?.last().map_or(1, |last| last.n + 1);
        loop {
            let name = format!("{REVIEWED}/{session}/{n}");
            let output = self
                .git
                .output(["update-ref", "--no-deref", &name, &commit, ""], None)?;
            if output.status.success() {
                break;
            }
            if self.git.resolve(&name)?.is_none() {
                return Err(self.git.failed(["update-ref", &name, &commit], &output));
            }
            n += 1;
        }
        self.prune(session)?;
        self.snapshots_blocking(session)?
            .into_iter()
            .find(|snapshot| snapshot.n == n)
            .ok_or_else(|| DiffError::SnapshotMissing {
                repo: self.path().to_owned(),
                session: session.to_owned(),
                n,
            })
    }

    /// The session's snapshots of this repo, oldest first.
    ///
    /// # Errors
    ///
    /// [`DiffError::BadSession`], or [`DiffError`] when git fails.
    pub fn snapshots_blocking(&self, session: &str) -> Result<Vec<Snapshot>, DiffError> {
        check_session(session)?;
        let pinned = self.refs_named(&format!("{PINNED}/{session}/"))?;
        let prefix = format!("{REVIEWED}/{session}/");
        let listing = self.git.run([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(tree)%00%(parent)%00%(committerdate:unix)",
            &prefix,
        ])?;
        let mut snapshots: Vec<Snapshot> = String::from_utf8_lossy(&listing)
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\0').collect();
                let [name, commit, tree, parent, at] = fields.as_slice() else {
                    return None;
                };
                let n: u32 = name.strip_prefix(&prefix)?.parse().ok()?;
                Some(Snapshot {
                    n,
                    commit: (*commit).to_owned(),
                    tree: (*tree).to_owned(),
                    head: (!parent.is_empty()).then(|| (*parent).to_owned()),
                    taken_at: at.parse().unwrap_or(0),
                    pinned: pinned.contains(&n),
                })
            })
            .collect();
        snapshots.sort_by_key(|snapshot| snapshot.n);
        Ok(snapshots)
    }

    /// Keeps snapshot `n` past the last 20, as RV-30 does for every one a
    /// comment was sent from; `false` lets it go again.
    ///
    /// # Errors
    ///
    /// [`DiffError::SnapshotMissing`] when there is no snapshot `n`.
    pub fn pin_blocking(&self, session: &str, n: u32, pinned: bool) -> Result<(), DiffError> {
        let commit = self.snapshot_ref(session, Some(n))?;
        let name = format!("{PINNED}/{session}/{n}");
        if pinned {
            self.git.run(["update-ref", "--no-deref", &name, &commit])?;
        } else {
            self.git.run(["update-ref", "-d", &name])?;
            self.prune(session)?;
        }
        Ok(())
    }

    /// Fetches the commit HEAD is on in `session`'s own clone of this repo,
    /// at `from`, into this clone as `refs/humata/other/<session>`, and
    /// returns it, to compare against (RV-1's other session). `from` is
    /// only read.
    ///
    /// # Errors
    ///
    /// [`DiffError::OtherSession`] when `from` isn't a clone with a commit.
    pub fn fetch_session_head_blocking(&self, from: &Path, session: &str) -> Result<String, DiffError> {
        check_session(session)?;
        let name = format!("{OTHER}/{session}");
        let refspec = format!("+HEAD:{name}");
        let args = [
            OsStr::new("fetch"),
            OsStr::new("--quiet"),
            OsStr::new("--no-tags"),
            OsStr::new("--no-write-fetch-head"),
            from.as_os_str(),
            OsStr::new(&refspec),
        ];
        let output = self.git.output(args, None)?;
        if !output.status.success() {
            return Err(DiffError::OtherSession {
                session: session.to_owned(),
                from: from.to_owned(),
                status: output.status,
                stderr: trimmed(&output.stderr),
            });
        }
        self.commit(&name)
    }

    /// The commit of snapshot `n`, or of the latest for `None`.
    pub(crate) fn snapshot_ref(&self, session: &str, n: Option<u32>) -> Result<String, DiffError> {
        check_session(session)?;
        match n {
            Some(n) => self
                .git
                .resolve(&format!("{REVIEWED}/{session}/{n}^{{commit}}"))?
                .ok_or_else(|| DiffError::SnapshotMissing {
                    repo: self.path().to_owned(),
                    session: session.to_owned(),
                    n,
                }),
            None => self
                .snapshots_blocking(session)?
                .pop()
                .map(|snapshot| snapshot.commit)
                .ok_or_else(|| DiffError::NoSnapshot {
                    repo: self.path().to_owned(),
                    session: session.to_owned(),
                }),
        }
    }

    fn prune(&self, session: &str) -> Result<(), DiffError> {
        let snapshots = self.snapshots_blocking(session)?;
        let unpinned: Vec<&Snapshot> = snapshots.iter().filter(|s| !s.pinned).collect();
        let excess = unpinned.len().saturating_sub(KEPT_SNAPSHOTS);
        if excess == 0 {
            return Ok(());
        }
        let commands: String = unpinned[..excess]
            .iter()
            .map(|snapshot| format!("delete {REVIEWED}/{session}/{}\n", snapshot.n))
            .collect();
        self.git
            .run_with_input(["update-ref", "--stdin"], commands.as_bytes())?;
        Ok(())
    }

    fn refs_named(&self, prefix: &str) -> Result<Vec<u32>, DiffError> {
        let listing = self
            .git
            .run(["for-each-ref", "--format=%(refname)", prefix])?;
        Ok(String::from_utf8_lossy(&listing)
            .lines()
            .filter_map(|name| name.strip_prefix(prefix)?.parse().ok())
            .collect())
    }
}

fn check_session(session: &str) -> Result<(), DiffError> {
    if valid_session(session) {
        Ok(())
    } else {
        Err(DiffError::BadSession {
            session: session.to_owned(),
        })
    }
}

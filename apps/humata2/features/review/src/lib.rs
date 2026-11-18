//! Reviewing a session's changes (RV-1–27): files fold, and lines wrap or
//! scroll. Comments on lines belong to the daemon and live in [`comments`];
//! this page only hands over what older builds saved here (D151).
//!
//! The rows the view draws are prepared off the UI thread by an effect and
//! arrive ready to draw ([`ReviewAction::Prepared`]); a newer request
//! replaces an older one still in flight.

pub mod comments;
pub mod order;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::views::PlanView;
use humata2_clients::{Daemon, DaemonError, Workspace};
use humata2_clients::model::{
    Comment, DiffBase, DiffLine, DiffTargets, PushChoice, RepoBase, RepoBases, RepoDiff,
    SessionId, Status,
};
use serde::{Deserialize, Serialize};
use tca::{Clock, Effect, Reducer};

use comments::HandOver;

/// A line on the review page, ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    /// A file's header.
    File {
        /// Its path.
        path: String,
        /// Lines added.
        added: u32,
        /// Lines removed.
        removed: u32,
        /// Whether its lines show.
        open: bool,
    },
    /// A diff line.
    Line {
        /// The file.
        path: String,
        /// The line.
        line: DiffLine,
    },
}

/// One row of a review plan's reading order, ready to draw (RV-32, P8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanRow {
    /// A titled section starts here.
    Group {
        /// Its title.
        title: String,
        /// What the agent says about it.
        note: String,
    },
    /// A changed file. `repo` is the diff's own spelling of it.
    File {
        /// The file's repository, as the diff names it.
        repo: String,
        /// The file.
        path: String,
        /// What the agent says about it, in the plan's order.
        notes: Vec<PlanRowNote>,
    },
}

/// A note of the agent's under a file of a review plan (RV-32, RV-34).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRowNote {
    /// What the agent says.
    pub text: String,
    /// The lines it is on, first and last, where they are now; `None` for a
    /// note on the whole file.
    pub lines: Option<(u32, u32)>,
    /// Its lines changed since the agent wrote it.
    pub outdated: bool,
}

/// Whether plan key `a` and diff repo `b` name one repository: the same, or
/// one is the other with an `owner/` before it.
fn same_repo(a: &str, b: &str) -> bool {
    let after = |long: &str, short: &str| {
        long.strip_suffix(short)
            .is_some_and(|rest| rest.ends_with('/'))
    };
    a == b || after(a, b) || after(b, a)
}

/// Where a comment goes. Only older builds' saved drafts still name one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Anchor {
    /// The file.
    pub path: String,
    /// The line in the new file.
    pub line: u32,
}

/// What survives a restart: the selected file and folded files. Comments
/// older builds kept here are read once and handed to the daemon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSaved {
    /// The selected repository and file, per session.
    #[serde(default)]
    pub selected: BTreeMap<SessionId, (String, String)>,
    /// Folded files per session.
    pub folded: BTreeMap<SessionId, BTreeSet<String>>,
    /// Comments older builds wrote but did not send, per session. Never
    /// written any more; removed as the daemon takes each.
    #[serde(default, rename = "queued", skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy_queued: BTreeMap<SessionId, Vec<Comment>>,
    /// A comment older builds were writing, per session. Never written any
    /// more; removed as the daemon takes it.
    #[serde(default, rename = "drafts", skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy_drafts: BTreeMap<SessionId, (Anchor, String)>,
    /// Each session's diff targets, saved only where they differ from the
    /// default: the base each repository's changes are compared with (D5).
    /// Saved files from before this field load with none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<SessionId, DiffTargets>,
}

/// What the page knows about its reads of one session's changes (D179).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fetch {
    /// A read is in flight.
    pub loading: bool,
    /// Something asked for another read while one was in flight.
    pub queued: bool,
    /// When the diff shown was read, on the clock.
    pub read_at: Option<Duration>,
    /// Why the last read or Mark reviewed failed, as the panel says it.
    pub error: Option<String>,
    /// The last diff read.
    pub diffs: Option<Vec<RepoDiff>>,
    /// The repository the last read couldn't compare with its default
    /// branch, because its clone doesn't know it (frame 98).
    pub no_default: Option<String>,
    /// Humata is asking that repository's remote for its default branch.
    pub finding: bool,
    /// Why the last Find the default branch failed.
    pub find_error: Option<String>,
}

/// Where pushing one repository of a session's changes stands (RV-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushState {
    /// The push is running.
    Pushing,
    /// Pushed; `pull_request` is the pull request open for its branch, if any.
    Pushed {
        /// The pull request's number.
        pull_request: Option<u32>,
        /// What the push says about a pull request it left on another base,
        /// such as that it targets main (VIEWS P5); empty otherwise.
        note: String,
    },
    /// The push didn't finish: why, and what to do.
    Failed(String),
}

/// One repository's base menu while it is open (RV-31, VIEWS P3): the
/// branches its clone knew at the last fetch, and the filter typed over them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoPicker {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// Its branches, or why they couldn't be read; `None` while they are read.
    pub bases: Option<Result<RepoBases, String>>,
    /// What the filter field holds: a part of a branch name, or a branch or
    /// commit to compare with.
    pub filter: String,
}

impl RepoPicker {
    /// The branches the filter keeps, in the clone's order (the default
    /// branch first). The filter matches anywhere in a name, ignoring case.
    #[must_use]
    pub fn branches(&self) -> Vec<&str> {
        let wanted = self.filter.trim().to_lowercase();
        match &self.bases {
            Some(Ok(bases)) => bases
                .branches
                .iter()
                .map(String::as_str)
                .filter(|branch| branch.to_lowercase().contains(&wanted))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// The review page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewState {
    /// Where the diffs come from.
    pub workspace: Arc<Workspace>,
    /// The session whose changes show.
    pub session: Option<SessionId>,
    /// The rows to draw, once prepared.
    pub rows: Option<Arc<Vec<DiffRow>>>,
    /// Which preparation the rows are from; older results are dropped.
    pub generation: u64,
    /// Long lines wrap instead of scrolling (RV-9).
    pub wrap: bool,
    /// What is saved.
    pub saved: ReviewSaved,
    /// What each session's changes are compared with: a preset and any
    /// per-repository bases (RV-1, RV-31). Saved with the rest of this page.
    pub targets: HashMap<SessionId, DiffTargets>,
    /// The reads of each session's changes.
    pub fetch: HashMap<SessionId, Fetch>,
    /// Whether the base menu under the Diff header is open (RV-1).
    pub base_menu: bool,
    /// The one repository base menu open, if any (RV-31).
    pub repo_picker: Option<RepoPicker>,
    /// Where each repository's push of a session's changes stands.
    pub pushes: HashMap<(SessionId, String), PushState>,
    /// The review plan each session's view opened, while it shows.
    pub plans: HashMap<SessionId, PlanView>,
    /// The repositories folded per session, as `(session, repo)`: their
    /// files are hidden while they are in here. Not saved (DV-7).
    pub collapsed: HashSet<(SessionId, String)>,
}

impl ReviewState {
    /// The rows of `session`'s changes in its review plan's order, or `None`
    /// when it has no plan. A group's title and note come before its first
    /// file; files the order leaves out come last; entries the diff lacks
    /// are skipped.
    #[must_use]
    pub fn plan_rows(&self, session: &SessionId) -> Option<Vec<PlanRow>> {
        let plan = self.plans.get(session)?;
        let read = self.fetch.get(session).and_then(|fetch| fetch.diffs.as_ref());
        let diffs: Vec<&RepoDiff> = match read {
            Some(diffs) => diffs.iter().filter(|diff| &diff.session == session).collect(),
            None => self
                .workspace
                .diffs
                .iter()
                .filter(|diff| &diff.session == session)
                .collect(),
        };
        let files: Vec<(&str, &str)> = diffs
            .iter()
            .flat_map(|diff| {
                diff.files
                    .iter()
                    .map(move |file| (diff.repo.as_str(), file.path.as_str()))
            })
            .collect();
        let notes_of = |repo: &str, path: &str| -> Vec<PlanRowNote> {
            plan.notes
                .iter()
                .filter(|note| note.path == path && same_repo(&note.repo, repo))
                .map(|note| PlanRowNote {
                    text: note.text.clone(),
                    lines: note.lines,
                    outdated: note.outdated,
                })
                .collect()
        };
        let mut used = vec![false; files.len()];
        let mut met: Vec<(&str, &str)> = Vec::new();
        let mut pending: Vec<PlanRow> = Vec::new();
        let mut rows = Vec::new();
        for (repo, path) in &plan.order {
            if met.contains(&(repo.as_str(), path.as_str())) {
                continue;
            }
            met.push((repo.as_str(), path.as_str()));
            for group in plan
                .groups
                .iter()
                .filter(|group| group.first.0 == *repo && group.first.1 == *path)
            {
                pending.push(PlanRow::Group {
                    title: group.title.clone(),
                    note: group.note.clone(),
                });
            }
            let at = files
                .iter()
                .enumerate()
                .position(|(index, (held, file))| {
                    !used[index] && *file == path.as_str() && same_repo(repo, held)
                });
            if let Some(at) = at {
                used[at] = true;
                rows.append(&mut pending);
                rows.push(PlanRow::File {
                    repo: files[at].0.to_owned(),
                    path: path.clone(),
                    notes: notes_of(repo, path),
                });
            }
        }
        for (index, (repo, path)) in files.iter().enumerate() {
            if !used[index] {
                rows.push(PlanRow::File {
                    repo: (*repo).to_owned(),
                    path: (*path).to_owned(),
                    notes: notes_of(repo, path),
                });
            }
        }
        Some(rows)
    }

    /// The diff targets of `session`: the bases its changes are compared
    /// with, or its default when none were picked.
    #[must_use]
    pub fn targets_of(&self, session: &SessionId) -> DiffTargets {
        self.targets
            .get(session)
            .cloned()
            .unwrap_or_else(|| self.default_targets(session))
    }

    /// Whether `session`'s repositories are compared with bases that differ,
    /// so the Diff header reads "Comparing per repo" (VIEWS P1). Overrides
    /// that all come to the same base don't count.
    #[must_use]
    pub fn compares_per_repo(&self, session: &SessionId) -> bool {
        let targets = self.targets_of(session);
        if !targets.per_repo() {
            return false;
        }
        let repos = repos_of(self, session);
        let mut bases = repos.iter().map(|repo| targets.of(repo));
        let first = bases.next();
        bases.any(|base| Some(base) != first)
    }

    /// What `session`'s changes are compared with before any pick: each
    /// pull request's base for a pull request review (PR-10), else
    /// origin/main.
    fn default_targets(&self, session: &SessionId) -> DiffTargets {
        let reviews = self
            .workspace
            .session(session)
            .is_some_and(|each| each.pull_request_review);
        DiffTargets {
            preset: if reviews {
                DiffBase::PullRequestBase
            } else {
                DiffBase::OriginMain
            },
            ..DiffTargets::default()
        }
    }

    /// Takes what was saved: the folds, the older comments and each
    /// session's diff targets, which become the targets in use.
    pub fn load_saved(&mut self, saved: ReviewSaved) {
        self.targets = saved.targets.clone().into_iter().collect();
        self.saved = saved;
    }
}

/// What the review page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// These comments older builds saved are the daemon's to keep now.
    HandOver(HandOver),
    /// The daemon read these changes for the session: they replace what
    /// the app holds for it.
    Fetched(SessionId, Vec<RepoDiff>),
}

/// What happens on the review page.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewAction {
    /// Show a file from the open session's comparison.
    SelectFile { repo: String, path: String },
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved folds and older comments came back at launch.
    Restore(ReviewSaved),
    /// Show this session's changes.
    Open(SessionId),
    /// Rows were prepared.
    Prepared(u64, Arc<Vec<DiffRow>>),
    /// A file header was clicked.
    ToggleFile(String),
    /// A repository's chevron was clicked in the open session: fold its
    /// files, or unfold them if they are folded (DV-7).
    ToggleRepo {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// Wrap long lines, or not.
    ToggleWrap,
    /// The daemon took this saved comment: forget it.
    HandedOver(SessionId, Comment),
    /// Compare the open session's changes with another base (RV-1).
    PickBase(DiffBase),
    /// Give one repository of `session` its own base; its other repositories
    /// keep theirs (RV-31).
    PickRepoBase {
        /// The session.
        session: SessionId,
        /// The repository, as `owner/name`.
        repo: String,
        /// What its changes are compared with.
        base: RepoBase,
    },
    /// Open one repository's base menu, or close it if it is open; any
    /// other menu closes (RV-31).
    ToggleRepoBase {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// Close the repository base menu.
    CloseRepoBase,
    /// Read the open repository menu's branches again.
    ReloadRepoBases,
    /// The filter over the open repository menu's branches changed.
    FilterRepoBases(String),
    /// A read of one repository's branches ended.
    RepoBasesRead {
        /// The session.
        session: SessionId,
        /// The repository.
        repo: String,
        /// Its branches, or why they couldn't be read.
        result: Result<RepoBases, String>,
    },
    /// Open the base menu, or close it if it is open (RV-1).
    ToggleBaseMenu,
    /// Close the base menu.
    CloseBaseMenu,
    /// Mark the open session's changes reviewed (RV-6).
    MarkReviewed,
    /// Read the open session's changes again.
    Retry,
    /// Commit, push and open a pull request for one repository of the open
    /// session's changes (RV-2).
    Push {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// A push of one repository ended.
    Pushed {
        /// The session pushed.
        session: SessionId,
        /// The repository pushed.
        repo: String,
        /// The pull request the push opened or found and the push's note
        /// about it, or why it failed.
        result: Result<(Option<u32>, String), String>,
    },
    /// Have Humata ask `repo`'s remote for its default branch in the open
    /// session's clone (frame 98), then read the changes again.
    FindDefaultBranch {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// Finding a repository's default branch ended: the branch, or why not.
    DefaultBranchFound {
        session: SessionId,
        repo: String,
        result: Result<String, String>,
    },
    /// A read of a session's changes ended.
    Fetched {
        session: SessionId,
        base: DiffBase,
        result: Result<Vec<RepoDiff>, DaemonError>,
    },
    /// Marking a session reviewed ended.
    Reviewed {
        session: SessionId,
        result: Result<(), String>,
    },
    /// For the parent.
    Delegate(Delegate),
}

/// The review page's reducer.
#[derive(Clone)]
pub struct Review {
    daemon: Arc<dyn Daemon>,
    clock: Arc<dyn Clock>,
}

impl Review {
    /// A reducer that reads changes from `daemon` and stamps reads on `clock`.
    #[must_use]
    pub fn new(daemon: Arc<dyn Daemon>, clock: Arc<dyn Clock>) -> Self {
        Self { daemon, clock }
    }

    /// Reads `session`'s changes, or queues a read behind the one in flight.
    fn fetch(&self, state: &mut ReviewState, session: &SessionId) -> Effect<ReviewAction> {
        let fetch = state.fetch.entry(session.clone()).or_default();
        if fetch.loading {
            fetch.queued = true;
            return Effect::none();
        }
        fetch.loading = true;
        let base = state.targets_of(session).base(&repos_of(state, session));
        let reply = self.daemon.diffs(session, &base);
        let session = session.clone();
        Effect::future(async move {
            ReviewAction::Fetched {
                session,
                base,
                result: reply.await,
            }
        })
    }

    /// Reads the branches of the open repository menu's repository.
    fn read_bases(&self, state: &mut ReviewState) -> Effect<ReviewAction> {
        let (Some(session), Some(picker)) = (state.session.clone(), state.repo_picker.as_mut())
        else {
            return Effect::none();
        };
        picker.bases = None;
        let repo = picker.repo.clone();
        let reply = self.daemon.bases(&session, &repo);
        Effect::future(async move {
            ReviewAction::RepoBasesRead {
                session,
                repo,
                result: reply.await.map_err(|error| error.to_string()),
            }
        })
    }

    /// Pushes one repository of `session`'s changes: plans the push, then
    /// commits and pushes it and opens its pull request if it has none.
    fn push(&self, state: &ReviewState, session: SessionId, repo: String) -> Effect<ReviewAction> {
        let daemon = self.daemon.clone();
        let title = state
            .workspace
            .sessions
            .iter()
            .find(|each| each.id == session)
            .map(|each| each.title.clone())
            .unwrap_or_default();
        // A repository compared with a branch of its own opens its pull
        // request against that branch (VIEWS P5).
        let base_branch = match state.targets_of(&session).of(&repo) {
            RepoBase::Branch(branch) => branch
                .strip_prefix("origin/")
                .unwrap_or(branch.as_str())
                .to_owned(),
            _ => String::new(),
        };
        Effect::future(async move {
            let result = push_repo(daemon.as_ref(), &session, &repo, &title, base_branch)
                .await
                .map_err(|error| error.to_string());
            ReviewAction::Pushed {
                session,
                repo,
                result,
            }
        })
    }
}

/// Pushes `repo` as the session's current plan has it: commits its
/// uncommitted files when it has any, and opens a pull request when its
/// branch has none, against `base_branch` when it is set. Returns the pull
/// request for the branch, if there is one, and what the push says about it.
async fn push_repo(
    daemon: &dyn Daemon,
    session: &SessionId,
    repo: &str,
    title: &str,
    base_branch: String,
) -> Result<(Option<u32>, String), DaemonError> {
    let plan = daemon.plan_push(session).await?;
    let Some(found) = plan.repos.into_iter().find(|each| each.repo == repo) else {
        return Err(DaemonError::Refused {
            what: format!("Pushing {repo}"),
            reason: format!(
                "{repo} has no changes in the session's plan now. Read the changes again, then push it from there"
            ),
        });
    };
    let choice = PushChoice {
        repo: found.repo,
        commit_message: if found.dirty_files > 0 {
            format!("Changes from {title}")
        } else {
            String::new()
        },
        open_pull_request: found.open_pull_request.is_none(),
        title: title.to_owned(),
        body: String::new(),
        base_branch,
    };
    let opened = daemon.push(session, &plan.token, vec![choice]).await?;
    let pushed = opened.into_iter().find(|pull_request| pull_request.repo == repo);
    let note = pushed
        .as_ref()
        .map(|pull_request| pull_request.note.clone())
        .unwrap_or_default();
    let number = pushed.map(|pull_request| pull_request.number);
    Ok((number.or(found.open_pull_request), note))
}

/// The repositories `session` works in: its own, those its changes were
/// read for, and any with a base of its own, so a pick is never dropped for
/// want of a read. Sorted, so a spec lists them the same way each time.
fn repos_of(state: &ReviewState, session: &SessionId) -> Vec<String> {
    let mut repos: BTreeSet<String> = BTreeSet::new();
    repos.extend(
        state
            .workspace
            .sessions
            .iter()
            .filter(|each| &each.id == session)
            .map(|each| each.repo.clone())
            .filter(|repo| !repo.is_empty()),
    );
    repos.extend(
        state
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == session)
            .map(|diff| diff.repo.clone()),
    );
    if let Some(targets) = state.targets.get(session) {
        repos.extend(targets.repos.keys().cloned());
    }
    repos.into_iter().collect()
}

/// Copies the diff targets that differ from their session's default into
/// what is saved.
fn sync_saved(state: &mut ReviewState) {
    let saved = state
        .targets
        .iter()
        .filter(|(session, targets)| **targets != state.default_targets(session))
        .map(|(session, targets)| (session.clone(), targets.clone()))
        .collect();
    state.saved.targets = saved;
}

fn working(workspace: &Workspace, session: &SessionId) -> bool {
    workspace
        .sessions
        .iter()
        .any(|each| &each.id == session && each.status == Status::Working)
}

/// Makes `diffs` the changes the workspace holds for `session`.
fn replace_diffs(workspace: &mut Arc<Workspace>, session: &SessionId, diffs: &[RepoDiff]) {
    let same = workspace
        .diffs
        .iter()
        .filter(|diff| &diff.session == session)
        .eq(diffs.iter());
    if same {
        return;
    }
    let workspace = Arc::make_mut(workspace);
    workspace.diffs.retain(|diff| &diff.session != session);
    workspace.diffs.extend(diffs.iter().cloned());
}

impl Reducer for Review {
    type State = ReviewState;
    type Action = ReviewAction;

    fn reduce(&self, state: &mut ReviewState, action: ReviewAction) -> Effect<ReviewAction> {
        match action {
            ReviewAction::SelectFile { repo, path } => {
                if let Some(session) = state.session.clone()
                    && state.workspace.diffs.iter().any(|diff| {
                        diff.session == session
                            && diff.repo == repo
                            && diff.files.iter().any(|file| file.path == path)
                    })
                {
                    state.saved.selected.insert(session, (repo, path));
                }
            }
            ReviewAction::Show(workspace) => {
                let turn_ended = state.session.clone().filter(|session| {
                    working(&state.workspace, session) && !working(&workspace, session)
                });
                state.workspace = workspace;
                let shown: Vec<(SessionId, Vec<RepoDiff>)> = state
                    .fetch
                    .iter()
                    .filter_map(|(session, fetch)| Some((session.clone(), fetch.diffs.clone()?)))
                    .collect();
                for (session, diffs) in &shown {
                    replace_diffs(&mut state.workspace, session, diffs);
                }
                let mut effects = vec![prepare(state)];
                if let Some(session) = turn_ended {
                    effects.push(self.fetch(state, &session));
                }
                return Effect::merge(effects);
            }
            ReviewAction::PickBase(base) => {
                state.base_menu = false;
                state.repo_picker = None;
                if let Some(session) = state.session.clone() {
                    state
                        .targets
                        .entry(session.clone())
                        .or_default()
                        .pick_all(base);
                    sync_saved(state);
                    return self.fetch(state, &session);
                }
            }
            ReviewAction::PickRepoBase {
                session,
                repo,
                base,
            } => {
                state.repo_picker = None;
                // The others keep what they were compared with before the
                // pick: a pull request's base for a review (PR-10).
                let before = state.targets_of(&session);
                state
                    .targets
                    .entry(session.clone())
                    .or_insert(before)
                    .pick(&repo, base);
                sync_saved(state);
                return self.fetch(state, &session);
            }
            ReviewAction::Retry => {
                if let Some(session) = state.session.clone() {
                    return self.fetch(state, &session);
                }
            }
            ReviewAction::FindDefaultBranch { repo } => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                let fetch = state.fetch.entry(session.clone()).or_default();
                if fetch.finding {
                    return Effect::none();
                }
                fetch.finding = true;
                fetch.find_error = None;
                let reply = self.daemon.find_default_branch(&session, &repo);
                return Effect::future(async move {
                    ReviewAction::DefaultBranchFound {
                        session,
                        repo,
                        result: reply.await.map_err(|error| error.to_string()),
                    }
                });
            }
            ReviewAction::DefaultBranchFound { session, repo, result } => {
                let fetch = state.fetch.entry(session.clone()).or_default();
                fetch.finding = false;
                match result {
                    Ok(_) => {
                        if fetch.no_default.as_deref() == Some(repo.as_str()) {
                            fetch.no_default = None;
                        }
                        return self.fetch(state, &session);
                    }
                    Err(reason) => fetch.find_error = Some(reason),
                }
            }
            ReviewAction::ToggleRepoBase { repo } => {
                if state.repo_picker.as_ref().is_some_and(|picker| picker.repo == repo) {
                    state.repo_picker = None;
                } else if state.session.is_some() {
                    state.base_menu = false;
                    state.repo_picker = Some(RepoPicker {
                        repo,
                        bases: None,
                        filter: String::new(),
                    });
                    return self.read_bases(state);
                }
            }
            ReviewAction::CloseRepoBase => state.repo_picker = None,
            ReviewAction::ReloadRepoBases => return self.read_bases(state),
            ReviewAction::FilterRepoBases(filter) => {
                if let Some(picker) = state.repo_picker.as_mut() {
                    picker.filter = filter;
                }
            }
            ReviewAction::RepoBasesRead {
                session,
                repo,
                result,
            } => {
                if state.session.as_ref() == Some(&session)
                    && let Some(picker) = state.repo_picker.as_mut()
                    && picker.repo == repo
                {
                    picker.bases = Some(result);
                }
            }
            ReviewAction::ToggleBaseMenu => {
                state.base_menu = !state.base_menu;
                state.repo_picker = None;
            }
            ReviewAction::CloseBaseMenu => state.base_menu = false,
            ReviewAction::MarkReviewed => {
                state.base_menu = false;
                if let Some(session) = state.session.clone() {
                    let reply = self.daemon.mark_reviewed(&session);
                    return Effect::future(async move {
                        ReviewAction::Reviewed {
                            session,
                            result: reply.await.map_err(|error| error.to_string()),
                        }
                    });
                }
            }
            ReviewAction::Reviewed { session, result } => match result {
                Ok(()) => return self.fetch(state, &session),
                Err(error) => state.fetch.entry(session).or_default().error = Some(error),
            },
            ReviewAction::Push { repo } => {
                if let Some(session) = state.session.clone() {
                    let key = (session.clone(), repo.clone());
                    if state.pushes.get(&key) == Some(&PushState::Pushing) {
                        return Effect::none();
                    }
                    state.pushes.insert(key, PushState::Pushing);
                    return self.push(state, session, repo);
                }
            }
            ReviewAction::Pushed {
                session,
                repo,
                result,
            } => {
                let outcome = match result {
                    Ok((pull_request, note)) => PushState::Pushed { pull_request, note },
                    Err(error) => PushState::Failed(error),
                };
                state.pushes.insert((session, repo), outcome);
            }
            ReviewAction::Fetched { session, base, result } => {
                let current = state.targets_of(&session).base(&repos_of(state, &session));
                let fetch = state.fetch.entry(session.clone()).or_default();
                fetch.loading = false;
                let queued = std::mem::take(&mut fetch.queued);
                let mut effects = Vec::new();
                if base == current {
                    match result {
                        Ok(diffs) => {
                            fetch.error = None;
                            fetch.no_default = None;
                            fetch.read_at = Some(self.clock.now());
                            fetch.diffs = Some(diffs.clone());
                            replace_diffs(&mut state.workspace, &session, &diffs);
                            effects.push(prepare(state));
                            effects.push(Effect::send(ReviewAction::Delegate(Delegate::Fetched(
                                session.clone(),
                                diffs,
                            ))));
                        }
                        Err(error) => {
                            fetch.no_default = match &error {
                                DaemonError::NoDefaultBranch { repo } => Some(repo.clone()),
                                _ => None,
                            };
                            fetch.error = Some(error.to_string());
                        }
                    }
                }
                if queued {
                    effects.push(self.fetch(state, &session));
                }
                return Effect::merge(effects);
            }
            ReviewAction::Restore(saved) => {
                state.load_saved(saved);
                let handovers = hand_overs(state);
                if !handovers.is_empty() {
                    return Effect::merge(
                        handovers
                            .into_iter()
                            .map(|hand| {
                                Effect::send(ReviewAction::Delegate(Delegate::HandOver(hand)))
                            })
                            .collect::<Vec<_>>(),
                    );
                }
            }
            ReviewAction::Open(session) => {
                if state.session.as_ref() != Some(&session) {
                    state.session = Some(session.clone());
                    state.rows = None;
                    state.base_menu = false;
                    state.repo_picker = None;
                }
                // Opening again while its read is in flight asks for
                // nothing new: the files may not have changed since it
                // started, unlike a turn end or Mark reviewed.
                if state.fetch.get(&session).is_some_and(|fetch| fetch.loading) {
                    return prepare(state);
                }
                return Effect::merge(vec![prepare(state), self.fetch(state, &session)]);
            }
            ReviewAction::Prepared(generation, rows) => {
                if generation == state.generation {
                    state.rows = Some(rows);
                }
            }
            ReviewAction::ToggleFile(path) => {
                if let Some(session) = state.session.clone() {
                    let folded = state.saved.folded.entry(session).or_default();
                    if !folded.remove(&path) {
                        folded.insert(path);
                    }
                    return prepare(state);
                }
            }
            ReviewAction::ToggleRepo { repo } => {
                if let Some(session) = state.session.clone() {
                    let key = (session, repo);
                    if !state.collapsed.remove(&key) {
                        state.collapsed.insert(key);
                    }
                }
            }
            ReviewAction::ToggleWrap => state.wrap = !state.wrap,
            ReviewAction::HandedOver(session, comment) => forget(state, &session, &comment),
            ReviewAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

/// What older builds saved, one hand-over per session: its unsent comments
/// in the order they were saved, then the one that was being written.
fn hand_overs(state: &ReviewState) -> Vec<HandOver> {
    let sessions: BTreeSet<&SessionId> = state
        .saved
        .legacy_queued
        .keys()
        .chain(state.saved.legacy_drafts.keys())
        .collect();
    sessions
        .into_iter()
        .filter_map(|session| {
            let mut comments = state
                .saved
                .legacy_queued
                .get(session)
                .cloned()
                .unwrap_or_default();
            if let Some((anchor, text)) = state.saved.legacy_drafts.get(session) {
                comments.push(draft_comment(anchor, text));
            }
            if comments.is_empty() {
                return None;
            }
            let mut repos_with_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for diff in state.workspace.diffs.iter().filter(|diff| &diff.session == session) {
                for file in &diff.files {
                    let repos = repos_with_path.entry(file.path.clone()).or_default();
                    if !repos.contains(&diff.repo) {
                        repos.push(diff.repo.clone());
                    }
                }
            }
            Some(HandOver {
                session: session.clone(),
                comments,
                selected_repo: state
                    .saved
                    .selected
                    .get(session)
                    .map(|(repo, _)| repo.clone()),
                repos_with_path,
            })
        })
        .collect()
}

/// A comment being written when an older build quit, as a saved comment.
fn draft_comment(anchor: &Anchor, text: &str) -> Comment {
    Comment {
        path: anchor.path.clone(),
        line: anchor.line,
        body: text.trim().to_owned(),
        sent: false,
        context: None,
    }
}

/// Drops the saved comment the daemon took from the legacy maps.
fn forget(state: &mut ReviewState, session: &SessionId, comment: &Comment) {
    if let Some(queued) = state.saved.legacy_queued.get_mut(session)
        && let Some(index) = queued.iter().position(|saved| saved == comment)
    {
        queued.remove(index);
        if queued.is_empty() {
            state.saved.legacy_queued.remove(session);
        }
        return;
    }
    if state
        .saved
        .legacy_drafts
        .get(session)
        .is_some_and(|(anchor, text)| draft_comment(anchor, text) == *comment)
    {
        state.saved.legacy_drafts.remove(session);
    }
}

/// Prepares the shown session's rows in the background.
fn prepare(state: &mut ReviewState) -> Effect<ReviewAction> {
    let Some(session) = state.session.clone() else {
        return Effect::none();
    };
    state.generation += 1;
    let generation = state.generation;
    let workspace = state.workspace.clone();
    let folded = state
        .saved
        .folded
        .get(&session)
        .cloned()
        .unwrap_or_default();
    Effect::future(async move {
        ReviewAction::Prepared(generation, Arc::new(rows(&workspace, &session, &folded)))
    })
    .cancellable("review-prepare", true)
}

/// The rows for `session`'s changes, with `folded` files closed.
#[must_use]
pub fn rows(workspace: &Workspace, session: &SessionId, folded: &BTreeSet<String>) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    for diff in workspace
        .diffs
        .iter()
        .filter(|diff| &diff.session == session)
    {
        for file in &diff.files {
            let open = !folded.contains(&file.path);
            rows.push(DiffRow::File {
                path: file.path.clone(),
                added: file.added,
                removed: file.removed,
                open,
            });
            if !open {
                continue;
            }
            for line in &file.lines {
                rows.push(DiffRow::Line {
                    path: file.path.clone(),
                    line: line.clone(),
                });
            }
        }
    }
    rows
}

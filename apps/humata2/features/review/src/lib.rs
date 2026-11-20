//! Reviewing a session's changes (RV-1–27): files fold, and lines wrap or
//! scroll. Comments on lines belong to the daemon and live in [`comments`];
//! this page only hands over what older builds saved here (D151).
//!
//! The rows the view draws are prepared off the UI thread by an effect and
//! arrive ready to draw ([`ReviewAction::Prepared`]); a newer request
//! replaces an older one still in flight.

pub mod comments;
pub mod file_filter;
pub mod looks;
pub mod opening_prompt;
pub mod order;
pub mod stack;
mod reopen;

use stack::{StackRows, Tier};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::FileTier;
use humata2_clients::views::{PlanView, StackView};
use humata2_clients::{Daemon, DaemonError, Workspace};
use humata2_clients::model::{
    Agent, Comment, DiffBase, DiffLine, DiffTargets, FileDiff, LineKind, LookedFile, PushChoice, RepoBase, RepoBases,
    RepoDiff, SessionId, Status,
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

/// A review plan's list as the Diff panel draws it (RV-32, D802): its rows,
/// then each repository's skim files folded into one row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanLayout {
    /// The groups and the key and normal files, in reading order.
    pub rows: Vec<PlanRow>,
    /// One folded row per repository with skim files, in the order the
    /// repositories first have one.
    pub skims: Vec<PlanSkim>,
}

/// One repository's skim files in a plan's list (D802, DS-67).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSkim {
    /// The repository, as the diff names it.
    pub repo: String,
    /// Its skim files, in reading order.
    pub paths: Vec<String>,
    /// The folded row draws before `rows[after]`: just after the
    /// repository's last row, or at the end when it has none.
    pub after: usize,
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
    /// Repositories of a session with several whose read failed while the
    /// others were read (DS-35): repository to why, as the panel says it.
    /// The panel puts each message and Try again under that repository's row.
    /// Empty when every repository was read, or the whole read failed (then
    /// `error` says why).
    pub repo_errors: BTreeMap<String, String>,
    /// How many files the last Whole stack read listed, so the Whole stack
    /// row keeps that count while a layer is picked (frames 119 to 121).
    pub whole_files: Option<u32>,
}

/// Where the keyboard cursor stands in a session's file list (DS-84): the
/// row the focus ring is drawn on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListCursor {
    /// A repository's row.
    Repo(String),
    /// A file's row.
    File {
        /// The repository the file is in, as `owner/name`.
        repo: String,
        /// The file's path.
        path: String,
    },
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
    /// Whether the Push & PR menu in the footer is open. Not saved.
    pub push_menu: bool,
    /// The one repository base menu open, if any (RV-31).
    pub repo_picker: Option<RepoPicker>,
    /// Where each repository's push of a session's changes stands.
    pub pushes: HashMap<(SessionId, String), PushState>,
    /// The review plan each session's view opened, while it shows.
    pub plans: HashMap<SessionId, PlanView>,
    /// The repositories folded per session, as `(session, repo)`: their
    /// files are hidden while they are in here. Not saved (DV-7).
    pub collapsed: HashSet<(SessionId, String)>,
    /// The stack layer each session shows, by title; `None` (or no entry)
    /// is Whole stack (D800). Not saved.
    pub layer: HashMap<SessionId, Option<String>>,
    /// The repositories whose skim files are open, as `(session, repo)`;
    /// they are folded into one row otherwise (D802). Not saved.
    pub skim_open: HashSet<(SessionId, String)>,
    /// Why asking the agent to send its stack again failed, by session
    /// (DS-65); cleared by the next ask. Not saved.
    pub ask_again_error: HashMap<SessionId, String>,
    /// The change (hunk) each session's open file is stepped to, counted
    /// from 0; a session with no entry is on its first. Reset whenever the
    /// open file changes (DS-27). Not saved.
    pub change: HashMap<SessionId, usize>,
    /// The sessions whose narrow file list is open over the file bar (DS-3).
    /// Not saved.
    pub file_list_open: HashSet<SessionId>,
    /// What each session's "Filter files" field holds (RV-6); kept while the
    /// panel is closed. Not saved.
    pub file_filter: HashMap<SessionId, String>,
    /// The row each session's file list has its keyboard cursor on (DS-84).
    /// Not saved.
    pub list_cursor: HashMap<SessionId, ListCursor>,
    /// Whether the full-screen Diff header's New agent tab menu is open
    /// (UXSIMPLE 3-3). Not saved.
    pub agent_tab_menu: bool,
    /// The agent last picked there, marked "last used" in the menu. Not saved.
    pub last_agent_tab: Option<Agent>,
    /// The files each session's person looked at, as `(repo, path)`, each
    /// with the fingerprint of the diff it showed (D852, [`looks`]). The host
    /// keeps them: read when a session opens, saved after each change.
    pub looked: HashMap<SessionId, HashMap<(String, String), u64>>,
    /// The files each layer covered when Looks good was pressed on it, as
    /// `(repo, path, fingerprint)`, by session and layer title, so a layer
    /// row reads its status without its diff. Not saved: rebuilt when a
    /// layer's diff is read and every file of it has a look.
    pub layer_looks: HashMap<(SessionId, String), Vec<(String, String, u64)>>,
    /// The base each session had before "Since I last looked" was switched
    /// on, restored when it is switched off. Not saved.
    pub base_before_looked: HashMap<SessionId, DiffBase>,
    /// The sessions whose saved looks were read from the host. Not saved.
    pub looks_read: HashSet<SessionId>,
}

/// The changes (hunks) of a file: its hunk headers, or one when it has none.
#[must_use]
pub fn change_count(file: &humata2_clients::model::FileDiff) -> usize {
    file.lines
        .iter()
        .filter(|line| line.kind == LineKind::Hunk)
        .count()
        .max(1)
}

impl ReviewState {
    /// The row `session`'s file list has its keyboard cursor on, if the
    /// keys have placed it (DS-84).
    #[must_use]
    pub fn list_cursor(&self, session: &SessionId) -> Option<&ListCursor> {
        self.list_cursor.get(session)
    }

    /// What `session`'s "Filter files" field holds.
    #[must_use]
    pub fn file_filter(&self, session: &SessionId) -> &str {
        self.file_filter.get(session).map_or("", String::as_str)
    }

    /// Whether `session` has enough changed files to get the field.
    #[must_use]
    pub fn file_filter_shown(&self, session: &SessionId) -> bool {
        file_filter::shown_for(self.panel_files(session).len())
    }

    /// Whether the filter lets `path` of `session` show. A session too short
    /// for the field is never filtered.
    #[must_use]
    pub fn file_visible(&self, session: &SessionId, path: &str) -> bool {
        !self.file_filter_shown(session) || file_filter::matches(path, self.file_filter(session))
    }

    /// `session`'s files as `(repo, path)` that the filter lets show; a
    /// repository with no match has none.
    #[must_use]
    pub fn visible_files(&self, session: &SessionId) -> Vec<(String, String)> {
        let shown = self.file_filter_shown(session);
        let query = self.file_filter(session);
        self.panel_files(session)
            .into_iter()
            .filter(|(_, path)| !shown || file_filter::matches(path, query))
            .collect()
    }

    /// Every file of `session`'s changes in the order the panel draws them,
    /// as `(repo, path)`.
    fn panel_files(&self, session: &SessionId) -> Vec<(String, String)> {
        self.drawn_rows(session, false)
            .into_iter()
            .filter_map(|row| match row {
                ListCursor::File { repo, path } => Some((repo, path)),
                ListCursor::Repo(_) => None,
            })
            .collect()
    }

    /// The rows of `session`'s file list exactly as the panel draws them:
    /// each repository's row and its files, with Whole stack's captions left
    /// out (they are not files), a plan's rows with each repository's skim
    /// files where the folded row sits, else the diff's own order. Drawing
    /// and stepping share this order. A folded repository's files are left
    /// out when `skip_folded`.
    fn drawn_rows(&self, session: &SessionId, skip_folded: bool) -> Vec<ListCursor> {
        let diffs: Vec<&RepoDiff> = self
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == session)
            .collect();
        let folded = |repo: &str| {
            skip_folded && self.collapsed.contains(&(session.clone(), repo.to_owned()))
        };
        let file = |repo: &str, path: &str| ListCursor::File {
            repo: repo.to_owned(),
            path: path.to_owned(),
        };
        let has = |repo: &str, path: &str| {
            diffs
                .iter()
                .any(|diff| diff.repo == repo && diff.files.iter().any(|each| each.path == path))
        };
        let mut rows = Vec::new();
        let Some(layout) = self.plan_layout(session) else {
            for diff in &diffs {
                rows.push(ListCursor::Repo(diff.repo.clone()));
                if folded(&diff.repo) {
                    continue;
                }
                rows.extend(diff.files.iter().map(|each| file(&diff.repo, &each.path)));
            }
            return rows;
        };
        let grouped = layout
            .rows
            .iter()
            .any(|row| matches!(row, PlanRow::Group { .. }));
        if self.stack_of(session).is_some() && !grouped {
            // A stack without groups: repository head, its files (Whole
            // stack by layer), then its skim files.
            let whole = self.whole_groups(session, &layout, &diffs);
            for (index, diff) in diffs.iter().enumerate() {
                rows.push(ListCursor::Repo(diff.repo.clone()));
                if folded(&diff.repo) {
                    continue;
                }
                match whole.as_ref().and_then(|groups| groups.get(index)) {
                    Some(entries) => {
                        for entry in entries {
                            if let stack::WholeEntry::File { path, .. } = entry
                                && has(&diff.repo, path)
                            {
                                rows.push(file(&diff.repo, path));
                            }
                        }
                    }
                    None => {
                        for row in &layout.rows {
                            if let PlanRow::File { repo, path, .. } = row
                                && *repo == diff.repo
                                && has(repo, path)
                            {
                                rows.push(file(repo, path));
                            }
                        }
                    }
                }
                for skim in layout.skims.iter().filter(|skim| skim.repo == diff.repo) {
                    for path in &skim.paths {
                        if has(&diff.repo, path) {
                            rows.push(file(&diff.repo, path));
                        }
                    }
                }
            }
            return rows;
        }
        // A plan: every repository's row first, then the plan's rows with a
        // skim row before the row it names.
        rows.extend(diffs.iter().map(|diff| ListCursor::Repo(diff.repo.clone())));
        let mut skims = layout.skims.iter().peekable();
        let push_skim = |rows: &mut Vec<ListCursor>, skim: &PlanSkim| {
            for path in &skim.paths {
                if has(&skim.repo, path) && !folded(&skim.repo) {
                    rows.push(file(&skim.repo, path));
                }
            }
        };
        for (at, row) in layout.rows.iter().enumerate() {
            while let Some(skim) = skims.next_if(|skim| skim.after == at) {
                push_skim(&mut rows, skim);
            }
            if let PlanRow::File { repo, path, .. } = row
                && has(repo, path)
                && !folded(repo)
            {
                rows.push(file(repo, path));
            }
        }
        for skim in skims {
            push_skim(&mut rows, skim);
        }
        rows
    }

    /// The file `session` shows: the one picked, or the first when none is
    /// picked or the picked one is gone.
    fn open_file(&self, session: &SessionId) -> Option<(String, String)> {
        let files = self.panel_files(session);
        self.saved
            .selected
            .get(session)
            .filter(|picked| files.contains(picked))
            .cloned()
            .or_else(|| files.into_iter().next())
    }

    /// The change `session`'s open file is stepped to, within its count.
    #[must_use]
    pub fn current_change(&self, session: &SessionId) -> usize {
        let Some((repo, path)) = self.open_file(session) else {
            return 0;
        };
        let count = self
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == session && diff.repo == repo)
            .flat_map(|diff| diff.files.iter())
            .find(|file| file.path == path)
            .map_or(1, change_count);
        self.change
            .get(session)
            .copied()
            .unwrap_or(0)
            .min(count - 1)
    }

    /// The rows of `session`'s changes in its review plan's order, or `None`
    /// when it has no plan. A group's title and note come before its first
    /// file; files the order leaves out come last; entries the diff lacks
    /// are skipped. Skim files are left out: they are in
    /// [`Self::plan_layout`]'s folded rows.
    #[must_use]
    pub fn plan_rows(&self, session: &SessionId) -> Option<Vec<PlanRow>> {
        self.plan_layout(session).map(|layout| layout.rows)
    }

    /// `session`'s plan list with each file at the tier [`Self::tier_of`]
    /// gives it, or `None` when it has no plan.
    #[must_use]
    pub fn plan_layout(&self, session: &SessionId) -> Option<PlanLayout> {
        self.plan_layout_with(session, |repo, file| self.tier_of(session, repo, file))
    }

    /// `session`'s plan list with each file at `tier(repo, file)` (D802),
    /// or `None` when it has no plan. As [`Self::plan_rows`], and: skim
    /// files leave the rows and their groups (a group left without files
    /// draws no header) and fold into one row per repository after its
    /// last row; with no order sent, key files come first in their
    /// repository.
    #[must_use]
    pub fn plan_layout_with(
        &self,
        session: &SessionId,
        tier: impl Fn(&str, &FileDiff) -> Tier,
    ) -> Option<PlanLayout> {
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
        let files: Vec<(&str, &FileDiff)> = diffs
            .iter()
            .flat_map(|diff| diff.files.iter().map(move |file| (diff.repo.as_str(), file)))
            .collect();
        let tiers: Vec<Tier> = files.iter().map(|(repo, file)| tier(repo, file)).collect();
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
        let row_of = |at: usize| {
            let (repo, file) = files[at];
            PlanRow::File {
                repo: repo.to_owned(),
                path: file.path.clone(),
                notes: notes_of(repo, &file.path),
            }
        };
        let mut used = vec![false; files.len()];
        let mut met: Vec<(&str, &str)> = Vec::new();
        let mut pending: Vec<PlanRow> = Vec::new();
        // Whether a skim file left the pending groups since they were met:
        // a group that then starts drops them, as they have no files.
        let mut skimmed = false;
        let mut rows = Vec::new();
        let mut skims: Vec<usize> = Vec::new();
        for (repo, path) in &plan.order {
            if met.contains(&(repo.as_str(), path.as_str())) {
                continue;
            }
            met.push((repo.as_str(), path.as_str()));
            let mut starts = plan
                .groups
                .iter()
                .filter(|group| group.first.0 == *repo && group.first.1 == *path)
                .peekable();
            if skimmed && starts.peek().is_some() {
                pending.clear();
                skimmed = false;
            }
            pending.extend(starts.map(|group| PlanRow::Group {
                title: group.title.clone(),
                note: group.note.clone(),
            }));
            let at = files.iter().enumerate().position(|(index, (held, file))| {
                !used[index] && file.path == path.as_str() && same_repo(repo, held)
            });
            if let Some(at) = at {
                used[at] = true;
                if tiers[at] == Tier::Skim {
                    skims.push(at);
                    skimmed = true;
                    continue;
                }
                rows.append(&mut pending);
                skimmed = false;
                rows.push(row_of(at));
            }
        }
        let mut index = 0;
        while index < files.len() {
            let repo = files[index].0;
            let end = files[index..]
                .iter()
                .position(|(held, _)| *held != repo)
                .map_or(files.len(), |run| index + run);
            let mut key = Vec::new();
            let mut normal = Vec::new();
            for at in index..end {
                if used[at] {
                    continue;
                }
                match tiers[at] {
                    Tier::Skim => skims.push(at),
                    Tier::Key if plan.order.is_empty() => key.push(at),
                    _ => normal.push(at),
                }
            }
            rows.extend(key.into_iter().chain(normal).map(&row_of));
            index = end;
        }
        let mut folded: Vec<PlanSkim> = Vec::new();
        for at in skims {
            let (repo, file) = files[at];
            match folded.iter_mut().find(|skim| skim.repo == repo) {
                Some(skim) => skim.paths.push(file.path.clone()),
                None => folded.push(PlanSkim {
                    repo: repo.to_owned(),
                    paths: vec![file.path.clone()],
                    after: rows
                        .iter()
                        .rposition(|row| matches!(row, PlanRow::File { repo: held, .. } if held == repo))
                        .map_or(rows.len(), |last| last + 1),
                }),
            }
        }
        Some(PlanLayout { rows, skims: folded })
    }

    /// Whole stack's files grouped by layer, one list per entry of `diffs`
    /// (D867), in `layout`'s order within each group; `None` when a layer is
    /// picked, the plan has no stack, or no layer lists any file.
    #[must_use]
    pub fn whole_groups(
        &self,
        session: &SessionId,
        layout: &PlanLayout,
        diffs: &[&RepoDiff],
    ) -> Option<Vec<Vec<stack::WholeEntry>>> {
        if self.layer_of(session).is_some() {
            return None;
        }
        let repos: Vec<(&str, Vec<(&str, Tier)>)> = diffs
            .iter()
            .map(|diff| {
                let files = layout
                    .rows
                    .iter()
                    .filter_map(|row| match row {
                        PlanRow::File { repo, path, .. } if *repo == diff.repo => {
                            let file = diff.files.iter().find(|file| &file.path == path)?;
                            Some((path.as_str(), self.tier_of(session, &diff.repo, file)))
                        }
                        _ => None,
                    })
                    .collect();
                (diff.repo.as_str(), files)
            })
            .collect();
        stack::whole_groups(self.stack_of(session)?, &repos)
    }

    /// How much `file` of `repo` matters in `session`'s review (D802): the
    /// agent's tier from the plan, else the one its diff was served with.
    /// A plan can arrive before the diffs are read again, so the plan's
    /// list comes first.
    #[must_use]
    pub fn tier_of(&self, session: &SessionId, repo: &str, file: &FileDiff) -> Tier {
        match self.agent_tier(session, repo, &file.path).unwrap_or(file.tier) {
            FileTier::Key => Tier::Key,
            FileTier::Normal => Tier::Normal,
            FileTier::Skim => Tier::Skim,
        }
    }

    /// The tier the agent gave `path` of `repo`: the picked layer's list,
    /// else the plan's (for a stack's Whole view, the layers' joined).
    fn agent_tier(&self, session: &SessionId, repo: &str, path: &str) -> Option<FileTier> {
        let plan = self.plans.get(session)?;
        let list = match (plan.stack.as_ref(), self.layer_of(session)) {
            (Some(stack), Some(title)) => &stack.layers.iter().find(|layer| layer.title == title)?.importance,
            _ => &plan.importance,
        };
        list.iter()
            .find(|entry| entry.repo == repo && entry.path == path)
            .map(|entry| entry.tier)
    }

    /// The stack `session`'s plan carries, if any.
    pub(crate) fn stack_of(&self, session: &SessionId) -> Option<&StackView> {
        self.plans.get(session)?.stack.as_ref()
    }

    /// The layer `session` shows, by title; `None` is Whole stack.
    #[must_use]
    pub fn layer_of(&self, session: &SessionId) -> Option<&str> {
        self.layer.get(session)?.as_deref()
    }

    /// The layer list of the open session, when its plan carries a stack.
    #[must_use]
    pub fn stack_rows(&self) -> Option<StackRows> {
        let session = self.session.as_ref()?;
        let stack = self.stack_of(session)?;
        let picked = self.layer_of(session);
        let fetch = self.fetch.get(session);
        let whole_files = if picked.is_none() {
            fetch.and_then(|fetch| fetch.diffs.as_ref()).map(|diffs| {
                let files = diffs.iter().map(|diff| diff.files.len()).sum::<usize>();
                u32::try_from(files).unwrap_or(u32::MAX)
            })
        } else {
            fetch.and_then(|fetch| fetch.whole_files)
        };
        // The Whole stack names every repository the stack touches, so a
        // layer leaves out the ones the session's diffs no longer list too.
        let mut repos = repos_of(self, session);
        for spec in &stack.whole {
            if !repos.contains(&spec.repo) {
                repos.push(spec.repo.clone());
            }
        }
        repos.sort();
        Some(stack::rows(stack, picked, &repos, whole_files))
    }

    /// What `session` reads: the picked layer's repositories while a stack
    /// shows, else its diff targets.
    fn current_base(&self, session: &SessionId) -> DiffBase {
        match self.stack_of(session) {
            Some(stack) => stack::layer_base(stack, self.layer_of(session)),
            None => self.targets_of(session).base(&repos_of(self, session)),
        }
    }

    /// Whether a stack shows for `session`: its base menus stay shut.
    fn shows_stack(&self, session: &SessionId) -> bool {
        self.stack_of(session).is_some()
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
    /// Step to the next file in panel order, opening its repository if it
    /// is folded; stays on the last (DS-2, DS-78).
    NextFile,
    /// Step to the previous file in panel order; stays on the first.
    PrevFile,
    /// Step to the next change (hunk) of the open file; stays on the last
    /// (DS-27).
    NextChange,
    /// Step to the previous change of the open file; stays on the first.
    PrevChange,
    /// Open the narrow file list over the file bar, or fold it (DS-3).
    ToggleFileList,
    /// Down (↓) while the file list has focus: the cursor moves to
    /// the next row, stopping at the last. The first press with no cursor
    /// only places it, on the open file or the first row (DS-84).
    ListDown,
    /// Up (↑) while the file list has focus: the cursor moves to the
    /// previous row, stopping at the first. The first press with no cursor
    /// only places it (DS-84).
    ListUp,
    /// Return (↩) while the file list has focus: opens the file under
    /// the cursor, or folds or unfolds the repository under it (DS-84).
    ListOpen,
    /// Left (←) while the file list has focus: from a file the cursor
    /// goes to its repository; an open repository folds; a folded one stays
    /// (DS-84).
    ListCollapse,
    /// Right (→) while the file list has focus: a folded repository
    /// opens; an open one moves the cursor to its first file; a file stays
    /// (DS-84).
    ListExpand,
    /// The session's "Filter files" field changed (RV-6); empty clears it.
    FilterFiles(SessionId, String),
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
    /// Give every repository of `session` the base `repo` is compared with
    /// now, and close the repository menu (D868, "Use for every repo").
    UseRepoBaseForEvery {
        /// The session.
        session: SessionId,
        /// The repository whose base the others take, as `owner/name`.
        repo: String,
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
    /// Show this layer of the open session's stack, by title; `None` is
    /// Whole stack (D800).
    PickLayer(Option<String>),
    /// Step to the next layer: Whole stack, then 1 (the bottom) up to the
    /// top. Stays at the top.
    NextLayer,
    /// Step back toward Whole stack. Stays at Whole stack.
    PrevLayer,
    /// Open or fold a repository's skim files in the open session (D802).
    ToggleSkim {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// Ask the open session's agent to send its stack again because `repo`
    /// moved after it sent `layer` (DS-65).
    AskAgain {
        /// The layer's title.
        layer: String,
        /// The repository that moved, as `owner/name`.
        repo: String,
    },
    /// Typing the ask into the agent's terminal ended.
    AskedAgain {
        /// The session asked.
        session: SessionId,
        /// Whether the terminal took it.
        result: Result<(), DaemonError>,
    },
    /// Open the footer's Push & PR menu, or close it if it is open; the base
    /// menus close.
    TogglePushMenu,
    /// Open the full-screen Diff header's New agent tab menu, or close it;
    /// the other menus close (UXSIMPLE 3-3).
    ToggleAgentTabMenu,
    /// An agent (or the plain terminal) was picked there: the menu closes
    /// and the pick is remembered as last used.
    AgentTabPicked(Agent),
    /// Close the Push & PR menu.
    ClosePushMenu,
    /// Mark the open session's changes reviewed (RV-6).
    MarkReviewed,
    /// Looks good (D852): every file of the diff shown is looked at, and the
    /// bookmark moves like [`ReviewAction::MarkReviewed`].
    LooksGood,
    /// Clear the looks of every file of the diff shown.
    ClearLooks,
    /// Mark or clear one file's look.
    ToggleLooked {
        /// The repository, as `owner/name`.
        repo: String,
        /// The file's path.
        path: String,
    },
    /// Switch the Diff between all changes (`false`) and what changed since
    /// the bookmark (`true`); off returns to the base it had. Not offered
    /// while a stack's layers show.
    SinceLooked(bool),
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
    /// The whole read failed, so each repository of the session was read on
    /// its own and at least one worked (DS-35): the changes of those that
    /// did, and why each of the others failed.
    FetchedByRepo {
        /// The session.
        session: SessionId,
        /// The base the reads were made against.
        base: DiffBase,
        /// The changes of the repositories that were read, in repository
        /// order.
        diffs: Vec<RepoDiff>,
        /// The repositories that failed, each with why, as the panel says it.
        errors: Vec<(String, String)>,
    },
    /// The host's saved looks of a session were read.
    LooksRead {
        session: SessionId,
        result: Result<Vec<LookedFile>, String>,
    },
    /// Saving a session's looks ended.
    LooksSaved {
        session: SessionId,
        result: Result<(), String>,
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
    /// Moves D183's bookmark for the open session, then reads again.
    fn mark_reviewed(&self, state: &ReviewState) -> Effect<ReviewAction> {
        let Some(session) = state.session.clone() else {
            return Effect::none();
        };
        let reply = self.daemon.mark_reviewed(&session);
        Effect::future(async move {
            ReviewAction::Reviewed {
                session,
                result: reply.await.map_err(|error| error.to_string()),
            }
        })
    }

    /// Has the host keep `session`'s looks as they are now (D852).
    fn save_looks(&self, state: &ReviewState, session: &SessionId) -> Effect<ReviewAction> {
        let mut looks: Vec<LookedFile> = state
            .looked
            .get(session)
            .into_iter()
            .flatten()
            .map(|((repo, path), fingerprint)| LookedFile {
                repo: repo.clone(),
                path: path.clone(),
                fingerprint: *fingerprint,
            })
            .collect();
        looks.sort_by(|a, b| (&a.repo, &a.path).cmp(&(&b.repo, &b.path)));
        let reply = self.daemon.save_looks(session, looks);
        let session = session.clone();
        Effect::future(async move {
            ReviewAction::LooksSaved {
                session,
                result: reply.await.map_err(|error| error.to_string()),
            }
        })
    }

    /// Reads what the host kept of `session`'s looks (D852).
    fn read_looks(&self, session: &SessionId) -> Effect<ReviewAction> {
        let reply = self.daemon.looks(session);
        let session = session.clone();
        Effect::future(async move {
            ReviewAction::LooksRead {
                session,
                result: reply.await.map_err(|error| error.to_string()),
            }
        })
    }

    fn fetch(&self, state: &mut ReviewState, session: &SessionId) -> Effect<ReviewAction> {
        let fetch = state.fetch.entry(session.clone()).or_default();
        if fetch.loading {
            fetch.queued = true;
            return Effect::none();
        }
        fetch.loading = true;
        let base = state.current_base(session);
        let reply = self.daemon.diffs(session, &base);
        let repos = repos_of(state, session);
        let daemon = Arc::clone(&self.daemon);
        let session = session.clone();
        Effect::future(async move {
            let error = match reply.await {
                Ok(diffs) => {
                    return ReviewAction::Fetched {
                        session,
                        base,
                        result: Ok(diffs),
                    };
                }
                Err(error) => error,
            };
            // DS-35: one repository failing shouldn't hide the others. A
            // missing default branch has its own fix, so it stays whole.
            if repos.len() >= 2 && !matches!(error, DaemonError::NoDefaultBranch { .. }) {
                let mut diffs = Vec::new();
                let mut errors = Vec::new();
                for repo in &repos {
                    match daemon.repo_diff(&session, &base, repo).await {
                        Ok(read) => diffs.extend(read),
                        Err(failed) => errors.push((repo.clone(), failed.to_string())),
                    }
                }
                // Only a split outcome is one repository's fault: when every
                // repository read fine, or none did, the whole read failed.
                if !errors.is_empty() && errors.len() < repos.len() {
                    return ReviewAction::FetchedByRepo {
                        session,
                        base,
                        diffs,
                        errors,
                    };
                }
            }
            ReviewAction::Fetched {
                session,
                base,
                result: Err(error),
            }
        })
    }

    /// A read of `session`'s changes against `base` ended with `outcome`:
    /// the diffs and the repositories that failed beside them, or why the
    /// whole read failed. A read against a base no longer in use is dropped.
    fn finish_read(
        &self,
        state: &mut ReviewState,
        session: SessionId,
        base: DiffBase,
        outcome: Result<(Vec<RepoDiff>, BTreeMap<String, String>), DaemonError>,
    ) -> Effect<ReviewAction> {
        let current = state.current_base(&session);
        let whole = state.stack_of(&session).is_some() && state.layer_of(&session).is_none();
        let fetch = state.fetch.entry(session.clone()).or_default();
        fetch.loading = false;
        let queued = std::mem::take(&mut fetch.queued);
        let mut effects = Vec::new();
        if base == current {
            match outcome {
                Ok((diffs, repo_errors)) => {
                    fetch.error = None;
                    fetch.no_default = None;
                    fetch.repo_errors = repo_errors;
                    fetch.read_at = Some(self.clock.now());
                    fetch.diffs = Some(diffs.clone());
                    if whole {
                        let files = diffs.iter().map(|diff| diff.files.len()).sum::<usize>();
                        fetch.whole_files = Some(u32::try_from(files).unwrap_or(u32::MAX));
                    }
                    replace_diffs(&mut state.workspace, &session, &diffs);
                    rebuild_layer_looks(state, &session);
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
                    fetch.repo_errors.clear();
                    fetch.error = Some(error.to_string());
                }
            }
        }
        if queued {
            effects.push(self.fetch(state, &session));
        }
        Effect::merge(effects)
    }

    /// Shows `layer` of the open session's stack and reads its diff. A pick
    /// of the layer already showing, or with no stack, does nothing; the
    /// session's own diff targets are never touched.
    fn pick_layer(&self, state: &mut ReviewState, layer: Option<String>) -> Effect<ReviewAction> {
        let Some(session) = state.session.clone() else {
            return Effect::none();
        };
        let Some(stack) = state.stack_of(&session) else {
            return Effect::none();
        };
        // A title the stack doesn't have is Whole stack.
        let layer = layer.filter(|title| stack.layers.iter().any(|each| &each.title == title));
        if state.layer_of(&session) == layer.as_deref() {
            return Effect::none();
        }
        state.layer.insert(session.clone(), layer);
        self.fetch(state, &session)
    }

    /// Steps the open session's stack one layer.
    fn step_layer(&self, state: &mut ReviewState, way: stack::Step) -> Effect<ReviewAction> {
        let Some(session) = state.session.clone() else {
            return Effect::none();
        };
        let Some(stack) = state.stack_of(&session) else {
            return Effect::none();
        };
        let to = stack::step(stack, state.layer_of(&session), way);
        self.pick_layer(state, to)
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
/// Every file of the diff last read for `session`, as `(repo, path,
/// fingerprint)`.
fn shown_files(state: &ReviewState, session: &SessionId) -> Vec<(String, String, u64)> {
    state
        .fetch
        .get(session)
        .and_then(|fetch| fetch.diffs.as_deref())
        .unwrap_or_default()
        .iter()
        .flat_map(|diff| {
            diff.files
                .iter()
                .map(move |file| (diff.repo.clone(), file.path.clone(), looks::fingerprint(file)))
        })
        .collect()
}

/// The layer shown covered what was looked at, once its diff is read and
/// every file of it has a look: a layer's row reads Looks good (or Changed)
/// again after a restart, which kept only the files' looks.
fn rebuild_layer_looks(state: &mut ReviewState, session: &SessionId) {
    let Some(title) = state.layer_of(session).map(str::to_owned) else {
        return;
    };
    if state.layer_looks.contains_key(&(session.clone(), title.clone())) {
        return;
    }
    let diffs = state
        .fetch
        .get(session)
        .and_then(|fetch| fetch.diffs.as_deref())
        .unwrap_or_default();
    if let Some(covered) = looks::covered_by_looks(state, session, diffs) {
        state.layer_looks.insert((session.clone(), title), covered);
    }
}

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
            ReviewAction::SelectFile { repo, path } => select_file(state, repo, path),
            ReviewAction::NextFile => step_file(state, true),
            ReviewAction::PrevFile => step_file(state, false),
            ReviewAction::NextChange => step_change(state, true),
            ReviewAction::PrevChange => step_change(state, false),
            ReviewAction::ToggleFileList => {
                if let Some(session) = state.session.clone()
                    && !state.file_list_open.remove(&session)
                {
                    state.file_list_open.insert(session);
                }
            }
            ReviewAction::FilterFiles(session, query) => {
                if query.is_empty() {
                    state.file_filter.remove(&session);
                } else {
                    state.file_filter.insert(session, query);
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
                if let Some(session) = state.session.clone()
                    && !state.shows_stack(&session)
                {
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
                if state.shows_stack(&session) {
                    return Effect::none();
                }
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
            ReviewAction::UseRepoBaseForEvery { session, repo } => {
                state.repo_picker = None;
                if state.shows_stack(&session) {
                    return Effect::none();
                }
                let before = state.targets_of(&session);
                let base = before.of(&repo);
                let repos = repos_of(state, &session);
                let targets = state.targets.entry(session.clone()).or_insert(before);
                for each in repos {
                    targets.pick(&each, base.clone());
                }
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
                } else if state
                    .session
                    .as_ref()
                    .is_some_and(|session| !state.shows_stack(session))
                {
                    state.base_menu = false;
                    state.push_menu = false;
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
                let stacked = state
                    .session
                    .as_ref()
                    .is_some_and(|session| state.shows_stack(session));
                state.base_menu = !state.base_menu && !stacked;
                state.repo_picker = None;
                state.push_menu = false;
                state.agent_tab_menu = false;
            }
            ReviewAction::CloseBaseMenu => state.base_menu = false,
            ReviewAction::PickLayer(layer) => return self.pick_layer(state, layer),
            ReviewAction::NextLayer => return self.step_layer(state, stack::Step::Next),
            ReviewAction::PrevLayer => return self.step_layer(state, stack::Step::Previous),
            ReviewAction::ToggleSkim { repo } => {
                if let Some(session) = state.session.clone() {
                    let key = (session, repo);
                    if !state.skim_open.remove(&key) {
                        state.skim_open.insert(key);
                    }
                }
            }
            ReviewAction::AskAgain { layer, repo } => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                state.ask_again_error.remove(&session);
                let key = format!(
                    "stack-again/{session}/{layer}/{repo}/{}",
                    self.clock.now().as_millis()
                );
                let call = self
                    .daemon
                    .send_keyed(&session, &stack::ask_again_text(&layer, &repo), &key);
                return Effect::future(async move {
                    ReviewAction::AskedAgain { session, result: call.await }
                });
            }
            ReviewAction::AskedAgain { session, result } => match result {
                Ok(()) => {
                    state.ask_again_error.remove(&session);
                }
                Err(error) => {
                    state.ask_again_error.insert(session, error.to_string());
                }
            },
            ReviewAction::TogglePushMenu => {
                state.push_menu = !state.push_menu;
                state.base_menu = false;
                state.repo_picker = None;
                state.agent_tab_menu = false;
            }
            ReviewAction::ToggleAgentTabMenu => {
                state.agent_tab_menu = !state.agent_tab_menu;
                state.base_menu = false;
                state.repo_picker = None;
                state.push_menu = false;
            }
            ReviewAction::AgentTabPicked(agent) => {
                state.agent_tab_menu = false;
                state.last_agent_tab = Some(agent);
            }
            ReviewAction::ClosePushMenu => state.push_menu = false,
            ReviewAction::MarkReviewed => {
                state.base_menu = false;
                return self.mark_reviewed(state);
            }
            ReviewAction::LooksGood => {
                state.base_menu = false;
                if let Some(session) = state.session.clone() {
                    let shown = shown_files(state, &session);
                    let layer = state.layer_of(&session).map(str::to_owned);
                    let looked = state.looked.entry(session.clone()).or_default();
                    for (repo, path, seen) in &shown {
                        looked.insert((repo.clone(), path.clone()), *seen);
                    }
                    if let Some(title) = layer {
                        state.layer_looks.insert((session.clone(), title), shown);
                    }
                    return Effect::merge(vec![
                        self.save_looks(state, &session),
                        self.mark_reviewed(state),
                    ]);
                }
                return self.mark_reviewed(state);
            }
            ReviewAction::ClearLooks => {
                if let Some(session) = state.session.clone() {
                    let shown = shown_files(state, &session);
                    if let Some(looked) = state.looked.get_mut(&session) {
                        for (repo, path, _) in shown {
                            looked.remove(&(repo, path));
                        }
                    }
                    return self.save_looks(state, &session);
                }
            }
            ReviewAction::ToggleLooked { repo, path } => {
                if let Some(session) = state.session.clone() {
                    let seen = shown_files(state, &session)
                        .into_iter()
                        .find(|(r, p, _)| *r == repo && *p == path)
                        .map(|(_, _, seen)| seen);
                    let looked = state.looked.entry(session.clone()).or_default();
                    let key = (repo, path);
                    if looked.contains_key(&key) && looked.get(&key) == seen.as_ref() {
                        looked.remove(&key);
                    } else if let Some(seen) = seen {
                        looked.insert(key, seen);
                    }
                    return self.save_looks(state, &session);
                }
            }
            ReviewAction::SinceLooked(on) => {
                if let Some(session) = state.session.clone()
                    && !state.shows_stack(&session)
                {
                    let current = state.targets_of(&session).preset;
                    let base = if on {
                        if current != DiffBase::SinceLastReviewed {
                            state.base_before_looked.insert(session.clone(), current);
                        }
                        DiffBase::SinceLastReviewed
                    } else {
                        state
                            .base_before_looked
                            .remove(&session)
                            .unwrap_or(DiffBase::OriginMain)
                    };
                    state.base_menu = false;
                    state.repo_picker = None;
                    state
                        .targets
                        .entry(session.clone())
                        .or_default()
                        .pick_all(base);
                    sync_saved(state);
                    return self.fetch(state, &session);
                }
            }
            ReviewAction::LooksRead { session, result } => match result {
                Ok(saved) => {
                    state.looks_read.insert(session.clone());
                    // What was looked at since the read began wins.
                    let looked = state.looked.entry(session.clone()).or_default();
                    for look in saved {
                        looked
                            .entry((look.repo, look.path))
                            .or_insert(look.fingerprint);
                    }
                    rebuild_layer_looks(state, &session);
                }
                Err(error) => state.fetch.entry(session).or_default().error = Some(error),
            },
            ReviewAction::LooksSaved { session, result } => {
                // The tick stays in memory; the person sees why it may not
                // be there next time.
                if let Err(error) = result {
                    state.fetch.entry(session).or_default().error = Some(error);
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
                let outcome = result.map(|diffs| (diffs, BTreeMap::new()));
                return self.finish_read(state, session, base, outcome);
            }
            ReviewAction::FetchedByRepo {
                session,
                base,
                diffs,
                errors,
            } => {
                let outcome = Ok((diffs, errors.into_iter().collect()));
                return self.finish_read(state, session, base, outcome);
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
                let same = state.session.as_ref() == Some(&session);
                if !same {
                    state.session = Some(session.clone());
                    state.rows = None;
                    state.base_menu = false;
                    state.push_menu = false;
                    state.repo_picker = None;
                }
                // D852: the host keeps the looks; read them once per session.
                let read_looks = if state.looks_read.contains(&session) {
                    Effect::none()
                } else {
                    self.read_looks(&session)
                };
                // Opening again while its read is in flight asks for
                // nothing new: the files may not have changed since it
                // started, unlike a turn end or Mark reviewed.
                if state.fetch.get(&session).is_some_and(|fetch| fetch.loading) {
                    return Effect::merge(vec![prepare(state), read_looks]);
                }
                // DS-76: opening the panel again keeps the diff it holds.
                if !reopen::reads_again(same, state.fetch.get(&session), self.clock.now()) {
                    return Effect::merge(vec![prepare(state), read_looks]);
                }
                return Effect::merge(vec![prepare(state), self.fetch(state, &session), read_looks]);
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
            ReviewAction::ToggleRepo { repo } => toggle_repo(state, repo),
            ReviewAction::ListDown => move_cursor(state, true),
            ReviewAction::ListUp => move_cursor(state, false),
            ReviewAction::ListOpen => match cursor_row(state) {
                Some((_, ListCursor::File { repo, path })) => select_file(state, repo, path),
                Some((_, ListCursor::Repo(repo))) => toggle_repo(state, repo),
                None => {}
            },
            ReviewAction::ListCollapse => match cursor_row(state) {
                Some((session, ListCursor::File { repo, .. })) => {
                    state.list_cursor.insert(session, ListCursor::Repo(repo));
                }
                Some((session, ListCursor::Repo(repo))) => {
                    state.collapsed.insert((session, repo));
                }
                None => {}
            },
            ReviewAction::ListExpand => match cursor_row(state) {
                Some((session, ListCursor::Repo(repo))) => {
                    if !state.collapsed.remove(&(session.clone(), repo.clone())) {
                        let first = list_rows(state, &session).into_iter().find(|row| {
                            matches!(row, ListCursor::File { repo: of, .. } if *of == repo)
                        });
                        if let Some(first) = first {
                            state.list_cursor.insert(session, first);
                        }
                    }
                }
                Some((_, ListCursor::File { .. })) | None => {}
            },
            ReviewAction::ToggleWrap => state.wrap = !state.wrap,
            ReviewAction::HandedOver(session, comment) => forget(state, &session, &comment),
            ReviewAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

/// Shows `path` of `repo` in the open session if its diff has it: opens its
/// repository if folded (DS-78), folds the file list (DS-3) and starts on its
/// first change when the file is a different one.
fn select_file(state: &mut ReviewState, repo: String, path: String) {
    let Some(session) = state.session.clone() else {
        return;
    };
    let has = state.workspace.diffs.iter().any(|diff| {
        diff.session == session
            && diff.repo == repo
            && diff.files.iter().any(|file| file.path == path)
    });
    if !has {
        return;
    }
    if state.open_file(&session).as_ref() != Some(&(repo.clone(), path.clone())) {
        state.change.remove(&session);
    }
    state.file_list_open.remove(&session);
    state.collapsed.remove(&(session.clone(), repo.clone()));
    state.saved.selected.insert(session.clone(), (repo, path));
    settle_cursor(state, &session);
}

/// Folds `repo` of the open session's file list, or unfolds it if folded
/// (DV-7).
fn toggle_repo(state: &mut ReviewState, repo: String) {
    let Some(session) = state.session.clone() else {
        return;
    };
    let key = (session.clone(), repo);
    if !state.collapsed.remove(&key) {
        state.collapsed.insert(key);
    }
    settle_cursor(state, &session);
}

/// The rows of `session`'s file list in order (DS-84): each repository's
/// row, then its files unless it is folded.
fn list_rows(state: &ReviewState, session: &SessionId) -> Vec<ListCursor> {
    state.drawn_rows(session, true)
}

/// The open session and the row its cursor is on, when it is on a row the
/// list still shows.
fn cursor_row(state: &ReviewState) -> Option<(SessionId, ListCursor)> {
    let session = state.session.clone()?;
    let cursor = state.list_cursor.get(&session)?.clone();
    list_rows(state, &session)
        .contains(&cursor)
        .then_some((session, cursor))
}

/// Moves the open session's list cursor one row, stopping at the ends. With
/// no cursor on a row, only places it: on the open file, else the first row.
fn move_cursor(state: &mut ReviewState, forward: bool) {
    let Some(session) = state.session.clone() else {
        return;
    };
    let rows = list_rows(state, &session);
    let Some(last) = rows.len().checked_sub(1) else {
        return;
    };
    let at_cursor = state
        .list_cursor
        .get(&session)
        .and_then(|cursor| rows.iter().position(|row| row == cursor));
    let to = match at_cursor {
        Some(at) if forward => (at + 1).min(last),
        Some(at) => at.saturating_sub(1),
        None => state
            .open_file(&session)
            .and_then(|(repo, path)| {
                let repo_row = ListCursor::Repo(repo.clone());
                let file_row = ListCursor::File { repo, path };
                rows.iter()
                    .position(|row| *row == file_row)
                    .or_else(|| rows.iter().position(|row| *row == repo_row))
            })
            .unwrap_or(0),
    };
    if let Some(row) = rows.into_iter().nth(to) {
        state.list_cursor.insert(session, row);
    }
}

/// A cursor on a file whose repository is folded moves to the repository's
/// row, so it never rests on a row that isn't drawn.
fn settle_cursor(state: &mut ReviewState, session: &SessionId) {
    let hidden = match state.list_cursor.get(session) {
        Some(ListCursor::File { repo, .. }) => state
            .collapsed
            .contains(&(session.clone(), repo.clone()))
            .then(|| repo.clone()),
        _ => None,
    };
    if let Some(repo) = hidden {
        state.list_cursor.insert(session.clone(), ListCursor::Repo(repo));
    }
}

/// Moves the open session's file one step in panel order, stopping at the
/// ends.
fn step_file(state: &mut ReviewState, forward: bool) {
    let Some(session) = state.session.clone() else {
        return;
    };
    let files = state.panel_files(&session);
    let Some(open) = state.open_file(&session) else {
        return;
    };
    let Some(at) = files.iter().position(|file| *file == open) else {
        return;
    };
    let to = if forward {
        files.get(at + 1)
    } else {
        at.checked_sub(1).and_then(|before| files.get(before))
    };
    if let Some((repo, path)) = to.cloned() {
        select_file(state, repo, path);
    }
}

/// Moves the open file's current change one step, within its count.
fn step_change(state: &mut ReviewState, forward: bool) {
    let Some(session) = state.session.clone() else {
        return;
    };
    let Some((repo, path)) = state.open_file(&session) else {
        return;
    };
    let count = state
        .workspace
        .diffs
        .iter()
        .filter(|diff| diff.session == session && diff.repo == repo)
        .flat_map(|diff| diff.files.iter())
        .find(|file| file.path == path)
        .map_or(1, change_count);
    let now = state.current_change(&session);
    let to = if forward {
        (now + 1).min(count - 1)
    } else {
        now.saturating_sub(1)
    };
    state.change.insert(session, to);
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

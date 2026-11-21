//! Showing a prepared view and putting back what the person had (VW-6 to
//! VW-9, P8): Open it takes a snapshot of the page, the panel, the To-do
//! filter and each repository's diff base, then shows the view; Revert puts
//! each of them back and never touches the person's comments.

use humata2_clients::model::{DiffTargets, RepoBase, RepoDiffSpec, Scope, SessionId, ViewId};
use humata2_todo_feature::{ChannelPick, SessionPick, TodoAction, ViewFilter};
use humata2_clients::views::{PlanView, ViewContent, WalkthroughStep};
use humata2_review::ReviewAction;
use humata2_session::{Panel, SessionAction, Tab, TabGroup};
use humata2_views::{Delegate, ViewsAction};
use tca::Effect;

use crate::layout::FullScreen;
use crate::{App, AppAction, AppState, Route};

/// What the person had on `session`'s page, and around it, when a view
/// opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    route: Route,
    trail: Vec<Route>,
    session: SessionId,
    groups: Vec<TabGroup>,
    focused_group: usize,
    tab: usize,
    panel: Option<Panel>,
    widened: bool,
    full_screen: Option<FullScreen>,
    collapsed: bool,
    todo: ViewFilter,
    targets: Option<DiffTargets>,
    plan: Option<PlanView>,
    /// The stack layer showing, by title; goes back with the plan (D800).
    layer: Option<String>,
}

impl Snapshot {
    fn take(state: &AppState, session: &SessionId) -> Self {
        let page = state.session.views.get(session).cloned().unwrap_or_default();
        Self {
            route: state.route.clone(),
            trail: state.trail.clone(),
            session: session.clone(),
            groups: page.groups,
            focused_group: page.focused_group,
            tab: page.tab,
            panel: page.panel,
            widened: page.widened,
            full_screen: state.full_screens.get(session).copied(),
            collapsed: state.layout.panel.collapsed,
            todo: state.todo.view.filter.clone(),
            targets: state.review.targets.get(session).cloned(),
            plan: state.review.plans.get(session).cloned(),
            layer: state.review.layer.get(session).cloned().flatten(),
        }
    }
}

/// The To-do filter that shows what a board view's scope held.
fn todo_filter(scope: &Scope) -> ViewFilter {
    match scope {
        Scope::Global => ViewFilter::default(),
        Scope::Channel(id) => ViewFilter {
            channel: Some(ChannelPick::Channel(id.as_str().to_owned())),
            ..ViewFilter::default()
        },
        Scope::Session { session, .. } => ViewFilter {
            session: Some(SessionPick::Session(session.as_str().to_owned())),
            ..ViewFilter::default()
        },
    }
}

impl App {
    /// Handles what the views page asked for.
    pub(super) fn views_delegate(&self, state: &mut AppState, delegate: &Delegate) -> Effect<AppAction> {
        match delegate {
            Delegate::Opening(id) => self.open_view(state, id),
            Delegate::Reverting(id) => self.revert_view(state, id),
            Delegate::PlanRead(id) => {
                if let Some(record) = state.views.record(id)
                    && let ViewContent::ReviewPlan(plan) = &record.content
                {
                    let session = record.view.session.clone();
                    let mut plan = plan.clone();
                    resolve_plan_repos(state, &session, &mut plan);
                    state.review.plans.insert(session, plan);
                }
                Effect::none()
            }
        }
    }

    /// Moves a walkthrough to step `to`, or to its last step if that is
    /// further, and opens the step's file beside the view while it shows.
    pub(super) fn step_view(&self, state: &mut AppState, id: &ViewId, to: usize) -> Effect<AppAction> {
        let last = match state.views.record(id).map(|record| &record.content) {
            Some(ViewContent::Walkthrough(steps)) => steps.len().saturating_sub(1),
            Some(ViewContent::PullRequest(drafts)) => drafts.len().saturating_sub(1),
            _ => 0,
        };
        state.view_step.insert(id.clone(), to.min(last));
        if state.views.showing.as_ref().is_some_and(|showing| &showing.id == id) {
            self.sync_view_file(state, id)
        } else {
            Effect::none()
        }
    }

    /// Keep or Close on the view showing: what the person had before it is
    /// no longer theirs to go back to, and Close also closes its tab. Answers
    /// the effect and the view under it, which shows again once the views
    /// page has answered.
    pub(super) fn finish_view(
        &self,
        state: &mut AppState,
        close: bool,
    ) -> (Effect<AppAction>, Option<ViewId>) {
        let Some(showing) = state.views.showing.clone() else {
            return (Effect::none(), None);
        };
        let mut under = None;
        if matches!(state.view_snapshots.last(), Some((held, _)) if *held == showing.id) {
            state.view_snapshots.pop();
            under = state.view_snapshots.last().map(|(id, _)| id.clone());
        }
        state.view_files.remove(&showing.id);
        if !close {
            return (Effect::none(), under);
        }
        let Some(session) = state
            .views
            .record(&showing.id)
            .map(|record| record.view.session.clone())
        else {
            return (Effect::none(), under);
        };
        if state.session.open.as_ref() != Some(&session) {
            return (Effect::none(), under);
        }
        let tab = Tab::View(showing.id.to_string());
        (self.close_tab_of(state, &session, &tab), under)
    }

    /// Closes `tab` on `session`'s page, if it is there.
    fn close_tab_of(&self, state: &mut AppState, session: &SessionId, tab: &Tab) -> Effect<AppAction> {
        match tab_place(state, session, tab) {
            Some((group, index)) => self.child(
                state,
                AppAction::Session(SessionAction::CloseTab { group, index }),
            ),
            None => Effect::none(),
        }
    }

    /// Makes the file tab the view opened match its walkthrough's step: the
    /// tab it opened before goes, and the step's file opens in the group
    /// beside the view's tab. A step in a repository the session does not
    /// have opens nothing.
    fn sync_view_file(&self, state: &mut AppState, id: &ViewId) -> Effect<AppAction> {
        let Some(record) = state.views.record(id) else {
            return Effect::none();
        };
        let session = record.view.session.clone();
        if state.session.open.as_ref() != Some(&session) {
            return Effect::none();
        }
        let step = state.view_step.get(id).copied().unwrap_or(0);
        if matches!(record.content, ViewContent::Walkthrough(_))
            && !state.workspace.session_files.contains_key(&session)
        {
            // The session's files are fetched lazily; the step's file opens
            // once they arrive.
            return self.ask_files(state, session);
        }
        let wanted = match &record.content {
            ViewContent::Walkthrough(steps) => steps
                .get(step)
                .and_then(|step| step_tab(state, &session, step)),
            _ => return Effect::none(),
        };
        let held = state.view_files.get(id).cloned();
        if wanted == held {
            return Effect::none();
        }
        let mut effects = Vec::new();
        if let Some(old) = held {
            state.view_files.remove(id);
            effects.push(self.close_tab_of(state, &session, &old));
        }
        if let Some(tab) = wanted
            && tab_place(state, &session, &tab).is_none()
        {
            effects.push(self.open_file_beside(state, &session, id, &tab));
            state.view_files.insert(id.clone(), tab);
        }
        Effect::merge(effects)
    }

    /// `session`'s files arrived: the walkthrough showing on it, if any,
    /// opens its step's file.
    pub(super) fn view_files_arrived(&self, state: &mut AppState, session: &SessionId) -> Effect<AppAction> {
        let Some(id) = state.views.showing.as_ref().map(|showing| showing.id.clone()) else {
            return Effect::none();
        };
        if state.views.record(&id).is_none_or(|record| &record.view.session != session) {
            return Effect::none();
        }
        self.sync_view_file(state, &id)
    }

    /// Opens `tab`, a file, in the group other than the one holding `id`'s
    /// tab. The person's own Files selection stays as it was.
    fn open_file_beside(
        &self,
        state: &mut AppState,
        session: &SessionId,
        id: &ViewId,
        tab: &Tab,
    ) -> Effect<AppAction> {
        let Tab::File { repo, path } = tab else {
            return Effect::none();
        };
        let prior = state
            .session
            .views
            .get(session)
            .and_then(|page| page.file.clone());
        let select = self.child(
            state,
            AppAction::Session(SessionAction::SelectFile {
                repo: repo.clone(),
                path: path.clone(),
            }),
        );
        let open = self.child(state, AppAction::Session(SessionAction::OpenFile));
        if let Some(page) = state.session.views.get_mut(session) {
            page.file = prior;
        }
        let mut effects = vec![select, open];
        let beside = tab_place(state, session, &Tab::View(id.to_string()));
        if let (Some((file_group, index)), Some((view_group, _))) =
            (tab_place(state, session, tab), beside)
            && file_group == view_group
        {
            effects.push(self.child(
                state,
                AppAction::Session(SessionAction::MoveTab {
                    group: file_group,
                    index,
                }),
            ));
        }
        Effect::merge(effects)
    }

    fn open_view(&self, state: &mut AppState, id: &ViewId) -> Effect<AppAction> {
        let Some(record) = state.views.record(id).cloned() else {
            return Effect::none();
        };
        // A view already in the stack moves nothing and pushes nothing; the
        // one on top is the one showing.
        if state.view_snapshots.iter().any(|(held, _)| held == id) {
            let top = state.view_snapshots.last().map(|(top, _)| top.clone());
            return match top {
                Some(top) if top != *id => {
                    self.child(state, AppAction::Views(ViewsAction::Resume(top)))
                }
                _ => Effect::none(),
            };
        }
        let session = record.view.session.clone();
        let snapshot = Snapshot::take(state, &session);
        state.view_snapshots.push((id.clone(), snapshot));
        match record.content {
            // A board view opens the To-do list narrowed the same way (D874).
            ViewContent::Board(scope) => self.open_todo(state, todo_filter(&scope)),
            ViewContent::SessionBoard {
                session: board, ..
            } => {
                if state.workspace.session(&board).is_some() {
                    let filter = ViewFilter {
                        session: Some(SessionPick::Session(board.as_str().to_owned())),
                        ..ViewFilter::default()
                    };
                    self.open_todo(state, filter)
                } else {
                    self.open_view_tab(state, id, &session)
                }
            }
            ViewContent::ReviewPlan(mut plan) => {
                resolve_plan_repos(state, &session, &mut plan);
                let targets = plan.targets.clone();
                let stacked = plan.stack.is_some();
                state.review.plans.insert(session.clone(), plan);
                // A stack opens at Whole stack (D800).
                state.review.layer.remove(&session);
                let go = self.go_to_session(state, &session);
                let panel = self.handle(
                    state,
                    AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
                );
                let mut effects = vec![go, panel];
                if stacked || targets.is_empty() {
                    // The layers' repositories (or the session's current bases
                    // when a plain plan names no targets) decide what is read.
                    effects.push(self.child(state, AppAction::Review(ReviewAction::Retry)));
                } else {
                    for RepoDiffSpec { repo, base, .. } in targets {
                        effects.push(self.child(
                            state,
                            AppAction::Review(ReviewAction::PickRepoBase {
                                session: session.clone(),
                                repo,
                                base,
                            }),
                        ));
                    }
                }
                Effect::merge(effects)
            }
            ViewContent::PullRequest(drafts) => {
                let open_tab = self.open_view_tab(state, id, &session);
                let panel = self.handle(
                    state,
                    AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
                );
                let mut effects = vec![open_tab, panel];
                if drafts.is_empty() {
                    effects.push(self.child(state, AppAction::Review(ReviewAction::Retry)));
                } else {
                    for draft in drafts {
                        let repo = resolve_repo(state, &session, &draft.repo);
                        let base = if draft.base_branch.trim().is_empty() {
                            RepoBase::DefaultBranch
                        } else {
                            RepoBase::Branch(draft.base_branch)
                        };
                        effects.push(self.child(
                            state,
                            AppAction::Review(ReviewAction::PickRepoBase {
                                session: session.clone(),
                                repo,
                                base,
                            }),
                        ));
                    }
                }
                Effect::merge(effects)
            }
            ViewContent::Walkthrough(_) | ViewContent::Summary => {
                self.open_view_tab(state, id, &session)
            }
        }
    }

    /// Shows `id` as a tab on `session`'s page, with its step's file beside
    /// it.
    fn open_view_tab(&self, state: &mut AppState, id: &ViewId, session: &SessionId) -> Effect<AppAction> {
        let go = self.go_to_session(state, session);
        let tab = self.child(
            state,
            AppAction::Session(SessionAction::OpenView(id.to_string())),
        );
        let file = self.sync_view_file(state, id);
        go.and(tab).and(file)
    }

    /// Opens the To-do list showing `filter`.
    fn open_todo(&self, state: &mut AppState, filter: ViewFilter) -> Effect<AppAction> {
        let go = self.navigate(state, Route::Todo);
        let shown = self.child(state, AppAction::Todo(TodoAction::SetFilter(filter)));
        go.and(shown)
    }

    fn go_to_session(&self, state: &mut AppState, session: &SessionId) -> Effect<AppAction> {
        if state.route == Route::Session(session.clone()) {
            Effect::none()
        } else {
            self.navigate(state, Route::Session(session.clone()))
        }
    }

    fn revert_view(&self, state: &mut AppState, id: &ViewId) -> Effect<AppAction> {
        if !matches!(state.view_snapshots.last(), Some((held, _)) if held == id) {
            return Effect::none();
        }
        let Some((_, snap)) = state.view_snapshots.pop() else {
            return Effect::none();
        };
        state.view_files.remove(id);
        let under = state.view_snapshots.last().map(|(under, _)| under.clone());
        let session = snap.session.clone();
        let mut effects = Vec::new();
        state.trail = snap.trail.clone();
        if state.route != snap.route {
            state.route = snap.route.clone();
            effects.push(self.arrive(state));
        }
        let page = state.session.views.entry(session.clone()).or_default();
        page.groups = snap.groups;
        page.focused_group = snap.focused_group;
        page.tab = snap.tab;
        page.panel = snap.panel;
        page.widened = snap.widened;
        match snap.full_screen {
            Some(side) => {
                state.full_screens.insert(session.clone(), side);
            }
            None => {
                state.full_screens.remove(&session);
            }
        }
        let stack_before = (
            state.review.plans.get(&session).map(|plan| plan.stack.clone()),
            state.review.layer.get(&session).cloned().flatten(),
        );
        match snap.layer.clone() {
            Some(layer) => {
                state.review.layer.insert(session.clone(), Some(layer));
            }
            None => {
                state.review.layer.remove(&session);
            }
        }
        match snap.plan {
            Some(plan) => {
                state.review.plans.insert(session.clone(), plan);
            }
            None => {
                state.review.plans.remove(&session);
            }
        }
        state.layout.panel.collapsed = snap.collapsed;
        if snap.route == Route::Session(session.clone()) {
            state.panel_collapsed.insert(session.clone(), snap.collapsed);
        }
        if state.todo.view.filter != snap.todo {
            effects.push(self.child(state, AppAction::Todo(TodoAction::SetFilter(snap.todo))));
        }
        // The diff bases go back as they were, and the diff is read again
        // when they differ. Comments are not touched here.
        let stack_after = (
            state.review.plans.get(&session).map(|plan| plan.stack.clone()),
            snap.layer.clone(),
        );
        let stack_changed = stack_before != stack_after;
        if stack_changed && state.route == Route::Session(session.clone()) {
            effects.push(self.child(state, AppAction::Review(ReviewAction::Retry)));
        }
        if state.review.targets.get(&session) != snap.targets.as_ref() {
            match snap.targets {
                Some(targets) => {
                    state.review.targets.insert(session.clone(), targets.clone());
                    state.review.saved.targets.insert(session.clone(), targets);
                }
                None => {
                    state.review.targets.remove(&session);
                    state.review.saved.targets.remove(&session);
                }
            }
            if state.route == Route::Session(session) {
                effects.push(self.child(state, AppAction::Review(ReviewAction::Retry)));
            }
        }
        // The view under this one shows again.
        if let Some(under) = under {
            effects.push(self.child(state, AppAction::Views(ViewsAction::Resume(under))));
        }
        Effect::merge(effects)
    }
}

/// Where `tab` is on `session`'s page: its group and its index there.
fn tab_place(state: &AppState, session: &SessionId, tab: &Tab) -> Option<(usize, usize)> {
    state.session.views.get(session).and_then(|page| {
        page.groups.iter().enumerate().find_map(|(group, each)| {
            each.tabs
                .iter()
                .position(|held| held == tab)
                .map(|index| (group, index))
        })
    })
}

/// The file tab for a walkthrough step: the session's own spelling of the
/// step's `owner/name` repository (the name alone, or the same key), and
/// only for a file the session's clones hold.
fn step_tab(state: &AppState, session: &SessionId, step: &WalkthroughStep) -> Option<Tab> {
    let files = state.workspace.session_files.get(session)?;
    let held = |file: &&humata2_clients::model::SessionFile| {
        !file.folder && file.path == step.path
    };
    let found = files
        .iter()
        .filter(held)
        .find(|file| file.repo == step.repo)
        .or_else(|| {
            files
                .iter()
                .filter(held)
                .find(|file| is_name_suffix(&file.repo, &step.repo))
        })?;
    Some(Tab::File {
        repo: found.repo.clone(),
        path: step.path.clone(),
    })
}

/// Whether one of `a` and `b` is the other with an `owner/` before it.
fn is_name_suffix(a: &str, b: &str) -> bool {
    let after = |long: &str, short: &str| {
        long.strip_suffix(short)
            .is_some_and(|rest| rest.ends_with('/'))
    };
    after(a, b) || after(b, a)
}

/// Whether `a` and `b` name the same repository: identical, or one is the
/// other with an `owner/` prefix.
fn same_repo(a: &str, b: &str) -> bool {
    let a = a.strip_prefix('/').unwrap_or(a);
    let b = b.strip_prefix('/').unwrap_or(b);
    a == b || is_name_suffix(a, b)
}

/// Resolves a short repository name (like `"api"` or `"/api"`) against
/// `session`'s known `owner/name` repositories. Returns the input unchanged
/// (sans any stray leading `/`) if it already carries an owner or no known
/// repository matches.
pub(super) fn resolve_repo(state: &AppState, session: &SessionId, repo: &str) -> String {
    let trimmed = repo.strip_prefix('/').unwrap_or(repo);
    if trimmed.contains('/') {
        return trimmed.to_owned();
    }
    let has_owner = |candidate: &str| {
        candidate
            .split_once('/')
            .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty())
    };
    let mut known: Vec<&str> = Vec::new();
    if let Some(found) = state.workspace.session(session) {
        if !found.repo.is_empty() {
            known.push(&found.repo);
        }
        known.extend(found.clones.iter().map(|clone| clone.repo.as_str()));
        if let Some(channel) = state.workspace.channel(&found.channel) {
            known.extend(channel.repos.iter().map(String::as_str));
        }
    }
    known.extend(
        state
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == session)
            .map(|diff| diff.repo.as_str()),
    );
    known.extend(
        state
            .workspace
            .pulls
            .iter()
            .filter(|pull| pull.session.as_ref() == Some(session))
            .map(|pull| pull.repo.as_str()),
    );
    known
        .into_iter()
        .find(|candidate| has_owner(candidate) && same_repo(candidate, trimmed))
        .map_or_else(|| trimmed.to_owned(), str::to_owned)
}

/// Resolves any short repository names inside `plan` against `session`'s known
/// `owner/name` repositories.
pub(super) fn resolve_plan_repos(state: &AppState, session: &SessionId, plan: &mut PlanView) {
    for target in &mut plan.targets {
        target.repo = resolve_repo(state, session, &target.repo);
    }
    for (repo, _) in &mut plan.order {
        *repo = resolve_repo(state, session, repo);
    }
    for group in &mut plan.groups {
        group.first.0 = resolve_repo(state, session, &group.first.0);
    }
    for note in &mut plan.notes {
        note.repo = resolve_repo(state, session, &note.repo);
    }
    for tier in &mut plan.importance {
        tier.repo = resolve_repo(state, session, &tier.repo);
    }
    if let Some(stack) = &mut plan.stack {
        for spec in &mut stack.whole {
            spec.repo = resolve_repo(state, session, &spec.repo);
        }
        for layer in &mut stack.layers {
            for part in &mut layer.repos {
                part.repo = resolve_repo(state, session, &part.repo);
                part.spec.repo = resolve_repo(state, session, &part.spec.repo);
            }
            for (repo, _) in &mut layer.order {
                *repo = resolve_repo(state, session, repo);
            }
            for group in &mut layer.groups {
                group.first.0 = resolve_repo(state, session, &group.first.0);
            }
            for tier in &mut layer.importance {
                tier.repo = resolve_repo(state, session, &tier.repo);
            }
        }
    }
}

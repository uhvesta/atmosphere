//! [`ViewsClient`] for the real daemon, over its ViewService (VW-4 to
//! VW-10, P7). Changes arrive on the event feed narrowed to views; nothing
//! polls. Every answer is a SetViewState with a fresh idempotency key.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_clients::model::{
    Agent, ChannelId, FileTier, PreparedView, RepoBase, RepoDiffSpec, Scope, SessionId, ViewId, ViewState,
};
use humata2_clients::views::{
    LayerDrift, LayerRepoView, LayerView, PlanGroup, PlanNote, PlanTier, PlanView, StackView, ViewContent, ViewEvents, ViewRecord, ViewsClient, WalkthroughStep,
};
use humata2_clients::{DaemonError, Reply};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::view_service_client::ViewServiceClient;
use tokio::sync::mpsc;
use tonic::Streaming;

use crate::connection::{LiveDaemon, Shared, fresh_key};
use crate::map::{self, AgentNames};

/// Views the watcher may have waiting before the feed waits for it.
const QUEUE: usize = 16;

const PAGE_SIZE: u32 = 200;

/// Views on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LiveViews {
    shared: Arc<Shared>,
}

impl LiveViews {
    /// Views over `daemon`'s connection.
    #[must_use]
    pub fn new(daemon: &LiveDaemon) -> Self {
        Self {
            shared: daemon.shared.clone(),
        }
    }
}

fn client(shared: &Shared) -> ViewServiceClient<crate::connection::Wire> {
    ViewServiceClient::new(shared.wire.clone())
}

fn view_state(value: i32) -> Option<ViewState> {
    Some(match pb::ViewState::try_from(value).ok()? {
        pb::ViewState::Unspecified => return None,
        pb::ViewState::Draft => ViewState::Draft,
        pb::ViewState::Waiting => ViewState::Waiting,
        pb::ViewState::Later => ViewState::Later,
        pb::ViewState::NotNow => ViewState::NotNow,
        pb::ViewState::Opened => ViewState::Opened,
        pb::ViewState::Reverted => ViewState::Reverted,
        pb::ViewState::OpenedAndClosed => ViewState::OpenedAndClosed,
        pb::ViewState::Kept => ViewState::Kept,
    })
}

fn wire_state(state: ViewState) -> pb::ViewState {
    match state {
        ViewState::Draft => pb::ViewState::Draft,
        ViewState::Waiting => pb::ViewState::Waiting,
        ViewState::Later => pb::ViewState::Later,
        ViewState::NotNow => pb::ViewState::NotNow,
        ViewState::Opened => pb::ViewState::Opened,
        ViewState::Reverted => pb::ViewState::Reverted,
        ViewState::OpenedAndClosed => pb::ViewState::OpenedAndClosed,
        ViewState::Kept => pb::ViewState::Kept,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

/// The agent's name as people read it; empty when the offer names none. A
/// name the app has no agent for is shown as the daemon spells it.
fn agent_name(actor: Option<&pb::AgentActor>, agents: &AgentNames) -> String {
    let Some(name) = actor
        .and_then(|actor| actor.agent.as_ref())
        .map(|agent| agent.value.as_str())
        .filter(|name| !name.is_empty())
    else {
        return String::new();
    };
    let agent = agents.agent(name);
    if agent == Agent::Shell && name != Agent::Shell.command() {
        name.to_owned()
    } else {
        agent.name().to_owned()
    }
}

fn file(path: Option<&pb::RepoPath>) -> Option<(String, String)> {
    let path = path?;
    let repo = path.repo.as_ref().map(map::repo_text)?;
    Some((repo, path.path.clone()))
}

/// The base a plan's repository is compared with: the inverse of how the
/// Diff panel sends one.
pub(crate) fn repo_base(base: Option<&pb::DiffSpecBase>) -> RepoBase {
    use pb::diff_spec_base::Base;
    match base.and_then(|base| base.base.as_ref()) {
        None => RepoBase::DefaultBranch,
        Some(Base::MergeBaseWith(branch)) if branch == "origin/HEAD" => RepoBase::DefaultBranch,
        Some(Base::MergeBaseWith(branch)) => {
            RepoBase::Branch(branch.strip_prefix("origin/").unwrap_or(branch.as_str()).to_owned())
        }
        Some(Base::Rev(rev)) if rev == "HEAD" => RepoBase::Uncommitted,
        Some(Base::Rev(rev)) => RepoBase::Rev(rev.clone()),
        Some(Base::ReviewedSnapshot(_)) => RepoBase::LastReviewed,
        Some(Base::ForkPoint(_)) => RepoBase::ForkPoint,
        Some(Base::Index(_)) => RepoBase::Uncommitted,
        Some(Base::PullRequestBase(_)) => RepoBase::PullRequestBase,
    }
}

/// The agent's tiers (D802); an entry without a file is dropped.
fn tiers(list: &[pb::ReviewImportance]) -> Vec<PlanTier> {
    list.iter()
        .filter_map(|entry| {
            let (repo, path) = file(entry.file.as_ref())?;
            Some(PlanTier {
                repo,
                path,
                tier: crate::diff::file_tier(entry.tier),
            })
        })
        .collect()
}

/// The inverse of [`tiers`].
fn tiers_to_wire(list: &[PlanTier]) -> Vec<pb::ReviewImportance> {
    use pb::review_importance::Tier;
    list.iter()
        .map(|entry| pb::ReviewImportance {
            file: Some(repo_path_wire(&entry.repo, &entry.path)),
            tier: match entry.tier {
                FileTier::Key => Tier::Key,
                FileTier::Normal => Tier::Normal,
                FileTier::Skim => Tier::Skim,
            } as i32,
        })
        .collect()
}

fn plan_group(group: &pb::ReviewGroup) -> Option<PlanGroup> {
    Some(PlanGroup {
        title: group.title.clone(),
        note: group.note.clone(),
        first: file(group.first.as_ref())?,
    })
}

fn layer_drift(wire: &pb::ReviewLayerResolved) -> Option<LayerDrift> {
    use pb::review_layer_resolved::Drift;
    match Drift::try_from(wire.drift) {
        Ok(Drift::Moved) => Some(LayerDrift::Moved),
        Ok(Drift::Missing) => Some(LayerDrift::Missing),
        Ok(Drift::HeadMoved) => Some(LayerDrift::HeadMoved),
        _ => None,
    }
}

/// One layer's part in a repository: a range as it was sent, or its
/// uncommitted files on top of the commit HEAD resolved to.
fn layer_repo(wire: &pb::ReviewLayerRepo) -> Option<LayerRepoView> {
    use pb::review_layer_repo::Part;
    let repo = wire.repo.as_ref().map(map::repo_text)?;
    let base = match wire.part.as_ref()? {
        Part::Range(range) => RepoBase::Range {
            base: range.base.clone(),
            head: range.head.clone(),
        },
        Part::Worktree(worktree) => RepoBase::Overlay {
            base: wire
                .resolved
                .as_ref()
                .map(|resolved| resolved.head_commit.clone())
                .filter(|commit| !commit.is_empty())
                .unwrap_or_else(|| "HEAD".to_owned()),
            paths: worktree.include_paths.clone(),
        },
    };
    Some(LayerRepoView {
        repo: repo.clone(),
        spec: RepoDiffSpec { repo, base },
        pull_request: wire.pull_request.clone(),
        files: wire.resolved.as_ref().map_or(0, |resolved| resolved.files),
        drift: wire.resolved.as_ref().and_then(layer_drift),
    })
}

/// Each repository's Whole stack (D800): from the first layer's base to the
/// top range head, or, when the repository has uncommitted layers, that base
/// with the uncommitted files under all their paths. Repositories in the order
/// the layers first reach them.
fn whole_stack(layers: &[LayerView]) -> Vec<RepoDiffSpec> {
    let mut whole: Vec<(String, Option<String>, Option<String>, Vec<String>, bool)> = Vec::new();
    for part in layers.iter().flat_map(|layer| &layer.repos) {
        let at = match whole.iter().position(|each| each.0 == part.repo) {
            Some(at) => at,
            None => {
                whole.push((part.repo.clone(), None, None, Vec::new(), false));
                whole.len() - 1
            }
        };
        let entry = &mut whole[at];
        match &part.spec.base {
            RepoBase::Range { base, head } => {
                entry.1.get_or_insert_with(|| base.clone());
                entry.2 = Some(head.clone());
            }
            RepoBase::Overlay { base, paths } => {
                entry.1.get_or_insert_with(|| base.clone());
                entry.4 = true;
                for path in paths {
                    if !entry.3.contains(path) {
                        entry.3.push(path.clone());
                    }
                }
            }
            _ => {}
        }
    }
    whole
        .into_iter()
        .filter_map(|(repo, base, head, paths, overlay)| {
            let base = base?;
            let base = match (overlay, head) {
                (true, _) => RepoBase::Overlay { base, paths },
                (false, Some(head)) => RepoBase::Range { base, head },
                (false, None) => return None,
            };
            Some(RepoDiffSpec { repo, base })
        })
        .collect()
}

pub(crate) fn stack(wire: &pb::ReviewStack) -> StackView {
    let layers: Vec<LayerView> = wire
        .layers
        .iter()
        .map(|layer| LayerView {
            title: layer.title.clone(),
            purpose: layer.purpose.clone(),
            topic: layer.topic.clone(),
            repos: layer.repos.iter().filter_map(layer_repo).collect(),
            order: layer.order.iter().filter_map(|path| file(Some(path))).collect(),
            groups: layer.groups.iter().filter_map(plan_group).collect(),
            importance: tiers(&layer.importance),
        })
        .collect();
    let whole = whole_stack(&layers);
    StackView { layers, whole }
}

fn repo_path_wire(repo: &str, path: &str) -> pb::RepoPath {
    pb::RepoPath {
        repo: map::parse_repo(repo),
        path: path.to_owned(),
    }
}

/// A stack as the app reports it back to the daemon: the inverse of
/// [`stack`]. The daemon re-resolves the revs, so the commits are not sent.
pub(crate) fn stack_to_wire(stack: &StackView) -> pb::ReviewStack {
    use pb::review_layer_repo::Part;
    use pb::review_layer_resolved::Drift;
    pb::ReviewStack {
        layers: stack
            .layers
            .iter()
            .map(|layer| pb::ReviewLayer {
                title: layer.title.clone(),
                purpose: layer.purpose.clone(),
                topic: layer.topic.clone(),
                repos: layer
                    .repos
                    .iter()
                    .filter_map(|part| {
                        let (part_wire, head_commit) = match &part.spec.base {
                            RepoBase::Range { base, head } => (
                                Part::Range(pb::ReviewLayerRange {
                                    base: base.clone(),
                                    head: head.clone(),
                                }),
                                String::new(),
                            ),
                            RepoBase::Overlay { base, paths } => (
                                Part::Worktree(pb::ReviewLayerWorktree {
                                    include_paths: paths.clone(),
                                }),
                                base.clone(),
                            ),
                            _ => return None,
                        };
                        let drift = match part.drift {
                            None => Drift::None,
                            Some(LayerDrift::Moved) => Drift::Moved,
                            Some(LayerDrift::Missing) => Drift::Missing,
                            Some(LayerDrift::HeadMoved) => Drift::HeadMoved,
                        };
                        Some(pb::ReviewLayerRepo {
                            repo: map::parse_repo(&part.repo),
                            part: Some(part_wire),
                            pull_request: part.pull_request.clone(),
                            resolved: Some(pb::ReviewLayerResolved {
                                base_commit: String::new(),
                                head_commit,
                                drift: drift as i32,
                                files: part.files,
                            }),
                        })
                    })
                    .collect(),
                order: layer
                    .order
                    .iter()
                    .map(|(repo, path)| repo_path_wire(repo, path))
                    .collect(),
                groups: layer
                    .groups
                    .iter()
                    .map(|group| pb::ReviewGroup {
                        title: group.title.clone(),
                        note: group.note.clone(),
                        first: Some(repo_path_wire(&group.first.0, &group.first.1)),
                    })
                    .collect(),
                importance: tiers_to_wire(&layer.importance),
            })
            .collect(),
    }
}

pub(crate) fn plan(wire: &pb::ReviewPlan) -> PlanView {
    PlanView {
        stack: wire.stack.as_ref().map(stack),
        targets: wire
            .spec
            .iter()
            .flat_map(|spec| &spec.repos)
            .filter_map(|each| {
                Some(RepoDiffSpec {
                    repo: each.repo.as_ref().map(map::repo_text)?,
                    base: repo_base(each.base.as_ref()),
                })
            })
            .collect(),
        order: wire.order.iter().filter_map(|path| file(Some(path))).collect(),
        groups: wire.groups.iter().filter_map(plan_group).collect(),
        importance: tiers(&wire.importance),
        notes: wire
            .notes
            .iter()
            .filter_map(|note| {
                let (repo, path) = file(note.file.as_ref())?;
                Some(PlanNote {
                    repo,
                    path,
                    text: note.text.clone(),
                    lines: note.lines.as_ref().map(|lines| (lines.first, lines.last)),
                    outdated: note.outdated,
                })
            })
            .collect(),
    }
}

/// A board view: a channel's board, or a session's board. The offer carries
/// no channel for a session's, so the app finds it from its workspace.
fn board(wire: &pb::BoardViewOffer) -> ViewContent {
    use pb::board_view_offer::Scope as Offered;
    match wire.scope.as_ref() {
        Some(Offered::SessionWithForks(session)) if !session.value.is_empty() => {
            ViewContent::SessionBoard { session: SessionId::new(session.value.clone()), with_forks: true }
        }
        Some(Offered::OneSession(session)) if !session.value.is_empty() => {
            ViewContent::SessionBoard { session: SessionId::new(session.value.clone()), with_forks: false }
        }
        Some(Offered::Channel(channel) | Offered::ChannelWithSessions(channel)) if !channel.value.is_empty() => {
            ViewContent::Board(Scope::Channel(ChannelId::new(channel.value.clone())))
        }
        _ => ViewContent::Summary,
    }
}

fn content(wire: &pb::PreparedView) -> ViewContent {
    use pb::prepared_view::Content;
    match wire.content.as_ref() {
        Some(Content::Review(walkthrough)) => ViewContent::Walkthrough(
            walkthrough
                .steps
                .iter()
                .filter_map(|step| {
                    let (repo, path) = file(step.file.as_ref())?;
                    Some(WalkthroughStep {
                        repo,
                        path,
                        note: step.note.clone(),
                    })
                })
                .collect(),
        ),
        Some(Content::Board(offer)) => board(offer),
        Some(Content::PullRequest(_)) => ViewContent::PullRequest,
        Some(Content::ReviewPlan(wire)) => ViewContent::ReviewPlan(plan(wire)),
        None => ViewContent::Summary,
    }
}

/// A view as the app holds it; `None` when the daemon sent no id or a state
/// this app does not know.
fn record(wire: &pb::PreparedView, agents: &AgentNames) -> Option<ViewRecord> {
    let id = wire.id.as_ref().filter(|id| !id.value.is_empty())?;
    let state = view_state(wire.state)?;
    Some(ViewRecord {
        id: ViewId::new(id.value.clone()),
        view: PreparedView {
            title: wire.title.clone(),
            session: map::session_id(wire.session.as_ref()),
            body: wire.summary.clone(),
            accepted: matches!(
                state,
                ViewState::Opened | ViewState::OpenedAndClosed | ViewState::Kept
            ),
            note: wire.summary.clone(),
            prepared: wire
                .prepared_at
                .as_ref()
                .map(|at| crate::boards::ago(now_secs(), at.seconds))
                .unwrap_or_default(),
        },
        state,
        revision: wire.revision.as_ref().map_or(0, |revision| revision.value),
        prepared_by: agent_name(wire.prepared_by.as_ref(), agents),
        content: content(wire),
    })
}

const WHAT: &str = "These views";

impl Shared {
    /// A session's views, or with `None` every session's the person can see.
    async fn list_views(&self, session: Option<&SessionId>) -> Result<Vec<ViewRecord>, DaemonError> {
        let mut views = Vec::new();
        let mut token = String::new();
        loop {
            let reply = client(self)
                .list_views(pb::ListViewsRequest {
                    session: session.map(|session| pb::SessionId {
                        value: session.as_str().to_owned(),
                    }),
                    include_forks: false,
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: token,
                    }),
                })
                .await
                .map_err(|status| self.error(&status, WHAT))?
                .into_inner();
            views.extend(reply.views.iter().filter_map(|wire| record(wire, &self.agents)));
            token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if token.is_empty() {
                return Ok(views);
            }
        }
    }

    /// Opens the feed narrowed to views. It is open before this answers.
    async fn watch_views(self: &Arc<Self>) -> Result<ViewEvents, DaemonError> {
        let stream = self
            .event_stream()
            .subscribe(pb::SubscribeRequest {
                after: None,
                channels: Vec::new(),
                sessions: Vec::new(),
                topics: vec![pb::Topic::Views as i32],
            })
            .await
            .map_err(|status| self.error(&status, "Following views"))?
            .into_inner();
        let (sender, receiver) = mpsc::channel(QUEUE);
        self.spawn(forward(stream, sender, self.agents.clone()));
        Ok(receiver)
    }
}

/// Maps each view event until the feed ends or nobody listens. A view that
/// was deleted has no state to show and is left out.
async fn forward(mut stream: Streaming<pb::Event>, sender: mpsc::Sender<ViewRecord>, agents: AgentNames) {
    loop {
        let next = tokio::select! {
            () = sender.closed() => return,
            next = stream.message() => next,
        };
        match next {
            Ok(Some(event)) => {
                if event.change == pb::Change::Deleted as i32 {
                    continue;
                }
                let Some(pb::event::Payload::View(wire)) = event.payload else {
                    continue;
                };
                let Some(mapped) = record(&wire, &agents) else {
                    continue;
                };
                if sender.send(mapped).await.is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

impl ViewsClient for LiveViews {
    fn list(&self, session: &SessionId) -> Reply<Vec<ViewRecord>> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.list_views(Some(&session)).await })
    }

    fn list_offered(&self) -> Reply<Vec<ViewRecord>> {
        let shared = self.shared.clone();
        Box::pin(async move {
            let views = shared.list_views(None).await?;
            Ok(views
                .into_iter()
                .filter(|record| record.state != ViewState::Draft)
                .collect())
        })
    }

    fn set_state(&self, id: &ViewId, state: ViewState) -> Reply<ViewRecord> {
        let shared = self.shared.clone();
        let id = id.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .set_view_state(pb::SetViewStateRequest {
                    idempotency_key: fresh_key(),
                    id: Some(pb::ViewId {
                        value: id.as_str().to_owned(),
                    }),
                    state: wire_state(state) as i32,
                })
                .await
                .map_err(|status| shared.error(&status, "This view"))?
                .into_inner();
            record(&answer, &shared.agents).ok_or_else(|| DaemonError::Refused {
                what: "This view".to_owned(),
                reason: "the daemon answered with a view the app can't read; update Humata and try again"
                    .to_owned(),
            })
        })
    }

    fn watch(&self) -> Reply<ViewEvents> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.watch_views().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(state: pb::ViewState) -> pb::PreparedView {
        pb::PreparedView {
            id: Some(pb::ViewId { value: "vw_1".to_owned() }),
            session: Some(pb::SessionId { value: "se_1".to_owned() }),
            title: "Walkthrough".to_owned(),
            summary: "Three files".to_owned(),
            state: state as i32,
            revision: Some(pb::Revision { value: 4 }),
            ..pb::PreparedView::default()
        }
    }

    #[test]
    fn every_state_survives_the_wire() {
        for state in [
            ViewState::Draft,
            ViewState::Waiting,
            ViewState::Later,
            ViewState::NotNow,
            ViewState::Opened,
            ViewState::Reverted,
            ViewState::OpenedAndClosed,
            ViewState::Kept,
        ] {
            assert_eq!(view_state(wire_state(state) as i32), Some(state));
        }
    }

    #[test]
    fn a_view_takes_its_id_session_state_and_revision() {
        let mapped = record(&wire(pb::ViewState::Opened), &AgentNames::default())
            .expect("a view with an id and a state maps");
        assert_eq!(mapped.id, ViewId::new("vw_1"));
        assert_eq!(mapped.view.session, SessionId::new("se_1"));
        assert_eq!(mapped.state, ViewState::Opened);
        assert!(mapped.view.accepted);
        assert_eq!(mapped.revision, 4);
    }

    #[test]
    fn a_waiting_view_is_not_accepted() {
        let mapped = record(&wire(pb::ViewState::Waiting), &AgentNames::default()).expect("maps");
        assert!(!mapped.view.accepted);
    }

    #[test]
    fn a_view_without_an_id_or_a_state_is_left_out() {
        let mut no_id = wire(pb::ViewState::Waiting);
        no_id.id = None;
        let agents = AgentNames::default();
        assert_eq!(record(&no_id, &agents), None);
        assert_eq!(record(&wire(pb::ViewState::Unspecified), &agents), None);
    }

    fn path(repo: &str, file: &str) -> pb::RepoPath {
        let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
        pb::RepoPath {
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            path: file.to_owned(),
        }
    }

    #[test]
    fn a_walkthrough_keeps_its_steps_in_order_and_who_prepared_it() {
        let mut view = wire(pb::ViewState::Waiting);
        view.prepared_by = Some(pb::AgentActor {
            agent: Some(pb::AgentId { value: "claude".to_owned() }),
            ..pb::AgentActor::default()
        });
        view.content = Some(pb::prepared_view::Content::Review(pb::ReviewWalkthrough {
            steps: vec![
                pb::review_walkthrough::Step {
                    file: Some(path("a/b", "src/one.rs")),
                    lines: None,
                    note: "first".to_owned(),
                },
                pb::review_walkthrough::Step {
                    file: Some(path("a/b", "src/two.rs")),
                    lines: None,
                    note: "second".to_owned(),
                },
            ],
        }));
        let mapped = record(&view, &AgentNames::default()).expect("maps");
        assert_eq!(mapped.prepared_by, Agent::Claude.name());
        let ViewContent::Walkthrough(steps) = mapped.content else {
            panic!("a walkthrough maps to steps");
        };
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].path, "src/one.rs");
        assert_eq!(steps[1].repo, "a/b");
        assert_eq!(steps[1].note, "second");
    }

    #[test]
    fn a_review_plan_maps_its_bases_back_to_the_apps() {
        use pb::diff_spec_base::Base;
        let spec = |repo: &str, base: Base| pb::RepoDiffSpec {
            repo: map::parse_repo(repo),
            base: Some(pb::DiffSpecBase { base: Some(base) }),
            ..pb::RepoDiffSpec::default()
        };
        let mut view = wire(pb::ViewState::Waiting);
        view.content = Some(pb::prepared_view::Content::ReviewPlan(pb::ReviewPlan {
            spec: Some(pb::DiffSpec {
                repos: vec![
                    spec("a/b", Base::MergeBaseWith("origin/HEAD".to_owned())),
                    spec("a/c", Base::MergeBaseWith("origin/release".to_owned())),
                    spec("a/d", Base::Rev("HEAD".to_owned())),
                ],
            }),
            order: vec![path("a/b", "x.rs")],
            ..pb::ReviewPlan::default()
        }));
        let mapped = record(&view, &AgentNames::default()).expect("maps");
        let ViewContent::ReviewPlan(plan) = mapped.content else {
            panic!("a review plan maps to a plan");
        };
        let bases: Vec<RepoBase> = plan.targets.iter().map(|each| each.base.clone()).collect();
        assert_eq!(
            bases,
            vec![
                RepoBase::DefaultBranch,
                RepoBase::Branch("release".to_owned()),
                RepoBase::Uncommitted
            ]
        );
        assert_eq!(plan.order, vec![("a/b".to_owned(), "x.rs".to_owned())]);
    }

    #[test]
    fn a_stack_plan_maps_layers_pull_requests_and_the_whole_stack() {
        use pb::review_layer_repo::Part;
        let repo = |name: &str, part: Part, pr: &str, head: &str| pb::ReviewLayerRepo {
            repo: map::parse_repo(name),
            part: Some(part),
            pull_request: pr.to_owned(),
            resolved: Some(pb::ReviewLayerResolved {
                head_commit: head.to_owned(),
                files: 3,
                drift: pb::review_layer_resolved::Drift::Moved as i32,
                ..pb::ReviewLayerResolved::default()
            }),
        };
        let range = |base: &str, head: &str| {
            Part::Range(pb::ReviewLayerRange {
                base: base.to_owned(),
                head: head.to_owned(),
            })
        };
        let layer = |title: &str, repos: Vec<pb::ReviewLayerRepo>| pb::ReviewLayer {
            title: title.to_owned(),
            repos,
            ..pb::ReviewLayer::default()
        };
        let plan = plan(&pb::ReviewPlan {
            stack: Some(pb::ReviewStack {
                layers: vec![
                    layer("one", vec![repo("a/b", range("m", "x1"), "a/b#1", "")]),
                    layer("two", vec![repo("a/b", range("x1", "x2"), "a/b#2", "")]),
                    layer(
                        "wip",
                        vec![repo(
                            "a/b",
                            Part::Worktree(pb::ReviewLayerWorktree {
                                include_paths: vec!["src".to_owned()],
                            }),
                            "",
                            "x2",
                        )],
                    ),
                ],
            }),
            ..pb::ReviewPlan::default()
        });
        let Some(stack) = plan.stack else {
            panic!("a stack plan carries its stack");
        };
        let titles: Vec<&str> = stack.layers.iter().map(|each| each.title.as_str()).collect();
        assert_eq!(titles, vec!["one", "two", "wip"]);
        assert_eq!(stack.layers[1].repos[0].pull_request, "a/b#2");
        assert_eq!(
            stack.layers[1].repos[0].spec.base,
            RepoBase::Range {
                base: "x1".to_owned(),
                head: "x2".to_owned()
            }
        );
        assert_eq!(
            stack.layers[2].repos[0].spec.base,
            RepoBase::Overlay {
                base: "x2".to_owned(),
                paths: vec!["src".to_owned()]
            }
        );
        assert_eq!(stack.layers[0].repos[0].files, 3);
        assert_eq!(stack.layers[0].repos[0].drift, Some(LayerDrift::Moved));
        assert_eq!(
            stack.whole,
            vec![RepoDiffSpec {
                repo: "a/b".to_owned(),
                base: RepoBase::Overlay {
                    base: "m".to_owned(),
                    paths: vec!["src".to_owned()]
                }
            }]
        );
    }

    #[test]
    fn whole_stack_of_committed_layers_runs_from_the_first_base_to_the_top_head() {
        let part = |base: &str, head: &str| LayerRepoView {
            repo: "a/b".to_owned(),
            spec: RepoDiffSpec {
                repo: "a/b".to_owned(),
                base: RepoBase::Range {
                    base: base.to_owned(),
                    head: head.to_owned(),
                },
            },
            pull_request: String::new(),
            files: 0,
            drift: None,
        };
        let layer = |repos| LayerView {
            repos,
            ..LayerView::default()
        };
        let whole = whole_stack(&[layer(vec![part("m", "1")]), layer(vec![part("1", "2")])]);
        assert_eq!(
            whole[0].base,
            RepoBase::Range {
                base: "m".to_owned(),
                head: "2".to_owned()
            }
        );
    }

    #[test]
    fn a_channel_board_view_names_its_scope() {
        let mut view = wire(pb::ViewState::Waiting);
        view.content = Some(pb::prepared_view::Content::Board(pb::BoardViewOffer {
            scope: Some(pb::board_view_offer::Scope::Channel(pb::ChannelId {
                value: "ch_1".to_owned(),
            })),
        }));
        let mapped = record(&view, &AgentNames::default()).expect("maps");
        assert_eq!(mapped.content, ViewContent::Board(Scope::Channel(ChannelId::new("ch_1"))));
    }

    #[test]
    fn a_session_board_view_names_its_session_and_whether_forks_come_too() {
        let offered = |scope: pb::board_view_offer::Scope| {
            let mut view = wire(pb::ViewState::Waiting);
            view.content = Some(pb::prepared_view::Content::Board(pb::BoardViewOffer { scope: Some(scope) }));
            record(&view, &AgentNames::default()).expect("maps").content
        };
        let id = || pb::SessionId { value: "se_9".to_owned() };
        assert_eq!(
            offered(pb::board_view_offer::Scope::SessionWithForks(id())),
            ViewContent::SessionBoard { session: SessionId::new("se_9"), with_forks: true }
        );
        assert_eq!(
            offered(pb::board_view_offer::Scope::OneSession(id())),
            ViewContent::SessionBoard { session: SessionId::new("se_9"), with_forks: false }
        );
    }
}

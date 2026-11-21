//! Reviewing a pull request with an agent beside you, in the app: what the
//! start dialog, the follower, Submit review and the new-commits notices ask
//! of the app (PR-1 to PR-26), and following a review's chat when its session
//! opens.

use humata2_clients::comments::Side;
use humata2_clients::model::{MachineKind, RepoDiffOptions, RepoDiffSpec, SessionId};
use humata2_clients::review_chat::{Place, ReviewView};
use humata2_palette::PullRef;
use humata2_pr_review::ask::AskAction;
use humata2_pr_review::dialog::{Delegate as DialogDelegate, DialogAction};
use humata2_pr_review::follower::{Delegate as FollowerDelegate, FollowerAction};
use humata2_pr_review::heads::{Delegate as HeadsDelegate, HeadsAction};
use humata2_pr_review::info::InfoAction;
use humata2_pr_review::submit::{Delegate as SubmitDelegate, SubmitAction};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use tca::Effect;

use crate::{App, AppAction, AppState, Route};

/// The name the daemon makes the PR reviews channel with.
const PR_REVIEWS_CHANNEL: &str = "PR reviews";

impl App {
    /// Handles what a pull request review part asked for.
    pub(super) fn pr_review_delegate(&self, state: &mut AppState, action: &AppAction) -> Effect<AppAction> {
        match action {
            // The review's session exists: open it, as the sidebar does.
            AppAction::PrDialog(DialogAction::Delegate(DialogDelegate::Opened(id))) => {
                self.switch(state, Route::Session(id.clone()))
            }
            // Ask took the box's text as its question: the box closes, and
            // the comment it made for the agent goes with it (PR-19).
            AppAction::PrAsk(AskAction::Ask { session, .. }) => {
                self.child(state, AppAction::Comments(CommentsAction::Cancel(session.clone())))
            }
            // The agent moved the view: show that file in the Diff panel.
            // The review feature has no scroll-to-line action yet, so the
            // line of the place is not revealed.
            AppAction::PrFollower(FollowerAction::Delegate(FollowerDelegate::Show(place))) => {
                let select = self.child(
                    state,
                    AppAction::Review(ReviewAction::SelectFile {
                        repo: place.repo.clone(),
                        path: place.path.clone(),
                    }),
                );
                let report = self.pr_session(state).map_or_else(Effect::none, |session| {
                    self.report_review_view(state, &session)
                });
                select.and(report)
            }
            // The agent moved the stack to a layer (or Whole stack).
            AppAction::PrFollower(FollowerAction::Delegate(FollowerDelegate::ShowLayer(layer))) => {
                let pick = self.child(state, AppAction::Review(ReviewAction::PickLayer(layer.clone())));
                let report = self.pr_session(state).map_or_else(Effect::none, |session| {
                    self.report_review_view(state, &session)
                });
                pick.and(report)
            }
            // The agent prepared (or undid) a review plan for the PR review.
            AppAction::PrFollower(FollowerAction::Delegate(FollowerDelegate::Plan(plan))) => {
                let Some(session) = self.pr_session(state) else {
                    return Effect::none();
                };
                let mut effects = Vec::new();
                match plan {
                    Some(plan) => {
                        let mut plan = plan.clone();
                        crate::views::resolve_plan_repos(state, &session, &mut plan);
                        let stacked = plan.stack.is_some();
                        let targets = plan.targets.clone();
                        state.review.plans.insert(session.clone(), plan);
                        state.review.layer.remove(&session);
                        if stacked || targets.is_empty() {
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
                    }
                    None => {
                        state.review.plans.remove(&session);
                        state.review.layer.remove(&session);
                        effects.push(self.child(state, AppAction::Review(ReviewAction::Retry)));
                    }
                }
                effects.push(self.report_review_view(state, &session));
                Effect::merge(effects)
            }
            // Every review posted: Submit review is done.
            AppAction::PrSubmit(SubmitAction::Delegate(SubmitDelegate::Posted(_))) => {
                self.child(state, AppAction::PrSubmit(SubmitAction::Close))
            }
            // The pull request merged or closed: the session's own archive
            // dialog names what would be lost.
            AppAction::PrHeads(HeadsAction::Delegate(HeadsDelegate::Archiving(id))) => {
                self.archive_session(state, id.clone())
            }
            _ => Effect::none(),
        }
    }

    /// Review a pull request (PR-1, reverses D197): the session that already
    /// reviews it opens; otherwise the Review a pull request dialog opens with
    /// its link pasted, and GitHub is asked about it at once.
    pub(super) fn review_pull(&self, state: &mut AppState, pull: &PullRef) -> Effect<AppAction> {
        let reviewed = state
            .workspace
            .pulls
            .iter()
            .find(|p| p.number == pull.number && p.repo == format!("{}/{}", pull.owner, pull.repo))
            .and_then(|p| p.session.clone());
        if let Some(session) = reviewed {
            return self.navigate(state, Route::Session(session));
        }
        let link = format!("https://github.com/{}/{}/pull/{}", pull.owner, pull.repo, pull.number);
        // The reviews' own channel says which agent a start takes there
        // (D770); before the first review makes it, Settings' pick does.
        let reviews = state
            .workspace
            .channels
            .iter()
            .find(|channel| channel.name.eq_ignore_ascii_case(PR_REVIEWS_CHANNEL))
            .map(|channel| channel.id.clone());
        let Ok(agent) = state.default_agent(reviews.as_ref()) else {
            let clock = self.clients.clock.clone();
            return state.toast_no_agent(&clock);
        };
        // This Mac when it answers, else the first machine that does, as New
        // channel picks; none leaves Start waiting for a pick.
        let machine = state
            .workspace
            .machines
            .iter()
            .find(|machine| machine.online && matches!(machine.kind, MachineKind::ThisMac))
            .or_else(|| state.workspace.machines.iter().find(|machine| machine.online))
            .map(|machine| machine.id.clone());
        let open = self.child(state, AppAction::PrDialog(DialogAction::Open { machine, agent }));
        let paste = self.child(state, AppAction::PrDialog(DialogAction::Paste(link)));
        open.and(paste)
    }

    /// Follows the chat of a review session, once, when it opens.
    pub(super) fn follow_review(&self, state: &mut AppState, id: &SessionId) -> Effect<AppAction> {
        let Some(push_allowed) = state
            .workspace
            .session(id)
            .filter(|session| session.pull_request_review)
            .map(|session| session.push_allowed)
        else {
            return Effect::none();
        };
        if state.pr_follower.session.as_ref() == Some(id) {
            return Effect::none();
        }
        let info = self.child(
            state,
            AppAction::PrInfo(InfoAction::Open {
                session: id.clone(),
                push_allowed,
            }),
        );
        let follow = self.child(state, AppAction::PrFollower(FollowerAction::Follow(id.clone())));
        let report = self.report_review_view(state, id);
        info.and(follow).and(report)
    }

    /// The PR review session currently being followed, or the open session.
    fn pr_session(&self, state: &AppState) -> Option<SessionId> {
        state
            .pr_follower
            .session
            .clone()
            .or_else(|| state.session.open.clone())
    }

    /// Reports the current PR review view to the daemon so `review_context`
    /// reflects what the person sees (PR-22).
    pub(super) fn report_review_view(&self, state: &AppState, session: &SessionId) -> Effect<AppAction> {
        let view = build_review_view(state, session);
        let client = self.clients.review_chat.clone();
        let session = session.clone();
        Effect::run(move |_| async move {
            let _ = client.report_view(&session, view).await;
        })
    }
}

/// Builds the [`ReviewView`] snapshot of what `session`'s PR review currently
/// shows.
fn build_review_view(state: &AppState, session: &SessionId) -> ReviewView {
    let plan = state.review.plans.get(session);
    let layer = state.review.layer_of(session).map(str::to_owned);
    let targets = if let Some(stack) = plan.and_then(|plan| plan.stack.as_ref()) {
        match layer.as_deref().and_then(|title| stack.layers.iter().find(|l| l.title == title)) {
            Some(picked) => picked.repos.iter().map(|repo| repo.spec.clone()).collect(),
            None => stack.whole.clone(),
        }
    } else if let Some(plan) = plan.filter(|plan| !plan.targets.is_empty()) {
        plan.targets.clone()
    } else {
        let targets = state.review.targets_of(session);
        let mut repos: Vec<String> = Vec::new();
        if let Some(found) = state.workspace.session(session) {
            if !found.repo.is_empty() && !repos.contains(&found.repo) {
                repos.push(found.repo.clone());
            }
            for clone in &found.clones {
                if !clone.repo.is_empty() && !repos.contains(&clone.repo) {
                    repos.push(clone.repo.clone());
                }
            }
        }
        for diff in state.workspace.diffs.iter().filter(|diff| &diff.session == session) {
            if !repos.contains(&diff.repo) {
                repos.push(diff.repo.clone());
            }
        }
        repos
            .into_iter()
            .map(|repo| RepoDiffSpec {
                base: targets.of(&repo),
                repo,
                options: RepoDiffOptions::default(),
            })
            .collect()
    };
    let open = state
        .review
        .saved
        .selected
        .get(session)
        .map(|(repo, path)| Place {
            repo: repo.clone(),
            path: path.clone(),
            visible: None,
            selection: None,
            side: Side::New,
        })
        .or_else(|| state.pr_follower.shown.clone());
    ReviewView {
        targets,
        open,
        order: plan.map(|plan| plan.order.clone()).unwrap_or_default(),
        groups: plan.map(|plan| plan.groups.clone()).unwrap_or_default(),
        viewed: Vec::new(),
        highlights: state.pr_follower.highlights.clone(),
        stack: plan.and_then(|plan| plan.stack.clone()),
        layer,
    }
}

//! Reviewing a pull request with an agent beside you, in the app: what the
//! start dialog, the follower, Submit review and the new-commits notices ask
//! of the app (PR-1 to PR-26), and following a review's chat when its session
//! opens.

use humata2_clients::model::{MachineKind, SessionId};
use humata2_palette::PullRef;
use humata2_pr_review::ask::AskAction;
use humata2_pr_review::dialog::{Delegate as DialogDelegate, DialogAction};
use humata2_pr_review::follower::{Delegate as FollowerDelegate, FollowerAction};
use humata2_pr_review::heads::{Delegate as HeadsDelegate, HeadsAction};
use humata2_pr_review::info::InfoAction;
use humata2_pr_review::submit::{Delegate as SubmitDelegate, SubmitAction};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::SessionAction;
use tca::Effect;

use crate::{App, AppAction, AppState, Route};

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
            AppAction::PrFollower(FollowerAction::Delegate(FollowerDelegate::Show(place))) => self.child(
                state,
                AppAction::Review(ReviewAction::SelectFile {
                    repo: place.repo.clone(),
                    path: place.path.clone(),
                }),
            ),
            // Every review posted: Submit review is done.
            AppAction::PrSubmit(SubmitAction::Delegate(SubmitDelegate::Posted(_))) => {
                self.child(state, AppAction::PrSubmit(SubmitAction::Close))
            }
            // The pull request merged or closed: the session's own archive
            // dialog names what would be lost.
            AppAction::PrHeads(HeadsAction::Delegate(HeadsDelegate::Archiving(id))) => {
                let arrived = if state.session.open.as_ref() == Some(id) {
                    Effect::none()
                } else {
                    self.switch(state, Route::Session(id.clone()))
                };
                let archive = self.child(state, AppAction::Session(SessionAction::Archive));
                arrived.and(archive)
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
        let agent = crate::new_session::default_agent(state);
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
        info.and(follow)
    }
}

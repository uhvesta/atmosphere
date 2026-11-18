//! The start dialog (PR-1 to PR-8, D283, D287): the pasted pull request is
//! resolved at once, more pull requests go under "Also review", and the
//! person picks the machine, the agent and what to tell it first. Each
//! problem shows where it blocks Start; Start opens the new session.

use std::sync::Arc;

use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::pull_requests::{
    PullRequestError, PullRequestInfo, PullRequestRef, PullRequestsClient, ReviewStart,
};
use tca::{Effect, Reducer};

/// Where reading a pull request from GitHub stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch {
    /// Asking GitHub with the machine's own gh.
    Fetching,
    /// GitHub answered.
    Found(PullRequestInfo),
    /// It couldn't be read. The error says what to do; a rate limit says to
    /// wait and for how long. The card offers Try again.
    Failed(PullRequestError),
}

/// A pull request in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Which pull request.
    pub pr: PullRequestRef,
    /// What reading it came to.
    pub fetch: Fetch,
}

/// Why a pasted text wasn't added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteProblem {
    /// The text isn't a pull request link or `owner/name#number`.
    NotAPullRequest {
        /// What was pasted.
        text: String,
    },
    /// The pull request is in the dialog already.
    AlreadyAdded(PullRequestRef),
}

/// Where starting the review stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Start {
    /// Not asked yet.
    Idle,
    /// The daemon is making the session.
    Starting,
    /// The daemon couldn't start it; the error says what to do.
    Failed(PullRequestError),
}

/// What stops Start, and where it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// No pull request was pasted.
    NoPullRequest,
    /// This pull request is still being read.
    Fetching(PullRequestRef),
    /// This pull request couldn't be read; its card shows why.
    CannotRead(PullRequestRef),
    /// No machine is picked.
    NoMachine,
    /// The session is being made already.
    Starting,
}

/// The dialog's state.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogState {
    /// Whether the dialog shows.
    pub open: bool,
    /// The pasted pull request, the review's main one.
    pub main: Option<Candidate>,
    /// The pull requests under "Also review".
    pub also: Vec<Candidate>,
    /// Why the last paste wasn't added.
    pub paste_problem: Option<PasteProblem>,
    /// The machine the clones are made on.
    pub machine: Option<MachineId>,
    /// The agent the review runs.
    pub agent: Agent,
    /// The agent's model, or empty for its default.
    pub model: String,
    /// What the agent is told first.
    pub context: String,
    /// Whether the agent may push (PR-8). Off until the person allows it.
    pub push_allowed: bool,
    /// Where Start stands.
    pub start: Start,
}

impl Default for DialogState {
    fn default() -> Self {
        Self {
            open: false,
            main: None,
            also: Vec::new(),
            paste_problem: None,
            machine: None,
            agent: Agent::Claude,
            model: String::new(),
            context: String::new(),
            push_allowed: false,
            start: Start::Idle,
        }
    }
}

impl DialogState {
    /// What stops Start now, if anything.
    #[must_use]
    pub fn blocker(&self) -> Option<Blocker> {
        let Some(main) = &self.main else {
            return Some(Blocker::NoPullRequest);
        };
        for candidate in std::iter::once(main).chain(&self.also) {
            match candidate.fetch {
                Fetch::Fetching => return Some(Blocker::Fetching(candidate.pr.clone())),
                Fetch::Failed(_) => return Some(Blocker::CannotRead(candidate.pr.clone())),
                Fetch::Found(_) => {}
            }
        }
        if self.machine.is_none() {
            return Some(Blocker::NoMachine);
        }
        (self.start == Start::Starting).then_some(Blocker::Starting)
    }

    /// The pull requests in the dialog, the main one first.
    #[must_use]
    pub fn pull_requests(&self) -> Vec<&PullRequestRef> {
        self.main
            .iter()
            .chain(&self.also)
            .map(|candidate| &candidate.pr)
            .collect()
    }

    fn holds(&self, pr: &PullRequestRef) -> bool {
        self.pull_requests().contains(&pr)
    }

    fn candidate_mut(&mut self, pr: &PullRequestRef) -> Option<&mut Candidate> {
        self.main
            .iter_mut()
            .chain(self.also.iter_mut())
            .find(|candidate| &candidate.pr == pr)
    }
}

/// What happens in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogAction {
    /// Show the dialog, empty, with the machine and agent the person
    /// usually picks.
    Open {
        /// The machine to start on, when there is a default.
        machine: Option<MachineId>,
        /// The agent to start with.
        agent: Agent,
    },
    /// Hide the dialog and forget what it held.
    Close,
    /// A link or `owner/name#number` was pasted: it becomes the main pull
    /// request and is read.
    Paste(String),
    /// A link was pasted under "Also review".
    AddAlso(String),
    /// Take a pull request out of "Also review".
    RemoveAlso(PullRequestRef),
    /// GitHub answered for a pull request.
    Resolved {
        /// The pull request asked about.
        pr: PullRequestRef,
        /// What it is, or why it couldn't be read.
        result: Result<PullRequestInfo, PullRequestError>,
    },
    /// Try again after a failure.
    Retry(PullRequestRef),
    /// Pick the machine.
    PickMachine(MachineId),
    /// Pick the agent.
    PickAgent(Agent),
    /// Set the agent's model.
    SetModel(String),
    /// Set what the agent is told first.
    SetContext(String),
    /// Let the agent push, or not (PR-8).
    AllowPush(bool),
    /// Start the review.
    Start,
    /// The daemon answered Start.
    Started(Result<SessionId, PullRequestError>),
    /// For the parent.
    Delegate(Delegate),
}

/// What the dialog asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// The review's session exists: open it.
    Opened(SessionId),
}

/// The dialog's reducer.
#[derive(Clone)]
pub struct Dialog {
    client: Arc<dyn PullRequestsClient>,
}

impl Dialog {
    /// A dialog that asks `client`.
    #[must_use]
    pub fn new(client: Arc<dyn PullRequestsClient>) -> Self {
        Self { client }
    }

    fn resolve(&self, pr: PullRequestRef) -> Effect<DialogAction> {
        let client = self.client.clone();
        Effect::future(async move {
            let result = client.resolve(&pr).await;
            DialogAction::Resolved { pr, result }
        })
    }

    /// Adds `text` as the main pull request or under "Also review".
    fn add(&self, state: &mut DialogState, text: &str, as_main: bool) -> Effect<DialogAction> {
        let Some(pr) = PullRequestRef::parse(text) else {
            state.paste_problem = Some(PasteProblem::NotAPullRequest {
                text: text.trim().to_owned(),
            });
            return Effect::none();
        };
        if state.holds(&pr) && !(as_main && state.main.as_ref().is_some_and(|main| main.pr != pr)) {
            state.paste_problem = Some(PasteProblem::AlreadyAdded(pr));
            return Effect::none();
        }
        state.paste_problem = None;
        state.start = Start::Idle;
        let candidate = Candidate {
            pr: pr.clone(),
            fetch: Fetch::Fetching,
        };
        if as_main || state.main.is_none() {
            state.also.retain(|held| held.pr != pr);
            state.main = Some(candidate);
        } else {
            state.also.push(candidate);
        }
        self.resolve(pr)
    }
}

impl Reducer for Dialog {
    type State = DialogState;
    type Action = DialogAction;

    fn reduce(&self, state: &mut DialogState, action: DialogAction) -> Effect<DialogAction> {
        match action {
            DialogAction::Open { machine, agent } => {
                *state = DialogState {
                    open: true,
                    machine,
                    agent,
                    ..DialogState::default()
                };
            }
            DialogAction::Close => *state = DialogState::default(),
            DialogAction::Paste(text) => return self.add(state, &text, true),
            DialogAction::AddAlso(text) => return self.add(state, &text, false),
            DialogAction::RemoveAlso(pr) => {
                state.also.retain(|candidate| candidate.pr != pr);
                state.start = Start::Idle;
            }
            DialogAction::Resolved { pr, result } => {
                // An answer for a pull request taken out, or asked again
                // since, is old news.
                if let Some(candidate) = state.candidate_mut(&pr)
                    && candidate.fetch == Fetch::Fetching
                {
                    candidate.fetch = match result {
                        Ok(info) => Fetch::Found(info),
                        Err(error) => Fetch::Failed(error),
                    };
                }
            }
            DialogAction::Retry(pr) => {
                if let Some(candidate) = state.candidate_mut(&pr)
                    && matches!(candidate.fetch, Fetch::Failed(_))
                {
                    candidate.fetch = Fetch::Fetching;
                    return self.resolve(pr);
                }
            }
            DialogAction::PickMachine(machine) => state.machine = Some(machine),
            DialogAction::PickAgent(agent) => state.agent = agent,
            DialogAction::SetModel(model) => state.model = model,
            DialogAction::SetContext(context) => state.context = context,
            DialogAction::AllowPush(allowed) => state.push_allowed = allowed,
            DialogAction::Start => {
                if state.blocker().is_some() {
                    return Effect::none();
                }
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                let start = ReviewStart {
                    prs: state.pull_requests().into_iter().cloned().collect(),
                    machine,
                    agent: state.agent,
                    model: state.model.clone(),
                    first_message: state.context.clone(),
                    push_allowed: state.push_allowed,
                };
                state.start = Start::Starting;
                let client = self.client.clone();
                return Effect::future(async move {
                    DialogAction::Started(client.start_review(start).await)
                });
            }
            DialogAction::Started(result) => match result {
                Ok(session) => {
                    *state = DialogState::default();
                    return Effect::send(DialogAction::Delegate(Delegate::Opened(session)));
                }
                Err(error) => state.start = Start::Failed(error),
            },
            DialogAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

//! New desk session (DK-14, frame 40). The form never asks about the
//! folder: every new session gets one Humata makes.

use humata2_clients::desk::{self, Access, FolderChoice, NewDeskSession};
use humata2_clients::model::{Agent, MachineId};
use tca::Effect;

use crate::{Desk, DeskAction, DeskState};

/// The New desk session form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewForm {
    /// The title; blank gets one from the agent.
    pub title: String,
    /// What it runs; [`Agent::Shell`] is "Terminal".
    pub agent: Agent,
    /// The machine it runs on.
    pub machine: Option<MachineId>,
    /// Repositories to clone, such as `acme/ledger`.
    pub repos: Vec<String>,
    /// The two Agent access switches.
    pub access: Access,
    /// The Machine pop-up is open.
    pub machines_open: bool,
    /// The field for typing a repository to add is showing.
    pub adding_repo: bool,
    /// The start is in flight.
    pub starting: bool,
    /// Why it didn't start, to show on the form.
    pub error: Option<String>,
}

impl NewForm {
    /// The title the session starts with: what was typed, or "New <agent>
    /// session" when that is blank.
    #[must_use]
    pub fn start_title(&self) -> String {
        if self.title.trim().is_empty() {
            format!("New {} session", self.agent.name())
        } else {
            self.title.trim().to_owned()
        }
    }

    /// The scratch folder's name (DK-2, D148): the FC-13 slug of the title
    /// the session starts with, or `desk-session` when that has no letter
    /// or digit. The daemon adds a suffix when the name is taken.
    #[must_use]
    pub fn scratch_name(&self) -> String {
        let name = desk::folder_name(&self.start_title());
        if name.is_empty() {
            "desk-session".to_owned()
        } else {
            name
        }
    }
}

/// A change to the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewAction {
    /// The title was typed.
    Title(String),
    /// An agent, or Terminal, was picked.
    Agent(Agent),
    /// A machine was picked.
    Machine(MachineId),
    /// The Machine pop-up was opened or closed.
    Machines(bool),
    /// The field for typing a repository was shown or hidden.
    AddingRepo(bool),
    /// A repository was added.
    AddRepo(String),
    /// A repository was removed.
    RemoveRepo(String),
    /// A repository was ticked or unticked in the repo search; the search
    /// stays open for more.
    ToggleRepo(String),
    /// The access switches were set.
    Access(Access),
    /// Start was clicked.
    Start,
}

impl DeskState {
    /// The path the new session's scratch folder will get, for the dialog to
    /// show (DK-14, D148): the form's name, with a suffix when a Desk
    /// session this workspace knows on the picked machine already works
    /// there (FC-14). `None` when no form is open. The daemon has the last
    /// word when the session starts.
    #[must_use]
    pub fn scratch_preview(&self) -> Option<String> {
        let form = self.new.as_ref()?;
        let name = form.scratch_name();
        let taken = |path: &str| {
            self.workspace.desk.sessions.values().any(|record| {
                same_folder(record.folder.path(), path)
                    && self
                        .workspace
                        .session(&record.session)
                        .is_some_and(|session| Some(&session.machine) == form.machine.as_ref())
            })
        };
        let root = form.machine.as_ref().map_or(desk::DEFAULT_ROOT, |picked| {
            desk::root_of(&self.workspace.machines, picked)
        });
        let mut path = desk::scratch_path_in(root, &name);
        let mut suffix = 2;
        while taken(&path) {
            path = desk::scratch_path_in(root, &format!("{name}-{suffix}"));
            suffix += 1;
        }
        Some(path)
    }
}

/// Whether a recorded folder is the one `preview` names. The preview is
/// written under the short root (`~/Humata`); a live daemon reports the full
/// path under the machine's root, so only the part after the root is
/// compared then.
fn same_folder(recorded: &str, preview: &str) -> bool {
    if recorded == preview {
        return true;
    }
    preview
        .strip_prefix(desk::DEFAULT_ROOT)
        .is_some_and(|rest| recorded.ends_with(rest))
}

impl Desk {
    pub(super) fn start_new(state: &mut DeskState, agent: Agent) {
        if state.sheet_open() {
            return;
        }
        let machine = state
            .workspace
            .machines
            .iter()
            .find(|machine| machine.online)
            .map(|machine| machine.id.clone());
        state.menu = None;
        state.new = Some(NewForm {
            title: String::new(),
            agent,
            machine,
            repos: Vec::new(),
            access: Access::default(),
            machines_open: false,
            adding_repo: false,
            starting: false,
            error: None,
        });
    }

    pub(super) fn reduce_new(&self, state: &mut DeskState, action: NewAction) -> Effect<DeskAction> {
        let online: Vec<MachineId> = state
            .workspace
            .machines
            .iter()
            .filter(|machine| machine.online)
            .map(|machine| machine.id.clone())
            .collect();
        let Some(form) = state.new.as_mut() else {
            return Effect::none();
        };
        if form.starting {
            return Effect::none();
        }
        match action {
            NewAction::Title(title) => form.title = title,
            NewAction::Agent(agent) => form.agent = agent,
            NewAction::Machine(machine) => {
                if online.contains(&machine) {
                    form.machine = Some(machine);
                }
                form.machines_open = false;
            }
            NewAction::Machines(open) => form.machines_open = open,
            NewAction::AddingRepo(adding) => form.adding_repo = adding,
            NewAction::AddRepo(repo) => {
                let repo = repo.trim();
                if !repo.is_empty() && !form.repos.iter().any(|have| have == repo) {
                    form.repos.push(repo.to_owned());
                }
                form.adding_repo = false;
            }
            NewAction::RemoveRepo(repo) => form.repos.retain(|have| *have != repo),
            NewAction::ToggleRepo(repo) => {
                let repo = repo.trim();
                if let Some(at) = form.repos.iter().position(|have| have == repo) {
                    form.repos.remove(at);
                } else if !repo.is_empty() {
                    form.repos.push(repo.to_owned());
                }
            }
            NewAction::Access(access) => form.access = access,
            NewAction::Start => return self.start_session(form),
        }
        Effect::none()
    }

    fn start_session(&self, form: &mut NewForm) -> Effect<DeskAction> {
        let Some(machine) = form.machine.clone() else {
            form.error = Some(
                "There is no online machine to run this on. Pick an online machine, or add one on the Hosts page."
                    .to_owned(),
            );
            return Effect::none();
        };
        let folder = FolderChoice::Scratch {
            name: form.scratch_name(),
        };
        let title = form.start_title();
        form.starting = true;
        form.error = None;
        let request = NewDeskSession {
            title,
            agent: form.agent,
            machine,
            folder,
            repos: form.repos.clone(),
            access: form.access,
        };
        let client = self.client.clone();
        Effect::future(async move { DeskAction::Started(client.start(request).await) })
    }
}

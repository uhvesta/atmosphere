//! New desk session (DK-14, frame 40).

use humata2_clients::desk::{self, Access, FolderChoice, NewDeskSession};
use humata2_clients::model::{Agent, MachineId};
use tca::Effect;

use crate::{Desk, DeskAction, DeskState};

/// Which folder the new session gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FolderPick {
    /// A new scratch folder.
    #[default]
    Scratch,
    /// A folder on the machine, picked by its path.
    Picked,
}

/// The New desk session form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewForm {
    /// The title; blank gets one from the agent.
    pub title: String,
    /// What it runs; [`Agent::Shell`] is "Terminal".
    pub agent: Agent,
    /// The machine it runs on.
    pub machine: Option<MachineId>,
    /// Which kind of folder.
    pub folder: FolderPick,
    /// The picked folder's path.
    pub path: String,
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

    /// Where the scratch folder goes when its name is free.
    #[must_use]
    pub fn scratch_path(&self) -> String {
        desk::scratch_path(&self.scratch_name())
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
    /// A folder kind was picked.
    Folder(FolderPick),
    /// The picked folder's path was typed.
    Path(String),
    /// A repository was added.
    AddRepo(String),
    /// A repository was removed.
    RemoveRepo(String),
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
                record.folder.path() == path
                    && self
                        .workspace
                        .session(&record.session)
                        .is_some_and(|session| Some(&session.machine) == form.machine.as_ref())
            })
        };
        let mut path = desk::scratch_path(&name);
        let mut suffix = 2;
        while taken(&path) {
            path = desk::scratch_path(&format!("{name}-{suffix}"));
            suffix += 1;
        }
        Some(path)
    }
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
            folder: FolderPick::Scratch,
            path: String::new(),
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
            NewAction::Folder(folder) => form.folder = folder,
            NewAction::Path(path) => form.path = path,
            NewAction::AddRepo(repo) => {
                let repo = repo.trim();
                if !repo.is_empty() && !form.repos.iter().any(|have| have == repo) {
                    form.repos.push(repo.to_owned());
                }
                form.adding_repo = false;
            }
            NewAction::RemoveRepo(repo) => form.repos.retain(|have| *have != repo),
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
        let folder = match form.folder {
            FolderPick::Scratch => FolderChoice::Scratch {
                name: form.scratch_name(),
            },
            FolderPick::Picked => {
                let path = form.path.trim();
                if path.is_empty() {
                    form.error = Some(
                        "Type the path of the folder on this machine, or choose a new scratch folder."
                            .to_owned(),
                    );
                    return Effect::none();
                }
                FolderChoice::Picked {
                    path: path.to_owned(),
                }
            }
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

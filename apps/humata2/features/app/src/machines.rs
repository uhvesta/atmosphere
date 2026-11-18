//! The Add machine dialog and a machine's page inside the app (HO-1, HO-2,
//! HO-4, MP-12 to MP-17, SK-2, SK-5): each feature's reducer runs on its part
//! of the state, and what it delegates becomes the app's own actions.

use humata2_clients::model::Agent;
use humata2_clients::{FileRoot, Folder};
use humata2_files::FilesAction;
use humata2_inbox::InboxAction;
use humata2_hosts::add::{AddMachine, AddMachineAction, Delegate as AddDelegate};
use humata2_hosts::page::{Delegate as PageDelegate, MachinePage, MachinePageAction};
use humata2_hosts::HostsAction;
use humata2_settings::{Section, SettingsAction};
use tca::{Effect, Reducer};

use crate::{App, AppAction, AppState, NewSessionAction, Route};

impl App {
    /// Runs the Add machine dialog's reducer, then does what it asked for:
    /// an added machine is selected on Hosts, and a notice becomes a toast.
    pub(super) fn add_machine(
        &self,
        state: &mut AppState,
        action: AddMachineAction,
    ) -> Effect<AppAction> {
        let delegate = match &action {
            AddMachineAction::Delegate(delegate) => Some(delegate.clone()),
            _ => None,
        };
        let effect = AddMachine::new(self.clients.machines.clone())
            .reduce(&mut state.add_machine, action)
            .map(AppAction::AddMachine);
        let Some(delegate) = delegate else {
            return effect;
        };
        let clock = self.clients.clock.clone();
        let asked = match delegate {
            AddDelegate::Added(machine) => {
                state.hosts.selected = Some(machine);
                self.switch(state, Route::Hosts)
            }
            AddDelegate::Toast(text) => state.toast(text, &clock),
            // The Inbox's own Migrate click, on the item the dialog names.
            AddDelegate::Migrate(request) => self.child(state, AppAction::Inbox(InboxAction::Migrate(request))),
            // Providers and their host variables live in Settings › Machines.
            AddDelegate::ManageProviders => {
                let go = self.navigate(state, Route::Settings);
                go.and(self.child(state, AppAction::Settings(SettingsAction::Section(Section::Machines))))
            }
        };
        effect.and(asked)
    }

    /// Runs a machine page's reducer, then does what it asked for.
    pub(super) fn machine_page(
        &self,
        state: &mut AppState,
        action: MachinePageAction,
    ) -> Effect<AppAction> {
        let delegate = match &action {
            MachinePageAction::Delegate(delegate) => Some(delegate.clone()),
            _ => None,
        };
        let effect = MachinePage::new(self.clients.machines.clone())
            .with_daemon(self.clients.daemon.clone())
            .reduce(&mut state.machine_page, action)
            .map(AppAction::MachinePage);
        let Some(delegate) = delegate else {
            return effect;
        };
        let clock = self.clients.clock.clone();
        let asked = match delegate {
            PageDelegate::Reconnect(machine) => self.child(
                state,
                AppAction::Hosts(HostsAction::Connect(machine)),
            ),
            PageDelegate::OpenShell(machine) => {
                let start = self.handle(state, AppAction::StartNewSession);
                let host = self.handle(
                    state,
                    AppAction::NewSession(NewSessionAction::Machine(machine)),
                );
                let agent = self.handle(
                    state,
                    AppAction::NewSession(NewSessionAction::Agent(Agent::Shell)),
                );
                Effect::merge([start, host, agent])
            }
            PageDelegate::NewSession(machine) => {
                let start = self.handle(state, AppAction::StartNewSession);
                let host = self.handle(
                    state,
                    AppAction::NewSession(NewSessionAction::Machine(machine)),
                );
                Effect::merge([start, host])
            }
            PageDelegate::OpenSession(session) => self.navigate(state, Route::Session(session)),
            PageDelegate::OpenFolder { machine, channel } => {
                let go = self.navigate(state, Route::Files);
                let side = state.files.active;
                let folder = Folder::root(machine, FileRoot::ChannelFolder(channel));
                let show = self.child(
                    state,
                    AppAction::Files(FilesAction::Navigate(side, folder)),
                );
                Effect::merge([go, show])
            }
            PageDelegate::ChangeAgentAccess(_) => {
                let go = self.navigate(state, Route::Settings);
                let section = self.child(
                    state,
                    AppAction::Settings(SettingsAction::Section(Section::Access)),
                );
                Effect::merge([go, section])
            }
            PageDelegate::OpenInInbox(request) => {
                let go = self.navigate(state, Route::Inbox);
                let select = self.child(state, AppAction::Inbox(InboxAction::Select(request)));
                Effect::merge([go, select])
            }
            PageDelegate::Toast(text) => state.toast(text, &clock),
            PageDelegate::Running(humata2_hosts::RunningDelegate::Open { session: Some(session), .. }) => {
                self.navigate(state, Route::Session(session))
            }
            // SEAM (D784): attaching a terminal no session names by its tab
            // needs a client call the Daemon trait doesn't have yet.
            PageDelegate::Running(humata2_hosts::RunningDelegate::Open { session: None, .. }) => Effect::none(),
            PageDelegate::Running(humata2_hosts::RunningDelegate::ArchiveSession(session)) => {
                // The session page's own Archive, frame 65's dialog included.
                let go = self.navigate(state, Route::Session(session));
                let archive = self.handle(state, AppAction::Session(humata2_session::SessionAction::Archive));
                Effect::merge([go, archive])
            }
        };
        effect.and(asked)
    }
}

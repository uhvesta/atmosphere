//! Adding a machine and a machine's page as a person does them, headless on
//! the fakes (SPEC HO-1, HO-2, HO-4, HO-5, MP-12 to MP-17, SK-5; frames 56 to
//! 59). The scripted `FakeMachines` stands in for the daemon; the real shell
//! is driven by clicks and keys found by accessible role and name.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::machines::{
    AddEvent, AddFailure, AddOutcome, AddRequest, AddTarget, Call, Choices, Fix, GhLogin,
    LoginEvent, LoginReason, LoginTarget, MachineDetail, MachineHealth, OperationId, PromptKind, Reach, ReachRule,
    ChannelFolder, Provider, ProviderParam, SkillsPreview, SshHost, StepKind, StepStatus, StepUpdate, UnattendedMethod,
};
use humata2_clients::model::{
    Agent, Capability, ChannelId, MachineId, MachineStop, RuleSetting, SessionId, StoppedBy,
};
use humata2_clients::{Clients, DaemonError, fixture};
use humata2_engine::Resources;
use humata2_hosts::add::Choice;
use humata2_hosts::page::MachinePageAction;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::TestClock;
use tokio::runtime::Runtime;

struct App {
    runtime: Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    fake: humata2_clients::machines::FakeMachines,
}

fn id(text: &str) -> MachineId {
    MachineId::new(text)
}

impl App {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
        let fake = fakes.machines.clone();
        let ui = Ui::new(runtime.handle(), clients, Recorder::new());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)),
            ))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let (window, _shell) = cx
            .update(|cx| humata2_ui::open(cx, &ui, options))
            .expect("the window opens");
        let mut app = Self { runtime, cx, window, ui, fake };
        app.settle();
        app
    }

    fn settle(&mut self) {
        for _ in 0..10 {
            self.runtime.block_on(async {
                for _ in 0..32 {
                    tokio::task::yield_now().await;
                }
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    fn play(&mut self, action: &Action) {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(action)
            .expect("the step can be played");
        self.settle();
    }

    fn click(&mut self, role: Role, label: &str) {
        self.play(&Action::Click(Target::new(role, label)));
    }

    fn type_text(&mut self, text: &str) {
        self.play(&Action::Type(text.to_owned()));
    }

    fn send(&mut self, action: AppAction) {
        self.cx.update(|cx| self.ui.send(action, cx));
        self.settle();
    }

    /// Whether the view shows a control with this role and accessible name.
    fn shows(&mut self, role: Role, label: &str) -> bool {
        let view = GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read");
        view.contains(&format!("{role:?} {label:?}"))
    }

    fn open_add(&mut self) {
        self.send(AppAction::Switch(Route::Hosts));
        self.click(Role::Button, "Add a machine\u{2026}");
        assert!(self.ui.read(|s| s.add_machine.open), "the dialog opens");
    }

    /// Picks a host or provider through the Host pop-up, as a person does.
    fn pick(&mut self, entry: &str) {
        self.click(Role::Button, "Host");
        self.click(Role::ListBoxOption, entry);
    }

    /// Whether the view's text mentions `text` anywhere.
    fn view_has(&mut self, text: &str) -> bool {
        let view = GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read");
        view.contains(text)
    }

    fn dialog_open(&self) -> bool {
        self.ui.read(|s| s.add_machine.open)
    }

    fn step_states(&self) -> Vec<(StepKind, StepStatus)> {
        self.ui.read(|s| s.add_machine.steps.iter().map(|step| (step.kind, step.status)).collect())
    }

    fn adds(&self) -> Vec<AddRequest> {
        self.fake
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Add(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    fn selected_host(&self) -> Option<MachineId> {
        self.ui.read(|s| s.hosts.selected.clone())
    }

    fn toasts(&self) -> usize {
        self.ui.read(|s| s.toasts.len())
    }
}

const ALL: [StepKind; 5] = [
    StepKind::Connect,
    StepKind::Check,
    StepKind::Install,
    StepKind::Start,
    StepKind::FindAgents,
];

fn started(operation: &str) -> AddEvent {
    AddEvent::Started { operation: OperationId::new(operation), steps: ALL.to_vec() }
}

fn step(kind: StepKind, status: StepStatus) -> AddEvent {
    AddEvent::Step(StepUpdate { step: kind, status, detail: String::new() })
}

fn added(machine: &str) -> AddEvent {
    AddEvent::Outcome(AddOutcome::Added(id(machine)))
}

fn ssh(host: &str) -> AddRequest {
    AddRequest { target: AddTarget::Ssh { host: host.to_owned() }, name: None, trusted_fingerprint: None }
}

fn one_host(name: &str) -> Choices {
    Choices {
        hosts: vec![SshHost { name: name.to_owned(), address: format!("avesta@{name}") }],
        providers: Vec::new(),
    }
}

fn install_failure() -> AddEvent {
    AddEvent::Outcome(AddOutcome::Failed(AddFailure {
        step: StepKind::Install,
        title: "Install humatad failed on devbox-2".to_owned(),
        what: "Humata can't write to ~/.local/bin on devbox-2.".to_owned(),
        fix: Some(Fix {
            command: "sudo chown -R $USER ~/.local/bin".to_owned(),
            where_to_run: "devbox-2".to_owned(),
        }),
    }))
}

// ---- Add machine -----------------------------------------------------------------

#[test]
fn an_add_over_ssh_fails_at_install_and_try_again_retries_the_same_add() {
    let mut app = App::new();
    app.fake.set_choices(one_host("devbox-2"));
    app.fake.script_add(vec![
        started("op-1"),
        step(StepKind::Connect, StepStatus::Done),
        step(StepKind::Check, StepStatus::Done),
        step(StepKind::Install, StepStatus::Running),
        install_failure(),
    ]);
    app.fake.script_add(vec![
        step(StepKind::Install, StepStatus::Done),
        step(StepKind::Start, StepStatus::Done),
        step(StepKind::FindAgents, StepStatus::Done),
        added("devbox-2"),
    ]);
    app.open_add();
    app.pick("devbox-2");
    assert!(!app.shows(Role::ListBox, "Hosts and providers"), "a pick closes the pop-up");
    app.click(Role::Button, "Add");

    assert_eq!(app.adds(), vec![ssh("devbox-2")]);
    assert_eq!(
        app.step_states(),
        vec![
            (StepKind::Connect, StepStatus::Done),
            (StepKind::Check, StepStatus::Done),
            (StepKind::Install, StepStatus::Failed),
            (StepKind::Start, StepStatus::NotRun),
            (StepKind::FindAgents, StepStatus::NotRun),
        ],
    );
    assert!(app.dialog_open(), "a failure keeps the dialog open");
    assert!(app.shows(Role::Alert, "Install humatad failed on devbox-2"));
    assert!(app.shows(Role::Code, "sudo chown -R $USER ~/.local/bin"));
    assert!(app.shows(Role::Button, "Try again"));

    app.click(Role::Button, "Try again");
    assert!(
        app.fake.calls().contains(&Call::Retry(OperationId::new("op-1"))),
        "Try again asks to retry the operation that failed, not to add again",
    );
    assert_eq!(app.adds().len(), 1);
    assert!(!app.dialog_open(), "the finished add closes the dialog");
    assert_eq!(app.selected_host(), Some(id("devbox-2")));
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Hosts);
    assert_eq!(app.toasts(), 1, "a toast says the machine was added");
}

#[test]
fn a_host_can_be_typed_instead_of_picked() {
    let mut app = App::new();
    app.fake.script_add(vec![started("op-1"), added("gpu-9")]);
    app.open_add();
    app.type_text("gpu-9");
    app.click(Role::Button, "Add");
    assert_eq!(app.adds(), vec![ssh("gpu-9")]);
    assert_eq!(app.selected_host(), Some(id("gpu-9")));
}

#[test]
fn a_typed_host_wins_over_the_popups_pick() {
    let mut app = App::new();
    app.fake.set_choices(one_host("devbox-2"));
    app.fake.script_add(vec![started("op-1"), added("gpu-9")]);
    app.open_add();
    app.type_text("gpu-9");
    app.click(Role::Button, "Add");
    assert_eq!(app.adds(), vec![ssh("gpu-9")]);
}

#[test]
fn the_popup_lists_the_hosts_then_the_providers_and_a_provider_adds_with_its_params() {
    let mut app = App::new();
    app.fake.set_choices(Choices {
        hosts: vec![SshHost { name: "devbox-2".to_owned(), address: "avesta@devbox-2".to_owned() }],
        providers: vec![corp_ssh()],
    });
    app.fake.script_add(vec![started("op-1"), added("corp-vm")]);
    app.open_add();
    app.click(Role::Button, "Host");
    assert!(app.shows(Role::ListBoxOption, "devbox-2"));
    assert!(app.shows(Role::ListBoxOption, "Corporate SSH"));
    app.click(Role::ListBoxOption, "Corporate SSH");
    app.click(Role::Button, "Add");
    let adds = app.adds();
    assert_eq!(adds.len(), 1);
    assert!(matches!(&adds[0].target, AddTarget::Provider { provider, .. } if provider == "corp-ssh"));
}

#[test]
fn manage_providers_ends_the_popup_and_opens_settings_machines() {
    let mut app = App::new();
    app.fake.set_choices(Choices {
        hosts: vec![SshHost { name: "devbox-2".to_owned(), address: "avesta@devbox-2".to_owned() }],
        providers: vec![corp_ssh()],
    });
    app.open_add();
    app.click(Role::Button, "Host");
    assert!(app.shows(Role::ListBoxOption, "Manage providers\u{2026}"));
    app.click(Role::ListBoxOption, "Manage providers\u{2026}");
    assert!(!app.dialog_open(), "the dialog closes");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Settings);
    assert_eq!(app.ui.read(|s| s.settings.section), humata2_settings::Section::Machines);
    assert!(app.adds().is_empty());
}

#[test]
fn a_provider_with_an_unset_variable_says_so_and_can_not_add() {
    let mut app = App::new();
    app.fake.set_choices(Choices {
        hosts: Vec::new(),
        providers: vec![Provider { unset: vec!["HCLOUD_TOKEN".to_owned()], ..corp_ssh() }],
    });
    app.open_add();
    app.pick("Corporate SSH");
    let needs = "Corporate SSH needs HCLOUD_TOKEN. Set it in Settings > Machines.";
    assert!(app.shows(Role::Alert, needs));
    app.click(Role::Button, "Add");
    app.play(&Action::Key("enter".to_owned()));
    assert!(app.adds().is_empty(), "Add stays off while the variable is unset");

    app.click(Role::Button, "Open Settings");
    assert!(!app.dialog_open());
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Settings);
    assert_eq!(app.ui.read(|s| s.settings.section), humata2_settings::Section::Machines);
}

/// Whether the pop-up's pick is still the provider `id`.
fn picked_provider(app: &App, id: &str) -> bool {
    app.ui.read(|s| {
        s.add_machine.typed.is_empty()
            && matches!(&s.add_machine.choice, Some(Choice::Provider { id: picked, .. }) if picked == id)
    })
}

#[test]
fn picking_a_provider_without_params_puts_the_keyboard_in_its_name() {
    let mut app = App::new();
    app.fake.set_choices(Choices {
        hosts: vec![SshHost { name: "devbox-2".to_owned(), address: "avesta@devbox-2".to_owned() }],
        providers: vec![Provider {
            id: "orbstack".to_owned(),
            label: "OrbStack".to_owned(),
            interactive: false,
            params: Vec::new(),
            unset: Vec::new(),
        }],
    });
    app.fake.script_add(vec![started("op-1"), added("box-1")]);
    app.open_add();
    app.pick("OrbStack");
    app.type_text("box-1");
    assert_eq!(app.ui.read(|s| s.add_machine.name.clone()), "box-1");
    assert!(picked_provider(&app, "orbstack"), "typing the name keeps the provider");

    app.play(&Action::Key("enter".to_owned()));
    let adds = app.adds();
    assert_eq!(adds.len(), 1, "Enter in the name starts the add");
    assert!(matches!(&adds[0].target, AddTarget::Provider { provider, .. } if provider == "orbstack"));
    assert_eq!(adds[0].name.as_deref(), Some("box-1"));
}

#[test]
fn a_providers_params_and_name_each_take_the_keys_when_clicked() {
    let mut app = App::new();
    app.fake.set_choices(Choices {
        hosts: Vec::new(),
        providers: vec![Provider {
            id: "cloud".to_owned(),
            label: "Cloud".to_owned(),
            interactive: false,
            params: vec![ProviderParam {
                name: "zone".to_owned(),
                label: "Zone".to_owned(),
                default: String::new(),
                required: true,
            }],
            unset: Vec::new(),
        }],
    });
    app.fake.script_add(vec![started("op-1"), added("box-2")]);
    app.open_add();
    app.pick("Cloud");
    // The pick moves the keyboard into the first param.
    app.type_text("eu-1");
    app.click(Role::TextInput, "Machine name");
    app.type_text("box-2");
    app.click(Role::TextInput, "Zone");
    app.type_text("a");
    assert_eq!(app.ui.read(|s| s.add_machine.name.clone()), "box-2");
    assert!(picked_provider(&app, "cloud"), "typing in the fields keeps the provider");
    let zone = app.ui.read(|s| match &s.add_machine.choice {
        Some(Choice::Provider { params, .. }) => params.get("zone").cloned(),
        _ => None,
    });
    assert!(
        zone.as_deref().is_some_and(|zone| zone.replacen('a', "", 1) == "eu-1"),
        "both typings reached the zone: {zone:?}"
    );

    app.play(&Action::Key("enter".to_owned()));
    let adds = app.adds();
    assert_eq!(adds.len(), 1, "Enter in a param starts the add");
    assert!(matches!(
        &adds[0].target,
        AddTarget::Provider { provider, params } if provider == "cloud" && params.get("zone") == zone.as_ref()
    ));
    assert_eq!(adds[0].name.as_deref(), Some("box-2"));
}

#[test]
fn confirming_a_new_host_key_adds_trusting_it() {
    let mut app = App::new();
    app.fake.set_choices(one_host("devbox-2"));
    app.fake.script_add(vec![
        started("op-1"),
        AddEvent::Outcome(AddOutcome::HostKey { fingerprint: "SHA256:abc123".to_owned() }),
    ]);
    app.fake.script_add(vec![started("op-1"), added("devbox-2")]);
    app.open_add();
    app.click(Role::Button, "Add");
    assert_eq!(app.ui.read(|s| s.add_machine.host_key.clone()), Some("SHA256:abc123".to_owned()));
    assert!(app.shows(Role::Group, "Host key"));

    app.click(Role::Button, "Confirm");
    let adds = app.adds();
    assert_eq!(adds.len(), 2);
    assert_eq!(adds[1].trusted_fingerprint, Some("SHA256:abc123".to_owned()));
    assert!(!app.dialog_open());
    assert_eq!(app.selected_host(), Some(id("devbox-2")));
}

#[test]
fn declining_a_new_host_key_adds_nothing() {
    let mut app = App::new();
    app.fake.set_choices(one_host("devbox-2"));
    app.fake.script_add(vec![
        started("op-1"),
        AddEvent::Outcome(AddOutcome::HostKey { fingerprint: "SHA256:abc123".to_owned() }),
    ]);
    app.open_add();
    let before = app.selected_host();
    app.click(Role::Button, "Add");
    app.click(Role::Button, "Don't add");
    assert_eq!(app.adds().len(), 1, "no second add is started");
    assert_eq!(app.ui.read(|s| s.add_machine.host_key.clone()), None);
    assert_eq!(app.selected_host(), before);
    assert_eq!(app.toasts(), 0);
}

fn corp_ssh() -> Provider {
    Provider {
        id: "corp-ssh".to_owned(),
        label: "Corporate SSH".to_owned(),
        interactive: true,
        params: Vec::new(),
        unset: Vec::new(),
    }
}

fn ready(can_unattended: bool) -> LoginEvent {
    LoginEvent::Ready {
        machine: id("corp-vm"),
        can_make_unattended: can_unattended,
        daemon_version: Some("2.0.4".to_owned()),
    }
}

/// Why the scripted adds need a login: the connect printed a code prompt.
fn code_prompt() -> LoginReason {
    LoginReason::Prompted { kind: PromptKind::Code, line: "Verification code:".to_owned() }
}

/// An add through the `corp-ssh` provider that asks for a login.
fn login_script(app: &App, can_unattended: bool) {
    app.fake.set_choices(Choices { hosts: Vec::new(), providers: vec![corp_ssh()] });
    app.fake.script_add(vec![
        started("op-9"),
        step(StepKind::Connect, StepStatus::Running),
        AddEvent::Outcome(AddOutcome::NeedsLogin { reason: code_prompt() }),
    ]);
    app.fake.script_login(
        vec![LoginEvent::Output(b"Verification code: ".to_vec())],
        vec![step_login(StepKind::Connect, StepStatus::Done), ready(can_unattended)],
    );
}

fn step_login(kind: StepKind, status: StepStatus) -> LoginEvent {
    LoginEvent::Step(StepUpdate { step: kind, status, detail: String::new() })
}

/// Runs the add to the login and opens it.
fn open_login(app: &mut App) {
    app.open_add();
    app.pick("Corporate SSH");
    app.click(Role::Button, "Add");
    assert!(app.shows(Role::Button, "Open interactively"));
    app.click(Role::Button, "Open interactively");
}

#[test]
fn a_login_shows_the_terminal_takes_keys_and_leads_to_the_unattended_offer() {
    let mut app = App::new();
    login_script(&app, true);
    open_login(&mut app);
    let target = LoginTarget::Operation(OperationId::new("op-9"));
    assert!(app.fake.calls().contains(&Call::OpenLogin(target.clone())));
    let printed = app.ui.read(|s| s.add_machine.login.as_ref().map(|login| login.bytes.clone()));
    assert_eq!(printed, Some(b"Verification code: ".to_vec()), "what the terminal printed is kept");
    assert!(app.shows(Role::Log, "Login terminal"));

    app.type_text("123456");
    let typed: Vec<u8> = app
        .fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::LoginInput(on, bytes) if on == target => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(typed, b"123456", "keys typed in the terminal reach the client");

    app.click(Role::Button, "Done, I'm logged in");
    assert!(app.fake.calls().contains(&Call::LoginDone(target)));
    assert!(app.ui.read(|s| s.add_machine.unattended.is_some()), "the unattended offer follows");
    assert!(app.shows(Role::RadioButton, "Add a Humata key to authorized_keys"));
    assert!(app.shows(Role::ListItem, "Log in: done"), "the login is its own step");
    assert!(
        !app.step_states().iter().any(|(kind, _)| *kind == StepKind::Connect),
        "the login takes the place of the connect attempt",
    );

    app.click(Role::RadioButton, "Keep corp-ssh's cached token");
    app.click(Role::Button, "Make it unattended");
    assert!(
        app.fake.calls().contains(&Call::MakeUnattended(id("corp-vm"), UnattendedMethod::KeepToken)),
        "the chosen method goes to the client",
    );
    assert!(!app.dialog_open());
    assert_eq!(app.selected_host(), Some(id("corp-vm")));
}

#[test]
fn a_plain_ssh_login_offers_no_cached_token() {
    let mut app = App::new();
    app.fake.set_choices(one_host("corp-vm"));
    app.fake.script_add(vec![
        started("op-9"),
        AddEvent::Outcome(AddOutcome::NeedsLogin { reason: code_prompt() }),
    ]);
    app.fake.script_login(vec![], vec![ready(true)]);
    app.open_add();
    app.click(Role::Button, "Add");
    app.click(Role::Button, "Open interactively");
    app.click(Role::Button, "Done, I'm logged in");
    assert!(app.shows(Role::RadioButton, "Add a Humata key to authorized_keys"));
    assert!(app.shows(Role::RadioButton, "Keep logging in by hand"));
    assert!(!app.view_has("cached token"), "only a provider has a token to keep");
}

#[test]
fn keeping_the_login_by_hand_adds_the_machine_without_making_it_unattended() {
    let mut app = App::new();
    login_script(&app, true);
    open_login(&mut app);
    app.click(Role::Button, "Done, I'm logged in");
    app.click(Role::RadioButton, "Keep logging in by hand");
    assert!(
        !app.fake.calls().iter().any(|call| matches!(call, Call::MakeUnattended(..))),
        "skipping asks the client for nothing",
    );
    assert!(!app.dialog_open());
    assert_eq!(app.selected_host(), Some(id("corp-vm")));
}

#[test]
fn a_login_that_cannot_be_made_unattended_closes_when_done() {
    let mut app = App::new();
    login_script(&app, false);
    open_login(&mut app);
    app.click(Role::Button, "Done, I'm logged in");
    assert!(!app.dialog_open());
    assert_eq!(app.selected_host(), Some(id("corp-vm")));
}

#[test]
fn escape_and_cancel_close_the_dialog() {
    let mut app = App::new();
    app.open_add();
    app.send(AppAction::Escape);
    assert!(!app.dialog_open(), "Escape closes it");

    app.open_add();
    app.click(Role::Button, "Cancel");
    assert!(!app.dialog_open(), "Cancel closes it");
    assert!(
        !app.fake.calls().iter().any(|call| matches!(call, Call::Cancel(_))),
        "nothing was running, so nothing is cancelled",
    );
}

#[test]
fn cancel_while_an_add_runs_cancels_the_operation() {
    let mut app = App::new();
    app.fake.set_choices(one_host("devbox-2"));
    // The add stays open, as a running one does, until Cancel reaches it.
    app.fake.script_add_held(vec![started("op-1"), step(StepKind::Connect, StepStatus::Running)]);
    let before = app.selected_host();
    app.open_add();
    app.click(Role::Button, "Add");
    assert!(app.ui.read(|s| s.add_machine.running), "the add is still running");
    assert!(!app.shows(Role::Button, "Add"), "a running add can't be started again");

    app.click(Role::Button, "Cancel");
    assert!(app.fake.calls().contains(&Call::Cancel(OperationId::new("op-1"))));
    assert!(!app.dialog_open());
    assert_eq!(app.adds().len(), 1, "no second add is made");
    assert_eq!(app.selected_host(), before, "nothing is added");
    assert_eq!(app.toasts(), 0);
}

// ---- The machine page -------------------------------------------------------------

fn rule(capability: &str, label: &str, setting: RuleSetting) -> ReachRule {
    ReachRule { capability: Capability::new(capability), label: label.to_owned(), setting }
}

/// What frame 59 shows for devbox-1.
fn devbox_detail() -> MachineDetail {
    MachineDetail {
        machine: id("devbox-1"),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: "devbox-1.lan".to_owned(),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".into(), "read:org".into()] }),
            agents: vec![(Agent::Claude, "2.4".to_owned()), (Agent::Codex, "0.98".to_owned())],
            disk_free: 11_000_000_000,
            disk_total: 500_000_000_000,
            variables: vec!["DATABASE_URL".to_owned(), "STRIPE_KEY".to_owned()],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![
            ChannelFolder { channel: ChannelId::new("payments-rewrite"), path: "~/Humata/payments-rewrite/".into(), sessions: 4, bytes: 3_100_000_000 },
            ChannelFolder { channel: ChannelId::new("infra"), path: "~/Humata/infra/".into(), sessions: 2, bytes: 800_000_000 },
            ChannelFolder { channel: ChannelId::new("pr-reviews"), path: "~/Humata/pr-reviews/".into(), sessions: 0, bytes: 200_000_000 },
        ],
        sessions: ["ledger", "brin", "rate", "partial-brin", "benchmark", "postgres"]
            .map(SessionId::new)
            .to_vec(),
        rules: vec![
            rule("files_read_other_machine", "Read files on other machines", RuleSetting::Allow),
            rule("git_push", "Push its branch or open a pull request", RuleSetting::Ask),
        ],
        stop: None,
    }
}

fn open_devbox(app: &mut App) {
    app.fake.set_detail(Ok(devbox_detail()));
    app.click(Role::Link, "devbox-1");
}

#[test]
fn clicking_a_machine_in_the_sidebar_opens_its_page_with_the_detail() {
    let mut app = App::new();
    open_devbox(&mut app);
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Machine(id("devbox-1")));
    assert!(app.fake.calls().contains(&Call::Detail(id("devbox-1"))));
    assert_eq!(app.ui.read(|s| s.machine_page.detail.clone()), Some(devbox_detail()));
    assert!(app.shows(Role::Heading, "devbox-1"));
    assert!(app.shows(Role::Button, "Ledger schema migration"));
    assert!(app.shows(Role::Button, "Upgrade Postgres 17"));
    assert!(app.shows(Role::ListItem, "# infra"));
    assert!(app.shows(Role::Group, "Health"));
}

#[test]
fn a_session_row_opens_that_session() {
    let mut app = App::new();
    open_devbox(&mut app);
    app.click(Role::Button, "Try BRIN index");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Session(SessionId::new("brin")));
}

#[test]
fn new_session_on_the_machine_opens_the_sheet_for_it() {
    let mut app = App::new();
    open_devbox(&mut app);
    app.click(Role::Button, "New session on devbox-1");
    assert!(app.ui.read(|s| matches!(
        s.overlay(),
        Some(Overlay::NewSession(sheet)) if sheet.machine == Some(id("devbox-1"))
    )));
}

#[test]
fn an_offline_machine_keeps_its_last_detail_and_reconnects() {
    let mut app = App::new();
    let mut detail = devbox_detail();
    detail.machine = id("gpu-box");
    app.fake.set_detail(Ok(detail.clone()));
    app.click(Role::Link, "gpu-box");
    assert_eq!(app.ui.read(|s| s.machine_page.detail.clone()), Some(detail.clone()));
    assert!(app.shows(Role::Status, "The link is down"));

    // The daemon can't be reached for a reload: the last detail stays.
    app.fake.fail("detail", DaemonError::Offline { machine: id("gpu-box") });
    app.send(AppAction::MachinePage(MachinePageAction::Reload));
    assert_eq!(app.ui.read(|s| s.machine_page.detail.clone()), Some(detail));
    assert!(app.ui.read(|s| s.machine_page.load_error.is_some()));

    // Offline, "Reconnect now" is the link-down row's item alone (HO-5), in the "…" menu.
    app.click(Role::Button, "More");
    app.click(Role::Button, "Reconnect now");
    assert!(app.ui.read(|s| s.workspace.machine(&id("gpu-box")).is_some_and(|m| m.online)));
}

#[test]
fn sync_skills_shows_the_preview_and_needs_a_confirm() {
    let mut app = App::new();
    app.fake.set_preview(Ok(SkillsPreview {
        first_sync: true,
        added: vec!["review".to_owned()],
        ..SkillsPreview::default()
    }));
    open_devbox(&mut app);
    app.click(Role::Button, "More");
    app.click(Role::Button, "Sync skills");
    assert!(app.fake.calls().contains(&Call::SkillsPreview(id("devbox-1"))));
    assert!(app.shows(Role::Dialog, "Sync skills"));
    assert!(
        !app.fake.calls().contains(&Call::SyncSkills(id("devbox-1"))),
        "the preview alone syncs nothing",
    );

    app.click(Role::Button, "Sync");
    assert!(app.fake.calls().contains(&Call::SyncSkills(id("devbox-1"))));
}

#[test]
fn cancelling_the_skills_preview_syncs_nothing() {
    let mut app = App::new();
    open_devbox(&mut app);
    app.click(Role::Button, "More");
    app.click(Role::Button, "Sync skills");
    app.click(Role::Button, "Cancel");
    assert!(!app.shows(Role::Dialog, "Sync skills"));
    assert!(!app.fake.calls().iter().any(|call| matches!(call, Call::SyncSkills(_))));
}

// ---- Stop all sessions ------------------------------------------------------------

fn stopped_at(delivered: bool) -> MachineStop {
    MachineStop { stop_id: "stop-1".to_owned(), at: "14:02".to_owned(), by: StoppedBy::You, delivered }
}

fn stop_of_page(app: &App) -> Option<MachineStop> {
    app.ui.read(|s| s.machine_page.detail.as_ref().and_then(|detail| detail.stop.clone()))
}

#[test]
fn stop_all_sessions_asks_first_and_stops_only_when_confirmed() {
    let mut app = App::new();
    open_devbox(&mut app);
    app.click(Role::Button, "More");
    app.click(Role::Button, "Stop all sessions\u{2026}");
    assert!(app.shows(Role::Dialog, "Stop all sessions"));
    assert!(
        !app.fake.calls().contains(&Call::StopSessions(id("devbox-1"))),
        "opening the confirm stops nothing",
    );

    app.click(Role::Button, "Cancel");
    assert!(!app.shows(Role::Dialog, "Stop all sessions"));
    assert!(!app.fake.calls().contains(&Call::StopSessions(id("devbox-1"))));

    app.click(Role::Button, "More");
    app.click(Role::Button, "Stop all sessions\u{2026}");
    app.click(Role::Button, "Stop all sessions");
    assert!(app.fake.calls().contains(&Call::StopSessions(id("devbox-1"))));
    assert!(!app.shows(Role::Dialog, "Stop all sessions"));
    assert!(stop_of_page(&app).is_some_and(|stop| stop.delivered), "the page reloads to show the stop");
    assert!(app.shows(Role::Status, "Sessions stopped"));
    assert!(app.shows(Role::Button, "Start sessions"));

    // Stopped, the menu no longer offers to stop them again.
    app.click(Role::Button, "More");
    assert!(!app.shows(Role::Button, "Stop all sessions\u{2026}"));
}

#[test]
fn while_stopped_open_a_shell_is_refused_with_a_note() {
    let mut app = App::new();
    app.fake.set_stop(&id("devbox-1"), Some(stopped_at(true)));
    open_devbox(&mut app);
    assert!(app.shows(Role::Status, "Sessions stopped"));
    app.click(Role::Button, "Open a shell");
    assert!(app.shows(Role::Status, "Start sessions first"));
    assert!(app.ui.read(|s| s.overlay().is_none()), "no session sheet opens");
}

#[test]
fn start_sessions_asks_first_and_starts_only_when_confirmed() {
    let mut app = App::new();
    app.fake.set_stop(&id("devbox-1"), Some(stopped_at(true)));
    open_devbox(&mut app);
    app.click(Role::Button, "Start sessions");
    assert!(app.shows(Role::Dialog, "Start sessions"));
    assert!(!app.fake.calls().contains(&Call::StartSessions(id("devbox-1"))));

    app.send(AppAction::MachinePage(MachinePageAction::ConfirmSessions));
    assert!(app.fake.calls().contains(&Call::StartSessions(id("devbox-1"))));
    assert_eq!(stop_of_page(&app), None);
    assert!(!app.shows(Role::Status, "Sessions stopped"));
}

#[test]
fn a_stop_that_has_not_reached_the_machine_is_cancelled_without_a_confirm() {
    let mut app = App::new();
    app.fake.set_stop(&id("devbox-1"), Some(stopped_at(false)));
    open_devbox(&mut app);
    assert!(app.shows(Role::Status, "Stop not delivered"));
    assert!(app.shows(Role::Button, "Reconnect now"));

    app.click(Role::Button, "Cancel stop");
    assert!(app.fake.calls().contains(&Call::StartSessions(id("devbox-1"))));
    assert_eq!(stop_of_page(&app), None);
    assert!(!app.shows(Role::Status, "Stop not delivered"));
}

// ---- The Hosts page's card --------------------------------------------------------

/// devbox-1 as its daemon reports it. The versions differ from the ones the design
/// workspace carries for it ("gh 2.83", "claude 2.4", "codex 0.98"), so a card that
/// shows these can only have them from the machines client.
fn reported_devbox() -> MachineDetail {
    let mut detail = devbox_detail();
    detail.health.gh_version = "2.90.1".to_owned();
    detail.health.agents = vec![(Agent::Claude, "2.7.3".to_owned()), (Agent::Codex, "0.101.0".to_owned())];
    detail.health.disk_free = 64_000_000_000;
    detail.health.disk_total = 500_000_000_000;
    detail
}

#[test]
fn the_hosts_card_shows_the_selected_machines_tools_as_its_daemon_reports_them() {
    let mut app = App::new();
    app.fake.set_detail(Ok(reported_devbox()));
    app.send(AppAction::Switch(Route::Hosts));
    app.click(Role::ListItem, "devbox-1");
    assert_eq!(app.selected_host(), Some(id("devbox-1")));

    // Each chip is in the window by its words: tool, then major.minor (HO-4, frame 28).
    for chip in ["gh 2.90", "claude 2.7", "codex 0.101"] {
        assert!(app.view_has(&format!("{chip:?}")), "the card shows the chip {chip:?}");
    }
    // What the workspace said before the daemon answered is not shown beside it.
    for stale in ["gh 2.83", "claude 2.4", "codex 0.98"] {
        assert!(!app.view_has(&format!("{stale:?}")), "the card still shows {stale:?}");
    }
}

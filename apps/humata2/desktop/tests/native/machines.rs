//! Add machine and the machine page (frames 56 to 59): the real window on a
//! scripted `FakeMachines`, in the state each approved frame shows, recorded
//! and compared like the other native frames. Run by `native_test`; record
//! with `bazel run //apps/humata2/desktop:native_test_record`.
use super::*;
use humata2_clients::machines::{
    AddEvent, AddFailure, AddOutcome, ChannelFolder, Choices, Fix, GhLogin, LoginEvent, LoginReason,
    MachineDetail, MachineHealth, OperationId, PromptKind, Provider, Reach, ReachRule, SshHost,
    StepKind, StepStatus, StepUpdate,
};
use humata2_clients::model::{Agent, Capability, ChannelId, MachineId, RuleSetting};
use humata2_hosts::HostsAction as H;
use humata2_hosts::add::AddMachineAction as Add;

const ALL: [StepKind; 5] = [
    StepKind::Connect,
    StepKind::Check,
    StepKind::Install,
    StepKind::Start,
    StepKind::FindAgents,
];

fn step(kind: StepKind, status: StepStatus, detail: &str) -> AddEvent {
    AddEvent::Step(StepUpdate { step: kind, status, detail: detail.to_owned() })
}

fn started() -> AddEvent {
    AddEvent::Started { operation: OperationId::new("op-1"), steps: ALL.to_vec() }
}

fn host(name: &str) -> Choices {
    Choices {
        hosts: vec![SshHost { name: name.to_owned(), address: format!("avesta@{name}") }],
        providers: Vec::new(),
    }
}

/// The Hosts page under the dialog, as the frames draw it.
fn hosts_page(app: &mut App) {
    app.fold_quiet_channels();
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Hosts));
    app.send(AppAction::Hosts(H::Select(MachineId::new("devbox-1"))));
}

#[test]
fn add_machine_failed_step_reference_image() {
    let mut app = App::new(fixture::design());
    app.fakes.machines.set_choices(host("devbox-2"));
    app.fakes.machines.script_add(vec![
        started(),
        step(StepKind::Connect, StepStatus::Done, "devbox-2 answered as avesta@10.0.4.22, host key accepted"),
        step(StepKind::Check, StepStatus::Done, "Linux x86_64 \u{b7} git 2.47 \u{b7} gh 2.83 \u{b7} 38 GB free"),
        step(StepKind::Install, StepStatus::Failed, "Stopped here. Nothing was installed."),
        AddEvent::Outcome(AddOutcome::Failed(AddFailure {
            step: StepKind::Install,
            title: "Install humatad failed on devbox-2".to_owned(),
            what: "Humata can't write to ~/.local/bin on devbox-2: permission denied, because the folder belongs to root.".to_owned(),
            fix: Some(Fix {
                command: "sudo chown -R $USER ~/.local/bin".to_owned(),
                where_to_run: "devbox-2".to_owned(),
            }),
        })),
    ]);
    hosts_page(&mut app);
    app.send(AppAction::AddMachine(Add::Open));
    // The host is picked through the pop-up, as a person does.
    app.click("add-machine-host-popup");
    assert!(app.contains("add-machine-host-menu"));
    app.click("add-machine-host devbox-2");
    assert!(!app.contains("add-machine-host-menu"));
    app.click("add-machine-start");
    assert!(app.contains("add-machine-failure"));
    app.snapshot("56-add-machine");
}

fn login_step(kind: StepKind, detail: &str) -> LoginEvent {
    LoginEvent::Step(StepUpdate { step: kind, status: StepStatus::Done, detail: detail.to_owned() })
}

/// The add that needs a login, up to the login being open. It goes through
/// the `corp-ssh` provider, picked in the Host pop-up.
fn login_app() -> App {
    let mut app = App::new(fixture::design());
    app.fakes.machines.set_choices(Choices {
        hosts: Vec::new(),
        providers: vec![Provider {
            id: "corp-ssh".to_owned(),
            label: "Corporate SSH".to_owned(),
            interactive: true,
            params: Vec::new(),
        }],
    });
    app.fakes.machines.script_add(vec![
        started(),
        step(StepKind::Connect, StepStatus::Running, ""),
        // The daemon saw a code prompt; the sentence above the terminal and the Log in
        // step's detail (frames 57 and 58) are built from that.
        AddEvent::Outcome(AddOutcome::NeedsLogin {
            reason: LoginReason::Prompted { kind: PromptKind::Code, line: "Verification code:".to_owned() },
        }),
    ]);
    let screen = "$ corp-ssh corp-vm -- 'echo HUMATA_READY_7f3a'\r\n\
        Opening a browser\u{2026} or enter the code at\r\n\
        https://corp.example/device to sign in.\r\n\r\n\
        Verification code: ";
    // What the add reports once the person is logged in: the login itself is
    // the dialog's step, and the machine reports its version.
    app.fakes.machines.script_login(
        vec![LoginEvent::Output(screen.as_bytes().to_vec())],
        vec![
            login_step(StepKind::Check, "Linux x86_64 \u{b7} git 2.47 \u{b7} gh 2.83 \u{b7} 38 GB free"),
            login_step(StepKind::Install, "humatad 2.0.4, checksum ok"),
            login_step(StepKind::Start, "Linked through the terminal you logged in with"),
            login_step(StepKind::FindAgents, "claude, codex"),
            LoginEvent::Ready {
                machine: MachineId::new("corp-vm"),
                can_make_unattended: true,
                daemon_version: Some("2.0.4".to_owned()),
            },
        ],
    );
    hosts_page(&mut app);
    app.send(AppAction::AddMachine(Add::Open));
    app.click("add-machine-host-popup");
    app.click("add-machine-provider corp-ssh");
    app.send(AppAction::AddMachine(Add::SetName("corp-vm".to_owned())));
    app.click("add-machine-start");
    assert!(app.contains("add-machine-open-login"));
    app.click("add-machine-open-login");
    app
}

#[test]
fn add_machine_login_reference_image() {
    let mut app = login_app();
    assert!(app.contains("add-machine-terminal"));
    // Golden 57 is dark.
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("57-add-machine-login");
}

#[test]
fn add_machine_unattended_offer_reference_image() {
    let mut app = login_app();
    app.click("add-machine-login-done");
    assert!(app.contains("add-machine-make-unattended"));
    assert!(app.contains("add-machine-method-token"));
    app.snapshot("58-add-machine-unattended");
}

fn rule(capability: &str, label: &str, setting: RuleSetting) -> ReachRule {
    ReachRule { capability: Capability::new(capability), label: label.to_owned(), setting }
}

#[test]
fn machine_page_reference_image() {
    let mut app = machine_page_with(running_list);
    // The view text is compared before the image, so a changed list would
    // stop the run before an image is produced: write one out as well.
    if let Some(out) = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR") {
        app.draw();
        let folder = std::path::PathBuf::from(out).join("candidates");
        std::fs::create_dir_all(&folder).expect("candidate folder");
        let image = app.cx.capture_screenshot(app.window).expect("headless image");
        image.save(folder.join("59-machine-page-candidate.png")).expect("image written");
    }
    app.snapshot("59-machine-page");
}

/// The machine page with its "…" menu open.
#[test]
fn machine_menu_open_reference_image() {
    let mut app = machine_page();
    app.click("machine-menu");
    assert!(app.contains("machine-sync-skills"));
    app.snapshot("machine-menu-open");
}

/// D784: devbox-1's running list, with two terminals no channel has.
fn running_list(now: i64) -> Vec<humata2_clients::RunningHolder> {
    let holder = |session: Option<&str>, title: &str, agent: Agent, pid: Option<u32>, folder: &str, minutes: i64| {
        humata2_clients::RunningHolder {
            tab: pid.map(|pid| format!("tab-{pid}")),
            session: session.map(SessionId::new),
            title: title.to_owned(),
            channel: session.map(|_| ChannelId::new("payments-rewrite")),
            desk: false,
            agent,
            pid,
            folder: folder.to_owned(),
            started_unix_ms: pid.map(|_| now - minutes * 60_000),
            attached: false,
            machine: MachineId::new("devbox-1"),
            stopped: pid.is_none(),
            agent_tab: false,
        }
    };
    let mut ledger = holder(Some("ledger"), "Ledger schema migration", Agent::Claude, Some(40_117), "~/Humata/payments-rewrite/ledger", 26 * 60);
    ledger.attached = true;
    let mut benchmark = holder(Some("benchmark"), "Benchmark BRIN", Agent::Codex, Some(41_207), "~/Humata/infra/benchmark", 95);
    benchmark.channel = Some(ChannelId::new("infra"));
    let mut postgres = holder(Some("postgres"), "Upgrade Postgres 17", Agent::Claude, None, "~/Humata/infra/postgres", 0);
    postgres.channel = Some(ChannelId::new("infra"));
    vec![
        ledger,
        holder(Some("brin"), "Try BRIN index", Agent::Claude, Some(40_533), "~/Humata/payments-rewrite/brin", 190),
        holder(Some("rate"), "API rate limiter", Agent::Codex, Some(41_822), "~/Humata/payments-rewrite/api", 135),
        holder(Some("partial-brin"), "Partial BRIN on posted_at", Agent::Claude, Some(40_901), "~/Humata/payments-rewrite/partial", 31),
        benchmark,
        postgres,
        holder(None, "", Agent::Shell, Some(39_001), "~/scratch/probe", 12),
        holder(None, "", Agent::Shell, Some(39_002), "~/scratch/bench", 45),
    ]
}

/// Devbox-1's page, in light mode, with nothing open.
fn machine_page() -> App {
    machine_page_with(|_| Vec::new())
}

fn machine_page_with(holders: impl FnOnce(i64) -> Vec<humata2_clients::RunningHolder>) -> App {
    let mut app = App::new(fixture::design());
    app.fakes.machines.set_detail(Ok(MachineDetail {
        machine: MachineId::new("devbox-1"),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: "devbox-1.lan".to_owned(),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".to_owned(), "read:org".to_owned()] }),
            agents: vec![(Agent::Claude, "2.4.1".to_owned()), (Agent::Codex, "0.98.0".to_owned())],
            disk_free: 11_000_000_000,
            disk_total: 500_000_000_000,
            variables: vec!["DATABASE_URL".to_owned(), "STRIPE_KEY".to_owned()],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![
            ChannelFolder { channel: ChannelId::new("payments-rewrite"), path: "~/Humata/payments-rewrite/".to_owned(), sessions: 4, bytes: 3_100_000_000 },
            ChannelFolder { channel: ChannelId::new("infra"), path: "~/Humata/infra/".to_owned(), sessions: 2, bytes: 800_000_000 },
            ChannelFolder { channel: ChannelId::new("pr-reviews"), path: "~/Humata/pr-reviews/".to_owned(), sessions: 0, bytes: 200_000_000 },
        ],
        sessions: ["ledger", "brin", "rate", "partial-brin", "benchmark", "postgres"]
            .map(SessionId::new)
            .to_vec(),
        rules: vec![
            rule("files_read_other_machine", "Read files on other machines", RuleSetting::Allow),
            rule("git_push", "Push its branch or open a pull request", RuleSetting::Ask),
            rule("board_read", "Read the global board or other channels", RuleSetting::Allow),
            rule("board_change", "Change the global board or other channels", RuleSetting::Ask),
        ],
        stop: None,
    }));
    let held = holders(app.ui.clients().daemon.now_ms());
    if !held.is_empty() {
        app.fakes.daemon.set_running_holders(held);
    }
    app.fold_quiet_channels();
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Machine(MachineId::new("devbox-1"))));
    assert!(app.contains("machine-overview"));
    app
}

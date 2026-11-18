//! Stop all sessions on a machine (frames 72 to 76): the real window on
//! fakes, in the state each approved frame shows, recorded and compared like
//! the other native frames. Run by `native_test`; record with
//! `bazel run //apps/humata2/desktop:native_test_record`.
use super::*;
use humata2_clients::machines::{
    ChannelFolder, GhLogin, MachineDetail, MachineHealth, Reach, ReachRule,
};
use humata2_clients::model::{
    Agent, Capability, ChannelId, MachineId, MachineStop, RuleSetting, StoppedBy,
};
use humata2_hosts::HostsAction as H;

fn rule(capability: &str, label: &str, setting: RuleSetting) -> ReachRule {
    ReachRule { capability: Capability::new(capability), label: label.to_owned(), setting }
}

fn devbox() -> MachineId {
    MachineId::new("devbox-1")
}

/// The stop devbox-1 carries in the frames: made by you at 14:02.
fn stop_at_1402(delivered: bool) -> MachineStop {
    MachineStop { stop_id: "stop-1".to_owned(), at: "14:02".to_owned(), by: StoppedBy::You, delivered }
}

/// An app whose fake machines report devbox-1 as the frames draw it, in
/// `mode`, with `stop` already made on it, before any page is open; devbox-1
/// is offline unless `online`.
fn app_with(mode: ThemeMode, stop: Option<MachineStop>, online: bool) -> App {
    let mut workspace = fixture::design();
    for machine in workspace.machines.iter_mut().filter(|machine| machine.id == devbox()) {
        machine.online = online;
        // An unreachable machine shows when it was last seen (frame 76).
        machine.lost_at = (!online).then(|| "14:02".to_owned());
    }
    let mut app = App::new(workspace);
    app.fakes.machines.set_detail(Ok(MachineDetail {
        machine: devbox(),
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
    // The fake keeps the stop on the workspace's machine (and its sessions
    // read Stopped when it is delivered), as a stop made earlier would.
    app.fakes.machines.set_stop(&devbox(), stop);
    app.fold_quiet_channels();
    app.cx.update(|cx| Theme::change(mode, None, cx));
    app
}

/// devbox-1's page open on `app`.
fn open_machine(app: &mut App) {
    app.send(AppAction::Switch(Route::Machine(devbox())));
    assert!(app.contains("machine-overview"));
}

#[test]
fn stop_all_sessions_confirm_reference_image() {
    let mut app = app_with(ThemeMode::Light, None, true);
    open_machine(&mut app);
    app.click("machine-menu");
    assert!(app.contains("machine-stop-sessions"));
    app.click("machine-stop-sessions");
    assert!(app.contains("machine-stop-dialog"));
    assert!(!app.contains("machine-stop-banner"));
    app.snapshot("72-stop-confirm");
}

#[test]
fn machine_page_with_sessions_stopped_reference_image() {
    let mut app = app_with(ThemeMode::Dark, Some(stop_at_1402(true)), true);
    open_machine(&mut app);
    assert!(app.contains("machine-stop-banner"));
    assert!(app.contains("machine-start-sessions"));
    assert!(!app.contains("machine-stop-dialog"));
    assert!(!app.contains("machine-start-dialog"));
    app.snapshot("73-machine-sessions-stopped");
}

#[test]
fn hosts_page_with_sessions_stopped_reference_image() {
    let mut app = app_with(ThemeMode::Light, Some(stop_at_1402(true)), true);
    app.send(AppAction::Switch(Route::Hosts));
    app.send(AppAction::Hosts(H::Select(devbox())));
    // The row's own Start sessions button and the card's.
    assert!(app.contains("host-start devbox-1"));
    assert!(!app.contains("machine-stop-banner"));
    app.snapshot("74-hosts-sessions-stopped");
}

#[test]
fn start_sessions_confirm_reference_image() {
    let mut app = app_with(ThemeMode::Dark, Some(stop_at_1402(true)), true);
    open_machine(&mut app);
    app.click("machine-start-sessions");
    assert!(app.contains("machine-start-dialog"));
    assert!(app.contains("machine-stop-banner"));
    app.snapshot("75-start-confirm");
}

#[test]
fn stop_not_delivered_reference_image() {
    // devbox-1 can't be reached, so the stop waits on the host (frame 76).
    let mut app = app_with(ThemeMode::Light, Some(stop_at_1402(false)), false);
    open_machine(&mut app);
    assert!(app.contains("machine-stop-banner"));
    assert!(app.contains("machine-reconnect"));
    assert!(app.contains("machine-cancel-stop"));
    assert!(!app.contains("machine-start-sessions"));
    app.snapshot("76-stop-not-delivered");
}

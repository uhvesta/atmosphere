//! D784: the close-tab dialog, its toasts and the machine page's running
//! list, drawn by the real window on fakes.
//!
//! These are golden candidates, never registered: nothing here compares
//! with `__snapshots__`. To look at them, set `HUMATA_D784_SHOTS` to a
//! folder (for example `--test_env=HUMATA_D784_SHOTS=<folder>` with a
//! folder Bazel's sandbox can write to, or read `candidates` from the
//! test's undeclared outputs, which are used when it is not set) and each
//! test writes its PNG there.
use super::*;
use gpui_kit::component::theme::ThemeMode;
use humata2_clients::machines::{ChannelFolder, GhLogin, MachineDetail, MachineHealth, Reach};
use humata2_clients::model::{Agent, ChannelId, MachineId};
use humata2_clients::RunningHolder;
use humata2_session::{CloseAction, CloseChoice};
use std::path::PathBuf;

/// Where candidate images go, when anyone asked for them.
fn shots() -> Option<PathBuf> {
    let folder = std::env::var_os("HUMATA_D784_SHOTS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR").map(|out| PathBuf::from(out).join("candidates"))
        })?;
    std::fs::create_dir_all(&folder).expect("the candidate folder can be made");
    Some(folder)
}

/// Draws the window and writes it as `name.png`, when asked to.
fn shoot(app: &mut App, name: &str) {
    app.draw();
    let Some(folder) = shots() else {
        return;
    };
    let image = app.cx.capture_screenshot(app.window).expect("headless image");
    image.save(folder.join(format!("{name}.png"))).expect("the image is written");
}

fn devbox() -> MachineId {
    MachineId::new("devbox-1")
}

/// An app on the design workspace in `mode` whose session `id` has `title`.
fn app_titled(mode: ThemeMode, id: &str, title: &str) -> App {
    let mut workspace = fixture::design();
    // The desk sessions live in the desk fixture, not in the design workspace.
    fixture::desk(&mut workspace);
    for session in workspace.sessions.iter_mut().filter(|session| session.id.as_str() == id) {
        session.title = title.to_owned();
    }
    let mut app = App::new(workspace);
    app.fold_quiet_channels();
    app.cx.update(|cx| Theme::change(mode, None, cx));
    app
}

fn ask(app: &mut App, session: &str) {
    app.send(AppAction::Navigate(Route::Session(SessionId::new(session))));
    app.send(AppAction::Session(SessionAction::Close(CloseAction::Ask { group: 0, index: 0 })));
}

fn closing(app: &App) -> bool {
    app.ui.read(|state| state.session.closing.is_some())
}

#[test]
fn an_agent_on_a_remote_machine_asks_with_three_ways_to_close() {
    let mut app = app_titled(ThemeMode::Light, "desk-brin-rfc", "BRIN rollout RFC");
    ask(&mut app, "desk-brin-rfc");
    assert!(closing(&app));
    for id in ["close-dialog", "close-cancel", "close-end", "close-stop", "close-keep", "close-always"] {
        assert!(app.contains(id), "{id} is drawn");
    }
    shoot(&mut app, "close-agent-remote");
    app.click("close-cancel");
    assert!(!closing(&app), "Cancel leaves the tab as it was");
}

#[test]
fn a_shell_on_this_mac_asks_without_an_archive_line() {
    let mut app = app_titled(ThemeMode::Dark, "desk-tidy-infra", "");
    ask(&mut app, "desk-tidy-infra");
    assert!(closing(&app));
    assert!(app.contains("close-dialog"));
    shoot(&mut app, "close-shell-this-mac");
    let asked = app.ui.read(|state| state.session.closing.clone()).expect("asking");
    assert_eq!(asked.agent, "Shell");
    assert_eq!(asked.machine, "This Mac");
}

#[test]
fn the_always_keep_running_box_is_ticked_from_the_dialog() {
    let mut app = app_titled(ThemeMode::Light, "desk-brin-rfc", "BRIN rollout RFC");
    ask(&mut app, "desk-brin-rfc");
    app.click("close-always");
    assert!(app.ui.read(|state| state.session.closing.as_ref().is_some_and(|c| c.always_keep_running)));
}

#[test]
fn stopping_says_where_and_offers_resume() {
    let mut app = app_titled(ThemeMode::Light, "desk-brin-rfc", "BRIN rollout RFC");
    ask(&mut app, "desk-brin-rfc");
    app.click("close-stop");
    assert!(!closing(&app));
    let toast = app.ui.read(|state| state.toasts.iter().map(|toast| toast.text.clone()).collect::<Vec<_>>());
    assert_eq!(toast, vec!["Stopped BRIN rollout RFC on devbox-1.".to_owned()]);
    shoot(&mut app, "close-stopped-toast");
}

#[test]
fn keep_running_is_the_default_and_hides_the_tab() {
    let mut app = app_titled(ThemeMode::Light, "desk-brin-rfc", "BRIN rollout RFC");
    ask(&mut app, "desk-brin-rfc");
    app.send(AppAction::Session(SessionAction::Close(CloseAction::Choose(CloseChoice::KeepRunning))));
    assert!(!closing(&app));
}

fn holder(session: Option<&str>, title: &str, agent: Agent, pid: Option<u32>, folder: &str) -> RunningHolder {
    RunningHolder {
        tab: pid.map(|pid| format!("tab-{pid}")),
        session: session.map(SessionId::new),
        title: title.to_owned(),
        channel: session.map(|_| ChannelId::new("payments-rewrite")),
        desk: false,
        agent,
        pid,
        folder: folder.to_owned(),
        started_unix_ms: None,
        attached: false,
        machine: devbox(),
        stopped: pid.is_none(),
    }
}

fn machine_page_app(mode: ThemeMode, online: bool) -> App {
    let mut workspace = fixture::design();
    for machine in workspace.machines.iter_mut().filter(|machine| machine.id == devbox()) {
        machine.online = online;
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
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".to_owned()] }),
            agents: vec![(Agent::Claude, "2.4.1".to_owned())],
            disk_free: 11_000_000_000,
            disk_total: 500_000_000_000,
            variables: Vec::new(),
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![ChannelFolder {
            channel: ChannelId::new("payments-rewrite"),
            path: "~/Humata/payments-rewrite/".to_owned(),
            sessions: 2,
            bytes: 3_100_000_000,
        }],
        sessions: ["ledger", "brin"].map(SessionId::new).to_vec(),
        rules: Vec::new(),
        stop: None,
    }));
    let now = app.ui.clients().daemon.now_ms();
    let minutes = |n: i64| Some(now - n * 60_000);
    let mut ledger = holder(Some("ledger"), "Ledger schema migration", Agent::Claude, Some(40_117), "~/Humata/payments-rewrite/ledger");
    ledger.started_unix_ms = minutes(26 * 60);
    ledger.attached = true;
    let mut retry = holder(Some("rate"), "Retry budget fix", Agent::Copilot, Some(41_822), "~/Humata/payments-rewrite/api");
    retry.started_unix_ms = minutes(135);
    let brin = holder(Some("brin"), "BRIN index experiment", Agent::Claude, None, "~/Humata/payments-rewrite/brin");
    let mut orphan = holder(None, "", Agent::Shell, Some(39_001), "~/scratch/probe");
    orphan.started_unix_ms = minutes(12);
    app.fakes.daemon.set_running_holders(vec![ledger, retry, brin, orphan]);
    app.fold_quiet_channels();
    app.cx.update(|cx| Theme::change(mode, None, cx));
    app.send(AppAction::Switch(Route::Machine(devbox())));
    app
}

#[test]
fn the_machine_page_lists_sessions_stopped_ones_and_orphans() {
    let mut app = machine_page_app(ThemeMode::Light, true);
    assert!(app.contains("machine-running"));
    assert!(app.contains("running-session-menu-0-button"));
    assert!(app.contains("running-orphan-menu-0-button"));
    app.click("running-orphan-menu-0-button");
    assert!(app.contains("running-orphan-menu-0-0"), "an orphan's menu opens with Open first");
    shoot(&mut app, "machine-running-list");
}

#[test]
fn an_offline_machine_offers_no_row_menus() {
    let mut app = machine_page_app(ThemeMode::Light, false);
    assert!(!app.contains("running-session-menu-0-button"));
}

#[test]
fn ending_a_session_row_asks_once() {
    let mut app = machine_page_app(ThemeMode::Light, true);
    app.click("running-orphan-menu-0-button");
    app.click("running-orphan-menu-0-2");
    assert!(app.contains("running-end-dialog"));
    shoot(&mut app, "machine-running-end");
}

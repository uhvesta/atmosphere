//! MASSDELETE (D875): multi-select mode on Home and the machine page, the
//! delete confirmation sheet (picked, delete-all count gate, stale re-plan,
//! dark appearance), and the post-delete toast.
use super::*;
use humata2_clients::machines::{
    ChannelFolder, GhLogin, MachineDetail, MachineHealth, Reach, ReachRule,
};
use humata2_clients::mass_delete::DeleteScope;
use humata2_clients::model::{Agent, Capability, ChannelId, ForkKind, MachineId, RuleSetting};
use humata2_mass_delete::{MassDeleteAction, Surface};
use humata2_palette::{Command, PaletteAction};

fn devbox_detail() -> MachineDetail {
    let rule = |capability: &str, label: &str, setting: RuleSetting| ReachRule {
        capability: Capability::new(capability),
        label: label.to_owned(),
        setting,
    };
    MachineDetail {
        machine: MachineId::new("devbox-1"),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: "devbox-1.lan".to_owned(),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin {
                user: "uhvesta".to_owned(),
                scopes: vec!["repo".to_owned(), "read:org".to_owned()],
            }),
            agents: vec![
                (Agent::Claude, "2.4.1".to_owned()),
                (Agent::Codex, "0.98.0".to_owned()),
            ],
            disk_free: 11_000_000_000,
            disk_total: 500_000_000_000,
            variables: vec!["DATABASE_URL".to_owned(), "STRIPE_KEY".to_owned()],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![
            ChannelFolder {
                channel: ChannelId::new("payments-rewrite"),
                path: "~/Humata/payments-rewrite/".to_owned(),
                sessions: 4,
                bytes: 3_100_000_000,
            },
            ChannelFolder {
                channel: ChannelId::new("infra"),
                path: "~/Humata/infra/".to_owned(),
                sessions: 2,
                bytes: 800_000_000,
            },
        ],
        sessions: [
            "ledger",
            "brin",
            "rate",
            "partial-brin",
            "benchmark",
            "postgres",
        ]
        .map(SessionId::new)
        .to_vec(),
        rules: vec![
            rule(
                "files_read_other_machine",
                "Read files on other machines",
                RuleSetting::Allow,
            ),
            rule(
                "git_push",
                "Push its branch or open a pull request",
                RuleSetting::Ask,
            ),
        ],
        stop: None,
    }
}

/// Home select mode: `home-select`, ticking sessions (with a context fork going
/// along), the 48 pt bottom bar and its Select menu, opening the confirm sheet
/// with unpushed work blockers, and confirming to see the no-Undo toast.
#[test]
fn home_select_mode_picked_sheet_and_toast() {
    let mut workspace = fixture::design();
    if let Some(brin) = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "brin")
    {
        brin.fork = Some(ForkKind::Conversation);
    }
    let mut app = App::new(workspace);
    app.fakes.daemon.set_unpushed(
        &SessionId::new("ledger"),
        vec![(
            "ledger".into(),
            "3 uncommitted files on uhvesta/ledger-schema, never pushed".into(),
        )],
    );
    app.fakes.daemon.set_unpushed(
        &SessionId::new("refund"),
        vec![(
            "api".into(),
            "2 commits on uhvesta/refund-retries never pushed".into(),
        )],
    );
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));

    // Enter select mode from Home's toolbar button.
    assert!(app.contains("home-select"));
    app.click("home-select");
    assert!(app.contains("home-select-done"));
    assert!(app.contains("select-bar-delete"));
    assert!(app.contains("select-menu"));
    assert_eq!(
        app.ui.read(|s| s.mass_delete.surface.clone()),
        Some(Surface::Home)
    );

    // Pick a whole channel from the Select menu in the bottom bar (m2-1).
    app.click("select-menu");
    assert!(app.contains("select-menu-card"));
    assert!(app.contains("select-channel-payments-rewrite"));
    app.click("select-channel-payments-rewrite");
    assert_eq!(app.ui.read(|s| s.mass_delete.selected_count()), 6);
    app.click("select-menu");
    assert!(app.contains("select-menu-card"));
    app.snapshot("massdelete-home-select");
    app.click("select-menu");

    // Reset and pick 3 sessions; context forks `brin` and `schema-review` go along automatically.
    app.send(AppAction::MassDelete(MassDeleteAction::SelectNothing));
    for id in ["ledger", "refund", "postgres"] {
        app.send(AppAction::MassDelete(MassDeleteAction::Toggle(
            SessionId::new(id),
        )));
    }
    assert_eq!(app.ui.read(|s| s.mass_delete.picked_count()), 3);
    assert_eq!(app.ui.read(|s| s.mass_delete.fork_count()), 2);
    assert_eq!(app.ui.read(|s| s.mass_delete.selected_count()), 5);
    assert!(
        app.ui
            .read(|s| s.mass_delete.is_along(&SessionId::new("brin")))
    );

    // Open the confirm sheet from the bottom bar's Delete button (m1-2).
    app.click("select-bar-delete");
    assert!(app.contains("delete-sessions-dialog"));
    assert!(app.contains("delete-sessions-totals"));
    assert!(app.contains("delete-work-ledger"));
    assert!(app.contains("delete-work-refund"));
    assert_eq!(
        app.ui
            .read(|s| s.mass_delete.sheet.as_ref().unwrap().planned_count()),
        5
    );
    app.ui.read(|s| {
        let plan = s
            .mass_delete
            .sheet
            .as_ref()
            .unwrap()
            .plan
            .as_ref()
            .unwrap();
        for fork in ["brin", "schema-review"] {
            let entry = plan
                .sessions
                .iter()
                .find(|session| session.session.as_str() == fork)
                .expect("conversation fork included in plan");
            assert_eq!(entry.goes_with, Some(SessionId::new("ledger")));
        }
    });
    assert!(!app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));

    // Tick only ledger's work blocker: refund still blocks Delete.
    app.click("delete-work-ledger");
    assert!(!app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));
    app.snapshot("massdelete-plan-picked");

    // Tick refund's work blocker too: Delete turns on.
    app.click("delete-work-refund");
    assert!(app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));

    // Confirm: sheet closes, select mode exits, sessions vanish, no-Undo toast shows (m2-6).
    app.click("delete-sessions-confirm");
    assert!(!app.contains("delete-sessions-dialog"));
    assert!(!app.ui.read(|s| s.mass_delete.selecting()));
    for gone in ["ledger", "brin", "schema-review", "refund", "postgres"] {
        assert!(
            app.ui
                .read(|s| s.workspace.session(&SessionId::new(gone)).is_none()),
            "{gone} should be deleted"
        );
        assert!(
            app.ui.read(|s| !s
                .workspace
                .requests
                .iter()
                .any(|r| r.session.as_str() == gone)),
            "requests for {gone} should be withdrawn"
        );
    }
    assert!(app.ui.read(|s| s
        .workspace
        .requests
        .iter()
        .all(|r| s.workspace.session(&r.session).is_some())));
    let toast = app
        .ui
        .read(|s| s.toasts.last().cloned())
        .expect("post-delete toast appears");
    assert!(toast.text.contains("Deleted 5 sessions."));
    assert!(toast.text.contains("for 30 days."));
    assert!(toast.action.is_none(), "delete toast has no Undo");
    app.snapshot("massdelete-after-delete");
}

/// Machine page select mode: `machine-select` link, `machine-select-all` heading,
/// selecting every session on the machine, and opening/cancelling the confirm sheet (m2-2).
#[test]
fn machine_page_select_all_and_confirm_sheet() {
    let mut app = App::new(fixture::design());
    app.fakes.machines.set_detail(Ok(devbox_detail()));
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Machine(MachineId::new("devbox-1"))));
    assert!(app.contains("machine-overview"));
    assert!(app.contains("machine-select"));

    app.click("machine-select");
    assert!(app.contains("machine-select-all"));
    assert!(app.contains("machine-select-done"));
    assert!(app.contains("select-bar-delete"));
    assert_eq!(
        app.ui.read(|s| s.mass_delete.surface.clone()),
        Some(Surface::Machine(MachineId::new("devbox-1")))
    );

    // Select all sessions on devbox-1 (6 on devbox-1 + 1 context fork along).
    app.click("select-menu");
    app.click("select-everything");
    assert_eq!(app.ui.read(|s| s.mass_delete.selected_count()), 7);
    app.snapshot("massdelete-machine-select-all");

    // Open the confirm sheet, then Cancel: ticks remain intact.
    app.click("select-bar-delete");
    assert!(app.contains("delete-sessions-dialog"));
    assert_eq!(
        app.ui
            .read(|s| s.mass_delete.sheet.as_ref().unwrap().planned_count()),
        7
    );
    app.click("delete-sessions-cancel");
    assert!(!app.contains("delete-sessions-dialog"));
    assert_eq!(app.ui.read(|s| s.mass_delete.selected_count()), 7);
}

/// Delete all sessions from the palette: `SelectSessions` orders before
/// `DeleteAllSessions`, count gate stays locked until work boxes are ticked
/// and exact count is typed, dark appearance renders cleanly, and offline
/// machines are left alone (m1-3, m2-3, m2-5).
#[test]
fn delete_all_from_palette_count_gate_and_dark_mode() {
    let mut workspace = fixture::design();
    let mut offline = workspace
        .session(&SessionId::new("pr-482"))
        .unwrap()
        .clone();
    offline.id = SessionId::new("gpu-reranker");
    offline.title = "Train reranker".into();
    offline.channel = ChannelId::new("infra");
    offline.machine = MachineId::new("gpu-box");
    offline.status = Status::Offline;
    workspace.sessions.push(offline);

    let mut app = App::new(workspace);
    app.fakes.daemon.set_unpushed(
        &SessionId::new("ledger"),
        vec![(
            "ledger".into(),
            "3 uncommitted files on uhvesta/ledger-schema, never pushed".into(),
        )],
    );
    app.fakes.daemon.set_unpushed(
        &SessionId::new("refund"),
        vec![(
            "api".into(),
            "2 commits on uhvesta/refund-retries never pushed".into(),
        )],
    );
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));

    // Palette lists "Select sessions…" before "Delete all sessions…".
    app.send(AppAction::OpenPalette);
    app.send(AppAction::Palette(PaletteAction::Query("delete".into())));
    let select_idx = app
        .ui
        .read(|s| {
            s.palette
                .entries
                .iter()
                .position(|e| e.command == Command::SelectSessions)
        })
        .expect("SelectSessions command in palette");
    let delete_all_idx = app
        .ui
        .read(|s| {
            s.palette
                .entries
                .iter()
                .position(|e| e.command == Command::DeleteAllSessions)
        })
        .expect("DeleteAllSessions command in palette");
    assert!(
        select_idx < delete_all_idx,
        "Select sessions… must come before Delete all sessions…"
    );

    // Pick DeleteAllSessions -> opens the --all sheet with count gate.
    app.send(AppAction::Palette(PaletteAction::Pick(delete_all_idx)));
    assert!(app.contains("delete-sessions-dialog"));
    assert!(app.contains("delete-sessions-count"));
    let (scope, planned, needs_count) = app.ui.read(|s| {
        let sheet = s.mass_delete.sheet.as_ref().expect("sheet open");
        (sheet.scope.clone(), sheet.planned_count(), sheet.needs_count())
    });
    assert_eq!(scope, DeleteScope::All);
    assert_eq!(planned, 9);
    assert!(needs_count);
    assert!(!app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));
    app.snapshot("massdelete-plan-all-empty");

    // Dark appearance: tick both work blockers and type the count (m2-5).
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.send(AppAction::MassDelete(MassDeleteAction::TickWork(
        SessionId::new("ledger"),
    )));
    app.send(AppAction::MassDelete(MassDeleteAction::TickWork(
        SessionId::new("refund"),
    )));
    // Wrong count keeps Delete disabled.
    app.send(AppAction::MassDelete(MassDeleteAction::Typed("99".into())));
    assert!(!app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));
    // Exact count enables the filled red Delete all button.
    app.send(AppAction::MassDelete(MassDeleteAction::Typed(
        planned.to_string(),
    )));
    assert!(app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));
    app.draw();
    app.scroll_by("delete-sessions-list", -2000.);
    app.draw();
    app.snapshot("massdelete-plan-all-typed-dark");

    // Confirm: reachable sessions are deleted, offline gpu-box session stays.
    app.click("delete-sessions-confirm");
    assert!(!app.contains("delete-sessions-dialog"));
    assert_eq!(app.ui.read(|s| s.workspace.sessions.len()), 1);
    assert!(
        app.ui
            .read(|s| s.workspace.session(&SessionId::new("gpu-reranker")).is_some())
    );
    let toast = app
        .ui
        .read(|s| s.toasts.last().cloned())
        .expect("toast after delete-all");
    assert!(toast.text.contains("Deleted 9 sessions."));
    assert!(toast.text.contains("gpu-box"));
    assert!(toast.text.contains("left alone"));
}

/// Stale plan refusal: when a session changes after the plan, Delete refuses,
/// deletes nothing, shows the amber `changed` notice and re-plan, clears the
/// changed session's work tick, and lets the user confirm the new plan (m1-4).
#[test]
fn stale_plan_shows_changed_banner_and_requires_reconfirm() {
    let mut app = App::new(fixture::design());
    app.fakes.daemon.set_unpushed(
        &SessionId::new("refund"),
        vec![(
            "api".into(),
            "2 commits on uhvesta/refund-retries never pushed".into(),
        )],
    );
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));

    app.click("home-select");
    for id in ["ledger", "refund"] {
        app.send(AppAction::MassDelete(MassDeleteAction::Toggle(
            SessionId::new(id),
        )));
    }
    app.click("select-bar-delete");
    assert!(app.contains("delete-sessions-dialog"));

    // Tick refund's unpushed work box so Delete is enabled.
    app.click("delete-work-refund");
    assert!(app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));

    // Mutate refund's work and mark the next delete as stale.
    app.fakes.daemon.set_unpushed(
        &SessionId::new("refund"),
        vec![(
            "api".into(),
            "3 commits on uhvesta/refund-retries never pushed".into(),
        )],
    );
    app.fakes.daemon.change_after_plan();
    app.click("delete-sessions-confirm");

    // Nothing was deleted; the amber stale notice is shown and refund's work tick cleared.
    assert!(app.contains("delete-sessions-dialog"));
    assert!(app.contains("delete-sessions-stale"));
    assert!(
        app.ui
            .read(|s| s.workspace.session(&SessionId::new("ledger")).is_some())
    );
    assert!(
        app.ui
            .read(|s| s.workspace.session(&SessionId::new("refund")).is_some())
    );
    assert!(app.ui.read(|s| {
        let sheet = s.mass_delete.sheet.as_ref().unwrap();
        sheet.stale.as_ref().is_some_and(|stale| {
            stale
                .changes
                .iter()
                .any(|line| line.starts_with("Refund retries: 1 new commit on "))
        }) && sheet.is_changed(&SessionId::new("refund"))
            && !sheet.can_confirm()
    }));
    app.snapshot("massdelete-plan-stale");

    // Re-tick refund's work box and confirm the new plan.
    app.click("delete-work-refund");
    assert!(app.ui.read(|s| s.mass_delete.sheet.as_ref().unwrap().can_confirm()));
    app.click("delete-sessions-confirm");
    assert!(!app.contains("delete-sessions-dialog"));
    assert!(
        app.ui
            .read(|s| s.workspace.session(&SessionId::new("ledger")).is_none())
    );
    assert!(
        app.ui
            .read(|s| s.workspace.session(&SessionId::new("refund")).is_none())
    );
}

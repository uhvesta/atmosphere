//! The Fork (frame 62), Move (frame 63) and Archive-with-unpushed-work
//! (frame 65) dialogs, natively. The goldens are checked by eye against the
//! design frames.
use super::*;

/// FK-1 to FK-5, D873: the dialog opens collapsed on Code + context with its
/// facts line; More options shows the kind cards and fields; Fork sends
/// exactly what the dialog holds.
#[test]
fn the_fork_dialog_matches_frame_62_and_forks_what_it_shows() {
    use humata2_clients::model::{ForkKind, MachineId};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.click("session-fork");
    // Collapsed: no kind cards or fields, only the facts.
    assert!(app.contains("fork-facts"));
    assert!(app.contains("fork-more-options"));
    assert!(!app.contains("fork-kind-0-on"));
    assert!(!app.contains("fork-from"));
    assert!(!app.contains("fork-agent"));
    assert!(!app.contains("fork-machine"));
    assert!(!app.contains("fork-branch"));
    assert!(!app.contains("fork-preview"));
    app.snapshot("fork-dialog");
    app.click("fork-more-options");
    // Code + context is picked first, on Now, the same agent and machine,
    // with a branch suggested.
    assert!(app.contains("fork-kind-0-on"));
    assert!(app.contains("fork-from"));
    assert!(app.contains("fork-agent"));
    assert!(app.contains("fork-machine"));
    assert!(app.contains("fork-branch"));
    assert!(app.contains("fork-facts"));
    let branch = app.ui.read(|s| s.session.fork.as_ref().unwrap().branch.clone());
    assert!(!branch.is_empty());
    app.snapshot("fork-dialog-expanded");

    // Before an earlier turn, on another machine.
    app.send(AppAction::Session(SessionAction::ForkBeforeTurn(Some(2))));
    app.send(AppAction::Session(SessionAction::ForkMachine(MachineId::new(
        "this-mac",
    ))));
    app.click("fork-create");
    let sent = app.fakes.daemon.last_fork().expect("the fork was sent");
    assert_eq!(sent.kind, ForkKind::Worktree);
    assert_eq!(sent.before_turn, Some(2));
    assert_eq!(sent.machine, Some(MachineId::new("this-mac")));
    assert_eq!(sent.branch, branch);
    assert!(app.ui.read(|s| s.session.fork.is_none()));

    // Code only has no Fork from: a fresh agent has no turns to cut at. The
    // dialog opens collapsed again.
    app.click("session-fork");
    assert!(!app.contains("fork-kind-2"));
    app.click("fork-more-options");
    app.click("fork-kind-2");
    assert!(!app.contains("fork-from"));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.click("fork-more-options");
    app.snapshot("fork-dialog-dark");
    app.click("fork-more-options");
    app.click("fork-kind-1");
    app.snapshot("fork-dialog-expanded-dark");
    app.click("fork-cancel");
    assert!(app.ui.read(|s| s.session.fork.is_none()));
}

/// MV-1 to MV-3, MV-6: a machine list (here, offline and the rest), what
/// moves, and a button naming the target. Moving where it is, or to an
/// offline machine, can't be picked.
#[test]
fn the_move_dialog_matches_frame_63_and_moves_to_the_picked_machine() {
    use humata2_clients::model::{MachineId, SessionId};
    let mut app = App::new(fixture::design());
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.open("rate", Panel::Info);
    app.send(AppAction::Session(SessionAction::StartMove));
    assert!(app.contains("move-dialog"));
    assert!(app.contains("move-what"));
    // devbox-1 is where it runs and gpu-box is offline: neither is a target.
    app.click("move-machine-devbox-1");
    app.click("move-machine-gpu-box");
    assert!(app.ui.read(|s| s.session.moving.as_ref().unwrap().to.as_ref() != Some(&MachineId::new("gpu-box"))));
    app.click("move-machine-this-mac");
    app.snapshot("move-dialog");
    app.click("move-confirm");
    let (session, to) = app.fakes.daemon.last_move().expect("the move was sent");
    assert_eq!(session, SessionId::new("rate"));
    assert_eq!(to, MachineId::new("this-mac"));
    assert!(!app.contains("move-dialog"));

    app.send(AppAction::Session(SessionAction::StartMove));
    app.click("move-cancel");
    assert!(!app.contains("move-dialog"));
}

/// AR-3/AR-4: archiving a session with unpushed work asks first, naming each
/// repository's unpushed work; a clean session archives at once with Undo.
#[test]
fn archiving_unpushed_work_asks_first_as_in_frame_65() {
    use humata2_clients::model::SessionId;
    let mut app = App::new(fixture::design());
    let ledger = SessionId::new("ledger");
    app.fakes.daemon.set_archive_frees(&ledger, "2.4 GB");
    app.fakes.daemon.set_unpushed(
        &ledger,
        vec![
            ("ledger".into(), "3 uncommitted files on uhvesta/ledger-schema, never pushed.".into()),
            ("api".into(), "1 commit on uhvesta/ledger-reads not pushed.".into()),
        ],
    );
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Session(SessionAction::Archive));
    assert!(app.contains("archive-dialog"));
    assert!(!app.ui.read(|s| s.workspace.session(&ledger).unwrap().archived));
    app.snapshot("archive-unpushed");

    app.click("archive-cancel");
    assert!(!app.contains("archive-dialog"));
    assert!(!app.ui.read(|s| s.workspace.session(&ledger).unwrap().archived));

    // Commit, push and open a PR, then archive.
    app.send(AppAction::Session(SessionAction::Archive));
    app.click("archive-push");
    assert_eq!(app.fakes.daemon.last_archive(), Some((ledger.clone(), true)));
    assert!(!app.contains("archive-dialog"));

    // Undo the archive to try the other choice.
    let toast = app
        .ui
        .read(|s| s.toasts.iter().find(|t| t.action.is_some()).unwrap().id);
    app.click(&format!("toast-undo-{toast}"));
    app.open("ledger", Panel::Changes);

    // Archive and delete the work.
    app.send(AppAction::Session(SessionAction::Archive));
    app.click("archive-delete");
    assert_eq!(app.fakes.daemon.last_archive(), Some((ledger.clone(), false)));
    assert!(app.ui.read(|s| s.workspace.session(&ledger).unwrap().archived));
}

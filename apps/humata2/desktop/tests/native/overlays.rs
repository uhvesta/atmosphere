//! The palette, New session and toasts (frames 30-32).
use super::*;

#[test]
fn palette_new_session_and_toast_reference_images() {
    use humata2_app::NewSessionAction as N;
    use humata2_clients::model::{ItemId, Scope};
    use humata2_palette::PaletteAction as P;
    let mut workspace = fixture::design_session(Status::Working);
    workspace.views.clear();
    let mut app = App::new(workspace);
    app.fold_quiet_channels();
    app.open("ledger", Panel::Changes);
    app.file_tab();
    app.send(AppAction::Review(
        humata2_review::ReviewAction::SelectFile {
            repo: "api".into(),
            path: "src/client.rs".into(),
        },
    ));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.send(AppAction::OpenPalette);
    app.send(AppAction::Palette(P::Query("ledg".into())));
    app.snapshot("30-palette");
    app.send(AppAction::Escape);
    app.send(AppAction::Switch(Route::Board(Scope::Global)));
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::StartNewForItem(ItemId::new("PAY-9")));
    app.send(AppAction::NewSession(N::ToggleRepo("acme/api".into())));
    app.snapshot("31-new-session");
    app.send(AppAction::NewSession(N::FirstMessage(
        "Work on PAY-9\nKeep tests passing".into(),
    )));
    app.click("new-session-create");
    let id = app
        .ui
        .read(|s| s.session.open.clone())
        .expect("new session opens");
    assert!(
        app.ui
            .read(|s| s.workspace.session_options[&id].repos == vec!["acme/ledger".to_owned()])
    );
    assert!(
        app.ui
            .read(|s| s.workspace.session_options[&id].model.is_none())
    );
    assert!(app.ui.read(|s| {
        s.workspace
            .items
            .iter()
            .find(|i| i.id.as_str() == "PAY-9")
            .unwrap()
            .session
            .as_ref()
            == Some(&id)
    }));
}

#[test]
fn archiving_offers_a_working_undo() {
    use humata2_clients::model::SessionId;
    let mut workspace = fixture::design();
    let mut currency = workspace
        .session(&SessionId::new("pr-482"))
        .unwrap()
        .clone();
    currency.id = SessionId::new("currency");
    currency.title = "Currency codes".into();
    currency.channel = humata2_clients::model::ChannelId::new("payments-rewrite");
    workspace.sessions.push(currency);
    // Golden 32's sidebar shows Pinned: Ledger and acme/api #482.
    for pinned in ["ledger", "pr-482"] {
        if let Some(session) = workspace.sessions.iter_mut().find(|s| s.id.as_str() == pinned) {
            session.pinned = true;
        }
    }
    let mut app = App::new(workspace);
    app.fakes
        .daemon
        .set_archive_frees(&SessionId::new("currency"), "1.4 GB");
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.open("currency", Panel::Info);
    app.send(AppAction::Session(SessionAction::Archive));
    assert!(app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("currency"))
            .unwrap()
            .archived
    }));
    app.snapshot("32-toast");
    let toast = app
        .ui
        .read(|s| s.toasts.iter().find(|t| t.action.is_some()).unwrap().id);
    app.click(&format!("toast-undo-{toast}"));
    assert!(!app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("currency"))
            .unwrap()
            .archived
    }));
}

#[test]
fn palette_file_results_open_the_current_sessions_clone() {
    use humata2_palette::{Command, PaletteAction as P};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.send(AppAction::OpenPalette);
    app.send(AppAction::Palette(P::Query("client.rs".into())));
    let index=app.ui.read(|s|s.palette.entries.iter().position(|e|matches!(&e.command,Command::File{session,repo,path} if session.as_str()=="ledger"&&repo=="api"&&path=="src/client.rs")).unwrap());
    app.send(AppAction::Palette(P::Pick(index)));
    assert!(app.contains("session-file"));
    assert!(app.ui.read(|s| {
        s.session
            .open
            .as_ref()
            .is_some_and(|id| id.as_str() == "ledger")
    }));
}

/// The keyboard shortcuts sheet over Home, in one appearance.
fn shortcuts_sheet(mode: ThemeMode, name: &str) {
    let mut app = App::new(fixture::design());
    app.cx.update(|cx| Theme::change(mode, None, cx));
    app.send(AppAction::Switch(Route::Home));
    app.send(AppAction::ShowShortcuts);
    assert!(
        app.ui
            .read(|s| matches!(s.overlay(), Some(humata2_app::Overlay::Shortcuts)))
    );
    app.snapshot(name);
}

#[test]
fn shortcuts_sheet_light() {
    shortcuts_sheet(ThemeMode::Light, "shortcuts-sheet-light");
}

#[test]
fn shortcuts_sheet_dark() {
    shortcuts_sheet(ThemeMode::Dark, "shortcuts-sheet-dark");
}

/// The New channel sheet after Create with no name: its error banner, in light mode.
#[test]
fn new_channel_error_light() {
    use humata2_app::NewChannelAction as N;
    let mut app = App::new(fixture::design());
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));
    app.send(AppAction::StartNewChannel);
    app.send(AppAction::NewChannel(N::Create));
    assert!(app.ui.read(|s| {
        matches!(
            s.overlay(),
            Some(humata2_app::Overlay::NewChannel(sheet)) if sheet.error.is_some()
        )
    }));
    app.snapshot("new-channel-error-light");
}

/// The shortcuts list scrolled to its end: the last shortcut is in the view.
#[test]
fn shortcuts_sheet_scrolled_light() {
    let mut app = App::new(fixture::design());
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));
    app.send(AppAction::ShowShortcuts);
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll(
                "shortcuts-list",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-2000.))),
                cx,
            );
        })
        .unwrap();
    app.settle();
    app.draw();
    let tree = app
        .cx
        .update_window(app.window, |_, window, _| view::capture(window))
        .expect("window stays open")
        .expect("view");
    assert!(tree.contains("Take a bug report snapshot"));
    app.snapshot("shortcuts-sheet-scrolled-light");
}

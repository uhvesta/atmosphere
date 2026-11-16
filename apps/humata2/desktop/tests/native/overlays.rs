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
    let mut app = App::new(workspace);
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

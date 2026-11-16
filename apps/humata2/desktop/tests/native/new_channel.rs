//! New channel over Home (frame 44; CH-1 to CH-4, CH-10).
use super::*;
use humata2_app::{NewChannelAction as N, Overlay};
use humata2_clients::model::MachineId;

/// Home as frame 25 draws it, with New channel open. The frame lists three
/// machines, so the fixture's stopped cloud VM is left out.
fn sheet_over_home() -> App {
    let mut workspace = fixture::design();
    workspace
        .machines
        .retain(|machine| machine.id.as_str() != "cloud-vm-3");
    for s in &mut workspace.sessions {
        s.vip = s.id.as_str() == "ledger";
        s.pinned = ["ledger", "pr-482"].contains(&s.id.as_str());
    }
    let mut app = App::new(workspace);
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));
    app.fold_quiet_channels();
    app.send(AppAction::StartNewChannel);
    app
}

fn ticked(app: &App) -> (Vec<String>, Vec<MachineId>) {
    app.ui.read(|s| match s.overlay() {
        Some(Overlay::NewChannel(sheet)) => (sheet.repos.clone(), sheet.machines.clone()),
        _ => panic!("New channel is open"),
    })
}

/// Frame 44: a name, "acme" searched, two repos and two machines ticked,
/// the offline gpu-box shown but not ticked.
#[test]
fn new_channel_frame_44() {
    let mut app = sheet_over_home();
    app.send(AppAction::NewChannel(N::Name("Billing exports".into())));
    app.send(AppAction::NewChannel(N::Query("acme".into())));
    app.click("new-channel-repo-acme/ledger");
    app.click("new-channel-repo-acme/api");
    app.click("new-channel-machine-devbox-1");
    assert_eq!(
        ticked(&app),
        (
            vec!["acme/ledger".to_owned(), "acme/api".to_owned()],
            vec![MachineId::new("this-mac"), MachineId::new("devbox-1")],
        )
    );
    assert!(app.contains("new-channel-repo-acme/docs"));
    assert!(!app.contains("new-channel-repo-octo/notes"));
    app.snapshot("44-new-channel");
}

/// CH-4: clicking an offline machine leaves it unticked.
#[test]
fn an_offline_machine_does_not_tick() {
    let mut app = sheet_over_home();
    app.click("new-channel-machine-gpu-box");
    assert_eq!(ticked(&app).1, vec![MachineId::new("this-mac")]);
}

/// CH-1, CH-10: Create with no repos makes the channel and opens it.
#[test]
fn create_with_no_repos_opens_the_channel() {
    let mut app = sheet_over_home();
    app.send(AppAction::NewChannel(N::Name("Notes".into())));
    app.click("new-channel-create");
    assert!(app.ui.read(|s| s.overlay().is_none()));
    assert!(
        app.fakes
            .daemon
            .workspace()
            .channels
            .iter()
            .any(|channel| channel.name == "notes" && channel.repos.is_empty())
    );
}

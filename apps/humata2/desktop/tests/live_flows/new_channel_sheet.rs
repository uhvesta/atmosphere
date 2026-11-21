//! Macro case 02 on a real `humatad2` whose `gh` is a fake that sees every
//! repo the world makes: step 4's whole-word match order in the repo
//! search, and step 10's progress sheet while the channel's clones run,
//! then the channel opens. The headless app is driven by clicks and typing.
use gpui_kit::Role;
use humata2_app::{NewChannel, Overlay, Route};
use humata2_clients::model::SetupState;
use humata2_flow::Action;

use super::world::World;
use super::{App, runtime};

/// What the open New channel sheet holds, or `None` while it isn't open.
fn sheet<R>(app: &App, read: impl FnOnce(&NewChannel) -> R) -> Option<R> {
    app.ui.read(|state| {
        state
            .overlays
            .iter()
            .rev()
            .find_map(|overlay| match overlay {
                Overlay::NewChannel(sheet) => Some(sheet),
                _ => None,
            })
            .map(|sheet| read(sheet))
    })
}

/// Waits until the app has loaded This Mac, so the sheet can tick it.
fn wait_for_machine(app: &mut App) {
    app.wait_until("This Mac to load", |app| {
        app.ui
            .read(|state| state.workspace.machines.iter().any(|machine| machine.online))
    });
}

/// Opens the sheet with its shortcut and waits until gh's repos are listed.
fn open_sheet(app: &mut App) {
    app.play(&Action::Key("shift-cmd-n".to_owned()));
    app.wait_until("the New channel sheet to open", |app| {
        sheet(app, |_| ()).is_some()
    });
    app.wait_until("the repos gh sees to be listed", |app| {
        sheet(app, |sheet| sheet.search.result.is_some()).unwrap_or(false)
    });
}

/// Types `text` into the field labelled `label` and waits until the sheet
/// holds it.
fn fill(app: &mut App, label: &str, text: &str, holds: fn(&NewChannel, &str) -> bool) {
    app.click(Role::TextInput, label);
    app.type_text(text);
    app.wait_until("the typed text to reach the sheet", |app| {
        sheet(app, |sheet| holds(sheet, text)).unwrap_or(false)
    });
}

/// Macro case 02 step 4: a repo whose name holds the search whole comes
/// before one that only holds its letters in order, though gh lists the
/// scattered one first.
#[test]
fn whole_word_matches_come_before_scattered_ones_in_the_repo_search() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        // The fake gh lists repos in the order they are made, so the
        // scattered match ("es" is in acme/docs only as e...s) comes first.
        world.repo("acme", "docs").expect("make a repo");
        world.repo("octo", "notes").expect("make a repo");
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);
    open_sheet(&mut app);
    assert_eq!(
        sheet(&app, |sheet| sheet.shown_repos().into_iter().map(str::to_owned).collect::<Vec<_>>()),
        Some(vec!["acme/docs".to_owned(), "octo/notes".to_owned()]),
        "with no search the list keeps gh's order"
    );

    fill(&mut app, "Search repos", "es", |sheet, text| sheet.query == text);
    assert_eq!(
        sheet(&app, |sheet| sheet.shown_repos().into_iter().map(str::to_owned).collect::<Vec<_>>()),
        Some(vec!["octo/notes".to_owned(), "acme/docs".to_owned()]),
        "the whole match comes first"
    );
    // The window draws them in that order too.
    let view = app.view();
    let notes = view
        .find("CheckBox \"octo/notes\"")
        .expect("octo/notes is a row");
    let docs = view
        .find("CheckBox \"acme/docs\"")
        .expect("acme/docs is a row");
    assert!(notes < docs, "the rows are drawn whole match first:\n{view}");
    drop(app);
    drop(world);
}

/// Macro case 02 step 10: while the clones run the sheet becomes the
/// progress sheet and the channel does not open; once the clones are done
/// the channel opens. The test holds the daemon's `git clone` and lets it
/// go, so no timing is assumed.
#[test]
fn the_progress_sheet_shows_while_the_clones_run_then_the_channel_opens() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start_holding_clones()
            .await
            .expect("start the daemon with clones held");
        for name in ["api", "ledger"] {
            world.repo("owner", name).expect("make a repo");
        }
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);
    open_sheet(&mut app);
    fill(&mut app, "Channel name", "billing-exports", |sheet, text| {
        sheet.name == text
    });
    app.click(Role::CheckBox, "owner/api");
    app.wait_until("owner/api to be ticked", |app| {
        sheet(app, |sheet| sheet.repos == ["owner/api"]).unwrap_or(false)
    });
    app.click(Role::CheckBox, "owner/ledger");
    app.wait_until("owner/ledger to be ticked", |app| {
        sheet(app, |sheet| sheet.repos == ["owner/api", "owner/ledger"]).unwrap_or(false)
    });
    app.click(Role::Button, "Create channel and clone 2 repos");

    // Every clone waits on the release pipe until the test writes to it.
    // Under SMB-001 the sheet closes and the channel opens as soon as the
    // channel record is created, while clone progress continues in
    // `workspace.setups`.
    app.wait_until("the channel to open with clone progress active", |app| {
        app.ui.read(|state| {
            matches!(state.route, Route::Channel(_))
                && !state
                    .overlays
                    .iter()
                    .any(|overlay| matches!(overlay, Overlay::NewChannel(_)))
                && state
                    .workspace
                    .setups
                    .iter()
                    .any(|setup| setup.name == "billing-exports")
        })
    });
    // The active setup shows a clone row for each repo, none done.
    let clones = app.ui.read(|state| {
        state
            .workspace
            .setups
            .iter()
            .find(|setup| setup.name == "billing-exports")
            .map(|setup| {
                setup
                    .steps
                    .iter()
                    .filter(|step| step.repo.is_some())
                    .map(|step| step.state == SetupState::Done)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    assert_eq!(clones.len(), 2, "one clone row per repo");
    assert!(
        clones.iter().all(|done| !done),
        "no clone is done while git is held"
    );
    let folder = world.channels_folder().join("billing-exports");
    assert!(
        !folder.join("main/api/.git").exists(),
        "the held clone has not landed"
    );

    // Let the clones go: setup clears and the clones are on disk.
    world.release_clones();
    app.wait_until("the channel setup to finish", |app| {
        app.ui.read(|state| state.workspace.setups.is_empty())
    });
    assert!(folder.join("main/api/.git").exists(), "api is cloned");
    assert!(folder.join("main/ledger/.git").exists(), "ledger is cloned");
    drop(app);
    drop(world);
}

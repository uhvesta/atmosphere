//! Flow 02 steps 1 to 5 and 8 to 10 (the New channel sheet) on a real
//! `humatad2` whose `gh` is a fake that sees every repo the world makes:
//! the sheet opens on ⇧⌘N with This Mac ticked, shows the folder the name
//! gives, narrows the repos gh sees, says how many it will clone, and
//! creating the channel leaves real clones on disk and opens the empty
//! channel page.
use gpui_kit::Role;
use humata2_app::{NewChannel, Overlay, Route};
use humata2_flow::Action;

use super::world::World;
use super::{App, runtime};

/// What the open New channel sheet holds, or `None` while it isn't open.
fn sheet<R>(app: &App, read: impl FnOnce(&NewChannel) -> R) -> Option<R> {
    app.ui.read(|state| match state.overlays.last() {
        Some(Overlay::NewChannel(sheet)) => Some(read(sheet)),
        _ => None,
    })
}

/// Waits until the app has loaded This Mac, so the sheet can tick it.
fn wait_for_machine(app: &mut App) {
    app.wait_until("This Mac to load", |app| {
        app.ui
            .read(|state| state.workspace.machines.iter().any(|machine| machine.online))
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

/// Flow 02 steps 1 to 5, 8, 9 and 10: from ⇧⌘N to the empty channel.
#[test]
fn a_channel_made_from_the_new_channel_sheet_clones_its_repos_and_opens_empty() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        for name in ["api", "ledger", "web"] {
            world.repo("owner", name).expect("make a repo");
        }
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);

    // Step 1: ⇧⌘N opens the sheet.
    app.play(&Action::Key("shift-cmd-n".to_owned()));
    app.wait_until("the New channel sheet to open", |app| {
        sheet(app, |_| ()).is_some()
    });

    // Step 2: gh's repos are listed and This Mac is ticked.
    app.wait_until("the repos gh sees to be listed", |app| {
        sheet(app, |sheet| sheet.search.result.is_some()).unwrap_or(false)
    });
    let ticked_machines = sheet(&app, |sheet| sheet.machines.len()).expect("the sheet is open");
    assert_eq!(ticked_machines, 1, "This Mac is ticked");
    let view = app.view();
    for repo in ["owner/api", "owner/ledger", "owner/web"] {
        assert!(view.contains(repo), "the list shows {repo}:\n{view}");
    }

    // Step 3: the name gives the folder, which the sheet never shows (PL-9).
    fill(&mut app, "Channel name", "billing-exports", |sheet, text| {
        sheet.name == text
    });
    assert!(
        !app.view().contains("~/Humata/billing-exports"),
        "the sheet hides the folder it will use"
    );

    // Step 4: a search narrows the list to what gh sees that matches.
    fill(&mut app, "Search repos", "api", |sheet, text| sheet.query == text);
    // The search runs after a pause in typing; wait for its answer.
    app.wait_until("the search for api to answer", |app| {
        sheet(app, |sheet| sheet.search.query == "api" && !sheet.search.loading).unwrap_or(false)
    });
    let view = app.view();
    assert!(view.contains("owner/api"), "the match stays:\n{view}");
    assert!(!view.contains("owner/web"), "a non-match goes:\n{view}");
    // Step 5: tick it, search again; a ticked repo stays at the top.
    app.click(Role::CheckBox, "owner/api");
    app.wait_until("owner/api to be ticked", |app| {
        sheet(app, |sheet| sheet.repos == ["owner/api"]).unwrap_or(false)
    });
    // The driver can't clear a field, so the new search is sent as the
    // action the field itself sends on each keystroke.
    app.cx.update(|cx| {
        app.ui.send(
            humata2_app::AppAction::NewChannel(humata2_app::NewChannelAction::Query(
                "ledger".to_owned(),
            )),
            cx,
        )
    });
    app.settle();
    app.wait_until("the second search to reach the sheet", |app| {
        sheet(app, |sheet| sheet.query == "ledger").unwrap_or(false)
    });
    app.wait_until("the search for ledger to answer", |app| {
        sheet(app, |sheet| sheet.search.query == "ledger" && !sheet.search.loading)
            .unwrap_or(false)
    });
    let view = app.view();
    assert!(view.contains("owner/api"), "the ticked repo stays:\n{view}");
    assert!(view.contains("owner/ledger"), "the new match shows:\n{view}");
    assert!(!view.contains("owner/web"), "a non-match goes:\n{view}");
    app.click(Role::CheckBox, "owner/ledger");
    app.wait_until("owner/ledger to be ticked", |app| {
        sheet(app, |sheet| sheet.repos == ["owner/api", "owner/ledger"]).unwrap_or(false)
    });
    assert!(
        app.view().contains("Create channel and clone 2 repos"),
        "the button says it will clone 2"
    );

    // Steps 9 and 10: create; the sheet closes and the channel opens
    // (the progress sheet is too brief to hold still for a local clone).
    app.click(Role::Button, "Create channel and clone 2 repos");
    app.wait_until("the channel page to open", |app| {
        app.ui.read(|state| {
            matches!(&state.route, Route::Channel(id) if state.workspace.channel(id).is_some())
                && !state
                    .overlays
                    .iter()
                    .any(|overlay| matches!(overlay, Overlay::NewChannel(_)))
        })
    });
    app.click(Role::Tab, "Sessions");
    app.wait_until("the empty Sessions state", |app| {
        app.view().contains("Nothing in # billing-exports yet")
    });
    assert!(app.view().contains("New session"), "it offers New session");
    app.wait_until("the channel's clones to finish", |app| {
        app.ui.read(|state| state.workspace.setups.is_empty())
    });

    // The daemon holds the channel with the two repos, and the disk has
    // main clones of those two and not the third.
    let channel = app
        .ui
        .read(|state| {
            state
                .workspace
                .channels
                .iter()
                .find(|channel| channel.name == "billing-exports")
                .cloned()
        })
        .expect("the channel is listed");
    assert_eq!(channel.repos.len(), 2);
    let folder = world.channels_folder().join("billing-exports");
    for sub in ["main", "sessions", "shared"] {
        assert!(folder.join(sub).is_dir(), "{sub} is missing");
    }
    assert!(folder.join("main/api/.git").exists(), "api is cloned");
    assert!(folder.join("main/ledger/.git").exists(), "ledger is cloned");
    assert!(!folder.join("main/web").exists(), "web is not cloned");
    drop(app);
    drop(world);
}

/// Flow 02 step 2 with a signed-out gh: the sheet says why the repos can't
/// be listed and offers Retry, not an empty list.
#[test]
fn a_signed_out_gh_shows_the_sheets_signed_out_state_not_an_empty_list() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        world.repo("owner", "api").expect("make a repo");
        world.sign_gh_out().expect("sign gh out");
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);
    app.play(&Action::Key("shift-cmd-n".to_owned()));
    app.wait_until("the sheet to say the repos can't be listed", |app| {
        sheet(app, |sheet| sheet.search.problem().is_some()).unwrap_or(false)
    });
    let (shown, error) = sheet(&app, |sheet| {
        (
            sheet.shown_repos().into_iter().map(str::to_owned).collect::<Vec<_>>(),
            sheet.search.problem().map(str::to_owned),
        )
    })
    .expect("the sheet is open");
    assert!(shown.is_empty(), "no list is shown, not an empty one");
    assert!(error.is_some_and(|error| !error.is_empty()));
    let view = app.view();
    assert!(view.contains("Retry"), "it offers Retry:\n{view}");
    assert!(!view.contains("owner/api"), "no repo is listed:\n{view}");
    drop(app);
    drop(world);
}

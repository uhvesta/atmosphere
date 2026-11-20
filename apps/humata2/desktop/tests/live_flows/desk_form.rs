//! The New desk session form on a real `humatad2` (macro case 01, step 2
//! and 4): the path the form previews is the folder the daemon then makes,
//! and a repo added with the form's own chip is cloned into that folder.
//! The app is headless.
use std::path::{Path, PathBuf};

use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_clients::model::SessionId;

use super::world::World;
use super::{App, runtime};

/// The session whose page is open, if one is.
fn open_session(app: &App) -> Option<SessionId> {
    app.ui.read(|state| match &state.route {
        Route::Session(id) => Some(id.clone()),
        _ => None,
    })
}

/// The folder the app shows for a Desk session, once it has loaded it.
fn folder_of(app: &App, id: &SessionId) -> Option<PathBuf> {
    app.ui.read(|state| {
        state
            .desk
            .record(id)
            .map(|record| PathBuf::from(record.folder.path()))
    })
}

/// The last two parts of a path: the Desk folder's name and the scratch
/// folder's name. The preview starts at `~/Humata`, the real path at the
/// daemon's own root, so only the tail can be the same.
fn tail(path: &Path) -> (String, String) {
    let part = |path: Option<&Path>| {
        path.and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    (part(path.parent()), part(Some(path)))
}

fn wait_for_machine(app: &mut App) {
    app.wait_until("This Mac to load as online", |app| {
        app.ui
            .read(|state| state.workspace.machines.iter().any(|machine| machine.online))
    });
}

/// Opens the form and types `title` into it. The Desk page has two buttons
/// named New desk session when nothing is open, so the driver can't pick
/// one by label; the app action is what both of them send.
fn open_form_with_title(app: &mut App, title: &str) {
    app.click(Role::Link, "Desk");
    app.wait_until("the Desk page", |app| {
        app.ui.read(|state| state.route == Route::Desk)
    });
    app.cx
        .update(|cx| app.ui.send(AppAction::StartNewDeskSession, cx));
    app.settle();
    app.wait_until("the New desk session form", |app| {
        app.ui.read(|state| state.desk.new.is_some())
    });
    app.click(Role::TextInput, "Title");
    app.type_text(title);
    app.wait_until("the typed title to reach the form", |app| {
        app.ui.read(|state| {
            state
                .desk
                .new
                .as_ref()
                .is_some_and(|form| form.title == title)
        })
    });
}

/// The path the open form says its folder will get.
fn preview(app: &App) -> String {
    app.ui
        .read(|state| state.desk.scratch_preview())
        .expect("the form is open, so it has a preview")
}

/// Macro case 01 steps 2 and 4: the form's previewed folder is the one the
/// daemon makes, and the form's own repo chip is cloned into it.
#[test]
fn the_new_desk_session_form_previews_the_folder_the_daemon_makes_and_clones_its_repo_chip() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        world.repo("owner", "api").expect("make the repo to add");
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);

    let title = "batch export idea";
    open_form_with_title(&mut app, title);

    // Step 4: the form's own repo chip. Add a repo…, search, tick the
    // result, Done (the repo search adds what is ticked).
    app.click(Role::Button, "Add a repo\u{2026}");
    app.wait_until("the form's repo field", |app| {
        app.ui
            .read(|state| state.desk.new.as_ref().is_some_and(|form| form.adding_repo))
    });
    app.click(Role::TextInput, "Repo to add");
    app.type_text("owner/api");
    app.wait_until("owner/api among the results", |app| {
        app.view().contains("CheckBox \"owner/api\"")
    });
    app.click(Role::CheckBox, "owner/api");
    app.click(Role::Button, "Done");
    app.wait_until("the repo to become a chip on the form", |app| {
        app.ui.read(|state| {
            state
                .desk
                .new
                .as_ref()
                .is_some_and(|form| form.repos == ["owner/api"] && !form.adding_repo)
        })
    });
    assert!(
        app.view().contains("Remove owner/api"),
        "the chip can be removed"
    );

    // Step 2: the path the dialog draws (it reads `scratch_preview`; the
    // view text doesn't carry the folder caption).
    let previewed = preview(&app);
    app.click(Role::Button, "Start");
    app.wait_until("the session page to open", |app| open_session(app).is_some());
    let id = open_session(&app).expect("the session page is open");
    app.wait_until("the app to know the new Desk session's folder", |app| {
        folder_of(app, &id).is_some()
    });
    let folder = folder_of(&app, &id).expect("the folder is known");

    assert!(folder.is_dir(), "the folder exists on disk: {}", folder.display());
    assert_eq!(
        tail(Path::new(&previewed)),
        tail(&folder),
        "the folder the daemon made is the one the form previewed ({previewed} vs {})",
        folder.display()
    );

    // The chip's repo is cloned into the folder and listed on the session.
    app.wait_until("the chip's repo cloned into the folder", |_| {
        folder.join("api").join("README.md").is_file()
    });
    assert!(
        app.ui.read(|state| state
            .desk
            .record(&id)
            .is_some_and(|record| record.repos.iter().any(|repo| repo == "owner/api"))),
        "the Desk session lists the repo"
    );
    drop(app);
    drop(world);
}

/// Macro case 01 step 2: when the title's folder is taken, the form
/// previews the next suffix and the daemon makes that one.
#[test]
fn a_taken_title_previews_the_suffixed_folder_the_daemon_then_makes() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);

    let title = "batch export idea";
    open_form_with_title(&mut app, title);
    app.click(Role::Button, "Start");
    app.wait_until("the first session page", |app| open_session(app).is_some());
    let first = open_session(&app).expect("the first session page is open");
    app.wait_until("the first folder to be known", |app| {
        folder_of(app, &first).is_some()
    });
    let first_folder = folder_of(&app, &first).expect("the first folder is known");

    open_form_with_title(&mut app, title);
    let previewed = preview(&app);
    app.click(Role::Button, "Start");
    app.wait_until("the second session page", |app| {
        open_session(app).is_some_and(|id| id != first)
    });
    let second = open_session(&app).expect("the second session page is open");
    app.wait_until("the second folder to be known", |app| {
        folder_of(app, &second).is_some()
    });
    let second_folder = folder_of(&app, &second).expect("the second folder is known");

    assert_ne!(first_folder, second_folder, "a taken name gets another folder");
    assert_eq!(
        tail(Path::new(&previewed)),
        tail(&second_folder),
        "the form previewed the folder the daemon made ({previewed} vs {})",
        second_folder.display()
    );
    drop(app);
    drop(world);
}

//! The Desk on a real `humatad2`: starting a session from the page, adding a
//! repo, copying a document to a channel, closing, keeping, reopening and
//! cleaning up, and a picked folder that Humata never deletes. The app is
//! headless.
//!
//! These prove the screens in flows 01 (steps 1, 2, 3, 5, 7, 9, 10) and 04
//! (steps 1, 2, 9) and the macro cases 01 (steps 1, 2, 4, 8) and 04 (steps 1
//! and 11).
//!
//! The Desk has no events of its own. The page, the folder bar and the
//! clean-up list read the Desk data the app loaded with the workspace, so
//! these tests wait for the app to show what the daemon holds, as a person
//! would after moving between pages.
use std::path::{Path, PathBuf};

use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_clients::Clients;
use humata2_clients::desk::{Access, FolderChoice, NewDeskSession};
use humata2_clients::model::{Agent, SessionId};
use tokio::runtime::Runtime;

use super::world::World;
use super::{App, channel_with_repo, runtime};

/// The daemon's folder: where it keeps `ch/` (channels and the Desk).
fn root_of(world: &World) -> PathBuf {
    world
        .conversations()
        .parent()
        .expect("the conversations folder has a parent")
        .to_path_buf()
}

/// The same place, with links resolved, so `/tmp` and `/private/tmp` agree.
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

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

/// Waits until the app has a machine that is online, which New desk session
/// starts on.
fn wait_for_machine(app: &mut App) {
    app.wait_until("This Mac to load as online", |app| {
        app.ui
            .read(|state| state.workspace.machines.iter().any(|machine| machine.online))
    });
}

/// Clicks Desk in the sidebar and waits for the page.
fn open_desk_page(app: &mut App) {
    app.click(Role::Link, "Desk");
    app.wait_until("the Desk page", |app| {
        app.ui.read(|state| state.route == Route::Desk)
    });
}

/// New desk session from the page: the form opens, the title is typed, and
/// Start is clicked. Waits for the session page and returns its session.
///
/// The Desk page has two buttons named New desk session when nothing is
/// open (the toolbar's and the empty card's), so the driver can't pick one
/// by label; the app action is what both of them send.
fn start_from_the_page(app: &mut App, title: &str) -> SessionId {
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
    app.click(Role::Button, "Start");
    app.wait_until("the session page to open", |app| open_session(app).is_some());
    let id = open_session(app).expect("the session page is open");
    app.wait_until("the app to know the new Desk session's folder", |app| {
        folder_of(app, &id).is_some()
    });
    id
}

/// Closes the open session from its menu.
fn close_from_the_menu(app: &mut App) {
    app.click(Role::Button, "Session actions");
    app.click(Role::Button, "Close session");
}

/// The words of a size, as the clean-up button writes them.
fn sized(bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes >= GB {
        format!("{}.{} GB", bytes / GB, bytes % GB / (GB / 10))
    } else {
        format!("{} MB", bytes / MB)
    }
}

/// The Clean up dialog's delete button: its label names how many sessions
/// are ticked and what they free.
fn delete_button(app: &App) -> String {
    app.ui.read(|state| {
        let dialog = state
            .desk
            .clean_up
            .as_ref()
            .expect("the Clean up dialog is open");
        let freed = dialog.freed_bytes();
        let count = match dialog.ticked.len() {
            0 => "sessions".to_owned(),
            1 => "1 session".to_owned(),
            n => format!("{n} sessions"),
        };
        if freed > 0 {
            format!("Delete {count} \u{b7} frees {}", sized(freed))
        } else {
            format!("Delete {count}")
        }
    })
}

/// Clean up…: opens the dialog, makes sure `title`'s session is the one
/// ticked, and deletes. Waits until the dialog closes.
fn clean_up_only(app: &mut App, title: &str) {
    app.click(Role::Button, "Clean up\u{2026}");
    app.wait_until("the closed sessions to be listed", |app| {
        app.ui.read(|state| {
            state
                .desk
                .clean_up
                .as_ref()
                .is_some_and(|dialog| !dialog.loading && dialog.rows.len() == 1)
        })
    });
    // A scratch folder comes ticked; a picked folder doesn't.
    let ticked = app.ui.read(|state| {
        state
            .desk
            .clean_up
            .as_ref()
            .is_some_and(|dialog| dialog.ticked.len() == 1)
    });
    if !ticked {
        app.click(Role::CheckBox, title);
        app.wait_until("the row to be ticked", |app| {
            app.ui.read(|state| {
                state
                    .desk
                    .clean_up
                    .as_ref()
                    .is_some_and(|dialog| dialog.ticked.len() == 1)
            })
        });
    }
    let label = delete_button(app);
    app.click(Role::Button, &label);
    app.wait_until("the Clean up dialog to close", |app| {
        app.ui.read(|state| state.desk.clean_up.is_none())
    });
}

/// The channel's copy of `file` in a `shared/` folder, anywhere under the
/// channels folder except the Desk's own.
fn shared_copy(channels: &Path, file: &str) -> Option<PathBuf> {
    std::fs::read_dir(channels)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name() != "desk")
        .map(|entry| entry.path().join("shared").join(file))
        .find(|path| path.is_file())
}

/// Flow 01 steps 1, 2, 3, 5, 7 and 9 (copy), macro case 01 steps 1, 2, 4 and
/// 8, and flow 04 steps 1, 2 and 9: the Desk page, a new scratch session,
/// its folder bar, a repo added, a document the agent wrote, and the copy.
#[test]
fn a_desk_session_starts_from_the_page_gets_a_repo_and_its_document_is_copied_to_a_channel() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        channel_with_repo(&world, &clients, "payments").await;
        world.repo("owner", "api").expect("make the repo to add");
        (world, clients)
    });
    let channels = root_of(&world).join("ch");
    let mut app = App::open(&runtime, clients);
    wait_for_machine(&mut app);

    // Step 1: the Desk page with nothing open.
    open_desk_page(&mut app);
    assert!(
        app.ui.read(|state| state.desk.open().is_empty()),
        "no desk session is open yet"
    );

    // Step 2: New desk session, a title, Start.
    let title = "Checkout errors from Slack";
    let id = start_from_the_page(&mut app, title);
    assert_eq!(open_session(&app), Some(id.clone()), "the session page opens");
    assert!(
        app.ui.read(|state| state.desk.new.is_none()),
        "the form closes once the session starts"
    );
    let folder = folder_of(&app, &id).expect("the folder is known");
    assert!(folder.is_dir(), "the scratch folder exists on disk: {}", folder.display());
    assert!(
        resolved(&folder).starts_with(resolved(&channels.join("desk"))),
        "the scratch folder is under the Desk's folder: {}",
        folder.display()
    );

    // Step 3: the folder bar with its "Goes 7 days after close" chip (DK-3, DV-4), no Fork,
    // and the sidebar lists the session under Desk. The path is hidden and
    // Copy path copies it (PL-10, D466).
    app.wait_until("the folder bar and the agent", |app| {
        let view = app.view();
        view.contains("Status \"Goes 7 days after close\"") && view.contains("agent up")
    });
    let view = app.view();
    assert!(
        view.contains("Button \"Copy path\""),
        "the folder bar offers Copy path:\n{view}"
    );
    assert!(
        !view.contains(&folder.display().to_string()),
        "the folder bar hides the path (D466):\n{view}"
    );
    assert!(!view.contains("Fork"), "a Desk session has no Fork button:\n{view}");
    assert!(
        app.ui.read(|state| state.is_desk_session(&id)),
        "the session is a Desk session"
    );
    // The sidebar's rows have no public text; their Debug shows the title.
    assert!(
        app.ui
            .read(|state| format!("{:?}", state.sidebar.desk).contains(title)),
        "the sidebar lists it under Desk"
    );

    // Step 5: Add repo in the folder bar clones the repo into the folder.
    app.click(Role::Button, "Add repo");
    app.wait_until("the Add repo sheet", |app| {
        app.ui.read(|state| state.desk.menu.is_some())
    });
    app.click(Role::TextInput, "Repo to add");
    app.type_text("owner/api");
    app.play(&humata2_flow::Action::Key("enter".to_owned()));
    app.wait_until("the repo to be listed on the session", |app| {
        app.ui.read(|state| {
            state
                .desk
                .record(&id)
                .is_some_and(|record| record.repos.iter().any(|repo| repo == "owner/api"))
        })
    });
    app.wait_until("the repo's clone in the folder", |_| {
        folder.join("api").join("README.md").is_file()
    });
    app.wait_until("the repo's chip in the folder bar", |app| {
        app.view().contains("owner/api")
    });

    // Step 7: the agent writes notes.md (stood in for here); back on the
    // Desk page the document is listed, and Copy to a channel… copies it.
    std::fs::write(folder.join("notes.md"), "# Notes\nit was the retry loop\n")
        .expect("write the document the agent would write");
    open_desk_page(&mut app);
    app.wait_until("notes.md under Documents", |app| {
        app.ui.read(|state| {
            state
                .desk
                .documents()
                .iter()
                .any(|(document, session)| document.path == "notes.md" && session.id == id)
        })
    });
    app.click(Role::Button, "Copy to a channel\u{2026}");
    app.wait_until("the Copy dialog", |app| {
        app.ui.read(|state| state.desk.copy.is_some())
    });
    app.click(Role::RadioButton, "# payments");
    app.click(Role::Button, "Copy");
    app.wait_until("the copy in the channel's shared folder", |_| {
        shared_copy(&channels, "notes.md").is_some()
    });
    app.wait_until("a note saying where it was copied", |app| {
        app.ui.read(|state| {
            state
                .toasts
                .iter()
                .any(|toast| toast.text.contains("notes.md") && toast.text.contains("payments"))
        })
    });
    let copy = shared_copy(&channels, "notes.md").expect("the copy exists");
    assert_eq!(
        std::fs::read_to_string(copy).expect("read the copy"),
        "# Notes\nit was the retry loop\n"
    );
    assert!(folder.join("notes.md").is_file(), "the Desk keeps its copy");
    assert!(
        app.ui.read(|state| state.desk.copy.is_none()),
        "the Copy dialog closes"
    );
    drop(app);
    drop(world);
}

/// Starts a Desk session through the client, so the app has it when it
/// opens, and returns its id.
async fn start_through_the_client(clients: &Clients, title: &str) -> SessionId {
    let mac = World::this_mac(clients)
        .await
        .expect("the workspace has This Mac");
    clients
        .desk
        .start(NewDeskSession {
            title: title.to_owned(),
            agent: Agent::Claude,
            machine: mac,
            folder: FolderChoice::Scratch {
                name: "checkout-errors-from-slack".to_owned(),
            },
            repos: Vec::new(),
            access: Access::default(),
        })
        .await
        .expect("start the desk session")
}

/// Flow 01 steps 9 and 10, flow 04 step 9 and macro case 04 step 11: Close
/// session, Keep, Reopen (the agent resumes), close again, Clean up….
#[test]
fn closing_keeping_reopening_and_cleaning_up_from_the_desk_page() {
    let runtime: Runtime = runtime();
    let title = "Checkout errors from Slack";
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = start_through_the_client(&clients, title).await;
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the app to know the Desk session", |app| {
        folder_of(app, &id).is_some()
    });
    let folder = folder_of(&app, &id).expect("the folder is known");
    assert!(folder.is_dir(), "the scratch folder exists: {}", folder.display());
    app.show_session(&id);
    app.wait_until("the agent in its terminal", |app| {
        app.view().contains("agent up --session-id")
    });

    // Close session: the page moves to the Desk, the session is under
    // Closed with days left, and its folder stays.
    close_from_the_menu(&mut app);
    app.wait_until("the Desk page after closing", |app| {
        app.ui.read(|state| state.route == Route::Desk)
    });
    let days_left = |app: &App| {
        app.ui.read(|state| {
            state
                .desk
                .closed()
                .iter()
                .find(|row| row.session.id == id)
                .map(|row| row.desk.closed.as_ref().and_then(|closed| closed.days_left))
        })
    };
    app.wait_until("the session under Closed", |app| days_left(app).is_some());
    assert!(
        days_left(&app).flatten().is_some_and(|days| days > 0),
        "a scratch folder has days left: {:?}",
        days_left(&app)
    );
    assert!(folder.is_dir(), "closing keeps the folder");
    assert!(
        app.ui.read(|state| state.desk.open().is_empty()),
        "nothing is open now"
    );

    // Keep takes the delete time away.
    app.click(Role::Button, "Keep");
    app.wait_until("no days left after Keep", |app| days_left(app) == Some(None));
    assert!(folder.is_dir());

    // Reopen opens the session again and the agent resumes its conversation.
    app.click(Role::Button, "Reopen");
    app.wait_until("the session page after Reopen", |app| {
        open_session(app) == Some(id.clone())
    });
    app.wait_until("the agent to resume", |app| {
        app.view().contains("agent up --resume")
    });
    app.wait_until("the session open again on the Desk", |app| {
        app.ui.read(|state| {
            state
                .desk
                .open()
                .iter()
                .any(|row| row.session.id == id && row.desk.closed.is_none())
        })
    });
    assert_eq!(folder_of(&app, &id), Some(folder.clone()), "the same folder");

    // Close again, then Clean up… deletes it and its folder.
    close_from_the_menu(&mut app);
    app.wait_until("the Desk page after closing again", |app| {
        app.ui.read(|state| state.route == Route::Desk)
    });
    app.wait_until("the session under Closed again", |app| {
        days_left(app).is_some()
    });
    clean_up_only(&mut app, title);
    app.wait_until("the session gone from the Desk", |app| {
        app.ui.read(|state| {
            state.desk.record(&id).is_none()
                && state.desk.open().is_empty()
                && state.desk.closed().is_empty()
        })
    });
    // The host moves the folder to the Bin before it marks the row deleted
    // (D440), so it is gone once the Desk drops the row.
    assert!(!folder.exists(), "Clean up… deletes the scratch folder");
    drop(app);
    drop(world);
}

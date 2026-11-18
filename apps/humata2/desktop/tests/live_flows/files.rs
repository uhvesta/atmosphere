//! The Files page on a real `humatad2`: two panes, a new folder, Copy, Move,
//! rename, delete to the trash and Restore, and files dropped in from Finder,
//! all through the live file manager. The app is headless.
//!
//! The harness has one daemon, so one machine (This Mac). Copying between
//! two machines is not tried here; the daemon-level transfer tests cover it.

use std::path::{Path, PathBuf};

use humata2_app::{AppAction, Route};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{EntryKind, FileRoot, Folder, TransferPhase};
use humata2_files::{FilesAction, Side};
use tokio::runtime::Runtime;

use super::{App, World, running_session, runtime, session_now};

const REPO: &str = "owner/ledger";

/// The app on a daemon with one channel and one running session, plus where
/// that channel's folders are on disk. `app` comes first so it is dropped
/// before the daemon.
struct Scene<'r> {
    app: App<'r>,
    _world: World,
    mac: MachineId,
    session: SessionId,
    channel: ChannelId,
    /// The channel's `shared/` folder on disk.
    shared: PathBuf,
    /// The session's clone of the repo on disk.
    clone: PathBuf,
}

impl Scene<'_> {
    fn shared_folder(&self) -> Folder {
        Folder::root(self.mac.clone(), FileRoot::ChannelShared(self.channel.clone()))
    }

    fn clone_folder(&self) -> Folder {
        Folder::root(
            self.mac.clone(),
            FileRoot::SessionClone {
                session: self.session.clone(),
                repo: REPO.to_owned(),
            },
        )
    }
}

/// The daemon names the channel's and the session's folders itself, so they
/// are found under `<channels>/<channel>/...`, not guessed. They are there
/// once the session has started.
fn find_folders(channels: &Path) -> (PathBuf, PathBuf) {
    let children = |folder: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(folder)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    // The channel's folders come with the channel, and the clone before
    // the agent launches (D441): one look is enough.
    for channel in children(channels) {
        for session in children(&channel.join("sessions")) {
            let clone = session.join("ledger");
            let shared = channel.join("shared");
            if clone.join(".git").exists() && shared.is_dir() {
                return (shared, clone);
            }
        }
    }
    panic!("the running session has no clone under {}", channels.display());
}

fn start(runtime: &Runtime) -> Scene<'_> {
    let (world, clients, mac, session, channel) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session = running_session(&world, &clients, "Fix the ledger", "hello agent").await;
        let channel = session_now(&clients.daemon, &session)
            .await
            .expect("the session is listed")
            .channel;
        let mac = World::this_mac(&clients).await.expect("the workspace has This Mac");
        (world, clients, mac, session, channel)
    });
    // The daemon's folder is the conversations folder's parent.
    let root = world
        .conversations()
        .parent()
        .expect("the conversations folder has a parent")
        .to_path_buf();
    let (shared, clone) = find_folders(&root.join("ch"));
    let app = App::open(runtime, clients);
    Scene {
        app,
        _world: world,
        mac,
        session,
        channel,
        shared,
        clone,
    }
}

fn files(app: &mut App, action: FilesAction) {
    app.cx.update(|cx| app.ui.send(AppAction::Files(action), cx));
    app.settle();
}

fn names(app: &App, side: Side) -> Vec<String> {
    app.ui.read(|state| {
        state
            .files
            .pane(side)
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    })
}

fn lists(app: &mut App, side: Side, name: &str) -> bool {
    names(app, side).iter().any(|listed| listed == name)
}

/// Whether the pane has read `folder` and shows what it read.
fn shows(app: &App, side: Side, folder: &Folder) -> bool {
    app.ui.read(|state| {
        let pane = state.files.pane(side);
        pane.folder.as_ref() == Some(folder) && pane.loaded && !pane.loading
    })
}

/// Waits until at least `count` transfers are in the tray and all of them are
/// Done; a failed or cancelled one ends the test.
fn wait_transfers_done(app: &mut App, count: usize) {
    app.wait_until("the transfers to be Done", |app| {
        let rows = app.ui.read(|state| state.files.transfers.clone());
        for row in &rows {
            match &row.phase {
                TransferPhase::Failed { reason } => panic!("a transfer failed: {reason}"),
                TransferPhase::Cancelled => panic!("a transfer was cancelled"),
                _ => {}
            }
        }
        rows.len() >= count && rows.iter().all(|row| row.phase == TransferPhase::Done)
    });
}

fn open_page_on(scene: &mut Scene) {
    let (left, right) = (scene.clone_folder(), scene.shared_folder());
    let app = &mut scene.app;
    app.cx.update(|cx| app.ui.send(AppAction::Navigate(Route::Files), cx));
    app.settle();
    app.wait_until("the Files page to be open and set up", |app| {
        app.ui.read(|state| state.files.opened && state.files.ready)
    });
    files(app, FilesAction::Navigate(Side::Left, left.clone()));
    files(app, FilesAction::Navigate(Side::Right, right.clone()));
    app.wait_until("both panes to list their folders", |app| {
        shows(app, Side::Left, &left) && shows(app, Side::Right, &right)
    });
}

#[test]
fn the_files_page_copies_moves_renames_and_deletes_on_a_real_daemon() {
    let runtime = runtime();
    let mut scene = start(&runtime);
    open_page_on(&mut scene);
    let (shared, clone, mac) = (scene.shared.clone(), scene.clone.clone(), scene.mac.clone());
    let app = &mut scene.app;

    // The left pane lists the clone's files.
    assert!(lists(app, Side::Left, "README.md"));

    // New folder in the right pane, through the name prompt.
    files(app, FilesAction::Activate(Side::Right));
    files(app, FilesAction::BeginNewFolder);
    assert!(app.ui.read(|state| state.files.naming.is_some()), "the prompt opens");
    files(app, FilesAction::CommitName("out".to_owned()));
    app.wait_until("the new folder to be listed", |app| lists(app, Side::Right, "out"));
    assert!(shared.join("out").is_dir());
    let out_is_a_folder = app.ui.read(|state| {
        state
            .files
            .right
            .entries
            .iter()
            .any(|entry| entry.name == "out" && entry.kind == EntryKind::Folder)
    });
    assert!(out_is_a_folder);

    // Copy README.md from the clone into shared/.
    files(app, FilesAction::Select(Side::Left, "README.md".to_owned(), false));
    files(app, FilesAction::Copy);
    // The row comes from the daemon's transfer feed, which pushes after the
    // start answers: settling can't wait for it, so wait for the row.
    app.wait_until("the copy's tray row", |app| {
        app.ui.read(|state| !state.files.transfers.is_empty())
    });
    wait_transfers_done(app, 1);
    app.wait_until("the copy to be listed on the right", |app| {
        lists(app, Side::Right, "README.md")
    });
    assert_eq!(
        std::fs::read(shared.join("README.md")).expect("the copy is on disk"),
        std::fs::read(clone.join("README.md")).expect("the original is still there"),
    );

    // Rename it on the right.
    files(app, FilesAction::Select(Side::Right, "README.md".to_owned(), false));
    files(app, FilesAction::BeginRename);
    files(app, FilesAction::CommitName("NOTES.md".to_owned()));
    app.wait_until("the new name to be listed", |app| {
        lists(app, Side::Right, "NOTES.md") && !lists(app, Side::Right, "README.md")
    });
    assert!(shared.join("NOTES.md").is_file());
    assert!(!shared.join("README.md").exists());

    // Move a second file from the clone into shared/.
    let moved: Vec<u8> = (0..2000u32).map(|index| u8::try_from(index % 251).unwrap_or(0)).collect();
    std::fs::write(clone.join("move-me.txt"), &moved).expect("seed a file in the clone");
    let left = app.ui.read(|state| state.files.left.folder.clone()).expect("the left pane has a folder");
    files(app, FilesAction::Navigate(Side::Left, left));
    app.wait_until("the seeded file to be listed on the left", |app| {
        lists(app, Side::Left, "move-me.txt")
    });
    files(app, FilesAction::Select(Side::Left, "move-me.txt".to_owned(), false));
    files(app, FilesAction::Move);
    wait_transfers_done(app, 2);
    app.wait_until("the moved file to leave the clone and arrive in shared", |_| {
        !clone.join("move-me.txt").exists() && shared.join("move-me.txt").is_file()
    });
    assert_eq!(std::fs::read(shared.join("move-me.txt")).expect("moved bytes"), moved);

    // Delete NOTES.md on the right: gone from the listing, in the trash.
    app.wait_until("the right pane to list NOTES.md again", |app| {
        lists(app, Side::Right, "NOTES.md")
    });
    files(app, FilesAction::Select(Side::Right, "NOTES.md".to_owned(), false));
    files(app, FilesAction::Delete);
    app.wait_until("NOTES.md to leave the listing", |app| !lists(app, Side::Right, "NOTES.md"));
    assert!(!shared.join("NOTES.md").exists());
    app.wait_until("the trash count to show it", |app| {
        app.ui.read(|state| state.files.trash_count >= 1)
    });

    // The trash lists it; Restore brings it back.
    files(app, FilesAction::OpenTrash(mac));
    let mut item = None;
    app.wait_until("the trash to list NOTES.md", |app| {
        item = app.ui.read(|state| {
            state
                .files
                .trash
                .as_ref()
                .and_then(|trash| trash.listing.as_ref())
                .and_then(|listing| {
                    listing
                        .items
                        .iter()
                        .find(|item| item.description.contains("NOTES.md"))
                        .map(|item| item.id.clone())
                })
        });
        item.is_some()
    });
    files(app, FilesAction::Restore(item.expect("the trash lists it after the wait")));
    app.wait_until("NOTES.md to be listed again", |app| lists(app, Side::Right, "NOTES.md"));
    assert!(shared.join("NOTES.md").is_file());
    files(app, FilesAction::CloseTrash);

    // Making "out" again is refused, shows an error and changes nothing.
    assert!(app.ui.read(|state| state.files.error.is_none()));
    let before = names(app, Side::Right);
    files(app, FilesAction::Activate(Side::Right));
    files(app, FilesAction::BeginNewFolder);
    files(app, FilesAction::CommitName("out".to_owned()));
    app.wait_until("the name-taken error", |app| app.ui.read(|state| state.files.error.is_some()));
    assert_eq!(names(app, Side::Right), before);
    assert!(shared.join("out").is_dir());
}

#[test]
fn files_dropped_from_finder_upload_into_the_pane() {
    let runtime = runtime();
    let mut scene = start(&runtime);
    open_page_on(&mut scene);
    let shared = scene.shared.clone();
    let app = &mut scene.app;

    // A folder outside the daemon's own, as Finder would drop from.
    let local = tempfile::tempdir().expect("make a local folder");
    let big: Vec<u8> = (0..3 * 1024 * 1024u32)
        .map(|index| u8::try_from(index % 251).unwrap_or(0))
        .collect();
    let small = b"a small file\n".to_vec();
    let big_path = local.path().join("big.bin");
    let small_path = local.path().join("small.txt");
    std::fs::write(&big_path, &big).expect("write the big file");
    std::fs::write(&small_path, &small).expect("write the small file");

    files(app, FilesAction::Dropped(Side::Right, vec![big_path, small_path]));
    wait_transfers_done(app, 1);
    app.wait_until("both files to be listed on the right", |app| {
        lists(app, Side::Right, "big.bin") && lists(app, Side::Right, "small.txt")
    });
    assert_eq!(std::fs::read(shared.join("big.bin")).expect("the big file arrived"), big);
    assert_eq!(std::fs::read(shared.join("small.txt")).expect("the small file arrived"), small);
    assert!(app.ui.read(|state| state.files.error.is_none()));
}

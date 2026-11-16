//! FileService's folder operations through `humatad2`'s gRPC API: list,
//! stat, make a folder, rename and move to the trash, in a session's clone,
//! a channel's folder, its `shared/` and a main clone (FM-2, FM-3, FM-10).

mod common;

use std::path::{Path, PathBuf};

use common::{Daemon, error_of, folder, invalid_field, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, ChannelRepo, MainClone as MainState, SessionRecord, Store};
use tonic::transport::Channel;
use tonic::{Code, Status};

const SESSION: &str = "se_ops";
const CHANNEL: &str = "ch_ops";
const CHANNEL_FOLDER: &str = "ops-channel";
const REPO: &str = "api";

struct World {
    daemon: Daemon,
    /// The session's clone of `REPO`.
    clone: PathBuf,
    shared: PathBuf,
    channel: PathBuf,
    main: PathBuf,
    /// The folder every other folder is in, which the daemon may not leave.
    outside: PathBuf,
}

/// A daemon that knows one session and one channel, with their folders.
async fn world() -> World {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: CHANNEL.to_owned(),
            machine: String::new(),
            title: "ops".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "ops".to_owned(),
            folder: CHANNEL_FOLDER.to_owned(),
            branch_prefix: String::new(),
            default_agent: "zsh".to_owned(),
            repos: vec![ChannelRepo {
                owner: "acme".to_owned(),
                name: REPO.to_owned(),
                main: MainState::Ready,
            }],
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let clone = dir.path().join(REPO);
    let channel = dir.path().join("Humata").join(CHANNEL_FOLDER);
    let shared = channel.join("shared");
    let main = channel.join("main").join(REPO);
    for path in [&clone, &shared, &main, &channel.join("sessions")] {
        std::fs::create_dir_all(path).unwrap();
    }
    let outside = dir.path().to_owned();
    World {
        daemon: Daemon::start_in(dir).await,
        clone,
        shared,
        channel,
        main,
        outside,
    }
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

fn repo() -> pb::RepoRef {
    pb::RepoRef {
        owner: "acme".to_owned(),
        name: REPO.to_owned(),
    }
}

fn channel_id() -> Option<pb::ChannelId> {
    Some(pb::ChannelId {
        value: CHANNEL.to_owned(),
    })
}

fn at(root: pb::file_location::Root, path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(root),
        path: path.to_owned(),
    }
}

fn in_clone(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::SessionClone(pb::SessionClone {
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
            repo: Some(repo()),
        }),
        path,
    )
}

fn in_shared(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::ChannelShared(pb::ChannelShared {
            channel: channel_id(),
        }),
        path,
    )
}

fn in_channel(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::ChannelFolder(pb::ChannelFolder {
            channel: channel_id(),
        }),
        path,
    )
}

/// A path under the user's home folder, which holds Humata's folders.
fn in_home(path: &str) -> pb::FileLocation {
    at(pb::file_location::Root::Home(pb::MachineHome {}), path)
}

/// FC-2: the refusal a change to a main clone gets.
fn main_clone_refused(status: &Status) {
    assert!(
        matches!(error_of(status).kind, Some(pb::error::Kind::MainCloneReadOnly(_))),
        "{status:?}"
    );
}

fn in_main(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::MainClone(pb::MainClone {
            channel: channel_id(),
            repo: Some(repo()),
        }),
        path,
    )
}

async fn list(
    daemon: &Daemon,
    location: pb::FileLocation,
    page: Option<pb::PageRequest>,
) -> Result<pb::ListDirectoryResponse, Status> {
    files(daemon)
        .await
        .list_directory(pb::ListDirectoryRequest {
            location: Some(location),
            page,
        })
        .await
        .map(tonic::Response::into_inner)
}

/// The last path segment of each entry, in the order listed.
fn names(response: &pb::ListDirectoryResponse) -> Vec<String> {
    response
        .entries
        .iter()
        .map(|entry| {
            let path = &entry.location.as_ref().unwrap().path;
            path.rsplit('/').next().unwrap().to_owned()
        })
        .collect()
}

async fn stat(daemon: &Daemon, location: pb::FileLocation) -> Result<pb::FileEntry, Status> {
    files(daemon)
        .await
        .stat(pb::StatRequest {
            location: Some(location),
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn mkdir(daemon: &Daemon, location: pb::FileLocation) -> Result<pb::FileEntry, Status> {
    files(daemon)
        .await
        .create_folder(pb::CreateFolderRequest {
            idempotency_key: None,
            location: Some(location),
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn rename(
    daemon: &Daemon,
    location: pb::FileLocation,
    new_name: &str,
) -> Result<pb::FileEntry, Status> {
    files(daemon)
        .await
        .rename(pb::RenameRequest {
            idempotency_key: None,
            location: Some(location),
            new_name: new_name.to_owned(),
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn trash(
    daemon: &Daemon,
    locations: Vec<pb::FileLocation>,
) -> Result<pb::MoveToTrashResponse, Status> {
    files(daemon)
        .await
        .move_to_trash(pb::MoveToTrashRequest {
            idempotency_key: None,
            locations,
        })
        .await
        .map(tonic::Response::into_inner)
}

fn is_outside(status: &Status) -> bool {
    status.code() == Code::PermissionDenied
        && matches!(
            error_of(status).kind,
            Some(pb::error::Kind::OutsideHumataFolders(_))
        )
}

fn is_name_taken(status: &Status) -> bool {
    status.code() == Code::AlreadyExists
        && matches!(error_of(status).kind, Some(pb::error::Kind::NameTaken(_)))
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
}

#[tokio::test]
async fn a_listing_shows_folders_first_and_hides_transfer_leftovers() {
    let world = world().await;
    std::fs::create_dir(world.shared.join("zdir")).unwrap();
    std::fs::create_dir(world.shared.join("adir")).unwrap();
    write(&world.shared.join("b.txt"), "b");
    write(&world.shared.join("A.txt"), "aaa");
    write(&world.shared.join(".humata-transfer-x"), "x");
    write(&world.shared.join(".A.txt.humata-partial-1234"), "x");

    let listing = list(&world.daemon, in_shared(""), None).await.unwrap();

    assert_eq!(names(&listing), ["adir", "zdir", "A.txt", "b.txt"]);
    let kinds: Vec<i32> = listing.entries.iter().map(|entry| entry.kind).collect();
    assert_eq!(kinds[0], pb::file_entry::Kind::Folder as i32);
    assert_eq!(kinds[2], pb::file_entry::Kind::File as i32);
    assert_eq!(listing.entries[2].size, 3);
    assert_eq!(listing.entries[0].size, 0);
    assert!(listing.entries.iter().all(|entry| entry.writable && !entry.changed));
    assert_eq!(listing.entries[2].location.as_ref().unwrap().path, "A.txt");
}

#[tokio::test]
async fn a_long_listing_pages_without_repeats_or_gaps() {
    let world = world().await;
    std::fs::create_dir(world.shared.join("sub")).unwrap();
    for number in 0..1200 {
        write(&world.shared.join(format!("file-{number:04}.txt")), "x");
    }

    let mut seen = Vec::new();
    let mut token = String::new();
    let mut pages = 0;
    loop {
        let page = pb::PageRequest {
            page_size: 500,
            page_token: token.clone(),
        };
        let listing = list(&world.daemon, in_shared(""), Some(page)).await.unwrap();
        assert!(listing.entries.len() <= 500);
        seen.extend(names(&listing));
        pages += 1;
        token = listing.page.unwrap().next_page_token;
        if token.is_empty() {
            break;
        }
        assert!(pages < 10, "the pages never ended");
    }

    assert_eq!(pages, 3);
    let mut expected = vec!["sub".to_owned()];
    expected.extend((0..1200).map(|number| format!("file-{number:04}.txt")));
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn stat_describes_a_file_a_folder_and_the_root() {
    let world = world().await;
    write(&world.shared.join("notes.md"), "twelve bytes");
    std::fs::create_dir(world.shared.join("docs")).unwrap();

    let file = stat(&world.daemon, in_shared("notes.md")).await.unwrap();
    let docs = stat(&world.daemon, in_shared("docs")).await.unwrap();
    let root = stat(&world.daemon, in_shared("")).await.unwrap();

    assert_eq!(file.kind, pb::file_entry::Kind::File as i32);
    assert_eq!(file.size, 12);
    assert!(file.modified_at.is_some());
    assert_eq!(docs.kind, pb::file_entry::Kind::Folder as i32);
    assert_eq!(root.kind, pb::file_entry::Kind::Folder as i32);
    let missing = stat(&world.daemon, in_shared("nothing")).await.unwrap_err();
    assert_eq!(common::not_found(&missing), pb::EntityKind::File);
}

#[tokio::test]
async fn nothing_can_leave_its_root_by_dots_or_a_symlink() {
    let world = world().await;
    std::fs::create_dir(world.outside.join("elsewhere")).unwrap();
    write(&world.outside.join("elsewhere").join("secret.txt"), "s");
    std::os::unix::fs::symlink(world.outside.join("elsewhere"), world.shared.join("out")).unwrap();
    let daemon = &world.daemon;

    // Whatever reads or lists follows the link, so the link is refused too.
    for path in ["../elsewhere", "out", "out/secret.txt"] {
        assert!(is_outside(&list(daemon, in_shared(path), None).await.unwrap_err()), "list {path}");
        assert!(is_outside(&stat(daemon, in_shared(path)).await.unwrap_err()), "stat {path}");
    }
    // Nothing reached through the link, or by "..", can be renamed or deleted.
    for path in ["../elsewhere", "out/secret.txt"] {
        assert!(is_outside(&rename(daemon, in_shared(path), "x").await.unwrap_err()), "rename {path}");
        assert!(is_outside(&trash(daemon, vec![in_shared(path)]).await.unwrap_err()), "trash {path}");
    }
    for path in ["../new", "out/new"] {
        assert!(is_outside(&mkdir(daemon, in_shared(path)).await.unwrap_err()), "mkdir {path}");
    }
    assert!(!world.outside.join("elsewhere").join("new").exists());
    assert!(world.outside.join("elsewhere").join("secret.txt").exists());
}

#[tokio::test]
async fn a_link_itself_can_be_renamed_and_deleted_without_touching_what_it_points_to() {
    let world = world().await;
    std::fs::create_dir(world.outside.join("elsewhere")).unwrap();
    write(&world.outside.join("elsewhere").join("secret.txt"), "s");
    std::os::unix::fs::symlink(world.outside.join("elsewhere"), world.shared.join("out")).unwrap();
    let daemon = &world.daemon;

    let renamed = rename(daemon, in_shared("out"), "kept").await.unwrap();

    assert_eq!(renamed.kind, pb::file_entry::Kind::Symlink as i32);
    assert!(std::fs::symlink_metadata(world.shared.join("out")).is_err());
    assert!(
        std::fs::symlink_metadata(world.shared.join("kept"))
            .unwrap()
            .file_type()
            .is_symlink()
    );

    trash(daemon, vec![in_shared("kept")]).await.unwrap();

    assert!(std::fs::symlink_metadata(world.shared.join("kept")).is_err());
    assert!(world.outside.join("elsewhere").join("secret.txt").exists());
}

/// CreateFolder with an idempotency key.
async fn mkdir_keyed(
    daemon: &Daemon,
    location: pb::FileLocation,
    key: &str,
) -> Result<pb::FileEntry, Status> {
    files(daemon)
        .await
        .create_folder(pb::CreateFolderRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: key.to_owned(),
            }),
            location: Some(location),
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test]
async fn a_folder_is_made_once_and_a_taken_name_is_refused() {
    let world = world().await;
    write(&world.shared.join("taken"), "a file");

    let first = mkdir(&world.daemon, in_shared("plans")).await.unwrap();
    let again = mkdir(&world.daemon, in_shared("plans")).await.unwrap_err();
    let blocked = mkdir(&world.daemon, in_shared("taken")).await.unwrap_err();
    let no_parent = mkdir(&world.daemon, in_shared("missing/child")).await.unwrap_err();

    assert_eq!(first.kind, pb::file_entry::Kind::Folder as i32);
    assert!(is_name_taken(&again), "a folder with the name is there already");
    assert!(world.shared.join("plans").is_dir());
    assert!(is_name_taken(&blocked));
    assert_eq!(common::not_found(&no_parent), pb::EntityKind::File);
    assert!(!world.shared.join("missing").exists());
}

#[tokio::test]
async fn a_retry_with_the_same_key_gets_the_folder_it_made() {
    let world = world().await;

    let first = mkdir_keyed(&world.daemon, in_shared("notes"), "key-1").await.unwrap();
    let retried = mkdir_keyed(&world.daemon, in_shared("notes"), "key-1").await.unwrap();
    let other = mkdir_keyed(&world.daemon, in_shared("notes"), "key-2").await.unwrap_err();

    assert_eq!(first.location, retried.location);
    assert!(is_name_taken(&other));
}

#[tokio::test]
async fn rename_changes_the_name_and_never_replaces_or_moves() {
    let world = world().await;
    write(&world.shared.join("old.txt"), "old");
    write(&world.shared.join("other.txt"), "other");
    std::fs::create_dir(world.shared.join("dir")).unwrap();

    let taken = rename(&world.daemon, in_shared("old.txt"), "other.txt").await.unwrap_err();
    let slash = rename(&world.daemon, in_shared("old.txt"), "dir/new.txt").await.unwrap_err();
    let dots = rename(&world.daemon, in_shared("old.txt"), "..").await.unwrap_err();
    let empty = rename(&world.daemon, in_shared("old.txt"), "").await.unwrap_err();
    let root = rename(&world.daemon, in_shared(""), "x").await.unwrap_err();

    assert!(is_name_taken(&taken));
    assert_eq!(invalid_field(&slash).0, "new_name");
    assert_eq!(invalid_field(&dots).0, "new_name");
    assert_eq!(invalid_field(&empty).0, "new_name");
    assert_eq!(root.code(), Code::InvalidArgument);
    assert_eq!(std::fs::read_to_string(world.shared.join("old.txt")).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(world.shared.join("other.txt")).unwrap(), "other");

    let renamed = rename(&world.daemon, in_shared("dir"), "folder").await.unwrap();
    assert_eq!(renamed.location.unwrap().path, "folder");
    assert_eq!(renamed.kind, pb::file_entry::Kind::Folder as i32);
    assert!(world.shared.join("folder").is_dir());
    assert!(!world.shared.join("dir").exists());
}

#[tokio::test]
async fn a_main_clone_and_the_folders_holding_it_cannot_be_changed_through_another_root() {
    let world = world().await;
    write(&world.main.join("README.md"), "reference");
    let daemon = &world.daemon;
    let channel = format!("Humata/{CHANNEL_FOLDER}");
    let main = format!("{channel}/main/{REPO}");
    let readme = format!("{main}/README.md");

    // Inside it, through the home folder: a rename, a delete, a new folder.
    main_clone_refused(&rename(daemon, in_home(&readme), "x.md").await.unwrap_err());
    main_clone_refused(&trash(daemon, vec![in_home(&readme)]).await.unwrap_err());
    main_clone_refused(&mkdir(daemon, in_home(&format!("{main}/new"))).await.unwrap_err());
    // The clone's own folder and every folder that holds it.
    for holder in [main.clone(), format!("{channel}/main"), channel.clone(), "Humata".to_owned()] {
        main_clone_refused(&rename(daemon, in_home(&holder), "gone").await.unwrap_err());
        main_clone_refused(&trash(daemon, vec![in_home(&holder)]).await.unwrap_err());
    }
    // The same through the channel's folder.
    main_clone_refused(&trash(daemon, vec![in_channel("main")]).await.unwrap_err());
    let inside = in_channel(&format!("main/{REPO}/README.md"));
    main_clone_refused(&rename(daemon, inside, "x.md").await.unwrap_err());

    assert_eq!(std::fs::read_to_string(world.main.join("README.md")).unwrap(), "reference");
    assert!(!world.main.join("new").exists());
    assert!(world.channel.join("main").is_dir());
}

#[tokio::test]
async fn through_the_home_folder_a_main_clone_is_listed_and_its_neighbours_still_change() {
    let world = world().await;
    write(&world.main.join("README.md"), "reference");
    let daemon = &world.daemon;
    let channel = format!("Humata/{CHANNEL_FOLDER}");

    let listing = list(daemon, in_home(&format!("{channel}/main/{REPO}")), None).await.unwrap();
    let made = mkdir(daemon, in_home(&format!("{channel}/notes"))).await.unwrap();
    rename(daemon, in_home(&format!("{channel}/notes")), "kept").await.unwrap();

    assert_eq!(names(&listing), ["README.md"]);
    assert_eq!(made.kind, pb::file_entry::Kind::Folder as i32);
    assert!(world.channel.join("kept").is_dir());
    trash(daemon, vec![in_home(&format!("{channel}/kept"))]).await.unwrap();
    assert!(!world.channel.join("kept").exists());
}

#[tokio::test]
async fn a_main_clone_can_be_listed_but_never_changed() {
    let world = world().await;
    write(&world.main.join("README.md"), "reference");
    std::fs::create_dir(world.main.join("src")).unwrap();

    let listing = list(&world.daemon, in_main(""), None).await.unwrap();

    assert_eq!(names(&listing), ["src", "README.md"]);
    assert!(listing.entries.iter().all(|entry| !entry.writable));
    let created = mkdir(&world.daemon, in_main("new")).await.unwrap_err();
    let renamed = rename(&world.daemon, in_main("README.md"), "x.md").await.unwrap_err();
    let trashed = trash(&world.daemon, vec![in_main("README.md")]).await.unwrap_err();
    for status in [created, renamed, trashed] {
        assert_eq!(status.code(), Code::PermissionDenied);
        assert!(matches!(
            error_of(&status).kind,
            Some(pb::error::Kind::MainCloneReadOnly(_))
        ));
    }
    assert!(world.main.join("README.md").exists());
    assert!(!world.main.join("new").exists());
}

#[tokio::test]
async fn a_clone_a_channel_folder_and_shared_all_take_changes() {
    let world = world().await;
    let roots: [(fn(&str) -> pb::FileLocation, &Path); 3] = [
        (in_clone, &world.clone),
        (in_channel, &world.channel),
        (in_shared, &world.shared),
    ];
    for (location, folder) in roots {
        let made = mkdir(&world.daemon, location("made")).await.unwrap();
        assert_eq!(made.kind, pb::file_entry::Kind::Folder as i32);
        write(&folder.join("made").join("a.txt"), "a");

        let listing = list(&world.daemon, location("made"), None).await.unwrap();
        assert_eq!(names(&listing), ["a.txt"]);
        rename(&world.daemon, location("made/a.txt"), "b.txt").await.unwrap();
        assert!(folder.join("made").join("b.txt").exists());
        let moved = trash(&world.daemon, vec![location("made/b.txt")]).await.unwrap();
        assert_eq!(moved.items.len(), 1);
        assert!(!folder.join("made").join("b.txt").exists());
    }
}

#[tokio::test]
async fn trashing_the_root_or_nothing_is_refused_and_a_bad_path_moves_none() {
    let world = world().await;
    write(&world.shared.join("keep.txt"), "keep");

    let root = trash(&world.daemon, vec![in_shared("")]).await.unwrap_err();
    let none = trash(&world.daemon, Vec::new()).await.unwrap_err();
    let partly = trash(&world.daemon, vec![in_shared("keep.txt"), in_shared("missing.txt")])
        .await
        .unwrap_err();

    assert_eq!(root.code(), Code::InvalidArgument);
    assert_eq!(none.code(), Code::InvalidArgument);
    assert_eq!(common::not_found(&partly), pb::EntityKind::File);
    assert!(world.shared.join("keep.txt").exists());
}

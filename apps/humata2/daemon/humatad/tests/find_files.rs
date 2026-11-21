//! FileService.FindFiles through `humatad2`'s gRPC API: "Go to file…" across
//! a session's repos, ranked, paged, and limited to the files git knows or
//! would add (RV-9, RV-11, RV-14).

use humatad_test_common as common;

use std::fs;
use std::path::Path;

use common::{Daemon, folder, invalid_field, not_found, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionDetail, SessionRecord, SessionRepo, StartStep, Store};
use tonic::transport::Channel;
use tonic::{Code, Status};

const SESSION: &str = "se_find";
const EMPTY: &str = "se_find_empty";
const PLAIN: &str = "se_find_plain";
const DESK: &str = "se_find_desk";
const ROOT_FILES: &str = "se_find_root";

struct World {
    daemon: Daemon,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn git_in(dir: &Path, args: &[&str]) {
    let mut all = vec!["-C", dir.to_str().unwrap()];
    all.extend_from_slice(args);
    common::git(&all);
}

/// A git repository with these files committed, then an untracked file and
/// an ignored `target/` folder.
fn repo_with(dir: &Path, tracked: &[&str]) {
    fs::create_dir_all(dir).unwrap();
    git_in(dir, &["init", "-b", "main"]);
    write(&dir.join(".gitignore"), "target/\n");
    let mut add = vec!["add", ".gitignore"];
    for path in tracked {
        write(&dir.join(path), "x\n");
        add.push(path);
    }
    git_in(dir, &add);
    git_in(dir, &["commit", "-m", "one"]);
    write(&dir.join("notes.txt"), "untracked\n");
    write(&dir.join("target/lib.o"), "ignored\n");
}

fn record(id: &str, folder: &Path) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_find".to_owned(),
        machine: String::new(),
        title: id.to_owned(),
        agent: "zsh".to_owned(),
        folder: folder.to_owned(),
        created_unix_ms: 0,
    }
}

fn detail(id: &str, repos: &[&str]) -> SessionDetail {
    SessionDetail {
        session: id.to_owned(),
        repos: repos
            .iter()
            .map(|name| SessionRepo {
                owner: "acme".to_owned(),
                name: (*name).to_owned(),
                branch: "main".to_owned(),
                path: Path::new(name).to_owned(),
            })
            .collect(),
        model: None,
        first_message: None,
        start: StartStep::Done,
        operation: None,
        revision: 1,
    }
}

/// A daemon that knows a session with two clones, `web` then `api`; a
/// session with no repos; and a session whose clone isn't a git repository.
async fn world() -> &'static World {
    static WORLD: tokio::sync::OnceCell<World> = tokio::sync::OnceCell::const_new();
    WORLD.get_or_init(build_world).await
}

async fn build_world() -> World {
    let dir = folder();
    let session_dir = dir.path().join("main-session");
    let empty_dir = dir.path().join("empty-session");
    fs::create_dir_all(&empty_dir).unwrap();
    let plain_parent = dir.path().join("plain-session");
    let desk = dir.path().join("desk-session");
    let root_session = dir.path().join("root-session");
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store.put_session(&record(SESSION, &session_dir)).unwrap();
    store.put_session_detail(&detail(SESSION, &["web", "api"])).unwrap();
    store.put_session(&record(EMPTY, &empty_dir)).unwrap();
    store.put_session_detail(&detail(EMPTY, &[])).unwrap();
    store.put_session(&record(PLAIN, &plain_parent)).unwrap();
    store.put_session_detail(&detail(PLAIN, &["docs"])).unwrap();
    store
        .put_session(&SessionRecord {
            channel: "desk".to_owned(),
            ..record(DESK, &desk)
        })
        .unwrap();
    store.put_session_detail(&detail(DESK, &["ledger"])).unwrap();
    store.put_session(&record(ROOT_FILES, &root_session)).unwrap();
    store.put_session_detail(&detail(ROOT_FILES, &["api"])).unwrap();
    drop(store);

    write(&desk.join("rfc.md"), "# RFC\n");
    write(&desk.join("notes/bench.md"), "bench\n");
    repo_with(&desk.join("ledger"), &["schema.sql"]);

    write(&root_session.join("notes.md"), "# Session notes\n");
    write(&root_session.join("scratch/plan.md"), "plan\n");
    repo_with(&root_session.join("api"), &["src/lib.rs"]);

    repo_with(
        &session_dir.join("api"),
        &[
            "README.md",
            "src/lib.rs",
            "src/library_notes.md",
            "src/domain.rs",
            "src/deep/main.rs",
        ],
    );
    repo_with(
        &session_dir.join("web"),
        &["package.json", "src/app.ts", "src/lib/index.ts"],
    );
    write(&plain_parent.join("docs/a.txt"), "a\n");
    write(&plain_parent.join("docs/sub/b.txt"), "b\n");
    World {
        daemon: Daemon::start_in(dir).await,
    }
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

async fn find_in(
    daemon: &Daemon,
    session: &str,
    query: &str,
    page: Option<pb::PageRequest>,
) -> Result<pb::FindFilesResponse, Status> {
    files(daemon)
        .await
        .find_files(pb::FindFilesRequest {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            query: query.to_owned(),
            page,
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn find(daemon: &Daemon, query: &str) -> Vec<String> {
    shown(&find_in(daemon, SESSION, query, None).await.unwrap())
}

/// `repo:path` of each entry, in the order returned.
fn shown(response: &pb::FindFilesResponse) -> Vec<String> {
    response
        .entries
        .iter()
        .map(|entry| {
            let location = entry.location.as_ref().unwrap();
            let Some(pb::file_location::Root::SessionClone(clone)) = &location.root else {
                panic!("not a session clone: {location:?}");
            };
            format!("{}:{}", clone.repo.as_ref().unwrap().name, location.path)
        })
        .collect()
}

fn page(size: u32, token: &str) -> Option<pb::PageRequest> {
    Some(pb::PageRequest {
        page_size: size,
        page_token: token.to_owned(),
    })
}

#[tokio::test]
async fn a_query_finds_matching_paths_in_each_repo_in_the_sessions_order() {
    let world = world().await;
    assert_eq!(
        find(&world.daemon, "lib").await,
        [
            "web:src/lib/index.ts",
            "api:src/lib.rs",
            "api:src/library_notes.md",
        ]
    );
}

#[tokio::test]
async fn a_match_has_the_files_size_kind_and_clone_location() {
    let world = world().await;
    let response = find_in(&world.daemon, SESSION, "package.json", None)
        .await
        .unwrap();
    let [entry] = response.entries.as_slice() else {
        panic!("{response:?}");
    };
    assert_eq!(entry.kind(), pb::file_entry::Kind::File);
    assert_eq!(entry.size, 2);
    assert!(entry.writable);
    assert!(!entry.changed);
    assert!(entry.modified_at.is_some());
}

#[tokio::test]
async fn ignored_files_and_git_internals_never_show_but_untracked_files_do() {
    let world = world().await;
    let all = find(&world.daemon, "").await;
    assert!(all.contains(&"api:notes.txt".to_owned()), "{all:?}");
    assert!(all.contains(&"api:.gitignore".to_owned()), "{all:?}");
    assert!(
        !all.iter().any(|path| path.contains("target/") || path.contains(".git/")),
        "{all:?}"
    );
    assert!(find(&world.daemon, "lib.o").await.is_empty());
}

#[tokio::test]
async fn a_few_characters_in_order_find_the_path() {
    let world = world().await;
    let found = find(&world.daemon, "srcli").await;
    assert!(found.contains(&"api:src/lib.rs".to_owned()), "{found:?}");
    assert!(!found.contains(&"api:README.md".to_owned()), "{found:?}");
    let upper = find(&world.daemon, "SRCLI").await;
    assert_eq!(found, upper);
}

#[tokio::test]
async fn an_exact_file_name_comes_before_a_looser_match() {
    let world = world().await;
    assert_eq!(
        find(&world.daemon, "main.rs").await,
        ["api:src/deep/main.rs", "api:src/domain.rs"]
    );
}

#[tokio::test]
async fn paging_an_empty_query_visits_every_file_once() {
    let world = world().await;
    let everything = find(&world.daemon, "").await;
    assert!(everything.len() > 6, "{everything:?}");

    let mut seen = Vec::new();
    let mut token = String::new();
    let mut pages = 0;
    loop {
        let response = find_in(&world.daemon, SESSION, "", page(2, &token))
            .await
            .unwrap();
        assert!(response.entries.len() <= 2);
        seen.extend(shown(&response));
        pages += 1;
        assert!(pages < 50, "paging doesn't end");
        token = response.page.unwrap().next_page_token;
        if token.is_empty() {
            break;
        }
    }
    assert!(pages > 1);
    assert_eq!(seen, everything);
}

#[tokio::test]
async fn a_session_with_no_repos_has_no_matches() {
    let world = world().await;
    let response = find_in(&world.daemon, EMPTY, "", None).await.unwrap();
    assert!(response.entries.is_empty());
    assert!(response.page.unwrap().next_page_token.is_empty());
}

#[tokio::test]
async fn an_unknown_session_is_not_found() {
    let world = world().await;
    let error = find_in(&world.daemon, "se_nope", "x", None).await.unwrap_err();
    assert_eq!(not_found(&error), pb::EntityKind::Session);
}

#[tokio::test]
async fn a_clone_that_is_not_a_git_repository_still_lists_its_files() {
    let world = world().await;
    let response = find_in(&world.daemon, PLAIN, "", None).await.unwrap();
    let mut found = shown(&response);
    found.sort();
    assert_eq!(found, ["docs:a.txt", "docs:sub/b.txt"]);
}

#[tokio::test]
async fn a_query_that_is_too_long_is_refused() {
    let world = world().await;
    let query = "a".repeat(257);
    let error = find_in(&world.daemon, SESSION, &query, None)
        .await
        .unwrap_err();
    let (field, problem) = invalid_field(&error);
    assert_eq!(field, "query");
    assert_eq!(problem, pb::invalid_field::Problem::OutOfRange);
    assert_eq!(error.code(), Code::InvalidArgument);
}

/// D382: a Desk session's own folder holds its documents, outside every
/// clone. They are found first, under the folder's root, and the clone in
/// the folder is found once, as a clone.
#[tokio::test]
async fn a_desk_sessions_own_files_are_found_before_its_clones_and_only_once() {
    let world = world().await;
    let response = find_in(&world.daemon, DESK, "", None).await.unwrap();
    let found: Vec<String> = response
        .entries
        .iter()
        .map(|entry| {
            let location = entry.location.as_ref().unwrap();
            match &location.root {
                Some(pb::file_location::Root::SessionFolder(_)) => format!("folder:{}", location.path),
                Some(pb::file_location::Root::SessionClone(clone)) => {
                    format!("{}:{}", clone.repo.as_ref().unwrap().name, location.path)
                }
                other => panic!("an unexpected root: {other:?}"),
            }
        })
        .collect();
    let mut own: Vec<&String> = found.iter().filter(|path| path.starts_with("folder:")).collect();
    own.sort();
    assert_eq!(own, ["folder:notes/bench.md", "folder:rfc.md"], "{found:?}");
    assert!(found[0].starts_with("folder:") && found[1].starts_with("folder:"), "{found:?}");
    assert!(found.contains(&"ledger:schema.sql".to_owned()), "{found:?}");
    assert!(
        !found.iter().any(|path| path.starts_with("folder:ledger")),
        "the clone isn't listed again through the folder: {found:?}"
    );
}

#[tokio::test]
async fn a_channel_sessions_root_files_are_found_under_session_folder_without_duplicating_clones() {
    let world = world().await;
    let response = find_in(&world.daemon, ROOT_FILES, "", None).await.unwrap();
    let found: Vec<String> = response
        .entries
        .iter()
        .map(|entry| {
            let location = entry.location.as_ref().unwrap();
            match &location.root {
                Some(pb::file_location::Root::SessionFolder(_)) => {
                    format!("folder:{}", location.path)
                }
                Some(pb::file_location::Root::SessionClone(clone)) => {
                    format!("{}:{}", clone.repo.as_ref().unwrap().name, location.path)
                }
                other => panic!("an unexpected root: {other:?}"),
            }
        })
        .collect();
    let mut own: Vec<&String> = found
        .iter()
        .filter(|path| path.starts_with("folder:"))
        .collect();
    own.sort();
    assert_eq!(own, ["folder:notes.md", "folder:scratch/plan.md"], "{found:?}");
    assert!(found.contains(&"api:src/lib.rs".to_owned()), "{found:?}");
    assert!(
        !found.iter().any(|path| path.starts_with("folder:api")),
        "clone files must not be duplicated under SessionFolder: {found:?}"
    );
}

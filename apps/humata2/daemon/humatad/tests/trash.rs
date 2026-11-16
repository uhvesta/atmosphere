//! The Humata trash through `humatad2`'s gRPC API: files deleted in the
//! file manager (FM-3) can be listed, restored and emptied (LD-6, LD-7).

mod common;

use std::path::{Path, PathBuf};

use common::{Daemon, error_of, folder, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, Store};
use tonic::transport::Channel;
use tonic::{Code, Status};

const CHANNEL: &str = "ch_trash";
const CHANNEL_FOLDER: &str = "trash-channel";
const THIRTY_DAYS_SECONDS: i64 = 30 * 24 * 60 * 60;

/// A daemon that knows one channel; `shared` is its `shared/` folder.
async fn world() -> (Daemon, PathBuf) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "trash".to_owned(),
            folder: CHANNEL_FOLDER.to_owned(),
            branch_prefix: String::new(),
            default_agent: "zsh".to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let shared = dir.path().join("Humata").join(CHANNEL_FOLDER).join("shared");
    std::fs::create_dir_all(&shared).unwrap();
    (Daemon::start_in(dir).await, shared)
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

async fn bin(daemon: &Daemon) -> pb::trash_service_client::TrashServiceClient<Channel> {
    pb::trash_service_client::TrashServiceClient::new(daemon.channel().await)
}

fn shared(path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
            channel: Some(pb::ChannelId {
                value: CHANNEL.to_owned(),
            }),
        })),
        path: path.to_owned(),
    }
}

async fn trash(daemon: &Daemon, paths: &[&str]) -> Vec<pb::TrashItemId> {
    files(daemon)
        .await
        .move_to_trash(pb::MoveToTrashRequest {
            idempotency_key: None,
            locations: paths.iter().map(|path| shared(path)).collect(),
        })
        .await
        .unwrap()
        .into_inner()
        .items
}

async fn listed(daemon: &Daemon) -> pb::ListTrashResponse {
    bin(daemon)
        .await
        .list_trash(pb::ListTrashRequest::default())
        .await
        .unwrap()
        .into_inner()
}

async fn restore(daemon: &Daemon, id: &pb::TrashItemId) -> Result<pb::RestoreFromTrashResponse, Status> {
    bin(daemon)
        .await
        .restore_from_trash(pb::RestoreFromTrashRequest {
            idempotency_key: None,
            id: Some(id.clone()),
        })
        .await
        .map(tonic::Response::into_inner)
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
}

#[tokio::test]
async fn a_trashed_file_leaves_its_folder_and_comes_back_whole() {
    let (daemon, folder) = world().await;
    write(&folder.join("notes.txt"), "hello trash");

    let ids = trash(&daemon, &["notes.txt"]).await;

    assert_eq!(ids.len(), 1);
    assert!(!folder.join("notes.txt").exists());
    let list = listed(&daemon).await;
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.total_bytes, 11);
    let item = &list.items[0];
    assert_eq!(item.id, Some(ids[0].clone()));
    assert_eq!(item.bytes, 11);
    assert!(!item.description.is_empty());
    let deleted = item.deleted_at.as_ref().unwrap().seconds;
    let empties = item.empties_at.as_ref().unwrap().seconds;
    assert_eq!(empties - deleted, THIRTY_DAYS_SECONDS);
    assert!(item.deleted_by.is_some());
    assert!(matches!(
        item.was,
        Some(pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(_))
        })
    ));

    let restored = restore(&daemon, &ids[0]).await.unwrap();

    assert!(restored.restored.is_some());
    assert_eq!(std::fs::read_to_string(folder.join("notes.txt")).unwrap(), "hello trash");
    assert!(listed(&daemon).await.items.is_empty());
}

#[tokio::test]
async fn restoring_onto_a_path_something_else_took_keeps_both() {
    let (daemon, folder) = world().await;
    write(&folder.join("plan.md"), "the old one");
    let ids = trash(&daemon, &["plan.md"]).await;
    write(&folder.join("plan.md"), "the new one");

    let refused = restore(&daemon, &ids[0]).await.unwrap_err();

    assert_eq!(refused.code(), Code::AlreadyExists);
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::NameTaken(_))));
    assert_eq!(std::fs::read_to_string(folder.join("plan.md")).unwrap(), "the new one");
    assert_eq!(listed(&daemon).await.items.len(), 1);

    std::fs::rename(folder.join("plan.md"), folder.join("plan-new.md")).unwrap();
    restore(&daemon, &ids[0]).await.unwrap();
    assert_eq!(std::fs::read_to_string(folder.join("plan.md")).unwrap(), "the old one");
}

#[tokio::test]
async fn restoring_an_unknown_item_is_not_found() {
    let (daemon, _folder) = world().await;

    let missing = restore(
        &daemon,
        &pb::TrashItemId {
            value: "nothing".to_owned(),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(common::not_found(&missing), pb::EntityKind::TrashItem);
}

#[tokio::test]
async fn a_trashed_folder_comes_back_with_everything_in_it() {
    let (daemon, folder) = world().await;
    std::fs::create_dir_all(folder.join("docs").join("deep")).unwrap();
    write(&folder.join("docs").join("a.txt"), "aa");
    write(&folder.join("docs").join("deep").join("b.txt"), "bbb");

    let ids = trash(&daemon, &["docs"]).await;

    assert!(!folder.join("docs").exists());
    assert_eq!(listed(&daemon).await.items[0].bytes, 5);
    restore(&daemon, &ids[0]).await.unwrap();
    assert_eq!(std::fs::read_to_string(folder.join("docs").join("a.txt")).unwrap(), "aa");
    assert_eq!(
        std::fs::read_to_string(folder.join("docs").join("deep").join("b.txt")).unwrap(),
        "bbb"
    );
}

#[tokio::test]
async fn emptying_frees_what_the_plan_said_and_a_token_works_once() {
    let (daemon, folder) = world().await;
    write(&folder.join("one.txt"), "1234");
    write(&folder.join("two.txt"), "123456");
    trash(&daemon, &["one.txt", "two.txt"]).await;

    let plan = bin(&daemon)
        .await
        .plan_empty_trash(pb::PlanEmptyTrashRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(plan.frees_bytes, 10);
    assert_eq!(plan.items, 2);
    let empty = |plan: &pb::EmptyTrashPlan| pb::EmptyTrashRequest {
        idempotency_key: None,
        machine: None,
        plan: plan.token.clone(),
        presence: None,
    };

    let freed = bin(&daemon).await.empty_trash(empty(&plan)).await.unwrap().into_inner();

    assert_eq!(freed.freed_bytes, plan.frees_bytes);
    let list = listed(&daemon).await;
    assert!(list.items.is_empty());
    assert_eq!(list.total_bytes, 0);
    let again = bin(&daemon).await.empty_trash(empty(&plan)).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn emptying_removes_only_what_was_planned() {
    let (daemon, folder) = world().await;
    write(&folder.join("before.txt"), "before");
    trash(&daemon, &["before.txt"]).await;
    let plan = bin(&daemon)
        .await
        .plan_empty_trash(pb::PlanEmptyTrashRequest::default())
        .await
        .unwrap()
        .into_inner();
    write(&folder.join("after.txt"), "after");
    let later = trash(&daemon, &["after.txt"]).await;

    let freed = bin(&daemon)
        .await
        .empty_trash(pb::EmptyTrashRequest {
            idempotency_key: None,
            machine: None,
            plan: plan.token,
            presence: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(freed.freed_bytes, 6);
    let list = listed(&daemon).await;
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].id, Some(later[0].clone()));
}

#[tokio::test]
async fn the_trash_survives_a_daemon_restart() {
    let (mut daemon, folder) = world().await;
    write(&folder.join("kept.txt"), "still here");
    let ids = trash(&daemon, &["kept.txt"]).await;

    daemon.restart().await;

    let list = listed(&daemon).await;
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].id, Some(ids[0].clone()));
    restore(&daemon, &ids[0]).await.unwrap();
    assert_eq!(std::fs::read_to_string(folder.join("kept.txt")).unwrap(), "still here");
}

#[tokio::test]
async fn trashing_with_the_same_key_twice_moves_once() {
    let (daemon, folder) = world().await;
    write(&folder.join("once.txt"), "once");
    let request = pb::MoveToTrashRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: "key-1".to_owned(),
        }),
        locations: vec![shared("once.txt")],
    };

    let first = files(&daemon).await.move_to_trash(request.clone()).await.unwrap().into_inner();
    let second = files(&daemon).await.move_to_trash(request).await.unwrap().into_inner();

    assert_eq!(first.items, second.items);
    assert_eq!(listed(&daemon).await.items.len(), 1);
}

#[tokio::test]
async fn a_long_trash_lists_in_pages_oldest_first() {
    let (daemon, folder) = world().await;
    let mut ids = Vec::new();
    for number in 0..5 {
        write(&folder.join(format!("f{number}.txt")), "x");
        let name = format!("f{number}.txt");
        ids.extend(trash(&daemon, &[name.as_str()]).await);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let mut seen = Vec::new();
    let mut token = String::new();
    loop {
        let page = bin(&daemon)
            .await
            .list_trash(pb::ListTrashRequest {
                machine: None,
                page: Some(pb::PageRequest {
                    page_size: 2,
                    page_token: token.clone(),
                }),
            })
            .await
            .unwrap()
            .into_inner();
        seen.extend(page.items.iter().map(|item| item.id.clone().unwrap()));
        token = page.page.unwrap().next_page_token;
        if token.is_empty() {
            break;
        }
    }

    assert_eq!(seen, ids);
}

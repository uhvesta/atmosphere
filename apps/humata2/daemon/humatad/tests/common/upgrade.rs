//! Releases for the UPGRADE tests (UP-1, D281 a): the `bundle.json` a
//! daemon reads which release it is from, written as the app's install
//! writes it, and waits on a daemon's Inbox for its Migrate items.
//!
//! A release is told by its bundle id, the sha256 of its `bundle.json`
//! (the tools bundle's `manifest.json`, installed). A daemon finds the file
//! in `bin/` beside its state folder: `~/.humata2/bin/bundle.json` on a
//! machine, `<test dir>/bin/bundle.json` for a test daemon.
//!
//! Use it as `mod common; #[path = "common/upgrade.rs"] mod upgrade;`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use sha2::{Digest, Sha256};
use tonic::Streaming;

use crate::common::{Daemon, binary};

/// The metadata a CLI names its release in.
pub const BUNDLE_HEADER: &str = "humata-bundle";

/// The most a test waits for something a daemon does on its own: a safety
/// net that fails the test saying what it waited for, never a pace. Every
/// wait here ends on a pushed Inbox change.
pub const SLOW: Duration = Duration::from_secs(120);

/// Every platform Humata ships, as a bundle names them.
const PLATFORMS: [&str; 4] = ["darwin-arm64", "darwin-x86_64", "linux-aarch64", "linux-x86_64"];

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The sha256 of the file at `path`, as a bundle writes it.
pub fn sha256_of(path: &Path) -> String {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("couldn't read {} to hash it: {error}", path.display()));
    hex(&Sha256::digest(&bytes))
}

/// The admin test daemon's sha256: what `humatad2` is on every daemon the
/// tests start with `Build::Admin`, a host and `Rig::start_seen`'s boxes
/// alike.
pub fn admin_sha() -> String {
    sha256_of(&binary("HUMATAD2"))
}

/// The release daemon's sha256: a build that is not the admin one.
pub fn release_sha() -> String {
    sha256_of(&binary("HUMATAD2_RELEASE"))
}

/// This test's platform, as a bundle names it.
pub fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        (os, arch) => panic!("Humata ships no bundle for {os}-{arch}"),
    }
}

/// A platform Humata ships that isn't this test's.
pub fn other_platform() -> &'static str {
    PLATFORMS
        .into_iter()
        .find(|name| *name != platform())
        .expect("Humata ships more than one platform")
}

/// One platform's entry: `humatad2` is `daemon_sha`, the holder and the CLIs
/// are the ones the tests run.
fn entry(platform: &str, daemon_sha: &str) -> serde_json::Value {
    entry_with(platform, daemon_sha, &sha256_of(&binary("HUMATA2_HOLDER")))
}

/// One platform's entry with `holder_sha` as its `humata2_holder`.
fn entry_with(platform: &str, daemon_sha: &str, holder_sha: &str) -> serde_json::Value {
    serde_json::json!({
        "humatad2": {"path": format!("{platform}/humatad2"), "sha256": daemon_sha},
        "humata2_holder": {"path": format!("{platform}/humata2_holder"), "sha256": holder_sha},
        "humata": {"path": format!("{platform}/humata"), "sha256": sha256_of(&binary("HUMATA"))},
        "stack": {"path": format!("{platform}/stack"), "sha256": sha256_of(&binary("STACK"))},
        "humata2_fusefs": {"path": format!("{platform}/humata2_fusefs"), "sha256": sha256_of(&binary("HUMATA2_FUSEFS"))},
    })
}

/// A `bundle.json` whose `daemon_platform` entry names `daemon_sha` as
/// `humatad2`. `release` tells releases apart: it is the sha of another
/// platform's daemon, so two releases differ in bytes, and so in id, while
/// this platform's entry stays the same.
pub fn manifest_on(daemon_platform: &str, daemon_sha: &str, release: &str) -> Vec<u8> {
    let mut platforms = serde_json::Map::new();
    platforms.insert(daemon_platform.to_owned(), entry(daemon_platform, daemon_sha));
    let elsewhere = PLATFORMS
        .into_iter()
        .find(|name| *name != daemon_platform && *name != platform())
        .expect("Humata ships more than two platforms");
    platforms.insert(
        elsewhere.to_owned(),
        entry(elsewhere, &hex(&Sha256::digest(release.as_bytes()))),
    );
    serde_json::to_vec_pretty(&serde_json::json!({ "platforms": platforms }))
        .expect("a manifest serializes")
}

/// Release `release` of this platform, with `daemon_sha` as its `humatad2`.
pub fn manifest(daemon_sha: &str, release: &str) -> Vec<u8> {
    manifest_on(platform(), daemon_sha, release)
}

/// As [`manifest`], with `holder_sha` as this platform's `humata2_holder`:
/// a release whose holder is another file than the tests' own (the
/// holders tests' release b ships `HUMATA2_HOLDER_FAULTS`).
pub fn manifest_with_holder(daemon_sha: &str, holder_sha: &str, release: &str) -> Vec<u8> {
    let mut platforms = serde_json::Map::new();
    platforms.insert(platform().to_owned(), entry_with(platform(), daemon_sha, holder_sha));
    let elsewhere = other_platform();
    platforms.insert(
        elsewhere.to_owned(),
        entry(elsewhere, &hex(&Sha256::digest(release.as_bytes()))),
    );
    serde_json::to_vec_pretty(&serde_json::json!({ "platforms": platforms }))
        .expect("a manifest serializes")
}

/// The bundle id of a `bundle.json`: the sha256 of its bytes.
pub fn bundle_id(bundle_json: &[u8]) -> String {
    hex(&Sha256::digest(bundle_json))
}

/// The `bin` folder beside `daemon`'s state, where it finds `bundle.json`.
pub fn bin_of(daemon: &Daemon) -> PathBuf {
    daemon
        .state
        .parent()
        .expect("a daemon's state folder is in a folder of its own")
        .join("bin")
}

/// `~/.humata2/bin` of the machine whose home is `home`.
pub fn machine_bin(home: &Path) -> PathBuf {
    home.join(".humata2").join("bin")
}

/// Writes `bundle_json` as `bin/bundle.json`, whole (a reader never sees a
/// part of it), and returns its bundle id.
pub fn install(bin: &Path, bundle_json: &[u8]) -> String {
    std::fs::create_dir_all(bin).unwrap_or_else(|error| panic!("couldn't make {}: {error}", bin.display()));
    let part = bin.join("bundle.json.part");
    std::fs::write(&part, bundle_json).unwrap_or_else(|error| panic!("couldn't write {}: {error}", part.display()));
    std::fs::rename(&part, bin.join("bundle.json"))
        .unwrap_or_else(|error| panic!("couldn't put bundle.json in {}: {error}", bin.display()));
    bundle_id(bundle_json)
}

/// `daemon`'s build as its ServerInfo reports it.
pub async fn build_of(daemon: &Daemon) -> pb::BuildInfo {
    crate::common::server_info(daemon)
        .await
        .build
        .expect("ServerInfo carries the daemon's build")
}

/// `message` with the metadata `headers`.
pub fn with_headers<T>(message: T, headers: &[(&'static str, &str)]) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    for (name, value) in headers {
        let value = tonic::metadata::MetadataValue::try_from(*value)
            .unwrap_or_else(|error| panic!("{name}: {value:?} isn't a header value: {error}"));
        request.metadata_mut().insert(*name, value);
    }
    request
}

/// The Migrate card of an Inbox item, if it is one.
pub fn migrate_of(item: &pb::InboxRequest) -> Option<&pb::MigratePreview> {
    match item.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Migrate(migrate)) => Some(migrate),
        _ => None,
    }
}

/// Whether `item` is a pending Migrate item for `thing` on `machine`.
pub fn pending_migrate(item: &pb::InboxRequest, thing: pb::MigrateThing, machine: &pb::MachineId) -> bool {
    item.kind() == pb::RequestKind::Migrate
        && item.state() == pb::RequestState::Pending
        && migrate_of(item).is_some_and(|migrate| {
            migrate.thing() == thing && migrate.machine.as_ref() == Some(machine)
        })
}

async fn inbox_feed(daemon: &Daemon) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Inbox as i32],
            ..Default::default()
        })
        .await
        .expect("the daemon's Inbox feed opens")
        .into_inner()
}

/// Every Inbox item `daemon` holds, answered ones too.
pub async fn inbox(daemon: &Daemon) -> Vec<pb::InboxRequest> {
    daemon
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest {
            include_answered: true,
            ..Default::default()
        })
        .await
        .expect("the daemon lists its Inbox")
        .into_inner()
        .requests
}

/// Reads `daemon`'s Inbox until `reached` finds what it looks for, and
/// returns that. Nothing sleeps: the Inbox is read again each time its feed
/// tells of a change, and the feed is opened before the first read, so no
/// change falls between.
pub async fn inbox_until<T>(
    daemon: &Daemon,
    what: &str,
    reached: impl Fn(&[pb::InboxRequest]) -> Option<T>,
) -> T {
    let mut feed = inbox_feed(daemon).await;
    let deadline = tokio::time::Instant::now() + SLOW;
    loop {
        let now = inbox(daemon).await;
        if let Some(found) = reached(&now) {
            return found;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the Inbox feed ended ({ended:?}) before {what}; the Inbox held {now:?}"),
            Err(_) => panic!("never {what} within {SLOW:?}; the Inbox held {now:?}"),
        }
    }
}

/// The pending Migrate item for `thing` on `machine`, once there is one.
pub async fn migrate_item(daemon: &Daemon, thing: pb::MigrateThing, machine: &pb::MachineId) -> pb::InboxRequest {
    inbox_until(daemon, "a Migrate item was pending", |items| {
        items
            .iter()
            .find(|item| pending_migrate(item, thing, machine))
            .cloned()
    })
    .await
}

/// The owner clicks Migrate on the pending `thing` item for `machine` in
/// `daemon`'s Inbox, once there is one. Only this Mac's daemon, CLI and host
/// tools migrate without the click (D892). Returns the item as it was clicked.
pub async fn approve_migrate(daemon: &Daemon, thing: pb::MigrateThing, machine: &pb::MachineId) -> pb::InboxRequest {
    let item = migrate_item(daemon, thing, machine).await;
    let id = item.id.clone().expect("a Migrate item has an id");
    daemon
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("migrate-{}-{}", id.value, item.revision.as_ref().map_or(0, |revision| revision.value)),
            }),
            id: Some(id),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .unwrap_or_else(|status| panic!("the owner's Migrate on {machine:?} is taken: {status:?}"));
    item
}

/// Item `id` as `daemon` shows it now.
pub async fn request(daemon: &Daemon, id: &pb::RequestId) -> pb::InboxRequest {
    daemon
        .inbox_service()
        .await
        .get_request(pb::GetRequestRequest { id: Some(id.clone()) })
        .await
        .unwrap_or_else(|status| panic!("the daemon shows request {}: {status:?}", id.value))
        .into_inner()
}

/// Item `id` once `reached` holds for it, read again on each Inbox change.
pub async fn request_until(
    daemon: &Daemon,
    id: &pb::RequestId,
    what: &str,
    reached: impl Fn(&pb::InboxRequest) -> bool,
) -> pb::InboxRequest {
    let mut feed = inbox_feed(daemon).await;
    let deadline = tokio::time::Instant::now() + SLOW;
    loop {
        let now = request(daemon, id).await;
        if reached(&now) {
            return now;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the Inbox feed ended ({ended:?}) before request {} {what}; it was {now:?}", id.value),
            Err(_) => panic!("request {} never {what} within {SLOW:?}; it was {now:?}", id.value),
        }
    }
}

/// The `migration_needed` a refusal carries; fails the test on any other.
pub fn migration_needed(error: &pb::Error) -> pb::MigrationNeeded {
    match &error.kind {
        Some(pb::error::Kind::MigrationNeeded(needed)) => needed.clone(),
        other => panic!("expected migration_needed, got {other:?} ({error:?})"),
    }
}

// ---------------------------------------------------------------------------
// D379: a Migrate card's steps and its holders' rows.

/// The states of the steps `item`'s card shows, in order.
pub fn step_states(item: &pb::InboxRequest) -> Vec<pb::MigrateStepState> {
    migrate_of(item)
        .map(|card| card.steps.iter().map(|step| step.state()).collect())
        .unwrap_or_default()
}

/// How far along a step is: a step only ever moves to a later one.
fn progress(state: pb::MigrateStepState, item: &pb::InboxRequest) -> u8 {
    match state {
        pb::MigrateStepState::Pending => 0,
        pb::MigrateStepState::Doing => 1,
        pb::MigrateStepState::Done | pb::MigrateStepState::Failed => 2,
        pb::MigrateStepState::Unspecified => panic!("a step has no state: {item:?}"),
    }
}

/// Fails unless `item`'s steps read as one run in order: the steps done
/// first, then at most one under way or failed, then the ones not begun,
/// each with a label.
pub fn assert_steps_in_order(item: &pb::InboxRequest) {
    let card = migrate_of(item).unwrap_or_else(|| panic!("not a Migrate item: {item:?}"));
    for step in &card.steps {
        assert!(!step.label.is_empty(), "a step has no label: {item:?}");
    }
    let place = |state: pb::MigrateStepState| match state {
        pb::MigrateStepState::Done => 0,
        pb::MigrateStepState::Doing | pb::MigrateStepState::Failed => 1,
        pb::MigrateStepState::Pending => 2,
        pb::MigrateStepState::Unspecified => panic!("a step has no state: {item:?}"),
    };
    let places: Vec<u8> = step_states(item).into_iter().map(place).collect();
    assert!(places.windows(2).all(|pair| pair[0] <= pair[1]), "steps out of order: {item:?}");
    assert!(
        places.iter().filter(|at| **at == 1).count() <= 1,
        "more than one step is under way or failed: {item:?}"
    );
}

/// Fails unless every item in `seen`, read in that order, shows its steps
/// in order, keeps the same steps once it has any, and never moves a step
/// back.
pub fn assert_steps_only_advance(seen: &[pb::InboxRequest]) {
    let mut before: Option<&pb::InboxRequest> = None;
    for item in seen {
        assert_steps_in_order(item);
        if step_states(item).is_empty() {
            assert!(
                before.is_none(),
                "the steps went away once shown: {item:?} after {before:?}"
            );
            continue;
        }
        if let Some(earlier) = before {
            let (was, now) = (migrate_of(earlier).unwrap(), migrate_of(item).unwrap());
            let labels = |card: &pb::MigratePreview| card.steps.iter().map(|step| step.label.clone()).collect::<Vec<_>>();
            assert_eq!(labels(was), labels(now), "the same steps, in the same order: {item:?} after {earlier:?}");
            for (then, later) in step_states(earlier).into_iter().zip(step_states(item)) {
                assert!(
                    progress(then, earlier) <= progress(later, item),
                    "a step went back from {then:?} to {later:?}: {item:?} after {earlier:?}"
                );
            }
        }
        before = Some(item);
    }
}

/// Item `id` as `daemon` showed it after each Inbox change, from now until
/// `reached` holds for it (that last one included). Nothing sleeps.
pub async fn request_history(
    daemon: &Daemon,
    id: &pb::RequestId,
    what: &str,
    reached: impl Fn(&pb::InboxRequest) -> bool,
) -> Vec<pb::InboxRequest> {
    let mut feed = inbox_feed(daemon).await;
    let deadline = tokio::time::Instant::now() + SLOW;
    let mut seen = Vec::new();
    loop {
        let now = request(daemon, id).await;
        let done = reached(&now);
        seen.push(now);
        if done {
            return seen;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the Inbox feed ended ({ended:?}) before request {} {what}; it went {seen:?}", id.value),
            Err(_) => panic!("request {} never {what} within {SLOW:?}; it went {seen:?}", id.value),
        }
    }
}

/// As [`request_history`] for a daemon about to start itself again: item
/// `id` after each change until it is no longer pending, or until the
/// daemon stops answering (its feed ends, or a read fails), whichever
/// comes first.
pub async fn request_history_while_up(daemon: &Daemon, id: &pb::RequestId) -> Vec<pb::InboxRequest> {
    // The daemon may already have taken its socket away to start itself
    // again (a stop no longer waits on open streams): then nothing is seen
    // while it is up.
    let Ok(channel) = crate::common::try_connect(&daemon.socket).await else {
        return Vec::new();
    };
    let Ok(feed) = pb::event_service_client::EventServiceClient::new(channel.clone())
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Inbox as i32],
            ..Default::default()
        })
        .await
    else {
        return Vec::new();
    };
    let mut feed = feed.into_inner();
    // One channel for every read: a fresh connect fails once the daemon
    // has taken its socket away, and a read through this one just fails.
    let mut reads = pb::inbox_service_client::InboxServiceClient::new(channel);
    let deadline = tokio::time::Instant::now() + SLOW;
    let mut seen = Vec::new();
    loop {
        let read = reads
            .get_request(pb::GetRequestRequest { id: Some(id.clone()) })
            .await;
        let Ok(now) = read.map(tonic::Response::into_inner) else {
            return seen;
        };
        let ended = now.state() != pb::RequestState::Pending;
        seen.push(now);
        if ended {
            return seen;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(_) => return seen,
            Err(_) => panic!("request {} neither ended nor did the daemon stop within {SLOW:?}; it went {seen:?}", id.value),
        }
    }
}

/// The rows of a Holders card for agent sessions: session id to note.
pub fn agent_rows(item: &pb::InboxRequest) -> std::collections::BTreeMap<String, pb::HolderNote> {
    migrate_of(item)
        .map(|card| {
            card.holders
                .iter()
                .filter_map(|row| {
                    let session = row.session.as_ref().filter(|session| !session.value.is_empty())?;
                    Some((session.value.clone(), row.note()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The notes of a Holders card's rows for shells, which name no session.
pub fn shell_rows(item: &pb::InboxRequest) -> Vec<pb::HolderNote> {
    migrate_of(item)
        .map(|card| {
            card.holders
                .iter()
                .filter(|row| row.session.as_ref().is_none_or(|session| session.value.is_empty()))
                .map(|row| row.note())
                .collect()
        })
        .unwrap_or_default()
}

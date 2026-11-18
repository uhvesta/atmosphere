//! UP-1, D281 f: a machine is upgraded over the SSH path that installed it.
//! A release is staged on the machine as verified `.<name>.next` files and
//! `.bundle.json.next` (checked on the far end, installed files untouched);
//! the daemon's Upgrade call then swaps them in and re-executes, and the
//! machine's link comes back at the new release with the old files kept as
//! `humatad2.previous` and nothing left staged.
//!
//! The Upgrade call is the daemon's (`upgrade/run.rs`); staging is the
//! machines crate's `stage_next_remote`.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/upgrade.rs"]
mod upgrade;
#[path = "common/upgrade_rig.rs"]
mod upgrade_rig;

use std::path::Path;
use std::sync::Arc;

use common::binary;
use humata2_fake_machine::HOST_KEY;
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{
    manifest_json, stage_next_remote, EnvValue, InstalledFile, Invocation, Link, MachineName, Output, Payload, Platform,
    RunError, Runner, SshAccess, SshTarget, StagedRelease, SystemRunner,
};
use upgrade_rig::BOX;

/// `humatad2` as the box has it from setup: a wrapper that tells the
/// machine's daemon where its state is, around the real binary. The release
/// staged here ships the same wrapper, so a link made after the upgrade
/// still finds the state.
const WRAPPER: &str = r##"#!/bin/sh
# humatad2 on this test's machine, release b.
case " $* " in *" --stdio "*|" stop "*)
  at=$(cd -P "$HOME/.humata2" && pwd -P) || exit 70
  exec '__REAL__' "$@" --state "$at/state" --socket "$at/d.sock" ;;
esac
exec '__REAL__' "$@"
"##;

/// Runs commands with the rig's `ssh` first on PATH, as the host daemon's
/// own does.
struct OnRigPath {
    path: String,
}

impl Runner for OnRigPath {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        let mut invocation = invocation.clone();
        invocation
            .env
            .push(("PATH".to_owned(), EnvValue::Value(self.path.clone())));
        SystemRunner::new(&()).run(&invocation)
    }
}

/// A platform Humata ships that isn't this test's, so the release's
/// entries for the machine and for this daemon never meet.
fn machine_platform(host: Platform) -> Platform {
    Platform::SHIPPED
        .into_iter()
        .find(|platform| *platform != host)
        .expect("Humata ships more than one platform")
}

/// Release `b`: the machine's own files for `platform` and a manifest that
/// also names the real daemon binary for `host`, so a daemon started from
/// either reports this release.
fn release_b(host: Platform, platform: Platform) -> StagedRelease {
    let wrapper = WRAPPER.replace("__REAL__", &binary("HUMATAD2").display().to_string());
    let files = vec![
        Payload::new(InstalledFile::Humatad, Arc::from(wrapper.into_bytes())),
        Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\n# holder b\nexit 0\n"[..])),
        Payload::new(InstalledFile::Cli, Arc::from(&b"#!/bin/sh\n# humata b\nexit 0\n"[..])),
        Payload::new(InstalledFile::Stack, Arc::from(&b"#!/bin/sh\n# stack b\nexit 0\n"[..])),
    ];
    let mut entries: Vec<(Platform, InstalledFile, String)> = files
        .iter()
        .map(|payload| (platform, payload.file(), payload.sha256().to_owned()))
        .collect();
    entries.push((host, InstalledFile::Humatad, upgrade::admin_sha()));
    entries.push((host, InstalledFile::Holder, files[1].sha256().to_owned()));
    entries.push((host, InstalledFile::Cli, files[2].sha256().to_owned()));
    entries.push((host, InstalledFile::Stack, files[3].sha256().to_owned()));
    StagedRelease::new(platform, manifest_json(&entries), files).expect("release b agrees with its manifest")
}

fn staged_names(bin: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(bin)
        .expect("the machine's bin folder exists")
        .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.') && name.ends_with(".next"))
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_staged_over_ssh_becomes_the_machines_release_once_migrated() {
    let (mut rig, release_a) = upgrade_rig::at_one_release().await;
    let host_platform = Platform::host().expect("the tests run on a platform Humata ships");
    let release = release_b(host_platform, machine_platform(host_platform));
    let bin = upgrade::machine_bin(rig.the(BOX).home());

    // The host moves to release b; gpu is still at a, so there is one item.
    upgrade::install(&upgrade::bin_of(&rig.host), release.bundle_json());
    rig.restart_host(&[]).await;
    upgrade_rig::daemon_item(&rig).await;

    // Staging puts the whole release beside what runs, and changes nothing.
    let name = MachineName::new(BOX).expect("gpu is a machine name");
    let known_hosts = rig.host.dir.path().join("upgrade-known-hosts");
    std::fs::write(&known_hosts, format!("humata2.{name} {HOST_KEY}\n")).expect("the host key is written");
    let link = Link::ssh(
        name,
        SshTarget {
            host: format!("{BOX}.internal"),
            access: SshAccess::Config { ssh_config: "/dev/null".to_owned() },
            host_key: None,
        },
        &known_hosts,
    );
    let runner = OnRigPath {
        path: format!("{}:/usr/bin:/bin", upgrade::bin_of(&rig.host).display()),
    };
    let id = stage_next_remote(&link, &runner, &release).expect("the release is staged on gpu");

    assert_eq!(id, upgrade::bundle_id(release.bundle_json()));
    assert_eq!(
        staged_names(&bin),
        [".bundle.json.next", ".humata.next", ".humata2_holder.next", ".humatad2.next", ".stack.next"]
    );
    assert_eq!(std::fs::read(bin.join(".bundle.json.next")).unwrap(), release.bundle_json().to_vec());
    assert_eq!(upgrade::bundle_id(&std::fs::read(bin.join("bundle.json")).unwrap()), release_a);

    // Migrate: the daemon verifies the staged files, swaps and re-executes.
    let mut daemon = pb::daemon_service_client::DaemonServiceClient::new(rig.the(BOX).daemon().channel().await);
    daemon
        .upgrade(pb::UpgradeRequest {
            to_bundle: id.clone(),
            upgrade_id: "upgrade-box-1".to_owned(),
            thing: pb::MigrateThing::Unspecified.into(),
        })
        .await
        .expect("the daemon on gpu takes the staged release");

    // gpu comes back at the host's release: nothing is left to migrate.
    upgrade::inbox_until(&rig.host, "nothing was left to migrate on gpu", |items| {
        (!items.iter().any(upgrade_rig::pending_on_box)).then_some(())
    })
    .await;
    assert_eq!(upgrade::build_of(rig.the(BOX).daemon()).await.bundle, id);
    assert_eq!(upgrade::bundle_id(&std::fs::read(bin.join("bundle.json")).unwrap()), id);
    assert_eq!(staged_names(&bin), Vec::<String>::new());
    assert!(bin.join("humatad2.previous").exists(), "the old daemon is kept for a rollback");
}

/// Puts the host at the release its own `--tools` bundle names, with a
/// `humatad2` for the machine (linux-x86_64) that is [`WRAPPER`] around the
/// admin test daemon, the one the rig's machines run, so a machine migrated
/// to it is still seen at that release. Restarts the host, which reads its
/// bundle when it starts; returns the bundle id a Migrate stages.
async fn host_at_tools_release(rig: &mut remote_rig::Rig) -> String {
    use std::os::unix::fs::PermissionsExt;
    let tools = rig.host.dir.path().join("tools");
    let wrapper = WRAPPER.replace("__REAL__", &binary("HUMATAD2").display().to_string());
    let file = tools.join("linux-x86_64").join("humatad2");
    std::fs::write(&file, &wrapper).expect("the machine's humatad2 is rewritten");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("it stays executable");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tools.join("manifest.json")).expect("the rig's tools manifest is there"))
            .expect("the rig's tools manifest is JSON");
    manifest["platforms"]["linux-x86_64"]["humatad2"]["sha256"] =
        serde_json::Value::String(upgrade::bundle_id(wrapper.as_bytes()));
    // The machine and the host run the same admin daemon on this platform, so
    // this platform's entry names it too.
    let mut host = manifest["platforms"]["linux-x86_64"].clone();
    host["humatad2"]["sha256"] = serde_json::Value::String(upgrade::admin_sha());
    manifest["platforms"][upgrade::platform()] = host;
    let bytes = serde_json::to_vec(&manifest).expect("a manifest serializes");
    std::fs::write(tools.join("manifest.json"), &bytes).expect("the rig's tools manifest is rewritten");
    let bundle = humata2_machines::canonical_manifest(&bytes).expect("the rig's manifest is a bundle manifest");
    let id = upgrade::install(&upgrade::bin_of(&rig.host), &bundle);
    rig.restart_host(&[]).await;
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_migrate_stages_the_release_over_ssh_and_the_machine_comes_back_at_it() {
    let (mut rig, release_a) = upgrade_rig::at_one_release().await;
    assert_ne!(
        upgrade::platform(),
        "linux-x86_64",
        "the rig's machine files are linux-x86_64, so a host of that platform can't name its own daemon beside them"
    );
    let bundle = host_at_tools_release(&mut rig).await;
    assert_ne!(bundle, release_a);
    let item = upgrade_rig::daemon_item(&rig).await;
    let id = item.id.clone().expect("an Inbox item has an id");
    let bin = upgrade::machine_bin(rig.the(BOX).home());

    rig.host
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "migrate-gpu".to_owned() }),
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            expected_revision: item.revision.clone(),
            ..Default::default()
        })
        .await
        .expect("the user migrates gpu");

    // The item ends when gpu is seen at the host's release.
    let done = upgrade::request_until(&rig.host, &id, "the Migrate item ended", |request| {
        request.state() != pb::RequestState::Pending
    })
    .await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(upgrade::build_of(rig.the(BOX).daemon()).await.bundle, bundle);
    assert_eq!(upgrade::bundle_id(&std::fs::read(bin.join("bundle.json")).unwrap()), bundle);
    assert_eq!(staged_names(&bin), Vec::<String>::new());
    assert!(bin.join("humatad2.previous").exists(), "the old daemon is kept for a rollback");
}

#[tokio::test(flavor = "multi_thread")]
async fn migrating_a_machines_daemon_shows_four_steps_that_advance_as_it_goes_and_all_end_done() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    host_at_tools_release(&mut rig).await;
    let item = upgrade_rig::daemon_item(&rig).await;
    let id = item.id.clone().expect("an Inbox item has an id");

    rig.host
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "migrate-gpu".to_owned() }),
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            expected_revision: item.revision.clone(),
            ..Default::default()
        })
        .await
        .expect("the user migrates gpu");

    // The host stays up throughout, so every change it pushes is read.
    let mut seen = vec![item];
    seen.extend(
        upgrade::request_history(&rig.host, &id, "ended", |request| {
            request.state() != pb::RequestState::Pending
        })
        .await,
    );
    let done = seen.last().expect("the history ends with the item ended").clone();
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    assert_eq!(upgrade::step_states(&done), vec![pb::MigrateStepState::Done; 4], "{done:?}");
    upgrade::assert_steps_only_advance(&seen);
    // The machine is away between its restart and its link coming back
    // (the host dials again seconds after a link ends), so the card was
    // seen with a step under way before it ended.
    assert!(
        seen.iter()
            .any(|shown| upgrade::step_states(shown).contains(&pb::MigrateStepState::Doing)),
        "no step was ever shown under way: {seen:?}"
    );
}

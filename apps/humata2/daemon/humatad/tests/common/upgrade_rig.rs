//! The multi-machine rig with each daemon at a release the test picks
//! (UP-1, D281): a host and the machine `gpu`, both the admin test daemon
//! (`Rig::start_seen`), so one `bundle.json` can name the one `humatad2`
//! both run, and another bundle can make `gpu` another release without
//! another binary.
//!
//! Use it as
//! `mod common; #[path = "common/remote_rig.rs"] mod remote_rig;
//! #[path = "common/upgrade.rs"] mod upgrade;
//! #[path = "common/upgrade_rig.rs"] mod upgrade_rig;`.

#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use humata2_grpc::humata::v1 as pb;

use crate::common::binary;
use crate::remote_rig::Rig;
use crate::upgrade;

/// The rig's one machine.
pub const BOX: &str = "gpu";

pub fn box_id() -> pb::MachineId {
    Rig::box_id(BOX)
}

/// Whether `item` is a pending Migrate item about the machine, whatever
/// it would migrate.
pub fn pending_on_box(item: &pb::InboxRequest) -> bool {
    item.kind() == pb::RequestKind::Migrate
        && item.state() == pb::RequestState::Pending
        && upgrade::migrate_of(item).is_some_and(|migrate| migrate.machine == Some(box_id()))
}

/// Puts the CLI, the holder and the stack the tests run in `bin`, whole, as the files
/// of the release `upgrade::manifest` names: the machine's own copies from
/// when it was added are other files, which a release that names these would
/// call a CLI and a host tools item (D371).
fn put_release_tools(bin: &Path) {
    for (name, variable) in [
        ("humata2_holder", "HUMATA2_HOLDER"),
        ("humata", "HUMATA"),
        ("stack", "STACK"),
    ] {
        let beside = bin.join(format!(".{name}.rig"));
        std::fs::copy(binary(variable), &beside)
            .unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", beside.display()));
        std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(0o755))
            .expect("the file is made runnable");
        std::fs::rename(&beside, bin.join(name))
            .unwrap_or_else(|error| panic!("couldn't put {name} in {}: {error}", bin.display()));
    }
}

/// The host and `gpu` at one release, linked, with the git remote
/// `acme/api`; returns the rig and the release's bundle id.
pub async fn at_one_release() -> (Rig, String) {
    let mut rig = Rig::start_seen(&[BOX], &[("acme", "api")]).await;
    let release = upgrade::manifest(&upgrade::admin_sha(), "a");
    let id = upgrade::install(&upgrade::bin_of(&rig.host), &release);
    let box_bin = upgrade::machine_bin(rig.the(BOX).home());
    upgrade::install(&box_bin, &release);
    put_release_tools(&box_bin);
    // Each daemon reads its bundle when it starts.
    rig.restart_host(&[]).await;
    rig.restart_box_env(BOX, &[]).await;
    // Between the two restarts one ran with a bundle and the other without,
    // which is a mismatch; once both have it, nothing is left to migrate.
    upgrade::inbox_until(&rig.host, "nothing was left to migrate on gpu", |items| {
        (!items.iter().any(pending_on_box)).then_some(())
    })
    .await;
    (rig, id)
}

/// Installs `bundle_json` on `gpu` and restarts its daemon, which reads it
/// when it starts; returns the bundle id once the host shows the link up.
pub async fn box_at(rig: &mut Rig, bundle_json: &[u8]) -> String {
    let id = upgrade::install(&upgrade::machine_bin(rig.the(BOX).home()), bundle_json);
    rig.restart_box_env(BOX, &[]).await;
    id
}

/// `gpu` at release `release`: the same `humatad2` it runs, in another
/// bundle.
pub async fn box_at_release(rig: &mut Rig, release: &str) -> String {
    box_at(rig, &upgrade::manifest(&upgrade::admin_sha(), release)).await
}

/// Puts the host at the release its own `--tools` bundle names, which is
/// the release a Migrate stages on a machine: the rig's tools manifest gains
/// an entry for this test's platform that names the real daemon (so the host
/// reports it), and the host's `bundle.json` is that manifest in canonical
/// form, so its bundle id is the one `Bundle::release` computes. Restarts the
/// host, which reads its bundle when it starts; returns the bundle id.
///
/// The rig's machine files are for `linux-x86_64`, so this is for tests run
/// on another platform.
pub async fn host_at_tools_release(rig: &mut Rig) -> String {
    assert_ne!(
        upgrade::platform(),
        "linux-x86_64",
        "the rig's machine files are linux-x86_64, so a host of that platform can't name its own daemon beside them"
    );
    let path = rig.host.dir.path().join("tools").join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("the rig's tools manifest is there"))
            .expect("the rig's tools manifest is JSON");
    let mut host = manifest["platforms"]["linux-x86_64"].clone();
    host["humatad2"]["sha256"] = serde_json::Value::String(upgrade::admin_sha());
    manifest["platforms"][upgrade::platform()] = host;
    let bytes = serde_json::to_vec(&manifest).expect("a manifest serializes");
    std::fs::write(&path, &bytes).expect("the rig's tools manifest is rewritten");
    let bundle = humata2_machines::canonical_manifest(&bytes).expect("the rig's manifest is a bundle manifest");
    let id = upgrade::install(&upgrade::bin_of(&rig.host), &bundle);
    rig.restart_host(&[]).await;
    id
}

/// The pending "Migrate the daemon on gpu" item, once the host has it.
pub async fn daemon_item(rig: &Rig) -> pb::InboxRequest {
    upgrade::migrate_item(&rig.host, pb::MigrateThing::Daemon, &box_id()).await
}

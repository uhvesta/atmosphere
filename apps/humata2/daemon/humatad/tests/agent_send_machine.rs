//! An agent in a session names its channel's machines by name alone (PL-3,
//! PL-12, TP-1, TP-4), with the real `humata` and its session token, against
//! a real host and boxes behind a fake `ssh`. The agent sees its channel's
//! machines and no others: one outside the channel is not found, as if it
//! weren't there. It copies a file from another machine into its own, and
//! sending one to another machine is refused as a write outside its own
//! folders, not misread as a path in its clone.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::time::{Duration, Instant};

use common::{approve, humata_as_agent, pending_requests, shell_token};
use remote_rig::{Rig, is_live};
use serde_json::Value;

/// The most the test waits for the copy to land.
const LONG: Duration = Duration::from_secs(120);

/// The machine ids an agent's `humata host ls` printed.
fn listed(json: &Value) -> Vec<String> {
    json["result"]["machines"]
        .as_array()
        .map(|machines| {
            machines
                .iter()
                .filter_map(|machine| machine["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_names_its_channels_machine_by_name_and_sees_no_other() {
    let rig = Rig::start(&["box-1", "box-2"], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let inside = Rig::box_id("box-1");
    let outside = Rig::box_id("box-2");
    let channel = rig
        .create_channel("Files", &[("acme", "api")], &[host.clone(), inside.clone()])
        .await;
    let session = rig.start_on(&channel, &host, "Copy it").await;
    rig.wait_session(&session, "live", is_live).await;
    std::fs::write(rig.host_path(&channel, "shared/note.txt"), b"from this mac\n").unwrap();
    let remote = rig.the("box-1").path(&channel, "shared/remote.txt");
    std::fs::create_dir_all(remote.parent().unwrap()).unwrap();
    std::fs::write(&remote, b"from box-1\n").unwrap();
    let (token, _shell) = shell_token(&rig.host, &session).await;

    // TP-1: the channel's machine is there for the agent; the other isn't.
    let (_, hosts) = humata_as_agent(&rig.host, &token, &["host", "ls"]).await;
    let ids = listed(&hosts);
    assert!(ids.contains(&inside.value), "{hosts}");
    assert!(!ids.contains(&outside.value), "{hosts}");
    let (code, shown) = humata_as_agent(&rig.host, &token, &["host", "show", "box-1"]).await;
    assert_eq!(code, 0, "{shown}");
    let (code, hidden) = humata_as_agent(&rig.host, &token, &["host", "show", "box-2"]).await;
    assert_ne!(code, 0, "{hidden}");
    assert_eq!(hidden["error"]["kind"], "not_found", "{hidden}");

    // An agent writes only on its own machine, so sending to box-1 is
    // refused, by the rule and not as a malformed clone path.
    let send = ["cp", "this-mac:shared/note.txt", "box-1"];
    let (code, sent) = humata_as_agent(&rig.host, &token, &send).await;
    assert_ne!(code, 0, "{sent}");
    assert_eq!(sent["error"]["kind"], "denied", "{sent}");
    assert!(!rig.the("box-1").path(&channel, "shared/note.txt").exists());

    // TP-4 and PL-3: a file of the channel's shared folder on box-1 lands in
    // the channel's shared folder on the agent's own machine. The rules may
    // ask the user first.
    let fetch = ["cp", "box-1:shared/remote.txt", "this-mac"];
    let (code, copied) = humata_as_agent(&rig.host, &token, &fetch).await;
    if code != 0 {
        assert_eq!(copied["error"]["kind"], "pending", "{copied}");
    }
    let landed = rig.host_path(&channel, "shared/remote.txt");
    let deadline = Instant::now() + LONG;
    while std::fs::read(&landed).ok().as_deref() != Some(b"from box-1\n".as_slice()) {
        assert!(Instant::now() < deadline, "{} never got the file", landed.display());
        for request in pending_requests(&rig.host).await {
            approve(&rig.host, &request).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits for `path` to hold `bytes`, approving whatever the rules ask the
/// user on the way.
async fn lands(rig: &Rig, path: &std::path::Path, bytes: &[u8]) {
    let deadline = Instant::now() + LONG;
    while std::fs::read(path).ok().as_deref() != Some(bytes) {
        assert!(Instant::now() < deadline, "{} never got the file", path.display());
        for request in pending_requests(&rig.host).await {
            approve(&rig.host, &request).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// D768: an agent on a machine's own daemon names machines and pulls from
// This Mac through the host, which keeps the machines and their links.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_on_a_machine_lists_its_channels_machines_and_pulls_from_this_mac() {
    let rig = Rig::start_seen(&["box-1"], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let inside = Rig::box_id("box-1");
    let channel = rig
        .create_channel("Files", &[("acme", "api")], &[host.clone(), inside.clone()])
        .await;
    let session = rig.start_on(&channel, &inside, "Pull it").await;
    rig.wait_session(&session, "live", is_live).await;
    std::fs::write(rig.host_path(&channel, "shared/agent-note.txt"), b"from this mac\n").unwrap();
    let token = rig.agent_token("box-1", &session).await;
    let daemon = rig.the("box-1").daemon();

    let (code, hosts) = humata_as_agent(daemon, &token, &["host", "ls"]).await;
    assert_eq!(code, 0, "{hosts}");
    let ids = listed(&hosts);
    assert!(ids.contains(&host.value), "{hosts}");
    assert!(ids.contains(&inside.value), "{hosts}");

    let pull = ["cp", "this-mac:shared/agent-note.txt", "box-1:shared/"];
    let (code, copied) = humata_as_agent(daemon, &token, &pull).await;
    if code != 0 {
        assert_eq!(copied["error"]["kind"], "pending", "{copied}");
    }
    lands(&rig, &rig.the("box-1").path(&channel, "shared/agent-note.txt"), b"from this mac\n").await;
}

// D769: as plain `cp a b`, a copy to a name nothing holds makes that file.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_copy_to_a_new_name_in_its_clone_makes_that_file() {
    let rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Files", &[("acme", "api")], &[host.clone()]).await;
    let session = rig.start_on(&channel, &host, "Copy it").await;
    rig.wait_session(&session, "live", is_live).await;
    std::fs::write(rig.host_path(&channel, "shared/note.txt"), b"a note\n").unwrap();
    let clone = std::path::PathBuf::from(&rig.session(&session).await.repos[0].path);
    let (token, _shell) = shell_token(&rig.host, &session).await;

    let copy = ["cp", "this-mac:shared/note.txt", "vm-b.txt"];
    let (code, copied) = humata_as_agent(&rig.host, &token, &copy).await;
    if code != 0 {
        assert_eq!(copied["error"]["kind"], "pending", "{copied}");
    }
    lands(&rig, &clone.join("vm-b.txt"), b"a note\n").await;
}

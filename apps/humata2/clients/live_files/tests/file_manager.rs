//! The file manager's behaviour, checked on `FakeFileManager` and on
//! `LiveFileManager` against a real `humatad2`, so the fake cannot drift
//! from what the daemon does. Not covered because the daemon can't do it
//! yet: the home root, other machines, Storage (`GetStorage`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::desk::{Access, DeskClient, FolderChoice, NewDeskSession};
use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId, SessionOptions, TransferId};
use humata2_clients::{
    Daemon, FakeFileManager, FileManager, FileManagerError, FileRoot, Folder, TransferMode, TransferPhase, TransferRow,
};
use humata2_clients_live_core::LiveOptions;
use humata2_clients_live_daemon::LiveDaemon;
use humata2_clients_live_desk::LiveDesk;
use humata2_clients_live_files::LiveFileManager;
use tempfile::TempDir;
use tokio::sync::mpsc;

const DEADLINE: Duration = Duration::from_secs(30);
const AGENT_NAME: &str = "fake";
const CHANNEL: &str = "payments";
const REPO_OWNER: &str = "owner";
const REPO_NAME: &str = "ledger";

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

/// What a case needs: a file manager, a channel with a session and its clone,
/// a way to put a file in the clone, and a way to let a transfer finish
/// (the fake's are stepped by hand; the daemon's run on their own).
struct Setup {
    fm: Arc<dyn FileManager>,
    mac: MachineId,
    channel: ChannelId,
    session: SessionId,
    seed: Box<dyn Fn(&str, &[u8]) + Send + Sync>,
    finish: Box<dyn Fn(&TransferId) + Send + Sync>,
    _world: Option<RealWorld>,
}

impl Setup {
    fn shared(&self) -> Folder {
        Folder::root(self.mac.clone(), FileRoot::ChannelShared(self.channel.clone()))
    }

    fn clone_root(&self) -> Folder {
        Folder::root(
            self.mac.clone(),
            FileRoot::SessionClone {
                session: self.session.clone(),
                repo: format!("{REPO_OWNER}/{REPO_NAME}"),
            },
        )
    }

    async fn names_in(&self, folder: &Folder) -> Vec<String> {
        within("list", self.fm.list(folder))
            .await
            .expect("list")
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }

    /// Waits for `id` to be Done on `events`; a failure ends the case.
    async fn wait_done(&self, events: &mut mpsc::Receiver<TransferRow>, id: &TransferId) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, events.recv()).await {
                Ok(Some(row)) if &row.id == id => match row.phase {
                    TransferPhase::Done => return,
                    TransferPhase::Failed { reason } => panic!("the transfer failed: {reason}"),
                    TransferPhase::Cancelled => panic!("the transfer was cancelled"),
                    _ => {}
                },
                Ok(Some(_)) => {}
                Ok(None) => panic!("the transfer events ended"),
                Err(_) => panic!("timed out waiting for the transfer to be done"),
            }
        }
    }
}

fn fake_setup() -> Setup {
    let mac = MachineId::new("this-mac");
    let fake = Arc::new(FakeFileManager::new(mac.clone()));
    let session = SessionId::new("se_fake");
    let clone_root = Folder::root(
        mac.clone(),
        FileRoot::SessionClone {
            session: session.clone(),
            repo: format!("{REPO_OWNER}/{REPO_NAME}"),
        },
    );
    let seeding = fake.clone();
    let finishing = fake.clone();
    Setup {
        fm: fake,
        mac,
        channel: ChannelId::new("ch_fake"),
        session,
        seed: Box::new(move |name, bytes| seeding.put_file(&clone_root, name, bytes.len() as u64)),
        finish: Box::new(move |id| finishing.finish(id)),
        _world: None,
    }
}

// -- The real daemon, as in tests/support/real.rs ---------------------------

struct RealWorld {
    dir: TempDir,
    child: Child,
}

impl Drop for RealWorld {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Holders outlive the daemon on purpose; everything this test
        // started has its folder in its command line.
        let _ = Command::new("pkill").arg("-f").arg(self.dir.path()).status();
    }
}

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).expect("the binary exists")
}

fn git(args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A bare remote `owner/name` on `main` with one commit.
fn make_remote(root: &Path, owner: &str, name: &str) {
    let bare = root.join("remotes").join(owner).join(format!("{name}.git"));
    std::fs::create_dir_all(&bare).expect("make the remote folder");
    let bare_text = bare.display().to_string();
    git(&["init", "--bare", "--initial-branch=main", &bare_text]);
    let work = root.join(format!("scratch-{owner}-{name}"));
    std::fs::create_dir_all(&work).expect("make the scratch folder");
    let work_text = work.display().to_string();
    git(&["init", "--initial-branch=main", &work_text]);
    std::fs::write(work.join("README.md"), format!("{owner}/{name}\n")).expect("write");
    git(&["-C", &work_text, "add", "."]);
    git(&["-C", &work_text, "commit", "-m", "first"]);
    git(&["-C", &work_text, "push", &bare_text, "main"]);
}

/// Stands in for an agent: answers every line typed with `got: <line>`.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv.display()
    )
}

/// A real daemon in a temp folder, and a client connected to it.
async fn start_real() -> (RealWorld, LiveDaemon) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("h2f")
        .tempdir_in("/tmp")
        .expect("make a temp folder");
    // The daemon refuses a socket folder other users can open.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).expect("restrict the temp folder");
    let root = dir.path().to_owned();
    let conv = root.join("conv");
    std::fs::create_dir_all(&conv).expect("make conversations folder");
    std::fs::create_dir_all(root.join("remotes")).expect("make remotes folder");
    make_remote(&root, REPO_OWNER, REPO_NAME);
    let script = root.join("agent.sh");
    std::fs::write(&script, agent_script(&conv)).expect("write the agent");
    let agents = root.join("agents.json");
    let spec = serde_json::json!([{
        "name": AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
    }]);
    std::fs::write(&agents, spec.to_string()).expect("write agents.json");
    let socket = root.join("d.sock");
    let child = Command::new(binary("HUMATAD2"))
        .arg("serve")
        .arg("--admin")
        .arg("--channels")
        .arg(root.join("ch"))
        .arg("--agents")
        .arg(&agents)
        .arg("--default-agent")
        .arg(AGENT_NAME)
        .arg("--remote-template")
        .arg(format!("file://{}/remotes/{{owner}}/{{name}}.git", root.display()))
        .arg("--socket")
        .arg(&socket)
        .arg("--state")
        .arg(root.join("state"))
        .arg("--root")
        .arg(&root)
        .arg("--holder")
        .arg(binary("HUMATA2_HOLDER"))
        .arg("--shell")
        .arg("/bin/sh")
        .arg("--shell-arg=-i")
        .env("PS1", "$ ")
        .env("ENV", "/dev/null")
        .spawn()
        .expect("start humatad2");
    let world = RealWorld { dir, child };
    let deadline = Instant::now() + Duration::from_secs(15);
    while tokio::net::UnixStream::connect(&socket).await.is_err() {
        assert!(Instant::now() < deadline, "humatad2 never started listening");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let live = LiveDaemon::connect(LiveOptions::new(socket).with_agent(Agent::Claude, AGENT_NAME))
        .await
        .expect("connect to humatad2");
    (world, live)
}

async fn real_setup() -> Setup {
    let (world, live) = start_real().await;
    let root = world.dir.path().to_owned();
    let workspace = within("load", live.load()).await.expect("load");
    let mac = workspace.machines.first().expect("the workspace has a machine").id.clone();
    let repo = format!("{REPO_OWNER}/{REPO_NAME}");
    let channel = within(
        "create_channel",
        live.create_channel(CHANNEL, vec![repo.clone()], vec![mac.clone()]),
    )
    .await
    .expect("create channel");
    let session = within(
        "create_session",
        live.create_session(
            &channel,
            Agent::Claude,
            &mac,
            "Files",
            SessionOptions {
                repos: vec![repo],
                first_message: String::new(),
                ..SessionOptions::default()
            },
        ),
    )
    .await
    .expect("create session");
    let channels = root.join("ch");
    let fm = LiveFileManager::new(&live);
    Setup {
        fm: Arc::new(fm),
        mac,
        channel,
        session,
        seed: Box::new(move |name, bytes| {
            let clone_dir = session_clone(&channels);
            std::fs::write(clone_dir.join(name), bytes).expect("seed a file in the clone");
        }),
        // The daemon's transfers run on their own.
        finish: Box::new(|_| {}),
        _world: Some(world),
    }
}

/// The one session's clone of the repo on disk. The daemon names the
/// channel's and the session's folders itself, so the clone is found under
/// `<channels>/<channel>/sessions/<session>/<repo>`, not guessed; it is
/// there once the session's start has cloned it.
fn session_clone(channels: &Path) -> PathBuf {
    let folders = |folder: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(folder)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    let deadline = Instant::now() + DEADLINE;
    loop {
        for channel in folders(channels) {
            for session in folders(&channel.join("sessions")) {
                let clone = session.join(REPO_NAME);
                if clone.join(".git").exists() {
                    return clone;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "the session's clone of {REPO_NAME} never appeared under {}",
            channels.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

// -- The cases --------------------------------------------------------------

async fn lists_new_folder(setup: Setup) {
    within("create_folder", setup.fm.create_folder(&setup.shared(), "out"))
        .await
        .expect("create folder");
    let listing = within("list", setup.fm.list(&setup.shared())).await.expect("list");
    let out = listing
        .entries
        .iter()
        .find(|entry| entry.name == "out")
        .expect("the new folder is listed");
    assert_eq!(out.kind, humata2_clients::EntryKind::Folder);
    assert_eq!(out.size, None);
    assert_eq!(listing.folder, setup.shared());
}

async fn rename_delete_then_restore_from_trash(setup: Setup) {
    let shared = setup.shared();
    within("create_folder", setup.fm.create_folder(&shared, "out"))
        .await
        .expect("create folder");
    within("rename", setup.fm.rename(&shared, "out", "results"))
        .await
        .expect("rename");
    let names = setup.names_in(&shared).await;
    assert!(names.contains(&"results".to_owned()), "{names:?}");
    assert!(!names.contains(&"out".to_owned()), "{names:?}");

    within("delete", setup.fm.delete(&shared, vec!["results".to_owned()]))
        .await
        .expect("delete");
    assert!(!setup.names_in(&shared).await.contains(&"results".to_owned()));
    let trash = within("trash", setup.fm.trash(&setup.mac)).await.expect("trash");
    let item = trash
        .items
        .iter()
        .find(|item| item.description.contains("results"))
        .expect("the deleted folder is in the trash");

    within("restore", setup.fm.restore(&setup.mac, &item.id))
        .await
        .expect("restore");
    assert!(setup.names_in(&shared).await.contains(&"results".to_owned()));
    let trash = within("trash", setup.fm.trash(&setup.mac)).await.expect("trash");
    assert!(trash.items.iter().all(|item| !item.description.contains("results")));
}

async fn copy_from_a_clone_reaches_done(setup: Setup) {
    (setup.seed)("data.bin", &[7u8; 2000]);
    let mut events = setup.fm.transfer_events();
    let id = within(
        "start_transfer",
        setup.fm.start_transfer(
            &setup.clone_root(),
            vec!["data.bin".to_owned()],
            &setup.shared(),
            TransferMode::Copy,
        ),
    )
    .await
    .expect("start the transfer");
    (setup.finish)(&id);
    setup.wait_done(&mut events, &id).await;
    let listing = within("list", setup.fm.list(&setup.shared())).await.expect("list");
    let copied = listing
        .entries
        .iter()
        .find(|entry| entry.name == "data.bin")
        .expect("the copy is in the shared folder");
    assert_eq!(copied.size, Some(2000));
    let rows = within("transfers", setup.fm.transfers()).await.expect("transfers");
    let row = rows.iter().find(|row| row.id == id).expect("the transfer is in the tray");
    assert_eq!(row.phase, TransferPhase::Done);
    assert_eq!(row.done_bytes, row.total_bytes);
}

async fn upload_of_3_mib_is_listed_with_its_size(setup: Setup) {
    let local = tempfile::tempdir().expect("make a local folder");
    let path = local.path().join("big.bin");
    let size = 3 * 1024 * 1024;
    let content: Vec<u8> = (0..size).map(|index| u8::try_from(index % 251).unwrap_or(0)).collect();
    std::fs::write(&path, &content).expect("write the local file");
    let mut events = setup.fm.transfer_events();
    let id = within("upload", setup.fm.upload(vec![path], &setup.shared()))
        .await
        .expect("upload");
    (setup.finish)(&id);
    setup.wait_done(&mut events, &id).await;
    let listing = within("list", setup.fm.list(&setup.shared())).await.expect("list");
    let uploaded = listing
        .entries
        .iter()
        .find(|entry| entry.name == "big.bin")
        .expect("the upload is listed");
    assert_eq!(uploaded.size, Some(content.len() as u64));
}

async fn a_name_already_taken_is_refused(setup: Setup) {
    let shared = setup.shared();
    within("create_folder", setup.fm.create_folder(&shared, "dup"))
        .await
        .expect("create folder");
    let again = within("create_folder", setup.fm.create_folder(&shared, "dup")).await;
    assert_eq!(again, Err(FileManagerError::NameTaken { name: "dup".to_owned() }));
    within("create_folder", setup.fm.create_folder(&shared, "other"))
        .await
        .expect("create folder");
    let rename = within("rename", setup.fm.rename(&shared, "other", "dup")).await;
    assert_eq!(rename, Err(FileManagerError::NameTaken { name: "dup".to_owned() }));
}

async fn emptying_the_trash_frees_what_the_plan_said(setup: Setup) {
    let local = tempfile::tempdir().expect("make a local folder");
    let path = local.path().join("junk.bin");
    std::fs::write(&path, vec![1u8; 4096]).expect("write the local file");
    let mut events = setup.fm.transfer_events();
    let id = within("upload", setup.fm.upload(vec![path], &setup.shared()))
        .await
        .expect("upload");
    (setup.finish)(&id);
    setup.wait_done(&mut events, &id).await;
    within(
        "delete",
        setup.fm.delete(&setup.shared(), vec!["junk.bin".to_owned()]),
    )
    .await
    .expect("delete");

    let plan = within("plan", setup.fm.plan_empty_trash(&setup.mac))
        .await
        .expect("plan");
    assert_eq!(plan.items, 1);
    assert!(plan.frees_bytes >= 4096, "the plan frees {}", plan.frees_bytes);
    let freed = within("empty", setup.fm.empty_trash(plan.clone())).await.expect("empty");
    assert_eq!(freed, plan.frees_bytes);
    let trash = within("trash", setup.fm.trash(&setup.mac)).await.expect("trash");
    assert!(trash.items.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drop_with_one_name_already_taken_uploads_nothing() {
    let setup = real_setup().await;
    let shared = setup.shared();
    within("create_folder", setup.fm.create_folder(&shared, "b.txt"))
        .await
        .expect("make the name taken");
    let local = tempfile::tempdir().expect("make a local folder");
    let dropped: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .map(|name| {
            let path = local.path().join(name);
            std::fs::write(&path, b"contents").expect("write a local file");
            path
        })
        .collect();

    let refused = within("upload", setup.fm.upload(dropped, &shared)).await;

    assert!(refused.is_err(), "a drop with a taken name is refused");
    assert_eq!(setup.names_in(&shared).await, ["b.txt"], "nothing else was uploaded");
}

/// D335: files pasted into a session's terminal from another computer are
/// uploaded into the session's pastes, each paste in a folder of its own,
/// and the paths that come back are where the files are on the session's
/// machine, in order.
#[tokio::test(flavor = "multi_thread")]
async fn pasted_files_land_in_the_sessions_paste_folder() {
    let setup = real_setup().await;
    let local = tempfile::tempdir().expect("make a local folder");
    let write = |name: &str, bytes: &[u8]| {
        let path = local.path().join(name);
        std::fs::write(&path, bytes).expect("write a local file");
        path
    };
    let first = vec![write("notes.txt", b"first notes"), write("a b.png", b"image")];

    let pasted = within(
        "paste_files",
        setup.fm.paste_files(first, &setup.mac, &setup.session),
    )
    .await
    .expect("the files are pasted");

    assert_eq!(pasted.len(), 2, "{pasted:?}");
    assert!(pasted[0].ends_with("notes.txt"), "{pasted:?}");
    assert!(pasted[1].ends_with("a b.png"), "{pasted:?}");
    assert_eq!(std::fs::read(&pasted[0]).expect("the first file is there"), b"first notes");
    assert_eq!(std::fs::read(&pasted[1]).expect("the second file is there"), b"image");

    let again = vec![write("notes.txt", b"second notes")];
    let repasted = within(
        "paste_files again",
        setup.fm.paste_files(again, &setup.mac, &setup.session),
    )
    .await
    .expect("a name pasted before is pasted again");

    assert_ne!(repasted[0], pasted[0]);
    assert_eq!(std::fs::read(&repasted[0]).expect("the new file is there"), b"second notes");
    assert_eq!(std::fs::read(&pasted[0]).expect("the old file stays"), b"first notes");
}

macro_rules! parity {
    ($($case:ident => $fake:ident, $real:ident;)*) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $fake() {
            $case(fake_setup()).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn $real() {
            $case(real_setup().await).await;
        }
    )*};
}

parity! {
    lists_new_folder => lists_new_folder_on_fake, lists_new_folder_on_real;
    rename_delete_then_restore_from_trash => rename_delete_restore_on_fake, rename_delete_restore_on_real;
    copy_from_a_clone_reaches_done => copy_from_a_clone_on_fake, copy_from_a_clone_on_real;
    upload_of_3_mib_is_listed_with_its_size => upload_3_mib_on_fake, upload_3_mib_on_real;
    a_name_already_taken_is_refused => name_taken_on_fake, name_taken_on_real;
    emptying_the_trash_frees_what_the_plan_said => empty_trash_on_fake, empty_trash_on_real;
}

/// D382: a Desk session has no repository; its folder is a root of its own.
/// The fake lists what it was given; the real daemon lists the session's
/// folder on disk and reads a document in it as the session's own.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_sessions_folder_is_listed_and_read_on_the_real_daemon() {
    let (_world, live) = start_real().await;
    let workspace = within("load", live.load()).await.expect("load");
    let mac = workspace.machines.first().expect("the workspace has a machine").id.clone();
    let id = within(
        "desk start",
        LiveDesk::new(&live).start(NewDeskSession {
            title: "Draft".to_owned(),
            agent: Agent::Claude,
            machine: mac.clone(),
            folder: FolderChoice::Scratch {
                name: humata2_clients::desk::folder_name("Draft"),
            },
            repos: Vec::new(),
            access: Access::default(),
        }),
    )
    .await
    .expect("start a desk session");
    let workspace = within("load", live.load()).await.expect("load");
    let humata2_clients::desk::Folder::Scratch { path, .. } = workspace.desk.sessions[&id].folder.clone() else {
        panic!("a scratch folder");
    };
    std::fs::write(Path::new(&path).join("rfc.md"), "the document").expect("write the document");
    let fm = LiveFileManager::new(&live);
    let folder = Folder::root(mac, FileRoot::SessionFolder { session: id.clone() });

    within("create_folder", fm.create_folder(&folder, "drafts")).await.expect("make a sub-folder");
    let listing = within("list", fm.list(&folder)).await.expect("list the session's folder");
    let read = within("read", live.read_file(&id, "", "rfc.md")).await;

    let names: Vec<String> = listing.entries.iter().map(|entry| entry.name.clone()).collect();
    assert!(names.contains(&"rfc.md".to_owned()), "{names:?}");
    assert!(names.contains(&"drafts".to_owned()), "{names:?}");
    assert!(Path::new(&path).join("drafts").is_dir());
    assert_eq!(read.expect("the document is read"), "the document");
}

/// The fake takes the same root, so a pane on a Desk session's folder
/// lists what was put in it.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_sessions_folder_is_listed_on_the_fake() {
    let mac = MachineId::new("this-mac");
    let fake = FakeFileManager::new(mac.clone());
    let folder = Folder::root(mac, FileRoot::SessionFolder { session: SessionId::new("se_desk") });
    fake.put_file(&folder, "rfc.md", 12);

    let listing = within("list", fake.list(&folder)).await.expect("list");

    let names: Vec<String> = listing.entries.iter().map(|entry| entry.name.clone()).collect();
    assert_eq!(names, ["rfc.md"]);
}

/// PL-3, PL-4: a copy into another machine's received place lands in
/// `<channels>/received/<machine>/` and that place lists what landed.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_into_a_received_place_lands_there_and_lists_on_the_real_daemon() {
    let setup = real_setup().await;
    (setup.seed)("data.bin", &[7u8; 2000]);
    let received = Folder::root(setup.mac.clone(), FileRoot::Received { from: MachineId::new("gpu-box") });
    let mut events = setup.fm.transfer_events();

    let id = within(
        "start_transfer",
        setup.fm.start_transfer(&setup.clone_root(), vec!["data.bin".to_owned()], &received, TransferMode::Copy),
    )
    .await
    .expect("a copy into the received place starts");
    setup.wait_done(&mut events, &id).await;

    let listing = within("list", setup.fm.list(&received)).await.expect("list the received place");
    let landed = listing.entries.iter().find(|entry| entry.name == "data.bin").expect("the copy is listed");
    assert_eq!(landed.size, Some(2000));
    let world = setup._world.as_ref().expect("a real world");
    assert!(world.dir.path().join("ch/received/gpu-box/data.bin").is_file());
}

/// A received place nothing has landed in yet lists empty, not as an error.
#[tokio::test(flavor = "multi_thread")]
async fn a_received_place_with_nothing_in_it_lists_empty_on_the_real_daemon() {
    let setup = real_setup().await;
    let received = Folder::root(setup.mac.clone(), FileRoot::Received { from: MachineId::new("never-sent") });

    let names = setup.names_in(&received).await;

    assert!(names.is_empty(), "{names:?}");
}

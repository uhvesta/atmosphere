//! The To-do list through `LiveTodo` against a real `humatad2` (admin
//! build, so every call is the person's): the shipping daemon serves the
//! list the To-do page and the session panel draw from (BOARDSIMPLE).
//! Every case has its own daemon, folder and socket, and starts no
//! sessions.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_clients::model::Agent;
use humata2_clients::todo::{NewTodo, TodoClient, TodoFilter, TodoPatch};
use humata2_clients_live_daemon::todo::LiveTodo;
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use humata2_todo::State;
use tempfile::TempDir;

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often the socket is tried while the daemon starts.
const POLL: Duration = Duration::from_millis(2);

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).expect("the binary exists")
}

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

/// A real `humatad2` in a folder of its own. Dropping it stops the daemon;
/// no case starts a session, so there are no holders to end.
struct Served {
    _dir: TempDir,
    child: Child,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start() -> (Served, LiveTodo) {
    use std::os::unix::fs::PermissionsExt;
    // Short, so the socket's path fits.
    let dir = tempfile::Builder::new()
        .prefix("h2t")
        .tempdir_in("/tmp")
        .expect("make a temp folder");
    // The daemon refuses a socket folder other users can open.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restrict the temp folder");
    let root = dir.path().to_path_buf();
    let socket = root.join("d.sock");
    let child = Command::new(binary("HUMATAD2"))
        .arg("serve")
        .arg("--admin")
        .arg("--channels")
        .arg(root.join("ch"))
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
        .env_remove("HUMATA_SESSION_TOKEN")
        .env_remove("HUMATA_SOCKET")
        .spawn()
        .expect("start humatad2");
    let served = Served { _dir: dir, child };
    let until = Instant::now() + DEADLINE;
    while tokio::net::UnixStream::connect(&socket).await.is_err() {
        assert!(Instant::now() < until, "humatad2 never started listening");
        tokio::time::sleep(POLL).await;
    }
    let daemon = within(
        "the connection to humatad2",
        LiveDaemon::connect(LiveOptions::new(socket).with_agent(Agent::Claude, "fake")),
    )
    .await
    .expect("connect to humatad2");
    (served, LiveTodo::new(&daemon))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_keeps_a_to_do_the_person_adds_and_closes() {
    let (_served, todo) = start().await;

    let added = within(
        "adding a to-do",
        todo.create(NewTodo {
            title: "Write the release notes".to_owned(),
            ..NewTodo::default()
        }),
    )
    .await
    .expect("the daemon adds the to-do");
    assert_eq!(added.todo.state, State::Open);

    let listed = within("listing", todo.list(&TodoFilter::default()))
        .await
        .expect("the daemon lists the to-dos");
    assert!(
        listed.iter().any(|entry| entry.todo.key == added.todo.key),
        "the new to-do is in the list"
    );

    let closed = within(
        "closing the to-do",
        todo.update(
            &added.todo.key,
            TodoPatch {
                state: Some(State::Closed),
                ..TodoPatch::default()
            },
            added.revision,
        ),
    )
    .await
    .expect("the daemon closes the to-do");
    assert_eq!(closed.todo.state, State::Closed);

    let open_only = within("listing open items", todo.list(&TodoFilter::default()))
        .await
        .expect("the daemon lists the to-dos");
    assert!(
        open_only.iter().all(|entry| entry.todo.key != added.todo.key),
        "a closed to-do leaves the open list"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_daemon_has_no_boards_to_move_so_no_migration_notice() {
    let (_served, todo) = start().await;
    let notice = within("the migration notice", todo.migration())
        .await
        .expect("the daemon answers the migration notice");
    assert!(notice.is_none(), "nothing was moved, so nothing is announced");
}

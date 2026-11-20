//! Live To-do flows against a real `humatad2`:
//! 1. An agent's ask to close a person's item answered Yes from the item's
//!    To-do drawer: the agent's call succeeds, the request leaves the list,
//!    and the item closes.
//! 2. The same ask answered No: the agent's call fails as denied, the request
//!    leaves the list, and the item stays open.
//! 3. Clicking "Start a session on this" in the item's To-do drawer opens the
//!    prefilled New session dialog, starts the session, attaches it to the
//!    item, and moves the item to In progress.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gpui_kit::SharedString;
use gpui_kit::test::TestWindowExt as _;
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::model::ItemId;
use humata2_clients::todo::NewTodo;
use humata2_grpc::humata::v1 as pb;
use rustix::process::{Pid, Signal, kill_process};

use super::world::World;
use super::{App, channel_with_repo, runtime, session_in};

/// `humata` exit code for a denied approval request (`cli/src/error.rs`).
const EXIT_DENIED: i32 = 4;

fn humata_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA")
        .map(PathBuf::from)
        .expect("set HUMATA to the binary's path (the bazel target passes it)");
    std::fs::canonicalize(path).expect("the humata binary exists")
}

/// Runs `humata --json --wait 600 board move <key> done` as the running
/// session's agent and wakes the test thread as soon as the command exits.
/// Dropping before completion kills the child so no process leaks on panic.
struct AgentMove {
    pid: u32,
    finished: Arc<AtomicBool>,
    done: Arc<Mutex<Option<Output>>>,
}

impl AgentMove {
    fn move_to_done(world: &World, key: &str) -> Self {
        let child = Command::new(humata_binary())
            .args(["--json", "--wait", "600", "board", "move", key, "done"])
            .env("HUMATA_SOCKET", world.socket())
            .env("HUMATA_SESSION_TOKEN", world.agent_token())
            .env_remove("HUMATA_LOGIN_TOKEN")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn humata board move as the agent");
        let pid = child.id();
        let finished = Arc::new(AtomicBool::new(false));
        let done = Arc::new(Mutex::new(None));
        let flag = finished.clone();
        let slot = done.clone();
        let waiter = std::thread::current();
        std::thread::spawn(move || {
            let output = child
                .wait_with_output()
                .expect("wait for the agent's humata board move");
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(output);
            }
            flag.store(true, Ordering::Release);
            waiter.unpark();
        });
        Self {
            pid,
            finished,
            done,
        }
    }

    fn is_done(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    fn take(&self) -> Output {
        self.done
            .lock()
            .expect("agent output lock")
            .take()
            .expect("agent call finished")
    }
}

impl Drop for AgentMove {
    fn drop(&mut self) {
        if !self.finished.load(Ordering::Acquire)
            && let Ok(raw) = i32::try_from(self.pid)
            && let Some(pid) = Pid::from_raw(raw)
        {
            let _ = kill_process(pid, Signal::KILL);
        }
    }
}

fn send(app: &mut App<'_>, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// Whether the window draws an element with this id.
fn draws(app: &mut App<'_>, id: &str) -> bool {
    app.cx
        .update_window(app.window, |_, window, _| {
            window.try_find(SharedString::from(id.to_owned())).is_some()
        })
        .expect("the window stays open")
}

fn click_id(app: &mut App<'_>, id: &str) {
    app.settle();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.click(SharedString::from(id.to_owned()), cx)
        })
        .expect("the control is drawn");
    app.settle();
}

async fn daemon_todo(world: &World, key: &str) -> Option<pb::TodoItem> {
    let channel = world.host_channel().await;
    let items = pb::todo_service_client::TodoServiceClient::new(channel)
        .list_todo(pb::ListTodoRequest {
            filter: Some(pb::TodoFilter {
                show_closed: true,
                ..pb::TodoFilter::default()
            }),
            page: None,
        })
        .await
        .expect("list to-dos from the daemon")
        .into_inner()
        .items;
    items
        .into_iter()
        .find(|entry| entry.fields.as_ref().is_some_and(|f| f.key == key))
}

async fn pending_requests(world: &World) -> Vec<pb::InboxRequest> {
    let channel = world.host_channel().await;
    pb::inbox_service_client::InboxServiceClient::new(channel)
        .list_requests(pb::ListRequestsRequest {
            include_answered: false,
            ..pb::ListRequestsRequest::default()
        })
        .await
        .expect("list pending requests from the daemon")
        .into_inner()
        .requests
}

/// Sets up a channel with a running session, creates a person's To-do item
/// there, opens the app on To-do, and starts the agent's `board move <key> done`
/// call until the ask appears on the item.
fn start_ask_scenario(
    runtime: &tokio::runtime::Runtime,
) -> (World, humata2_clients::Clients, String) {
    runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, repo) = channel_with_repo(&world, &clients, "payments").await;
        session_in(&world, &clients, &channel, repo, "Ledger", "hello agent").await;
        let created = clients
            .todo
            .create(NewTodo {
                title: "Schema".to_owned(),
                channel: Some(channel.as_str().to_owned()),
                ..NewTodo::default()
            })
            .await
            .expect("create the person's to-do item");
        (world, clients, created.todo.key)
    })
}

/// Answering Yes to an agent's ask from the To-do item drawer closes the item,
/// removes the pending request, and lets the agent's call succeed.
#[test]
fn approving_an_agents_ask_from_the_todo_drawer_closes_the_item_and_returns_the_call() {
    let runtime = runtime();
    let (world, clients, key) = start_ask_scenario(&runtime);
    let mut app = App::open(&runtime, clients);

    send(&mut app, AppAction::Switch(Route::Todo));
    app.wait_until("the to-do item to load", |app| {
        app.ui
            .read(|s| s.todo.loaded && s.todo.items.iter().any(|e| e.todo.key == key))
    });

    let agent = AgentMove::move_to_done(&world, &key);
    app.wait_until("the agent's ask to appear on the to-do item", |app| {
        app.ui.read(|s| {
            s.todo
                .items
                .iter()
                .any(|e| e.todo.key == key && e.ask_id.is_some() && e.todo.ask.is_some())
        })
    });

    let row_id = format!("todo-row {key}");
    app.wait_until("the to-do row to draw", |app| draws(app, &row_id));
    click_id(&mut app, &row_id);
    app.wait_until("the Yes button in the drawer", |app| draws(app, "todo-yes"));
    click_id(&mut app, "todo-yes");

    app.wait_until("the agent's call to finish", |_| agent.is_done());
    let output = agent.take();
    assert!(
        output.status.success(),
        "approved agent call should exit 0, got {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&key),
        "approved agent call stdout should name {key}: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    app.wait_until("the item to close and lose its ask in the app", |app| {
        app.ui.read(|s| {
            s.todo.items.iter().any(|e| {
                e.todo.key == key
                    && e.ask_id.is_none()
                    && e.todo.ask.is_none()
                    && format!("{:?}", e.todo.state) == "Closed"
            })
        })
    });

    let (daemon_item, requests) = runtime.block_on(async {
        (
            daemon_todo(&world, &key).await.expect("item still on daemon"),
            pending_requests(&world).await,
        )
    });
    assert_eq!(
        daemon_item.fields.as_ref().map(|f| f.state),
        Some(pb::TodoState::Closed as i32),
        "the item is closed on the daemon"
    );
    assert!(
        daemon_item
            .item
            .as_ref()
            .is_some_and(|item| item.pending.is_empty()),
        "no pending ask remains on the item"
    );
    assert!(requests.is_empty(), "the request left the pending list");

    drop(app);
    drop(world);
}

/// Answering No to an agent's ask from the To-do item drawer fails the agent's
/// call as denied, removes the pending request, and keeps the item open.
#[test]
fn denying_an_agents_ask_from_the_todo_drawer_keeps_the_item_open_and_fails_the_call() {
    let runtime = runtime();
    let (world, clients, key) = start_ask_scenario(&runtime);
    let mut app = App::open(&runtime, clients);

    send(&mut app, AppAction::Switch(Route::Todo));
    app.wait_until("the to-do item to load", |app| {
        app.ui
            .read(|s| s.todo.loaded && s.todo.items.iter().any(|e| e.todo.key == key))
    });

    let agent = AgentMove::move_to_done(&world, &key);
    app.wait_until("the agent's ask to appear on the to-do item", |app| {
        app.ui.read(|s| {
            s.todo
                .items
                .iter()
                .any(|e| e.todo.key == key && e.ask_id.is_some() && e.todo.ask.is_some())
        })
    });

    let row_id = format!("todo-row {key}");
    app.wait_until("the to-do row to draw", |app| draws(app, &row_id));
    click_id(&mut app, &row_id);
    app.wait_until("the No button in the drawer", |app| draws(app, "todo-no"));
    click_id(&mut app, "todo-no");

    app.wait_until("the agent's call to finish", |_| agent.is_done());
    let output = agent.take();
    assert_eq!(
        output.status.code(),
        Some(EXIT_DENIED),
        "denied agent call should exit {EXIT_DENIED}; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    app.wait_until("the item to stay open and lose its ask in the app", |app| {
        app.ui.read(|s| {
            s.todo.items.iter().any(|e| {
                e.todo.key == key
                    && e.ask_id.is_none()
                    && e.todo.ask.is_none()
                    && format!("{:?}", e.todo.state) != "Closed"
            })
        })
    });

    let (daemon_item, requests) = runtime.block_on(async {
        (
            daemon_todo(&world, &key).await.expect("item still on daemon"),
            pending_requests(&world).await,
        )
    });
    assert_ne!(
        daemon_item.fields.as_ref().map(|f| f.state),
        Some(pb::TodoState::Closed as i32),
        "the item stays open on the daemon"
    );
    assert!(
        daemon_item
            .item
            .as_ref()
            .is_some_and(|item| item.pending.is_empty()),
        "no pending ask remains on the item"
    );
    assert!(requests.is_empty(), "the request left the pending list");

    drop(app);
    drop(world);
}

/// Clicking "Start a session on this" in a To-do item's drawer opens the
/// prefilled New session sheet, starts the session, attaches it to the item,
/// and moves the item to In progress.
#[test]
fn starting_a_session_from_the_todo_drawer_opens_it_and_moves_the_item_in_progress() {
    let runtime = runtime();
    let (world, clients, channel, key) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, _repo) = channel_with_repo(&world, &clients, "payments").await;
        let created = clients
            .todo
            .create(NewTodo {
                title: "Schema".to_owned(),
                channel: Some(channel.as_str().to_owned()),
                ..NewTodo::default()
            })
            .await
            .expect("create the to-do item");
        (world, clients, channel, created.todo.key)
    });

    let mut app = App::open(&runtime, clients);
    let item_id = ItemId::new(key.as_str());
    app.wait_until("the channel, host and item to load", |app| {
        app.ui.read(|s| {
            s.workspace
                .channel_machines(&channel)
                .any(|m| m.online)
                && s.workspace.items.iter().any(|i| i.id == item_id)
        })
    });

    send(&mut app, AppAction::Switch(Route::Todo));
    app.wait_until("the to-do item to load on the To-do page", |app| {
        app.ui
            .read(|s| s.todo.loaded && s.todo.items.iter().any(|e| e.todo.key == key))
    });

    let row_id = format!("todo-row {key}");
    app.wait_until("the to-do row to draw", |app| draws(app, &row_id));
    click_id(&mut app, &row_id);
    app.wait_until("the Start a session button in the drawer", |app| {
        draws(app, "todo-start-session")
    });
    click_id(&mut app, "todo-start-session");

    app.wait_until("the prefilled New session sheet to open", |app| {
        app.sheet_holds(|sheet| {
            sheet.title == "Schema"
                && sheet.item.as_ref() == Some(&item_id)
                && sheet.channel.as_ref() == Some(&channel)
                && sheet.machine.is_some()
        })
    });

    app.wait_until("the Start session button to draw", |app| {
        draws(app, "new-session-create")
    });
    click_id(&mut app, "new-session-create");

    app.wait_until("the new session to be listed", |app| {
        app.session_titled("Schema").is_some()
    });
    let session = app
        .session_titled("Schema")
        .expect("listed after the wait");
    app.wait_until("the new session's page to open", |app| {
        app.ui
            .read(|s| s.route == Route::Session(session.id.clone()))
    });
    assert!(
        !app.ui
            .read(|s| matches!(s.overlays.last(), Some(Overlay::NewSession(_)))),
        "the New session sheet closes once the session starts"
    );

    app.wait_until("the item to move to In progress with the session attached", |app| {
        app.runtime.block_on(async {
            daemon_todo(&world, &key).await.is_some_and(|entry| {
                entry.fields.as_ref().map(|f| f.state) == Some(pb::TodoState::InProgress as i32)
                    && entry.item.as_ref().is_some_and(|item| {
                        item.sessions
                            .iter()
                            .any(|s| s.value == session.id.as_str())
                    })
            })
        })
    });

    drop(app);
    drop(world);
}

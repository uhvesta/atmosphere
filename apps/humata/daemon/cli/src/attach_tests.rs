use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use humata_wire::{Client, Reply, Service, ServiceFuture, WireError, pb, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex, split};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::{AttachError, Ended, RESET, Size, Target, attach};

type Items = mpsc::Sender<Result<pb::StreamItem, pb::Error>>;

/// A host with one terminal, "shell": each attach hands the test the
/// sender of its output, and what's typed and each resize are reported.
/// Typing takes a while to arrive, as over a slow link.
struct Host {
    attached: mpsc::UnboundedSender<Items>,
    typed: mpsc::UnboundedSender<Vec<u8>>,
    resized: mpsc::UnboundedSender<(u32, u32)>,
}

impl Service for Host {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            let unknown = || {
                Reply::Response(humata_wire::error(
                    pb::ErrorCode::UnknownTerminal,
                    "no such terminal",
                ))
            };
            match request {
                pb::request::Kind::AttachTerminal(attach) if attach.terminal_id == "shell" => {
                    let (items, receiver) = mpsc::channel(8);
                    let _ = self.attached.send(items);
                    Reply::Stream {
                        response: pb::response::Kind::Watching(pb::Watching {}),
                        items: receiver,
                    }
                }
                pb::request::Kind::TerminalInput(input) if input.terminal_id == "shell" => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let _ = self.typed.send(input.data);
                    Reply::Response(pb::response::Kind::TerminalInputSent(
                        pb::TerminalInputSent {},
                    ))
                }
                pb::request::Kind::ResizeTerminal(resize) if resize.terminal_id == "shell" => {
                    let _ = self.resized.send((resize.rows, resize.columns));
                    Reply::Response(pb::response::Kind::TerminalResized(pb::TerminalResized {}))
                }
                _ => unknown(),
            }
        })
    }
}

async fn connected(host: Arc<Host>) -> Result<Client, WireError> {
    let (ours, theirs) = duplex(64 * 1024);
    let (reader, writer) = split(theirs);
    tokio::spawn(async move { serve(reader, writer, pb::Role::Daemon, "test", host).await });
    let (reader, writer) = split(ours);
    Client::connect(reader, writer, pb::Role::Client, "test").await
}

/// An attach running against a [`Host`], and the ends the test drives.
struct Running {
    ended: JoinHandle<Result<Ended, AttachError>>,
    keyboard: DuplexStream,
    screen: DuplexStream,
    sizes: watch::Sender<Option<Size>>,
    attached: mpsc::UnboundedReceiver<Items>,
    typed: mpsc::UnboundedReceiver<Vec<u8>>,
    resized: mpsc::UnboundedReceiver<(u32, u32)>,
    /// How many connections the attach made.
    connections: Arc<AtomicUsize>,
}

fn start(terminal: &str) -> Running {
    let (attached, attached_receiver) = mpsc::unbounded_channel();
    let (typed, typed_receiver) = mpsc::unbounded_channel();
    let (resized, resized_receiver) = mpsc::unbounded_channel();
    let host = Arc::new(Host {
        attached,
        typed,
        resized,
    });
    let (keyboard, input) = duplex(1024);
    let (output, screen) = duplex(1024 * 1024);
    let (sizes, sizes_receiver) = watch::channel(Some(Size {
        rows: 24,
        columns: 80,
    }));
    let target = Target {
        host: String::new(),
        terminal: terminal.to_owned(),
    };
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = connections.clone();
    let ended = tokio::spawn(async move {
        attach(
            &target,
            || {
                counted.fetch_add(1, Ordering::Relaxed);
                connected(host.clone())
            },
            input,
            output,
            sizes_receiver,
            Duration::from_millis(10),
        )
        .await
    });
    Running {
        ended,
        keyboard,
        screen,
        sizes,
        attached: attached_receiver,
        typed: typed_receiver,
        resized: resized_receiver,
        connections,
    }
}

fn output(data: &[u8], replay: bool) -> Result<pb::StreamItem, pb::Error> {
    Ok(pb::StreamItem {
        kind: Some(pb::stream_item::Kind::TerminalOutput(pb::TerminalOutput {
            data: data.to_vec(),
            replay,
            ..Default::default()
        })),
    })
}

fn exit(code: i32) -> Result<pb::StreamItem, pb::Error> {
    Ok(pb::StreamItem {
        kind: Some(pb::stream_item::Kind::TerminalOutput(pb::TerminalOutput {
            exited: true,
            exit_code: code,
            ..Default::default()
        })),
    })
}

async fn next<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
    match tokio::time::timeout(Duration::from_secs(5), receiver.recv()).await {
        Ok(Some(value)) => value,
        Ok(None) => panic!("the host stopped before it was sent"),
        Err(_) => panic!("timed out waiting for the host"),
    }
}

async fn send(items: &Items, item: Result<pb::StreamItem, pb::Error>) {
    if items.send(item).await.is_err() {
        panic!("the attach stopped listening to the host");
    }
}

async fn finish(running: Running) -> (Result<Ended, AttachError>, Vec<u8>) {
    let Running {
        ended, mut screen, ..
    } = running;
    let ended = match tokio::time::timeout(Duration::from_secs(5), ended).await {
        Ok(Ok(ended)) => ended,
        Ok(Err(error)) => panic!("the attach panicked: {error}"),
        Err(_) => panic!("the attach didn't end"),
    };
    let mut shown = Vec::new();
    if let Err(error) = screen.read_to_end(&mut shown).await {
        panic!("couldn't read what was shown: {error}");
    }
    (ended, shown)
}

#[tokio::test]
async fn shows_the_replay_after_a_reset_then_what_follows_and_ends_with_the_exit_code() {
    let mut running = start("shell");
    let items = next(&mut running.attached).await;
    send(&items, output(b"$ ls\r\n", true)).await;
    send(&items, output(b"README\r\n", false)).await;
    send(&items, exit(3)).await;
    let (ended, shown) = finish(running).await;
    assert!(matches!(ended, Ok(Ended::Exited(3))));
    assert_eq!(shown, [RESET, b"$ ls\r\n", b"README\r\n"].concat());
}

#[tokio::test]
async fn sends_keys_in_the_order_they_were_typed() {
    let mut running = start("shell");
    let _items = next(&mut running.attached).await;
    for key in [b"e", b"c", b"h", b"o"] {
        if running.keyboard.write_all(key).await.is_err() {
            panic!("couldn't type");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut typed = Vec::new();
    while typed.len() < 4 {
        typed.extend(next(&mut running.typed).await);
    }
    assert_eq!(typed, b"echo");
}

#[tokio::test]
async fn sends_its_size_on_attach_then_the_last_of_quick_changes() {
    let mut running = start("shell");
    let _items = next(&mut running.attached).await;
    assert_eq!(next(&mut running.resized).await, (24, 80));
    for (rows, columns) in [(30, 100), (31, 101)] {
        let _ = running.sizes.send(Some(Size { rows, columns }));
    }
    assert_eq!(next(&mut running.resized).await, (31, 101));
}

#[tokio::test]
async fn reattaches_after_the_connection_drops() {
    let mut running = start("shell");
    let first = next(&mut running.attached).await;
    send(&first, output(b"before", true)).await;
    drop(first);
    let second = next(&mut running.attached).await;
    send(&second, output(b"after", true)).await;
    send(&second, exit(0)).await;
    let (ended, shown) = finish(running).await;
    assert!(matches!(ended, Ok(Ended::Exited(0))));
    // The replay after reattaching clears the note about the lost
    // connection, so what follows the last reset is the host's alone.
    let last_reset = shown
        .windows(RESET.len())
        .rposition(|window| window == RESET);
    assert_eq!(
        last_reset.map(|start| &shown[start + RESET.len()..]),
        Some(&b"after"[..])
    );
}

#[tokio::test]
async fn falling_behind_attaches_again_on_the_same_connection() {
    let mut running = start("shell");
    let first = next(&mut running.attached).await;
    // The screen isn't read while the host prints far more than the attach
    // can hold, so it falls behind.
    let line = vec![b'x'; 1024];
    for _ in 0..4096 {
        let _ = first.send(output(&line, false)).await;
    }
    let Running {
        ended,
        mut screen,
        mut attached,
        connections,
        ..
    } = running;
    let shown = tokio::spawn(async move {
        let mut shown = Vec::new();
        let _ = screen.read_to_end(&mut shown).await;
        shown
    });
    let second = next(&mut attached).await;
    send(&second, output(b"after", true)).await;
    send(&second, exit(0)).await;
    let ended = match tokio::time::timeout(Duration::from_secs(5), ended).await {
        Ok(Ok(ended)) => ended,
        Ok(Err(error)) => panic!("the attach panicked: {error}"),
        Err(_) => panic!("the attach didn't end"),
    };
    assert!(matches!(ended, Ok(Ended::Exited(0))));
    assert_eq!(connections.load(Ordering::Relaxed), 1);
    let shown = match shown.await {
        Ok(shown) => shown,
        Err(error) => panic!("couldn't read what was shown: {error}"),
    };
    assert!(shown.ends_with(&[RESET, b"after"].concat()));
}

#[tokio::test]
async fn a_terminal_that_is_gone_ends_the_attach() {
    let running = start("closed");
    let (ended, _) = finish(running).await;
    assert!(matches!(ended, Err(AttachError::Gone { .. })));
}

#[tokio::test]
async fn ends_when_the_keyboard_closes() {
    let mut running = start("shell");
    let _items = next(&mut running.attached).await;
    let Running {
        ended,
        keyboard,
        screen,
        ..
    } = running;
    drop(keyboard);
    drop(screen);
    let ended = match tokio::time::timeout(Duration::from_secs(5), ended).await {
        Ok(Ok(ended)) => ended,
        Ok(Err(error)) => panic!("the attach panicked: {error}"),
        Err(_) => panic!("the attach didn't end"),
    };
    assert!(matches!(ended, Ok(Ended::InputClosed)));
}

#[test]
fn passes_exit_codes_that_fit_and_fails_others() {
    assert_eq!(super::exit_status(0), 0);
    assert_eq!(super::exit_status(130), 130);
    assert_eq!(super::exit_status(-1), 1);
    assert_eq!(super::exit_status(300), 1);
}

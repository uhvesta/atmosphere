//! A tab's terminal link on the fake: its screen as a real checkpoint, then
//! the lease, then an answer to each resize, all in order.

use std::sync::Arc;
use std::time::Duration;

use ghostty_vt::{Point, Terminal};
use humata2_clients::model::SessionId;
use humata2_clients::{DaemonError, FakeDaemon, SendError, Terminals, TerminalLink, fixture};
use tca::TestClock;
use terminal_wire::{ClientFrame, SNAPSHOT_ENGINE, ServerFrame};
use tokio::runtime::Builder;

/// A guard against hanging, never what a test waits on.
const GUARD: Duration = Duration::from_secs(5);

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn daemon() -> FakeDaemon {
    FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()))
}

/// A session with something printed in its first tab, and that first line.
fn printed(daemon: &FakeDaemon) -> (SessionId, String) {
    let workspace = daemon.workspace();
    workspace
        .sessions
        .iter()
        .find_map(|session| {
            let line = session.transcript.first()?.text.lines().next()?.trim();
            (!line.is_empty()).then(|| (session.id.clone(), line.to_owned()))
        })
        .expect("a fixture session has a transcript")
}

async fn next(link: &mut TerminalLink) -> ServerFrame {
    match tokio::time::timeout(GUARD, link.frames.recv()).await {
        Ok(Some(frame)) => frame,
        other => panic!("the link gives a frame: {other:?}"),
    }
}

/// The next frame that answers the view. What the program prints meanwhile,
/// an echo or a redraw, may come first.
async fn answer(link: &mut TerminalLink) -> ServerFrame {
    loop {
        match next(link).await {
            ServerFrame::Output { .. } => {}
            frame => return frame,
        }
    }
}

fn resize(cols: u16, rows: u16) -> ClientFrame {
    ClientFrame::Resize {
        id: 0,
        cols,
        rows,
        cell_width: 8,
        cell_height: 16,
    }
}

#[test]
fn the_first_frame_is_a_screen_a_terminal_restores_with_the_tab_text() {
    let daemon = daemon();
    let (session, line) = printed(&daemon);
    let mut link = block_on(daemon.link(&session, 0)).expect("the link opens");
    let ServerFrame::Snapshot {
        engine,
        bytes,
        cols,
        rows,
        live,
        ..
    } = block_on(next(&mut link))
    else {
        panic!("the first frame is the screen");
    };
    assert_eq!(engine, SNAPSHOT_ENGINE);
    assert!(live);
    let restored = Terminal::from_checkpoint(&bytes).expect("the checkpoint decodes");
    assert_eq!((restored.size().cols, restored.size().rows), (cols, rows));
    let last = u32::try_from(restored.total_rows()).expect("rows fit") - 1;
    let text = restored
        .selection_text(Point { x: 0, y: 0 }, Point { x: cols - 1, y: last })
        .expect("the screen reads");
    assert!(text.contains(&line), "the screen shows {line:?}, got {text:?}");
}

#[test]
fn the_screen_is_followed_by_the_lease() {
    let daemon = daemon();
    let (session, _) = printed(&daemon);
    let mut link = block_on(daemon.link(&session, 0)).expect("the link opens");
    assert!(matches!(
        block_on(next(&mut link)),
        ServerFrame::Snapshot { .. }
    ));
    assert!(matches!(
        block_on(next(&mut link)),
        ServerFrame::Control {
            controller: true,
            ..
        }
    ));
}

#[test]
fn each_resize_is_answered_with_that_size_in_the_order_sent() {
    let daemon = daemon();
    let (session, _) = printed(&daemon);
    let mut link = block_on(daemon.link(&session, 0)).expect("the link opens");
    block_on(next(&mut link));
    block_on(next(&mut link));
    link.sink.send(resize(120, 40)).expect("the first resize is sent");
    link.sink.send(resize(90, 20)).expect("the second resize is sent");
    for (want_cols, want_rows) in [(120, 40), (90, 20)] {
        let ServerFrame::Resized { cols, rows, .. } = block_on(answer(&mut link)) else {
            panic!("a resize is answered with Resized");
        };
        assert_eq!((cols, rows), (want_cols, want_rows));
    }
}

#[test]
fn typed_input_is_confirmed_with_its_sequence() {
    let daemon = daemon();
    let (session, _) = printed(&daemon);
    let mut link = block_on(daemon.link(&session, 0)).expect("the link opens");
    block_on(next(&mut link));
    block_on(next(&mut link));
    link.sink
        .send(ClientFrame::Input {
            id: 0,
            seq: 7,
            bytes: b"ls\r".to_vec(),
        })
        .expect("input is sent");
    assert!(matches!(
        block_on(answer(&mut link)),
        ServerFrame::InputWritten { seq: 7, .. }
    ));
}

#[test]
fn sending_after_the_view_dropped_its_end_fails_as_closed() {
    let daemon = daemon();
    let (session, _) = printed(&daemon);
    let link = block_on(daemon.link(&session, 0)).expect("the link opens");
    let sink = link.sink.clone();
    drop(link);
    assert_eq!(sink.send(resize(80, 24)), Err(SendError::Closed));
}

#[test]
fn a_tab_that_does_not_exist_has_no_link() {
    let daemon = daemon();
    let (session, _) = printed(&daemon);
    let result = block_on(daemon.link(&session, 99));
    assert!(matches!(result, Err(DaemonError::NotFound { .. })));
}

#[test]
fn ending_a_tabs_links_closes_them_after_what_was_sent() {
    let daemon = daemon();
    let (session, _) = printed(&daemon);
    let mut link = block_on(daemon.link(&session, 0)).expect("the link opens");
    daemon.end_terminal_links(&session, 0);
    let ended = block_on(async {
        loop {
            match tokio::time::timeout(GUARD, link.frames.recv()).await {
                Ok(Some(_)) => {}
                Ok(None) => return true,
                Err(_) => return false,
            }
        }
    });
    assert!(ended, "the link closes");
    assert_eq!(link.sink.send(resize(80, 24)), Err(SendError::Closed));
}

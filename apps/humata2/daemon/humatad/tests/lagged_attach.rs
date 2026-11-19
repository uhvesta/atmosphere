//! An attach that falls behind is started over from a fresh screen (D271):
//! its size changes can't be queued for it without end, so the daemon
//! sends a new checkpoint instead of skipping bytes or ending the attach.

use humatad_test_common as common;

use common::{Attach, Daemon, open};
use ghostty_vt::Terminal;
use humata2_grpc::humata::v1 as pb;
use pb::attach_client_message::Message as ClientMessage;

/// More size changes than the daemon queues for one attach.
const RESIZES: u32 = 40;

fn checkpointing(tab: &pb::TabId) -> pb::AttachOpen {
    pb::AttachOpen {
        checkpoint_engine: ghostty_vt::CHECKPOINT_FORMAT.to_owned(),
        // Never run out of window: the attach must be held up by a client
        // that doesn't read, not by one that doesn't acknowledge.
        window_bytes: u32::MAX,
        ..open(tab, 0)
    }
}

// About 8 MB of lines, then a word the flood's own echo can't contain.
const FLOOD: &str = "yes 0123456789abcdefghijklmnopqrstuvwxyz | head -n 200000; echo flood-e''nd\n";

#[tokio::test(flavor = "multi_thread")]
async fn an_attach_that_fell_behind_is_resynced_with_the_current_screen() {
    let daemon = Daemon::start().await;
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut typist = Attach::start(
        &daemon,
        pb::AttachOpen {
            cols: 100,
            rows: 30,
            ..checkpointing(&tab)
        },
    )
    .await;
    let mut viewer = Attach::start(&daemon, checkpointing(&tab)).await;
    viewer.wait_until(|viewer| viewer.snapshots == 1).await;

    // The viewer reads nothing while the shell prints, so its attach is
    // held up with output it can't send; then the size changes many times.
    typist.type_text(FLOOD).await;
    typist.wait_for("flood-end").await;
    for step in 0..RESIZES {
        typist
            .send(ClientMessage::Resize(pb::TerminalResize {
                cols: 60 + step,
                rows: 20,
                cell_width: 9,
                cell_height: 18,
            }))
            .await;
    }
    let last = 60 + RESIZES - 1;
    typist
        .wait_until(|typist| typist.states.last().is_some_and(|state| state.cols == last))
        .await;

    // Reading again, the viewer is sent a new screen, not a gap: `read`
    // fails the test if output skips or repeats a byte, and the stream
    // ending fails it too.
    // A resync during the flood may come before its end was printed, so
    // the new screen plus the output after it must show the end; a later
    // resync starts that screen over.
    let mut seen = 1;
    'resync: loop {
        viewer.wait_until(|viewer| viewer.snapshots > seen).await;
        seen = viewer.snapshots;
        let mut screen =
            Terminal::from_checkpoint(&viewer.screen).expect("a checkpoint to install");
        let mut fed = viewer.screen.len();
        while screen.search("flood-end").expect("search").is_empty() {
            viewer.read().await.unwrap();
            if viewer.snapshots != seen {
                seen = viewer.snapshots - 1;
                continue 'resync;
            }
            screen.feed(&viewer.screen[fed..]);
            fed = viewer.screen.len();
        }
        break;
    }
    viewer
        .wait_until(|viewer| viewer.states.last().is_some_and(|state| state.cols == last))
        .await;

    // The attach is still open and carries on from the new screen.
    viewer.type_text("echo aft''er\n").await;
    viewer.wait_for("after").await;
}

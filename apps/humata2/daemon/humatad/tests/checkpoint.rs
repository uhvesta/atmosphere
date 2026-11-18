//! Attaches that ask for the daemon's own screen of a tab (D271): a
//! checkpoint to install, the screen's size in the state, and a screen that
//! answers the program's queries whether or not anyone is watching.

mod common;

use common::{Attach, Daemon, error_of, invalid_field, open};
use ghostty_vt::Terminal;
use humata2_grpc::humata::v1 as pb;
use pb::attach_client_message::Message as ClientMessage;
use tonic::Code;

async fn machine_shell(daemon: &Daemon) -> pb::TabId {
    daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap()
}

fn checkpointing(tab: &pb::TabId, from_offset: u64) -> pb::AttachOpen {
    pb::AttachOpen {
        checkpoint_engine: ghostty_vt::CHECKPOINT_FORMAT.to_owned(),
        ..open(tab, from_offset)
    }
}

// Quoting splits the word, so the shell's echo of the command line
// doesn't count as its output.
const SAY_HELLO: &str = "echo hel''lo\n";

// Asks the terminal where its cursor is, reads four bytes of the answer
// with the line discipline out of the way, and says "got" if they came.
const ASK_WHERE: &str =
    "stty raw -echo; printf '\\033[6n'; dd bs=1 count=4 >/dev/null 2>&1; stty sane; echo go''t\n";

#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_is_a_checkpoint_that_shows_what_was_printed() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut printer = Attach::start(&daemon, open(&tab, 0)).await;
    printer.type_text(SAY_HELLO).await;
    printer.wait_for("hello").await;

    let mut viewer = Attach::start(&daemon, checkpointing(&tab, 0)).await;
    viewer.wait_until(|viewer| viewer.snapshots == 1).await;

    let mut screen = Terminal::from_checkpoint(&viewer.screen).expect("a checkpoint to install");
    assert!(!screen.search("hello").expect("search").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn output_after_a_checkpoint_continues_from_its_offset() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut viewer = Attach::start(&daemon, checkpointing(&tab, 0)).await;
    viewer.wait_until(|viewer| viewer.snapshots == 1).await;
    let installed_at = viewer.cursor;

    viewer.type_text(SAY_HELLO).await;
    viewer.wait_until(|viewer| viewer.cursor > installed_at).await;

    // `Attach::read` fails the test if output skips or repeats a byte.
    assert!(viewer.cursor > installed_at);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_nobody_is_watching_for_is_answered() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut typist = Attach::start(&daemon, checkpointing(&tab, 0)).await;
    let seq = typist.type_text(ASK_WHERE).await;
    typist
        .wait_until(|typist| typist.accepted.contains(&seq))
        .await;
    drop(typist);

    // Nothing here answers the terminal: the daemon's screen did.
    let mut watcher = Attach::start(&daemon, open(&tab, 0)).await;
    watcher.wait_for("got").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resize_by_one_attach_reaches_the_others() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut resizer = Attach::start(
        &daemon,
        pb::AttachOpen {
            cols: 100,
            rows: 30,
            ..checkpointing(&tab, 0)
        },
    )
    .await;
    let mut watcher = Attach::start(&daemon, checkpointing(&tab, 0)).await;
    resizer.wait_until(|resizer| !resizer.states.is_empty()).await;
    watcher.wait_until(|watcher| !watcher.states.is_empty()).await;
    assert_eq!(
        (resizer.states[0].cols, resizer.states[0].rows),
        (100, 30),
        "the size the attach opened at is the screen's"
    );

    resizer
        .send(ClientMessage::Resize(pb::TerminalResize {
            cols: 90,
            rows: 20,
            cell_width: 9,
            cell_height: 18,
        }))
        .await;
    watcher
        .wait_until(|watcher| {
            watcher
                .states
                .iter()
                .any(|state| (state.cols, state.rows) == (90, 20))
        })
        .await;

    let state = watcher
        .states
        .iter()
        .find(|state| (state.cols, state.rows) == (90, 20))
        .unwrap();
    assert_eq!((state.cell_width, state.cell_height), (9, 18));
    // The shell has the new size too.
    watcher.type_text("stty size\n").await;
    watcher.wait_for("20 90").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_format_the_daemon_does_not_make_is_refused() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    let refused = match Attach::try_start(
        &daemon,
        pb::AttachOpen {
            checkpoint_engine: "some-other-engine/9".to_owned(),
            ..open(&tab, 0)
        },
    )
    .await
    {
        Ok(_) => panic!("an engine the daemon doesn't make was accepted"),
        Err(status) => status,
    };

    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(matches!(
        error_of(&refused).kind,
        Some(pb::error::Kind::PeerVersionUnsupported(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attach_without_an_engine_gets_raw_output_and_no_screen_size() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.type_text(SAY_HELLO).await;
    attach.wait_for("hello").await;

    let state = attach.states.first().unwrap();
    assert_eq!((state.cols, state.rows), (0, 0));
    assert_eq!(attach.snapshots, 1);
}

/// Whether `text` has the shape of a hyphenated UUID: 36 characters, hex
/// digits, and hyphens after the 8th, 13th, 18th and 23rd.
fn looks_like_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(index, c)| match index {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_checkpoint_names_the_terminal_and_keeps_the_name_across_attaches() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    let mut first = Attach::start(&daemon, checkpointing(&tab, 0)).await;
    first.wait_until(|attach| attach.snapshots == 1).await;
    let mut second = Attach::start(&daemon, checkpointing(&tab, 0)).await;
    second.wait_until(|attach| attach.snapshots == 1).await;
    // A client that takes snapshots in pieces gets the same name in each.
    let mut piecewise = Attach::start(
        &daemon,
        pb::AttachOpen {
            accept_codecs: humata2_chunk::Codec::wire_list(&[humata2_chunk::Codec::Lz4]),
            ..checkpointing(&tab, 0)
        },
    )
    .await;
    piecewise.wait_until(|attach| attach.snapshots == 1).await;
    let mut raw = Attach::start(&daemon, open(&tab, 0)).await;
    raw.wait_until(|attach| attach.snapshots == 1).await;

    assert!(
        looks_like_uuid(&first.terminal_uuid),
        "a checkpoint names the terminal: {:?}",
        first.terminal_uuid
    );
    assert_eq!(second.terminal_uuid, first.terminal_uuid);
    assert_eq!(piecewise.terminal_uuid, first.terminal_uuid);
    assert!(raw.terminal_uuid.is_empty(), "a raw snapshot names no terminal");
}

// Prints 6000 lines in which every cell has a color of its own, so the
// screen's checkpoint is several MiB: more than one gRPC message carries.
const PAINT_A_LOT: &str = "awk 'BEGIN{for(i=0;i<6000;i++){s=\"\";for(j=0;j<190;j++)s=s sprintf(\"\\033[3%dm%c\",(i+j)%7+1,65+(i+j)%26);print s}}'; printf '\\033[0m'; echo pain''ted\n";

/// A tab whose screen has been painted by `PAINT_A_LOT`.
async fn painted(daemon: &Daemon) -> pb::TabId {
    let tab = machine_shell(daemon).await;
    let mut printer = Attach::start(
        daemon,
        pb::AttachOpen {
            cols: 200,
            rows: 50,
            window_bytes: 64 << 20,
            ..open(&tab, 0)
        },
    )
    .await;
    printer.type_text(PAINT_A_LOT).await;
    printer.wait_for("painted").await;
    tab
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_bigger_than_a_message_comes_in_pieces_and_installs() {
    let daemon = Daemon::start().await;
    let tab = painted(&daemon).await;

    let mut viewer = Attach::start(
        &daemon,
        pb::AttachOpen {
            accept_codecs: vec![pb::ChunkCodec::None as i32],
            ..checkpointing(&tab, 0)
        },
    )
    .await;
    viewer.wait_until(|viewer| viewer.snapshots == 1).await;

    // One message carries at most 3 MiB of screen (the attach's WHOLE_MAX,
    // under gRPC's 4 MiB with room for the envelope).
    assert!(
        viewer.screen.len() > 3 << 20,
        "the painted screen's checkpoint is bigger than one message: {} bytes",
        viewer.screen.len()
    );
    assert!(viewer.snapshot_pieces > 1, "it came in {} pieces", viewer.snapshot_pieces);
    let mut screen = Terminal::from_checkpoint(&viewer.screen).expect("a checkpoint to install");
    assert!(!screen.search("painted").expect("search").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_too_big_to_send_whole_is_refused_to_an_attach_that_takes_no_pieces() {
    let daemon = Daemon::start().await;
    let tab = painted(&daemon).await;

    let refused = match Attach::try_start(&daemon, checkpointing(&tab, 0)).await {
        Ok(_) => panic!("a checkpoint no message can carry was sent whole"),
        Err(status) => status,
    };

    assert_eq!(refused.code(), Code::InvalidArgument);
    assert_eq!(invalid_field(&refused).0, "open.accept_codecs");
}

//! A tab's screen answers the program's terminal queries once, across a
//! daemon restart (D271): it never answers a query it already answered, and
//! never drops one that came while no daemon was there to hear it. The
//! holders keep running while the daemon is away, so the shell sits waiting
//! for its answer.

use humatad_test_common as common;

use common::{Attach, Daemon, open};
use humata2_grpc::humata::v1 as pb;

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

// Asks the terminal where its cursor is, reads the answer through its
// final R with the line discipline out of the way, and says "got" once it
// came. Reading all of it leaves nothing behind for the next command.
// Quoting splits the word, so the shell's echo of the command line doesn't
// count as its output.
const ASK_WHERE: &str =
    "stty raw -echo; printf '\\033[6n'; \
until [ \"$(dd bs=1 count=1 2>/dev/null)\" = R ]; do :; done; stty sane; echo go''t\n";

// Asks once and counts every reply that arrives within two quiet seconds,
// so a second answer to anything before it shows as a count of two. Some
// wc pad the count with spaces, so they are taken out.
const COUNT_REPLIES: &str = "stty raw -echo min 0 time 20; printf '\\033[6n'; \
n=$(dd bs=1 count=64 2>/dev/null | tr -cd R | wc -c | tr -d ' '); stty sane; echo repl''ies=$n\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_query_answered_before_a_restart_is_not_answered_again() {
    let mut daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut typist = Attach::start(&daemon, open(&tab, 0)).await;
    typist.type_text(ASK_WHERE).await;
    typist.wait_for("got").await;
    drop(typist);

    daemon.restart().await;

    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.wait_for("got").await;
    after.type_text(COUNT_REPLIES).await;
    after.wait_for("replies=").await;
    assert!(
        after.shows("replies=1"),
        "the restarted daemon answered a query it had answered, or missed the new one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_that_came_while_the_daemon_was_away_is_answered_when_it_is_back() {
    let mut daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let go = daemon.dir.path().join("go");
    let mut typist = Attach::start(&daemon, open(&tab, 0)).await;
    // Waits for the file before it asks, so it asks while nobody is there.
    let seq = typist
        .type_text(&format!(
            "stty raw -echo; while [ ! -f '{}' ]; do sleep 0.1; done; printf '\\033[6n'; \
             until [ \"$(dd bs=1 count=1 2>/dev/null)\" = R ]; do :; done; stty sane; echo go''t\n",
            go.display()
        ))
        .await;
    typist.wait_until(|typist| typist.accepted.contains(&seq)).await;
    drop(typist);

    daemon.kill();
    std::fs::write(&go, b"").unwrap();
    daemon.restart().await;

    // Nobody here answers it: the restarted daemon's screen did, with no
    // attach open when it started.
    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.wait_for("got").await;
}

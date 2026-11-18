//! BUG-T1: return is pressed as a key of its own, after the program has
//! taken the text. The program is a stand-in for an agent's CLI (see
//! `fake/agent_tui.rs`) that reads its input from the start but acts on it
//! only once it's up, and takes a return that came with the text as part
//! of a paste. The test brings it up when a return reached it early, or
//! when none has after `EARLY`; either way only a return that waited for
//! the text submits.

use std::path::PathBuf;
use std::time::Duration;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use humata2_holder::{
    Attachment, Control, Event, HolderError, HolderSpec, Ready, Typed, attach, await_keys,
    deliver, shutdown, spawn, type_then_return,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// How long the test gives a return to arrive too early before it brings
/// the program up anyway. A return that waits for the text never arrives
/// before then, however long this is. Well inside the time a starting
/// program gets to turn line editing off (five seconds).
const EARLY: Duration = Duration::from_millis(1500);
/// Catches a hang only.
const GUARD: Duration = Duration::from_secs(60);

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable}"));
    std::fs::canonicalize(path).unwrap()
}

struct Agent {
    socket: PathBuf,
    gate: PathBuf,
    /// The holder's process, ended with the test however it ends.
    pid: u32,
    _dir: TempDir,
}

impl Agent {
    /// A holder running the stand-in, which starts with line editing on
    /// when `cooked`.
    fn start(cooked: bool) -> Self {
        Self::in_mode(if cooked { "cooked" } else { "raw" })
    }

    /// A holder running the stand-in in `mode` (see `fake/agent_tui.rs`).
    fn in_mode(mode: &str) -> Self {
        // Socket paths must be short, so not under the test's own temp.
        let dir = humata2_tmp_folder::folder("h2r");
        let gate = dir.path().join("gate");
        let made = std::process::Command::new("mkfifo").arg(&gate).status().unwrap();
        assert!(made.success(), "mkfifo failed");
        let spec = HolderSpec {
            holder: binary("HUMATA2_HOLDER"),
            socket: dir.path().join("t.sock"),
            program: binary("HUMATA2_FAKE_AGENT"),
            args: vec![gate.clone().into(), mode.into()],
            cwd: dir.path().to_owned(),
            env: Vec::new(),
            cols: 80,
            rows: 24,
            scrollback_bytes: 1 << 20,
            log: dir.path().join("holder.log"),
        };
        let pid = spawn(&spec).unwrap().pid;
        Self {
            socket: spec.socket,
            gate,
            pid,
            _dir: dir,
        }
    }

    /// Brings the stand-in up.
    async fn open_gate(&self) {
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || std::fs::write(gate, b"up"))
            .await
            .unwrap()
            .unwrap();
    }
}

impl Drop for Agent {
    /// A test that fails before its shutdown still ends its holder, and the
    /// holder ends the stand-in with it. Only while the pid is still this
    /// test's holder, so a reused pid is left alone.
    fn drop(&mut self) {
        let pid = self.pid.to_string();
        let ours = std::process::Command::new("/bin/ps")
            .args(["-ww", "-o", "args=", "-p", &pid])
            .output()
            .is_ok_and(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .contains(&format!("--socket {} ", self.socket.display()))
            });
        if ours {
            let _ = std::process::Command::new("/bin/kill").args(["-TERM", &pid]).status();
        }
    }
}

/// The program's output as it comes, read in a task of its own because
/// reading isn't cancel-safe.
struct Screen {
    output: tokio::sync::mpsc::UnboundedReceiver<String>,
    seen: String,
    /// Kept open: the holder stops sending output once a client's side
    /// closes.
    _control: Control,
}

impl Screen {
    fn watch(Attachment { mut events, control, .. }: Attachment) -> Self {
        let (sent, output) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok(Some(event)) = events.next().await {
                match event {
                    Event::Output { data, .. } => {
                        if sent.send(String::from_utf8_lossy(&data).into_owned()).is_err() {
                            return;
                        }
                    }
                    Event::Exited { .. } => return,
                    _ => {}
                }
            }
        });
        Self {
            output,
            seen: String::new(),
            _control: control,
        }
    }

    /// Whether the output holds `wanted` within `limit`.
    async fn shows(&mut self, wanted: &str, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        while !self.seen.contains(wanted) {
            match tokio::time::timeout_at(deadline, self.output.recv()).await {
                Ok(Some(piece)) => self.seen.push_str(&piece),
                Ok(None) | Err(_) => return false,
            }
        }
        true
    }
}

async fn text_then_return_submits(cooked: bool) {
    let agent = Agent::start(cooked);
    let mut screen = Screen::watch(attach(&agent.socket, 0).await.unwrap());
    let socket = agent.socket.clone();
    let typing = tokio::spawn(async move {
        type_then_return(&socket, b"hello there", Duration::from_millis(400)).await
    });
    // Either way the program comes up: what counts is what reached it.
    let early = screen.shows("early return", EARLY).await;
    agent.open_gate().await;
    let submitted = !early && screen.shows("submitted: hello there", GUARD).await;
    if submitted {
        tokio::time::timeout(GUARD, typing).await.unwrap().unwrap().unwrap();
    }
    let _ = shutdown(&agent.socket).await;
    assert!(
        submitted,
        "the text wasn't submitted; the program printed {:?}",
        screen.seen
    );
}

#[tokio::test]
async fn text_typed_while_the_terminal_edits_lines_is_submitted_once_the_program_is_up() {
    text_then_return_submits(true).await;
}

#[tokio::test]
async fn text_the_program_reads_before_its_up_is_submitted_once_it_shows_it() {
    text_then_return_submits(false).await;
}

/// BUG-T1b: Copilot in a folder it doesn't trust yet asks about it before
/// it takes the first message. A return pressed then answers that question
/// for the person and leaves the message unsent; it waits until the person
/// has answered and the program shows the text.
#[tokio::test]
async fn a_return_waits_while_the_program_asks_a_question_of_its_own() {
    let agent = Agent::in_mode("asks");
    let mut screen = Screen::watch(attach(&agent.socket, 0).await.unwrap());
    let typed = tokio::time::timeout(
        GUARD,
        type_then_return(&agent.socket, b"hello there", Duration::from_millis(400)),
    )
    .await
    .expect("typing hung")
    .expect("typing failed");
    let answered = screen.shows("question answered by a return", Duration::from_secs(1)).await;
    let Typed::Held(mut held) = typed else {
        let _ = shutdown(&agent.socket).await;
        panic!("return was pressed though the program never showed the text; it printed {:?}", screen.seen);
    };
    assert!(!answered, "the program's question was answered: {:?}", screen.seen);
    // The person answers; the program then shows the text.
    agent.open_gate().await;
    let pressed = tokio::time::timeout(GUARD, held.once_shown(GUARD))
        .await
        .expect("the held return hung")
        .expect("the held return failed");
    let submitted = screen.shows("submitted: hello there", GUARD).await;
    let _ = shutdown(&agent.socket).await;
    assert!(pressed && submitted, "the text wasn't submitted; the program printed {:?}", screen.seen);
}

/// A stand-in for the daemon's screen check: clear once the test says the
/// person answered.
fn ready_when(answered: &Arc<AtomicBool>) -> Ready {
    let answered = answered.clone();
    Arc::new(move || {
        let clear = answered.load(Ordering::SeqCst);
        Box::pin(async move { clear })
    })
}

/// Types `hello there` while the program asks, then has the person answer
/// and [`deliver`] see it through. Returns what the program printed.
async fn answered_after_typing(mode: &str) -> (bool, String) {
    let agent = Agent::in_mode(mode);
    let mut screen = Screen::watch(attach(&agent.socket, 0).await.unwrap());
    let typed = tokio::time::timeout(
        GUARD,
        type_then_return(&agent.socket, b"hello there", Duration::from_millis(400)),
    )
    .await
    .expect("typing hung")
    .expect("typing failed");
    let Typed::Held(held) = typed else {
        let _ = shutdown(&agent.socket).await;
        panic!("return was pressed though the program never showed the text; it printed {:?}", screen.seen);
    };
    let answered = Arc::new(AtomicBool::new(false));
    let socket = agent.socket.clone();
    let ready = ready_when(&answered);
    let delivering = tokio::spawn(async move {
        deliver(&socket, b"hello there", Duration::from_millis(400), Some(held), Some(ready), GUARD).await
    });
    agent.open_gate().await;
    answered.store(true, Ordering::SeqCst);
    let sent = tokio::time::timeout(GUARD * 2, delivering)
        .await
        .expect("delivering hung")
        .unwrap()
        .expect("delivering failed");
    // Anything typed or pressed twice shows by now.
    screen.shows("submitted: hello there", Duration::from_secs(2)).await;
    let _ = screen.shows("never printed", Duration::from_secs(2)).await;
    let _ = shutdown(&agent.socket).await;
    (sent, screen.seen)
}

/// A program that drops what was typed while it asked gets the text typed
/// again once its input is clear, and it arrives exactly once.
#[tokio::test]
async fn a_text_the_program_dropped_while_it_asked_arrives_once() {
    let (sent, seen) = answered_after_typing("eats").await;
    assert!(sent, "deliver says the text wasn't sent; the program printed {seen:?}");
    assert_eq!(seen.matches("submitted: ").count(), 1, "the program printed {seen:?}");
    assert!(seen.contains("submitted: hello there\r"), "the program printed {seen:?}");
    assert!(!seen.contains("question answered by a return"), "the program printed {seen:?}");
}

/// A program that kept what was typed while it asked shows it once its
/// input is clear: it is submitted as it is, not typed a second time.
#[tokio::test]
async fn a_text_the_program_kept_while_it_asked_is_not_typed_twice() {
    let (sent, seen) = answered_after_typing("asks").await;
    assert!(sent, "deliver says the text wasn't sent; the program printed {seen:?}");
    assert_eq!(seen.matches("submitted: ").count(), 1, "the program printed {seen:?}");
    assert!(seen.contains("submitted: hello there\r"), "the program printed {seen:?}");
}

/// D763 (BUG-C): a screen read while the program asks can look clear for a
/// moment, as one taken while it redraws its question does. A text it kept
/// behind the question is still not typed again: once the person answers,
/// it is submitted exactly once.
#[tokio::test]
async fn a_text_kept_behind_a_question_that_looked_clear_for_a_moment_is_submitted_once() {
    let agent = Agent::in_mode("asks");
    let mut screen = Screen::watch(attach(&agent.socket, 0).await.unwrap());
    let typed = tokio::time::timeout(
        GUARD,
        type_then_return(&agent.socket, b"hello there", Duration::from_millis(400)),
    )
    .await
    .expect("typing hung")
    .expect("typing failed");
    let Typed::Held(held) = typed else {
        let _ = shutdown(&agent.socket).await;
        panic!("return was pressed though the program never showed the text; it printed {:?}", screen.seen);
    };
    // Clear on the first look only, then asking until the person answers.
    let answered = Arc::new(AtomicBool::new(false));
    let looked = Arc::new(AtomicBool::new(false));
    let ready: Ready = {
        let (answered, looked) = (answered.clone(), looked.clone());
        Arc::new(move || {
            let clear = !looked.swap(true, Ordering::SeqCst) || answered.load(Ordering::SeqCst);
            Box::pin(async move { clear })
        })
    };
    let socket = agent.socket.clone();
    let delivering = tokio::spawn(async move {
        deliver(&socket, b"hello there", Duration::from_millis(400), Some(held), Some(ready), GUARD * 2).await
    });
    // Longer than a program that kept the text takes to show it.
    tokio::time::sleep(Duration::from_secs(17)).await;
    agent.open_gate().await;
    answered.store(true, Ordering::SeqCst);
    let sent = tokio::time::timeout(GUARD * 3, delivering)
        .await
        .expect("delivering hung")
        .unwrap()
        .expect("delivering failed");
    // Anything typed or pressed twice shows by now.
    screen.shows("submitted: hello there", Duration::from_secs(2)).await;
    let _ = screen.shows("never printed", Duration::from_secs(2)).await;
    let _ = shutdown(&agent.socket).await;
    let seen = screen.seen;
    assert!(sent, "deliver says the text wasn't sent; the program printed {seen:?}");
    assert_eq!(seen.matches("submitted: ").count(), 1, "the program printed {seen:?}");
    assert!(seen.contains("submitted: hello there\r"), "the program printed {seen:?}");
}

/// While the input never clears, the text is never typed, and once the
/// limit passes [`deliver`] says it wasn't sent.
#[tokio::test]
async fn a_text_whose_input_never_clears_is_not_sent() {
    let agent = Agent::in_mode("asks");
    let mut screen = Screen::watch(attach(&agent.socket, 0).await.unwrap());
    let never = ready_when(&Arc::new(AtomicBool::new(false)));
    let sent = tokio::time::timeout(
        GUARD,
        deliver(&agent.socket, b"hello there", Duration::from_millis(400), None, Some(never), Duration::from_secs(2)),
    )
    .await
    .expect("delivering hung")
    .expect("delivering failed");
    // Answering now shows anything that was typed.
    agent.open_gate().await;
    let typed = screen.shows("hello", Duration::from_secs(2)).await;
    let _ = shutdown(&agent.socket).await;
    assert!(!sent, "deliver says it was sent; the program printed {:?}", screen.seen);
    assert!(!typed, "the text was typed while the program asked: {:?}", screen.seen);
}

/// A long paste the program shows as `[Paste #1 - n lines]` counts as
/// shown: its return comes at once, not after a wait.
#[tokio::test]
async fn a_paste_shown_as_a_summary_is_submitted() {
    let agent = Agent::in_mode("summary");
    let mut screen = Screen::watch(attach(&agent.socket, 0).await.unwrap());
    assert!(screen.shows("ready", GUARD).await, "the stand-in didn't come up");
    let typed = tokio::time::timeout(
        GUARD,
        type_then_return(&agent.socket, b"first line\nsecond line", Duration::from_millis(400)),
    )
    .await
    .expect("typing hung")
    .expect("typing failed");
    let submitted = screen.shows("submitted: first linesecond line", GUARD).await;
    let _ = shutdown(&agent.socket).await;
    assert!(matches!(typed, Typed::Submitted), "{typed:?}; the program printed {:?}", screen.seen);
    assert!(submitted, "the paste wasn't submitted; the program printed {:?}", screen.seen);
}

fn frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(body.len() + 1).unwrap().to_be_bytes().to_vec();
    out.push(tag);
    out.extend_from_slice(body);
    out
}

/// A holder from a build before the wait for keys: it welcomes, then hangs
/// up on the next frame, which it doesn't know. Counts the input frames it
/// is sent.
fn older_holder(dir: &TempDir) -> (PathBuf, tokio::task::JoinHandle<()>, tokio::sync::watch::Receiver<usize>) {
    let socket = dir.path().join("old.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (inputs, seen) = tokio::sync::watch::channel(0usize);
    let old = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut hello = [0u8; 4 + 1 + 4 + 8];
            if stream.read_exact(&mut hello).await.is_err() {
                continue;
            }
            let mut body = Vec::new();
            body.extend_from_slice(&0u64.to_be_bytes());
            body.extend_from_slice(&0u64.to_be_bytes());
            body.extend_from_slice(&80u16.to_be_bytes());
            body.extend_from_slice(&24u16.to_be_bytes());
            body.push(0);
            body.extend_from_slice(&0i32.to_be_bytes());
            let _ = stream.write_all(&frame(101, &body)).await;
            let mut head = [0u8; 5];
            if stream.read_exact(&mut head).await.is_ok() && matches!(head[4], 2 | 5) {
                inputs.send_modify(|count| *count += 1);
            }
        }
    });
    (socket, old, seen)
}

#[tokio::test]
async fn a_holder_from_an_older_build_is_an_error_to_migrate_not_a_skipped_wait() {
    let dir = tempfile::Builder::new().prefix("h2o").tempdir_in("/tmp").unwrap();
    let (socket, old, inputs) = older_holder(&dir);
    let asked = tokio::time::timeout(GUARD, await_keys(&socket, Duration::from_secs(5)))
        .await
        .expect("the wait for keys hung");
    assert!(matches!(asked, Err(HolderError::Outdated { .. })), "{asked:?}");
    let typed = tokio::time::timeout(GUARD, type_then_return(&socket, b"hello", Duration::ZERO))
        .await
        .expect("typing hung");
    assert!(matches!(typed, Err(HolderError::Outdated { .. })), "{typed:?}");
    assert_eq!(*inputs.borrow(), 0, "text was typed into a holder that can't wait for the program");
    // The refusals leave it running: it still answers an attach.
    assert!(attach(&socket, 0).await.is_ok(), "the old holder stopped answering");
    assert!(!old.is_finished(), "the old holder was stopped");
    old.abort();
}

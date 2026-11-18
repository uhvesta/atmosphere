//! D786: Copilot's folder trust box as a short pty clips it, drawn by a
//! test agent that reads its own size, and what a test needs to set that
//! size the way a client does and to watch the screen's size change.
//! Used by path: `#[path = "common/trust_box.rs"] mod trust_box;`.

#![allow(dead_code)]

use std::path::Path;
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;

use crate::common::{Attach, WAIT};

/// The rows of Copilot 1.0.113's folder trust box, as VERIFY captured it,
/// narrowed: down to the second choice, and from the third to the end.
pub const BOX_TOP: &str = r"  '╭──────────────────────────────────────────────────────────╮' \
  '│ Confirm folder trust                                     │' \
  '│ ╭──────────────────────────────────────────────────────╮ │' \
  '│ │ /work/api                                            │ │' \
  '│ ╰──────────────────────────────────────────────────────╯ │' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │' \
  '│   2. Yes, and remember this folder for future sessions   │'";
/// The same box as Copilot draws it on a screen too short for its list
/// (VERIFY 0.2.3, e2e023 c6c), narrowed and with a made-up folder.
pub const BOX_CLIPPED: &str = r"  '╭──────────────────────────────────────────────────────────╮' \
  '│ Confirm folder trust                                     │' \
  '│ ╭──────────────────────────────────────────────────────╮ │' \
  '│ │ /work/api                                            │ │' \
  '│ ╰──────────────────────────────────────────────────────╯ │' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │' \
  '│                                                          │' \
  '│ ↑/↓ to navigate · pgup/pgdn to scroll · enter to select  │' \
  '╰──────────────────────────────────────────────────────────╯'";
pub const BOX_REST: &str = r"  '│   3. No (Esc)                                            │' \
  '│                                                          │' \
  '│ ↑/↓ to navigate · enter to select · esc to cancel        │' \
  '╰──────────────────────────────────────────────────────────╯'";

/// The test agent for D786. It waits for `go` in `conv`, so a test can set
/// its pty's size first, then draws the trust box: clipped to `1. Yes`
/// (with a footer that says the list scrolls) while its pty has fewer than
/// `whole_from` rows, whole from there. Each size it draws at goes into
/// `sizes`, one `rows cols` line each. With `redraws` it draws again at its
/// new size on SIGWINCH, as Copilot does; without, it ignores a resize. The
/// bytes of the line it is answered with go into `trust-answer`, as hex;
/// every line after is answered `got: <line>`.
pub fn agent(conv: &Path, whole_from: u16, redraws: bool) -> String {
    let trap = if redraws { "trap draw WINCH\n" } else { "" };
    format!(
        r#"CONV_DIR='{conv}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
until [ -e "$CONV_DIR/go" ]; do sleep 0.1; done
clipped=$(printf '%s\n' \
{BOX_CLIPPED})
whole=$(printf '%s\n' \
{BOX_TOP} \
{BOX_REST})
draw() {{
  size=$(stty size)
  printf '%s\n' "$size" >> "$CONV_DIR/sizes"
  if [ "${{size%% *}}" -lt {whole_from} ]; then box=$clipped; else box=$whole; fi
  printf '\033[H\033[2J%s\n' "$box"
  woke=1
}}
draw
{trap}while :; do
  woke=0
  IFS= read -r answer
  rc=$?
  # A redraw that interrupts the read is not the answer: bash says so with
  # a status over 128, dash with 1.
  if [ "$rc" -ne 0 ] && [ "$woke" = 1 ]; then continue; fi
  break
done
trap - WINCH
printf '%s' "$answer" | od -An -tx1 | tr -d ' \n' > "$CONV_DIR/trust-answer.part"
mv "$CONV_DIR/trust-answer.part" "$CONV_DIR/trust-answer"
printf '\033[H\033[2J'
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; done
"#,
        conv = conv.display(),
    )
}

/// Lets the agent of [`agent`] draw.
pub fn go(conv: &Path) {
    std::fs::write(conv.join("go"), "").expect("write a test file");
}

/// The sizes the agent drew at, `rows cols`, oldest first.
pub fn sizes(conv: &Path) -> Vec<String> {
    std::fs::read_to_string(conv.join("sizes"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Waits until the agent has drawn at `rows cols` too.
pub async fn wait_size(conv: &Path, size: &str) {
    let deadline = Instant::now() + WAIT;
    while !sizes(conv).iter().any(|drawn| drawn == size) {
        assert!(
            Instant::now() < deadline,
            "the agent never drew at {size}; it drew at {:?}",
            sizes(conv)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// An attach to `tab` with input at `cols`x`rows`: a client's, which sizes
/// the pty to it (D14).
pub fn writer_open(session: Option<&pb::SessionId>, tab: &pb::TabId, cols: u32, rows: u32) -> pb::AttachOpen {
    pb::AttachOpen {
        session: session.cloned(),
        tab: Some(tab.clone()),
        cols,
        rows,
        ..Default::default()
    }
}

/// A read-only checkpoint attach to `tab`: it isn't a writer (SE-25), and
/// its states say the other writers and the screen's size as it changes.
pub fn reader_open(session: Option<&pb::SessionId>, tab: &pb::TabId) -> pb::AttachOpen {
    pb::AttachOpen {
        session: session.cloned(),
        tab: Some(tab.clone()),
        read_only: true,
        checkpoint_engine: ghostty_vt::CHECKPOINT_FORMAT.to_owned(),
        ..Default::default()
    }
}

/// Reads `reader` until its last state says `want`.
pub async fn wait_state(reader: &mut Attach, want: impl Fn(&pb::TerminalState) -> bool) -> pb::TerminalState {
    reader
        .wait_until(|attach| attach.states.last().is_some_and(&want))
        .await;
    reader.states.last().cloned().expect("a state")
}

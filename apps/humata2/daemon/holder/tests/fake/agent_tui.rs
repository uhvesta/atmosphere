//! A stand-in for an agent's CLI that takes typed text the way GitHub
//! Copilot CLI 1.0.113 does. It reads its input from the start, but only
//! acts on it once it's up, which it is when the test opens the gate (a
//! FIFO given as the first argument). What it read before then it takes
//! as one piece: a return in that piece is part of a paste, not a submit.
//! Once it's up, a return that comes on its own submits the prompt.
//!
//! With `cooked` as the second argument it starts with the terminal's line
//! editing on, as a CLI does before it sets its terminal up, and turns it
//! off when it's up; with `raw` it turns it off at once.
//!
//! With `asks` it reads keys at once but first asks a question of its own,
//! as Copilot's folder-trust prompt does in a folder it doesn't trust yet:
//! until the gate opens (the person answered it) it holds typed text without
//! showing it, and a return on its own answers the question, which it
//! reports as `question answered by a return`. With `summary` it is up at
//! once and shows a paste with a line break as `[Paste #1 - <n> lines]`, as
//! Copilot does, never its text; it prints `ready` once it is up. With
//! `eats` it asks as with `asks` but drops what was typed while it asked,
//! as a CLI that clears its input on answering would.
//!
//! It prints `early return` when a return reached it before it was up,
//! `> <prompt>` as the prompt changes and `submitted: <prompt>` on a
//! submit.

use std::io::{Read, Write};
use std::sync::mpsc;

fn main() {
    let mut args = std::env::args().skip(1);
    let gate = args.next().expect("the gate FIFO");
    let mode = args.next().unwrap_or_default();
    let cooked = mode == "cooked";
    let eats = mode == "eats";
    let asks = mode == "asks" || eats;
    let summary = mode == "summary";
    if !cooked {
        keys_raw();
    }
    let (sent, heard) = mpsc::channel();
    let typed = sent.clone();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0; 4096];
        while let Ok(read @ 1..) = stdin.read(&mut buffer) {
            if typed.send(Heard::Input(buffer[..read].to_vec())).is_err() {
                return;
            }
        }
    });
    std::thread::spawn(move || {
        // Blocks until the test opens the FIFO to write.
        if !summary {
            let _ = std::fs::read(&gate);
        }
        let _ = sent.send(Heard::Up);
    });

    let mut held = Vec::new();
    let mut warned = false;
    while let Ok(Heard::Input(chunk)) = heard.recv() {
        if asks && matches!(chunk.as_slice(), b"\r" | b"\n") {
            say(b"question answered by a return\r\n");
            continue;
        }
        if eats {
            continue;
        }
        held.extend_from_slice(&chunk);
        if !warned && held.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            warned = true;
            say(b"early return\r\n");
        }
    }
    if cooked {
        keys_raw();
    }
    if summary {
        say(b"ready\r\n");
    }
    let mut prompt = String::new();
    if !held.is_empty() {
        take(&mut prompt, &held, summary);
    }
    while let Ok(heard) = heard.recv() {
        if let Heard::Input(chunk) = heard {
            take(&mut prompt, &chunk, summary);
        }
    }
}

enum Heard {
    Input(Vec<u8>),
    /// The test opened the gate.
    Up,
}

/// Acts on one piece of input; with `summary`, a paste with a line break
/// is shown by its line count.
fn take(prompt: &mut String, chunk: &[u8], summary: bool) {
    if matches!(chunk, b"\r" | b"\n") {
        if !prompt.is_empty() {
            say(format!("\r\nsubmitted: {prompt}\r\n").as_bytes());
            prompt.clear();
        }
        return;
    }
    let lines = chunk.iter().filter(|byte| **byte == b'\n').count();
    if summary && lines > 0 {
        prompt.extend(String::from_utf8_lossy(chunk).chars().filter(|c| !c.is_control()));
        say(format!("\x1b[2K\r\x1b[1m> \x1b[0m[Paste #1 - {} lines]", lines + 1).as_bytes());
        return;
    }
    // A paste: its returns are part of the text.
    prompt.extend(
        String::from_utf8_lossy(chunk)
            .chars()
            .filter(|c| !c.is_control()),
    );
    say(format!("\x1b[2K\r\x1b[1m> \x1b[0m{prompt}").as_bytes());
}

fn say(bytes: &[u8]) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(bytes);
    let _ = stdout.flush();
}

/// Turns the terminal's line editing and echo off.
fn keys_raw() {
    use rustix::termios::{LocalModes, OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr};
    let stdin = std::io::stdin();
    let mut modes = tcgetattr(&stdin).expect("stdin is a terminal");
    modes.local_modes.remove(LocalModes::ICANON | LocalModes::ECHO);
    modes.special_codes[SpecialCodeIndex::VMIN] = 1;
    modes.special_codes[SpecialCodeIndex::VTIME] = 0;
    tcsetattr(&stdin, OptionalActions::Now, &modes).expect("set the terminal's modes");
}

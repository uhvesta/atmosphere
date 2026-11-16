//! Builders for the lines an agent writes to its conversation file, and
//! short hands for asking a mirror.

#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use humata2_mirror::{Hit, Mirror, MirrorEntry, PageAsk, Query, Scrubber, SessionMeta};
use serde_json::{Value, json};

/// A timestamp as the agent writes it, `n` minutes into a fixed morning.
pub fn at(n: u32) -> String {
    format!("2026-11-10T{:02}:{:02}:00.000Z", 9 + n / 60, n % 60)
}

fn line(kind: &str, role: &str, content: Value, at: &str) -> String {
    json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/work",
        "sessionId": "0b6c1a4e-1111-4222-8333-444455556666",
        "version": "9.9.9",
        "type": kind,
        "message": {"role": role, "content": content},
        "uuid": "7a1d2c3b-aaaa-4bbb-8ccc-ddddeeeeffff",
        "timestamp": at,
    })
    .to_string()
}

/// A prompt the user typed.
pub fn user(text: &str, at: &str) -> String {
    line("user", "user", json!(text), at)
}

/// A user line made of content blocks.
pub fn user_blocks(blocks: Vec<Value>, at: &str) -> String {
    line("user", "user", Value::Array(blocks), at)
}

/// An agent line made of content blocks.
pub fn assistant(blocks: Vec<Value>, at: &str) -> String {
    line("assistant", "assistant", Value::Array(blocks), at)
}

/// An agent line that only says `words`.
pub fn says(words: &str, at: &str) -> String {
    assistant(vec![text(words)], at)
}

pub fn text(words: &str) -> Value {
    json!({"type": "text", "text": words})
}

pub fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

/// A tool's answer given as one string.
pub fn tool_result(id: &str, answer: &str) -> Value {
    json!({"type": "tool_result", "tool_use_id": id, "content": answer})
}

/// A tool's answer given as a list of text blocks.
pub fn tool_result_blocks(id: &str, answer: &str) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": [{"type": "text", "text": answer}],
    })
}

/// Writes `lines` as the whole file, each ended by a newline.
pub fn write_lines(file: &Path, lines: &[String]) {
    fs::write(file, joined(lines)).unwrap();
}

/// Adds `lines` to the end of the file, each ended by a newline.
pub fn append_lines(file: &Path, lines: &[String]) {
    append_bytes(file, joined(lines).as_bytes());
}

/// Adds `bytes` to the end of the file as they are.
pub fn append_bytes(file: &Path, bytes: &[u8]) {
    let mut open = OpenOptions::new()
        .append(true)
        .create(true)
        .open(file)
        .unwrap();
    open.write_all(bytes).unwrap();
}

fn joined(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// A session in `channel` on `machine` run by `agent`, with names made
/// from the ids.
pub fn meta(session: &str, channel: &str, machine: &str, agent: &str) -> SessionMeta {
    SessionMeta {
        session: session.to_owned(),
        channel: channel.to_owned(),
        channel_name: format!("Channel {channel}"),
        title: format!("Title of {session}"),
        machine: machine.to_owned(),
        machine_name: format!("Machine {machine}"),
        agent: agent.to_owned(),
        archived: false,
    }
}

/// A scrubber for a machine without host variables.
pub fn no_vars() -> Scrubber {
    Scrubber::new(Vec::<(String, String)>::new())
}

/// Describes `session` (channel `ch_main`, machine `ma_mac`, agent
/// `claude`), writes `lines` as its conversation file under `dir` and
/// mirrors them. Returns the conversation file.
pub fn mirror_session(mirror: &Mirror, dir: &Path, session: &str, lines: &[String]) -> PathBuf {
    mirror_session_as(
        mirror,
        dir,
        &meta(session, "ch_main", "ma_mac", "claude"),
        lines,
    )
}

/// Like [`mirror_session`], with the session's channel, machine and agent
/// given.
pub fn mirror_session_as(
    mirror: &Mirror,
    dir: &Path,
    meta: &SessionMeta,
    lines: &[String],
) -> PathBuf {
    let file = dir.join(format!("{}.jsonl", meta.session));
    write_lines(&file, lines);
    mirror.describe(meta).unwrap();
    mirror.catch_up(&meta.session, &file, &no_vars()).unwrap();
    file
}

/// A query for `words` everywhere, first page.
pub fn words(words: &str) -> Query {
    Query {
        text: words.to_owned(),
        ..Query::default()
    }
}

/// Every hit for `query`, following page tokens to the end.
pub fn all_hits(mirror: &Mirror, query: &Query) -> Vec<Hit> {
    let mut query = query.clone();
    let mut hits = Vec::new();
    for _ in 0..1000 {
        let page = mirror.search(&query).unwrap();
        hits.extend(page.items);
        if page.next_token.is_empty() {
            return hits;
        }
        query.page.token = page.next_token;
    }
    panic!("the search never reached a last page");
}

/// Every hit for `text` in any session.
pub fn find(mirror: &Mirror, text: &str) -> Vec<Hit> {
    all_hits(mirror, &words(text))
}

/// The (session, turn) of each hit, sorted.
pub fn places(hits: &[Hit]) -> Vec<(String, u32)> {
    let mut places: Vec<(String, u32)> = hits
        .iter()
        .map(|hit| (hit.session.clone(), hit.turn))
        .collect();
    places.sort();
    places
}

/// Every entry of `session` in the given turns, following page tokens.
pub fn entries_in(
    mirror: &Mirror,
    session: &str,
    first_turn: Option<u32>,
    last_turn: Option<u32>,
) -> Vec<MirrorEntry> {
    let mut ask = PageAsk::default();
    let mut entries = Vec::new();
    for _ in 0..1000 {
        let page = mirror.read(session, first_turn, last_turn, &ask).unwrap();
        entries.extend(page.items);
        if page.next_token.is_empty() {
            return entries;
        }
        ask.token = page.next_token;
    }
    panic!("the read never reached a last page");
}

/// Every entry of `session`.
pub fn entries(mirror: &Mirror, session: &str) -> Vec<MirrorEntry> {
    entries_in(mirror, session, None, None)
}

/// The texts of `entries`, in order.
pub fn texts(entries: &[MirrorEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.text.as_str()).collect()
}

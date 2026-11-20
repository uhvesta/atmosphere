//! A daemon and a terminal that tests script and read back. Each test file
//! uses part of this, hence `dead_code`.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io;

use humata2_cli::{
    Console, Daemon, DaemonError, Environment, Keyboard, Replies, ReportReply, ReportRequest, Rpc,
};
use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::Value as Json;
use tokio::sync::mpsc;

/// gRPC's FAILED_PRECONDITION, the status a Humata error rides on here;
/// which one doesn't matter to the CLI when the error has a kind.
const REFUSED_STATUS: i32 = 9;

/// Answers each call with what the test queued for its method, in order,
/// and with an empty reply once none is left. Remembers every request.
#[derive(Default)]
pub struct FakeDaemon {
    replies: RefCell<HashMap<&'static str, VecDeque<Result<Vec<u8>, DaemonError>>>>,
    streams: RefCell<HashMap<&'static str, Vec<Vec<u8>>>>,
    calls: RefCell<Vec<(&'static str, Vec<u8>)>>,
    hangs: RefCell<Vec<&'static str>>,
    reports: RefCell<Vec<ReportRequest>>,
    report_reply: RefCell<Option<Result<ReportReply, DaemonError>>>,
    stale_as_of: RefCell<Option<String>>,
}

impl FakeDaemon {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next call to `rpc` answers `message`.
    pub fn reply(&self, rpc: Rpc, message: &impl Message) -> &Self {
        self.queue(rpc, Ok(message.encode_to_vec()))
    }

    /// The next call to `rpc` fails with a Humata error of this kind.
    pub fn refuse(&self, rpc: Rpc, kind: pb::error::Kind) -> &Self {
        self.refuse_with(
            rpc,
            pb::Error {
                message: "refused".into(),
                kind: Some(kind),
                ..Default::default()
            },
        )
    }

    pub fn refuse_with(&self, rpc: Rpc, error: pb::Error) -> &Self {
        self.queue(
            rpc,
            Err(DaemonError::Refused {
                rpc: rpc.path,
                code: REFUSED_STATUS,
                error: Box::new(error),
            }),
        )
    }

    /// The next call to `rpc` fails with a bare gRPC status.
    pub fn status(&self, rpc: Rpc, code: i32) -> &Self {
        self.queue(
            rpc,
            Err(DaemonError::Status {
                rpc: rpc.path,
                code,
                message: "status".into(),
            }),
        )
    }

    fn queue(&self, rpc: Rpc, reply: Result<Vec<u8>, DaemonError>) -> &Self {
        self.replies
            .borrow_mut()
            .entry(rpc.path)
            .or_default()
            .push_back(reply);
        self
    }

    /// What a server or bidi stream on `rpc` sends before it closes.
    pub fn stream(&self, rpc: Rpc, messages: &[&dyn MessageBytes]) -> &Self {
        self.streams.borrow_mut().insert(
            rpc.path,
            messages.iter().map(|message| message.bytes()).collect(),
        );
        self
    }

    /// Calls to `rpc` never answer, as a daemon that is waiting.
    pub fn hang(&self, rpc: Rpc) -> &Self {
        self.hangs.borrow_mut().push(rpc.path);
        self
    }

    /// Every reply so far was the machine's copy, which the host answered
    /// at `at` (the `humata-stale-as-of` header).
    pub fn copy_made_at(&self, at: &str) -> &Self {
        *self.stale_as_of.borrow_mut() = Some(at.to_owned());
        self
    }

    pub fn report_replies(&self, reply: Result<ReportReply, DaemonError>) {
        *self.report_reply.borrow_mut() = Some(reply);
    }

    /// Every request sent to `rpc`, decoded, in order. A client stream's
    /// messages count as one request each.
    pub fn sent<R: Message + Default>(&self, rpc: Rpc) -> Vec<R> {
        self.calls
            .borrow()
            .iter()
            .filter(|(path, _)| *path == rpc.path)
            .map(|(_, bytes)| R::decode(bytes.as_slice()).expect("the CLI sent a valid request"))
            .collect()
    }

    /// The one request sent to `rpc`.
    pub fn only<R: Message + Default>(&self, rpc: Rpc) -> R {
        let mut sent = self.sent::<R>(rpc);
        assert_eq!(sent.len(), 1, "expected one call to {}", rpc.path);
        sent.remove(0)
    }

    pub fn called(&self, rpc: Rpc) -> bool {
        self.calls
            .borrow()
            .iter()
            .any(|(path, _)| *path == rpc.path)
    }

    pub fn reports(&self) -> Vec<ReportRequest> {
        self.reports.borrow().clone()
    }

    async fn answer(&self, rpc: Rpc, request: Vec<u8>) -> Result<Vec<u8>, DaemonError> {
        self.calls.borrow_mut().push((rpc.path, request));
        if self.hangs.borrow().contains(&rpc.path) {
            std::future::pending::<()>().await;
        }
        self.replies
            .borrow_mut()
            .get_mut(rpc.path)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Ok(Vec::new()))
    }

    fn replies_for(&self, rpc: Rpc) -> Replies {
        let messages = self
            .streams
            .borrow_mut()
            .remove(rpc.path)
            .unwrap_or_default();
        let (send, replies) = mpsc::channel(messages.len().max(1));
        for message in messages {
            let _ = send.try_send(Ok(message));
        }
        replies
    }
}

/// A message as bytes, so a stream can mix types in a test's list.
pub trait MessageBytes {
    fn bytes(&self) -> Vec<u8>;
}

impl<M: Message> MessageBytes for M {
    fn bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

impl Daemon for FakeDaemon {
    async fn unary(&self, rpc: Rpc, request: Vec<u8>) -> Result<Vec<u8>, DaemonError> {
        self.answer(rpc, request).await
    }

    async fn server_stream(&self, rpc: Rpc, request: Vec<u8>) -> Result<Replies, DaemonError> {
        self.answer(rpc, request).await?;
        Ok(self.replies_for(rpc))
    }

    async fn client_stream(
        &self,
        rpc: Rpc,
        mut requests: mpsc::Receiver<Vec<u8>>,
    ) -> Result<Vec<u8>, DaemonError> {
        let mut last = Ok(Vec::new());
        let mut first = true;
        while let Some(request) = requests.recv().await {
            if first {
                last = self.answer(rpc, request).await;
                first = false;
            } else {
                self.calls.borrow_mut().push((rpc.path, request));
            }
        }
        last
    }

    async fn bidi_stream(
        &self,
        rpc: Rpc,
        _requests: mpsc::Receiver<Vec<u8>>,
    ) -> Result<Replies, DaemonError> {
        self.answer(rpc, Vec::new()).await?;
        Ok(self.replies_for(rpc))
    }

    fn stale_as_of(&self) -> Option<String> {
        self.stale_as_of.borrow().clone()
    }

    async fn report(&self, request: ReportRequest) -> Result<ReportReply, DaemonError> {
        self.reports.borrow_mut().push(request);
        self.report_reply
            .borrow_mut()
            .take()
            .unwrap_or(Err(DaemonError::NoReportService))
    }
}

/// A terminal, or a pipe when `interactive` is false. Records what was
/// printed; answers prompts from what the test queued.
#[derive(Default)]
pub struct FakeConsole {
    pub interactive: bool,
    out: RefCell<Vec<String>>,
    err: RefCell<Vec<String>>,
    written: RefCell<Vec<u8>>,
    secrets: RefCell<VecDeque<String>>,
    confirms: RefCell<VecDeque<bool>>,
    input: RefCell<Vec<u8>>,
}

impl FakeConsole {
    /// A pipe: no person to ask.
    pub fn pipe() -> Self {
        Self::default()
    }

    /// A terminal with a person at it.
    pub fn terminal() -> Self {
        Self {
            interactive: true,
            ..Self::default()
        }
    }

    pub fn typing_secret(self, secret: &str) -> Self {
        self.secrets.borrow_mut().push_back(secret.into());
        self
    }

    pub fn answering(self, yes: bool) -> Self {
        self.confirms.borrow_mut().push_back(yes);
        self
    }

    pub fn with_stdin(self, bytes: &[u8]) -> Self {
        *self.input.borrow_mut() = bytes.to_vec();
        self
    }

    pub fn stdout(&self) -> String {
        self.out.borrow().join("\n")
    }

    pub fn stderr(&self) -> String {
        self.err.borrow().join("\n")
    }

    pub fn written(&self) -> Vec<u8> {
        self.written.borrow().clone()
    }

    /// Stdout as the one JSON object `--json` prints.
    pub fn json(&self) -> Json {
        let out = self.out.borrow();
        assert_eq!(out.len(), 1, "--json prints exactly one line: {out:?}");
        serde_json::from_str(&out[0]).expect("--json prints JSON")
    }
}

impl Console for FakeConsole {
    fn print(&self, text: &str) {
        self.out.borrow_mut().push(text.into());
    }

    fn eprint(&self, text: &str) {
        self.err.borrow_mut().push(text.into());
    }

    fn write(&self, bytes: &[u8]) {
        self.written.borrow_mut().extend_from_slice(bytes);
    }

    fn interactive(&self) -> bool {
        self.interactive
    }

    fn secret(&self, _prompt: &str) -> io::Result<String> {
        self.secrets
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "nothing typed"))
    }

    fn line(&self, _prompt: &str) -> io::Result<String> {
        self.secrets
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "nothing typed"))
    }

    fn confirm(&self, _prompt: &str) -> io::Result<bool> {
        self.confirms
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "nothing typed"))
    }

    fn stdin(&self) -> io::Result<Vec<u8>> {
        Ok(self.input.borrow().clone())
    }

    fn keyboard(&self) -> io::Result<Keyboard> {
        let (_, input) = mpsc::channel(1);
        let (_, resizes) = mpsc::channel(1);
        Ok(Keyboard::new(input, resizes, (80, 24)))
    }
}

fn args(words: &[&str]) -> Vec<OsString> {
    std::iter::once("humata")
        .chain(words.iter().copied())
        .map(OsString::from)
        .collect()
}

/// Runs `humata <words>` as the person, never interrupted.
pub async fn run(daemon: &FakeDaemon, console: &FakeConsole, words: &[&str]) -> i32 {
    run_as(&Environment::default(), daemon, console, words).await
}

/// Runs `humata <words>` with this environment's tokens.
pub async fn run_as(
    environment: &Environment,
    daemon: &FakeDaemon,
    console: &FakeConsole,
    words: &[&str],
) -> i32 {
    humata2_cli::run_args(
        args(words),
        environment,
        daemon,
        console,
        std::future::pending(),
    )
    .await
}

/// Runs `humata <words>` with Ctrl-C already pressed: it stops at the
/// first call the daemon doesn't answer at once.
pub async fn run_interrupted(daemon: &FakeDaemon, console: &FakeConsole, words: &[&str]) -> i32 {
    humata2_cli::run_args(
        args(words),
        &Environment::default(),
        daemon,
        console,
        std::future::ready(()),
    )
    .await
}

pub fn session(value: &str) -> Option<pb::SessionId> {
    Some(pb::SessionId {
        value: value.into(),
    })
}

pub fn channel(value: &str) -> Option<pb::ChannelId> {
    Some(pb::ChannelId {
        value: value.into(),
    })
}

pub fn machine(value: &str) -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: value.into(),
    })
}

//! The commands that stream: a terminal, a provider's login, and a file's
//! bytes in or out.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::call::{Ctx, Done, decode};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, rpc};
use crate::error::CliError;
use crate::as_json;

/// Ctrl-]: leaves an attached terminal or a login, as telnet does
/// (provisional: CLI.md doesn't name a key).
const DETACH: u8 = 0x1d;

/// The size of each chunk `files put` sends.
const CHUNK_BYTES: usize = 1 << 20;

/// The daemon's attach window when `window_bytes` is 0 (terminal.proto).
const DEFAULT_WINDOW: u64 = 1 << 20;

/// The gRPC status a stream's own `failed` message is reported with.
const UNKNOWN: i32 = 2;

/// A live terminal can't be one JSON object.
pub(super) fn no_json<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<(), CliError> {
    if ctx.flags.json {
        return Err(CliError::BadValue {
            field: "--json",
            value: "on".into(),
            expected: "no --json: this command is a live terminal (`humata conversation` and \
                       `humata events` print JSON)",
        });
    }
    Ok(())
}

/// Shows a terminal here until it ends or Ctrl-] is typed: its snapshot,
/// then its output, with keystrokes and size changes sent back.
pub(super) async fn attach<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    open: pb::AttachOpen,
) -> Result<Done, CliError> {
    use pb::attach_client_message::Message as Client;
    use pb::attach_server_message::Message as Server;

    no_json(ctx)?;
    let read_only = open.read_only;
    // Acks only free the daemon's window (TR-8), so one per quarter
    // window keeps output flowing without a message per output chunk.
    let window = match open.window_bytes {
        0 => DEFAULT_WINDOW,
        bytes => u64::from(bytes),
    };
    let ack_every = (window / 4).max(1);
    let mut acked = 0;
    let mut keyboard = ctx
        .console
        .keyboard()
        .map_err(|source| CliError::Console { source })?;
    let (cols, rows) = keyboard.size;
    let (send, requests) = mpsc::channel::<Vec<u8>>(64);
    let message = |message: Client| {
        pb::AttachClientMessage {
            message: Some(message),
        }
        .encode_to_vec()
    };
    let _ = send.try_send(message(Client::Open(pb::AttachOpen { cols, rows, ..open })));
    let mut replies = ctx.daemon.bidi_stream(rpc::terminal::ATTACH, requests).await?;
    ctx.note("Attached. Ctrl-] detaches.");
    let mut input_seq = 0;
    loop {
        tokio::select! {
            reply = replies.recv() => {
                let Some(reply) = reply else { break };
                let reply: pb::AttachServerMessage = decode(rpc::terminal::ATTACH, &reply?)?;
                match reply.message {
                    Some(Server::Snapshot(snapshot)) => {
                        ctx.console.write(&snapshot.vt);
                        // The daemon counts a snapshot as drawn.
                        acked = snapshot.offset;
                    }
                    Some(Server::Output(output)) => {
                        ctx.console.write(&output.data);
                        let offset = output.offset + output.data.len() as u64;
                        if offset.saturating_sub(acked) >= ack_every {
                            acked = offset;
                            let _ = send.send(message(Client::Ack(pb::OutputAck { offset }))).await;
                        }
                    }
                    Some(Server::Notice(notice)) => {
                        ctx.console.eprint(&format!("\r\n[humata] {}\r", notice.text));
                    }
                    Some(Server::State(_) | Server::InputAccepted(_)) | None => {}
                }
            }
            keys = keyboard.input.recv() => {
                let Some(keys) = keys else { break };
                let detach = keys.iter().position(|byte| *byte == DETACH);
                let typed = &keys[..detach.unwrap_or(keys.len())];
                if !typed.is_empty() && !read_only {
                    input_seq += 1;
                    let input = pb::TerminalInput { data: typed.to_vec(), input_seq };
                    if send.send(message(Client::Input(input))).await.is_err() {
                        break;
                    }
                }
                if detach.is_some() {
                    break;
                }
            }
            Some((cols, rows)) = keyboard.resizes.recv() => {
                let _ = send.send(message(Client::Resize(pb::TerminalResize { cols, rows }))).await;
            }
        }
    }
    drop(keyboard);
    ctx.note("\r\nDetached.");
    Ok(Done::printed())
}

/// A provider's login on `machine`, typed into here, until it is ready.
pub(super) async fn provider_login<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    machine: pb::MachineId,
) -> Result<Done, CliError> {
    use pb::login_client_message::Message as Client;
    use pb::login_server_message::Message as Server;

    no_json(ctx)?;
    let mut keyboard = ctx
        .console
        .keyboard()
        .map_err(|source| CliError::Console { source })?;
    let (cols, rows) = keyboard.size;
    let (send, requests) = mpsc::channel::<Vec<u8>>(64);
    let message = |message: Client| {
        pb::LoginClientMessage {
            message: Some(message),
        }
        .encode_to_vec()
    };
    let open = pb::LoginOpen {
        target: Some(pb::login_open::Target::Machine(machine)),
        cols,
        rows,
    };
    let _ = send.try_send(message(Client::Open(open)));
    let rpc = rpc::provider::INTERACTIVE_LOGIN;
    let mut replies = ctx.daemon.bidi_stream(rpc, requests).await?;
    ctx.note("Logging in. Ctrl-] gives up.");
    let mut ready = None;
    loop {
        tokio::select! {
            reply = replies.recv() => {
                let Some(reply) = reply else { break };
                let reply: pb::LoginServerMessage = decode(rpc, &reply?)?;
                match reply.message {
                    Some(Server::Output(bytes)) => ctx.console.write(&bytes),
                    Some(Server::Ready(done)) => {
                        let _ = send.send(message(Client::Done(pb::LoginDone {}))).await;
                        ready = Some(done);
                        break;
                    }
                    Some(Server::Failed(error)) => {
                        return Err(DaemonError::Refused {
                            rpc: rpc.path,
                            code: UNKNOWN,
                            error: Box::new(error),
                        }
                        .into());
                    }
                    Some(Server::PromptSeen(_)) | None => {}
                }
            }
            keys = keyboard.input.recv() => {
                let Some(keys) = keys else { break };
                let detach = keys.iter().position(|byte| *byte == DETACH);
                let typed = &keys[..detach.unwrap_or(keys.len())];
                if !typed.is_empty() {
                    let _ = send.send(message(Client::Input(typed.to_vec()))).await;
                }
                if detach.is_some() {
                    let _ = send.send(message(Client::Cancel(pb::LoginCancel {}))).await;
                    return Err(CliError::Interrupted);
                }
            }
            Some((cols, rows)) = keyboard.resizes.recv() => {
                let _ = send.send(message(Client::Resize(pb::LoginResize { cols, rows }))).await;
            }
        }
    }
    drop(keyboard);
    match ready {
        Some(ready) => Ok(Done::reply(as_json::typed("humata.v1.LoginReady", &ready))),
        None => Err(DaemonError::Ended { rpc: rpc.path }.into()),
    }
}

/// A file's bytes: as they come on stdout, or with `--json` as base64
/// with their SHA-256.
pub(super) async fn cat<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    location: pb::FileLocation,
) -> Result<Done, CliError> {
    let request = pb::ReadFileRequest {
        location: Some(location),
        ..Default::default()
    };
    let rpc = rpc::files::READ_FILE;
    let mut replies = ctx.daemon.server_stream(rpc, request.encode_to_vec()).await?;
    let mut data = Vec::new();
    while let Some(reply) = replies.recv().await {
        let chunk: pb::FileChunk = decode(rpc, &reply?)?;
        if ctx.flags.json {
            data.extend_from_slice(&chunk.data);
        } else {
            ctx.console.write(&chunk.data);
        }
        if chunk.last {
            break;
        }
    }
    if !ctx.flags.json {
        return Ok(Done::printed());
    }
    Ok(Done::reply(json!({
        "data": as_json::base64(&data),
        "bytes": data.len(),
        "sha256": hex(&Sha256::digest(&data)),
    })))
}

/// Sends a local file: a header with its size and SHA-256, then 1 MiB
/// chunks.
pub(super) async fn put<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    local: &Path,
    location: pb::FileLocation,
    replace: bool,
) -> Result<Done, CliError> {
    use pb::write_file_message::Message as Part;

    // Read off the runtime and streamed through a short queue, so a big
    // file is never all in memory (TR-8). The hash pass comes first: the
    // header carries it.
    let path = local.to_path_buf();
    let (size, sha256) = tokio::task::spawn_blocking(move || digest(&path))
        .await
        .map_err(|_| read_failed(local, io::Error::other("the file reader stopped")))?
        .map_err(|source| read_failed(local, source))?;
    let header = pb::WriteFileHeader {
        idempotency_key: ctx.key(),
        location: Some(location),
        size,
        sha256,
        replace,
    };
    let (send, requests) = mpsc::channel(2);
    let header = pb::WriteFileMessage {
        message: Some(Part::Header(header)),
    };
    let _ = send.try_send(header.encode_to_vec());
    let path = local.to_path_buf();
    let reader = tokio::task::spawn_blocking(move || send_chunks(&path, size, &send));
    let rpc = rpc::files::WRITE_FILE;
    let reply = ctx.daemon.client_stream(rpc, requests).await;
    // A local read failure explains a refused upload better than the
    // daemon's size or hash mismatch does.
    match reader.await {
        Ok(Err(source)) => return Err(read_failed(local, source)),
        Ok(Ok(())) => {}
        Err(_) => return Err(read_failed(local, io::Error::other("the file reader stopped"))),
    }
    Ok(Done::reply(as_json::encoded(rpc.response, &reply?)))
}

fn read_failed(path: &Path, source: io::Error) -> CliError {
    CliError::ReadFile {
        path: path.to_path_buf(),
        source,
    }
}

/// The file's size and SHA-256, read in chunks.
fn digest(path: &Path) -> io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                hasher.update(&buffer[..read]);
                size += read as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok((size, hex(&hasher.finalize())))
}

/// Sends the file as `CHUNK_BYTES` chunks, the one ending at `size`
/// marked last. Stops early if the call ended.
fn send_chunks(path: &Path, size: u64, send: &mpsc::Sender<Vec<u8>>) -> io::Result<()> {
    use pb::write_file_message::Message as Part;

    let mut file = File::open(path)?;
    let mut offset = 0u64;
    loop {
        let mut data = Vec::with_capacity(CHUNK_BYTES);
        (&mut file).take(CHUNK_BYTES as u64).read_to_end(&mut data)?;
        let read = data.len() as u64;
        let last = read == 0 || offset + read >= size;
        let chunk = pb::WriteFileMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset,
                data,
                sha256: String::new(),
                last,
            })),
        };
        if send.blocking_send(chunk.encode_to_vec()).is_err() || last {
            return Ok(());
        }
        offset += read;
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

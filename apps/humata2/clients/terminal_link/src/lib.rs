//! One tab's attach, read as the terminal component's frames (D271).
//!
//! The daemon's gRPC Attach carries checkpoints of its authoritative
//! screen, output, input confirmations and state. [`Inbound`] turns each
//! message into `terminal_wire` server frames for a `TerminalReplica`, and
//! [`outbound`] turns the replica's client frames into attach messages.
//! Pure mapping: no runtime, no sockets, so the browser can use it too.
//!
//! Peers are on the same version (D271): a peer whose snapshot carries no
//! checkpoint is refused, never replayed as raw bytes.

use humata2_proto::humata::v1 as pb;
use pb::attach_client_message::Message as ClientMessage;
use pb::attach_server_message::Message as ServerMessage;
use pb::terminal_state::Writable;
use terminal_wire::{ClientFrame, SNAPSHOT_ENGINE, ServerFrame};

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error(
        "The machine serving this terminal runs an older Humata that can't send its screen. It is being upgraded; the terminal reconnects once it is. Nothing typed was sent."
    )]
    PeerVersionUnsupported,
    #[error(
        "The terminal's screen came in the {got} format, which this Humata can't install. Update Humata on this device and on the machine to the same version."
    )]
    EngineMismatch { got: String },
    #[error(
        "The terminal sent a compressed piece (codec {codec}) though none was asked for. Reopen the tab; if it happens again, the machine's Humata is at fault."
    )]
    UnexpectedCodec { codec: i32 },
    #[error(
        "The terminal sent output before its screen, so it would draw without the screen it continues. Reopen the tab to get the screen first."
    )]
    OutputBeforeSnapshot,
    #[error(
        "The terminal component asked to {frame}, which an attached tab doesn't do. This is a bug in the app; nothing was sent."
    )]
    NotOnAttach { frame: &'static str },
}

/// The first message of an attach that wants checkpoints: from the start,
/// so a screen always comes first. It takes snapshots in pieces, so a
/// screen bigger than one gRPC message still comes, and nothing compressed:
/// the mapping stays pure for the browser.
pub fn open(
    session: Option<pb::SessionId>,
    tab: pb::TabId,
    cols: u16,
    rows: u16,
    read_only: bool,
    window_bytes: u32,
) -> pb::AttachClientMessage {
    pb::AttachClientMessage {
        message: Some(ClientMessage::Open(pb::AttachOpen {
            session,
            tab: Some(tab),
            from_offset: 0,
            read_only,
            cols: u32::from(cols),
            rows: u32::from(rows),
            window_bytes,
            accept_codecs: vec![pb::ChunkCodec::None as i32],
            checkpoint_engine: SNAPSHOT_ENGINE.to_owned(),
        })),
    }
}

/// What to do with one of the replica's frames.
#[derive(Debug)]
pub enum Outgoing {
    Send(pb::AttachClientMessage),
    /// Attach again from the start: the way to a fresh screen.
    Reopen,
    /// End the attach; the program keeps running.
    Close,
    /// Nothing to send.
    Nothing,
}

/// Maps a frame the replica sent to what the attach carries.
pub fn outbound(frame: ClientFrame) -> Result<Outgoing, LinkError> {
    match frame {
        ClientFrame::Input { seq, bytes, .. } => send(ClientMessage::Input(pb::TerminalInput {
            data: bytes,
            input_seq: seq,
        })),
        ClientFrame::Resize {
            cols,
            rows,
            cell_width,
            cell_height,
            ..
        } => send(ClientMessage::Resize(pb::TerminalResize {
            cols: u32::from(cols),
            rows: u32::from(rows),
            cell_width: u32::from(cell_width),
            cell_height: u32::from(cell_height),
        })),
        ClientFrame::Ack { offset, .. } => send(ClientMessage::Ack(pb::OutputAck { offset })),
        ClientFrame::SnapshotRequest { .. } => Ok(Outgoing::Reopen),
        ClientFrame::Detach { .. } => Ok(Outgoing::Close),
        // Every attach that isn't read-only may type and resize (D14), and
        // the attach was opened before the replica bound to it.
        ClientFrame::TakeControl { .. } | ClientFrame::Open { .. } => Ok(Outgoing::Nothing),
        ClientFrame::Authenticate { .. } => Err(LinkError::NotOnAttach { frame: "authenticate" }),
        ClientFrame::List => Err(LinkError::NotOnAttach { frame: "list terminals" }),
        ClientFrame::Stop { .. } => Err(LinkError::NotOnAttach { frame: "stop the program" }),
        ClientFrame::Create { .. } => Err(LinkError::NotOnAttach { frame: "start a program" }),
        ClientFrame::SetRecovery { .. } => Err(LinkError::NotOnAttach { frame: "set recovery" }),
        ClientFrame::SetHistory { .. } => Err(LinkError::NotOnAttach { frame: "set history" }),
    }
}

fn send(message: ClientMessage) -> Result<Outgoing, LinkError> {
    Ok(Outgoing::Send(pb::AttachClientMessage {
        message: Some(message),
    }))
}

/// A snapshot still arriving in pieces.
struct Pieces {
    offset: u64,
    cols: u16,
    rows: u16,
    uuid: String,
    bytes: Vec<u8>,
}

/// Reads one attach's messages, in order, as server frames for the
/// replica bound to `route`.
pub struct Inbound {
    route: u64,
    pieces: Option<Pieces>,
    installed: bool,
    refused: bool,
    uuid: Option<String>,
    size: Option<(u16, u16, u16, u16)>,
    controller: Option<bool>,
    exited: Option<i32>,
}

fn narrow(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

impl Inbound {
    pub fn new(route: u64) -> Self {
        Inbound {
            route,
            pieces: None,
            installed: false,
            refused: false,
            uuid: None,
            size: None,
            controller: None,
            exited: None,
        }
    }

    /// The identity of the screen installed last: a new one means a new
    /// holder, so the replica for the tab is replaced, not fed.
    pub fn terminal_uuid(&self) -> Option<&str> {
        self.uuid.as_deref()
    }

    pub fn map(&mut self, message: pb::AttachServerMessage) -> Result<Vec<ServerFrame>, LinkError> {
        if self.refused {
            return Err(LinkError::PeerVersionUnsupported);
        }
        let id = self.route;
        match message.message {
            Some(ServerMessage::Snapshot(snapshot)) => self.snapshot(snapshot),
            Some(ServerMessage::Output(output)) => {
                if output.codec != pb::ChunkCodec::None as i32 {
                    return Err(LinkError::UnexpectedCodec { codec: output.codec });
                }
                if !self.installed {
                    return Err(LinkError::OutputBeforeSnapshot);
                }
                Ok(vec![ServerFrame::Output {
                    id,
                    offset: output.offset,
                    bytes: output.data,
                }])
            }
            Some(ServerMessage::InputAccepted(accepted)) => Ok(vec![ServerFrame::InputWritten {
                id,
                seq: accepted.input_seq,
            }]),
            Some(ServerMessage::State(state)) => Ok(self.state(&state)),
            // Humata's own lines are the app's to draw, beside the terminal.
            Some(ServerMessage::Notice(_)) | None => Ok(Vec::new()),
        }
    }

    fn snapshot(&mut self, snapshot: pb::TerminalSnapshot) -> Result<Vec<ServerFrame>, LinkError> {
        if snapshot.engine.is_empty() {
            self.refused = true;
            return Err(LinkError::PeerVersionUnsupported);
        }
        if snapshot.engine != SNAPSHOT_ENGINE {
            return Err(LinkError::EngineMismatch { got: snapshot.engine });
        }
        if snapshot.codec != pb::ChunkCodec::None as i32 {
            return Err(LinkError::UnexpectedCodec { codec: snapshot.codec });
        }
        let pieces = match self.pieces.take() {
            Some(mut pieces) if pieces.offset == snapshot.offset => {
                pieces.bytes.extend_from_slice(&snapshot.vt);
                pieces
            }
            // A new snapshot replaces one left unfinished.
            _ => Pieces {
                offset: snapshot.offset,
                cols: narrow(snapshot.cols),
                rows: narrow(snapshot.rows),
                uuid: snapshot.terminal_uuid,
                bytes: snapshot.vt,
            },
        };
        if snapshot.more {
            self.pieces = Some(pieces);
            return Ok(Vec::new());
        }
        let id = self.route;
        self.installed = true;
        self.uuid = Some(pieces.uuid);
        let (cell_width, cell_height) = self.size.map_or((0, 0), |(_, _, w, h)| (w, h));
        self.size = Some((pieces.cols, pieces.rows, cell_width, cell_height));
        let mut frames = vec![ServerFrame::Snapshot {
            id,
            engine: SNAPSHOT_ENGINE.to_owned(),
            bytes: pieces.bytes,
            offset: pieces.offset,
            cols: pieces.cols,
            rows: pieces.rows,
            live: self.exited.is_none(),
            exited: self.exited,
        }];
        // State that came before the screen applies once it is installed.
        if let Some(controller) = self.controller {
            frames.push(ServerFrame::Control { id, controller });
        }
        Ok(frames)
    }

    fn state(&mut self, state: &pb::TerminalState) -> Vec<ServerFrame> {
        let id = self.route;
        let mut frames = Vec::new();
        let size = (
            narrow(state.cols),
            narrow(state.rows),
            narrow(state.cell_width),
            narrow(state.cell_height),
        );
        if size.0 > 0 && size.1 > 0 && self.size != Some(size) {
            if self.installed {
                frames.push(ServerFrame::Resized {
                    id,
                    cols: size.0,
                    rows: size.1,
                    cell_width: size.2,
                    cell_height: size.3,
                });
            }
            self.size = Some(size);
        }
        let writable = state.writable();
        if writable == Writable::Exited {
            if self.exited.is_none() {
                self.exited = Some(state.exit_code);
                if self.installed {
                    frames.push(ServerFrame::Exited {
                        id,
                        code: Some(state.exit_code),
                    });
                }
            }
            return frames;
        }
        let controller = writable == Writable::Yes;
        if self.controller != Some(controller) {
            self.controller = Some(controller);
            if self.installed {
                frames.push(ServerFrame::Control { id, controller });
            }
        }
        frames
    }
}

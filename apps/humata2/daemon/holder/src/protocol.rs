//! The holder's wire format: frames of a big-endian `u32` length, a tag
//! byte and the body. Only humatad2 and the holder of the same build speak
//! it, so the version in `Hello` must match exactly.
//!
//! Keyed input (`KEYED_INPUT`, `KEY_QUERY` and their replies) was added
//! without a new version: a holder started by an older build keeps
//! running across upgrades, and an older holder answers a frame it
//! doesn't know by closing the connection, which is how a client finds
//! out it can't do keyed input. Older clients never send these frames,
//! and a newer holder only sends them in reply, so nothing else changes.

use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) const VERSION: u32 = 1;

/// No frame is bigger: output and input are sent in `CHUNK`s.
const MAX_FRAME: usize = 1 << 20;
pub(crate) const CHUNK: usize = 64 << 10;
/// The most one keyed input may hold: the holder buffers all of it before
/// typing it, so it is bounded.
pub(crate) const MAX_KEYED: usize = 4 << 20;
/// The longest key, in bytes.
pub(crate) const MAX_KEY: usize = 256;

const HELLO: u8 = 1;
const INPUT: u8 = 2;
const RESIZE: u8 = 3;
const SHUTDOWN: u8 = 4;
const KEYED_INPUT: u8 = 5;
const KEY_QUERY: u8 = 6;
const AWAIT_KEYS: u8 = 7;

const WELCOME: u8 = 101;
const OUTPUT: u8 = 102;
const TRIMMED: u8 = 103;
const EXITED: u8 = 104;
const INPUT_WRITTEN: u8 = 105;
const REFUSED: u8 = 106;
const KEYED_WRITTEN: u8 = 107;
const KEY_ANSWER: u8 = 108;
const KEYS_READ: u8 = 109;

/// Where the terminal stands when a client attaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// The oldest byte still kept.
    pub earliest: u64,
    /// The offset the next byte of output will have.
    pub end: u64,
    pub cols: u16,
    pub rows: u16,
    /// Set once the program has ended.
    pub exited: Option<i32>,
}

pub(crate) enum ClientFrame {
    Hello { version: u32, from: u64 },
    Input { seq: u64, last: bool, data: Vec<u8> },
    Resize { cols: u16, rows: u16 },
    Shutdown,
    /// A chunk of input to type once, all together, after its `last`
    /// chunk. Typed once per `key`, however often it's sent.
    KeyedInput { key: String, last: bool, data: Vec<u8> },
    /// Asks whether a keyed input with `key` was typed.
    KeyQuery { key: String },
    /// Asks to be answered once the program reads keys itself (the
    /// terminal's line editing is off), or once it has run for
    /// `starting_ms`.
    AwaitKeys { starting_ms: u32 },
}

pub(crate) enum ServerFrame {
    Welcome(Welcome),
    Output { offset: u64, data: Vec<u8> },
    Trimmed { resume_at: u64 },
    Exited { code: i32 },
    InputWritten { seq: u64 },
    Refused { message: String },
    /// A keyed input was typed now, or (`repeat`) had been before.
    KeyedWritten { repeat: bool },
    /// Whether the keyed input asked about was typed, and which holder
    /// process answers: `instance` differs between runs of a holder, even
    /// for the same tab, so a client can tell a "no" from the holder that
    /// was there from a "no" from a fresh one with an empty memory.
    KeyAnswer { arrived: bool, instance: u64 },
    /// The answer to `AwaitKeys`: whether the program reads keys itself
    /// now. `false` when it ran for the time given or ended first.
    KeysRead { ready: bool },
}

#[derive(Debug)]
pub(crate) enum FrameError {
    Io(std::io::Error),
    Malformed(&'static str),
}

struct Frame(Vec<u8>);

impl Frame {
    fn new(tag: u8) -> Self {
        Self(vec![0, 0, 0, 0, tag])
    }

    fn u8(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    fn u16(mut self, value: u16) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    fn u64(mut self, value: u64) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    fn i32(mut self, value: i32) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    fn key(self, key: &str) -> Self {
        // Callers keep keys within MAX_KEY.
        let len = u16::try_from(key.len()).unwrap_or(u16::MAX);
        self.u16(len).rest(key.as_bytes())
    }

    fn rest(mut self, bytes: &[u8]) -> Self {
        self.0.extend_from_slice(bytes);
        self
    }

    fn finish(mut self) -> Vec<u8> {
        // Frames are built from CHUNK-sized pieces, far under u32::MAX.
        let len = u32::try_from(self.0.len() - 4).unwrap_or(u32::MAX);
        self.0[..4].copy_from_slice(&len.to_be_bytes());
        self.0
    }
}

struct Body<'a>(&'a [u8]);

impl<'a> Body<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], FrameError> {
        let Some((head, tail)) = self.0.split_first_chunk::<N>() else {
            return Err(FrameError::Malformed("a frame ended early"));
        };
        self.0 = tail;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, FrameError> {
        Ok(u16::from_be_bytes(self.take()?))
    }

    fn u32(&mut self) -> Result<u32, FrameError> {
        Ok(u32::from_be_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, FrameError> {
        Ok(u64::from_be_bytes(self.take()?))
    }

    fn i32(&mut self) -> Result<i32, FrameError> {
        Ok(i32::from_be_bytes(self.take()?))
    }

    fn key(&mut self) -> Result<String, FrameError> {
        let len = usize::from(self.u16()?);
        let Some((head, tail)) = self.0.split_at_checked(len) else {
            return Err(FrameError::Malformed("a frame ended early"));
        };
        self.0 = tail;
        String::from_utf8(head.to_vec()).map_err(|_| FrameError::Malformed("a key was not text"))
    }

    fn rest(self) -> &'a [u8] {
        self.0
    }

    fn end(self) -> Result<(), FrameError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Malformed("a frame was longer than its kind"))
        }
    }
}

fn split(frame: &[u8]) -> Result<(u8, Body<'_>), FrameError> {
    match frame.split_first() {
        Some((tag, body)) => Ok((*tag, Body(body))),
        None => Err(FrameError::Malformed("a frame was empty")),
    }
}

impl ClientFrame {
    pub(crate) fn encode(&self) -> Vec<u8> {
        match self {
            Self::Hello { version, from } => Frame::new(HELLO).u32(*version).u64(*from),
            Self::Input { seq, last, data } => {
                Frame::new(INPUT).u64(*seq).u8(u8::from(*last)).rest(data)
            }
            Self::Resize { cols, rows } => Frame::new(RESIZE).u16(*cols).u16(*rows),
            Self::Shutdown => Frame::new(SHUTDOWN),
            Self::KeyedInput { key, last, data } => Frame::new(KEYED_INPUT)
                .key(key)
                .u8(u8::from(*last))
                .rest(data),
            Self::KeyQuery { key } => Frame::new(KEY_QUERY).key(key),
            Self::AwaitKeys { starting_ms } => Frame::new(AWAIT_KEYS).u32(*starting_ms),
        }
        .finish()
    }

    pub(crate) fn decode(frame: &[u8]) -> Result<Self, FrameError> {
        let (tag, mut body) = split(frame)?;
        let decoded = match tag {
            HELLO => Self::Hello {
                version: body.u32()?,
                from: body.u64()?,
            },
            INPUT => {
                return Ok(Self::Input {
                    seq: body.u64()?,
                    last: body.u8()? != 0,
                    data: body.rest().to_vec(),
                });
            }
            RESIZE => Self::Resize {
                cols: body.u16()?,
                rows: body.u16()?,
            },
            SHUTDOWN => Self::Shutdown,
            KEYED_INPUT => {
                return Ok(Self::KeyedInput {
                    key: body.key()?,
                    last: body.u8()? != 0,
                    data: body.rest().to_vec(),
                });
            }
            KEY_QUERY => Self::KeyQuery { key: body.key()? },
            AWAIT_KEYS => Self::AwaitKeys {
                starting_ms: body.u32()?,
            },
            _ => return Err(FrameError::Malformed("a client sent a frame of no known kind")),
        };
        body.end()?;
        Ok(decoded)
    }
}

/// An output frame without copying `data` into a `ServerFrame` first.
pub(crate) fn output_frame(offset: u64, data: &[u8]) -> Vec<u8> {
    Frame::new(OUTPUT).u64(offset).rest(data).finish()
}

impl ServerFrame {
    pub(crate) fn encode(&self) -> Vec<u8> {
        match self {
            Self::Welcome(welcome) => Frame::new(WELCOME)
                .u64(welcome.earliest)
                .u64(welcome.end)
                .u16(welcome.cols)
                .u16(welcome.rows)
                .u8(u8::from(welcome.exited.is_some()))
                .i32(welcome.exited.unwrap_or(0)),
            Self::Output { offset, data } => return output_frame(*offset, data),
            Self::Trimmed { resume_at } => Frame::new(TRIMMED).u64(*resume_at),
            Self::Exited { code } => Frame::new(EXITED).i32(*code),
            Self::InputWritten { seq } => Frame::new(INPUT_WRITTEN).u64(*seq),
            Self::Refused { message } => Frame::new(REFUSED).rest(message.as_bytes()),
            Self::KeyedWritten { repeat } => Frame::new(KEYED_WRITTEN).u8(u8::from(*repeat)),
            Self::KeyAnswer { arrived, instance } => {
                Frame::new(KEY_ANSWER).u8(u8::from(*arrived)).u64(*instance)
            }
            Self::KeysRead { ready } => Frame::new(KEYS_READ).u8(u8::from(*ready)),
        }
        .finish()
    }

    pub(crate) fn decode(frame: &[u8]) -> Result<Self, FrameError> {
        let (tag, mut body) = split(frame)?;
        let decoded = match tag {
            WELCOME => {
                let earliest = body.u64()?;
                let end = body.u64()?;
                let cols = body.u16()?;
                let rows = body.u16()?;
                let exited = body.u8()? != 0;
                let code = body.i32()?;
                Self::Welcome(Welcome {
                    earliest,
                    end,
                    cols,
                    rows,
                    exited: exited.then_some(code),
                })
            }
            OUTPUT => {
                return Ok(Self::Output {
                    offset: body.u64()?,
                    data: body.rest().to_vec(),
                });
            }
            TRIMMED => Self::Trimmed {
                resume_at: body.u64()?,
            },
            EXITED => Self::Exited { code: body.i32()? },
            INPUT_WRITTEN => Self::InputWritten { seq: body.u64()? },
            KEYED_WRITTEN => Self::KeyedWritten {
                repeat: body.u8()? != 0,
            },
            KEY_ANSWER => Self::KeyAnswer {
                arrived: body.u8()? != 0,
                instance: body.u64()?,
            },
            KEYS_READ => Self::KeysRead {
                ready: body.u8()? != 0,
            },
            REFUSED => {
                return Ok(Self::Refused {
                    message: String::from_utf8_lossy(body.rest()).into_owned(),
                });
            }
            _ => return Err(FrameError::Malformed("the holder sent a frame of no known kind")),
        };
        body.end()?;
        Ok(decoded)
    }
}

/// Reads one frame, or `None` if the other side closed between frames.
pub(crate) async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len = [0; 4];
    match reader.read_exact(&mut len).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(FrameError::Io(error)),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(FrameError::Malformed("a frame's length is out of range"));
    }
    let mut frame = vec![0; len];
    reader
        .read_exact(&mut frame)
        .await
        .map_err(FrameError::Io)?;
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::{ClientFrame, FrameError, ServerFrame};

    fn body(encoded: &[u8]) -> &[u8] {
        &encoded[4..]
    }

    #[test]
    fn a_keyed_input_survives_the_wire() {
        let sent = ClientFrame::KeyedInput {
            key: "s1/d-4".to_owned(),
            last: true,
            data: b"hello\n".to_vec(),
        };
        let Ok(ClientFrame::KeyedInput { key, last, data }) =
            ClientFrame::decode(body(&sent.encode()))
        else {
            panic!("decoded as something else");
        };
        assert_eq!((key.as_str(), last, data.as_slice()), ("s1/d-4", true, &b"hello\n"[..]));
    }

    #[test]
    fn a_key_query_and_its_answers_survive_the_wire() {
        let query = ClientFrame::KeyQuery { key: "k".to_owned() };
        assert!(matches!(
            ClientFrame::decode(body(&query.encode())),
            Ok(ClientFrame::KeyQuery { key }) if key == "k"
        ));
        let answer = ServerFrame::KeyAnswer {
            arrived: true,
            instance: 77,
        };
        assert!(matches!(
            ServerFrame::decode(body(&answer.encode())),
            Ok(ServerFrame::KeyAnswer {
                arrived: true,
                instance: 77
            })
        ));
        let written = ServerFrame::KeyedWritten { repeat: true };
        assert!(matches!(
            ServerFrame::decode(body(&written.encode())),
            Ok(ServerFrame::KeyedWritten { repeat: true })
        ));
    }

    #[test]
    fn a_key_longer_than_its_frame_is_malformed() {
        // Tag 6 (key query), a key length of 9, then 2 bytes.
        let frame = [6, 0, 9, b'a', b'b'];
        assert!(matches!(
            ClientFrame::decode(&frame),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn a_frame_of_an_unknown_kind_is_rejected() {
        assert!(matches!(
            ClientFrame::decode(&[200]),
            Err(FrameError::Malformed(_))
        ));
        assert!(matches!(
            ServerFrame::decode(&[200]),
            Err(FrameError::Malformed(_))
        ));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("couldn't read from the other side; the connection broke, so reconnect")]
    Read {
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't write to the other side; the connection broke, so reconnect")]
    Write {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the connection ended in the middle of a {length}-byte message; the other side stopped or the connection broke, so reconnect"
    )]
    Truncated {
        length: usize,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "a message of {length} bytes is over the limit of {max}; if it is a session's history, read it in parts. If the other side isn't humatad, check the command that starts it"
    )]
    TooLarge { length: usize, max: usize },

    #[error(
        "the other side sent a message this program can't decode; it may not be humatad, or not the same version. Check the command that starts it and update Humata on both sides"
    )]
    Decode {
        #[source]
        source: prost::DecodeError,
    },

    #[error(
        "the other side sent a compressed message that isn't valid zstd; the connection may be corrupted, so reconnect, and update Humata on both sides if it happens again"
    )]
    Decompress {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the other side didn't start with a hello, so it may not be humatad; check the command that starts it. On SSH, a login script that prints text breaks the connection: keep it quiet when not interactive"
    )]
    NoHello,

    #[error(
        "the other side closed the connection before saying hello; check that humatad is installed there and starts. Running the same command by hand shows its error"
    )]
    ClosedBeforeHello,

    #[error(
        "the other side speaks protocol version {theirs} (humatad {their_program}) and this program speaks version {ours}; update Humata so both sides are the same version"
    )]
    VersionMismatch {
        ours: i32,
        theirs: i32,
        their_program: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Wire(#[from] WireError),

    #[error(
        "the connection to humatad closed before it answered; check that it is still running, then retry. The request may or may not have been carried out"
    )]
    Disconnected,

    #[error(
        "humatad answered with {got} where it should send {expected}; update Humata so both sides are the same version"
    )]
    Unexpected { expected: &'static str, got: String },

    #[error("{message}")]
    Refused {
        code: crate::pb::ErrorCode,
        message: String,
    },
}

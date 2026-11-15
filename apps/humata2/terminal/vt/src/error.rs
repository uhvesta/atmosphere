use std::fmt;

use crate::ffi;

/// A result code libghostty-vt returned instead of success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status(i32);

impl Status {
    /// The raw `GhosttyResult` code.
    pub fn code(self) -> i32 {
        self.0
    }

    /// Whether libghostty-vt couldn't allocate memory.
    pub fn is_out_of_memory(self) -> bool {
        self.0 == ffi::OUT_OF_MEMORY
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.0 {
            ffi::OUT_OF_MEMORY => "it ran out of memory",
            ffi::INVALID_VALUE => "it was given a value it doesn't accept",
            ffi::OUT_OF_SPACE => "the buffer it was given was too small",
            ffi::NO_VALUE => "there was no value to return",
            ffi::IO_ERROR => "an I/O operation failed",
            ffi::LIMIT_EXCEEDED => "a configured limit was reached",
            ffi::REJECTED => "it rejected the request",
            ffi::UNSUPPORTED => "this build doesn't support the request",
            _ => "it returned an unknown result",
        };
        write!(f, "libghostty-vt: {what} (result {})", self.0)
    }
}

impl std::error::Error for Status {}

/// Turns a `GhosttyResult` into `Ok` or the [`Status`] it was.
pub(crate) fn check(result: ffi::GhosttyResult) -> Result<(), Status> {
    if result == ffi::SUCCESS {
        Ok(())
    } else {
        Err(Status(result))
    }
}

/// What went wrong in a call on a [`Terminal`](crate::Terminal).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Couldn't create a {cols}x{rows} terminal. Both sizes must be at least 1; if they \
         are, the system is probably out of memory, so close other terminals and try again."
    )]
    Create {
        cols: u16,
        rows: u16,
        #[source]
        source: Status,
    },

    #[error(
        "Couldn't set up the terminal's {what}. This build of ghostty_vt doesn't match the \
         libghostty-vt it links; rebuild both from the pinned source in libghostty_vt.bzl."
    )]
    Configure {
        what: &'static str,
        #[source]
        source: Status,
    },

    #[error(
        "Couldn't resize the terminal to {cols}x{rows} cells of {cell_width}x{cell_height} \
         pixels. Both sizes must be at least 1; if they are, the system is probably out of \
         memory."
    )]
    Resize {
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
        #[source]
        source: Status,
    },

    #[error(
        "Row {y} isn't on the terminal's screen. Ask only for rows below the screen's row \
         count, which changes as lines scroll into history and history is dropped."
    )]
    Row {
        y: u32,
        #[source]
        source: Status,
    },

    #[error(
        "The point at column {x}, row {y} isn't on the terminal's screen, so it can't start \
         or end a selection. Pick points inside the screen; the screen may have changed since \
         they were taken."
    )]
    Point {
        x: u16,
        y: u32,
        #[source]
        source: Status,
    },

    #[error("Couldn't copy the selected text out of the terminal. Try selecting it again.")]
    Format {
        #[source]
        source: Status,
    },

    #[error("Couldn't search the terminal for {needle:?}. Try the search again.")]
    Search {
        needle: String,
        #[source]
        source: Status,
    },

    #[error("Couldn't scroll the terminal's view.")]
    Scroll {
        #[source]
        source: Status,
    },

    #[error(
        "Couldn't turn the key press into the bytes a program reads, so it wasn't sent. \
         Press the key again."
    )]
    EncodeKey {
        #[source]
        source: Status,
    },

    #[error("Couldn't turn the mouse event into the bytes a program reads, so it wasn't sent.")]
    EncodeMouse {
        #[source]
        source: Status,
    },
}

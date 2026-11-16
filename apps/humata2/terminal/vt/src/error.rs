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
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error(
        "The restored terminal could not fit a {requested}-byte allocation within its {limit}-byte memory budget. Preserve the checkpoint and reduce retained history; damaged checkpoints must be restored from a compatible backup."
    )]
    CheckpointMemoryLimit {
        requested: usize,
        limit: usize,
        #[source]
        source: Status,
    },
    #[error(
        "The {cols}x{rows} terminal grid is unsupported. Use dimensions from 1 to {max_dimension} with at most {max_cells} cells; preserve any saved checkpoint."
    )]
    GridLimit {
        cols: u16,
        rows: u16,
        max_dimension: u16,
        max_cells: usize,
    },
    #[error(
        "The checkpoint's page capacity estimate is {requested} bytes, above the conservative {limit}-byte restore budget. Reduce retained history or restore a compatible backup; preserve the existing checkpoint."
    )]
    CheckpointPageLimit { requested: u64, limit: u64 },
    #[error(
        "Couldn't {operation} the terminal checkpoint. Preserve the previous checkpoint and retry with a compatible engine; an unfinished control sequence may exceed the configured continuation limit."
    )]
    Checkpoint {
        operation: &'static str,
        #[source]
        source: Status,
    },
    #[error(
        "The terminal checkpoint needs {required} bytes, above the configured {limit}-byte limit. Reduce retained history or raise the limit; preserve the existing checkpoint."
    )]
    CheckpointTooLarge { required: usize, limit: usize },
    #[error(
        "Couldn't allocate {bytes} bytes for the terminal checkpoint. Close other terminals or reduce retained history, and preserve the previous checkpoint."
    )]
    CheckpointAllocation {
        bytes: usize,
        #[source]
        source: std::collections::TryReserveError,
    },
    #[error(
        "The terminal checkpoint is incompatible or damaged ({reason}). Preserve it and restore a compatible backup; do not reinterpret it as terminal output."
    )]
    CheckpointHeader { reason: &'static str },
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

    #[error(
        "Couldn't capture the terminal's screen for drawing; the system is probably out of \
         memory. The last frame stays on screen; close other terminals and it redraws."
    )]
    Render {
        #[source]
        source: Status,
    },

    #[error("Couldn't set the terminal's {what}. Check the theme's colors and set them again.")]
    Colors {
        what: &'static str,
        #[source]
        source: Status,
    },

    #[error(
        "Couldn't change the selection. The screen may have changed under it; select the \
         text again."
    )]
    Selection {
        #[source]
        source: Status,
    },

    #[error("Couldn't prepare the pasted text for the program, so nothing was sent. Paste again.")]
    Paste {
        #[source]
        source: Status,
    },

    #[error(
        "The pasted text could run commands: it holds {line_breaks} line break(s) or the \
         marker that ends a bracketed paste, and the program didn't ask for bracketed paste, \
         so a break would act as Enter. Nothing was sent. Ask the user to confirm before \
         pasting it anyway; removing the breaks is safe only if the text is meant as one line."
    )]
    UnsafePaste { line_breaks: usize },

    #[error(
        "Couldn't encode the focus change for the program, so it wasn't told. It catches up \
         on the next focus change."
    )]
    EncodeFocus {
        #[source]
        source: Status,
    },
}

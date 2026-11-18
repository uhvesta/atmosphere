//! One terminal's pseudo-terminal and scrollback, held in a process of its
//! own so the program in it outlives humatad2.
//!
//! `humata2_holder` runs the program and serves its output on a Unix
//! socket. Every byte has an offset, counted from the program's first, so
//! a client that drops reattaches from the last offset it has and misses
//! nothing still kept. The holder stays after the program ends, so its
//! exit and last output can still be read, until it's told to shut down.
//! Its output is saved beside its socket, so a holder started there after
//! it was killed or lost to a reboot shows it again.

mod client;
mod end;
mod error;
mod protocol;
pub mod process;
mod saved;
mod scrollback;
mod server;
mod shown;

pub use client::{
    Attachment, Control, Event, Events, HeldReturn, HolderSpec, KeyState, KeyedInput, OutputWatch,
    Ready, ReadsKeys, Spawned, Typed, attach, await_keys, deliver, key_arrived, keyed_input,
    shutdown, spawn, spawn_memory_only, spawn_with_banner, type_then_return, watch_output,
};
pub use end::{EndError, Stopped, end_by_pid, find_holder, shutdown_or_end};
pub use error::HolderError;
pub use protocol::Welcome;
pub use saved::{SavedOutput, read_saved, read_saved_tail, saved_scrollback_path, stopped_mark_path};
pub use server::{ServeOptions, holder_pid, serve};

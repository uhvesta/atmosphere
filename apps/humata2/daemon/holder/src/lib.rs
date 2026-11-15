//! One terminal's pseudo-terminal and scrollback, held in a process of its
//! own so the program in it outlives humatad2.
//!
//! `humata2_holder` runs the program and serves its output on a Unix
//! socket. Every byte has an offset, counted from the program's first, so
//! a client that drops reattaches from the last offset it has and misses
//! nothing still kept. The holder stays after the program ends, so its
//! exit and last output can still be read, until it's told to shut down.

mod client;
mod error;
mod protocol;
mod scrollback;
mod server;

pub use client::{Attachment, Control, Event, Events, HolderSpec, Spawned, attach, shutdown, spawn};
pub use error::HolderError;
pub use protocol::Welcome;
pub use server::{ServeOptions, serve};

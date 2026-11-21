//! `humata2_daemon_runtime`: stateful daemon runtime internals and zero-service
//! runtime helpers for `humatad2` (SPLIT §3.1, §3.2; SPLIT_PHASE3_DAEMON §2.1
//! Step 6a).

pub mod agent_gate;
pub mod agent_switches;
pub mod fault;
pub mod paste;
pub mod ready;
pub mod release;
pub mod screen_question;
pub mod stop_gate;
pub mod terminal_uuid;
pub mod tool_path;

pub use release::{
    BUNDLE_HEADER, Release, host_files, installed_tools, names_file, names_whole_entry, short,
};

//! File transfers and the Humata trash: copy, move and upload files and
//! folders in chunks, resumable and checksummed, and finished or cleanly
//! undone after a crash at any step (FM-3, FM-6, FM-8, FM-14, LD-6, LD-7,
//! LD-19, TR-11, PER-27). Files are touched only through [`End`]; the
//! crate knows nothing about gRPC. Also what keeps disks tidy: measuring
//! what a machine holds (LD-13, LD-14, LD-15) and deciding what the
//! opt-in retention rules archive ([`retention`], LD-16, LD-17).

mod disk;
mod end;
mod engine;
mod error;
mod fsutil;
mod record;
pub mod retention;
mod storage;
mod trash;
mod upload;

pub use disk::Disk;
pub use end::{End, Entry, Kind, Sink};
pub use engine::{
    CHUNK, Config, Item, Landing, Listener, Mode, OnExisting, Problem, Request, Snapshot, State, Transfers,
    UploadRequest,
};
pub use error::{EndError, TransferError, TrashError};
pub use storage::{
    BuildCache, DiskSpace, Measured, StorageError, build_caches, clear, disk_space, measure,
    sweep_cleared, trim_logs,
};
pub use trash::{EmptyPlan, KEEP_MS, NewItem, Trash, TrashItem};
pub use upload::Upload;

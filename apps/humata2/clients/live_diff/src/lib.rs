//! Session diffs, push plans, and session file reads over the daemon's
//! `DiffService`, `FileService`, and `SessionService`.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod humata2_grpc {
    pub use humata2_grpc_core::{Duration, Timestamp};
    pub mod humata {
        pub mod v1 {
            pub use humata2_grpc_core::humata::v1::*;
            pub use humata2_grpc_files::humata::v1::*;
            pub use humata2_grpc_review::humata::v1::*;
            pub use humata2_grpc_sessions::humata::v1::*;
        }
    }
}

mod diff;

pub use diff::{
    file_tier, find_default_branch, find_session_files, looks, save_looks, mark_reviewed, plan_push, push,
    read_session_file, repo_bases, repo_diff, session_diffs, spec_to_wire,
};

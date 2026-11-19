//! Review comments, prepared views, pull request reviews, and review chat
//! clients over the daemon's review gRPC services.

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod humata2_grpc {
    pub use humata2_grpc_core::{Duration, Timestamp};
    pub mod humata {
        pub mod v1 {
            pub use humata2_grpc_core::humata::v1::*;
            pub use humata2_grpc_events::humata::v1::*;
            pub use humata2_grpc_review::humata::v1::*;
        }
    }
}

mod comments;
mod pull_requests;
mod review_chat;
mod views;

pub use comments::LiveComments;
pub use pull_requests::LivePullRequests;
pub use review_chat::LiveReviewChat;
pub use views::LiveViews;

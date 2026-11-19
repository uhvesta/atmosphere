//! [`DeskClient`] and document versions over `humata.v1.DeskService`.

pub(crate) mod pb {
    #![allow(unused_imports)]
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_desk::humata::v1::*;
    pub use humata2_grpc_review::humata::v1::*;
}

mod desk;
mod docs;

pub use desk::{LiveDesk, change, load_desk};

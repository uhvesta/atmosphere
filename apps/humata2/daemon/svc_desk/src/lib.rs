//! `DeskService` (`humata2_svc_desk`): Desk sessions, channel desks, document
//! watching/versions, and moving Desk sessions into channels.

pub mod ports;
pub mod desk_service;

pub use desk_service::{DeskDeps, DeskSvc, Locks, server};

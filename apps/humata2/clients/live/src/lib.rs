//! The app's real daemon clients: thin facade re-exporting [`LiveConnection`],
//! [`LiveDaemon`], and the per-domain live clients.

pub use humata2_clients_live_core::{
    Call, ConnectError, DaemonFacts, LiveConnection, LiveOptions, Observer,
};
pub use humata2_clients_live_daemon::{LiveDaemon, history, marks};
pub use humata2_clients_live_desk::LiveDesk;
pub use humata2_clients_live_files::LiveFileManager;
pub use humata2_clients_live_machines::LiveMachines;
pub use humata2_clients_live_review::{
    LiveComments, LivePullRequests, LiveReviewChat, LiveViews,
};

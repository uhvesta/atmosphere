//! Same-release peers (UP-1, D281, D290): what release this daemon is
//! (`build`), the VIP "Migrate <thing>" Inbox items a mismatch posts
//! (`items`), and the refusals that stand in for a mixed-version fallback
//! (`gate`). Carrying a Migrate out on this machine: the Upgrade call and
//! the swap (`run`), the record that makes a cut one safe (`record`), and
//! undoing a failed or cut one (`rollback`). Carrying one out on another
//! machine over SSH and its peer link (`remote`). What this Mac's own bin
//! folder holds beside the running daemon: a staged release, and a CLI or
//! holder that isn't the release's (`local`, `tools`). Which holder file
//! each session's holder was spawned from, and the item for the agents still
//! on an old one (`holders`).

pub(crate) mod build;
pub(crate) mod gate;
pub(crate) mod holders;
pub(crate) mod items;
pub(crate) mod local;
pub(crate) mod record;
pub(crate) mod remote;
pub(crate) mod rollback;
pub(crate) mod run;
pub(crate) mod tools;

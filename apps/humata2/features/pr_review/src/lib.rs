//! Reviewing a pull request with an agent beside you (PR-1 to PR-26, D59,
//! D283 to D300): the start dialog, the chat follower, Submit review, and the
//! notice that new commits arrived.
//!
//! The review agent never reviews on its own: it answers, moves the view and
//! leaves notes (D59). Everything here is state, actions and effects; the
//! daemon owns GitHub, the clones and the drafts, and the views are drawn in
//! `ui`. Each part is its own reducer, so the app holds the parts it shows:
//!
//! - [`ask`]: a question about lines of the pull request, typed into the
//!   review chat and never stored or posted (PR-19).
//! - [`dialog`]: paste a link, add more pull requests, pick where and who,
//!   Start (PR-1 to PR-8).
//! - [`follower`]: applies the agent's view changes once the person has been
//!   quiet, with Undo for each and Back (PR-24, D286).
//! - [`submit`]: Submit review, drafts per pull request, verdicts, partial
//!   failure (PR-12 to PR-14).
//! - [`info`]: the Info panel's switches: the agent may push, and may Focus,
//!   leave Notes and set the Order (PR-8, PR-25).
//! - [`heads`]: new commits arrive on the feed, never by a timer here
//!   (PR-15 to PR-17).

pub mod ask;
pub mod dialog;
pub mod follower;
pub mod heads;
pub mod info;
pub mod submit;

//! The Diff panel's loading line (D807 P1, T7): while a read of the open
//! session's changes is in flight, the shared sweep runs along the top edge
//! of the panel's body. Nothing shows for the first 300 ms, so a fast read
//! never flickers. It lies over the body, so the panel doesn't move when it
//! comes and goes.

use std::collections::HashMap;
use std::time::Duration;

use crate::theme::Look;
use crate::widgets;
use gpui_kit::{prelude::*, *};
use humata2_clients::model::SessionId;
use humata2_review::Fetch;

/// Whether `session`'s changes are being read: only the open session's own
/// read counts, so switching sessions mid-read shows the new one's state.
fn reading(fetch: &HashMap<SessionId, Fetch>, session: Option<&SessionId>) -> bool {
    session
        .and_then(|session| fetch.get(session))
        .is_some_and(|fetch| fetch.loading)
}

/// The loading line for the Diff panel's body, or nothing when no read of
/// `session`'s changes is in flight. The caller's body must be `relative()`.
pub fn changes_loading(
    look: Look,
    fetch: &HashMap<SessionId, Fetch>,
    session: Option<&SessionId>,
) -> Option<AnyElement> {
    let session = session.filter(|_| reading(fetch, session))?;
    // Keyed by session: another session's read starts its own wait.
    let key = &session.0;
    let line = div()
        .id("changes-loading")
        .role(Role::ProgressIndicator)
        .aria_label("Loading")
        .test_support()
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .right(px(0.))
        .child(widgets::progress_bar(
            look,
            None,
            widgets::BarMetrics::line(look),
            SharedString::from(format!("changes-sweep {key}")),
        ));
    // The wait before it shows: hidden until the read has taken 300 ms.
    // Element state lasts only while the line is drawn, so every read
    // waits afresh.
    let wait = Duration::from_millis(widgets::SHOW_AFTER_MS.unsigned_abs());
    Some(
        line.with_animation(
            SharedString::from(format!("changes-loading-wait {key}")),
            Animation::new(wait).with_max_fps(10.),
            |line, t| line.opacity(if t < 1. { 0. } else { 1. }),
        )
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;

    use super::*;

    fn id(name: &str) -> SessionId {
        SessionId(name.to_owned())
    }

    fn fetches(pairs: &[(&str, bool)]) -> HashMap<SessionId, Fetch> {
        pairs
            .iter()
            .map(|(name, loading)| {
                (
                    id(name),
                    Fetch {
                        loading: *loading,
                        ..Fetch::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_read_in_flight_shows_the_line() {
        assert!(reading(&fetches(&[("a", true)]), Some(&id("a"))));
    }

    #[test]
    fn the_diff_arriving_clears_it() {
        assert!(!reading(&fetches(&[("a", false)]), Some(&id("a"))));
    }

    #[test]
    fn switching_session_mid_read_shows_the_new_sessions_state_only() {
        let reads = fetches(&[("a", true), ("b", false)]);
        assert!(!reading(&reads, Some(&id("b"))));
        assert!(reading(&reads, Some(&id("a"))));
        let reads = fetches(&[("a", false), ("b", true)]);
        assert!(reading(&reads, Some(&id("b"))));
    }

    #[test]
    fn nothing_open_or_never_read_shows_nothing() {
        assert!(!reading(&fetches(&[("a", true)]), None));
        assert!(!reading(&fetches(&[]), Some(&id("a"))));
    }
}

//! DS-76: opening, closing, widening or going full screen on the Diff panel
//! does not read the whole diff again. Only a base pick, Try again, a turn
//! end or new commits do, and so does an open whose diff is older than
//! [`FRESH`] (D-REVIEWUI-1, provisional: a change made in the terminal
//! without a turn end still shows the next time the panel opens).

use std::time::Duration;

use crate::Fetch;

/// How long a read diff counts as current for another open of its panel.
pub(crate) const FRESH: Duration = Duration::from_secs(60);

/// Whether opening `session`'s panel again must read its diff. `same` says
/// the session is the one already open; `fetch` is its last read, if any;
/// `now` is the clock.
pub(crate) fn reads_again(same: bool, fetch: Option<&Fetch>, now: Duration) -> bool {
    let Some(fetch) = fetch.filter(|_| same) else {
        return true;
    };
    if fetch.diffs.is_none() || fetch.error.is_some() {
        return true;
    }
    match fetch.read_at {
        Some(read) => now.saturating_sub(read) >= FRESH,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(read: u64) -> Fetch {
        Fetch {
            diffs: Some(Vec::new()),
            read_at: Some(Duration::from_secs(read)),
            ..Fetch::default()
        }
    }

    #[test]
    fn a_fresh_diff_of_the_open_session_is_not_read_again() {
        assert!(!reads_again(true, Some(&held(100)), Duration::from_secs(130)));
    }

    #[test]
    fn another_session_is_read() {
        assert!(reads_again(false, Some(&held(100)), Duration::from_secs(101)));
    }

    #[test]
    fn nothing_held_or_a_failed_read_is_read() {
        assert!(reads_again(true, None, Duration::from_secs(1)));
        let failed = Fetch {
            error: Some("git broke".into()),
            ..held(100)
        };
        assert!(reads_again(true, Some(&failed), Duration::from_secs(101)));
        let empty = Fetch::default();
        assert!(reads_again(true, Some(&empty), Duration::from_secs(1)));
    }

    #[test]
    fn a_diff_older_than_a_minute_is_read_again() {
        assert!(reads_again(true, Some(&held(100)), Duration::from_secs(160)));
    }
}

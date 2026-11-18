//! A6, PR-15, D286: the daemon follows its reviews' pull requests on its
//! own. Every minute while the app reports a review open, every five
//! minutes otherwise, one batched GitHub query asks for the heads of every
//! pull request that is due, and each review whose pull requests moved
//! gets a `PullRequestHeadsChanged` on the sessions topic. Nothing is
//! fetched into a clone that the last check already fetched.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_store::ReviewPr;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::PullRequests;
use super::heads::{Found, HeadsError, changes_where, found_blocking, record_checked, remotes_blocking};
use crate::channel_service::blocking;
use crate::events::Scope;

/// How often a review the app has open is looked at.
const OPEN_EVERY: Duration = Duration::from_secs(60);

/// How often a review that isn't open is looked at.
const IDLE_EVERY: Duration = Duration::from_secs(5 * 60);

/// How often the polls look, by whether the review is open in the app. A
/// test daemon (an `admin` build) takes `HUMATA2_PR_POLL_MS` for the open
/// interval, and looks at the others five times as slowly, as a release
/// build does.
struct Cadence {
    open: Duration,
    idle: Duration,
}

impl Cadence {
    fn current() -> Self {
        #[cfg(feature = "admin")]
        if let Some(ms) = std::env::var("HUMATA2_PR_POLL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            let open = Duration::from_millis(ms.max(1));
            return Self {
                open,
                idle: open.saturating_mul(5),
            };
        }
        Self {
            open: OPEN_EVERY,
            idle: IDLE_EVERY,
        }
    }
}

/// One review's pull requests as a poll found them.
struct Looked {
    session: String,
    channel: String,
    found: Result<Vec<Found>, HeadsError>,
}

impl PullRequests {
    /// Starts following every review's pull requests, for as long as the
    /// daemon runs. Called once, in the daemon's runtime.
    pub(crate) fn spawn_poll(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.poll().await });
    }

    async fn poll(self) {
        let cadence = Cadence::current();
        let mut last: HashMap<String, Instant> = HashMap::new();
        // With no open pull request to follow, the poll runs no timer, so
        // an idle daemon sleeps (TR-7). A review only comes to be through
        // a write to the store, so the poll waits for the next write and
        // looks a cadence later.
        let wrote = Arc::new(Notify::new());
        let woken = wrote.clone();
        self.inner.store.on_commit(move || woken.notify_one());
        // Reviews kept from before a restart are looked at as usual.
        let mut following = true;
        loop {
            if !following {
                wrote.notified().await;
            }
            tokio::time::sleep(cadence.open).await;
            // This look sees every write before it, so their wake is taken
            // back; one that lands during the look brings another.
            tokio::select! {
                biased;
                () = wrote.notified() => {}
                () = std::future::ready(()) => {}
            }
            following = match self.poll_once(&cadence, &mut last).await {
                Ok(following) => following,
                Err(message) => {
                    crate::log::say!("humatad2: following the reviews' pull requests: {message}");
                    // Looked at again a cadence later.
                    true
                }
            };
        }
    }

    /// Whether the app has reported this review's view: it is open there.
    fn review_open(&self, session: &str) -> bool {
        let sessions = crate::review::lock(&self.inner.review.inner.sessions);
        sessions
            .get(session)
            .is_some_and(|review| crate::review::lock(&review.chat).reported)
    }

    /// One look at every review that is due. Says whether any review has
    /// an open pull request to follow.
    async fn poll_once(&self, cadence: &Cadence, last: &mut HashMap<String, Instant>) -> Result<bool, String> {
        let store = self.inner.store.clone();
        let reviews = blocking(move || store.review_sessions())
            .await
            .map_err(|status| status.message().to_owned())?
            .map_err(|error| format!("couldn't read the reviews: {error}"))?;
        let now = Instant::now();
        let mut due = Vec::new();
        let mut following = false;
        last.retain(|session, _| reviews.iter().any(|review| &review.session == session));
        for mut review in reviews {
            // A merged or closed pull request has no heads to follow.
            review.prs.retain(|pr| {
                pr.state == pb::PullRequestState::Open as i32 || pr.state == pb::PullRequestState::Unspecified as i32
            });
            if review.prs.is_empty() {
                continue;
            }
            following = true;
            let every = if self.review_open(&review.session) {
                cadence.open
            } else {
                cadence.idle
            };
            let waited = last.get(&review.session).map(|at| now.duration_since(*at));
            if waited.is_some_and(|waited| waited < every) {
                continue;
            }
            last.insert(review.session.clone(), now);
            due.push(review);
        }
        if due.is_empty() {
            return Ok(following);
        }

        let store = self.inner.store.clone();
        let looked = blocking(move || -> Result<Vec<Looked>, HeadsError> {
            let asked: Vec<&ReviewPr> = due.iter().flat_map(|review| review.prs.iter()).collect();
            let remotes = remotes_blocking(&asked)?;
            let mut looked = Vec::new();
            for review in &due {
                let Ok(Some(session)) = store.session(&review.session) else {
                    continue;
                };
                looked.push(Looked {
                    session: review.session.clone(),
                    channel: session.channel.clone(),
                    found: found_blocking(&session.folder, &review.prs, &remotes, true),
                });
            }
            Ok(looked)
        })
        .await
        .map_err(|status| status.message().to_owned())?
        .map_err(|error| crate::status::chain(&error))?;

        for review in looked {
            match review.found {
                Ok(found) => self.tell_heads(&review.session, &review.channel, found).await,
                Err(error) => crate::log::say!(
                    "humatad2: session {}: couldn't follow its pull requests: {}",
                    review.session,
                    crate::status::chain(&error)
                ),
            }
        }
        Ok(following)
    }

    /// Keeps what the poll saw and, if any pull request has news, says so
    /// on the feed.
    async fn tell_heads(&self, session: &str, channel: &str, found: Vec<Found>) {
        let changes: Vec<pb::HeadChange> = changes_where(&found, Found::news);
        let (store, id) = (self.inner.store.clone(), session.to_owned());
        match blocking(move || record_checked(&store, &id, &found)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                crate::log::say!("humatad2: session {session}: couldn't keep what the poll saw: {error}");
                // Not kept, so the next poll would say it again.
                return;
            }
            Err(status) => {
                crate::log::say!("humatad2: session {session}: {}", status.message());
                return;
            }
        }
        self.publish_heads(session, channel, changes);
    }

    /// Puts `changes` of a review's pull requests on the feed, if any.
    pub(super) fn publish_heads(
        &self,
        session: &str,
        channel: &str,
        changes: Vec<pb::HeadChange>,
    ) {
        if changes.is_empty() {
            return;
        }
        self.inner.feed.publish_in(
            pb::Change::Updated,
            pb::event::Payload::PullRequestHeads(pb::PullRequestHeadsChanged {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                changes,
            }),
            Scope {
                channel: channel.to_owned(),
                session: session.to_owned(),
            },
            None,
        );
    }
}

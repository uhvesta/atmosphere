//! The repo library hears of every repo whose add worked (RL-1, RL-2): a
//! channel created with repos, a repo added to a channel (the user's or an
//! approved agent proposal), a repo added to a Desk session, and the repos a
//! Desk session starts with. A name the user typed is not GitHub's own
//! spelling of it, so it is recorded as typed (`canonical` false). A write
//! the library can't make is logged and the add goes on: the library is a
//! convenience, never a reason for an add to fail.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use humata2_repo_library::{Library, Place, RepoName, Use};
use humata2_sessions::{Starter, remote_url};
use humata2_store::{ChannelRecord, MainClone};

use super::ChannelSvc;
use crate::status;

/// The host's library, set once after the services are made.
pub(super) type Slot = Arc<OnceLock<Library>>;

impl ChannelSvc {
    /// Hands the service the host's repo library. Once.
    pub(crate) fn use_repo_library(&self, library: Library) {
        let _ = self.library.set(library);
    }

    /// Records that `owner/name` was added in `place`. Blocking.
    pub(crate) fn record_added(&self, owner: &str, name: &str, place: &Place) {
        record(&self.library, &self.starter.remote_template, owner, name, place);
    }
}

/// The repos of `channel` whose main clone is ready now, by owner and name.
pub(super) fn ready_keys(channel: &ChannelRecord) -> HashSet<(String, String)> {
    channel
        .repos
        .iter()
        .filter(|repo| repo.main == MainClone::Ready)
        .map(|repo| (repo.owner.clone(), repo.name.clone()))
        .collect()
}

/// Records each repo of the channel whose main clone is ready now and was
/// not in `was_ready`: the ones this add made. A clone that failed, or that
/// another machine's setup is still doing, adds nothing. Blocking.
pub(super) fn record_new_ready(
    library: &Slot,
    starter: &Starter,
    channel: &str,
    was_ready: &HashSet<(String, String)>,
) {
    if library.get().is_none() {
        return;
    }
    let now = match starter.store.channel(channel) {
        Ok(Some(now)) => now,
        Ok(None) => return,
        Err(error) => {
            crate::log::say!(
                "humatad2: couldn't read channel {channel} to record its new repos in the repo library: {}",
                status::chain(&error)
            );
            return;
        }
    };
    let place = Place::Channel(channel.to_owned());
    for (owner, name) in ready_keys(&now).difference(was_ready) {
        record(library, &starter.remote_template, owner, name, &place);
    }
}

fn record(library: &Slot, template: &str, owner: &str, name: &str, place: &Place) {
    let Some(library) = library.get() else {
        return;
    };
    let repo = match RepoName::parse(&format!("{owner}/{name}")) {
        Ok(repo) => repo,
        Err(error) => {
            crate::log::say!(
                "humatad2: {owner}/{name} was added but isn't a name the repo library keeps, so it isn't listed there: {}",
                status::chain(&error)
            );
            return;
        }
    };
    let usage = Use {
        repo,
        remote: remote_url(template, owner, name).unwrap_or_default(),
        default_branch: String::new(),
        at: crate::desk_service::now_ms(),
        place: place.clone(),
        canonical: false,
    };
    if let Err(error) = library.record(&usage) {
        crate::log::say!(
            "humatad2: {owner}/{name} was added but couldn't be recorded in the repo library: {}",
            status::chain(&error)
        );
    }
}

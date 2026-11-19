//! Which branch prefix applies (FC-12, ST-13, D196): the channel's own if it
//! has one, even an empty one, else the one saved in Settings, else the
//! default. The default is the host's `gh` login and "/", kept in memory
//! only: until it is known the default is no prefix, and nothing waits for it.

use std::sync::RwLock;

use humata2_store::{ChannelRecord, Store, StoreError};

/// The settings key the global prefix is saved under.
pub const BRANCH_PREFIX_KEY: &str = "branch_prefix";

/// The default prefix the daemon learned from `gh`, empty until it has.
static DEFAULT: RwLock<String> = RwLock::new(String::new());

/// The default prefix: the host's `gh` login and "/", or "" before it is known.
pub fn default_prefix() -> String {
    DEFAULT.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Remembers `prefix` as the default; true when that changed it.
pub fn set_default_prefix(prefix: &str) -> bool {
    let mut held = DEFAULT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *held == prefix {
        return false;
    }
    prefix.clone_into(&mut held);
    true
}

/// The prefix new branch names in `channel` begin with.
pub fn effective_prefix(store: &Store, channel: &ChannelRecord) -> Result<String, StoreError> {
    if let Some(own) = &channel.branch_prefix {
        return Ok(own.clone());
    }
    Ok(store.setting(BRANCH_PREFIX_KEY)?.map_or_else(default_prefix, |saved| saved.value))
}

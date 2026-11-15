//! Where the person was, saved so a restart puts them back (PER-25): the
//! screen, the open sessions with their tabs, panels, drafts and queues,
//! pane sizes, folding, settings, unsent review comments and which hosts
//! were connected. Scroll positions and other fine-grained view state are
//! left out on purpose (D52).
//!
//! Structural changes save at once; drafts save at most once a second
//! while they change.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_clients::Storage;
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_palette::Command;
use humata2_review::ReviewSaved;
use humata2_session::SessionView;
use humata2_settings::Preferences;
use humata2_sidebar::Folding;
use humata2_views::Decisions;
use serde::{Deserialize, Serialize};

use crate::Route;
use crate::layout::{FullScreen, Layout};

/// The storage key the app's state is saved under.
pub const KEY: &str = "humata2-ui.json";

/// What is saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedUi {
    /// The screen showing.
    pub route: Route,
    /// Screens Back returns to, most recent last.
    pub trail: Vec<Route>,
    /// Each session's page.
    pub sessions: BTreeMap<SessionId, SessionView>,
    /// Pane sizes.
    pub layout: Layout,
    /// Full-window presentation saved per session.
    pub full_screens: BTreeMap<SessionId, FullScreen>,
    /// Each channel's selected tab, filters and folds.
    pub channels: BTreeMap<ChannelId, crate::channel::View>,
    /// The Board representation and filters.
    pub board: humata2_board::View,
    /// What the sidebar folded.
    pub folding: Folding,
    /// Settings.
    pub preferences: Preferences,
    /// Review folds and unsent comments.
    pub review: ReviewSaved,
    /// Recent palette picks.
    pub recent: Vec<Command>,
    /// Prepared views decided on.
    pub views: Decisions,
    /// The selected machine in Hosts.
    pub selected_host: Option<MachineId>,
    /// Hosts that answered.
    pub connected: BTreeSet<MachineId>,
}

impl SavedUi {
    /// The same state without drafts, to tell a structural change from
    /// typing.
    #[must_use]
    pub fn structure(&self) -> Self {
        let mut structure = self.clone();
        for view in structure.sessions.values_mut() {
            view.draft.clear();
            view.file_filter.clear();
        }
        structure.review.drafts.clear();
        structure
    }
}

/// Why saved state couldn't be read back.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// Storage refused.
    #[error("couldn't read where you were from the last run: {source}. Humata starts on Home.")]
    Load {
        /// Why.
        #[source]
        source: tca::StorageError,
    },
    /// What was saved isn't readable, such as after a downgrade.
    #[error(
        "the saved window state from the last run couldn't be read ({source}), so Humata starts on Home. It is replaced the next time anything changes."
    )]
    Decode {
        /// Why.
        #[source]
        source: serde_json::Error,
    },
}

/// Why state couldn't be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// Storage refused.
    #[error(
        "couldn't save where you are: {source}. If Humata restarts now it may open where you were a moment ago."
    )]
    Save {
        /// Why.
        #[source]
        source: tca::StorageError,
    },
    /// It couldn't be written as JSON.
    #[error("couldn't write where you are as JSON: {source}. This is a bug; please report it.")]
    Encode {
        /// Why.
        #[source]
        source: serde_json::Error,
    },
}

/// Reads what the last run saved; nothing saved is `Ok(None)`.
///
/// # Errors
///
/// [`RestoreError`] when storage fails or what was saved can't be read.
pub fn restore(storage: &dyn Storage) -> Result<Option<SavedUi>, RestoreError> {
    let Some(bytes) = storage
        .load(KEY)
        .map_err(|source| RestoreError::Load { source })?
    else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| RestoreError::Decode { source })
}

/// Writes saves in order: a slow older save never replaces a newer one.
#[derive(Clone)]
pub struct Saver {
    storage: Arc<dyn Storage>,
    written: Arc<Mutex<u64>>,
}

impl Saver {
    /// A saver into `storage`.
    #[must_use]
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            written: Arc::new(Mutex::new(0)),
        }
    }

    /// Saves `saved` as save number `generation`, unless a newer one was
    /// written already.
    ///
    /// # Errors
    ///
    /// [`SaveError`] when it can't be encoded or stored.
    pub fn save(&self, generation: u64, saved: &SavedUi) -> Result<(), SaveError> {
        let bytes = serde_json::to_vec(saved).map_err(|source| SaveError::Encode { source })?;
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if *written > generation {
            return Ok(());
        }
        self.storage
            .save(KEY, &bytes)
            .map_err(|source| SaveError::Save { source })?;
        *written = generation;
        Ok(())
    }
}

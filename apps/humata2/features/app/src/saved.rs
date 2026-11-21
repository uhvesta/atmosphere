//! Where the person was, saved so a restart puts them back (PER-25): the
//! screen, the open sessions with their tabs, panels, drafts and queues,
//! pane sizes, folding, settings, which sessions show resolved review comments and which hosts
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
use humata2_review::comments::CommentsSaved;
use humata2_session::SessionView;
use humata2_settings::Preferences;
use humata2_sidebar::Folding;
use serde::{Deserialize, Serialize};

use crate::Route;
use crate::layout::{FullScreen, Layout};

/// The storage key the app's state is saved under.
pub const KEY: &str = "humata2-ui.json";

/// The storage key per-channel Desk To-do split pane state is saved under (DESKTODO).
pub const DESK_TODO_PANES_KEY: &str = "humata2-desk-todo-panes.json";

/// Persisted visibility and width for a channel's Desk To-do split pane (DESKTODO).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedChannelTodoPane {
    /// Whether the split pane is shown.
    pub visible: bool,
    /// Clamped width in pixels.
    pub width: f32,
}

impl Default for SavedChannelTodoPane {
    fn default() -> Self {
        Self {
            visible: false,
            width: humata2_desk::TODO_PANE_DEFAULT_WIDTH,
        }
    }
}

/// Captures per-channel Desk To-do split pane state for persistence, clamping widths.
#[must_use]
pub fn capture_desk_todo_panes(
    panes: &BTreeMap<ChannelId, humata2_desk::ChannelTodoPane>,
) -> BTreeMap<ChannelId, SavedChannelTodoPane> {
    panes
        .iter()
        .map(|(channel, pane)| {
            (
                channel.clone(),
                SavedChannelTodoPane {
                    visible: pane.visible,
                    width: humata2_desk::clamp_todo_pane_width(pane.width),
                },
            )
        })
        .collect()
}

/// Builds the Desk actions to restore saved per-channel To-do split pane states.
#[must_use]
pub fn restore_desk_todo_pane_actions(
    saved: &BTreeMap<ChannelId, SavedChannelTodoPane>,
) -> Vec<humata2_desk::DeskAction> {
    saved
        .iter()
        .map(|(channel, pane)| humata2_desk::DeskAction::RestoreChannelTodoPane {
            channel: channel.clone(),
            visible: pane.visible,
            width: humata2_desk::clamp_todo_pane_width(pane.width),
        })
        .collect()
}

/// Saves per-channel Desk To-do split pane state to `storage`.
///
/// # Errors
///
/// [`SaveError`] when it can't be encoded or stored.
pub fn save_desk_todo_panes(
    storage: &dyn Storage,
    panes: &BTreeMap<ChannelId, humata2_desk::ChannelTodoPane>,
) -> Result<(), SaveError> {
    let captured = capture_desk_todo_panes(panes);
    let bytes = serde_json::to_vec(&captured).map_err(|source| SaveError::Encode { source })?;
    storage
        .save(DESK_TODO_PANES_KEY, &bytes)
        .map_err(|source| SaveError::Save { source })
}

/// Reads saved per-channel Desk To-do split pane state from `storage`; nothing
/// saved returns an empty map.
///
/// # Errors
///
/// [`RestoreError`] when storage fails or what was saved can't be read.
pub fn restore_desk_todo_panes(
    storage: &dyn Storage,
) -> Result<BTreeMap<ChannelId, SavedChannelTodoPane>, RestoreError> {
    let Some(bytes) = storage
        .load(DESK_TODO_PANES_KEY)
        .map_err(|source| RestoreError::Load { source })?
    else {
        return Ok(BTreeMap::new());
    };
    let mut panes: BTreeMap<ChannelId, SavedChannelTodoPane> =
        serde_json::from_slice(&bytes).map_err(|source| RestoreError::Decode { source })?;
    for pane in panes.values_mut() {
        pane.width = humata2_desk::clamp_todo_pane_width(pane.width);
    }
    Ok(panes)
}

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
    /// What the sidebar folded.
    pub folding: Folding,
    /// Settings.
    pub preferences: Preferences,
    /// Review folds, and comments older builds saved until the daemon takes them.
    pub review: ReviewSaved,
    /// Which sessions show resolved comments.
    pub comments: CommentsSaved,
    /// Recent palette picks. A pick this build no longer offers, such as
    /// an earlier build's Go to Board (D874), is left out.
    #[serde(deserialize_with = "known_commands")]
    pub recent: Vec<Command>,
    /// The selected machine in Hosts.
    pub selected_host: Option<MachineId>,
    /// Hosts that answered.
    pub connected: BTreeSet<MachineId>,
    /// How the Desk's Open list is sorted.
    pub desk_sort: humata2_desk::Sort,
}

/// The saved picks this build knows, in their order.
fn known_commands<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Command>, D::Error> {
    let saved = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(saved.into_iter().filter_map(|pick| serde_json::from_value(pick).ok()).collect())
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
        structure.review.legacy_drafts.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tca::StorageError;

    #[derive(Default)]
    struct MemoryStorage {
        entries: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl Storage for MemoryStorage {
        fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
            Ok(self
                .entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .cloned())
        }

        fn save(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
            self.entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_owned(), bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn desk_todo_panes_round_trip_and_clamp() {
        let storage = MemoryStorage::default();
        let ch_a = ChannelId::new("ch-a");
        let ch_b = ChannelId::new("ch-b");

        let mut panes = BTreeMap::new();
        panes.insert(
            ch_a.clone(),
            humata2_desk::ChannelTodoPane {
                visible: true,
                width: 100.0,
                ..Default::default()
            },
        );
        panes.insert(
            ch_b.clone(),
            humata2_desk::ChannelTodoPane {
                visible: false,
                width: 900.0,
                ..Default::default()
            },
        );

        save_desk_todo_panes(&storage, &panes).expect("save succeeds");
        let restored = restore_desk_todo_panes(&storage).expect("restore succeeds");

        assert_eq!(
            restored.get(&ch_a),
            Some(&SavedChannelTodoPane {
                visible: true,
                width: humata2_desk::TODO_PANE_MIN_WIDTH,
            })
        );
        assert_eq!(
            restored.get(&ch_b),
            Some(&SavedChannelTodoPane {
                visible: false,
                width: humata2_desk::TODO_PANE_MAX_WIDTH,
            })
        );

        let actions = restore_desk_todo_pane_actions(&restored);
        assert_eq!(
            actions,
            vec![
                humata2_desk::DeskAction::RestoreChannelTodoPane {
                    channel: ch_a,
                    visible: true,
                    width: humata2_desk::TODO_PANE_MIN_WIDTH,
                },
                humata2_desk::DeskAction::RestoreChannelTodoPane {
                    channel: ch_b,
                    visible: false,
                    width: humata2_desk::TODO_PANE_MAX_WIDTH,
                },
            ]
        );
    }

    #[test]
    fn restore_desk_todo_panes_empty_when_missing() {
        let storage = MemoryStorage::default();
        let restored = restore_desk_todo_panes(&storage).expect("empty restore succeeds");
        assert!(restored.is_empty());
    }
}

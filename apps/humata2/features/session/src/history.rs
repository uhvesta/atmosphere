//! The History strip of a Markdown tab in a Desk folder (DV-3 to DV-8): a
//! file's versions, which one the tab shows, and Save version.
//!
//! Only a file of a Desk session whose folder keeps versions has a history
//! ([`SessionState::history`]); any other tab has none. A history is made
//! when the file's text is first loaded, kept up to date when the file or
//! the session's `doc_head` changes, and read through
//! [`humata2_clients::DeskClient`].

use humata2_clients::DaemonError;
use humata2_clients::docs::{DocVersion, DocVersions, Restored, VersionDiff, VersionText};
use humata2_clients::model::{DiffLine, SessionId};
use tca::Effect;

use crate::document::is_versioned_doc;
use crate::{Panel, SessionAction, SessionState, Sessions};

/// Which text a tab shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Position {
    /// The file as it is now, unsaved changes included.
    #[default]
    Current,
    /// This version (a [`DocVersion::id`]), read-only (DV-6).
    Version(String),
}

/// One Markdown file's versions and where its tab is in them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocHistory {
    /// The file's versions, newest first. Empty until the first answer, and
    /// for a file with none ("No versions yet").
    pub versions: Vec<DocVersion>,
    /// The folder has changes no version holds (DV-3).
    pub unsaved: bool,
    /// What the tab shows.
    pub position: Position,
    /// The text and recorded comments of the version at `position`, once
    /// they are read. `None` while reading, and at [`Position::Current`].
    pub viewed: Option<VersionText>,
    /// What is typed in the Save version box (DV-5).
    pub message: String,
    /// The first answer came.
    pub loaded: bool,
    /// A save or a restore is on its way.
    pub working: bool,
    /// The session's `doc_head` when the versions were last asked for.
    head: String,
}

impl DocHistory {
    /// Whether the tab shows an old version: nothing can be edited,
    /// commented, sent, resolved, reopened or archived there (DV-6).
    #[must_use]
    pub const fn read_only(&self) -> bool {
        matches!(self.position, Position::Version(_))
    }

    /// The version the tab shows, when it shows an old one.
    #[must_use]
    pub fn viewing(&self) -> Option<&DocVersion> {
        match &self.position {
            Position::Current => None,
            Position::Version(id) => self.versions.iter().find(|version| &version.id == id),
        }
    }

    /// The number of the newest version, if there is one.
    #[must_use]
    pub fn newest(&self) -> Option<u32> {
        self.versions.first().map(|version| version.number)
    }

    /// Whether Save version is enabled: the folder has changes, and the tab
    /// shows the current file (DV-5).
    #[must_use]
    pub const fn can_save(&self) -> bool {
        self.loaded && self.unsaved && !self.working && !self.read_only()
    }

    /// Whether ◀ has an older version to go to.
    #[must_use]
    pub fn can_back(&self) -> bool {
        match self.index() {
            None => !self.versions.is_empty(),
            Some(at) => at + 1 < self.versions.len(),
        }
    }

    /// Whether ▶ has a newer version, or the current file, to go to.
    #[must_use]
    pub const fn can_forward(&self) -> bool {
        self.read_only()
    }

    /// Where the shown version is in `versions`.
    fn index(&self) -> Option<usize> {
        match &self.position {
            Position::Current => None,
            Position::Version(id) => self.versions.iter().position(|version| &version.id == id),
        }
    }
}

/// The old version the person asked to compare with the current file
/// (DV-7). The diff view reads it as its base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocCompare {
    /// The Desk session.
    pub session: SessionId,
    /// The file, relative to the session's folder.
    pub path: String,
    /// The version, a [`DocVersion::id`].
    pub version: String,
    /// The daemon's diff of that version against the file now.
    pub diff: CompareDiff,
}

/// Where the daemon's diff for a comparison is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CompareDiff {
    /// Asked for, not answered yet.
    #[default]
    Reading,
    /// The diff: hunk headers and lines, none when the file reads as the
    /// version does.
    Loaded(Vec<DiffLine>),
    /// The daemon refused or couldn't answer: what to tell the person.
    Failed(String),
}

/// What the History strip and the version bar do. Each acts on the Markdown
/// file `path` of the open session; answers carry their session.
#[derive(Clone, Debug, PartialEq)]
pub enum HistoryAction {
    /// Ask for the file's versions. Sent by the page as the file loads, and
    /// by the reducer when it needs them again.
    Load { session: SessionId, path: String },
    /// The daemon listed the versions, or didn't.
    Loaded {
        session: SessionId,
        path: String,
        result: Result<DocVersions, DaemonError>,
    },
    /// ◀: the next older version.
    Back { path: String },
    /// ▶: the next newer version, or the current file after the newest.
    Forward { path: String },
    /// A version was picked in History ▾.
    Pick { path: String, id: String },
    /// Back to current.
    BackToCurrent { path: String },
    /// A version's text came back.
    VersionRead {
        session: SessionId,
        path: String,
        id: String,
        result: Result<VersionText, DaemonError>,
    },
    /// The Save version box changed.
    Message { path: String, text: String },
    /// Save version, with the box's message.
    SaveVersion { path: String },
    /// The daemon saved a version, or didn't.
    Saved {
        session: SessionId,
        path: String,
        result: Result<DocVersion, DaemonError>,
    },
    /// Restore as new version: the shown version becomes the newest.
    Restore { path: String },
    /// The daemon restored, or didn't.
    Restored {
        session: SessionId,
        path: String,
        result: Result<Restored, DaemonError>,
    },
    /// Compare with current: open the diff view with the shown version as
    /// its base (DV-7).
    CompareWithCurrent { path: String },
    /// The daemon diffed the version against the file, or didn't.
    CompareLoaded {
        session: SessionId,
        path: String,
        version: String,
        result: Result<VersionDiff, DaemonError>,
    },
}

/// Whether `repo`/`path` of `session` can have a history: a versioned document
/// (Markdown or intent spec) in the Desk folder itself (no repository), and
/// the folder keeps versions.
fn eligible(state: &SessionState, session: &SessionId, repo: &str, path: &str) -> bool {
    repo.is_empty()
        && is_versioned_doc(path)
        && state
            .workspace
            .desk
            .sessions
            .get(session)
            .is_some_and(|record| record.has_versions)
}

fn head_of(state: &SessionState, session: &SessionId) -> String {
    state
        .workspace
        .desk
        .sessions
        .get(session)
        .map(|record| record.doc_head.clone())
        .unwrap_or_default()
}

impl SessionState {
    /// The history of a Markdown file of a Desk session; `None` for a file
    /// that has none (DV-3).
    #[must_use]
    pub fn history(&self, session: &SessionId, path: &str) -> Option<&DocHistory> {
        self.histories.get(&(session.clone(), path.to_owned()))
    }

    /// Whether the tab of this file shows an old version, so nothing can be
    /// edited, commented, sent, resolved or archived on it (DV-6).
    #[must_use]
    pub fn doc_read_only(&self, session: &SessionId, path: &str) -> bool {
        self.history(session, path).is_some_and(DocHistory::read_only)
    }
}

impl Sessions {
    /// Starts reading the versions of `path`, making its history if it has
    /// none yet. Nothing for a file that can't have one.
    fn load_history(
        &self,
        state: &mut SessionState,
        session: SessionId,
        path: String,
    ) -> Effect<SessionAction> {
        let key = (session.clone(), path.clone());
        if !eligible(state, &session, "", &path) {
            state.histories.remove(&key);
            return Effect::none();
        }
        let head = head_of(state, &session);
        state.histories.entry(key).or_default().head = head;
        let desk = self.clients.desk.clone();
        Effect::future(async move {
            let result = desk.list_versions(&session, &path).await;
            SessionAction::History(HistoryAction::Loaded {
                session,
                path,
                result,
            })
        })
    }

    /// A Markdown file's text was read: its history is made the first time,
    /// and read again when the text changed (unsaved changes may differ).
    pub(super) fn file_loaded_history(
        &self,
        state: &mut SessionState,
        session: &SessionId,
        repo: &str,
        path: &str,
        changed: bool,
    ) -> Effect<SessionAction> {
        if !eligible(state, session, repo, path) {
            return Effect::none();
        }
        let key = (session.clone(), path.to_owned());
        if changed || !state.histories.contains_key(&key) {
            return self.load_history(state, session.clone(), path.to_owned());
        }
        Effect::none()
    }

    /// The workspace changed: a history whose session's `doc_head` moved
    /// since its versions were read is read again (DV-5), and one whose
    /// folder no longer keeps versions goes.
    pub(super) fn sync_histories(&self, state: &mut SessionState) -> Vec<Effect<SessionAction>> {
        let keys: Vec<_> = state.histories.keys().cloned().collect();
        let mut effects = Vec::new();
        for (session, path) in keys {
            let moved = eligible(state, &session, "", &path)
                && state
                    .histories
                    .get(&(session.clone(), path.clone()))
                    .is_some_and(|history| history.head != head_of(state, &session));
            if moved || !eligible(state, &session, "", &path) {
                effects.push(self.load_history(state, session, path));
            }
        }
        effects
    }

    /// Shows version `id` of `path`: reads its text.
    fn show_version(
        &self,
        state: &mut SessionState,
        session: SessionId,
        path: String,
        id: String,
    ) -> Effect<SessionAction> {
        let Some(history) = state.histories.get_mut(&(session.clone(), path.clone())) else {
            return Effect::none();
        };
        if history.working || !history.versions.iter().any(|version| version.id == id) {
            return Effect::none();
        }
        history.position = Position::Version(id.clone());
        history.viewed = None;
        let desk = self.clients.desk.clone();
        Effect::future(async move {
            let result = desk.read_version(&session, &path, &id).await;
            SessionAction::History(HistoryAction::VersionRead {
                session,
                path,
                id,
                result,
            })
        })
    }

    /// The History strip's and the version bar's actions.
    #[allow(clippy::too_many_lines)]
    pub(super) fn history(
        &self,
        state: &mut SessionState,
        action: HistoryAction,
    ) -> Effect<SessionAction> {
        // The open session's file, for what the person did in its tab.
        let key = |state: &SessionState, path: &String| {
            state.open.clone().map(|session| (session, path.clone()))
        };
        match action {
            HistoryAction::Load { session, path } => return self.load_history(state, session, path),
            HistoryAction::Loaded {
                session,
                path,
                result,
            } => match result {
                Ok(list) => {
                    if let Some(history) = state.histories.get_mut(&(session, path)) {
                        history.versions = list.versions;
                        history.unsaved = list.unsaved;
                        history.loaded = true;
                        // Versions are never removed, so a shown one stays.
                        if history.read_only() && history.viewing().is_none() {
                            history.position = Position::Current;
                            history.viewed = None;
                        }
                    }
                }
                Err(error) => state.fail(&error),
            },
            HistoryAction::Back { path } => {
                let Some(key) = key(state, &path) else {
                    return Effect::none();
                };
                let Some(history) = state.histories.get(&key) else {
                    return Effect::none();
                };
                let next = match history.index() {
                    None => history.versions.first(),
                    Some(at) => history.versions.get(at + 1),
                };
                if let Some(id) = next.map(|version| version.id.clone()) {
                    return self.show_version(state, key.0, path, id);
                }
            }
            HistoryAction::Forward { path } => {
                let Some(key) = key(state, &path) else {
                    return Effect::none();
                };
                let Some(history) = state.histories.get(&key) else {
                    return Effect::none();
                };
                match history.index() {
                    Some(0) => return self.history(state, HistoryAction::BackToCurrent { path }),
                    Some(at) => {
                        let id = history.versions[at - 1].id.clone();
                        return self.show_version(state, key.0, path, id);
                    }
                    None => {}
                }
            }
            HistoryAction::Pick { path, id } => {
                if let Some(key) = key(state, &path) {
                    return self.show_version(state, key.0, path, id);
                }
            }
            HistoryAction::BackToCurrent { path } => {
                if let Some(key) = key(state, &path)
                    && let Some(history) = state.histories.get_mut(&key)
                    && !history.working
                {
                    history.position = Position::Current;
                    history.viewed = None;
                    state.compare = None;
                }
            }
            HistoryAction::VersionRead {
                session,
                path,
                id,
                result,
            } => {
                let Some(history) = state.histories.get_mut(&(session, path)) else {
                    return Effect::none();
                };
                // An answer for a version the tab has moved off is dropped.
                if history.position != Position::Version(id) {
                    return Effect::none();
                }
                match result {
                    Ok(text) => history.viewed = Some(text),
                    Err(error) => {
                        history.position = Position::Current;
                        state.fail(&error);
                    }
                }
            }
            HistoryAction::Message { path, text } => {
                if let Some(key) = key(state, &path)
                    && let Some(history) = state.histories.get_mut(&key)
                    && !history.read_only()
                {
                    history.message = text;
                }
            }
            HistoryAction::SaveVersion { path } => {
                let Some((session, path)) = key(state, &path) else {
                    return Effect::none();
                };
                let Some(history) = state.histories.get_mut(&(session.clone(), path.clone())) else {
                    return Effect::none();
                };
                if !history.can_save() {
                    return Effect::none();
                }
                history.working = true;
                let message = history.message.clone();
                let desk = self.clients.desk.clone();
                return Effect::future(async move {
                    let result = desk.save_version(&session, &message).await;
                    SessionAction::History(HistoryAction::Saved {
                        session,
                        path,
                        result,
                    })
                });
            }
            HistoryAction::Saved {
                session,
                path,
                result,
            } => {
                if let Some(history) = state.histories.get_mut(&(session.clone(), path.clone())) {
                    history.working = false;
                    if result.is_ok() {
                        history.message.clear();
                    }
                }
                match result {
                    Ok(_) => return self.load_history(state, session, path),
                    Err(error) => state.fail(&error),
                }
            }
            HistoryAction::Restore { path } => {
                let Some((session, path)) = key(state, &path) else {
                    return Effect::none();
                };
                let Some(history) = state.histories.get_mut(&(session.clone(), path.clone())) else {
                    return Effect::none();
                };
                let Position::Version(id) = history.position.clone() else {
                    return Effect::none();
                };
                if history.working || history.viewed.is_none() {
                    return Effect::none();
                }
                history.working = true;
                let desk = self.clients.desk.clone();
                return Effect::future(async move {
                    let result = desk.restore_version(&session, &path, &id).await;
                    SessionAction::History(HistoryAction::Restored {
                        session,
                        path,
                        result,
                    })
                });
            }
            HistoryAction::Restored {
                session,
                path,
                result,
            } => {
                if let Some(history) = state.histories.get_mut(&(session.clone(), path.clone())) {
                    history.working = false;
                    if result.is_ok() {
                        history.position = Position::Current;
                        history.viewed = None;
                    }
                }
                match result {
                    Ok(_) => {
                        state.compare = None;
                        // The file's text changed: read it again, and its versions.
                        return Effect::merge(vec![
                            self.load_history(state, session.clone(), path.clone()),
                            self.read_file(session, String::new(), path),
                        ]);
                    }
                    Err(error) => state.fail(&error),
                }
            }
            HistoryAction::CompareWithCurrent { path } => {
                let Some((session, path)) = key(state, &path) else {
                    return Effect::none();
                };
                let shown = state
                    .histories
                    .get(&(session.clone(), path.clone()))
                    .and_then(DocHistory::viewing)
                    .map(|version| version.id.clone());
                if let Some(version) = shown {
                    state.compare = Some(DocCompare {
                        session: session.clone(),
                        path: path.clone(),
                        version: version.clone(),
                        diff: CompareDiff::Reading,
                    });
                    let desk = self.clients.desk.clone();
                    return Effect::merge(vec![
                        Effect::send(SessionAction::SetPanel(Some(Panel::Changes))),
                        Effect::future(async move {
                            let result = desk.diff_version(&session, &path, &version).await;
                            SessionAction::History(HistoryAction::CompareLoaded {
                                session,
                                path,
                                version,
                                result,
                            })
                        }),
                    ]);
                }
            }
            HistoryAction::CompareLoaded {
                session,
                path,
                version,
                result,
            } => {
                // An answer for a comparison the person has left is dropped.
                if let Some(compare) = state.compare.as_mut()
                    && compare.session == session
                    && compare.path == path
                    && compare.version == version
                {
                    compare.diff = match result {
                        Ok(diff) => CompareDiff::Loaded(diff.lines),
                        Err(error) => CompareDiff::Failed(error.to_string()),
                    };
                }
            }
        }
        Effect::none()
    }
}

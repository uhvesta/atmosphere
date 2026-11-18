//! Versions of a file in a Desk folder (DV-2 to DV-8): what the app asks the
//! daemon for through [`crate::DeskClient`] (`list_versions`, `read_version`,
//! `save_version`, `restore_version`) and what it gets back.

use crate::comments::Comment;
use crate::model::DiffLine;

/// Who made a version (DV-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionBy {
    /// The person: Save version, or a restore.
    You,
    /// The session's agent, with a plain `git commit`.
    Agent,
}

/// One version of one file: a commit that changed it (DV-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocVersion {
    /// The commit.
    pub id: String,
    /// 1 for the file's oldest version. 0 in the answer to
    /// [`crate::DeskClient::save_version`], which is per folder.
    pub number: u32,
    /// When it was made, in milliseconds since the Unix epoch.
    pub at_ms: i64,
    /// Whether the person or the agent made it.
    pub by: VersionBy,
    /// What to show for `by`: "You", or the agent's name such as
    /// "Claude Code".
    pub who: String,
    /// Its message.
    pub message: String,
    /// Lines of the file it added.
    pub added: u32,
    /// Lines of the file it removed.
    pub removed: u32,
    /// A comment record was kept with it (DV-12).
    pub has_comments: bool,
}

/// A file's versions and whether the folder has changes no version holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocVersions {
    /// Newest first.
    pub versions: Vec<DocVersion>,
    /// Something in the folder differs from its newest version, so Save
    /// version has something to save (DV-3, DV-5).
    pub unsaved: bool,
}

/// One version's text, read-only (DV-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionText {
    /// The version.
    pub version: DocVersion,
    /// The file as it was.
    pub text: String,
    /// The file's comments as they were. `None` when none were recorded
    /// with this version (DV-12): the page says so and shows none.
    pub comments: Option<Vec<Comment>>,
}

/// What a restore did (DV-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restored {
    /// The version that saved unsaved changes first, when there were any.
    pub saved_first: Option<DocVersion>,
    /// The new version holding the restored text.
    pub restored: DocVersion,
}

/// A version of a file against the file as it is now, computed by the daemon
/// (DV-7): the lines the Changes panel draws, each hunk led by a
/// [`crate::model::LineKind::Hunk`] line. Empty when the file reads as the
/// version does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionDiff {
    /// Hunk headers and lines, in order.
    pub lines: Vec<DiffLine>,
    /// Lines added since the version.
    pub added: u32,
    /// Lines removed since the version.
    pub removed: u32,
}

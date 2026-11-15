use thiserror::Error;

/// Why the find bar can't search.
#[derive(Debug, Error)]
pub enum FindError {
    #[error(
        "`{pattern}` is not a valid regular expression ({source}). Fix the pattern, or turn \
         off Regex to search for the text as typed."
    )]
    Pattern {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

/// Something the terminal engine refused while the pane drove it. The
/// session goes on; only the one action is lost.
#[derive(Debug, Error)]
pub enum PaneError {
    #[error("The key wasn't sent to the program: {source} Press it again.")]
    Key {
        #[source]
        source: ghostty_vt::Error,
    },

    #[error("The mouse event wasn't sent to the program: {source}")]
    Mouse {
        #[source]
        source: ghostty_vt::Error,
    },

    #[error("The paste wasn't sent to the program: {source} Paste again.")]
    Paste {
        #[source]
        source: ghostty_vt::Error,
    },

    #[error("The terminal couldn't scroll its history: {source}")]
    Scroll {
        #[source]
        source: ghostty_vt::Error,
    },

    #[error("The selection couldn't be made or read: {source} Select again.")]
    Select {
        #[source]
        source: ghostty_vt::Error,
    },

    #[error(
        "The terminal couldn't change to {cols}×{rows} cells: {source} It keeps its old size \
         until the pane is resized again."
    )]
    Resize {
        cols: u16,
        rows: u16,
        #[source]
        source: ghostty_vt::Error,
    },

    #[error("The program wasn't told the pane's focus changed: {source}")]
    Focus {
        #[source]
        source: ghostty_vt::Error,
    },
}

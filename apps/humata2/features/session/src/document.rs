//! A document tab's view settings (D166): Markdown files open as a page.
use crate::SessionView;
use serde::{Deserialize, Serialize};

/// How a document tab shows its file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentMode {
    /// The rendered page.
    #[default]
    Preview,
    /// The file's text.
    Source,
    /// Source and page next to each other.
    SideBySide,
}

/// One document's view settings in a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentView {
    /// How the file is shown.
    pub mode: DocumentMode,
    /// The page follows the source.
    pub linked: bool,
}

impl Default for DocumentView {
    fn default() -> Self {
        Self {
            mode: DocumentMode::Preview,
            linked: true,
        }
    }
}

/// One stored entry: JSON can't key a map by a pair, so a list holds them.
/// Its fields stay private; read and write through `SessionView::document`
/// and `SessionView::set_document`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentEntry {
    repo: String,
    path: String,
    view: DocumentView,
}

/// A Markdown file: `.md`, `.mdx`, or `.markdown`, any case.
pub fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".mdx") || lower.ends_with(".markdown")
}

impl SessionView {
    /// A file's document settings; the defaults when none were picked.
    pub fn document(&self, repo: &str, path: &str) -> DocumentView {
        self.documents
            .iter()
            .find(|entry| entry.repo == repo && entry.path == path)
            .map(|entry| entry.view)
            .unwrap_or_default()
    }

    /// Store a file's settings; the defaults remove the entry. The reducer
    /// uses it, and tests state what a pick leaves behind with it.
    pub fn set_document(&mut self, repo: &str, path: &str, view: DocumentView) {
        self.documents
            .retain(|entry| !(entry.repo == repo && entry.path == path));
        if view != DocumentView::default() {
            self.documents.push(DocumentEntry {
                repo: repo.to_owned(),
                path: path.to_owned(),
                view,
            });
        }
    }
}

//! A document tab's view settings (D166): Markdown and intent spec files open as a page.
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

/// An intent spec file: `intent.yaml` or `<name>.intent.yaml`, any case.
pub fn is_intent_spec(path: &str) -> bool {
    let filename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lower = filename.to_ascii_lowercase();
    lower == "intent.yaml"
        || (lower.ends_with(".intent.yaml") && lower.len() > ".intent.yaml".len())
}

/// A versioned document file: Markdown or an intent spec (`intent.yaml` / `*.intent.yaml`).
pub fn is_versioned_doc(path: &str) -> bool {
    is_markdown(path) || is_intent_spec(path)
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

#[cfg(test)]
mod tests {
    use super::{is_intent_spec, is_versioned_doc};

    #[test]
    fn intent_spec_matches_expected_filenames() {
        for path in [
            "intent.yaml",
            "docs/intent.yaml",
            "docs\\INTENT.YAML",
            "cursor.intent.yaml",
            "sub/PAY-14.INTENT.YAML",
        ] {
            assert!(is_intent_spec(path), "expected intent spec: {path}");
            assert!(is_versioned_doc(path), "expected versioned doc: {path}");
        }
    }

    #[test]
    fn versioned_doc_matches_markdown_and_rejects_non_versioned_files() {
        for path in ["a.md", "b.mdx", "c.markdown"] {
            assert!(!is_intent_spec(path), "not an intent spec: {path}");
            assert!(is_versioned_doc(path), "expected versioned doc: {path}");
        }
        for path in [
            "config.yaml",
            "intent.yml",
            "myintent.yaml",
            "intent.yaml.bak",
            ".intent.yaml",
            "docs/.intent.yaml",
            "a.rs",
            "",
        ] {
            assert!(!is_intent_spec(path), "unexpected intent spec: {path}");
            assert!(!is_versioned_doc(path), "unexpected versioned doc: {path}");
        }
    }
}

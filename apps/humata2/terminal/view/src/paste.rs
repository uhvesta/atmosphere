use std::path::PathBuf;

use gpui_kit::{ClipboardEntry, ClipboardItem, ImageFormat};

/// What a paste brings to the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paste {
    /// Text, to send to the program as typed (through the engine's paste
    /// encoding, which brackets it and checks it is safe).
    Text(String),
    /// Files, as copied in a file manager. The host decides whether to
    /// type their paths or hand the files to the program.
    Files(Vec<PathBuf>),
    /// An image itself, such as a screenshot copied to the clipboard. A
    /// terminal can't type an image, so the host decides what to do with
    /// it, such as saving it and typing its path or attaching it to an
    /// agent's message.
    Image { format: ImageFormat, bytes: Vec<u8> },
}

impl Paste {
    /// What pasting `item` brings, by the rule Humata 1 uses: files first,
    /// as their names are the only text with them; then an image, unless
    /// the clipboard also holds text other than blank text or a lone web
    /// address (which a browser puts beside a copied image); then text.
    /// `None` when there is nothing to paste.
    pub(crate) fn from_clipboard(item: &ClipboardItem) -> Option<Paste> {
        let mut text: Option<String> = None;
        let mut image = None;
        let mut files = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::String(string) => {
                    text.get_or_insert_with(String::new).push_str(&string.text);
                }
                ClipboardEntry::Image(found) => {
                    image.get_or_insert(found);
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    files.extend(paths.paths().iter().cloned());
                }
            }
        }
        if !files.is_empty() {
            return Some(Paste::Files(files));
        }
        let text_wins = text.as_deref().is_some_and(is_text_to_paste);
        match (image, text) {
            (Some(image), _) if !text_wins => Some(Paste::Image {
                format: image.format,
                bytes: image.bytes.clone(),
            }),
            (_, Some(text)) if !text.is_empty() => Some(Paste::Text(text)),
            _ => None,
        }
    }
}

/// Whether `text` is something to paste as text: anything but blank text
/// or a lone web address.
fn is_text_to_paste(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && !is_web_address(trimmed)
}

/// Whether `text`, already trimmed, is one http or https address.
fn is_web_address(text: &str) -> bool {
    if text.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|rest| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        !host.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{ClipboardString, ExternalPaths, Image};

    fn item(entries: Vec<ClipboardEntry>) -> ClipboardItem {
        ClipboardItem { entries }
    }

    fn png() -> ClipboardEntry {
        ClipboardEntry::Image(Image::from_bytes(ImageFormat::Png, vec![0x89, b'P', b'N', b'G']))
    }

    fn string(text: &str) -> ClipboardEntry {
        ClipboardEntry::String(ClipboardString::new(text.to_owned()))
    }

    #[::std::prelude::v1::test]
    fn text_pastes_as_text() {
        assert_eq!(
            Paste::from_clipboard(&ClipboardItem::new_string("ls -la".to_owned())),
            Some(Paste::Text("ls -la".to_owned()))
        );
    }

    #[::std::prelude::v1::test]
    fn an_image_pastes_as_the_image_not_its_bytes_as_text() {
        let pasted = Paste::from_clipboard(&item(vec![png()]));
        assert_eq!(
            pasted,
            Some(Paste::Image {
                format: ImageFormat::Png,
                bytes: vec![0x89, b'P', b'N', b'G'],
            })
        );
    }

    #[::std::prelude::v1::test]
    fn an_image_beside_its_web_address_or_blank_text_is_the_image() {
        for text in ["https://example.com/cat.png", "  \n", "HTTP://example.com"] {
            let pasted = Paste::from_clipboard(&item(vec![string(text), png()]));
            assert!(matches!(pasted, Some(Paste::Image { .. })), "{text:?}");
        }
    }

    #[::std::prelude::v1::test]
    fn real_text_beside_an_image_is_the_text() {
        for text in ["see https://example.com", "notes.txt", "https://"] {
            let pasted = Paste::from_clipboard(&item(vec![string(text), png()]));
            assert_eq!(pasted, Some(Paste::Text(text.to_owned())));
        }
    }

    #[::std::prelude::v1::test]
    fn files_come_first() {
        let paths = ExternalPaths(vec![PathBuf::from("/tmp/a b.png")].into());
        let pasted = Paste::from_clipboard(&item(vec![
            string("a b.png"),
            png(),
            ClipboardEntry::ExternalPaths(paths),
        ]));
        assert_eq!(pasted, Some(Paste::Files(vec![PathBuf::from("/tmp/a b.png")])));
    }

    #[::std::prelude::v1::test]
    fn an_empty_clipboard_pastes_nothing() {
        assert_eq!(Paste::from_clipboard(&item(Vec::new())), None);
        assert_eq!(Paste::from_clipboard(&item(vec![string("")])), None);
    }
}

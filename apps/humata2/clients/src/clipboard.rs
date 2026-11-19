//! The clipboard, for Copy actions the reducers take (copying a prompt or a
//! link). Pasting goes through the GUI, which reads the platform's clipboard.

/// Where copied text goes.
pub trait Clipboard: Send + Sync + 'static {
    /// Puts `text` on the clipboard.
    fn copy(&self, text: &str);
}

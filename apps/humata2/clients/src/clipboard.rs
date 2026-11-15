//! The clipboard, for Copy actions the reducers take (copying a prompt or a
//! link). Pasting goes through the GUI, which reads the platform's clipboard.

use std::sync::{Arc, Mutex};

/// Where copied text goes.
pub trait Clipboard: Send + Sync + 'static {
    /// Puts `text` on the clipboard.
    fn copy(&self, text: &str);
}

/// A clipboard in memory that remembers what was copied last.
#[derive(Clone, Debug, Default)]
pub struct FakeClipboard {
    copied: Arc<Mutex<Option<String>>>,
}

impl FakeClipboard {
    /// What was copied last.
    #[must_use]
    pub fn copied(&self) -> Option<String> {
        match self.copied.lock() {
            Ok(copied) => copied.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Clipboard for FakeClipboard {
    fn copy(&self, text: &str) {
        let mut copied = match self.copied.lock() {
            Ok(copied) => copied,
            Err(poisoned) => poisoned.into_inner(),
        };
        *copied = Some(text.to_owned());
    }
}

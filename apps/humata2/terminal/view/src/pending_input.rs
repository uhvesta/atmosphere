//! Ordered input behind asynchronous clipboard reads. No timer or platform access.
use crate::{InputBufferError, keys::KeyIntent, paste::Paste};
use gpui_kit::ClipboardReadError;
use std::collections::VecDeque;
use terminal_grid::KeyAction;

pub(super) const MAX_EVENTS: usize = 1024;
pub(super) const MAX_BYTES: usize = 1 << 20;
pub(super) enum Input {
    Key {
        intent: KeyIntent,
        action: KeyAction,
    },
    Text(String),
    Clipboard {
        id: u64,
        result: Option<Result<Option<Paste>, ClipboardReadError>>,
    },
}
impl Input {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Key {
                    intent: KeyIntent::Send { text, .. },
                    ..
                }
                | Self::Text(text) => text.len(),
                Self::Clipboard {
                    result: Some(Ok(Some(Paste::Text(text)))),
                    ..
                } => text.len(),
                Self::Clipboard {
                    result: Some(Ok(Some(Paste::Image { bytes, .. }))),
                    ..
                } => bytes.len(),
                Self::Clipboard {
                    result: Some(Ok(Some(Paste::Files(paths)))),
                    ..
                } => paths.iter().map(|p| p.as_os_str().len()).sum(),
                Self::Clipboard {
                    result: Some(Err(ClipboardReadError::Denied(message))),
                    ..
                } => message.len(),
                _ => 0,
            }
    }
}
#[derive(Default)]
pub(super) struct PendingInput {
    queue: VecDeque<Input>,
    bytes: usize,
    next_id: u64,
}
impl PendingInput {
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn queued_presses(&self) -> impl Iterator<Item = terminal_grid::Key> + '_ {
        self.queue.iter().filter_map(|input| match input {
            Input::Key {
                intent: KeyIntent::Send { key, .. },
                action: KeyAction::Press | KeyAction::Repeat,
            } => Some(*key),
            _ => None,
        })
    }
    pub fn clear(&mut self) {
        self.queue.clear();
        self.bytes = 0;
    }
    pub fn push(&mut self, input: Input) -> Result<(), InputBufferError> {
        let bytes = input.bytes();
        if self.queue.len() >= MAX_EVENTS || bytes > MAX_BYTES.saturating_sub(self.bytes) {
            return Err(InputBufferError::Capacity {
                events: self.queue.len().saturating_add(1),
                bytes: self.bytes.saturating_add(bytes),
                max_events: MAX_EVENTS,
                max_bytes: MAX_BYTES,
            });
        }
        self.bytes += bytes;
        self.queue.push_back(input);
        Ok(())
    }
    pub fn begin_clipboard(&mut self) -> Result<u64, InputBufferError> {
        let id = self
            .next_id
            .checked_add(1)
            .ok_or(InputBufferError::ReadSequenceExhausted {
                last_id: self.next_id,
            })?;
        self.push(Input::Clipboard { id, result: None })?;
        self.next_id = id;
        Ok(id)
    }
    pub fn complete(
        &mut self,
        id: u64,
        result: Result<Option<Paste>, ClipboardReadError>,
    ) -> Result<bool, InputBufferError> {
        let Some(slot) = self
            .queue
            .iter_mut()
            .find(|input| matches!(input,Input::Clipboard{id:other,..} if *other==id))
        else {
            return Ok(false);
        };
        let old = slot.bytes();
        let replacement = Input::Clipboard {
            id,
            result: Some(result),
        };
        let bytes = replacement.bytes();
        if bytes > MAX_BYTES.saturating_sub(self.bytes - old) {
            return Err(InputBufferError::Capacity {
                events: self.queue.len(),
                bytes: (self.bytes - old).saturating_add(bytes),
                max_events: MAX_EVENTS,
                max_bytes: MAX_BYTES,
            });
        }
        *slot = replacement;
        self.bytes = self.bytes - old + bytes;
        Ok(true)
    }
    pub fn pop_ready(&mut self) -> Option<Input> {
        if matches!(
            self.queue.front(),
            None | Some(Input::Clipboard { result: None, .. })
        ) {
            return None;
        }
        let input = self.queue.pop_front()?;
        self.bytes -= input.bytes();
        Some(input)
    }
}

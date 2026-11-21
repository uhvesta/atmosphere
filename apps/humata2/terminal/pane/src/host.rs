//! The bridge from the view to a session: what the user does, encoded by
//! `ghostty_vt` the way the program asked for it, written to the pty.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use ghostty_vt::{
    DecMode, Error as VtError, Key, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent,
    OptionAsAlt, Scroll, Size, Terminal, ViewportPoint,
};
use gpui_kit::ImageFormat;
use terminal_view::{CellSize, Paste, ScrollTo, TerminalHost};

use crate::error::PaneError;

/// Where a pane sends the program's input: the session's pty, local or
/// across the daemon's link. The pane never blocks on it.
pub trait Pty {
    /// Bytes for the program, in order: typed keys, pastes, mouse reports
    /// and the terminal's answers to the program's queries.
    fn write(&mut self, bytes: &[u8]);

    /// Local enqueue disposition, not a PTY-written acknowledgement. Rejecting
    /// adapters report their own failure through their existing error channel.
    fn write_accepted(&mut self, bytes: &[u8]) -> bool {
        self.write(bytes);
        true
    }

    /// Engine-generated responses to terminal queries. A host-authoritative
    /// replica ignores these; its remote engine already answers the PTY.
    fn reply(&mut self, bytes: &[u8]) {
        self.write(bytes);
    }

    /// A remote replica waits for an ordered host resize event instead of
    /// changing its grid before earlier output has been applied.
    fn defer_grid_resize(&self) -> bool {
        false
    }

    /// The pane now shows `size` cells of `cell` pixels; tell the program
    /// (`TIOCSWINSZ`, so it gets `SIGWINCH`).
    fn resize(&mut self, size: Size, cell: CellSize);
}

/// Something the pane leaves to the app.
#[derive(Debug, Clone)]
pub enum PaneEvent {
    /// The program set its title (OSC 0 or 2), for the tab.
    TitleChanged(String),
    /// The program's working directory (OSC 7).
    PwdChanged(String),
    Bell,
    /// Modifier-clicked terminal hyperlink; the host applies its URL policy.
    OpenLink(String),
    /// The program asked to set a clipboard (OSC 52). Remote programs'
    /// writes reach the Mac's clipboard (D30); empty `text` clears it.
    ClipboardWrite {
        target: ghostty_vt::ClipboardTarget,
        text: String,
    },
    /// An image was pasted. Upload it to the session's paste folder and
    /// paste its path with [`TerminalPane::paste_text`](crate::TerminalPane::paste_text)
    /// (D30, `TerminalService.PasteImage`).
    PasteImage {
        format: ImageFormat,
        bytes: Vec<u8>,
    },
    /// Files were pasted; the app decides whether to type their paths or
    /// upload them first, as for a remote session.
    PasteFiles(Vec<PathBuf>),
    /// Something the user did was lost; show it, the session goes on.
    Failed(PaneError),
    /// The user pressed ⌘\\ (Ctrl+Shift+\\ off the Mac): split this pane
    /// with a new shell beside it (NV-30). The pane tree does the split.
    SplitRequested,
    /// Something typed or pasted was dropped because the pane takes no
    /// input now ([`TerminalPane::set_connected`](crate::TerminalPane::set_connected)).
    /// Told once until it takes input again, so the app can say why
    /// without repeating itself.
    InputRefused,
}

/// A paste held back until the user agrees: without bracketed paste, a
/// line break runs what came before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPaste {
    pub text: String,
    pub line_breaks: usize,
}

/// What the host and the pane share.
pub(crate) struct Shared {
    pub(crate) pty: Box<dyn Pty>,
    /// Off while the session's machine is offline (SE-16): input is
    /// dropped, never queued to arrive out of context later.
    pub(crate) connected: bool,
    /// Whether the app was told input is dropped since `connected` went
    /// off.
    pub(crate) told_refused: bool,
    pub(crate) pending_paste: Option<PendingPaste>,
    /// A clipboard the program asked to read (OSC 52 `?`), waiting for
    /// the user: never answered without them (D30).
    pub(crate) clipboard_read: Option<ghostty_vt::ClipboardTarget>,
    pub(crate) events: Vec<PaneEvent>,
    pub(crate) cell: CellSize,
    paste_accepted: bool,
}

impl Shared {
    pub(crate) fn new(pty: Box<dyn Pty>) -> Rc<RefCell<Shared>> {
        Rc::new(RefCell::new(Shared {
            pty,
            connected: true,
            told_refused: false,
            pending_paste: None,
            clipboard_read: None,
            events: Vec::new(),
            cell: CellSize::default(),
            paste_accepted: false,
        }))
    }

    /// Input was dropped while the pane takes none: tells the app once.
    pub(crate) fn refuse(&mut self) {
        if !self.told_refused {
            self.told_refused = true;
            self.events.push(PaneEvent::InputRefused);
        }
    }

    pub(crate) fn send(&mut self, bytes: &[u8]) {
        if self.connected && !bytes.is_empty() {
            self.pty.write(bytes);
        }
    }

    /// Engine replies can be generated by resize as well as incoming output.
    pub(crate) fn engine_event(&mut self, event: ghostty_vt::Event) {
        use ghostty_vt::Event;
        let event = match event {
            Event::Reply(bytes) => {
                if self.connected && !bytes.is_empty() {
                    self.pty.reply(&bytes);
                }
                return;
            }
            Event::TitleChanged(title) => PaneEvent::TitleChanged(title),
            Event::PwdChanged(pwd) => PaneEvent::PwdChanged(pwd),
            Event::Bell => PaneEvent::Bell,
            Event::ClipboardWrite { target, text } => PaneEvent::ClipboardWrite { target, text },
            // The pane asks the user; a newer read replaces one waiting.
            Event::ClipboardReadRequested { target } => {
                self.clipboard_read = Some(target);
                return;
            }
        };
        self.events.push(event);
    }
    fn fail(&mut self, error: PaneError) {
        self.events.push(PaneEvent::Failed(error));
    }
    pub(crate) fn clipboard_read_failed(&mut self, source: gpui_kit::ClipboardReadError) {
        let error = match source {
            gpui_kit::ClipboardReadError::Unavailable => PaneError::ClipboardUnavailable { source },
            gpui_kit::ClipboardReadError::Denied(_) => PaneError::ClipboardDenied { source },
            gpui_kit::ClipboardReadError::UnsupportedContent => {
                PaneError::ClipboardUnsupported { source }
            }
        };
        self.fail(error);
    }
}

/// [`TerminalHost`] for a [`Terminal`]: keys, mouse and pastes in the
/// program's encodings; scrolling, selection and copy in the engine.
pub(crate) struct GhosttyHost {
    pub(crate) shared: Rc<RefCell<Shared>>,
}

/// Puts the viewport back on the newest output, as typing does in
/// Ghostty (`scroll-to-bottom = keystroke`).
fn follow_output(grid: &mut Terminal) -> Result<(), VtError> {
    match grid.scrollbar() {
        Some(bar) if !bar.at_bottom() => grid.scroll_viewport(Scroll::Bottom),
        _ => Ok(()),
    }
}

impl GhosttyHost {
    /// The bytes for one wheel step: a mouse report when the program
    /// takes the mouse, arrow keys on the alternate screen when it asked
    /// for those (mode 1007, as `less` and `vim` do), otherwise nothing.
    fn wheel_bytes(
        grid: &mut Terminal,
        up: bool,
        at: ViewportPoint,
        mods: Mods,
        cell: CellSize,
    ) -> Result<Option<Vec<u8>>, PaneError> {
        if grid.is_mouse_tracking() {
            let event = MouseEvent {
                action: MouseAction::Press,
                button: Some(if up {
                    MouseButton::Four
                } else {
                    MouseButton::Five
                }),
                mods,
                x: (f32::from(at.x) + 0.5) * cell.width,
                y: (f32::from(at.y) + 0.5) * cell.height,
            };
            return grid
                .encode_mouse(&event)
                .map(Some)
                .map_err(|source| PaneError::Mouse { source });
        }
        if grid.is_alternate_screen() && grid.is_mode_set(DecMode::AlternateScroll) {
            let key = if up { Key::ArrowUp } else { Key::ArrowDown };
            return grid
                .encode_key(&KeyEvent::press(key))
                .map(Some)
                .map_err(|source| PaneError::Key { source });
        }
        Ok(None)
    }
}

/// With Option acting as Alt on a Mac, the key's own character instead
/// of the one macOS typed (`ƒ` for Option+F), so the program gets ESC and
/// the letter. GPUI doesn't say which Option key is held, so the left-only
/// and right-only settings act as on.
fn option_as_alt_text(grid: &Terminal, event: &KeyEvent<'_>) -> Option<String> {
    // Not gated on the build's platform: a browser on a Mac sends ⌥ keys
    // too, and only a Mac turns the setting on.
    if grid.option_as_alt() == OptionAsAlt::Off
        || !event.mods.contains(Mods::ALT)
        || event.mods.contains(Mods::CTRL)
        || event.mods.contains(Mods::SUPER)
    {
        return None;
    }
    let c = event.unshifted?;
    Some(if event.mods.contains(Mods::SHIFT) {
        c.to_uppercase().collect()
    } else {
        c.to_string()
    })
}

impl TerminalHost<Terminal> for GhosttyHost {
    fn paste_is_pending(&self) -> bool {
        self.shared.borrow().pending_paste.is_some()
    }
    fn paste_input_accepted(&self) -> bool {
        self.shared.borrow().paste_accepted
    }
    fn input_buffer_failed(&mut self, source: terminal_view::InputBufferError) {
        self.shared
            .borrow_mut()
            .fail(PaneError::BufferedInput { source });
    }
    fn clipboard_read_failed(&mut self, source: gpui_kit::ClipboardReadError) {
        self.shared.borrow_mut().clipboard_read_failed(source);
    }

    fn open_link(&mut self, uri: String) {
        self.shared
            .borrow_mut()
            .events
            .push(PaneEvent::OpenLink(uri));
    }

    fn key(&mut self, grid: &mut Terminal, event: KeyEvent<'_>) {
        let mut shared = self.shared.borrow_mut();
        if !shared.connected {
            shared.refuse();
            return;
        }
        let alt_text = option_as_alt_text(grid, &event);
        let event = match &alt_text {
            Some(text) => KeyEvent {
                text,
                consumed_mods: Mods::NONE,
                ..event
            },
            None => event,
        };
        match grid.encode_key(&event) {
            Ok(bytes) if !bytes.is_empty() => {
                if let Err(source) = follow_output(grid) {
                    shared.fail(PaneError::Scroll { source });
                }
                shared.send(&bytes);
            }
            Ok(_) => {}
            Err(source) => shared.fail(PaneError::Key { source }),
        }
    }

    fn mouse(&mut self, grid: &mut Terminal, event: MouseEvent) -> bool {
        let mut shared = self.shared.borrow_mut();
        // With nobody listening, as after the program ended with mouse
        // reporting on, the mouse selects instead.
        if !grid.is_mouse_tracking() || !shared.connected {
            return false;
        }
        match grid.encode_mouse(&event) {
            Ok(bytes) => shared.send(&bytes),
            Err(source) => shared.fail(PaneError::Mouse { source }),
        }
        true
    }

    fn scroll(&mut self, grid: &mut Terminal, lines: i32, at: ViewportPoint, mods: Mods) {
        let mut shared = self.shared.borrow_mut();
        let cell = shared.cell;
        let up = lines > 0;
        let mut sent_to_program = false;
        // ⇧ keeps the wheel for the history from a program that takes the
        // mouse, as it keeps a drag for selecting; and with nobody
        // listening it is the history's too.
        let program_hears =
            shared.connected && !(mods.contains(Mods::SHIFT) && grid.is_mouse_tracking());
        for _ in 0..if program_hears { lines.unsigned_abs() } else { 0 } {
            match GhosttyHost::wheel_bytes(grid, up, at, mods, cell) {
                Ok(Some(bytes)) => {
                    sent_to_program = true;
                    shared.send(&bytes);
                }
                Ok(None) => break,
                Err(error) => {
                    shared.fail(error);
                    return;
                }
            }
        }
        if sent_to_program {
            return;
        }
        let delta = -(lines as isize);
        if let Err(source) = grid.scroll_viewport(Scroll::Delta(delta)) {
            shared.fail(PaneError::Scroll { source });
        }
    }

    fn select(&mut self, grid: &mut Terminal, selection: Option<(ViewportPoint, ViewportPoint)>) {
        let result = match selection {
            Some((start, end)) => grid.select(start, end),
            None => grid.clear_selection(),
        };
        if let Err(source) = result {
            self.shared.borrow_mut().fail(PaneError::Select { source });
        }
    }

    fn select_rectangle(&mut self, grid: &mut Terminal, start: ViewportPoint, end: ViewportPoint) {
        if let Err(source) = grid.select_rectangle(start, end) {
            self.shared.borrow_mut().fail(PaneError::Select { source });
        }
    }

    fn select_all(&mut self, grid: &mut Terminal) {
        if let Err(source) = grid.select_all() {
            self.shared.borrow_mut().fail(PaneError::Select { source });
        }
    }

    fn scroll_to(&mut self, grid: &mut Terminal, to: ScrollTo) {
        let page = isize::try_from(grid.size().rows.max(1)).unwrap_or(1);
        let scroll = match to {
            ScrollTo::Top => Scroll::Top,
            ScrollTo::Bottom => Scroll::Bottom,
            ScrollTo::PageUp => Scroll::Delta(-page),
            ScrollTo::PageDown => Scroll::Delta(page),
            ScrollTo::Row(row) => Scroll::Row(usize::try_from(row).unwrap_or(usize::MAX)),
        };
        if let Err(source) = grid.scroll_viewport(scroll) {
            self.shared.borrow_mut().fail(PaneError::Scroll { source });
        }
    }

    fn text(&mut self, grid: &mut Terminal, text: &str) {
        let mut shared = self.shared.borrow_mut();
        if !shared.connected {
            shared.refuse();
            return;
        }
        if let Err(source) = follow_output(grid) {
            shared.fail(PaneError::Scroll { source });
        }
        shared.send(text.as_bytes());
    }

    fn copy(&mut self, grid: &mut Terminal) -> Option<String> {
        match grid.selected_text() {
            Ok(text) => text.filter(|text| !text.is_empty()),
            Err(source) => {
                self.shared.borrow_mut().fail(PaneError::Select { source });
                None
            }
        }
    }

    fn paste(&mut self, grid: &mut Terminal, paste: Paste) {
        let mut shared = self.shared.borrow_mut();
        shared.paste_accepted = false;
        if !shared.connected {
            shared.refuse();
            return;
        }
        match paste {
            Paste::Text(text) => paste_text(&mut shared, grid, text, false),
            Paste::Image { format, bytes } => {
                shared.events.push(PaneEvent::PasteImage { format, bytes });
            }
            Paste::Files(paths) => shared.events.push(PaneEvent::PasteFiles(paths)),
        }
    }

    fn resize(&mut self, grid: &mut Terminal, size: Size, cell: CellSize) {
        let mut shared = self.shared.borrow_mut();
        shared.cell = cell;
        if shared.pty.defer_grid_resize() {
            shared.pty.resize(size, cell);
            return;
        }
        // Pixels as whole numbers, at least one, for the engine's mouse
        // and image maths.
        let width = cell.width.round().max(1.0) as u32;
        let height = cell.height.round().max(1.0) as u32;
        match grid.resize_with_cell_size(size.cols, size.rows, width, height) {
            Ok(()) => {
                shared.pty.resize(size, cell);
                // DEC 2048 applications use the engine's in-band resize reply
                // instead of SIGWINCH. Deliver it now, even if output is idle.
                for event in grid.drain_events() {
                    shared.engine_event(event);
                }
            }
            Err(source) => shared.fail(PaneError::Resize {
                cols: size.cols,
                rows: size.rows,
                source,
            }),
        }
    }

    fn focus(&mut self, grid: &mut Terminal, focused: bool) {
        let mut shared = self.shared.borrow_mut();
        match grid.focus_report(focused) {
            Ok(Some(bytes)) => shared.send(&bytes),
            Ok(None) => {}
            Err(source) => shared.fail(PaneError::Focus { source }),
        }
    }

    fn option_as_alt(&self, grid: &Terminal) -> bool {
        grid.option_as_alt() != OptionAsAlt::Off
    }
}

/// Sends `text` as a paste, or holds it for the user to confirm when it
/// could run commands they didn't mean to.
pub(crate) fn paste_text(shared: &mut Shared, grid: &mut Terminal, text: String, confirmed: bool) {
    shared.paste_accepted = false;
    match grid.paste(&text, confirmed) {
        Ok(bytes) => {
            if let Err(source) = follow_output(grid) {
                shared.fail(PaneError::Scroll { source });
            }
            shared.pending_paste = None;
            shared.paste_accepted = shared.connected && shared.pty.write_accepted(&bytes);
        }
        Err(VtError::UnsafePaste { line_breaks }) => {
            shared.pending_paste = Some(PendingPaste { text, line_breaks });
        }
        Err(source) => shared.fail(PaneError::Paste { source }),
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use gpui_kit::ClipboardReadError;
    use std::error::Error as _;

    struct Sink;
    impl Pty for Sink {
        fn write(&mut self, _: &[u8]) {}
        fn resize(&mut self, _: Size, _: CellSize) {}
    }
    #[test]
    fn clipboard_read_failures_keep_distinct_categories_and_original_sources() {
        let shared = Shared::new(Box::new(Sink));
        let mut host = GhosttyHost {
            shared: shared.clone(),
        };
        for source in [
            ClipboardReadError::Unavailable,
            ClipboardReadError::Denied("fixture denied".into()),
            ClipboardReadError::UnsupportedContent,
        ] {
            host.clipboard_read_failed(source.clone());
            let Some(PaneEvent::Failed(error)) = shared.borrow_mut().events.pop() else {
                panic!("clipboard failure did not reach the embedder");
            };
            match (&source, &error) {
                (ClipboardReadError::Unavailable, PaneError::ClipboardUnavailable { .. })
                | (ClipboardReadError::Denied(_), PaneError::ClipboardDenied { .. })
                | (
                    ClipboardReadError::UnsupportedContent,
                    PaneError::ClipboardUnsupported { .. },
                ) => {}
                _ => panic!("clipboard failure lost its category"),
            }
            assert_eq!(
                error.source().unwrap().downcast_ref::<ClipboardReadError>(),
                Some(&source)
            );
        }
    }
}

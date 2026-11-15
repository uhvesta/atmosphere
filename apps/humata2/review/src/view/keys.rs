//! The pane's keys (RV-4). Everything a reviewer does has one:
//!
//! | Key | Does |
//! |---|---|
//! | `n` / `shift-n` | next / previous file not viewed |
//! | `j` `k` / `down` `up` | next / previous line; with `shift`, extend the selection |
//! | `]` / `[` | next / previous comment |
//! | `c` | comment on the selected lines |
//! | `a` | in a PR review, ask the review chat about the selected lines |
//! | `b` | back to where you were before the agent last moved the view |
//! | `v` | mark the file viewed (and go to the next one not viewed), or unmark it |
//! | `x` | collapse or expand the file |
//! | `m` | a markdown file's rendered view, or its source diff |
//! | `s` | send the comment or question under the cursor, after a preview |
//! | `shift-s` | send every draft and question together, after a preview |
//! | `enter` / `escape` | in the preview, send / don't |
//! | `1` `2` `3` | an image's side by side, swipe and onion skin |
//! | `d` | mark an image's changed pixels |
//! | `left` / `right` | move an image's swipe or onion skin |
//!
//! In the comment box, `cmd-enter` adds the comment and `escape` closes it.
//! Moving with keys, like scrolling and typing, holds the agent's view
//! changes until you stop (PR-24).

use gpui_kit::{Context, KeyDownEvent, Window};

use super::ReviewPane;
use crate::model::{BodyRow, Content, ImageMode, Viewed};

/// How far `left` and `right` move a swipe or onion skin, per mille.
const IMAGE_STEP: u16 = 100;

impl ReviewPane {
    pub(crate) fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;
        if mods.platform || mods.control || mods.alt {
            return;
        }
        let key = keystroke.key.as_str();
        let shift = mods.shift;
        if self.preview.is_some() {
            match key {
                "enter" => self.confirm_preview(cx),
                "escape" => {
                    self.preview = None;
                    cx.notify();
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if matches!(key, "n" | "j" | "k" | "down" | "up" | "]" | "[" | "}" | "{") {
            self.user_active(cx);
        }
        match (key, shift) {
            ("n", false) => self.next_unviewed(true, cx),
            ("n", true) => self.next_unviewed(false, cx),
            ("j" | "down", _) => self.step(1, shift, cx),
            ("k" | "up", _) => self.step(-1, shift, cx),
            ("]", _) | ("}", true) => self.next_comment(true, cx),
            ("[", _) | ("{", true) => self.next_comment(false, cx),
            ("c", false) => self.comment_on_selection(window, cx),
            ("v", false) => self.viewed_key(cx),
            ("x", false) => {
                if let Some(key) = self.cursor_file().and_then(|file| self.file_key(file)) {
                    self.on_memory(|memory| memory.toggle_collapsed(&key), cx);
                }
            }
            ("m", false) => self.toggle_rendered(cx),
            ("s", false) => {
                if let Some(BodyRow::Comment { id, .. }) = self.cursor.clone() {
                    self.open_preview(Some(vec![id]), cx);
                } else {
                    self.notice = Some("Move to a comment with ] first, or press shift-s to send every draft.".into());
                    cx.notify();
                }
            }
            ("s", true) => self.open_preview(None, cx),
            ("a", false) => self.ask_on_selection(window, cx),
            ("b", false) => self.back(cx),
            ("1", false) => self.image_key(Some(ImageMode::SideBySide), None, cx),
            ("2", false) => self.image_key(Some(ImageMode::Swipe { position: 500 }), None, cx),
            ("3", false) => self.image_key(Some(ImageMode::Onion { opacity: 500 }), None, cx),
            ("d", false) => {
                let on = self
                    .image_file()
                    .and_then(|file| self.file_key(file))
                    .and_then(|key| self.current_review().map(|r| r.memory().image(&key).highlight));
                if let Some(on) = on {
                    self.image_key(None, Some(!on), cx);
                }
            }
            ("left", false) => self.nudge_image(-1, cx),
            ("right", false) => self.nudge_image(1, cx),
            ("escape", _) => {
                self.anchor = None;
                self.notice = None;
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The comment box's keys: text goes in, `cmd-enter` adds, `escape` closes.
    pub(crate) fn composer_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;
        cx.stop_propagation();
        if mods.secondary(cx) && keystroke.key == "enter" {
            self.submit_composer(window, cx);
        } else if mods.secondary(cx) && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.type_in_composer(&text, cx);
            }
        } else if keystroke.key == "escape" {
            self.cancel_composer(window, cx);
        } else if keystroke.key == "backspace" {
            self.erase_in_composer(cx);
        } else if keystroke.key == "enter" {
            self.type_in_composer("\n", cx);
        } else if !mods.control && !mods.platform {
            if let Some(text) = keystroke.key_char.as_deref() {
                self.type_in_composer(text, cx);
            }
        }
    }

    fn cursor_index(&self) -> Option<usize> {
        self.cursor.as_ref().and_then(|row| self.row_index.get(row)).copied()
    }

    pub(crate) fn cursor_file(&self) -> Option<usize> {
        self.cursor.as_ref().map(BodyRow::file)
    }

    /// The image the cursor is in, if it's on an image change.
    fn image_file(&self) -> Option<usize> {
        let file = self.cursor_file()?;
        let review = self.current_review()?;
        matches!(review.review().files().get(file)?.content, Content::Image(_)).then_some(file)
    }

    fn step(&mut self, by: isize, extend: bool, cx: &mut Context<Self>) {
        let start = self.cursor_index();
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let mut index = match start {
            Some(index) => index,
            None if by > 0 => usize::MAX,
            None => len,
        };
        loop {
            index = if by > 0 {
                index.wrapping_add(1)
            } else {
                match index.checked_sub(1) {
                    Some(index) => index,
                    None => return,
                }
            };
            let Some(row) = self.rows.get(index) else {
                return;
            };
            if extend {
                // Extending stays among lines, or blocks, of one file.
                let Some(anchor) = self.anchor.clone().or_else(|| self.cursor.clone()) else {
                    return;
                };
                if row.file() != anchor.file() {
                    return;
                }
                if matches!(row, BodyRow::Line { .. } | BodyRow::Block { .. }) {
                    if self.anchor.is_none() {
                        self.anchor = Some(anchor);
                    }
                    let row = row.clone();
                    self.select(row, true, cx);
                    return;
                }
                continue;
            }
            if matches!(row, BodyRow::Hunk { .. } | BodyRow::Composer { .. } | BodyRow::Group(_)) {
                continue;
            }
            let row = row.clone();
            if matches!(row, BodyRow::Line { .. } | BodyRow::Block { .. }) {
                self.select(row, false, cx);
            } else {
                self.anchor = None;
                self.go_to(row, cx);
            }
            return;
        }
    }

    /// Goes to the next (or previous) file not viewed at its current
    /// version, after the cursor's, wrapping around.
    pub(crate) fn next_unviewed(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(review) = self.current_review() else {
            return;
        };
        let files: Vec<usize> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                BodyRow::File(file) => Some(*file),
                _ => None,
            })
            .collect();
        let unviewed = |file: &usize| {
            review
                .review()
                .files()
                .get(*file)
                .is_some_and(|change| review.memory().viewed(&change.key, &change.blob) != Viewed::Yes)
        };
        let here = self.cursor_file();
        let position = here.and_then(|here| files.iter().position(|file| *file == here));
        let count = files.len();
        let next = (1..=count).find_map(|step| {
            let index = match position {
                Some(position) if forward => (position + step) % count,
                Some(position) => (position + count * 2 - step) % count,
                None if forward => step - 1,
                None => count - step,
            };
            files.get(index).copied().filter(|file| unviewed(file) && Some(*file) != here)
        });
        match next {
            Some(file) => {
                self.notice = None;
                self.anchor = None;
                self.go_to(BodyRow::File(file), cx);
            }
            None => {
                self.notice = Some("Every other file is viewed.".into());
                cx.notify();
            }
        }
    }

    /// Goes to the next (or previous) comment card, wrapping around.
    fn next_comment(&mut self, forward: bool, cx: &mut Context<Self>) {
        let comments: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, BodyRow::Comment { .. }))
            .map(|(index, _)| index)
            .collect();
        if comments.is_empty() {
            self.notice = Some("There are no comments here yet: select lines and press c.".into());
            cx.notify();
            return;
        }
        let here = self.cursor_index();
        let next = if forward {
            comments
                .iter()
                .find(|index| here.is_none_or(|here| **index > here))
                .or(comments.first())
        } else {
            comments
                .iter()
                .rev()
                .find(|index| here.is_none_or(|here| **index < here))
                .or(comments.last())
        };
        if let Some(row) = next.and_then(|index| self.rows.get(*index)).cloned() {
            self.anchor = None;
            self.notice = None;
            self.go_to(row, cx);
        }
    }

    /// Marks the cursor's file viewed and moves on, or unmarks it.
    fn viewed_key(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.cursor_file() else {
            return;
        };
        let was = self.current_review().and_then(|review| {
            let change = review.review().files().get(file)?;
            Some(review.memory().viewed(&change.key, &change.blob) == Viewed::Yes)
        });
        self.toggle_viewed(file, cx);
        if was == Some(false) {
            self.next_unviewed(true, cx);
        }
    }

    fn toggle_rendered(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.cursor_file() else {
            return;
        };
        let markdown = self.current_review().is_some_and(|review| {
            matches!(
                review.review().files().get(file).map(|f| &f.content),
                Some(Content::Markdown(_))
            )
        });
        if !markdown {
            self.notice = Some("Only markdown files have a rendered view.".into());
            cx.notify();
            return;
        }
        self.toggle_rendered_file(file, cx);
    }

    pub(crate) fn toggle_rendered_file(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(key) = self.file_key(file) else {
            return;
        };
        self.anchor = None;
        self.cursor = Some(BodyRow::File(file));
        self.on_memory(
            |memory| {
                if !memory.rendered.remove(&key) {
                    memory.rendered.insert(key);
                }
            },
            cx,
        );
    }

    fn image_key(&mut self, mode: Option<ImageMode>, highlight: Option<bool>, cx: &mut Context<Self>) {
        match self.image_file() {
            Some(file) => self.set_image_mode(file, mode, highlight, cx),
            None => {
                self.notice = Some("Move to an image first: n goes from file to file.".into());
                cx.notify();
            }
        }
    }

    fn nudge_image(&mut self, by: i32, cx: &mut Context<Self>) {
        let Some(file) = self.image_file() else {
            return;
        };
        let Some(key) = self.file_key(file) else {
            return;
        };
        let Some(view) = self.current_review().map(|r| r.memory().image(&key)) else {
            return;
        };
        let nudge = |value: u16| {
            let moved = i32::from(value) + by * i32::from(IMAGE_STEP);
            u16::try_from(moved.clamp(0, 1000)).unwrap_or(500)
        };
        let mode = match view.mode {
            ImageMode::SideBySide => return,
            ImageMode::Swipe { position } => ImageMode::Swipe {
                position: nudge(position),
            },
            ImageMode::Onion { opacity } => ImageMode::Onion {
                opacity: nudge(opacity),
            },
        };
        self.set_image_mode(file, Some(mode), None, cx);
    }
}


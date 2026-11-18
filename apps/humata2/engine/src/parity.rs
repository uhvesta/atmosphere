//! Parity: a window drawing natural frames must show what a fresh window
//! shows after the same input.
//!
//! A [`Pair`] is two windows holding the same view, built the same way, in
//! one headless context. The subject draws as the app does: natural frames
//! only, with GPUI Fast's view retention (and, if the test turns them on,
//! scroll layers). The reference has retention and scroll layers off and is
//! refreshed before every comparison, so each of its frames is built from
//! scratch. Every input goes to both. [`Pair::check`] compares the frames
//! they last drew:
//!
//! - every observed element: role, label, value, states, focus, bounds and
//!   visibility;
//! - the text recorded in the subject, which must all be in the reference
//!   with its bounds (text of a retained view isn't recorded; see
//!   [`crate::frame`]);
//! - unless scroll layers are on, the painted quads, and on GPUI Fast the
//!   painted glyphs, icons and images, as multisets;
//! - the pixels, within a channel tolerance.
//!
//! On ordinary GPUI both windows rebuild their views every frame, so parity
//! there checks only that natural frames match refreshed ones.

use std::collections::BTreeMap;
use std::fmt;

use gpui_kit::base::test_support::TextSnapshot;
use gpui_kit::{AnyWindowHandle, App, HeadlessAppContext, Window};

use crate::frame::{self, Frame, FrameError, Picture};
use crate::window::{self, FrameCounts, Unsupported};

/// The subject and reference windows of a parity test.
#[derive(Clone, Copy, Debug)]
pub struct Pair {
    /// Draws natural frames with the engine's retention.
    pub subject: AnyWindowHandle,
    /// Draws fresh frames, nothing retained.
    pub reference: AnyWindowHandle,
    /// Whether the subject composites scroll layers.
    pub scroll_layers: bool,
}

/// Why a [`Pair`] couldn't be set up or compared.
#[derive(Debug, thiserror::Error)]
pub enum ParityError {
    /// A window couldn't be drawn, driven or read.
    #[error(transparent)]
    Frame(#[from] FrameError),
    /// The engine lacks a switch the test asked for.
    #[error(transparent)]
    Unsupported(#[from] Unsupported),
}

impl Pair {
    /// Makes `subject` and `reference` a pair: retention stays as the
    /// engine has it in the subject, scroll layers are set to
    /// `scroll_layers` there, and both are off in the reference. Draws a
    /// first frame in each.
    ///
    /// # Errors
    ///
    /// [`ParityError::Unsupported`] when asking ordinary GPUI for scroll
    /// layers, and [`ParityError::Frame`] when a window is closed.
    pub fn new(
        cx: &mut HeadlessAppContext,
        subject: AnyWindowHandle,
        reference: AnyWindowHandle,
        scroll_layers: bool,
    ) -> Result<Self, ParityError> {
        frame::in_window(cx, subject, |window, _| {
            window::set_scroll_layers(window, scroll_layers)
        })??;
        frame::in_window(cx, reference, |window, _| {
            window::set_view_retention(window, false)?;
            window::set_scroll_layers(window, false)
        })??;
        let pair = Self {
            subject,
            reference,
            scroll_layers,
        };
        pair.settle(cx)?;
        frame::in_window(cx, subject, |window, _| window::reset_frame_counts(window))?;
        Ok(pair)
    }

    /// Runs `input` in both windows: the same events, each followed by a
    /// natural frame (see the input functions in [`crate::frame`]).
    ///
    /// # Errors
    ///
    /// What `input` returns, or [`FrameError::WindowGone`].
    pub fn input(
        &self,
        cx: &mut HeadlessAppContext,
        mut input: impl FnMut(&mut Window, &mut App) -> Result<(), FrameError>,
    ) -> Result<(), FrameError> {
        for window in [self.subject, self.reference] {
            frame::in_window(cx, window, &mut input)??;
        }
        Ok(())
    }

    /// Runs `update` on both windows' state, outside any input: a change a
    /// daemon or a timer would make.
    ///
    /// # Errors
    ///
    /// [`FrameError::WindowGone`].
    pub fn update(
        &self,
        cx: &mut HeadlessAppContext,
        mut update: impl FnMut(&mut Window, &mut App),
    ) -> Result<(), FrameError> {
        for window in [self.subject, self.reference] {
            frame::in_window(cx, window, &mut update)?;
        }
        Ok(())
    }

    /// Lets both windows settle: tasks run and natural frames are drawn
    /// until two in a row match (at most 12).
    ///
    /// # Errors
    ///
    /// [`FrameError::Unsettled`] or [`FrameError::WindowGone`].
    pub fn settle(&self, cx: &mut HeadlessAppContext) -> Result<(), FrameError> {
        for window in [self.subject, self.reference] {
            frame::settle(
                cx,
                window,
                |cx: &mut HeadlessAppContext| cx.run_until_parked(),
                12,
            )?;
        }
        Ok(())
    }

    /// The subject's frame counters since the pair was made, or `None` on
    /// ordinary GPUI.
    ///
    /// # Errors
    ///
    /// [`FrameError::WindowGone`].
    pub fn subject_counts(
        &self,
        cx: &mut HeadlessAppContext,
    ) -> Result<Option<FrameCounts>, FrameError> {
        frame::in_window(cx, self.subject, |window, _| window::frame_counts(window))
    }

    /// Compares what the two windows show now. The subject is read as it
    /// was last drawn; the reference is refreshed and drawn first.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when a window is closed or can't be drawn to pixels.
    pub fn check(
        &self,
        cx: &mut HeadlessAppContext,
        channel_tolerance: u8,
    ) -> Result<Report, FrameError> {
        let (subject, subject_picture) = frame::in_window(cx, self.subject, |window, _| {
            Ok::<_, FrameError>((frame::observe(window), frame::pixels(window)?))
        })??;
        let (reference, reference_picture) =
            frame::in_window(cx, self.reference, |window, cx| {
                frame::full_refresh(window, cx);
                Ok::<_, FrameError>((frame::observe(window), frame::pixels(window)?))
            })??;
        Ok(compare(
            &subject,
            &reference,
            &subject_picture,
            &reference_picture,
            Comparison {
                channel_tolerance,
                painted: !self.scroll_layers,
            },
        ))
    }
}

/// Checks one window's last natural frame against the same window refreshed:
/// observes the frame as drawn, refreshes and draws it again, observes
/// that, and compares the two. Afterwards the window holds the refreshed
/// frame, which matches the natural one when the report does, so it can be
/// captured for snapshots (with the text of retained views, which the
/// natural frame doesn't record).
///
/// Compares painted quads and sprites only when `comparison.painted` is
/// set, which needs scroll layers off; compares pixels only when `pixels`
/// is set, which needs a context with a renderer.
///
/// # Errors
///
/// [`FrameError::Render`] when `pixels` is set and the context can't draw.
pub fn against_refresh(
    window: &mut Window,
    cx: &mut App,
    pixels: bool,
    comparison: Comparison,
) -> Result<Report, FrameError> {
    let picture = |window: &Window| {
        if pixels {
            frame::pixels(window)
        } else {
            Ok(Picture {
                width: 0,
                height: 0,
                rgba: Vec::new(),
            })
        }
    };
    let natural = frame::observe(window);
    let natural_picture = picture(window)?;
    frame::full_refresh(window, cx);
    let refreshed = frame::observe(window);
    let refreshed_picture = picture(window)?;
    Ok(compare(
        &natural,
        &refreshed,
        &natural_picture,
        &refreshed_picture,
        comparison,
    ))
}

/// One way the subject's frame differs from the reference's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// An element differs, or is in only one of them.
    Element {
        /// The element's path.
        path: String,
        /// What the subject shows, if it has the element.
        subject: Option<String>,
        /// What the reference shows, if it has the element.
        reference: Option<String>,
    },
    /// Text the subject drew that the reference didn't.
    Text {
        /// The text, its path and bounds.
        subject: String,
    },
    /// Painted primitives in only one of them.
    Painted {
        /// `quads` or `sprites`.
        kind: &'static str,
        /// In the subject only (the first few).
        subject_only: Vec<String>,
        /// In the reference only (the first few).
        reference_only: Vec<String>,
    },
    /// The pictures have different sizes.
    PictureSize {
        /// The subject's width and height.
        subject: (u32, u32),
        /// The reference's.
        reference: (u32, u32),
    },
    /// Pixels differ by more than the tolerance.
    Pixels {
        /// How many pixels differ.
        differing: usize,
        /// The first differing pixel, top to bottom, left to right.
        first: (u32, u32),
    },
}

/// What [`Pair::check`] found.
#[derive(Clone, Debug)]
pub struct Report {
    /// Every difference; empty when the frames match.
    pub mismatches: Vec<Mismatch>,
    /// How many of the reference's texts the subject didn't record because
    /// it drew them again from an earlier frame.
    pub texts_retained: usize,
    /// The subject's frame counters since the pair was made, if the engine
    /// counts.
    pub subject_counts: Option<FrameCounts>,
}

impl Report {
    /// Whether the frames match.
    #[must_use]
    pub fn matches(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mismatches.is_empty() {
            return write!(f, "the frames match");
        }
        writeln!(
            f,
            "the subject's frame differs from the fresh reference in {} ways:",
            self.mismatches.len()
        )?;
        for mismatch in &self.mismatches {
            match mismatch {
                Mismatch::Element {
                    path,
                    subject,
                    reference,
                } => writeln!(
                    f,
                    "- element {path}: subject {}, reference {}",
                    subject.as_deref().unwrap_or("(missing)"),
                    reference.as_deref().unwrap_or("(missing)")
                )?,
                Mismatch::Text { subject } => {
                    writeln!(f, "- text only in the subject: {subject}")?;
                }
                Mismatch::Painted {
                    kind,
                    subject_only,
                    reference_only,
                } => writeln!(
                    f,
                    "- {kind}: subject only {subject_only:?}, reference only {reference_only:?}"
                )?,
                Mismatch::PictureSize { subject, reference } => {
                    writeln!(
                        f,
                        "- picture size: subject {subject:?}, reference {reference:?}"
                    )?;
                }
                Mismatch::Pixels { differing, first } => {
                    writeln!(f, "- {differing} pixels differ, the first at {first:?}")?;
                }
            }
        }
        Ok(())
    }
}

/// What [`compare`] compares beyond the elements and text.
#[derive(Clone, Copy, Debug)]
pub struct Comparison {
    /// How far a pixel's channel may be off before it differs.
    pub channel_tolerance: u8,
    /// Whether to compare the painted quads and sprites. A composited
    /// scroll layer paints its content into the layer's tiles rather than
    /// the frame, so with scroll layers on only pixels can tell.
    pub painted: bool,
}

/// Compares two observed frames and their pictures.
#[must_use]
pub fn compare(
    subject: &Frame,
    reference: &Frame,
    subject_picture: &Picture,
    reference_picture: &Picture,
    comparison: Comparison,
) -> Report {
    let channel_tolerance = comparison.channel_tolerance;
    let mut mismatches = Vec::new();

    let elements = |frame: &Frame| -> BTreeMap<String, String> {
        frame
            .elements
            .iter()
            .map(|element| {
                (
                    without_entity_ids(&format!("{:?}", element.path())),
                    without_entity_ids(&format!("{element:?}")),
                )
            })
            .collect()
    };
    let (subject_elements, reference_elements) = (elements(subject), elements(reference));
    let mut paths: Vec<&String> = subject_elements
        .keys()
        .chain(reference_elements.keys())
        .collect();
    paths.sort();
    paths.dedup();
    for path in paths {
        let (s, r) = (subject_elements.get(path), reference_elements.get(path));
        if s != r {
            mismatches.push(Mismatch::Element {
                path: path.clone(),
                subject: s.cloned(),
                reference: r.cloned(),
            });
        }
    }

    let text_key = |text: &TextSnapshot| without_entity_ids(&format!("{text:?}"));
    let mut reference_texts = Multiset::of(reference.texts.iter().map(text_key));
    for text in &subject.texts {
        let text = text_key(text);
        if !reference_texts.take(&text) {
            mismatches.push(Mismatch::Text { subject: text });
        }
    }
    let texts_retained = reference_texts.len();

    if comparison.painted {
        painted("quads", &subject.quads, &reference.quads, &mut mismatches);
        if let (Some(s), Some(r)) = (&subject.sprites, &reference.sprites) {
            painted("sprites", s, r, &mut mismatches);
        }
    }

    let (s, r) = (subject_picture, reference_picture);
    if (s.width, s.height) != (r.width, r.height) {
        mismatches.push(Mismatch::PictureSize {
            subject: (s.width, s.height),
            reference: (r.width, r.height),
        });
    } else {
        let mut differing = 0;
        let mut first = None;
        for (index, (a, b)) in s
            .rgba
            .chunks_exact(4)
            .zip(r.rgba.chunks_exact(4))
            .enumerate()
        {
            if a.iter()
                .zip(b)
                .any(|(a, b)| a.abs_diff(*b) > channel_tolerance)
            {
                differing += 1;
                if first.is_none() {
                    let index = u32::try_from(index).unwrap_or(u32::MAX);
                    first = Some((index % s.width.max(1), index / s.width.max(1)));
                }
            }
        }
        if let Some(first) = first {
            mismatches.push(Mismatch::Pixels { differing, first });
        }
    }

    Report {
        mismatches,
        texts_retained,
        subject_counts: subject.counts,
    }
}

fn painted(kind: &'static str, subject: &[String], reference: &[String], out: &mut Vec<Mismatch>) {
    let mut only_reference = Multiset::of(reference.iter().cloned());
    let mut subject_only = Vec::new();
    for line in subject {
        if !only_reference.take(line) {
            subject_only.push(line.clone());
        }
    }
    let reference_only = only_reference.into_vec();
    if !subject_only.is_empty() || !reference_only.is_empty() {
        subject_only.truncate(5);
        out.push(Mismatch::Painted {
            kind,
            subject_only,
            reference_only: reference_only.into_iter().take(5).collect(),
        });
    }
}

/// Replaces every `EntityId(…)` with `EntityId(_)`, and every number of
/// 2^32 or more with `_`: the two windows hold different entities for the
/// same views, and views often name elements after their entity's id (as a
/// `u64`, whose top half is never zero), so paths are compared by shape. A
/// count or index that large would be hidden too; none is shown in a frame.
fn without_entity_ids(debug: &str) -> String {
    const MARK: &str = "EntityId(";
    let mut marked = String::with_capacity(debug.len());
    let mut rest = debug;
    while let Some(start) = rest.find(MARK) {
        let after = &rest[start + MARK.len()..];
        let Some(end) = after.find(')') else {
            break;
        };
        marked.push_str(&rest[..start]);
        marked.push_str("EntityId(_)");
        rest = &after[end + 1..];
    }
    marked.push_str(rest);

    let mut out = String::with_capacity(marked.len());
    let mut digits = String::new();
    let flush = |digits: &mut String, out: &mut String| {
        if digits
            .parse::<u64>()
            .map_or(digits.len() > 10, |n| n >= 1 << 32)
        {
            out.push('_');
        } else {
            out.push_str(digits);
        }
        digits.clear();
    };
    for character in marked.chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else {
            flush(&mut digits, &mut out);
            out.push(character);
        }
    }
    flush(&mut digits, &mut out);
    out
}

struct Multiset(BTreeMap<String, usize>);

impl Multiset {
    fn of(items: impl Iterator<Item = String>) -> Self {
        let mut counts = BTreeMap::new();
        for item in items {
            *counts.entry(item).or_insert(0) += 1;
        }
        Self(counts)
    }

    fn take(&mut self, item: &str) -> bool {
        match self.0.get_mut(item) {
            Some(count) if *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        }
    }

    fn len(&self) -> usize {
        self.0.values().sum()
    }

    fn into_vec(self) -> Vec<String> {
        self.0
            .into_iter()
            .flat_map(|(item, count)| std::iter::repeat_n(item, count))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::without_entity_ids;

    #[test]
    fn ids_made_from_entities_compare_by_shape_and_small_numbers_stay() {
        let subject =
            r#"[View(EntityId(5v1)), Name("21474836678"), NamedInteger("row", 3)] 873.5px"#;
        let reference =
            r#"[View(EntityId(9v1)), Name("30064771233"), NamedInteger("row", 3)] 873.5px"#;
        assert_eq!(without_entity_ids(subject), without_entity_ids(reference));
        let other_row =
            r#"[View(EntityId(9v1)), Name("30064771233"), NamedInteger("row", 4)] 873.5px"#;
        assert_ne!(without_entity_ids(subject), without_entity_ids(other_row));
        let moved = r#"[View(EntityId(9v1)), Name("30064771233"), NamedInteger("row", 3)] 874.5px"#;
        assert_ne!(without_entity_ids(subject), without_entity_ids(moved));
    }
}

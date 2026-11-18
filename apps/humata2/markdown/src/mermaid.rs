//! Mermaid diagrams: a fenced `mermaid` block becomes a picture. Diagrams are
//! drawn off the UI thread, with a placeholder until they are ready, and kept
//! in a cache keyed by source and theme, so scrolling back to one, or
//! streaming text after it, never draws it again.

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard};

use gpui_kit::base::markdown_ast::Node;
use gpui_kit::base::{MarkdownNode, MarkdownParseContext};
use gpui_kit::{RenderImage, SvgRenderer};
use mermaid_rs_renderer::{LayoutConfig, ParseError, RenderOptions, render_strict};

use crate::document::fence_closed;
use crate::error::DiagramError;
use crate::zoom::BASE_RASTER;

/// The name of the custom block the text view draws diagrams with.
pub(crate) const NODE: &str = "mermaid";

/// How many pictures a cache keeps; the least recently used goes first.
const CAPACITY: usize = 64;
/// How many zoomed pictures (drawn larger than the base) a cache keeps, and
/// their pixels in all (96 MB): they are large, so few are kept.
const ZOOMED_CAPACITY: usize = 4;
const ZOOMED_PIXELS: u64 = 24_000_000;

/// A diagram block, as the parser found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiagramBlock {
    /// The diagram's source, without its fences.
    pub(crate) source: Arc<str>,
    /// Whether the closing fence has arrived. An open diagram is still
    /// streaming, so it isn't drawn yet.
    pub(crate) closed: bool,
    /// The block's bytes in the document.
    pub(crate) bytes: Range<usize>,
}

/// Claim fenced `mermaid` code for the diagram renderer.
pub(crate) fn parse(node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
    let Node::Code(code) = node else { return None };
    if !code.lang.as_deref().is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid")) {
        return None;
    }
    let position = node.position()?;
    let raw = cx.node_source(node)?;
    let start = cx.offset() + position.start.offset;
    let block = DiagramBlock {
        source: Arc::from(code.value.as_str()),
        closed: fence_closed(raw),
        bytes: start..start + raw.len(),
    };
    let label = format!("Diagram: {}", first_line(&code.value));
    Some(
        MarkdownNode::new(NODE, block)
            .text(code.value.clone())
            .markdown(raw.to_owned())
            .accessibility_label(label),
    )
}

/// The first non-empty line of a diagram, which names its kind.
pub(crate) fn first_line(source: &str) -> &str {
    source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("empty")
}

/// A drawn diagram: its picture and the SVG it was drawn from, kept so a
/// zoom can draw it again at another size without laying it out again.
#[derive(Clone)]
pub(crate) struct Drawn {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) markup: Arc<str>,
}

/// Draw `source` with `theme`. Runs off the UI thread.
pub(crate) fn render(source: &str, theme: mermaid_rs_renderer::Theme, svg: &SvgRenderer) -> Result<Drawn, DiagramError> {
    let options = RenderOptions {
        theme,
        layout: LayoutConfig::default(),
    };
    // The renderer is young; a panic in it is its bug, and must not take the
    // app down.
    let drawn = catch_unwind(AssertUnwindSafe(|| render_strict(source, options)))
        .map_err(|_| DiagramError::Crashed)?;
    let markup = drawn.map_err(|source| {
        let (line, column) = position(&source);
        DiagramError::Syntax {
            line,
            column,
            source,
        }
    })?;
    let markup: Arc<str> = Arc::from(markup);
    // Twice the pixels, so the diagram stays sharp on a Retina screen.
    let image = raster(&markup, BASE_RASTER, svg)?;
    Ok(Drawn { image, markup })
}

/// Draw a laid-out diagram's SVG at `raster` times its natural size. Runs
/// off the UI thread.
pub(crate) fn raster(markup: &str, raster: u8, svg: &SvgRenderer) -> Result<Arc<RenderImage>, DiagramError> {
    svg.render_single_frame(markup.as_bytes(), f32::from(raster))
        .map_err(|source| DiagramError::Draw {
            source: Box::new(source),
        })
}

/// Where in the diagram's source a parse error is.
fn position(error: &ParseError) -> (Option<u32>, Option<u32>) {
    match error {
        ParseError::UnknownParticipant { line, .. } => (Some(*line), None),
        ParseError::UnclosedSubgraph { opened_at } => (Some(*opened_at), None),
        ParseError::UnexpectedToken { line, col, .. } | ParseError::InvalidDirective { line, col, .. } => {
            (Some(*line), Some(*col))
        }
        _ => (None, None),
    }
}

/// A hash of everything in a Mermaid theme, for cache keys.
pub(crate) fn theme_key(theme: &mermaid_rs_renderer::Theme) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{theme:?}").hash(&mut hasher);
    hasher.finish()
}

/// A diagram's state in the cache.
#[derive(Clone)]
pub(crate) enum DiagramEntry {
    Rendering,
    Ready(Drawn),
    Failed(Arc<DiagramError>),
}

/// Counts of diagram work, to prove a cached diagram isn't drawn again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiagramStats {
    /// Diagrams drawn, successfully or not.
    pub renders: u64,
    /// Lookups the cache answered with a finished diagram or error.
    pub hits: u64,
}

/// A picture: the diagram's source, its theme and the size it is drawn at
/// ([`BASE_RASTER`] or a zoom's bucket).
type Key = (Arc<str>, u64, u8);

#[derive(Default)]
struct Diagrams {
    entries: HashMap<Key, DiagramEntry>,
    /// Finished keys, least recently used first.
    order: VecDeque<Key>,
    /// The pixels of the zoomed pictures kept.
    zoomed_pixels: u64,
    stats: DiagramStats,
}

fn pixels(entry: &DiagramEntry) -> u64 {
    match entry {
        DiagramEntry::Ready(drawn) => {
            let size = drawn.image.size(0);
            u64::try_from(size.width.0).unwrap_or(0) * u64::try_from(size.height.0).unwrap_or(0)
        }
        _ => 0,
    }
}

impl Diagrams {
    /// Drop `key`, handing back its picture so the caller can free it on the
    /// GPU.
    fn evict(&mut self, key: &Key, evicted: &mut Vec<Arc<RenderImage>>) {
        if let Some(at) = self.order.iter().position(|k| k == key) {
            self.order.remove(at);
        }
        if let Some(entry) = self.entries.remove(key) {
            if key.2 != BASE_RASTER {
                self.zoomed_pixels = self.zoomed_pixels.saturating_sub(pixels(&entry));
            }
            if let DiagramEntry::Ready(drawn) = entry {
                evicted.push(drawn.image);
            }
        }
    }
}

/// Drawn diagrams, by source and theme. Clones share one cache, so views
/// showing the same diagram draw it once.
#[derive(Clone, Default)]
pub struct MermaidCache {
    inner: Arc<Mutex<Diagrams>>,
}

impl MermaidCache {
    fn lock(&self) -> MutexGuard<'_, Diagrams> {
        // A panic while holding the lock leaves the map whole; keep using it.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn get(&self, source: &Arc<str>, theme: u64, raster: u8) -> Option<DiagramEntry> {
        let mut diagrams = self.lock();
        let key = (source.clone(), theme, raster);
        let entry = diagrams.entries.get(&key).cloned();
        if matches!(entry, Some(DiagramEntry::Ready(_) | DiagramEntry::Failed(_))) {
            diagrams.stats.hits += 1;
            if let Some(at) = diagrams.order.iter().position(|k| *k == key) {
                diagrams.order.remove(at);
            }
            diagrams.order.push_back(key);
        }
        entry
    }

    /// Mark a diagram as being drawn; false when it is already known.
    pub(crate) fn begin(&self, source: &Arc<str>, theme: u64, raster: u8) -> bool {
        let mut diagrams = self.lock();
        let key = (source.clone(), theme, raster);
        if diagrams.entries.contains_key(&key) {
            return false;
        }
        diagrams.entries.insert(key, DiagramEntry::Rendering);
        true
    }

    /// Keep a drawn picture. Returns the pictures dropped to make room, for
    /// the caller to free on the GPU.
    #[must_use]
    pub(crate) fn finish(&self, source: Arc<str>, theme: u64, raster: u8, entry: DiagramEntry) -> Vec<Arc<RenderImage>> {
        let mut diagrams = self.lock();
        let key = (source, theme, raster);
        let mut evicted = Vec::new();
        diagrams.stats.renders += 1;
        if let Some(at) = diagrams.order.iter().position(|k| *k == key) {
            diagrams.order.remove(at);
        }
        if raster != BASE_RASTER {
            diagrams.zoomed_pixels += pixels(&entry);
        }
        if let Some(old) = diagrams.entries.insert(key.clone(), entry) {
            if raster != BASE_RASTER {
                diagrams.zoomed_pixels = diagrams.zoomed_pixels.saturating_sub(pixels(&old));
            }
            if let DiagramEntry::Ready(old) = old {
                evicted.push(old.image);
            }
        }
        diagrams.order.push_back(key.clone());
        while diagrams.order.len() > CAPACITY {
            let Some(old) = diagrams.order.front().cloned() else { break };
            diagrams.evict(&old, &mut evicted);
        }
        // Zoomed pictures past their budget go oldest first, never the one
        // just drawn.
        loop {
            let zoomed: Vec<&Key> = diagrams.order.iter().filter(|k| k.2 != BASE_RASTER).collect();
            let over = zoomed.len() > ZOOMED_CAPACITY || diagrams.zoomed_pixels > ZOOMED_PIXELS;
            let Some(oldest) = zoomed.first().filter(|oldest| over && ***oldest != key).map(|k| (*k).clone()) else {
                break;
            };
            diagrams.evict(&oldest, &mut evicted);
        }
        evicted
    }

    /// What the cache has done so far.
    pub fn stats(&self) -> DiagramStats {
        self.lock().stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SM-3
    #[test]
    fn a_finished_diagram_is_a_hit_and_isnt_drawn_again() {
        let cache = MermaidCache::default();
        let source: Arc<str> = Arc::from("graph TD; A-->B");
        assert!(cache.get(&source, 1, BASE_RASTER).is_none());
        assert!(cache.begin(&source, 1, BASE_RASTER));
        assert!(!cache.begin(&source, 1, BASE_RASTER));
        let _ = cache.finish(source.clone(), 1, BASE_RASTER, DiagramEntry::Failed(Arc::new(DiagramError::Crashed)));
        assert!(matches!(cache.get(&source, 1, BASE_RASTER), Some(DiagramEntry::Failed(_))));
        assert_eq!(cache.stats(), DiagramStats { renders: 1, hits: 1 });
        // Another theme is another picture.
        assert!(cache.begin(&source, 2, BASE_RASTER));
    }

    /// SM-3
    #[test]
    fn the_least_recently_used_diagram_is_dropped_first() {
        let cache = MermaidCache::default();
        let keep: Arc<str> = Arc::from("keep");
        let _ = cache.finish(keep.clone(), 0, BASE_RASTER, DiagramEntry::Failed(Arc::new(DiagramError::Crashed)));
        for ix in 0..CAPACITY {
            // Touch `keep` so it stays recent.
            cache.get(&keep, 0, BASE_RASTER);
            let other: Arc<str> = Arc::from(format!("other {ix}"));
            let _ = cache.finish(other, 0, BASE_RASTER, DiagramEntry::Failed(Arc::new(DiagramError::Crashed)));
        }
        assert!(cache.get(&keep, 0, BASE_RASTER).is_some());
        assert!(cache.get(&Arc::from("other 0"), 0, BASE_RASTER).is_none());
    }

    /// SM-3
    #[test]
    fn flowchart_sequence_class_state_and_gantt_diagrams_draw() {
        let diagrams = [
            "flowchart LR\n  A[Plan] --> B{Ready?}\n  B -->|yes| C[Ship]\n  B -->|no| A\n",
            "sequenceDiagram\n  participant U as User\n  participant H as Host\n  U->>H: attach\n  H-->>U: scrollback\n",
            "classDiagram\n  class Document {\n    +source: String\n    +append(text)\n  }\n  Document <|-- Diff\n",
            "stateDiagram-v2\n  [*] --> Current\n  Current --> Outdated: edited\n  Outdated --> Gone: deleted\n  Gone --> [*]\n",
            "gantt\n  title Plan\n  dateFormat YYYY-MM-DD\n  section Build\n  Parser :a1, 2026-11-01, 3d\n  View :after a1, 4d\n",
        ];
        for source in diagrams {
            let options = RenderOptions {
                theme: mermaid_rs_renderer::Theme::modern(),
                layout: LayoutConfig::default(),
            };
            let drawn = render_strict(source, options);
            assert!(
                drawn.as_ref().is_ok_and(|svg| svg.contains("<svg")),
                "{} didn't draw: {:?}",
                first_line(source),
                drawn.err()
            );
        }
    }

    /// SM-3
    #[test]
    fn a_syntax_error_says_where_it_is() {
        let options = || RenderOptions {
            theme: mermaid_rs_renderer::Theme::modern(),
            layout: LayoutConfig::default(),
        };
        let Err(error) = render_strict("flowchart TD\n  A --> B\n  --> C\n", options()) else {
            panic!("a dangling arrow should not parse");
        };
        assert_eq!(position(&error).0, Some(3));
        assert!(render_strict("flowchart TD\n  A --> B\n", options()).is_ok());
    }

    fn failed() -> DiagramEntry {
        DiagramEntry::Failed(Arc::new(DiagramError::Crashed))
    }

    #[test]
    fn a_zoomed_picture_is_kept_beside_the_base_and_found_again() {
        let cache = MermaidCache::default();
        let source: Arc<str> = Arc::from("graph TD; A-->B");
        let _ = cache.finish(source.clone(), 1, BASE_RASTER, failed());
        assert!(cache.begin(&source, 1, 4), "a zoom's bucket is its own picture");
        assert!(!cache.begin(&source, 1, 4));
        let _ = cache.finish(source.clone(), 1, 4, failed());
        assert!(cache.get(&source, 1, 4).is_some());
        assert!(cache.get(&source, 1, BASE_RASTER).is_some());
        assert!(cache.get(&source, 1, 8).is_none());
        assert_eq!(cache.stats().renders, 2);
    }

    #[test]
    fn only_a_few_zoomed_pictures_are_kept_and_the_bases_stay() {
        let cache = MermaidCache::default();
        let base: Arc<str> = Arc::from("base");
        let _ = cache.finish(base.clone(), 0, BASE_RASTER, failed());
        let sources: Vec<Arc<str>> = (0..=ZOOMED_CAPACITY).map(|ix| Arc::from(format!("zoomed {ix}"))).collect();
        for source in &sources {
            let _ = cache.finish(source.clone(), 0, 8, failed());
        }
        assert!(cache.get(&sources[0], 0, 8).is_none(), "the oldest zoomed picture went");
        for source in &sources[1..] {
            assert!(cache.get(source, 0, 8).is_some());
        }
        assert!(cache.get(&base, 0, BASE_RASTER).is_some(), "a base picture isn't dropped for zooms");
    }
}

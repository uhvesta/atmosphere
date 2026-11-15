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

/// The name of the custom block the text view draws diagrams with.
pub(crate) const NODE: &str = "mermaid";

/// How many diagrams a cache keeps; the least recently used goes first.
const CAPACITY: usize = 64;

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

/// Draw `source` with `theme`. Runs off the UI thread.
pub(crate) fn render(
    source: &str,
    theme: mermaid_rs_renderer::Theme,
    svg: &SvgRenderer,
) -> Result<Arc<RenderImage>, DiagramError> {
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
    // Twice the pixels, so the diagram stays sharp on a Retina screen.
    svg.render_single_frame(markup.as_bytes(), 2.0)
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
    Ready(Arc<RenderImage>),
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

type Key = (Arc<str>, u64);

#[derive(Default)]
struct Diagrams {
    entries: HashMap<Key, DiagramEntry>,
    /// Finished keys, least recently used first.
    order: VecDeque<Key>,
    stats: DiagramStats,
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

    pub(crate) fn get(&self, source: &Arc<str>, theme: u64) -> Option<DiagramEntry> {
        let mut diagrams = self.lock();
        let key = (source.clone(), theme);
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
    pub(crate) fn begin(&self, source: &Arc<str>, theme: u64) -> bool {
        let mut diagrams = self.lock();
        let key = (source.clone(), theme);
        if diagrams.entries.contains_key(&key) {
            return false;
        }
        diagrams.entries.insert(key, DiagramEntry::Rendering);
        true
    }

    pub(crate) fn finish(&self, source: Arc<str>, theme: u64, entry: DiagramEntry) {
        let mut diagrams = self.lock();
        let key = (source, theme);
        diagrams.stats.renders += 1;
        diagrams.entries.insert(key.clone(), entry);
        diagrams.order.push_back(key);
        while diagrams.order.len() > CAPACITY {
            if let Some(old) = diagrams.order.pop_front() {
                diagrams.entries.remove(&old);
            }
        }
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
        assert!(cache.get(&source, 1).is_none());
        assert!(cache.begin(&source, 1));
        assert!(!cache.begin(&source, 1));
        cache.finish(source.clone(), 1, DiagramEntry::Failed(Arc::new(DiagramError::Crashed)));
        assert!(matches!(cache.get(&source, 1), Some(DiagramEntry::Failed(_))));
        assert_eq!(cache.stats(), DiagramStats { renders: 1, hits: 1 });
        // Another theme is another picture.
        assert!(cache.begin(&source, 2));
    }

    /// SM-3
    #[test]
    fn the_least_recently_used_diagram_is_dropped_first() {
        let cache = MermaidCache::default();
        let keep: Arc<str> = Arc::from("keep");
        cache.finish(keep.clone(), 0, DiagramEntry::Failed(Arc::new(DiagramError::Crashed)));
        for ix in 0..CAPACITY {
            // Touch `keep` so it stays recent.
            cache.get(&keep, 0);
            let other: Arc<str> = Arc::from(format!("other {ix}"));
            cache.finish(other, 0, DiagramEntry::Failed(Arc::new(DiagramError::Crashed)));
        }
        assert!(cache.get(&keep, 0).is_some());
        assert!(cache.get(&Arc::from("other 0"), 0).is_none());
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
}

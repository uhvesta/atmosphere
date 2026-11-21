//! The view: gpui-kit's text view with diagrams, images, comments and a
//! rendered diff around it.
//!
//! gpui-kit's [`TextView`] does the Markdown: parsing (off the UI thread, and
//! append-only while text streams), virtualized layout, selection, copy and
//! code highlighting (gpui-component's highlighter, with the app's highlight
//! theme). This view adds what it doesn't have: Mermaid diagrams, image
//! loading confined to the document's folder, comments anchored to source,
//! diff marks, and semantic labels for every visible block.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use gpui_kit::assets::IconName;
use gpui_kit::base::{
    MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, RangeHighlight, RenderedText,
    SelectionFormat, TestSupportExt as _, TestTextExt as _, TextView, TextViewState, markdown_ast,
};
use gpui_kit::component::{Icon, Theme};
use gpui_kit::component::text::FrontmatterPlugin;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, ImageSource, InteractiveElement as _, IntoElement, KeyBinding, ListOffset, ParentElement as _,
    Pixels, Render, Role, SharedString, SharedUri, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, actions, canvas, div, px,
};

use crate::anchor::{AnchorStatus, AnchorTarget, Comment, SourceSelection};
use crate::diff::{BlockChange, RenderedDiff, diff};
use crate::document::{BlockKind, Document, LineRange, ParseStats, byline_text, follows_title};
use crate::image::{self, ImageCache, ImageEntry, ImageStats};
use crate::mermaid::{self, DiagramBlock, DiagramEntry, DiagramStats, MermaidCache};
use crate::theme::{MarkdownTheme, hex};

mod diagram_zoom;

use diagram_zoom::Zooms;

/// The key context of the view, for apps that bind its actions themselves.
pub const KEY_CONTEXT: &str = "MarkdownView";

actions!(
    markdown_view,
    [
        /// Copy the selection in the format the view doesn't copy by default.
        CopyOther,
        /// Ask to comment on the selection.
        CommentOnSelection,
        /// Switch between the rendered document and its source (SM-4).
        ToggleSource,
    ]
);

/// Copy the other format, comment on the selection, switch to the source:
/// ⌘ on the Mac, Ctrl elsewhere. Spelled out because `secondary-` only
/// parses where GPUI is told the OS, which `KeyBinding::new` isn't.
#[cfg(target_os = "macos")]
const KEYS: [&str; 3] = ["cmd-shift-c", "cmd-alt-m", "cmd-shift-u"];
#[cfg(not(target_os = "macos"))]
const KEYS: [&str; 3] = ["ctrl-shift-c", "ctrl-alt-m", "ctrl-shift-u"];

/// The view's key bindings; the text view's own (copy, select all) come with
/// gpui-kit.
pub fn key_bindings() -> Vec<KeyBinding> {
    let [copy_other, comment, toggle_source] = KEYS;
    vec![
        KeyBinding::new(copy_other, CopyOther, Some(KEY_CONTEXT)),
        KeyBinding::new(comment, CommentOnSelection, Some(KEY_CONTEXT)),
        KeyBinding::new(toggle_source, ToggleSource, Some(KEY_CONTEXT)),
    ]
}

/// What a copy puts on the clipboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CopyFormat {
    /// The selected Markdown source, exactly.
    #[default]
    Source,
    /// The text as the reader saw it, without markup.
    Plain,
}

impl CopyFormat {
    fn other(self) -> Self {
        match self {
            Self::Source => Self::Plain,
            Self::Plain => Self::Source,
        }
    }
}

/// Whether the view shows the rendered document or its source (SM-4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Rendered,
    Source,
}

/// How the view sizes itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Fill the parent, which must have a height, and scroll; only what is
    /// on screen is laid out.
    #[default]
    Scroll,
    /// Grow to fit the document, for a short message inside another list.
    /// Without its own scrolling the view can't tell where blocks are, so it
    /// draws no gutter, margin or per-block labels.
    Inline,
}

/// How the page shows where comments are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommentMarks {
    /// A margin column beside the text, one chip with the comment's words per
    /// comment.
    #[default]
    Notes,
    /// No margin column: each commented block is lit with a bar on its left
    /// and shows a count at its right end. Clicking the count reports the
    /// block's first comment.
    Counts,
}

/// How a view is set up.
#[derive(Clone, Debug)]
pub struct MarkdownOptions {
    /// What the document is, for the reader's assistive tools ("Plan", "Reply").
    pub label: SharedString,
    pub layout: Layout,
    /// What the copy shortcut copies; [`CopyOther`] copies the other format.
    pub copy: CopyFormat,
    /// The folder relative image links resolve in. Images outside it are not
    /// shown.
    pub base_dir: Option<PathBuf>,
    /// Whether `http(s)` images load. Off by default: a document then can't
    /// make the app fetch anything.
    pub allow_remote_images: bool,
    /// Whether the reader can comment: the gutter's buttons and the
    /// selection's Comment action.
    pub comments: bool,
    /// How the page shows the comments it has.
    pub marks: CommentMarks,
    /// Drawn diagrams; share one cache between views so a diagram shown in
    /// several places is drawn once.
    pub diagrams: MermaidCache,
    /// The file name, when the view is a document tab's page: the page then
    /// has the design's document typography, and a paragraph that is one
    /// emphasis span directly under the title is drawn as a byline ending in
    /// this name. `None` for chat and everything else.
    pub document: Option<SharedString>,
}

impl MarkdownOptions {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            layout: Layout::Scroll,
            copy: CopyFormat::Source,
            base_dir: None,
            allow_remote_images: false,
            comments: true,
            marks: CommentMarks::Notes,
            diagrams: MermaidCache::default(),
            document: None,
        }
    }
}

impl std::fmt::Debug for MermaidCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MermaidCache").field("stats", &self.stats()).finish()
    }
}

/// What the view tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownEvent {
    /// The reader clicked a link. The view opens nothing itself.
    LinkActivated { url: SharedString },
    /// The reader asked to comment on this passage (SM-6). Make a
    /// [`Comment`] from it with [`crate::Anchor::new`] and pass it back
    /// through [`MarkdownView::set_comments`].
    CommentRequested(SourceSelection),
    /// The reader clicked a comment's margin marker.
    CommentActivated { id: String },
    /// The reader wants to attach a comment whose passage is gone to the
    /// current selection (SM-7). The selection comes with it when there is one.
    ReattachRequested { id: String, selection: Option<SourceSelection> },
    /// The reader dropped a comment whose passage is gone (SM-7).
    DropRequested { id: String },
    /// The mode changed, by the reader or the app.
    ModeChanged(ViewMode),
}

/// A heading, for a table of contents (SM-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry {
    pub level: u8,
    pub title: String,
    /// The heading's block; pass it to [`MarkdownView::reveal_block`].
    pub block: usize,
    pub lines: LineRange,
}

/// Counters that show what the view reused, for tests and benchmarks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewStats {
    /// What the last source change parsed.
    pub parse: ParseStats,
    pub diagrams: DiagramStats,
    pub images: ImageStats,
    /// How often comment and diff highlights were handed to the text view;
    /// unchanged text and marks hand nothing.
    pub mark_syncs: u64,
    /// Blocks labelled in the last frame.
    pub labelled_blocks: usize,
    /// Whether the text view's blocks lined up with the document's in the
    /// last frame; when they don't, blocks go unlabelled rather than
    /// mislabelled.
    pub blocks_aligned: bool,
}

/// What diagrams and placeholders are drawn with.
struct Look {
    theme: MarkdownTheme,
    mermaid: mermaid_rs_renderer::Theme,
    mermaid_key: u64,
}

impl Look {
    fn new(theme: MarkdownTheme) -> Self {
        let mermaid = theme.mermaid();
        let mermaid_key = mermaid::theme_key(&mermaid);
        Self {
            theme,
            mermaid,
            mermaid_key,
        }
    }
}

struct ImageSettings {
    base: Option<PathBuf>,
    allow_remote: bool,
}

/// What the text view's callbacks, which must be `Send + Sync`, share with
/// the view.
struct Shared {
    view: WeakEntity<MarkdownView>,
    look: RwLock<Look>,
    diagrams: MermaidCache,
    images: ImageCache,
    image_settings: RwLock<ImageSettings>,
    /// The first byte of each diagram block a comment is on, for its frame.
    commented_diagrams: RwLock<HashSet<usize>>,
    /// How far each diagram is zoomed, and the one opened large.
    zooms: Zooms,
}

/// A poisoned lock still holds whole data here; keep using it.
fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

impl Shared {
    fn resource_ready(view: &WeakEntity<MarkdownView>, cx: &mut gpui_kit::AsyncApp) {
        // The view may be gone by the time its picture is; nothing to do then.
        let _ = view.update(cx, |view, cx| view.resource_ready(cx));
    }

    fn render_diagram(self: &Arc<Self>, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(block) = node.data::<DiagramBlock>() else {
            return div().into_any_element();
        };
        let look = read(&self.look);
        let theme = &look.theme;
        let name = mermaid::first_line(&block.source).to_owned();
        let commented = read(&self.commented_diagrams).contains(&block.bytes.start);
        let start = block.bytes.start;
        let border = if commented { theme.comment.opacity(1.) } else { theme.border };
        // The frame, on a div that may already measure its children.
        let framed = move |inner: gpui_kit::Div| {
            inner
                .id(("md-diagram", start))
                .test_support()
                .my_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(border)
                .when(commented, |frame| frame.border_2())
        };
        let frame = framed(div());
        let entry = if block.closed {
            self.diagrams.get(&block.source, look.mermaid_key, crate::zoom::BASE_RASTER)
        } else {
            None
        };
        match entry {
            Some(DiagramEntry::Ready(drawn)) => {
                // The zoom reads the look itself.
                drop(look);
                drop(frame);
                self.render_zoomable(block, &name, &drawn, framed, window, cx)
                    .role(Role::Image)
                    .aria_label(format!("Diagram: {name}"))
                    .into_any_element()
            }
            Some(DiagramEntry::Failed(error)) => {
                let message = match error.as_ref() {
                    crate::error::DiagramError::Syntax {
                        line: Some(line),
                        column,
                        ..
                    } => match column {
                        Some(column) => format!("Line {line}, column {column}: {error}"),
                        None => format!("Line {line}: {error}"),
                    },
                    _ => error.to_string(),
                };
                frame
                    .role(Role::Alert)
                    .aria_label(format!("Diagram error: {name}"))
                    .border_color(theme.error)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_color(theme.error).child(message.clone()).test_text(message))
                    .child(source_listing(&block.source, theme))
                    .into_any_element()
            }
            _ => {
                if block.closed && self.diagrams.begin(&block.source, look.mermaid_key, crate::zoom::BASE_RASTER) {
                    self.start_diagram(block.source.clone(), look.mermaid.clone(), look.mermaid_key, cx);
                }
                let shown = if block.closed {
                    format!("Drawing diagram: {name}")
                } else {
                    format!("Diagram arriving: {name}")
                };
                frame
                    .role(Role::Status)
                    .aria_label(shown.clone())
                    .min_h(px(96.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_color(theme.muted)
                    .child(div().child(shown.clone()).test_text(shown))
                    .into_any_element()
            }
        }
    }

    fn start_diagram(&self, source: Arc<str>, theme: mermaid_rs_renderer::Theme, key: u64, cx: &mut App) {
        let svg = cx.svg_renderer();
        let drawing = {
            let source = source.clone();
            cx.background_executor()
                .spawn(async move { mermaid::render(&source, theme, &svg) })
        };
        let cache = self.diagrams.clone();
        let view = self.view.clone();
        cx.spawn(async move |cx| {
            let entry = match drawing.await {
                Ok(image) => DiagramEntry::Ready(image),
                Err(error) => DiagramEntry::Failed(Arc::new(error)),
            };
            let evicted = cache.finish(source, key, crate::zoom::BASE_RASTER, entry);
            cx.update(|cx| {
                for image in evicted {
                    cx.drop_image(image, None);
                }
            });
            Self::resource_ready(&view, cx);
        })
        .detach();
    }

    fn image_source(self: &Arc<Self>, uri: &SharedUri) -> ImageSource {
        let url = uri.to_string();
        match self.images.get(&url) {
            Some(ImageEntry::Ready(image)) => return ImageSource::Render(image),
            Some(ImageEntry::Missing {
                placeholder: Some(placeholder),
                ..
            }) => return ImageSource::Render(placeholder),
            Some(_) => return ImageSource::Custom(Arc::new(|_, _| None)),
            None => {}
        }
        let remote = {
            let settings = read(&self.image_settings);
            matches!(
                image::locate(&url, settings.base.as_deref(), settings.allow_remote),
                Ok(image::ImageLocation::Remote(_))
            )
        };
        if remote {
            return ImageSource::from(uri.clone());
        }
        let shared = self.clone();
        ImageSource::Custom(Arc::new(move |_, cx| {
            shared.start_image(&url, cx);
            None
        }))
    }

    fn start_image(&self, url: &str, cx: &mut App) {
        if !self.images.begin(url) {
            return;
        }
        let location = {
            let settings = read(&self.image_settings);
            image::locate(url, settings.base.as_deref(), settings.allow_remote)
        };
        let (foreground, border, font) = {
            let look = read(&self.look);
            (hex(look.theme.muted), hex(look.theme.border), look.theme.sans.to_string())
        };
        let name = url
            .split(['?', '#'])
            .next()
            .and_then(|path| path.rsplit('/').next())
            .unwrap_or(url)
            .to_owned();
        let svg = cx.svg_renderer();
        let loading = cx.background_executor().spawn(async move {
            let loaded = location.and_then(|location| image::load(&location, &name, &svg));
            match loaded {
                Ok(image) => ImageEntry::Ready(image),
                Err(error) => {
                    let placeholder = image::placeholder_svg(&name, &foreground, &border, &font);
                    ImageEntry::Missing {
                        placeholder: svg.render_single_frame(placeholder.as_bytes(), 2.0).ok(),
                        error: Arc::new(error),
                    }
                }
            }
        });
        let cache = self.images.clone();
        let view = self.view.clone();
        let url = url.to_owned();
        cx.spawn(async move |cx| {
            let entry = loading.await;
            cache.finish(&url, entry, true);
            Self::resource_ready(&view, cx);
        })
        .detach();
    }
}

/// A diagram's source, shown under its error.
fn source_listing(source: &str, theme: &MarkdownTheme) -> impl IntoElement {
    div()
        .p_2()
        .rounded_sm()
        .bg(theme.code_background)
        .font_family(theme.mono.clone())
        .text_size(theme.mono_size)
        .whitespace_normal()
        .children(source.lines().map(|line| div().child(line.to_owned())))
        .test_text(source.to_owned())
}

/// Fenced `mermaid` blocks, as diagrams.
struct DiagramPlugin {
    shared: Arc<Shared>,
}

impl MarkdownPlugin for DiagramPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        mermaid::NODE
    }

    fn parse(&self, node: &markdown_ast::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        mermaid::parse(node, cx)
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.shared.render_diagram(node, window, cx)
    }
}

/// A document page's byline: the emphasis paragraph under the title, in the
/// secondary colour and upright, followed by the file's name.
struct BylinePlugin {
    shared: Arc<Shared>,
    file: SharedString,
}

impl MarkdownPlugin for BylinePlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "byline"
    }

    fn parse(&self, node: &markdown_ast::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        if !matches!(node, markdown_ast::Node::Paragraph(_)) {
            return None;
        }
        let start = node.position()?.start.offset;
        if !follows_title(cx.source().get(..start)?) {
            return None;
        }
        let raw = cx.node_source(node)?;
        let text = byline_text(raw)?;
        Some(
            MarkdownNode::new("byline", text.clone())
                .text(text.clone())
                .markdown(raw.to_owned())
                .accessibility_label(text),
        )
    }

    fn render(&self, node: &MarkdownNode, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let text = node.data::<String>().cloned().unwrap_or_default();
        let shown = format!("{text} \u{b7} {}", self.file);
        let look = read(&self.shared.look);
        div()
            .text_color(look.theme.muted)
            .text_size(px(11.5))
            .line_height(px(20.))
            .child(shown.clone())
            .test_text(shown)
    }
}

/// The room a lit list item keeps at its end for its count: 8 before the
/// chip, the chip's icon, gap and digits, and 4 after it.
fn count_room(count: usize) -> Pixels {
    px(26. + 6.5 * count.to_string().len() as f32)
}

/// The recorded bounds of the list item whose source starts where `span` does.
fn item_at(items: &[(Range<usize>, Bounds<Pixels>)], span: &Range<usize>) -> Option<Bounds<Pixels>> {
    items.iter().find(|(item, _)| item.start == span.start).map(|(_, bounds)| *bounds)
}

/// Visible blocks and where they are on screen.
type Placed = Vec<(usize, Bounds<Pixels>)>;

/// A Markdown document on screen.
pub struct MarkdownView {
    options: MarkdownOptions,
    theme: MarkdownTheme,
    /// Whether the theme follows the app's; false once the app sets one.
    follows_app_theme: bool,
    document: Document,
    text: Entity<TextViewState>,
    /// The source as a code block, made the first time it is shown.
    source_text: Option<Entity<TextViewState>>,
    /// The selection the toolbar was last drawn for.
    toolbar_selection: Option<SourceSelection>,
    mode: ViewMode,
    extensions: MarkdownExtensions,
    shared: Arc<Shared>,
    comments: Vec<Comment>,
    statuses: Vec<AnchorStatus>,
    diff_base: Option<Document>,
    diff: Option<RenderedDiff>,
    /// Bumped when comments or diff marks change.
    marks_generation: u64,
    /// The rendered text and marks last handed to the text view.
    synced: Option<(RenderedText, u64)>,
    stats: ViewStats,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MarkdownEvent> for MarkdownView {}

impl MarkdownView {
    /// A view of `source`, in the app's current theme.
    pub fn new(source: &str, options: MarkdownOptions, cx: &mut Context<Self>) -> Self {
        let theme = Self::app_theme(&options, cx);
        let shared = Arc::new(Shared {
            view: cx.weak_entity(),
            look: RwLock::new(Look::new(theme.clone())),
            diagrams: options.diagrams.clone(),
            images: ImageCache::default(),
            image_settings: RwLock::new(ImageSettings {
                base: options.base_dir.clone(),
                allow_remote: options.allow_remote_images,
            }),
            commented_diagrams: RwLock::new(HashSet::new()),
            zooms: Zooms::default(),
        });
        let mut extensions = MarkdownExtensions::default()
            .frontmatter()
            .plugin(FrontmatterPlugin::new())
            .plugin(DiagramPlugin {
                shared: shared.clone(),
            });
        if let Some(file) = options.document.clone() {
            extensions = extensions.plugin(BylinePlugin {
                shared: shared.clone(),
                file,
            });
        }
        let text = cx.new(|cx| TextViewState::markdown(source, cx));
        let subscriptions = vec![
            cx.observe(&text, |view, _, cx| view.sync_marks(cx)),
            cx.observe_global::<Theme>(|view, cx| {
                if view.follows_app_theme {
                    let theme = MarkdownView::app_theme(&view.options, cx);
                    view.apply_theme(theme, cx);
                }
            }),
        ];
        let document = Document::new(source);
        let mut stats = ViewStats {
            parse: document.stats(),
            blocks_aligned: true,
            ..ViewStats::default()
        };
        stats.diagrams = options.diagrams.stats();
        Self {
            options,
            theme,
            follows_app_theme: true,
            document,
            text,
            source_text: None,
            toolbar_selection: None,
            mode: ViewMode::Rendered,
            extensions,
            shared,
            comments: Vec::new(),
            statuses: Vec::new(),
            diff_base: None,
            diff: None,
            marks_generation: 0,
            synced: None,
            stats,
            _subscriptions: subscriptions,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Replace the source. A source that extends the current one is appended,
    /// so streaming through this re-parses only the end.
    pub fn set_source(&mut self, source: &str, cx: &mut Context<Self>) {
        self.document.set_source(source);
        self.text.update(cx, |text, cx| text.set_text(source, cx));
        self.source_changed(cx);
    }

    /// Append streamed text (SM-3).
    pub fn append(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.document.append(text);
        self.text.update(cx, |state, cx| state.push_str(text, cx));
        self.source_changed(cx);
    }

    fn source_changed(&mut self, cx: &mut Context<Self>) {
        self.stats.parse = self.document.stats();
        if let Some(source_text) = &self.source_text {
            let fenced = fenced(self.document.source());
            source_text.update(cx, |state, cx| state.set_text(&fenced, cx));
        }
        self.refresh_marks();
        cx.notify();
    }

    /// The app's theme, in the document look when the view is a document page.
    fn app_theme(options: &MarkdownOptions, cx: &App) -> MarkdownTheme {
        let theme = MarkdownTheme::from_app(cx);
        if options.document.is_some() { theme.document() } else { theme }
    }

    /// Use `theme` from now on instead of following the app's.
    pub fn set_theme(&mut self, theme: MarkdownTheme, cx: &mut Context<Self>) {
        self.follows_app_theme = false;
        self.apply_theme(theme, cx);
    }

    fn apply_theme(&mut self, theme: MarkdownTheme, cx: &mut Context<Self>) {
        if theme == self.theme {
            return;
        }
        self.theme = theme.clone();
        write(&self.shared.look, Look::new(theme));
        // Placeholders are drawn in the theme's colours.
        self.shared.images.clear();
        self.marks_generation += 1;
        self.resource_ready(cx);
    }

    /// Resolve relative image links in `base_dir` from now on.
    pub fn set_base_dir(&mut self, base_dir: Option<PathBuf>, cx: &mut Context<Self>) {
        write(
            &self.shared.image_settings,
            ImageSettings {
                base: base_dir.clone(),
                allow_remote: self.options.allow_remote_images,
            },
        );
        self.options.base_dir = base_dir;
        self.shared.images.clear();
        self.resource_ready(cx);
    }

    /// Show these comments. Each is drawn where its anchor resolves now;
    /// one whose passage changed is marked outdated, and one whose passage is
    /// gone is listed above the document with Re-attach and Drop (SM-7, SM-8).
    pub fn set_comments(&mut self, comments: Vec<Comment>, cx: &mut Context<Self>) {
        self.comments = comments;
        self.refresh_marks();
        cx.notify();
    }

    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Where each comment is now, in the order of [`Self::comments`].
    pub fn comment_statuses(&self) -> &[AnchorStatus] {
        &self.statuses
    }

    /// Mark the document against an older version of it: inserted and
    /// changed blocks in the gutter and as highlights, deleted blocks as
    /// markers where they were (SM-5). `None` clears the marks.
    pub fn set_diff_base(&mut self, old: Option<&str>, cx: &mut Context<Self>) {
        self.diff_base = old.map(Document::new);
        self.refresh_marks();
        cx.notify();
    }

    pub fn diff(&self) -> Option<&RenderedDiff> {
        self.diff.as_ref()
    }

    fn refresh_marks(&mut self) {
        self.statuses = self
            .comments
            .iter()
            .map(|comment| comment.anchor.resolve(&self.document))
            .collect();
        self.diff = self.diff_base.as_ref().map(|old| diff(old, &self.document));
        let diagrams: HashSet<usize> = self
            .statuses
            .iter()
            .filter_map(AnchorStatus::bytes)
            .filter_map(|bytes| {
                let block = &self.document.blocks()[self.document.block_at(bytes.start)?];
                matches!(block.kind, BlockKind::Mermaid { .. }).then_some(block.bytes.start)
            })
            .collect();
        write(&self.shared.commented_diagrams, diagrams);
        let starts: HashSet<usize> = self
            .document
            .blocks()
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Mermaid { .. }))
            .map(|block| block.bytes.start)
            .collect();
        self.shared.zooms.retain(&starts);
        self.marks_generation += 1;
        self.synced = None;
    }

    /// Hand comment and diff highlights to the text view, once its parse of
    /// the current source has landed.
    fn sync_marks(&mut self, cx: &mut Context<Self>) {
        let rendered = self.text.read(cx).rendered_text();
        if rendered.source() != self.document.source() {
            return;
        }
        let key = (rendered, self.marks_generation);
        if self.synced.as_ref() == Some(&key) {
            return;
        }
        let highlights = self.highlights(&key.0);
        self.synced = Some(key);
        self.stats.mark_syncs += 1;
        // Every range came from this rendered text, so the set is valid.
        let _ = self
            .text
            .update(cx, |state, cx| state.set_range_highlights(highlights, cx));
    }

    fn highlights(&self, rendered: &RenderedText) -> Vec<RangeHighlight> {
        let mut out = Vec::new();
        let mut push = |bytes: Range<usize>, color: Hsla| {
            if let Some(range) = rendered.range_for_source(bytes) {
                out.push(RangeHighlight::new(range, color));
            }
        };
        if let Some(diff) = &self.diff {
            for (block, change) in self.document.blocks().iter().zip(&diff.blocks) {
                match change {
                    BlockChange::Inserted => push(block.bytes.clone(), self.theme.inserted.opacity(0.14)),
                    BlockChange::Changed { inserted, .. } => {
                        for range in inserted {
                            push(range.clone(), self.theme.inserted.opacity(0.28));
                        }
                    }
                    BlockChange::Unchanged { .. } => {}
                }
            }
        }
        for status in &self.statuses {
            match status {
                AnchorStatus::Current { bytes, .. } => push(bytes.clone(), self.theme.comment),
                AnchorStatus::Outdated { bytes, .. } => push(bytes.clone(), self.theme.comment_outdated),
                AnchorStatus::Gone => {}
            }
        }
        out.sort_by_key(|highlight| highlight.range().start);
        out
    }

    /// Called when a diagram or image finishes: lay its block out again.
    fn resource_ready(&mut self, cx: &mut Context<Self>) {
        self.text.update(cx, |state, cx| state.invalidate_inline_layout(cx));
        if let Some(source_text) = &self.source_text {
            source_text.update(cx, |state, cx| state.invalidate_inline_layout(cx));
        }
        cx.notify();
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    /// Show the rendered document or its source (SM-4). Comments, diff marks
    /// and the gutter belong to the rendered document; the source shows the
    /// text only, and selecting in it still maps to source for comments.
    pub fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        if mode == ViewMode::Source && self.source_text.is_none() {
            let fenced = fenced(self.document.source());
            self.source_text = Some(cx.new(|cx| TextViewState::markdown(&fenced, cx)));
        }
        cx.emit(MarkdownEvent::ModeChanged(mode));
        cx.notify();
    }

    fn toggle_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.mode {
            ViewMode::Rendered => ViewMode::Source,
            ViewMode::Source => ViewMode::Rendered,
        };
        let had_focus = self.active_text().read(cx).focus_handle().contains_focused(window, cx);
        self.set_mode(next, cx);
        // The text view that had focus is no longer drawn; keep the keyboard
        // on the view so the shortcut can switch back.
        if had_focus {
            let handle = self.active_text().read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        }
    }

    fn active_text(&self) -> &Entity<TextViewState> {
        match (&self.mode, &self.source_text) {
            (ViewMode::Source, Some(source_text)) => source_text,
            _ => &self.text,
        }
    }

    /// The document's headings, for a table of contents (SM-4).
    pub fn outline(&self) -> Vec<OutlineEntry> {
        self.document
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(block, found)| match found.kind {
                BlockKind::Heading { level } => Some(OutlineEntry {
                    level,
                    title: found.text.trim().to_owned(),
                    block,
                    lines: found.lines,
                }),
                _ => None,
            })
            .collect()
    }

    /// Scroll so block `index` is at the top.
    pub fn reveal_block(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.mode != ViewMode::Rendered {
            self.set_mode(ViewMode::Rendered, cx);
        }
        let list = self.text.read(cx).list_state().clone();
        if index < list.item_count() {
            list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    /// Scroll to the passage of the comment `id`, when it has one.
    pub fn reveal_comment(&mut self, id: &str, cx: &mut Context<Self>) {
        let found = self
            .comments
            .iter()
            .zip(&self.statuses)
            .find(|(comment, _)| comment.id == id)
            .and_then(|(_, status)| status.bytes());
        if let Some(block) = found.and_then(|bytes| self.document.block_at(bytes.start)) {
            self.reveal_block(block, cx);
        }
    }

    /// Whether a drag is selecting text now. The selection toolbar waits
    /// for it to end.
    pub fn is_selecting(&self, cx: &App) -> bool {
        self.active_text().read(cx).is_selecting()
    }

    /// The selection, mapped to source (SM-6). Inside one table cell it
    /// targets the cell, inside one list item the item, a whole block the
    /// block, and a diagram the diagram.
    pub fn selection(&self, cx: &App) -> Option<SourceSelection> {
        let state = self.active_text().read(cx);
        let selected = state.selected_source_range()?;
        if self.mode == ViewMode::Source {
            let prefix = fence_prefix(self.document.source()).len();
            let start = selected.start.saturating_sub(prefix).min(self.document.source().len());
            let end = selected.end.saturating_sub(prefix).min(self.document.source().len());
            let quote = self.document.source().get(start..end)?.to_owned();
            return (start < end).then(|| self.document.selection(start..end, quote)).flatten();
        }
        if selected.is_empty() {
            return None;
        }
        let rendered = state.rendered_text();
        let quote = rendered
            .range_for_source(selected.clone())
            .and_then(|range| rendered.as_str().get(range))
            .unwrap_or_default()
            .to_owned();
        let selected = self.with_block_markup(selected);
        let blocks = self.document.blocks_in(&selected);
        if blocks.len() == 1 {
            let index = blocks.start;
            let block = &self.document.blocks()[index];
            if matches!(block.kind, BlockKind::Mermaid { .. }) {
                return self.document.block_selection(index);
            }
            if quote.trim() == block.text.trim() {
                let mut selection = self.document.selection(selected, quote)?;
                selection.target = AnchorTarget::Block { index };
                return Some(selection);
            }
            let part = block.parts.iter().position(|part| {
                let bytes = part.bytes();
                bytes.start <= selected.start && selected.end <= bytes.end
            });
            if let Some(part) = part {
                let target = self.document.part_selection(index, part)?.target;
                let mut selection = self.document.selection(selected, quote)?;
                selection.target = target;
                return Some(selection);
            }
        }
        self.document.selection(selected, quote)
    }

    /// `selected` grown to take in the markup that opens its first block and
    /// closes its last one, when the selection starts at the first character
    /// a reader sees or ends at the last: a selection from the start of a
    /// heading copies as `# Heading`, not `Heading`.
    fn with_block_markup(&self, selected: Range<usize>) -> Range<usize> {
        let source = self.document.source();
        let blocks = self.document.blocks_in(&selected);
        let only_markup = |text: &str| {
            text.chars()
                .all(|ch| ch.is_whitespace() || "#>-*+_~`|[]xX.)0123456789".contains(ch))
        };
        let mut grown = selected.clone();
        if let Some(first) = self.document.blocks().get(blocks.start)
            && first.bytes.start < selected.start
            && source.get(first.bytes.start..selected.start).is_some_and(only_markup)
        {
            grown.start = first.bytes.start;
        }
        if let Some(last) = blocks.end.checked_sub(1).and_then(|index| self.document.blocks().get(index))
            && selected.end < last.bytes.end
            && source.get(selected.end..last.bytes.end).is_some_and(only_markup)
        {
            grown.end = last.bytes.end;
        }
        grown
    }

    /// The selection as `format`, or `None` when nothing is selected.
    pub fn selected_text(&self, format: CopyFormat, cx: &App) -> Option<String> {
        let selection = self.selection(cx)?;
        Some(match format {
            CopyFormat::Source => selection.source,
            CopyFormat::Plain => selection.quote,
        })
    }

    /// Copy the selection as `format`.
    pub fn copy(&self, format: CopyFormat, cx: &mut App) {
        if let Some(text) = self.selected_text(format, cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Ask the owner to comment on the selection (SM-6). Only the source's
    /// lines take a comment: in the rendered page this does nothing.
    pub fn request_selection_comment(&mut self, cx: &mut Context<Self>) {
        if !self.options.comments || self.mode != ViewMode::Source {
            return;
        }
        if let Some(selection) = self.selection(cx) {
            cx.emit(MarkdownEvent::CommentRequested(selection));
        }
    }

    /// Ask the owner to comment on block `index`: a diagram, table or any
    /// other block as a whole.
    pub fn request_block_comment(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.options.comments {
            return;
        }
        if let Some(selection) = self.document.block_selection(index) {
            cx.emit(MarkdownEvent::CommentRequested(selection));
        }
    }

    /// Ask the owner to comment on one list item or table cell of a block,
    /// as [`crate::Block::parts`] lists them.
    pub fn request_part_comment(&mut self, block: usize, part: usize, cx: &mut Context<Self>) {
        if !self.options.comments {
            return;
        }
        if let Some(selection) = self.document.part_selection(block, part) {
            cx.emit(MarkdownEvent::CommentRequested(selection));
        }
    }

    pub fn stats(&self) -> ViewStats {
        ViewStats {
            diagrams: self.shared.diagrams.stats(),
            images: self.shared.images.stats(),
            ..self.stats
        }
    }

    /// The visible blocks and where they are, from the last layout.
    fn visible_blocks(&self, cx: &App) -> (Bounds<Pixels>, Placed, bool) {
        let state = self.text.read(cx);
        let view = state.bounds();
        let list = state.list_state();
        let count = list.item_count();
        if count != self.document.blocks().len() {
            return (view, Vec::new(), count == 0);
        }
        let mut placed = Vec::new();
        let mut index = list.logical_scroll_top().item_ix;
        while index < count {
            let Some(bounds) = list.bounds_for_item(index) else { break };
            if bounds.top() > view.bottom() {
                break;
            }
            if bounds.bottom() >= view.top() {
                placed.push((index, bounds));
            }
            index += 1;
        }
        (view, placed, true)
    }

    /// Each commented block, or list item in one: (block, item in a list
    /// block, first comment's id, how many, all outdated).
    fn comment_targets(&self) -> Vec<(usize, Option<usize>, String, usize, bool)> {
        let mut per_target: Vec<(usize, Option<usize>, String, usize, bool)> = Vec::new();
        for (comment, status) in self.comments.iter().zip(&self.statuses) {
            let (outdated, bytes) = match status {
                AnchorStatus::Current { bytes, .. } => (false, bytes),
                AnchorStatus::Outdated { bytes, .. } => (true, bytes),
                AnchorStatus::Gone => continue,
            };
            let Some(block) = self.document.block_at(bytes.start) else { continue };
            let item = self.document.item_at_line(block, self.document.line_of(bytes.start));
            match per_target.iter_mut().find(|entry| entry.0 == block && entry.1 == item) {
                Some(entry) => {
                    entry.3 += 1;
                    entry.4 &= outdated;
                }
                None => per_target.push((block, item, comment.id.clone(), 1, outdated)),
            }
        }
        per_target
    }

    fn render_text(&self, cx: &mut Context<Self>) -> TextView {
        let view = cx.weak_entity();
        let shared = self.shared.clone();
        let format = match self.options.copy {
            CopyFormat::Source => SelectionFormat::Source,
            CopyFormat::Plain => SelectionFormat::Plain,
        };
        let mut style = self.theme.text_view_style();
        if self.options.marks == CommentMarks::Counts {
            // A commented list item keeps room at its end for its count.
            let ends = self
                .comment_targets()
                .into_iter()
                .filter_map(|(block, item, _, count, _)| {
                    let part = self.document.blocks().get(block)?.parts.get(item?)?;
                    Some((part.bytes().start, count_room(count)))
                })
                .collect();
            style = style.with_list_item_ends(ends);
        }
        TextView::new(self.active_text())
            .style(style)
            .shared_code_block_highlighter(humata2_highlight::shared_code_block_highlighter(self.theme.dark))
            .selectable(true)
            .selection_format(format)
            .scrollable(self.options.layout == Layout::Scroll)
            .markdown_extensions(self.extensions.clone())
            .image_source(move |uri| shared.image_source(uri))
            .on_link_click(move |url, _, _, cx| {
                let url = url.clone();
                let _ = view.update(cx, |_, cx| cx.emit(MarkdownEvent::LinkActivated { url }));
            })
    }

    fn block_label(&self, index: usize) -> Option<(Role, String, String)> {
        let block = self.document.blocks().get(index)?;
        let number = index + 1;
        let (role, name) = match &block.kind {
            BlockKind::Heading { level } => (Role::Heading, format!("Heading {number}, level {level}")),
            BlockKind::Paragraph => (Role::Paragraph, format!("Paragraph {number}")),
            BlockKind::Image => {
                let url = block.images.first().map(|image| image.url.as_str()).unwrap_or_default();
                return Some(match self.shared.images.peek(url) {
                    Some(ImageEntry::Missing { error, .. }) => {
                        (Role::Image, format!("Missing image {number}"), error.to_string())
                    }
                    _ => {
                        let alt = block.images.first().map(|image| image.alt.clone()).unwrap_or_default();
                        (Role::Image, format!("Image {number}"), alt)
                    }
                });
            }
            BlockKind::List { tasks: true, .. } => (Role::List, format!("Task list {number}")),
            BlockKind::List { .. } => (Role::List, format!("List {number}")),
            BlockKind::Quote => (Role::Blockquote, format!("Quote {number}")),
            BlockKind::Table => (Role::Table, format!("Table {number}")),
            BlockKind::Code { .. } => (Role::Code, format!("Code block {number}")),
            BlockKind::Math => (Role::GenericContainer, format!("Formula {number}")),
            BlockKind::Footnote => (Role::Note, format!("Footnote {number}")),
            BlockKind::Frontmatter => (Role::GenericContainer, format!("Front matter {number}")),
            BlockKind::Html | BlockKind::Other => (Role::GenericContainer, format!("Block {number}")),
            // A diagram labels itself; a rule and a definition have nothing to read.
            BlockKind::Mermaid { .. } | BlockKind::Rule | BlockKind::Definition => return None,
        };
        Some((role, name, block.text.clone()))
    }

    fn render_rendered(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (view_bounds, placed, aligned) = self.visible_blocks(cx);
        self.stats.blocks_aligned = aligned;
        self.stats.labelled_blocks = 0;
        let origin = view_bounds.origin;
        let theme = self.theme.clone();
        let top = |bounds: &Bounds<Pixels>| bounds.top() - origin.y;
        let left = |bounds: &Bounds<Pixels>| bounds.left() - origin.x;

        // Labels over each visible block, which pass the mouse through.
        let mut labels = Vec::new();
        for (index, bounds) in &placed {
            let Some((role, name, text)) = self.block_label(*index) else { continue };
            self.stats.labelled_blocks += 1;
            labels.push(
                div()
                    .id(("md-block", *index))
                    .test_support()
                    .role(role)
                    .aria_label(name)
                    .absolute()
                    .top(top(bounds))
                    .left(left(bounds))
                    .w(bounds.size.width)
                    .h(bounds.size.height)
                    .child(div().size_full().test_text(text))
                    .into_any_element(),
            );
        }

        // The gutter: diff bars, deleted markers and Comment buttons.
        let mut gutter = Vec::new();
        let blocks = self.document.blocks();
        for (index, bounds) in &placed {
            let Some(block) = blocks.get(*index) else { continue };
            let kind = block.kind.name();
            let number = index + 1;
            let change = self.diff.as_ref().and_then(|diff| diff.blocks.get(*index));
            let bar = match change {
                Some(BlockChange::Inserted) => Some((format!("Inserted {kind} {number}"), theme.inserted)),
                Some(BlockChange::Changed { .. }) => Some((format!("Changed {kind} {number}"), theme.changed)),
                _ => None,
            };
            if let Some((label, color)) = bar {
                gutter.push(
                    div()
                        .id(("md-change", *index))
                        .test_support()
                        .role(Role::Mark)
                        .aria_label(label)
                        .absolute()
                        .top(top(bounds))
                        .right(px(0.))
                        .w(px(3.))
                        .h(bounds.size.height)
                        .bg(color)
                        .into_any_element(),
                );
            }
            if let Some(diff) = &self.diff {
                let last = *index + 1 == blocks.len();
                for (at, deleted) in diff.deleted.iter().enumerate() {
                    let here = deleted.before == *index;
                    let after_last = last && deleted.before >= blocks.len();
                    if !here && !after_last {
                        continue;
                    }
                    let y = if here { top(bounds) } else { top(bounds) + bounds.size.height };
                    let gist: String = deleted.text.trim().chars().take(60).collect();
                    gutter.push(
                        div()
                            .id(("md-deleted", at))
                            .test_support()
                            .role(Role::Mark)
                            .aria_label(format!("Deleted {}: {gist}", deleted.kind.name()))
                            .absolute()
                            .top(y - px(4.))
                            .right(px(0.))
                            .w(px(10.))
                            .h(px(8.))
                            .rounded_sm()
                            .bg(theme.deleted)
                            .child(div().size_full().test_text(deleted.source.clone()))
                            .into_any_element(),
                    );
                }
            }
            if self.options.comments && !matches!(block.kind, BlockKind::Definition) {
                let index = *index;
                gutter.push(
                    div()
                        .id(("md-comment-block", index))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(format!("Comment on {kind} {number}"))
                        .absolute()
                        .top(top(bounds))
                        .left(px(2.))
                        .w(px(16.))
                        .h(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .text_color(theme.muted.opacity(0.5))
                        .hover(|button| button.text_color(theme.foreground).bg(theme.surface))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.request_block_comment(index, cx)
                        }))
                        .child("+")
                        .into_any_element(),
                );
            }
        }

        // The margin: one marker per comment on a visible block (SM-8).
        let notes = self.options.marks == CommentMarks::Notes;
        let mut margin = Vec::new();
        let mut stacked: Vec<(usize, usize)> = Vec::new();
        for (comment, status) in self.comments.iter().zip(&self.statuses) {
            if !notes {
                break;
            }
            let (outdated, bytes) = match status {
                AnchorStatus::Current { bytes, .. } => (false, bytes),
                AnchorStatus::Outdated { bytes, .. } => (true, bytes),
                AnchorStatus::Gone => continue,
            };
            let Some(block) = self.document.block_at(bytes.start) else { continue };
            let Some((_, bounds)) = placed.iter().find(|(index, _)| *index == block) else { continue };
            let slot = match stacked.iter_mut().find(|(at, _)| *at == block) {
                Some((_, count)) => {
                    *count += 1;
                    *count - 1
                }
                None => {
                    stacked.push((block, 1));
                    0
                }
            };
            let gist: String = comment.body.trim().chars().take(60).collect();
            let label = if outdated {
                format!("Outdated comment: {gist}")
            } else {
                format!("Comment: {gist}")
            };
            let id = comment.id.clone();
            margin.push(
                div()
                    .id(SharedString::from(format!("md-comment-{}", comment.id)))
                    .test_support()
                    .role(Role::Comment)
                    .aria_label(label)
                    .absolute()
                    .top(top(bounds) + px(22. * slot as f32))
                    .left(px(6.))
                    .right(px(6.))
                    .h(px(20.))
                    .px_1()
                    .rounded_sm()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(theme.mono_size)
                    .bg(if outdated { theme.comment_outdated } else { theme.comment })
                    .text_color(if outdated { theme.muted } else { theme.foreground })
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(MarkdownEvent::CommentActivated { id: id.clone() })
                    }))
                    .child(gist.clone())
                    .test_text(gist)
                    .into_any_element(),
            );
        }

        // Counts: each visible block with comments is lit, with a bar on its
        // left and the number of its comments at its right end. The tint is
        // drawn under the text; the count answers a click with the block's
        // first comment.
        let mut lit = Vec::new();
        let mut counts = Vec::new();
        if self.options.marks == CommentMarks::Counts {
            let per_target = self.comment_targets();
            let item_bounds = self.active_text().read(cx).list_item_bounds();
            for (block, item, id, count, outdated) in per_target {
                let Some((_, block_bounds)) = placed.iter().find(|(index, _)| *index == block) else { continue };
                // A list item is lit alone, reaching a little left of the text
                // as the design draws it; before its bounds are recorded, or
                // for any other block, the whole block is lit.
                let item_span = item
                    .and_then(|item| self.document.blocks().get(block)?.parts.get(item))
                    .map(|part| part.bytes());
                let item_rect = item_span.and_then(|span| item_at(&item_bounds, &span));
                let (bounds, reach) = match &item_rect {
                    Some(rect) => (rect, px(10.)),
                    None => (block_bounds, px(0.)),
                };
                let accent = if outdated { theme.muted } else { theme.comment_accent };
                let on_item = item_rect.is_some();
                lit.push(
                    div()
                        .id(match item {
                            // A lit list item has its own id, so a block's
                            // id stays the block's index.
                            Some(item) => ("md-comment-lit-item", block * 10_000 + item),
                            None => ("md-comment-lit", block),
                        })
                        .test_support()
                        .absolute()
                        .top(top(bounds))
                        .left(left(bounds) - reach)
                        .w(bounds.size.width + reach)
                        .h(bounds.size.height)
                        .border_l_2()
                        .border_color(accent)
                        .bg(theme.comment_lit)
                        .into_any_element(),
                );
                // The chip stands for the block's comments, not one of them:
                // it reads as a count, and the cards beside carry the text.
                let noun = if count == 1 { "comment" } else { "comments" };
                let label = if outdated {
                    format!("{count} outdated {noun}")
                } else {
                    format!("{count} {noun}")
                };
                let text = count.to_string();
                counts.push(
                    div()
                        .id(SharedString::from(format!("md-comment-{id}")))
                        .test_support()
                        .role(Role::Comment)
                        .aria_label(label)
                        .absolute()
                        .top(top(bounds))
                        .when(on_item, |chip| chip.h(bounds.size.height).right(px(1.)))
                        .when(!on_item, |chip| chip.right(px(4.)))
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .px_1()
                        .rounded_sm()
                        .text_size(px(11.))
                        .text_color(accent)
                        .bg(theme.comment_lit)
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(MarkdownEvent::CommentActivated { id: id.clone() })
                        }))
                        .child(Icon::new(IconName::MessageSquare).size(px(11.)).text_color(accent))
                        .child(text.clone())
                        .test_text(text)
                        .into_any_element(),
                );
            }
        }

        // Re-check after layout that the labels sat where the blocks did;
        // when scrolling or a late layout moved them, draw again.
        let entity = cx.entity().downgrade();
        let used = placed.clone();
        let recheck = canvas(
            move |_, window, cx| {
                let moved = entity
                    .read_with(cx, |view, cx| view.visible_blocks(cx).1 != used)
                    .unwrap_or(false);
                if moved {
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let has_marks = self.diff.is_some();
        let gutter_width = px(if has_marks { 28. } else { 20. });
        // A document page's text keeps its column's full width: the gutter
        // hangs in the padding to its left.
        let hang = self.options.document.is_some();
        div()
            .flex()
            .flex_row()
            .size_full()
            .when(hang, |row| row.relative())
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(gutter_width)
                    .h_full()
                    .when(hang, |column| column.absolute().top_0().left(-gutter_width))
                    .children(gutter),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(lit)
                    .child(self.render_text(cx).size_full())
                    .children(labels)
                    .children(counts)
                    .child(recheck)
                    .children(self.selection_toolbar(cx)),
            )
            .when(notes && !self.comments.is_empty(), |row| {
                row.child(
                    div()
                        .relative()
                        .flex_none()
                        .w(px(180.))
                        .h_full()
                        .border_l_1()
                        .border_color(theme.border)
                        .children(margin),
                )
            })
            .into_any_element()
    }

    /// Actions on the selection, at the top right of the text while there
    /// is one.
    fn selection_toolbar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.is_selecting(cx) {
            self.toolbar_selection = None;
            return None;
        }
        // Kept for the buttons: pressing one clears the text's selection
        // before the button's handler runs.
        self.toolbar_selection = self.selection(cx);
        self.toolbar_selection.as_ref()?;
        let theme = &self.theme;
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .test_support()
                .role(Role::Button)
                .aria_label(label)
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|button| button.bg(theme.surface))
                .child(label)
        };
        // The buttons act on press: the selection below is cleared on
        // release, before a click would arrive.
        let source_label = "Copy as Markdown";
        let plain_label = "Copy as text";
        Some(
            div()
                .id("md-selection-actions")
                .test_support()
                .role(Role::Toolbar)
                .aria_label("Selection actions")
                .occlude()
                .absolute()
                .top(px(4.))
                .right(px(12.))
                .flex()
                .flex_row()
                .gap_1()
                .p_1()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .shadow_sm()
                // Comments are made on source lines (SM-6): the rendered page's
                // selection only copies.
                .when(self.options.comments && self.mode == ViewMode::Source, |bar| {
                    bar.child(button("md-comment-selection", "Comment on selection").on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            if view.options.comments
                                && view.mode == ViewMode::Source
                                && let Some(selection) = view.toolbar_selection.take()
                            {
                                cx.emit(MarkdownEvent::CommentRequested(selection));
                            }
                        }),
                    ))
                })
                .child(button("md-copy-source", source_label).on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(selection) = &view.toolbar_selection {
                            cx.write_to_clipboard(ClipboardItem::new_string(selection.source.clone()));
                        }
                    }),
                ))
                .child(button("md-copy-plain", plain_label).on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(selection) = &view.toolbar_selection {
                            cx.write_to_clipboard(ClipboardItem::new_string(selection.quote.clone()));
                        }
                    }),
                ))
                .into_any_element(),
        )
    }

    /// Comments whose passage is gone, each with Re-attach and Drop (SM-7).
    fn gone_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let gone: Vec<&Comment> = self
            .comments
            .iter()
            .zip(&self.statuses)
            .filter(|(_, status)| matches!(status, AnchorStatus::Gone))
            .map(|(comment, _)| comment)
            .collect();
        if gone.is_empty() {
            return None;
        }
        let theme = &self.theme;
        let rows = gone.into_iter().map(|comment| {
            let reattach = comment.id.clone();
            let drop = comment.id.clone();
            let quote: String = comment.anchor.quote.trim().chars().take(80).collect();
            let action = |kind: &str, id: &str| {
                div()
                    .id(SharedString::from(format!("md-{kind}-{id}")))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} comment {id}",
                        if kind == "reattach" { "Re-attach" } else { "Drop" }
                    ))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .child(if kind == "reattach" { "Re-attach" } else { "Drop" })
            };
            div()
                .flex()
                .flex_row()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(format!("\u{201c}{quote}\u{201d}"))
                        .test_text(quote),
                )
                .child(action("reattach", &comment.id).on_click(cx.listener(
                    move |view, _: &ClickEvent, _, cx| {
                        let selection = view.selection(cx);
                        cx.emit(MarkdownEvent::ReattachRequested {
                            id: reattach.clone(),
                            selection,
                        })
                    },
                )))
                .child(action("drop", &comment.id).on_click(cx.listener(
                    move |_, _: &ClickEvent, _, cx| cx.emit(MarkdownEvent::DropRequested { id: drop.clone() }),
                )))
        });
        let heading = "These comments' passages are no longer in the document; select where each belongs and Re-attach it, or Drop it.";
        Some(
            div()
                .id("md-gone-comments")
                .test_support()
                .role(Role::Status)
                .aria_label("Comments without a passage")
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.comment_outdated)
                .child(div().text_color(theme.muted).child(heading).test_text(heading))
                .children(rows)
                .into_any_element(),
        )
    }
}

impl Focusable for MarkdownView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_text().read(cx).focus_handle().clone()
    }
}

impl Render for MarkdownView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let opened = (self.mode == ViewMode::Rendered).then(|| self.shared.zooms.opened()).flatten();
        let body = match (self.options.layout, self.mode) {
            _ if opened.is_some() => {
                let fill = self.options.layout == Layout::Scroll;
                let shared = self.shared.clone();
                opened.map_or_else(|| div().into_any_element(), |open| shared.render_open(&open, fill, window, cx))
            }
            (Layout::Scroll, ViewMode::Rendered) => self.render_rendered(cx),
            (layout, _) => {
                self.stats.labelled_blocks = 0;
                let text = self.render_text(cx);
                div()
                    .relative()
                    .when(layout == Layout::Scroll, |body| body.size_full())
                    .when(layout == Layout::Inline, |body| body.w_full())
                    .child(if layout == Layout::Scroll { text.size_full() } else { text.w_full() })
                    .children(self.selection_toolbar(cx))
                    .into_any_element()
            }
        };
        let gone = if self.mode == ViewMode::Rendered {
            self.gone_notice(cx)
        } else {
            None
        };
        div()
            .id("md-view")
            .test_support()
            .role(Role::Document)
            .aria_label(self.options.label.clone())
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|view, _: &CopyOther, _, cx| view.copy(view.options.copy.other(), cx)))
            .on_action(cx.listener(|view, _: &CommentOnSelection, _, cx| view.request_selection_comment(cx)))
            .on_action(cx.listener(|view, _: &ToggleSource, window, cx| view.toggle_mode(window, cx)))
            .flex()
            .flex_col()
            .when(self.options.layout == Layout::Scroll, |root| root.size_full())
            .when(self.options.layout == Layout::Inline, |root| root.w_full())
            .text_color(theme.foreground)
            .font_family(theme.sans.clone())
            .text_size(theme.text_size)
            .children(gone)
            .when(theme.document, |root| root.line_height(px(20.)).pt(px(22.)))
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// The source as one fenced Markdown code block, fenced with more backticks
/// than any run in it.
fn fenced(source: &str) -> String {
    let prefix = fence_prefix(source);
    let fence = &prefix[..prefix.len() - "markdown\n".len()];
    let mut out = String::with_capacity(source.len() + prefix.len() * 2);
    out.push_str(&prefix);
    out.push_str(source);
    if !source.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(fence);
    out
}

fn fence_prefix(source: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for ch in source.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    format!("{}markdown\n", "`".repeat(longest.max(2) + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_item_is_found_by_where_its_source_starts() {
        let bounds = |width: f32| Bounds { origin: Default::default(), size: gpui_kit::size(px(width), px(20.)) };
        let items = vec![(10..30, bounds(100.)), (31..60, bounds(200.))];
        assert_eq!(item_at(&items, &(31..59)), Some(bounds(200.)));
        assert_eq!(item_at(&items, &(10..30)), Some(bounds(100.)));
        assert_eq!(item_at(&items, &(70..80)), None);
        assert_eq!(item_at(&[], &(10..30)), None);
    }

    /// SM-4
    #[test]
    fn the_source_fence_is_longer_than_any_backtick_run_in_it() {
        let source = "text\n````rust\ncode\n````\n";
        let shown = fenced(source);
        assert!(shown.starts_with("`````markdown\n"));
        assert!(shown.ends_with("\n`````"));
        assert_eq!(&shown[fence_prefix(source).len()..][..source.len()], source);
    }
}

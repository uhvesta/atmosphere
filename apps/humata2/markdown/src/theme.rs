//! Colours and fonts for the view. They come from the app's gpui-component
//! theme: build a [`MarkdownTheme`] with [`MarkdownTheme::from_app`] and
//! change any field. Code colours aren't here: fenced code is highlighted by
//! gpui-component's highlighter with the app's own highlight theme.

use gpui_kit::base::TextViewStyle;
use gpui_kit::component::Theme;
use gpui_kit::{
    App, FontWeight, HighlightStyle, Hsla, Pixels, Rgba, SharedString, StyleRefinement, Styled as _, px, rems, rgb,
};

/// Everything the view draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownTheme {
    pub dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub link: Hsla,
    pub selection: Hsla,
    pub surface: Hsla,
    pub code_background: Hsla,
    pub border: Hsla,
    /// Inserted text and blocks in a rendered diff.
    pub inserted: Hsla,
    /// Deleted blocks in a rendered diff.
    pub deleted: Hsla,
    /// Blocks whose text changed in a rendered diff.
    pub changed: Hsla,
    /// A comment's highlight, and one whose passage changed since.
    pub comment: Hsla,
    pub comment_outdated: Hsla,
    /// A commented block's tint and its left bar and count, when comments
    /// show as counts ([`crate::CommentMarks::Counts`]).
    pub comment_lit: Hsla,
    pub comment_accent: Hsla,
    /// An inline error, such as an invalid diagram.
    pub error: Hsla,
    pub sans: SharedString,
    pub mono: SharedString,
    /// Body text size, and the code size.
    pub text_size: Pixels,
    pub mono_size: Pixels,
    /// Whether the page is a document tab's: the page typography of the
    /// design instead of the chat's.
    pub(crate) document: bool,
}

impl MarkdownTheme {
    /// The app's current theme.
    pub fn from_app(cx: &App) -> Self {
        Self::from_component(Theme::global(cx))
    }

    /// Derive the view's theme from a gpui-component theme.
    pub fn from_component(theme: &Theme) -> Self {
        Self {
            dark: theme.is_dark(),
            background: theme.background,
            foreground: theme.foreground,
            muted: theme.muted_foreground,
            link: theme.link,
            selection: theme.selection,
            surface: theme.secondary,
            code_background: theme.muted,
            border: theme.border,
            inserted: theme.success,
            deleted: theme.danger,
            changed: theme.warning,
            comment: theme.info.opacity(0.25),
            comment_outdated: theme.muted_foreground.opacity(0.2),
            comment_lit: if theme.is_dark() { rgb(0x1c2a3f).into() } else { rgb(0xeaf2ff).into() },
            comment_accent: if theme.is_dark() { rgb(0x3b8cff).into() } else { rgb(0x0a6cff).into() },
            error: theme.danger,
            sans: theme.font_family.clone(),
            mono: theme.mono_font_family.clone(),
            text_size: theme.font_size,
            mono_size: theme.mono_font_size,
            document: false,
        }
    }

    /// The look of a document tab's page: 13/20 body, 22 title, 15 section
    /// headings, 12/17 mono code in a plain filled block, 6 between blocks
    /// and between list items.
    pub(crate) fn document(mut self) -> Self {
        self.document = true;
        self.text_size = px(13.);
        self.mono_size = px(12.);
        self
    }

    /// The text view's style for this theme.
    pub(crate) fn text_view_style(&self) -> TextViewStyle {
        let mono = self.mono.clone();
        let style = TextViewStyle::default()
            .with_foreground(self.foreground)
            .with_muted_foreground(self.muted)
            .with_link(self.link)
            .with_selection(self.selection)
            .with_code_background(self.code_background)
            .with_border(self.border)
            .with_paragraph_gap(rems(0.75))
            .with_code_block(
                StyleRefinement::default()
                    .font_family(mono.clone())
                    .text_size(self.mono_size),
            )
            .with_inline_code(HighlightStyle {
                background_color: Some(self.code_background),
                ..HighlightStyle::default()
            })
            .with_inline_code_style(StyleRefinement::default().font_family(mono))
            .with_dark(self.dark);
        if !self.document {
            return style;
        }
        let foreground = self.foreground;
        // The design's fill: code sits in it, a step off the page.
        let fill: Hsla = if self.dark { rgb(0x323235).into() } else { rgb(0xeceaee).into() };
        style
            .with_paragraph_gap(rems(6. / 16.))
            .with_list_item_gap(px(6.))
            .with_heading(|level| {
                let heading = StyleRefinement::default().line_height(px(20.)).pb(px(6.));
                match level {
                    1 => heading.text_size(px(22.)).font_weight(FontWeight::BOLD).pt(px(0.)),
                    2 => heading.text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).pt(px(11.)),
                    _ => heading.text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).pt(px(11.)),
                }
            })
            .with_code_block(
                StyleRefinement::default()
                    .font_family(self.mono.clone())
                    .text_size(self.mono_size)
                    .line_height(px(17.))
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(fill)
                    .border_0(),
            )
            // The bullet keeps the text colour and sits 8 before the text.
            .with_list_style(
                StyleRefinement::default()
                    .text_color(foreground)
                    .min_w(px(0.))
                    .pr(px(8.))
                    .text_left(),
            )
    }

    /// Mermaid's theme for this theme.
    pub(crate) fn mermaid(&self) -> mermaid_rs_renderer::Theme {
        let mut theme = if self.dark {
            mermaid_rs_renderer::Theme::dark()
        } else {
            mermaid_rs_renderer::Theme::modern()
        };
        let hex = |color: Hsla| hex(color);
        theme.font_family = self.sans.to_string();
        theme.font_size = f32::from(self.text_size) * 0.875;
        theme.primary_color = hex(self.surface);
        theme.primary_text_color = hex(self.foreground);
        theme.primary_border_color = hex(self.border);
        theme.line_color = hex(self.muted);
        theme.secondary_color = hex(self.code_background);
        theme.tertiary_color = hex(self.surface);
        theme.edge_label_background = hex(self.background);
        theme.cluster_background = hex(self.code_background);
        theme.cluster_border = hex(self.border);
        theme.background = hex(self.background);
        theme.sequence_actor_fill = hex(self.surface);
        theme.sequence_actor_border = hex(self.border);
        theme.sequence_actor_line = hex(self.muted);
        theme.sequence_note_fill = hex(self.code_background);
        theme.sequence_note_border = hex(self.border);
        theme.sequence_activation_fill = hex(self.code_background);
        theme.sequence_activation_border = hex(self.border);
        theme.text_color = hex(self.foreground);
        theme
    }
}

impl Default for MarkdownTheme {
    /// gpui-component's default colours, for tests and previews; apps pass
    /// their own through [`MarkdownTheme::from_app`].
    fn default() -> Self {
        Self::from_component(&Theme::default())
    }
}

/// `#rrggbb`, with the alpha dropped.
pub(crate) fn hex(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(rgba.r), byte(rgba.g), byte(rgba.b))
}

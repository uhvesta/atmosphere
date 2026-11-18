//! The live terminal under a question read off its agent's screen, in the
//! wide Inbox (D781). It draws the tab at the tab's own size: the font
//! shrinks to fit the columns, down to 9 pt, and a wider screen scrolls
//! sideways. The box is as tall as the rows, at most 420, and opens
//! scrolled to the bottom. It never takes focus by itself: a click in it,
//! or Tab from the list, gives it focus, shown by a 2 pt accent border.
use crate::terminals::{Link, font_theme, grid_text};
use crate::{bridge::Ui, theme::Look, widgets::row};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::RequestId;
use humata2_inbox::InboxAction;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use terminal_grid::Grid as _;
use terminal_replica::TerminalReplica;
use terminal_view::Theme;

/// The tallest the box grows; more rows scroll.
const MAX_HEIGHT: f32 = 420.;
/// The smallest font the screen shrinks to before it scrolls sideways.
const MIN_FONT: f32 = 9.;
/// Inside the box, around the screen.
const INSET: f32 = 8.;

/// The view the Inbox's detail draws under the card.
pub(crate) struct InboxTerminal {
    ui: Rc<Ui>,
    request: RequestId,
    /// The replica and its link; `None` when it could not be made.
    shown: Option<(Entity<TerminalReplica>, Rc<RefCell<Link>>)>,
    /// Why it could not be made.
    why: Option<String>,
    /// The font size setting, the most the screen is drawn at.
    font: u8,
    look: Option<Look>,
    /// The box's width at the last paint, to fit the font to.
    width: Rc<Cell<Pixels>>,
    /// Whether it has been scrolled to the bottom since it opened.
    at_bottom: bool,
}

impl InboxTerminal {
    pub(crate) fn new(
        ui: Rc<Ui>,
        request: RequestId,
        shown: Option<(Entity<TerminalReplica>, Rc<RefCell<Link>>)>,
        why: Option<String>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|_| Self {
            ui,
            request,
            shown,
            why,
            font: 0,
            look: None,
            width: Rc::new(Cell::new(px(0.))),
            at_bottom: false,
        })
    }

    /// What the shell's last sync saw.
    pub(crate) fn update(&mut self, request: RequestId, font: u8, look: Look, cx: &mut Context<Self>) {
        self.request = request;
        self.font = font;
        self.look = Some(look);
        cx.notify();
    }

    /// The reason in the link's place, with where to try instead.
    fn failure(&self, why: &str, look: Look) -> AnyElement {
        let ui = self.ui.clone();
        let id = self.request.clone();
        let message = format!("{}.", why.trim_end_matches('.'));
        row()
            .id("inbox-terminal-error")
            .role(Role::Alert)
            .aria_label(format!("{message} Open session ↗ to try there."))
            .test_support()
            .gap(px(6.))
            .flex_wrap()
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(div().child(message.clone()).test_text(message))
            .child(
                div()
                    .id("inbox-terminal-open-session")
                    .role(Role::Button)
                    .cursor_pointer()
                    .text_color(rgb(look.accent))
                    .child("Open session ↗")
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Inbox(InboxAction::OpenRequest(id.clone())), cx);
                    }),
            )
            .child("to try there.")
            .into_any_element()
    }
}

impl Render for InboxTerminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(look) = self.look else {
            return div().into_any_element();
        };
        let Some((replica, link)) = self.shown.clone() else {
            let why = self.why.clone().unwrap_or_default();
            return self.failure(&why, look);
        };
        let pane = replica.read(cx).pane().cloned();
        let Some(pane) = pane else {
            if let Link::Ended { why: Some(why), .. } = &*link.borrow() {
                return self.failure(why, look);
            }
            return div()
                .id("inbox-terminal-opening")
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child("Opening its terminal…")
                .into_any_element();
        };
        let view = pane.read(cx).view().clone();
        let (cols, rows, cell, current) = {
            let view = view.read(cx);
            let size = view.grid().size();
            (size.cols, size.rows, view.cell_size(), view.theme().font_size)
        };
        // The font that fits the columns in the box, between 9 pt and the
        // setting: a cell's width grows with the font.
        let base = font_theme(self.font.max(1)).font_size;
        let inner = f32::from(self.width.get()) - 2. * INSET - 4.;
        if let Some(cell) = cell
            && inner > 0.
            && cols > 0
        {
            let per_point = cell.width / current.max(1.);
            let fits = (inner / (f32::from(cols) * per_point)).floor();
            let wanted = fits.clamp(MIN_FONT, base.max(MIN_FONT));
            if (wanted - current).abs() >= 0.5 {
                view.update(cx, |view, cx| {
                    view.set_theme(
                        Theme {
                            font_size: wanted,
                            line_height: wanted + 6.5,
                            ..Default::default()
                        },
                        cx,
                    );
                });
                cx.notify();
            }
        }
        let line = cell.map_or(18., |cell| cell.height);
        let height = (f32::from(rows) * line + 1.).min(MAX_HEIGHT);
        if !self.at_bottom {
            let scroll = replica.read(cx).viewport_scroll().clone();
            let bottom = scroll.max_offset().y;
            if bottom > px(0.) {
                scroll.set_offset(point(scroll.offset().x, -bottom));
                self.at_bottom = true;
            } else if f32::from(rows) * line + 1. <= MAX_HEIGHT && cell.is_some() {
                self.at_bottom = true;
            } else {
                cx.notify();
            }
        }
        let focused = pane.read(cx).focus_handle(cx).contains_focused(window, cx);
        let text = grid_text(&pane, cx);
        let width = self.width.clone();
        let click = replica.clone();
        div()
            .id("inbox-terminal")
            .role(Role::Log)
            .aria_label("The question's terminal")
            .aria_description(text.clone())
            .test_text(text)
            .test_support()
            .relative()
            .w_full()
            .h(px(height + 2. * INSET + 4.))
            .rounded(px(8.))
            .bg(rgb(look.term))
            .border(px(if focused { 2. } else { 1. }))
            .border_color(rgb(if focused { look.accent } else { look.line }))
            .p(px(INSET + if focused { 0. } else { 1. }))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                click.update(cx, |replica, cx| replica.focus_input(window, cx));
            })
            .child(replica)
            .child(
                canvas(
                    move |bounds, window, _| {
                        if width.get() != bounds.size.width {
                            width.set(bounds.size.width);
                            window.refresh();
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }
}

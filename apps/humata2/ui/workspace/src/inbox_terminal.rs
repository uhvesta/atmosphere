//! The live terminal under a question read off its agent's screen, in the
//! wide Inbox (D781). It draws the tab at the tab's own size: the font
//! shrinks to fit the columns, down to 9 pt, and a wider screen scrolls
//! sideways. The box shows whole rows only, opens scrolled to the bottom,
//! and is at most 420 pt and at most what the detail has left under the card
//! and above the hint, so Dismiss stays in view; below 8 rows the detail
//! scrolls instead. It never takes focus by itself: a click in it, or Tab
//! from the list, gives it focus, shown by a 2 pt accent border.
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

/// The tallest the box grows, border and inset included; more rows scroll.
const MAX_HEIGHT: f32 = 420.;
/// The fewest rows the box shows when the detail is short: the detail
/// scrolls instead.
const MIN_ROWS: u16 = 8;
/// The smallest font the screen shrinks to before it scrolls sideways.
const MIN_FONT: f32 = 9.;
/// Inside the box, around the screen, on all four sides.
const INSET: f32 = 8.;
/// The box's frame, top and bottom: the inset and a 1 pt border each.
const FRAME: f32 = 2. * (INSET + 1.);
pub use humata2_ui_core::bridge::{DETAIL_GAP, DETAIL_PAD, InboxRoom, Measured};

/// The view the Inbox's detail draws under the card.
pub struct InboxTerminal {
    ui: Rc<Ui>,
    request: RequestId,
    /// The replica and its link; `None` when it could not be made.
    shown: Option<(Entity<TerminalReplica>, Rc<RefCell<Link>>)>,
    /// Why it could not be made.
    why: Option<String>,
    /// The font family setting.
    font_family: String,
    /// The font size setting, the most the screen is drawn at.
    font: u8,
    look: Option<Look>,
    /// The box's width at the last paint, to fit the font to.
    width: Rc<Cell<Pixels>>,
    /// Whether it has been scrolled to the bottom since it opened or
    /// changed how many rows it shows.
    at_bottom: bool,
    /// How many rows it showed at the last render.
    rows_shown: u16,
    /// Cached `grid_text` keyed by `(replica_offset, cols, rows, scroll_offset)`
    /// so idle 600ms cursor-blink frames do not re-allocate every row.
    cached_text: RefCell<Option<(u64, u16, u16, u64, SharedString)>>,
}

impl InboxTerminal {
    pub fn new(
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
            font_family: String::new(),
            font: 0,
            look: None,
            width: Rc::new(Cell::new(px(0.))),
            at_bottom: false,
            rows_shown: 0,
            cached_text: RefCell::new(None),
        })
    }

    /// What the shell's last sync saw.
    pub fn update(
        &mut self,
        request: RequestId,
        font_family: String,
        font: u8,
        look: Look,
        cx: &mut Context<Self>,
    ) {
        self.request = request;
        self.font_family = font_family;
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

    /// Whether it draws the screen (or is opening it), not the reason it
    /// can't: as [`Render`] decides.
    pub fn shows_screen(&self, cx: &App) -> bool {
        let Some((replica, link)) = &self.shown else {
            return false;
        };
        replica.read(cx).pane().is_some()
            || !matches!(&*link.borrow(), Link::Ended { why: Some(_), .. })
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
        let (cols, rows, cell, current, current_family, scroll_offset) = {
            let view = view.read(cx);
            let grid = view.grid();
            let size = grid.size();
            let theme = view.theme();
            (
                size.cols,
                size.rows,
                view.cell_size(),
                theme.font_size,
                theme.font_family.clone(),
                grid.scrollbar().map_or(0, |bar| bar.offset),
            )
        };
        // The font that fits the columns in the box, between 9 pt and the
        // setting: a cell's width grows with the font.
        let base_theme = font_theme(&self.font_family, self.font.max(1));
        let base = base_theme.font_size;
        let inner = f32::from(self.width.get()) - 2. * INSET - 4.;
        if let Some(cell) = cell
            && inner > 0.
            && cols > 0
        {
            let per_point = cell.width / current.max(1.);
            let fits = (inner / (f32::from(cols) * per_point)).floor();
            let wanted = fits.clamp(MIN_FONT, base.max(MIN_FONT));
            if (wanted - current).abs() >= 0.5 || current_family != base_theme.font_family {
                let family = base_theme.font_family.clone();
                view.update(cx, |view, cx| {
                    view.set_theme(
                        Theme {
                            font_family: family,
                            font_size: wanted,
                            line_height: (wanted * 1.45).round(),
                            ..Default::default()
                        },
                        cx,
                    );
                });
                cx.notify();
            }
        } else if current_family != base_theme.font_family {
            let family = base_theme.font_family.clone();
            view.update(cx, |view, cx| {
                view.set_theme(
                    Theme {
                        font_family: family,
                        font_size: current,
                        line_height: (current * 1.45).round(),
                        ..Default::default()
                    },
                    cx,
                );
            });
            cx.notify();
        }
        // Whole rows only: as many as fit in 420 pt and in what the detail
        // has left, at least 8 (the detail scrolls then), at most the tab's.
        let line = cell.map_or(18., |cell| cell.height);
        let most = self.ui.inbox_room().room().map_or(MAX_HEIGHT, |room| room.min(MAX_HEIGHT));
        // The grid is a pixel taller than its rows.
        let fit = ((most - FRAME - 1.) / line).floor().clamp(0., f32::from(u16::MAX)) as u16;
        let shown = fit.max(MIN_ROWS).min(rows.max(1));
        let height = f32::from(shown) * line + 1.;
        // A box that grew or shrank under the card opens at the bottom again,
        // where the question is.
        if shown != self.rows_shown {
            self.rows_shown = shown;
            self.at_bottom = false;
        }
        if !self.at_bottom {
            let scroll = replica.read(cx).viewport_scroll().clone();
            let bottom = scroll.max_offset().y;
            if bottom > px(0.) {
                scroll.set_offset(point(scroll.offset().x, -bottom));
                self.at_bottom = true;
            } else if shown >= rows && cell.is_some() {
                self.at_bottom = true;
            } else {
                cx.notify();
            }
        }
        let focused = pane.read(cx).focus_handle(cx).contains_focused(window, cx);
        let offset = replica.read(cx).state().offset;
        let text: SharedString = {
            let cached = self.cached_text.borrow();
            if let Some((o, c, r, s, ref text)) = *cached
                && o == offset
                && c == cols
                && r == rows
                && s == scroll_offset
            {
                text.clone()
            } else {
                drop(cached);
                let computed: SharedString = grid_text(&pane, cx).into();
                *self.cached_text.borrow_mut() =
                    Some((offset, cols, rows, scroll_offset, computed.clone()));
                computed
            }
        };
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
            .h(px(height + FRAME))
            .rounded(px(8.))
            .bg(rgb(look.term))
            .border(px(if focused { 2. } else { 1. }))
            .border_color(rgb(if focused { look.accent } else { look.line }))
            // The 2 pt focus border takes a point of the inset's place, so
            // the screen never moves.
            .p(px(INSET - if focused { 1. } else { 0. }))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, {
                let click = click.clone();
                move |_, window, cx| {
                    click.update(cx, |replica, cx| replica.focus_input(window, cx));
                }
            })
            // An accessibility press is a click, not a mouse-down: it
            // takes the keys the same way.
            .on_click(move |_, window, cx| {
                let held = click.read(cx).pane().is_some_and(|pane| {
                    pane.read(cx).focus_handle(cx).contains_focused(window, cx)
                });
                if !held {
                    click.update(cx, |replica, cx| replica.focus_input(window, cx));
                }
            })
            // Clipped to the inset, so it shows on all four sides.
            .child(div().size_full().overflow_hidden().child(replica))
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

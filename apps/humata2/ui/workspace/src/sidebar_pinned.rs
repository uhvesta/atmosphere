//! The sidebar's Pinned section (NV-20 to NV-25, frame 15): the pinned
//! sessions, channels, machines, PR reviews and board items in the person's
//! order, each with its VIP badge and the event it shows. A pin opens on a
//! click, moves by dragging it onto another pin or with its up arrow, and
//! comes out with its ×.

use std::rc::Rc;

use humata2_ui_core::widgets::{col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_clients::marks::{Pinned, Thing};
use humata2_clients::model::Status;
use humata2_clients::Workspace;
use humata2_find::{PinnedAction, PinnedState};

/// Sends a Pinned action to the store, such as
/// `move |action, cx| ui.send(AppAction::Pinned(action), cx)`.
pub type SendPinned = Rc<dyn Fn(PinnedAction, &mut App)>;

/// The color of a pin's event, as frame 15 draws "Asks to move …".
const EVENT: u32 = 0xff9f0a;
/// The VIP badge's fill.
const VIP: u32 = 0xbf5af2;
/// A pin's row, and the height of a row with an event under its title.
const ROW: f32 = 26.;
const EVENT_ROW: f32 = 42.;
/// The height of a row's title line, where its icon, badge and buttons sit.
const LINE_EVENT: f32 = 21.;
/// The space an event line leaves below its row, so the next row starts
/// where frame 15's does.
const EVENT_GAP: f32 = 0.3;

/// The height of the section's header, and what a pin's row takes.
const SECTION: f32 = 29.;

/// The least share of its height the section shrinks to. Frame 15's VIP card
/// is 37 pt tall and Home sits 5 pt under the last pin.
const MIN_SHARE: f32 = 0.77;

fn row_height(pin: &Pinned, workspace: &Workspace) -> f32 {
    if !pin.event.is_empty() || facts(&pin.thing, workspace).is_some_and(|f| f.summary.is_some()) {
        EVENT_ROW + EVENT_GAP
    } else {
        ROW
    }
}

/// The Pinned section for `pinned`, or nothing when Settings hides it or
/// nothing is pinned and nothing went wrong. `current` is the thing the
/// window shows, drawn selected. A pinned session shows what `workspace`
/// knows of it: its status, its pull request and its latest activity.
pub fn section(
    pinned: &PinnedState,
    workspace: &Workspace,
    current: Option<&Thing>,
    window_active: bool,
    look: Look,
    send: &SendPinned,
) -> Option<AnyElement> {
    if !pinned.shown || (pinned.list.items.is_empty() && pinned.error.is_none()) {
        return None;
    }
    // The section shrinks with the column as one block, from its own height
    // down to MIN_SHARE of it; its rows then share what it gives up, so
    // together they shrink as frame 15's rows do.
    let natural = SECTION
        + pinned
            .list
            .items
            .iter()
            .map(|pin| row_height(pin, workspace))
            .sum::<f32>();
    let mut list = col()
        .id("sidebar-pinned")
        .min_h(px(natural))
        .flex_shrink_0()
        .role(Role::List)
        .aria_label("Pinned")
        .child(chrome::section(look, "Pinned"));
    if let Some(error) = &pinned.error {
        let text = format!("Pins didn't update: {error}. They show as the host last sent them.");
        let retry = send.clone();
        let dismiss = send.clone();
        let current_list = pinned.list.clone();
        let hover_bg = if look.content != 0xffffff {
            rgba(0xffffff0f)
        } else {
            rgba(0x0000000a)
        };
        list = list.child(
            col()
                .id("sidebar-pinned-error")
                .role(Role::Alert)
                .aria_label(text.clone())
                .mx(px(10.))
                .px(px(8.))
                .py(px(4.))
                .gap(px(4.))
                .text_size(px(11.))
                .text_color(rgb(humata2_ui_core::theme::RED))
                .child(text)
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            div()
                                .id("sidebar-pinned-retry")
                                .role(Role::Button)
                                .aria_label("Retry")
                                .px(px(4.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover_bg))
                                .text_color(rgb(look.accent))
                                .child("Retry")
                                .on_click(move |_, _, cx| retry(PinnedAction::Start, cx))
                                .test_support(),
                        )
                        .child(
                            div()
                                .id("sidebar-pinned-dismiss")
                                .role(Role::Button)
                                .aria_label("Dismiss")
                                .px(px(4.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover_bg))
                                .text_color(rgb(look.secondary))
                                .child("Dismiss")
                                .on_click(move |_, _, cx| {
                                    dismiss(PinnedAction::Changed(current_list.clone()), cx);
                                })
                                .test_support(),
                        ),
                )
                .test_support(),
        );
    }
    for (index, pin) in pinned.list.items.iter().enumerate() {
        list = list.child(pin_row(
            pin,
            workspace,
            index,
            current == Some(&pin.thing),
            window_active,
            look,
            send,
        ));
    }
    Some(list.test_support().into_any_element())
}

/// What is carried while a pin is dragged: its place, and its name for the
/// preview under the pointer.
#[derive(Clone)]
struct DraggedPin {
    from: usize,
    title: SharedString,
}

impl Render for DraggedPin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::of(cx);
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(13.))
            .text_color(rgb(look.text))
            .child(self.title.clone())
    }
}

fn glyph(thing: &Thing) -> IconName {
    match thing {
        Thing::Session(_) => IconName::SquareTerminal,
        Thing::Channel(_) => IconName::FolderGit2,
        Thing::Machine(_) => IconName::Server,
        Thing::PrReview(_) => IconName::GitPullRequest,
        Thing::BoardItem(_) => IconName::SquareKanban,
    }
}

/// A small button on a pin's row: the click stops there, so the row
/// doesn't open too.
fn tool(
    id: String,
    label: &'static str,
    icon: IconName,
    color: u32,
    group: SharedString,
    action: PinnedAction,
    send: &SendPinned,
) -> impl IntoElement {
    let send = send.clone();
    div()
        .id(ElementId::Name(id.into()))
        .role(Role::Button)
        .aria_label(label)
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(px(4.))
        .cursor_pointer()
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .child(chrome::icon(icon, color, 11.))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            send(action.clone(), cx);
        })
        .test_support()
}

/// What the workspace says about a pinned session: its status, its pull
/// request and the line under its title.
struct Facts {
    status: Status,
    pull: Option<humata2_clients::model::PullRequest>,
    summary: Option<(String, bool)>,
}

fn facts(thing: &Thing, workspace: &Workspace) -> Option<Facts> {
    let Thing::Session(id) = thing else {
        return None;
    };
    let session = workspace.session(id)?;
    let pull = workspace
        .pulls
        .iter()
        .find(|p| p.session.as_ref() == Some(id))
        .cloned();
    let summary = (pull.is_none() && !session.activity.is_empty()).then(|| {
        (
            format!("{} · {}", session.activity, session.active),
            session.status == Status::NeedsYou,
        )
    });
    Some(Facts {
        status: session.status,
        pull,
        summary,
    })
}

fn pin_row(
    pin: &Pinned,
    workspace: &Workspace,
    index: usize,
    current: bool,
    window_active: bool,
    look: Look,
    send: &SendPinned,
) -> AnyElement {
    let active_current = current && window_active;
    let faint = if active_current { 0xffffff } else { look.tertiary };
    let id = format!("sidebar-pinned-{index}");
    let group = SharedString::from(id.clone());
    let facts = facts(&pin.thing, workspace);
    // The line under the title: a session's activity, else the pin's event.
    let under = match &facts {
        Some(Facts {
            summary: Some((text, asks)),
            ..
        }) => Some((text.clone(), if *asks { EVENT } else { look.secondary }, 25.)),
        Some(_) => None,
        None => (!pin.event.is_empty()).then(|| (pin.event.clone(), EVENT, 19.)),
    };
    let event = under.is_some();
    let label = match (pin.vip, event.then_some(under.as_ref().map(|u| u.0.as_str())).flatten()) {
        (true, Some(text)) => format!("{}, VIP, {text}", pin.title),
        (true, None) => format!("{}, VIP", pin.title),
        (false, Some(text)) => format!("{}, {text}", pin.title),
        (false, None) => pin.title.clone(),
    };
    let dragged = DraggedPin {
        from: index,
        title: pin.title.clone().into(),
    };
    let open = send.clone();
    let drop = send.clone();
    let icon = div()
        .h(px(if event { LINE_EVENT } else { ROW }))
        .flex()
        .items_center()
        .flex_shrink_0()
        .child(match &facts {
            Some(Facts { pull: Some(_), .. }) => {
                chrome::icon(IconName::GitPullRequest, look.accent, 12.).into_any_element()
            }
            Some(facts) => chrome::status(facts.status).into_any_element(),
            None => chrome::icon(
                glyph(&pin.thing),
                if active_current { 0xffffff } else { look.secondary },
                12.,
            )
            .into_any_element(),
        });
    // Frame 15's session row: the title, badge and status hang from the
    // row's top padding, and the badge keeps 7 pt clear of the row's edge.
    let mut line = row()
        .h(px(if event { LINE_EVENT } else { ROW }))
        .gap(px(7.))
        .when(event, |line| line.items_start().pr(px(7.)))
        .child(icon)
        .child(crate::full_text::full_text(
            div()
                .id(ElementId::Name(format!("{id}-title").into()))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(pin.title.clone()),
            pin.title.clone(),
        ));
    if pin.vip {
        line = line.child(
            div()
                .px(px(4.))
                .rounded(px(3.))
                .bg(rgb(VIP))
                .text_size(px(9.5))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xffffff))
                .child("VIP"),
        );
    }
    if let Some(Facts {
        pull: Some(pull), ..
    }) = &facts
    {
        line = line.child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(pull.checks.clone()),
        );
    }
    // The buttons float over the row's right end, so they take no room from
    // the title and badge until the pointer is on the row.
    let mut tools = div()
        .absolute()
        .top(px(if event { 5. } else { 0. }))
        .right(px(6.))
        .h(px(if event { LINE_EVENT } else { ROW }))
        .flex()
        .items_center()
        .gap(px(4.));
    if index > 0 {
        tools = tools.child(tool(
            format!("{id}-up"),
            "Move up",
            IconName::ArrowUp,
            faint,
            group.clone(),
            PinnedAction::Move {
                from: index,
                to: index - 1,
            },
            send,
        ));
    }
    tools = tools.child(tool(
        format!("{id}-unpin"),
        "Unpin",
        IconName::X,
        faint,
        group.clone(),
        PinnedAction::Toggle {
            thing: pin.thing.clone(),
            pin: false,
        },
        send,
    ));
    let event_id = format!("{id}-event");
    div()
        .id(ElementId::Name(id.into()))
        .group(group)
        .role(Role::ListItem)
        .aria_label(label)
        .aria_selected(current)
        .h(px(if event { EVENT_ROW } else { ROW }))
        .flex_shrink_0()
        .when(event, |x| x.pt(px(5.)).mb(px(EVENT_GAP)))
        .overflow_hidden()
        .mx(px(10.))
        .px(px(8.))
        .rounded(px(6.))
        .cursor_pointer()
        .when(current, |x| {
            x.bg(rgb(if window_active {
                look.selection
            } else {
                look.fill
            }))
            .text_color(rgb(if window_active { 0xffffff } else { look.text }))
        })
        .child(line)
        .children(under.map(|(text, color, left)| {
            // Hangs from the bottom edge, as frame 15's asks line does, so
            // the row's own height decides where it sits. Its left is measured
            // from the row's left edge, which sits 10 pt in from the sidebar's.
            crate::full_text::full_text(
                div()
                    .id(ElementId::Name(event_id.into()))
                    .absolute()
                    .left(px(left + 8.))
                    .right(px(8.))
                    .bottom(px(0.7))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .text_color(rgb(color))
                    .child(text.clone()),
                text,
            )
        }))
        .child(tools)
        .on_drag(dragged, |pin, _, _, cx| cx.new(|_| pin.clone()))
        .drag_over::<DraggedPin>(move |style, _, _, _| style.bg(rgb(look.fill)))
        .on_drop(move |pin: &DraggedPin, _, cx| {
            drop(
                PinnedAction::Move {
                    from: pin.from,
                    to: index,
                },
                cx,
            )
        })
        .on_click(move |_, _, cx| open(PinnedAction::Open(index), cx))
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    // The glob above brings GPUI's `test` macro; `#[test]` is the standard one.
    use ::core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt as _;
    use humata2_clients::marks::PinnedList;
    use humata2_clients::model::{ChannelId, SessionId};

    struct View {
        pinned: PinnedState,
        send: SendPinned,
    }

    impl Render for View {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(section(
                &self.pinned,
                &Workspace::default(),
                None,
                window.is_window_active(),
                Look::of(cx),
                &self.send,
            ))
        }
    }

    fn pin(thing: Thing, title: &str, vip: bool) -> Pinned {
        Pinned {
            thing,
            title: title.to_owned(),
            event: String::new(),
            vip,
        }
    }

    fn state(items: Vec<Pinned>) -> PinnedState {
        PinnedState {
            list: PinnedList { items, revision: 4 },
            ..PinnedState::default()
        }
    }

    /// Opens a window on `pinned`, draws it and runs `step` in it. Answers
    /// with whether the section is drawn and what was sent.
    fn run(
        cx: &mut TestAppContext,
        pinned: PinnedState,
        step: impl FnOnce(&mut Window, &mut App),
    ) -> (bool, Vec<PinnedAction>) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                    cx.new(|_| View {
                        pinned,
                        send: Rc::new(move |action, _| log.borrow_mut().push(action)),
                    })
                })
            })
            .expect("the test window opens");
        let drawn = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let drawn = window.try_find("sidebar-pinned").is_some();
                step(window, cx);
                drawn
            })
            .expect("the test window is open");
        (drawn, sent.take())
    }

    fn drawn(cx: &mut TestAppContext, pinned: PinnedState) -> bool {
        run(cx, pinned, |_, _| {}).0
    }

    /// Points at the row `row`, so its buttons show, and clicks `id`.
    fn sent_by_clicking(
        cx: &mut TestAppContext,
        pinned: PinnedState,
        row: &'static str,
        id: &'static str,
    ) -> Vec<PinnedAction> {
        run(cx, pinned, |window, cx| {
            window.hover(row, cx);
            window.click(id, cx);
        })
        .1
    }

    fn two_pins() -> PinnedState {
        state(vec![
            pin(Thing::Session(SessionId::new("s1")), "Ledger schema migration", true),
            pin(Thing::Channel(ChannelId::new("c1")), "payments-rewrite", false),
        ])
    }

    #[gpui_kit::test]
    fn clicking_a_pin_opens_it(cx: &mut TestAppContext) {
        assert_eq!(
            sent_by_clicking(cx, two_pins(), "sidebar-pinned-1", "sidebar-pinned-1"),
            vec![PinnedAction::Open(1)]
        );
    }

    #[gpui_kit::test]
    fn the_cross_unpins_and_does_not_open(cx: &mut TestAppContext) {
        assert_eq!(
            sent_by_clicking(cx, two_pins(), "sidebar-pinned-0", "sidebar-pinned-0-unpin"),
            vec![PinnedAction::Toggle {
                thing: Thing::Session(SessionId::new("s1")),
                pin: false,
            }]
        );
    }

    #[gpui_kit::test]
    fn the_up_arrow_moves_a_pin_one_place_up(cx: &mut TestAppContext) {
        assert_eq!(
            sent_by_clicking(cx, two_pins(), "sidebar-pinned-1", "sidebar-pinned-1-up"),
            vec![PinnedAction::Move { from: 1, to: 0 }]
        );
    }

    #[gpui_kit::test]
    fn dropping_a_pin_on_another_moves_it_there(cx: &mut TestAppContext) {
        let sent = run(cx, two_pins(), |window, cx| {
            window.drag_to("sidebar-pinned-0", "sidebar-pinned-1", cx);
        })
        .1;
        assert_eq!(sent, vec![PinnedAction::Move { from: 0, to: 1 }]);
    }

    #[gpui_kit::test]
    fn the_section_hides_when_settings_say_so(cx: &mut TestAppContext) {
        let hidden = PinnedState {
            shown: false,
            ..two_pins()
        };
        assert!(!drawn(cx, hidden));
    }

    #[gpui_kit::test]
    fn the_section_hides_when_nothing_is_pinned(cx: &mut TestAppContext) {
        assert!(!drawn(cx, state(Vec::new())));
    }

    #[gpui_kit::test]
    fn a_failure_shows_even_with_nothing_pinned(cx: &mut TestAppContext) {
        let failed = PinnedState {
            error: Some("the host is offline".to_owned()),
            ..state(Vec::new())
        };
        assert!(drawn(cx, failed));
    }

    #[gpui_kit::test]
    fn error_banner_offers_retry_and_dismiss(cx: &mut TestAppContext) {
        let failed = PinnedState {
            error: Some("the host is offline".to_owned()),
            ..two_pins()
        };
        let expected_list = failed.list.clone();
        let retried = run(cx, failed.clone(), |window, cx| {
            window.click("sidebar-pinned-retry", cx);
        })
        .1;
        assert_eq!(retried, vec![PinnedAction::Start]);

        let dismissed = run(cx, failed, |window, cx| {
            window.click("sidebar-pinned-dismiss", cx);
        })
        .1;
        assert_eq!(dismissed, vec![PinnedAction::Changed(expected_list)]);
    }
}

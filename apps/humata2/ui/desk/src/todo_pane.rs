//! Live To-do List-only split pane beside Channel Desk (DESKTODO).
//! No 5-column Kanban, no Columns|List switcher, no Live badge, no duplicate `+`.

use crate::{
    bridge::Ui,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, channel as channel_feature};
use humata2_clients::model::{Channel, ChannelId, Item, ItemStatus, Scope};
use humata2_desk::{ChannelTodoPane, DeskAction, clamp_todo_pane_width};
use std::rc::Rc;

/// Maximum number of Backlog items shown inline before capping.
pub const BACKLOG_PREVIEW_LIMIT: usize = 3;

/// Grouped channel To-do items for the List-only split pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoPaneGroups<'a> {
    pub in_progress: Vec<&'a Item>,
    pub in_review: Vec<&'a Item>,
    pub next: Vec<&'a Item>,
    pub backlog_shown: Vec<&'a Item>,
    pub backlog_total: usize,
    pub backlog_hidden: usize,
    pub done: Vec<&'a Item>,
    pub open_count: usize,
}

/// Groups a channel's To-do items in the List-only split pane order, applying
/// an optional case-insensitive search query and capping Backlog to 3 items.
#[must_use]
pub fn group_channel_items<'a>(
    items: &'a [Item],
    channel_id: &humata2_clients::model::ChannelId,
    query: &str,
) -> TodoPaneGroups<'a> {
    let q = query.trim().to_lowercase();
    let mut in_progress = Vec::new();
    let mut in_review = Vec::new();
    let mut next = Vec::new();
    let mut backlog = Vec::new();
    let mut done = Vec::new();

    for item in items {
        if item.scope != Scope::Channel(channel_id.clone()) {
            continue;
        }
        if !q.is_empty() && !item.title.to_lowercase().contains(&q) {
            continue;
        }
        match item.status {
            ItemStatus::InProgress => in_progress.push(item),
            ItemStatus::InReview => in_review.push(item),
            ItemStatus::Todo => next.push(item),
            ItemStatus::Backlog => backlog.push(item),
            ItemStatus::Done => done.push(item),
            ItemStatus::Blocked | ItemStatus::WontDo => {}
        }
    }

    let backlog_total = backlog.len();
    let backlog_hidden = backlog_total.saturating_sub(BACKLOG_PREVIEW_LIMIT);
    let backlog_shown = backlog.into_iter().take(BACKLOG_PREVIEW_LIMIT).collect();
    let open_count = in_progress.len() + in_review.len() + next.len() + backlog_total;

    TodoPaneGroups {
        in_progress,
        in_review,
        next,
        backlog_shown,
        backlog_total,
        backlog_hidden,
        done,
        open_count,
    }
}

/// Header title for the To-do List-only split pane.
#[must_use]
pub fn header_title(open_count: usize) -> String {
    format!("To-do · {open_count}")
}

fn section_head(look: Look, label: String) -> Div {
    div()
        .px(px(12.))
        .pt(px(10.))
        .pb(px(4.))
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(label)
}

fn item_row(item: &Item, look: Look) -> AnyElement {
    row()
        .id(SharedString::from(format!(
            "desk-todo-item-{}",
            item.id.as_str()
        )))
        .role(Role::ListItem)
        .aria_label(item.title.clone())
        .px(px(12.))
        .py(px(6.))
        .gap(px(8.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .text_color(rgb(look.text))
                .child(item.title.clone())
                .test_text(item.title.clone()),
        )
        .test_support()
        .into_any_element()
}

struct TodoSearchState {
    input: Entity<InputState>,
    last_synced: String,
    _sub: Subscription,
}

#[derive(IntoElement)]
struct TodoSearchBar {
    ui: Rc<Ui>,
    channel: ChannelId,
    query: String,
    look: Look,
}

impl RenderOnce for TodoSearchBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("channel-desk-todo-search-{}", self.channel.as_str()));
        let ui_sub = self.ui.clone();
        let ch_sub = self.channel.clone();
        let initial_query = self.query.clone();
        let state = window.use_keyed_state(key, cx, move |window, cx| {
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder("Filter To-do items…");
                if !initial_query.is_empty() {
                    s.set_value(initial_query.clone(), window, cx);
                }
                s.focus(window, cx);
                s
            });
            let sub = cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    ui_sub.send(
                        AppAction::Desk(DeskAction::SetTodoPaneSearchQuery {
                            channel: ch_sub.clone(),
                            query: input.read(cx).value().to_string(),
                        }),
                        cx,
                    );
                }
            });
            TodoSearchState {
                input,
                last_synced: initial_query,
                _sub: sub,
            }
        });

        let input = {
            let s = state.read(cx);
            let need_sync = s.last_synced != self.query
                && s.input.read(cx).value().as_ref() != self.query.as_str();
            let input = s.input.clone();
            if need_sync {
                let q = self.query.clone();
                input.update(cx, |i, cx| i.set_value(q, window, cx));
                state.update(cx, |s, _| s.last_synced = self.query.clone());
            } else if s.last_synced != self.query {
                state.update(cx, |s, _| s.last_synced = self.query.clone());
            }
            input
        };

        let esc_ui = self.ui.clone();
        let esc_ch = self.channel.clone();
        let esc_input = input.clone();
        row()
            .id("channel-desk-todo-search-bar")
            .px(px(12.))
            .py(px(6.))
            .border_b_1()
            .border_color(rgb(self.look.line))
            .bg(rgb(self.look.panel))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let mods = event.keystroke.modifiers;
                if event.keystroke.key.as_str() == "escape"
                    && !mods.shift
                    && !mods.alt
                    && !mods.platform
                    && !mods.control
                {
                    cx.stop_propagation();
                    if !esc_input.read(cx).value().is_empty() {
                        esc_input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                    }
                    esc_ui.send(
                        AppAction::Desk(DeskAction::SetTodoPaneSearchQuery {
                            channel: esc_ch.clone(),
                            query: String::new(),
                        }),
                        cx,
                    );
                    esc_ui.send(
                        AppAction::Desk(DeskAction::ToggleTodoPaneSearch(esc_ch.clone())),
                        cx,
                    );
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Input::new(&input)
                            .cleanable(true)
                            .small()
                            .min_h(px(26.))
                            .rounded(px(6.))
                            .bg(rgb(self.look.card))
                            .border_color(rgb(self.look.line))
                            .text_size(px(12.)),
                    ),
            )
            .test_support()
    }
}

/// Renders the live To-do List-only split pane beside Channel Desk.
pub fn pane(
    ui: &Rc<Ui>,
    state: &AppState,
    channel: &Channel,
    pane_state: &ChannelTodoPane,
    look: Look,
) -> AnyElement {
    let groups = group_channel_items(&state.workspace.items, &channel.id, &pane_state.search_query);
    let width = clamp_todo_pane_width(pane_state.width);
    let title = header_title(groups.open_count);

    let search_ui = ui.clone();
    let search_channel = channel.id.clone();
    let popout_ui = ui.clone();
    let close_ui = ui.clone();
    let close_channel = channel.id.clone();
    let done_ui = ui.clone();
    let done_channel = channel.id.clone();
    let backlog_full_ui = ui.clone();

    let header = row()
        .id("channel-desk-todo-pane-header")
        .h(px(36.))
        .px(px(12.))
        .gap(px(8.))
        .flex_shrink_0()
        .bg(rgb(look.panel))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .child(title.clone())
                .test_text(title),
        )
        .child(
            div()
                .id("channel-desk-todo-search-toggle")
                .role(Role::Button)
                .aria_label("Search To-do")
                .cursor_pointer()
                .px(px(4.))
                .text_color(rgb(look.secondary))
                .child("🔍")
                .on_click(move |_, _, cx| {
                    search_ui.send(
                        AppAction::Desk(DeskAction::ToggleTodoPaneSearch(search_channel.clone())),
                        cx,
                    );
                })
                .test_support(),
        )
        .child(
            div()
                .id("channel-desk-todo-open-full")
                .role(Role::Button)
                .aria_label("Open full To-do")
                .cursor_pointer()
                .px(px(4.))
                .text_color(rgb(look.secondary))
                .child("↗")
                .on_click(move |_, _, cx| {
                    popout_ui.send(
                        AppAction::Channel(channel_feature::Action::Tab(
                            channel_feature::Tab::Board,
                        )),
                        cx,
                    );
                })
                .test_support(),
        )
        .child(
            div()
                .id("channel-desk-todo-close")
                .role(Role::Button)
                .aria_label("Close To-do pane")
                .cursor_pointer()
                .px(px(4.))
                .text_color(rgb(look.secondary))
                .child("✕")
                .on_click(move |_, _, cx| {
                    close_ui.send(
                        AppAction::Desk(DeskAction::ToggleTodoPane(close_channel.clone())),
                        cx,
                    );
                })
                .test_support(),
        );

    let search_bar = pane_state.search_open.then(|| TodoSearchBar {
        ui: ui.clone(),
        channel: channel.id.clone(),
        query: pane_state.search_query.clone(),
        look,
    });

    let mut body = col()
        .id("channel-desk-todo-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll();

    if !groups.in_progress.is_empty() {
        body = body
            .child(section_head(
                look,
                format!("IN PROGRESS · {}", groups.in_progress.len()),
            ))
            .children(groups.in_progress.iter().map(|i| item_row(i, look)));
    }
    if !groups.in_review.is_empty() {
        body = body
            .child(section_head(
                look,
                format!("IN REVIEW · {}", groups.in_review.len()),
            ))
            .children(groups.in_review.iter().map(|i| item_row(i, look)));
    }
    if !groups.next.is_empty() {
        body = body
            .child(section_head(look, format!("NEXT · {}", groups.next.len())))
            .children(groups.next.iter().map(|i| item_row(i, look)));
    }
    if groups.backlog_total > 0 {
        body = body
            .child(section_head(
                look,
                format!("BACKLOG · {}", groups.backlog_total),
            ))
            .children(groups.backlog_shown.iter().map(|i| item_row(i, look)));
        if groups.backlog_hidden > 0 {
            let hidden_label = format!("Show {} more in Backlog", groups.backlog_hidden);
            body = body.child(
                row()
                    .px(px(12.))
                    .py(px(6.))
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(hidden_label)
                    .child(
                        div()
                            .id("channel-desk-todo-backlog-full")
                            .role(Role::Button)
                            .aria_label("Open full To-do")
                            .cursor_pointer()
                            .text_color(rgb(look.accent))
                            .child("Open full To-do →")
                            .on_click(move |_, _, cx| {
                                backlog_full_ui.send(
                                    AppAction::Channel(channel_feature::Action::Tab(
                                        channel_feature::Tab::Board,
                                    )),
                                    cx,
                                );
                            })
                            .test_support(),
                    ),
            );
        }
    }

    let done_label = if pane_state.done_expanded {
        format!("▾ Done · {}", groups.done.len())
    } else {
        format!("▸ Done · {}", groups.done.len())
    };
    body = body.child(
        div()
            .id("channel-desk-todo-done-toggle")
            .role(Role::Button)
            .aria_label(done_label.clone())
            .cursor_pointer()
            .px(px(12.))
            .py(px(8.))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(look.secondary))
            .child(done_label)
            .on_click(move |_, _, cx| {
                done_ui.send(
                    AppAction::Desk(DeskAction::ToggleTodoPaneDone(done_channel.clone())),
                    cx,
                );
            })
            .test_support(),
    );
    if pane_state.done_expanded {
        body = body.children(groups.done.iter().map(|i| item_row(i, look)));
    }

    col()
        .id("channel-desk-todo-pane")
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .overflow_hidden()
        .child(header)
        .children(search_bar)
        .child(body)
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_clients::model::{ChannelId, ItemId, ItemKind};

    fn make_item(id: &str, channel: &str, title: &str, status: ItemStatus) -> Item {
        Item {
            id: ItemId::new(id),
            scope: Scope::Channel(ChannelId::new(channel)),
            kind: ItemKind::Task,
            title: title.to_owned(),
            status,
            depends_on: Vec::new(),
            session: None,
            parent: None,
            revision: 0,
        }
    }

    #[::core::prelude::v1::test]
    fn groups_channel_items_and_caps_backlog_to_three() {
        let ch = ChannelId::new("payments");
        let items = vec![
            make_item("1", "payments", "Active task", ItemStatus::InProgress),
            make_item("2", "payments", "Review PR", ItemStatus::InReview),
            make_item("3", "payments", "Next up", ItemStatus::Todo),
            make_item("4", "payments", "Backlog 1", ItemStatus::Backlog),
            make_item("5", "payments", "Backlog 2", ItemStatus::Backlog),
            make_item("6", "payments", "Backlog 3", ItemStatus::Backlog),
            make_item("7", "payments", "Backlog 4", ItemStatus::Backlog),
            make_item("8", "payments", "Backlog 5", ItemStatus::Backlog),
            make_item("9", "payments", "Shipped", ItemStatus::Done),
            make_item("10", "other", "Other channel", ItemStatus::InProgress),
        ];

        let grouped = group_channel_items(&items, &ch, "");
        assert_eq!(grouped.in_progress.len(), 1);
        assert_eq!(grouped.in_review.len(), 1);
        assert_eq!(grouped.next.len(), 1);
        assert_eq!(grouped.backlog_total, 5);
        assert_eq!(grouped.backlog_shown.len(), 3);
        assert_eq!(grouped.backlog_hidden, 2);
        assert_eq!(grouped.done.len(), 1);
        assert_eq!(grouped.open_count, 8);
        assert_eq!(header_title(grouped.open_count), "To-do · 8");

        let filtered = group_channel_items(&items, &ch, "backlog 4");
        assert_eq!(filtered.backlog_total, 1);
        assert_eq!(filtered.backlog_shown.len(), 1);
        assert_eq!(filtered.backlog_hidden, 0);
        assert_eq!(filtered.open_count, 1);
    }
}

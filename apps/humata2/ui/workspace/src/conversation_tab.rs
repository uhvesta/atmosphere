//! A session's read-only Conversation tab (D186): the host mirror's entries
//! around a search hit, grouped by turn, landing on the hit's entry.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::base::{Disableable as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::history::{Turn as Entry, TurnRole};
use humata2_clients::model::{Agent, Session, SessionId};
use humata2_session::{Conversation, ConversationRead, ConversationTarget, SessionAction};

use humata2_ui_core::bridge::Ui;
use humata2_ui_core::theme::{self, Look};
use humata2_ui_core::widgets::{self, col, row};

/// Sends a Conversation action to the store.
pub type SendConversation = Rc<dyn Fn(SessionAction, &mut App)>;

pub use humata2_ui_core::bridge::ConversationScroll;

/// The Conversation tab of `session`, from what the store has read for it.
pub fn body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
    scroll: &ConversationScroll,
) -> AnyElement {
    let ui = ui.clone();
    let send: SendConversation =
        Rc::new(move |action, cx: &mut App| ui.send(AppAction::Session(action), cx));
    tab(
        state.session.conversations.get(&session.id),
        session.agent,
        &session.id,
        look,
        scroll,
        &send,
    )
}

/// Where the loading sweep sits in the Conversation tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Where {
    /// At the top, above the earlier turns.
    Top,
    /// At the bottom, below the later turns.
    Bottom,
}

/// Where the sweep shows: at the top before anything is read and while the
/// earlier turns (or a retried first read) load, at the bottom while the later
/// turns load, and nowhere when no read runs.
fn sweep_at(conversation: Option<&Conversation>) -> Option<Where> {
    match conversation {
        None => Some(Where::Top),
        Some(conversation) => match conversation.loading {
            Some(ConversationRead::Around | ConversationRead::Earlier) => Some(Where::Top),
            Some(ConversationRead::Later) => Some(Where::Bottom),
            None => None,
        },
    }
}

/// The loading sweep of the tab: a full-width line, no words.
fn sweep(look: Look) -> AnyElement {
    div()
        .id("conversation-loading")
        .role(Role::ProgressIndicator)
        .aria_label("Loading")
        .test_support()
        .child(widgets::progress_bar(
            look,
            None,
            widgets::BarMetrics::line(look),
            "conversation-sweep",
        ))
        .into_any_element()
}

fn tab(
    conversation: Option<&Conversation>,
    agent: Agent,
    session: &SessionId,
    look: Look,
    scroll: &ConversationScroll,
    send: &SendConversation,
) -> AnyElement {
    let mut children: Vec<AnyElement> = Vec::new();
    // The child the hit's turn is at, to scroll it into view.
    let mut marked_child = None;
    let at = sweep_at(conversation);
    match conversation {
        None => children.push(sweep(look)),
        Some(conversation) => {
            let loading = conversation.loading;
            if let Some(error) = &conversation.error {
                children.push(
                    row()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(theme::RED))
                                .child(error.clone()),
                        )
                        .child(
                            button(
                                "conversation-retry",
                                "Retry",
                                SessionAction::RetryConversation,
                                send,
                            )
                            .outline()
                            .h(px(24.))
                            .px(px(11.))
                            .into_any_element(),
                        )
                        .into_any_element(),
                );
            }
            if at == Some(Where::Top) {
                children.push(sweep(look));
            }
            if conversation.can_show_earlier() {
                children.push(
                    button(
                        "conversation-earlier",
                        "Show earlier turns",
                        SessionAction::ShowEarlierTurns,
                        send,
                    )
                    .outline()
                    .disabled(loading == Some(ConversationRead::Earlier))
                    .into_any_element(),
                );
            }
            for (turn, entries) in turns(&conversation.entries) {
                let holds_hit = entries.iter().any(|e| e.seq == conversation.target.seq);
                if holds_hit {
                    marked_child = Some(children.len());
                }
                children.push(turn_block(turn, &entries, conversation.target.seq, agent, look));
            }
            if at == Some(Where::Bottom) {
                children.push(sweep(look));
            }
            if conversation.can_show_later() {
                children.push(
                    button(
                        "conversation-later",
                        "Show later turns",
                        SessionAction::ShowLaterTurns,
                        send,
                    )
                    .outline()
                    .disabled(loading == Some(ConversationRead::Later))
                    .into_any_element(),
                );
            }
            if let Some(index) = marked_child {
                let key = (session.clone(), conversation.target);
                if scroll.shown.borrow().as_ref() != Some(&key) {
                    scroll.handle.scroll_to_item(index);
                    *scroll.shown.borrow_mut() = Some(key);
                }
            }
        }
    }
    let list = col()
        .id("conversation-tab")
        .role(Role::Document)
        .aria_label("Conversation")
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll.handle)
        .bg(rgb(look.content))
        .text_color(rgb(look.text))
        .p(px(18.))
        .gap(px(12.))
        .children(children)
        .test_support();
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .child(list)
        .vertical_scrollbar(&scroll.handle)
        .into_any_element()
}

/// A button that sends `action`.
fn button(
    id: &'static str,
    label: &'static str,
    action: SessionAction,
    send: &SendConversation,
) -> Button {
    let send = send.clone();
    Button::new(id)
        .label(label)
        .small()
        .on_click(move |_, _, cx| send(action.clone(), cx))
}

/// The entries grouped by turn, in the order they were read.
fn turns(entries: &[Entry]) -> Vec<(u32, Vec<&Entry>)> {
    let mut turns: Vec<(u32, Vec<&Entry>)> = Vec::new();
    for entry in entries {
        match turns.last_mut() {
            Some((turn, group)) if *turn == entry.turn => group.push(entry),
            _ => turns.push((entry.turn, vec![entry])),
        }
    }
    turns
}

/// Compact per-entry Copy button with local "Copied" feedback.
#[derive(IntoElement)]
struct CopyEntryButton {
    id: SharedString,
    text: String,
}

impl RenderOnce for CopyEntryButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let copied = window.use_keyed_state(
            SharedString::from(format!("{}-copied", self.id)),
            cx,
            |_, _| false,
        );
        let is_copied = *copied.read(cx);
        let text = self.text;
        Button::new(self.id)
            .label(if is_copied { "Copied" } else { "Copy" })
            .xsmall()
            .on_click(move |_, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                copied.update(cx, |v, _| *v = true);
                let copied_reset = copied.clone();
                window
                    .spawn(cx, async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(1500))
                            .await;
                        let _ = copied_reset.update(cx, |v, cx| {
                            *v = false;
                            cx.notify();
                        });
                    })
                    .detach();
                window.refresh();
            })
    }
}

/// One turn: its header and each entry, with the hit's entry marked.
fn turn_block(turn: u32, entries: &[&Entry], marked: u64, agent: Agent, look: Look) -> AnyElement {
    let header = if turn == 0 {
        "Before the first prompt".to_owned()
    } else {
        format!("Turn {turn}")
    };
    let mut block = col()
        .id(ElementId::Name(format!("conversation-turn-{turn}").into()))
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(header),
        );
    for entry in entries {
        let who = match entry.role {
            TurnRole::User => "You".to_owned(),
            TurnRole::Agent => agent.command().to_owned(),
            TurnRole::Tool => "Tool".to_owned(),
        };
        let mut text = div().child(entry.text.clone());
        if entry.role == TurnRole::Tool {
            text = text.font_family("Menlo").text_size(px(12.));
        }
        let item = col()
            .gap(px(2.))
            .child(
                row()
                    .justify_between()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(who),
                    )
                    .child(CopyEntryButton {
                        id: SharedString::from(format!("conversation-copy-{}", entry.seq)),
                        text: entry.text.clone(),
                    }),
            )
            .child(text.test_text(entry.text.clone()));
        block = block.child(if entry.seq == marked {
            item.id("conversation-marked")
                .aria_selected(true)
                .border_l_2()
                .border_color(rgb(look.accent))
                .pl(px(10.))
                .into_any_element()
        } else {
            item.into_any_element()
        });
    }
    block.into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    // The glob above brings GPUI's `test` macro, which expands `#[test]` into
    // itself; `#[test]` is the standard one.
    use ::core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt as _;

    struct View {
        conversation: Option<Conversation>,
        scroll: ConversationScroll,
        send: SendConversation,
    }

    impl Render for View {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(tab(
                self.conversation.as_ref(),
                Agent::Claude,
                &SessionId("s-1".to_owned()),
                Look::of(cx),
                &self.scroll,
                &self.send,
            ))
        }
    }

    /// A conversation aimed at turn 20 that has read 18 to 22.
    fn conversation() -> Conversation {
        Conversation {
            target: ConversationTarget { turn: 20, seq: 41 },
            first: 18,
            last: 22,
            entries: vec![Entry {
                seq: 41,
                turn: 20,
                role: TurnRole::User,
                text: "move the Ledger reads".to_owned(),
            }],
            loading: None,
            error: None,
            failed: None,
            at_end: false,
        }
    }

    /// Opens a window on `conversation`, clicks `id` and answers with what
    /// was sent.
    fn sent_by_clicking(
        cx: &mut TestAppContext,
        conversation: Option<Conversation>,
        id: &'static str,
    ) -> Vec<SessionAction> {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                    cx.new(|_| View {
                        conversation,
                        scroll: ConversationScroll::default(),
                        send: Rc::new(move |action, _| log.borrow_mut().push(action)),
                    })
                })
            })
            .expect("the test window opens");
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .expect("the test window is open");
        sent.take()
    }

    #[gpui_kit::test]
    fn show_earlier_turns_asks_for_the_turns_before(cx: &mut TestAppContext) {
        let sent = sent_by_clicking(cx, Some(conversation()), "conversation-earlier");
        assert_eq!(sent, vec![SessionAction::ShowEarlierTurns]);
    }

    #[gpui_kit::test]
    fn retry_repeats_a_failed_read(cx: &mut TestAppContext) {
        let failed = Conversation {
            error: Some("the host's copy didn't answer".to_owned()),
            failed: Some((ConversationRead::Around, 18, 22)),
            ..conversation()
        };
        let sent = sent_by_clicking(cx, Some(failed), "conversation-retry");
        assert_eq!(sent, vec![SessionAction::RetryConversation]);
    }

    fn reading(read: ConversationRead) -> Conversation {
        Conversation {
            loading: Some(read),
            ..conversation()
        }
    }

    #[test]
    fn nothing_read_yet_sweeps_the_top() {
        assert_eq!(sweep_at(None), Some(Where::Top));
    }

    #[test]
    fn reading_the_earlier_turns_sweeps_the_top() {
        assert_eq!(sweep_at(Some(&reading(ConversationRead::Earlier))), Some(Where::Top));
    }

    #[test]
    fn reading_the_later_turns_sweeps_the_bottom() {
        assert_eq!(sweep_at(Some(&reading(ConversationRead::Later))), Some(Where::Bottom));
    }

    #[test]
    fn a_retried_first_read_sweeps_the_top() {
        assert_eq!(sweep_at(Some(&reading(ConversationRead::Around))), Some(Where::Top));
    }

    #[test]
    fn a_conversation_that_is_not_reading_shows_no_sweep() {
        assert_eq!(sweep_at(Some(&conversation())), None);
    }
}

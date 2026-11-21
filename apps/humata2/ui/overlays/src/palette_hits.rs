//! The palette's Conversations section (LD-23, LD-24, frame 30): the turns
//! of every session that match the query, with where each one is. Picking a
//! hit opens its session at that turn.

use std::ops::Range;
use std::rc::Rc;

use humata2_ui_core::widgets::{col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_clients::history::SearchHit;
use humata2_clients::model::MachineId;
use humata2_find::FindAction;
#[cfg(test)]
use humata2_find::FindState;

/// Sends a search action to the store, such as
/// `move |action, cx| ui.send(AppAction::Find(action), cx)`.
pub(crate) type SendFind = Rc<dyn Fn(FindAction, &mut App)>;

/// The fewest characters, spaces not counted, the search runs for.
const SHORTEST_QUERY: usize = 2;

/// Whether `query` is long enough to search.
pub(crate) fn searchable(query: &str) -> bool {
    query.chars().filter(|c| !c.is_whitespace()).count() >= SHORTEST_QUERY
}

/// The Conversations section for `find`, or nothing unless the search has
/// hits, a next page or an error to show. `selected` is the hit the
/// palette's cursor is on, as a place in `find.hits`; `offline` says whether
/// a machine is offline now, so its hits say how old the host's copy is. A
/// click on a hit sends [`FindAction::Open`] with its place; the palette
/// sends the same for the selected hit when Enter is pressed.
#[cfg(test)]
pub(crate) fn conversations(
    find: &FindState,
    selected: Option<usize>,
    offline: &dyn Fn(&MachineId) -> bool,
    look: Look,
    send: &SendFind,
) -> Option<AnyElement> {
    if find.hits.is_empty() && find.next_page.is_empty() && find.error.is_none() {
        return None;
    }
    let mut section = col()
        .id("palette-conversations")
        .role(Role::Group)
        .aria_label("Conversations")
        .flex_shrink_0()
        .child(
            row()
                .flex_shrink_0()
                .px(px(16.))
                .pt(px(8.))
                .pb(px(3.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(div().flex_1().child("Conversations"))
                .when(find.loading && !find.hits.is_empty(), |x| {
                    x.child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(look.tertiary))
                            .child("Searching…"),
                    )
                }),
        );
    if let Some(error) = &find.error {
        section = section.child(note(
            "palette-hits-error",
            format!("Search didn't answer: {error}. Edit the query to try again."),
            humata2_ui_core::theme::RED,
            true,
        ));
    }
    let words = words(&find.query);
    for (index, hit) in find.hits.iter().enumerate() {
        section = section.child(hit_row(
            hit,
            index,
            selected == Some(index),
            offline(&hit.machine),
            &words,
            look,
            send,
        ));
    }
    if !find.next_page.is_empty() {
        section = section.child(more_row(selected == Some(find.hits.len()), look, send));
    }
    Some(section.into_any_element())
}

/// Flat rows for the Conversations section so `palette-entries` can scroll
/// directly to any selected hit with `ScrollHandle::scroll_to_item`.
/// Each entry in `hits` is `(raw_index_in_find_hits, hit)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn conversation_rows(
    hits: &[(usize, &SearchHit)],
    query: &str,
    next_page: &str,
    loading: bool,
    error: Option<&str>,
    selected: Option<usize>,
    offline: &dyn Fn(&MachineId) -> bool,
    look: Look,
    send: &SendFind,
) -> (Vec<AnyElement>, Option<usize>) {
    if hits.is_empty() && next_page.is_empty() && error.is_none() {
        return (Vec::new(), None);
    }
    let mut rows = Vec::new();
    let mut selected_offset = None;
    rows.push(
        row()
            .id("palette-conversations")
            .flex_shrink_0()
            .px(px(16.))
            .pt(px(8.))
            .pb(px(3.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(look.secondary))
            .child(div().flex_1().child("Conversations"))
            .when(loading && !hits.is_empty(), |x| {
                x.child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(look.tertiary))
                        .child("Searching…"),
                )
            })
            .into_any_element(),
    );
    if let Some(error) = error {
        rows.push(
            note(
                "palette-hits-error",
                format!("Search didn't answer: {error}. Edit the query to try again."),
                humata2_ui_core::theme::RED,
                true,
            )
            .into_any_element(),
        );
    }
    let words = words(query);
    for (visible_index, (raw_index, hit)) in hits.iter().enumerate() {
        let is_selected = selected == Some(visible_index);
        if is_selected {
            selected_offset = Some(rows.len());
        }
        rows.push(hit_row(
            hit,
            *raw_index,
            is_selected,
            offline(&hit.machine),
            &words,
            look,
            send,
        ));
    }
    if !next_page.is_empty() {
        let is_selected = selected == Some(hits.len());
        if is_selected {
            selected_offset = Some(rows.len());
        }
        rows.push(more_row(is_selected, look, send));
    }
    (rows, selected_offset)
}

/// The row that asks for the next page of hits. Enter on it does the same
/// through the palette (`Delegate::MoreHits`).
fn more_row(selected: bool, look: Look, send: &SendFind) -> AnyElement {
    let send = send.clone();
    row()
        .id("palette-hits-more")
        .role(Role::ListBoxOption)
        .aria_label("More conversations")
        .aria_selected(selected)
        .flex_shrink_0()
        .h(px(28.))
        .mx(px(6.))
        .px(px(10.))
        .rounded(px(6.))
        .cursor_pointer()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .when(selected, |x| {
            x.bg(rgb(look.selection)).text_color(rgb(0xffffff))
        })
        .child("More conversations")
        .on_click(move |_, _, cx| send(FindAction::More, cx))
        .test_support()
        .into_any_element()
}

/// A line in place of hits: nothing found, still looking, or what failed.
fn note(id: &'static str, text: String, color: u32, alert: bool) -> impl IntoElement {
    div()
        .id(id)
        .role(if alert { Role::Alert } else { Role::Status })
        .aria_label(text.clone())
        .flex_shrink_0()
        .px(px(16.))
        .py(px(8.))
        .text_size(px(12.))
        .text_color(rgb(color))
        .child(text)
}

fn hit_row(
    hit: &SearchHit,
    index: usize,
    selected: bool,
    offline: bool,
    words: &[String],
    look: Look,
    send: &SendFind,
) -> AnyElement {
    let color = if selected { 0xffffff } else { look.secondary };
    let faint = if selected { 0xffffff } else { look.tertiary };
    let strong = if selected { 0xffffff } else { look.text };
    let place = format!(
        "# {} · {} · turn {}",
        hit.channel_name, hit.machine_name, hit.turn
    );
    let label = format!("{}, {}", hit.title, place);
    let emphasis = HighlightStyle {
        font_weight: Some(FontWeight::SEMIBOLD),
        color: Some(rgb(strong).into()),
        ..Default::default()
    };
    let snippet = StyledText::new(hit.snippet.clone()).with_highlights(
        emphasis_ranges(&hit.snippet, words)
            .into_iter()
            .map(|range| (range, emphasis)),
    );
    let send = send.clone();
    col()
        .id(ElementId::Name(format!("palette-hit-{index}").into()))
        .role(Role::ListBoxOption)
        .aria_label(label)
        .aria_selected(selected)
        .flex_shrink_0()
        .mx(px(6.))
        .px(px(10.))
        .py(px(6.))
        .gap(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .when(selected, |x| {
            x.bg(rgb(look.selection)).text_color(rgb(0xffffff))
        })
        .child(
            row()
                .gap(px(9.))
                .child(chrome::icon(IconName::MessageSquareText, color, 15.))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(hit.title.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.5))
                        .text_color(rgb(color))
                        .child(place),
                )
                .when(offline, |x| {
                    x.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(rgb(faint))
                            .child(format!("as of {}", as_of(&hit.as_of))),
                    )
                }),
        )
        .child(
            div()
                .pl(px(23.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(11.5))
                .text_color(rgb(color))
                .child(snippet),
        )
        .on_click(move |_, _, cx| send(FindAction::Open(index), cx))
        .test_support()
        .into_any_element()
}

/// The query's words, lowercased.
fn words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect()
}

/// Where `words` appear in `text`, ignoring case, as sorted byte ranges
/// with overlaps joined. The host matches word prefixes, so a word counts
/// wherever it starts.
fn emphasis_ranges(text: &str, words: &[String]) -> Vec<Range<usize>> {
    let mut found: Vec<Range<usize>> = Vec::new();
    for word in words.iter().filter(|word| !word.is_empty()) {
        for (start, _) in text.char_indices() {
            if let Some(end) = match_at(text, start, word) {
                found.push(start..end);
            }
        }
    }
    found.sort_by_key(|range| range.start);
    let mut joined: Vec<Range<usize>> = Vec::new();
    for range in found {
        match joined.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => joined.push(range),
        }
    }
    joined
}

/// The end of `word` (already lowercase) when it starts at byte `start` of
/// `text`, ignoring case.
fn match_at(text: &str, start: usize, word: &str) -> Option<usize> {
    let rest = text.get(start..)?;
    let mut wanted = word.chars().peekable();
    let mut end = start;
    for (offset, c) in rest.char_indices() {
        if wanted.peek().is_none() {
            break;
        }
        for lower in c.to_lowercase() {
            if wanted.next() != Some(lower) {
                return None;
            }
        }
        end = start + offset + c.len_utf8();
    }
    wanted.peek().is_none().then_some(end)
}

/// An RFC 3339 time as people read it: `2026-11-15 12:52`. Anything else is
/// shown as it is.
fn as_of(when: &str) -> String {
    match (when.get(..10), when.get(10..11), when.get(11..16)) {
        (Some(day), Some("T"), Some(time)) => format!("{day} {time}"),
        _ => when.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::Arc;

    use super::*;
    // The glob above brings GPUI's `test` macro; `#[test]` is the standard one.
    use ::core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt as _;
    use humata2_clients::model::{ChannelId, SessionId};

    fn hit(title: &str, turn: u32) -> SearchHit {
        SearchHit {
            session: SessionId(format!("s-{title}")),
            channel: ChannelId("c1".to_owned()),
            channel_name: "payments-rewrite".to_owned(),
            title: title.to_owned(),
            machine: MachineId("devbox-1".to_owned()),
            machine_name: "devbox-1".to_owned(),
            agent: "claude".to_owned(),
            turn,
            seq: u64::from(turn),
            snippet: "move the Ledger reads over".to_owned(),
            as_of: "2026-11-15T12:52:00Z".to_owned(),
        }
    }

    #[test]
    fn every_word_is_emphasised_wherever_it_starts_whatever_its_case() {
        let ranges = emphasis_ranges("Ledger reads; ledger_entries", &words("ledg READS"));
        assert_eq!(ranges, vec![0..4, 7..12, 14..18]);
    }

    #[test]
    fn overlapping_words_are_emphasised_as_one_run() {
        assert_eq!(emphasis_ranges("ledger", &words("led edge")), vec![0..5]);
    }

    #[test]
    fn a_word_that_is_not_there_emphasises_nothing() {
        assert!(emphasis_ranges("ledger", &words("brin")).is_empty());
        assert!(emphasis_ranges("led", &words("ledger")).is_empty());
    }

    #[test]
    fn as_of_reads_as_a_day_and_a_time() {
        assert_eq!(as_of("2026-11-15T12:52:07Z"), "2026-11-15 12:52");
        assert_eq!(as_of("yesterday"), "yesterday");
    }

    struct View {
        find: FindState,
        send: SendFind,
    }

    impl Render for View {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(conversations(
                &self.find,
                Some(0),
                &|_| false,
                Look::of(cx),
                &self.send,
            ))
        }
    }

    #[gpui_kit::test]
    fn clicking_a_hit_opens_that_hit(cx: &mut TestAppContext) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let find = FindState {
            query: "ledger".to_owned(),
            hits: Arc::new(vec![hit("Ledger schema migration", 3), hit("Try BRIN index", 7)]),
            ..FindState::default()
        };
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                    cx.new(|_| View {
                        find,
                        send: Rc::new(move |action, _| log.borrow_mut().push(action)),
                    })
                })
            })
            .expect("the test window opens");
        cx.update_window(window, |_, window, cx| window.click("palette-hit-1", cx))
            .expect("the test window is open");
        assert_eq!(*sent.borrow(), vec![FindAction::Open(1)]);
    }

    #[gpui_kit::test]
    fn clicking_more_asks_for_the_next_page(cx: &mut TestAppContext) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let find = FindState {
            query: "ledger".to_owned(),
            hits: Arc::new(vec![hit("Ledger schema migration", 3)]),
            next_page: "page-2".to_owned(),
            ..FindState::default()
        };
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                    cx.new(|_| View {
                        find,
                        send: Rc::new(move |action, _| log.borrow_mut().push(action)),
                    })
                })
            })
            .expect("the test window opens");
        cx.update_window(window, |_, window, cx| window.click("palette-hits-more", cx))
            .expect("the test window is open");
        assert_eq!(*sent.borrow(), vec![FindAction::More]);
    }
}

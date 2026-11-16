//! The line under an agent's terminal output that says how many of its board
//! writes the host didn't apply when it reconnected, and why (AW-9, D267).
//! It is Humata's line, drawn beside the terminal like the reboot line:
//! nothing is typed into the agent, and it stays until a later reconnect
//! replaces it.
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_clients::model::{AwayLeftOut, Session};

use crate::{
    chrome,
    theme::Look,
    widgets::row,
};
use gpui_kit::assets::IconName;

/// Where the summary says the changes are kept: in one item's history, or
/// in several; and which new items weren't made.
fn trace(left: &AwayLeftOut) -> Option<String> {
    let mut parts = Vec::new();
    match left.items.as_slice() {
        [] => {}
        [only] => parts.push(format!("kept as lost in {only}'s history")),
        [first, rest @ ..] => parts.push(format!(
            "kept as lost in the history of {first} and {} more",
            rest.len()
        )),
    }
    match left.not_made.as_slice() {
        [] => {}
        [only] => parts.push(format!("new item “{only}” wasn't made")),
        [first, rest @ ..] => parts.push(format!(
            "new items “{first}” and {} more weren't made",
            rest.len()
        )),
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// The sentence, from the summary's fields. A refusal is the stricter, so it
/// words the line when the writes are mixed.
pub(crate) fn text(left: &AwayLeftOut) -> String {
    let one = left.count == 1;
    let changes = if one {
        "1 board change".to_owned()
    } else {
        format!("{} board changes", left.count)
    };
    let (were, it) = if one { ("wasn't", "it") } else { ("weren't", "them") };
    let (why, then) = if left.refused {
        (
            format!("the channel's rules now refuse {it}"),
            format!("allow {it} in Agent access, then ask the agent to make {it} again"),
        )
    } else {
        (
            "the channel's rule now says Ask me".to_owned(),
            format!("ask the agent to make {it} again and you'll be asked"),
        )
    };
    let kept = trace(left).map(|trace| format!(" ({trace})")).unwrap_or_default();
    format!(
        "{changes} this agent made while the host was away {were} applied because {why}{kept}; {then}."
    )
}

/// The line, when the session has something left out.
pub(crate) fn line(session: &Session, look: Look) -> Option<AnyElement> {
    let left = session.away_left_out.as_ref()?;
    let words = text(left);
    Some(
        row()
            .id("away-left-out")
            .role(Role::Status)
            .aria_label(words.clone())
            .mt(px(10.))
            .gap(px(8.))
            .items_start()
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(chrome::icon(IconName::Info, look.secondary, 12.))
            .child(div().flex_1().min_w_0().child(words.clone()).test_text(words))
            .test_support()
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use humata2_clients::model::AwayLeftOut;

    use super::text;

    fn left(count: u32, refused: bool) -> AwayLeftOut {
        AwayLeftOut {
            count,
            items: vec!["PAY-12".to_owned()],
            not_made: Vec::new(),
            refused,
        }
    }

    #[test]
    fn the_line_says_how_many_and_which_item_keeps_them() {
        let said = text(&left(2, true));
        assert!(said.contains('2'), "{said}");
        assert!(said.contains("PAY-12"), "{said}");
    }

    #[test]
    fn a_refusal_and_an_ask_tell_the_person_different_things_to_do() {
        assert_ne!(text(&left(1, true)), text(&left(1, false)));
    }

    #[test]
    fn a_new_item_that_wasnt_made_is_named_by_its_title() {
        let said = text(&AwayLeftOut {
            count: 1,
            items: Vec::new(),
            not_made: vec!["Fix login".to_owned()],
            refused: true,
        });
        assert!(said.contains("Fix login"), "{said}");
    }

    #[test]
    fn several_items_are_named_by_the_first_and_how_many_more() {
        let said = text(&AwayLeftOut {
            count: 3,
            items: vec!["PAY-12".to_owned(), "PAY-13".to_owned(), "PAY-14".to_owned()],
            not_made: Vec::new(),
            refused: false,
        });
        assert!(said.contains("PAY-12") && said.contains('2') && !said.contains("PAY-13"), "{said}");
    }
}

//! The prompt an agent opened from New agent tab starts with (D857): what
//! the Diff panel shows, in one sentence, and a note to leave comments
//! instead of editing.

use humata2_clients::model::Session;

use crate::ReviewState;

/// What every opening prompt ends with.
const TAIL: &str = " Don't edit files. Leave what you find on the lines.";

/// The prompt for an agent opened beside `session`'s Diff, worded for what
/// the panel shows now: a stack layer, the whole stack, a pull request, or
/// the plain changes.
#[must_use]
pub fn opening_prompt(state: &ReviewState, session: &Session) -> String {
    let what = if let Some(stack) = state.stack_of(&session.id) {
        match state
            .layer_of(&session.id)
            .and_then(|title| stack.layers.iter().position(|layer| layer.title == title))
        {
            Some(index) => format!("layer {} of the stack", index + 1),
            None => "the whole stack".to_owned(),
        }
    } else if let Some(number) = session
        .pull_request_review
        .then(|| only_number(&session.title))
        .flatten()
    {
        format!("pull request #{number}")
    } else {
        "the changes".to_owned()
    };
    format!("Look at {what} in the Diff panel.{TAIL}")
}

/// The digits after the one `#` in `title` that has any; `None` when there
/// are none or several.
fn only_number(title: &str) -> Option<&str> {
    let mut found = None;
    for (at, _) in title.match_indices('#') {
        let rest = &title[at + 1..];
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(&rest[..digits]);
    }
    found
}

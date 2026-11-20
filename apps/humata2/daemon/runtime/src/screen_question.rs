//! D762: a question an agent's CLI asks on its own
//! screen before its input box exists, such as Copilot's and Claude Code's
//! folder trust. Text typed then goes into the question, not the agent's
//! input, and the Enter that should submit it answers the question:
//! for folder trust, that grants trust the user never gave. The input gate
//! (`input_gate`) looks for such a question before anything is typed, and
//! `screen_ask` (D767) asks it in the Inbox. Only questions seen on a
//! recorded screen are listed.

/// A question the agent's CLI is asking on its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenQuestion {
    /// Whether to trust the files in the session's folder.
    FolderTrust,
    /// An offer the CLI shows on its own after a turn, such as Claude
    /// Code's "Teach auto mode about your environment?". It takes the keys
    /// like a question: a typed message's Enter would pick its highlighted
    /// choice.
    Offer,
}

impl ScreenQuestion {
    /// Whether the question holds its session at Needs you while it shows.
    /// An offer doesn't (D864): the agent isn't waiting on the person, so
    /// only the input gate refuses text while it is up.
    pub fn needs_you(self) -> bool {
        match self {
            Self::FolderTrust => true,
            Self::Offer => false,
        }
    }
}

/// What each question says on the screen, as `plain` leaves it: Copilot CLI
/// 1.0.113's "Confirm folder trust", as recorded in VERIFY, and Claude Code
/// 2.1.328's auto-mode offer, as recorded in AGENTS2's live check O1.
/// Codex's first-run questions wait for a recorded screen (D762).
const ASKED: &[(&str, ScreenQuestion)] = &[
    (
        "Do you trust the files in this folder?",
        ScreenQuestion::FolderTrust,
    ),
    (
        "Teach auto mode about your environment?",
        ScreenQuestion::Offer,
    ),
];

use std::sync::LazyLock;

static NORMALIZED_ASKED: LazyLock<Vec<(String, ScreenQuestion)>> = LazyLock::new(|| {
    ASKED
        .iter()
        .map(|(words, question)| (plain(words), *question))
        .collect()
});

/// The question `screen` (the text the tab shows now) is asking, if any.
pub fn asked_on(screen: &str) -> Option<ScreenQuestion> {
    let shown = plain(screen);
    NORMALIZED_ASKED
        .iter()
        .find(|(words, _)| shown.contains(words.as_str()))
        .map(|(_, question)| *question)
}

/// `text` without box drawing, with each run of white space, line breaks
/// included, as one space: a question drawn in a box, or wrapped over rows
/// on a narrow screen, reads the same.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c) {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COPILOT_TRUST: &str = "\
 [Current]  Sessions   Issues   Pull requests   Gists
╭──────────────────────────────────────────────────────────╮
│ Confirm folder trust                                     │
│ ╭──────────────────────────────────────────────────────╮ │
│ │ /home/vremote/Humata/remote-flows/sessions/rr-flow2b │ │
│ ╰──────────────────────────────────────────────────────╯ │
│ Do you trust the files in this folder?                   │
│                                                          │
│ ❯ 1. Yes                                                 │
│   2. Yes, and remember this folder for future sessions   │
│   3. No (Esc)                                            │
╰──────────────────────────────────────────────────────────╯";

    const COPILOT_READY: &str = "\
 ❯ Remember this code word: otter.                         18:16
 ● Noted: otter.
 ● MCP Servers reloaded: 2 servers connected
 ~/Humata/remote-flows/sessions/rr-flow3
──────────────────────────────────────────────────────────
❯
──────────────────────────────────────────────────────────
 ← open sidebar · Interactive · Manual Approval · / commands";

    /// Claude Code 2.1.328 after a turn, auto mode on (AGENTS2 O1), and
    /// the same session's input once the offer is gone.
    const CLAUDE_OFFER: &str = "\
⏺ You picked blue, and I wrote blue to colour.txt.

✻ Sautéed for 28s · done 2:12 AM

────────────────────────────────────────────────────────────────
  Teach auto mode about your environment?

  Auto mode works better when it knows your environment. Takes about a minute.

  ❯ 1. Yes
    2. Not now
    3. Don't show again

  Enter to confirm · Esc to cancel
────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle)";

    const CLAUDE_READY: &str = "\
❯ /clear

────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle)";

    #[test]
    fn claudes_auto_mode_offer_takes_the_keys_and_its_input_box_does_not() {
        assert_eq!(asked_on(CLAUDE_OFFER), Some(ScreenQuestion::Offer));
        assert_eq!(asked_on(CLAUDE_READY), None);
        assert!(!ScreenQuestion::Offer.needs_you());
        assert!(ScreenQuestion::FolderTrust.needs_you());
    }

    #[test]
    fn copilots_folder_trust_is_a_question_and_its_input_box_is_not() {
        assert_eq!(asked_on(COPILOT_TRUST), Some(ScreenQuestion::FolderTrust));
        assert_eq!(asked_on(COPILOT_READY), None);
    }

    #[test]
    fn a_question_wrapped_over_rows_in_a_narrow_box_is_still_seen() {
        let narrow = "│ Do you trust the files │\n│ in this    folder?     │";
        assert_eq!(asked_on(narrow), Some(ScreenQuestion::FolderTrust));
    }

    #[test]
    fn the_words_split_by_other_text_are_not_the_question() {
        assert_eq!(
            asked_on("printf 'Do you trust the files' 'in this folder?'"),
            None
        );
    }
}

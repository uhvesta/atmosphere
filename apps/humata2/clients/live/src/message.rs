//! How a queued message reaches a terminal.

const PASTE_START: &str = "\u{1b}[200~";
const PASTE_END: &str = "\u{1b}[201~";

/// The keystrokes that deliver one whole message and submit it. A message of
/// several lines goes as one bracketed paste (SE-23), so its newlines are
/// text and not early Enters; a one-line message is typed as it is.
pub(crate) fn typed(text: &str) -> String {
    if !text.contains('\n') {
        return format!("{text}\r");
    }
    // A paste end inside the text would close the paste early and type the
    // rest as keystrokes.
    let safe = text.replace(PASTE_END, "");
    format!("{PASTE_START}{safe}{PASTE_END}\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_lines_go_as_one_paste_then_enter() {
        assert_eq!(
            typed("first\nsecond"),
            "\u{1b}[200~first\nsecond\u{1b}[201~\r"
        );
    }

    #[test]
    fn one_line_is_typed_and_entered() {
        assert_eq!(typed("just this"), "just this\r");
    }

    #[test]
    fn text_cannot_end_the_paste_early() {
        let sent = typed("a\n\u{1b}[201~rm -rf x\nb");
        assert_eq!(sent.matches("\u{1b}[201~").count(), 1);
        assert!(sent.ends_with("\u{1b}[201~\r"));
    }
}

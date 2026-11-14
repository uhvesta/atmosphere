//! How a message one session's agent sends another names its sender: a
//! first line the agent reading it understands as it is, and that the app
//! and read_session take off to show who it's from. The line is all there
//! is: a host stores the message as any other, so one needs no update to
//! carry it.
//!
//! The app reads the same line in `AgentSender` (clients/daemon); change
//! both together.

const START: &str = "Message from the agent of session ";
const END: &str = "\", not from the person:\n\n";

/// `text` as session `id`, named `name`, sends it.
pub(crate) fn headed(id: &str, name: &str, text: &str) -> String {
    // The name ends at the first quote, so it holds none, and stays on
    // the line.
    let name: String = name
        .chars()
        .map(|c| if c == '"' || c.is_control() { ' ' } else { c })
        .collect();
    format!("{START}{id} \"{}{END}{text}", name.trim())
}

/// A message's sender and what it says, when another session's agent sent
/// it; None for the person's own.
pub(crate) fn sender(text: &str) -> Option<Sent<'_>> {
    let rest = text.strip_prefix(START)?;
    let (id, rest) = rest.split_once(" \"")?;
    let (name, text) = rest.split_once(END)?;
    if id.is_empty() || id.contains(char::is_whitespace) || name.contains(['"', '\n']) {
        return None;
    }
    Some(Sent { id, name, text })
}

/// A message another session's agent sent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Sent<'a> {
    pub(crate) id: &'a str,
    pub(crate) name: &'a str,
    pub(crate) text: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_headed_message_names_its_sender_and_keeps_its_text() {
        let text = "Run the tests.\n\nThen say what failed.";
        let headed = headed("s-1", "Build \"the\" app\n", text);
        let sent = sender(&headed).unwrap_or_else(|| panic!("no sender in {headed:?}"));
        assert_eq!(sent.id, "s-1");
        assert_eq!(sent.text, text);
        // The name survives but for what would end it early.
        assert!(sent.name.starts_with("Build") && sent.name.ends_with("app"));
    }

    #[test]
    fn the_persons_own_message_has_no_sender() {
        assert_eq!(sender("Run the tests."), None);
        assert_eq!(sender(""), None);
        // Quoting the line later in a message doesn't make it one.
        let quoted = format!("See: {}", headed("s-1", "Build", "hi"));
        assert_eq!(sender(&quoted), None);
    }
}

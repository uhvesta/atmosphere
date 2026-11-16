//! Scrubbing: what is taken out of a report before any of it is written.
//!
//! - **Seeded secrets** (tokens, host variable values) are replaced by
//!   `‹NAME›` wherever they appear: in typed and pasted text, labels, state,
//!   view and layout trees and their changes, daemon calls and logs, the
//!   context and the form. A secret typed over several events is found in
//!   the run of text those events typed together. A leading part of a
//!   secret, four characters or more and standing on its own (such as a
//!   field's value while the secret was being typed), is replaced too.
//! - **Masked fields** (role `PasswordInput`): what is typed or pasted into
//!   one is replaced, and the same text is scrubbed everywhere else, as a
//!   seeded secret is.
//! - **Paste-folder files**: the contents of each file seeded with
//!   [`Secrets::paste_file`] are replaced wherever they appear as text, and
//!   calls that carry such contents ([`Holds::PasteContents`]) lose their
//!   request and response. Paths and sizes are kept.
//! - **Terminal text** is kept only for terminals marked with
//!   [`Secrets::keep_terminal`]: a terminal's subtree in the view and layout
//!   trees, what is typed or pasted into it, labels of elements inside it,
//!   and calls that carry its bytes ([`Holds::Terminal`]).
//!
//! Tree changes are scrubbed by applying them, scrubbing each whole tree,
//! and diffing the scrubbed trees again, so a change can't carry what its
//! tree had removed.

use crate::change::{self, Hunk};
use crate::error::ScrubError;
use crate::event::{Element, Event, Gesture, Holds, Pasted, Trees};
use crate::form::Form;
use crate::recording::Captured;

/// A leading part of a secret this many characters long, or longer, is
/// replaced when nothing word-like touches it on either side.
const PREFIX_MIN: usize = 4;

/// The role of a masked text field, as the view snapshot writes it.
const MASKED_ROLE: &str = "PasswordInput";

/// The role of a terminal, as the view snapshot writes it.
const TERMINAL_ROLE: &str = "Terminal";

/// How the view snapshot writes an element with no role or label that has
/// the terminal view's id.
const TERMINAL_ID: &str = "#terminal";

const MASKED: &str = "‹masked input›";
const TERMINAL_INPUT: &str = "‹terminal input›";
const TERMINAL_TEXT: &str = "‹terminal text›";
const TERMINAL_OUTPUT: &str = "‹terminal output left out›";
const PASTE_CONTENTS: &str = "‹paste-folder contents left out›";
const TERMINAL_LEFT_OUT: &str = "(terminal text left out: mark this terminal to keep it)";

/// What to scrub from a report, seeded by the app with what it knows.
#[derive(Clone, Debug, Default)]
pub struct Secrets {
    needles: Vec<Needle>,
    kept_terminals: Vec<String>,
}

#[derive(Clone, Debug)]
struct Needle {
    value: String,
    marker: String,
    /// Whether a leading part of it standing on its own is replaced too.
    prefixes: bool,
}

impl Secrets {
    /// Nothing seeded yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// A secret value, such as a token or a host variable's value, shown as
    /// `‹name›` wherever it appears. An empty value is ignored.
    #[must_use]
    pub fn secret(mut self, name: &str, value: &str) -> Self {
        if !value.is_empty() {
            self.needles.push(Needle {
                value: value.to_owned(),
                marker: format!("‹{name}›"),
                prefixes: true,
            });
        }
        self
    }

    /// A file in a session's paste folder, by its path and contents. When
    /// the contents are text, they are shown as `‹contents of NAME›`
    /// wherever they appear. The path itself is kept.
    #[must_use]
    pub fn paste_file(mut self, path: &str, contents: &[u8]) -> Self {
        if let Ok(text) = std::str::from_utf8(contents)
            && !text.trim().is_empty()
        {
            let name = path.rsplit('/').next().unwrap_or(path);
            self.needles.push(Needle {
                value: text.to_owned(),
                marker: format!("‹contents of {name}›"),
                prefixes: false,
            });
        }
        self
    }

    /// Keeps the text of the terminal with accessible `label`.
    #[must_use]
    pub fn keep_terminal(mut self, label: &str) -> Self {
        self.kept_terminals.push(label.to_owned());
        self
    }
}

/// A report with everything [`Secrets`] names taken out, ready to write.
/// Only [`scrub`] makes one, so nothing unscrubbed can be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrubbed {
    captured: Captured,
    form: Form,
}

impl Scrubbed {
    /// The scrubbed report.
    pub fn captured(&self) -> &Captured {
        &self.captured
    }

    /// The scrubbed form.
    pub fn form(&self) -> &Form {
        &self.form
    }
}

/// Takes out of `captured` and `form` everything `secrets` names. See the
/// [module](self) for what that is.
///
/// # Errors
///
/// [`ScrubError::Mismatch`] when a recorded tree change doesn't fit the
/// tree it was recorded against, so the trees can't be rebuilt to scrub.
pub fn scrub(mut captured: Captured, mut form: Form, secrets: &Secrets) -> Result<Scrubbed, ScrubError> {
    let runs = runs(&captured.events);
    let mut needles = secrets.needles.clone();
    for run in &runs {
        if run.target.as_ref().is_some_and(masked) {
            let text = run_text(&captured.events, run);
            if !text.is_empty() {
                needles.push(Needle {
                    value: text,
                    marker: MASKED.to_owned(),
                    prefixes: true,
                });
            }
        }
    }
    let rules = Rules {
        matcher: Matcher { needles },
        kept: &secrets.kept_terminals,
    };

    let mut emptied = Vec::new();
    for run in &runs {
        emptied.extend(rules.scrub_run(&mut captured.events, run));
    }
    let mut index = 0;
    captured.events.retain(|_| {
        index += 1;
        !emptied.contains(&(index - 1))
    });

    for event in &mut captured.events {
        rules.scrub_event(event);
    }
    rules.scrub_trees(&mut captured)?;
    rules.scrub_context(&mut captured);
    rules.scrub_form(&mut form);
    Ok(Scrubbed { captured, form })
}

/// `form` with everything `secrets` seeds taken out, for a form filled in
/// after its report was written. Text typed into masked fields during the
/// recording is no longer known by then, so only seeded values are found.
pub(crate) fn scrub_form_alone(mut form: Form, secrets: &Secrets) -> Form {
    let rules = Rules {
        matcher: Matcher {
            needles: secrets.needles.clone(),
        },
        kept: &secrets.kept_terminals,
    };
    rules.scrub_form(&mut form);
    form
}

struct Rules<'a> {
    matcher: Matcher,
    kept: &'a [String],
}

impl Rules<'_> {
    /// Scrubs the text a run of typing events typed together, and returns
    /// the events left with no text.
    fn scrub_run(&self, events: &mut [Event], run: &Run) -> Vec<usize> {
        let texts: Vec<String> = run
            .events
            .iter()
            .map(|&index| typed(&events[index]).map(|(text, _)| text.to_owned()).unwrap_or_default())
            .collect();
        let joined: String = texts.concat();
        let whole = |marker: &str| {
            if joined.is_empty() {
                Vec::new()
            } else {
                vec![Span {
                    start: 0,
                    end: joined.len(),
                    marker: marker.to_owned(),
                }]
            }
        };
        let spans = match &run.target {
            Some(target) if masked(target) => whole(MASKED),
            Some(target) if self.hides_terminal(target) => whole(TERMINAL_INPUT),
            _ => self.matcher.spans(&joined),
        };
        if spans.is_empty() {
            return Vec::new();
        }

        let mut emptied = Vec::new();
        let mut offset = 0;
        for (&index, text) in run.events.iter().zip(&texts) {
            let (start, end) = (offset, offset + text.len());
            offset = end;
            let mut out = String::new();
            let mut at = start;
            for span in spans.iter().filter(|span| span.start < end && span.end > start) {
                if span.start > at {
                    out.push_str(&joined[at..span.start]);
                }
                if span.start >= start {
                    out.push_str(&span.marker);
                }
                at = at.max(span.end.min(end));
            }
            if at < end {
                out.push_str(&joined[at..end]);
            }
            if out.is_empty() {
                emptied.push(index);
            }
            if let Some(text) = typed_mut(&mut events[index]) {
                *text = out;
            }
        }
        emptied
    }

    /// Scrubs everything in `event` but the typed text runs and the tree
    /// changes, which have their own passes.
    fn scrub_event(&self, event: &mut Event) {
        match event {
            Event::Interaction(interaction) => match &mut interaction.gesture {
                Gesture::Click { target, .. } | Gesture::Scroll { target, .. } => {
                    self.scrub_element(target);
                }
                Gesture::Key { target, .. } | Gesture::Type { target, .. } => {
                    if let Some(target) = target {
                        self.scrub_element(target);
                    }
                }
                Gesture::Drag { from, to } => {
                    self.scrub_element(from);
                    self.scrub_element(to);
                }
                Gesture::Paste { target, content } => {
                    if let Some(target) = target {
                        self.scrub_element(target);
                    }
                    if let Pasted::Image { path, .. } | Pasted::File { path, .. } = content {
                        *path = self.matcher.scrub(path);
                    }
                }
            },
            Event::Action(step) => step.action = self.matcher.scrub(&step.action),
            Event::View(_) | Event::Layout(_) => {}
            Event::Rpc(rpc) => {
                let replace = match &rpc.holds {
                    Holds::Plain => None,
                    Holds::Terminal { terminal } if self.kept.contains(terminal) => None,
                    Holds::Terminal { .. } => Some(TERMINAL_OUTPUT),
                    Holds::PasteContents => Some(PASTE_CONTENTS),
                };
                for payload in [&mut rpc.request, &mut rpc.response].into_iter().flatten() {
                    *payload = match replace {
                        Some(marker) => marker.to_owned(),
                        None => self.matcher.scrub(payload),
                    };
                }
                if let Holds::Terminal { terminal } = &mut rpc.holds {
                    *terminal = self.matcher.scrub(terminal);
                }
                rpc.method = self.matcher.scrub(&rpc.method);
            }
            Event::DaemonError(error) => error.message = self.matcher.scrub(&error.message),
            Event::Log(line) => line.line = self.matcher.scrub(&line.line),
        }
    }

    fn scrub_element(&self, element: &mut Element) {
        let inside_hidden = element.within.iter().any(|node| {
            node.role == TERMINAL_ROLE && !node.label.as_ref().is_some_and(|label| self.kept.contains(label))
        });
        if inside_hidden && let Some(label) = &mut element.label {
            *label = TERMINAL_TEXT.to_owned();
        }
        if let Some(label) = &mut element.label {
            *label = self.matcher.scrub(label);
        }
        for node in &mut element.within {
            if let Some(label) = &mut node.label {
                *label = self.matcher.scrub(label);
            }
        }
    }

    /// Whether `target` is, or is in, a terminal whose text isn't kept.
    fn hides_terminal(&self, target: &Element) -> bool {
        let kept = |label: &Option<String>| label.as_ref().is_some_and(|label| self.kept.contains(label));
        (target.role == TERMINAL_ROLE && !kept(&target.label))
            || target
                .within
                .iter()
                .any(|node| node.role == TERMINAL_ROLE && !kept(&node.label))
    }

    /// Scrubs the start and end trees and rebuilds each change from the
    /// scrubbed trees on either side of it.
    fn scrub_trees(&self, captured: &mut Captured) -> Result<(), ScrubError> {
        for stream in [Stream::State, Stream::View, Stream::Layout] {
            let tree = |text: &str| match stream {
                Stream::State => self.matcher.scrub(text),
                Stream::View | Stream::Layout => self.matcher.scrub(&drop_terminals(text, self.kept)),
            };
            let mut current = change::lines(stream.pick(&captured.start));
            let mut shown = tree(stream.pick(&captured.start));
            stream.pick_mut(&mut captured.start).clone_from(&shown);
            for event in &mut captured.events {
                let Some((at_ms, hunks)) = stream.change_mut(event) else {
                    continue;
                };
                change::apply(&mut current, hunks).map_err(|mismatch| ScrubError::Mismatch {
                    tree: stream.name(),
                    at_ms,
                    line: mismatch.line,
                })?;
                let next = tree(&change::join(&current));
                *hunks = change::diff(&shown, &next);
                shown = next;
            }
            let end = tree(stream.pick(&captured.end));
            *stream.pick_mut(&mut captured.end) = end;
        }
        captured.events.retain(|event| match event {
            Event::View(change) | Event::Layout(change) => !change.change.is_empty(),
            _ => true,
        });
        Ok(())
    }

    fn scrub_context(&self, captured: &mut Captured) {
        let context = &mut captured.context;
        for session in &mut context.sessions {
            session.title = self.matcher.scrub(&session.title);
        }
    }

    fn scrub_form(&self, form: &mut Form) {
        form.happened = self.matcher.scrub(&form.happened);
        form.expected = self.matcher.scrub(&form.expected);
        for comment in &mut form.comments {
            comment.text = self.matcher.scrub(&comment.text);
            self.scrub_element(&mut comment.element);
        }
        for stroke in &mut form.drawing.strokes {
            for element in &mut stroke.covers {
                self.scrub_element(element);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Stream {
    State,
    View,
    Layout,
}

impl Stream {
    fn name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::View => "view",
            Self::Layout => "layout",
        }
    }

    fn pick(self, trees: &Trees) -> &str {
        match self {
            Self::State => &trees.state,
            Self::View => &trees.view,
            Self::Layout => &trees.layout,
        }
    }

    fn pick_mut(self, trees: &mut Trees) -> &mut String {
        match self {
            Self::State => &mut trees.state,
            Self::View => &mut trees.view,
            Self::Layout => &mut trees.layout,
        }
    }

    fn change_mut(self, event: &mut Event) -> Option<(u64, &mut Vec<Hunk>)> {
        match (self, event) {
            (Self::State, Event::Action(step)) => Some((step.at_ms, &mut step.change)),
            (Self::View, Event::View(change)) | (Self::Layout, Event::Layout(change)) => {
                Some((change.at_ms, &mut change.change))
            }
            _ => None,
        }
    }
}

/// Consecutive typing events into one target: text typed or pasted, with
/// nothing but non-interaction events (actions, trees, calls) between
/// them. A keystroke, click, scroll or drag ends a run.
struct Run {
    events: Vec<usize>,
    target: Option<Element>,
}

fn runs(events: &[Event]) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut open: Option<Run> = None;
    for (index, event) in events.iter().enumerate() {
        if !matches!(event, Event::Interaction(_)) {
            continue;
        }
        let Some((_, target)) = typed(event) else {
            runs.extend(open.take());
            continue;
        };
        let continues = open.as_ref().is_some_and(|run| run.target.as_ref() == target);
        if continues {
            if let Some(run) = open.as_mut() {
                run.events.push(index);
            }
        } else {
            runs.extend(open.take());
            open = Some(Run {
                events: vec![index],
                target: target.cloned(),
            });
        }
    }
    runs.extend(open);
    runs
}

fn run_text(events: &[Event], run: &Run) -> String {
    run.events
        .iter()
        .filter_map(|&index| typed(&events[index]).map(|(text, _)| text))
        .collect()
}

/// The text an event typed or pasted, and where.
fn typed(event: &Event) -> Option<(&str, Option<&Element>)> {
    let Event::Interaction(interaction) = event else {
        return None;
    };
    match &interaction.gesture {
        Gesture::Type { text, target }
        | Gesture::Paste {
            target,
            content: Pasted::Text(text),
        } => Some((text.as_str(), target.as_ref())),
        _ => None,
    }
}

fn typed_mut(event: &mut Event) -> Option<&mut String> {
    let Event::Interaction(interaction) = event else {
        return None;
    };
    match &mut interaction.gesture {
        Gesture::Type { text, .. }
        | Gesture::Paste {
            content: Pasted::Text(text),
            ..
        } => Some(text),
        _ => None,
    }
}

fn masked(element: &Element) -> bool {
    element.role == MASKED_ROLE
}

/// `tree` (a view or layout snapshot) with the subtree of each terminal
/// whose text isn't kept left out. The terminal's own line stays.
fn drop_terminals(tree: &str, kept: &[String]) -> String {
    let mut out = String::with_capacity(tree.len());
    let mut hiding: Option<usize> = None;
    let mut noted = false;
    for line in tree.lines() {
        let content = line.trim_start_matches(' ');
        let indent = line.len() - content.len();
        if let Some(depth) = hiding {
            if indent > depth && !content.is_empty() {
                if !noted {
                    out.push_str(&" ".repeat(depth + 2));
                    out.push_str(TERMINAL_LEFT_OUT);
                    out.push('\n');
                    noted = true;
                }
                continue;
            }
            hiding = None;
        }
        if is_terminal_line(content) && !kept.iter().any(|label| names_terminal(content, label)) {
            hiding = Some(indent);
            noted = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn is_terminal_line(content: &str) -> bool {
    [TERMINAL_ROLE, TERMINAL_ID]
        .iter()
        .any(|head| content == *head || content.starts_with(&format!("{head} ")))
}

fn names_terminal(content: &str, label: &str) -> bool {
    let head = format!("{TERMINAL_ROLE} {label:?}");
    content == head || content.starts_with(&format!("{head} "))
}

/// Finds seeded values in text.
struct Matcher {
    needles: Vec<Needle>,
}

/// A part of a text to replace by `marker`, in bytes.
#[derive(Debug)]
struct Span {
    start: usize,
    end: usize,
    marker: String,
}

impl Matcher {
    /// `text` with every seeded value replaced.
    fn scrub(&self, text: &str) -> String {
        let spans = self.spans(text);
        if spans.is_empty() {
            return text.to_owned();
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for span in spans {
            out.push_str(&text[at..span.start]);
            out.push_str(&span.marker);
            at = span.end;
        }
        out.push_str(&text[at..]);
        out
    }

    /// Where seeded values appear in `text`, in order, with overlapping
    /// finds merged.
    fn spans(&self, text: &str) -> Vec<Span> {
        let mut found: Vec<(usize, usize, usize)> = Vec::new();
        for (which, needle) in self.needles.iter().enumerate() {
            let value = needle.value.as_str();
            found.extend(
                text.match_indices(value)
                    .map(|(start, _)| (start, start + value.len(), which)),
            );
            if !needle.prefixes {
                continue;
            }
            let Some((head_end, _)) = value.char_indices().nth(PREFIX_MIN) else {
                continue;
            };
            for (start, _) in text.match_indices(&value[..head_end]) {
                let mut common = text[start..]
                    .bytes()
                    .zip(value.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                while !value.is_char_boundary(common) {
                    common -= 1;
                }
                if common < value.len()
                    && stands_alone_before(text, start)
                    && stands_alone_after(text, start + common)
                {
                    found.push((start, start + common, which));
                }
            }
        }
        found.sort_unstable();
        let mut spans: Vec<Span> = Vec::new();
        for (start, end, which) in found {
            match spans.last_mut() {
                Some(last) if start < last.end => last.end = last.end.max(end),
                _ => spans.push(Span {
                    start,
                    end,
                    marker: self.needles[which].marker.clone(),
                }),
            }
        }
        spans
    }
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

fn stands_alone_before(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|c| !word(c))
}

fn stands_alone_after(text: &str, at: usize) -> bool {
    text[at..].chars().next().is_none_or(|c| !word(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(value: &str) -> Matcher {
        Matcher {
            needles: Secrets::new().secret("TOKEN", value).needles,
        }
    }

    #[test]
    fn a_secret_is_replaced_wherever_it_appears() {
        let scrubbed = matcher("ghp_abcd1234").scrub("a ghp_abcd1234 and xghp_abcd1234y");
        assert!(!scrubbed.contains("ghp_abcd1234"));
    }

    #[test]
    fn a_leading_part_on_its_own_is_replaced_but_not_inside_a_word() {
        let m = matcher("production-db-password");
        assert!(!m.scrub("draft: \"production-db\"").contains("production-db"));
        assert!(m.scrub("productions are fine").contains("productions"));
        assert_eq!(m.scrub("pro"), "pro", "shorter than the minimum is kept");
    }

    #[test]
    fn a_kept_terminal_keeps_its_text_and_others_lose_theirs() {
        let tree = "Group \"Panes\"\n  Terminal \"web\"\n    text \"secret output\"\n  Terminal \"api\"\n    text \"kept output\"\n  Button \"Add\"\n";
        let out = drop_terminals(tree, &["api".to_owned()]);
        assert!(!out.contains("secret output"));
        assert!(out.contains("kept output"));
        assert!(out.contains("Button \"Add\""), "what follows a hidden terminal stays");
        assert!(out.contains("Terminal \"web\""), "the terminal itself is still named");
    }
}

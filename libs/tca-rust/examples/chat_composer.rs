//! A chat thread and the composer under it. The composer isn't handed the
//! thread: the thread builds a small `Composer` from several of its own
//! fields, runs the composer's reducer on that, and writes back only the
//! draft. The composer reads who is in the thread and whether it's muted,
//! but it can't change either, and it never sees the messages. When the
//! person sends, the composer tells the thread through a delegate action,
//! and the thread does the posting.
//!
//! Patterns: a projection of parent state for a child, written back in
//! part; a child telling its parent something through a delegate action.

use std::sync::Arc;
use std::time::Duration;

use tca::{Clock, Effect, Reducer, TestClock, TestStore};

/// The most characters a message may have.
const LIMIT: usize = 40;

/// What the composer is given: its view of the thread, rebuilt for each
/// action. Only `draft` goes back to the thread. The rest is a copy, so a
/// child that changed it would change nothing the thread keeps.
#[derive(Clone, Debug, PartialEq)]
struct Composer {
    draft: String,
    /// The people the composer may mention: everyone in the thread but
    /// the person writing.
    mentionable: Vec<String>,
    /// False while the thread is muted.
    can_send: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum ComposerAction {
    Typed(String),
    Mention(String),
    SendTapped,
    /// What the composer tells whoever holds it. It does nothing with
    /// these itself; the thread handles them.
    Delegate(ComposerDelegate),
}

#[derive(Clone, Debug, PartialEq)]
enum ComposerDelegate {
    Send(String),
}

struct Composing;

impl Reducer for Composing {
    type State = Composer;
    type Action = ComposerAction;

    fn reduce(&self, composer: &mut Composer, action: ComposerAction) -> Effect<ComposerAction> {
        match action {
            ComposerAction::Typed(text) => {
                composer.draft = text.chars().take(LIMIT).collect();
                Effect::none()
            }
            ComposerAction::Mention(name) => {
                if composer.mentionable.contains(&name) {
                    composer.draft.push_str(&format!("@{name} "));
                }
                Effect::none()
            }
            ComposerAction::SendTapped => {
                let text = composer.draft.trim().to_owned();
                if !composer.can_send || text.is_empty() {
                    return Effect::none();
                }
                composer.draft.clear();
                // The composer can't post: it doesn't hold the messages.
                // It says what it wants and the thread decides.
                Effect::send(ComposerAction::Delegate(ComposerDelegate::Send(text)))
            }
            ComposerAction::Delegate(_) => Effect::none(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Message {
    id: u64,
    author: String,
    text: String,
    /// Shown greyed out until the server has it.
    pending: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Thread {
    me: String,
    participants: Vec<String>,
    muted: bool,
    messages: Vec<Message>,
    draft: String,
    next_id: u64,
}

impl Thread {
    /// The composer's view of the thread, built from four of its fields.
    fn composer(&self) -> Composer {
        Composer {
            draft: self.draft.clone(),
            mentionable: self
                .participants
                .iter()
                .filter(|name| **name != self.me)
                .cloned()
                .collect(),
            can_send: !self.muted,
        }
    }
}

#[derive(Debug, PartialEq)]
enum ThreadAction {
    Composer(ComposerAction),
    Delivered(u64),
}

struct Threading {
    composing: Composing,
    clock: Arc<dyn Clock>,
}

impl Threading {
    /// Runs the composer on its view of the thread, then takes back the
    /// one field the composer owns.
    fn compose(&self, thread: &mut Thread, action: ComposerAction) -> Effect<ThreadAction> {
        let mut composer = thread.composer();
        let effect = self.composing.reduce(&mut composer, action);
        thread.draft = composer.draft;
        effect.map(ThreadAction::Composer)
    }

    /// Shows the message at once, marked pending, and sends it.
    fn post(&self, thread: &mut Thread, text: String) -> Effect<ThreadAction> {
        let id = thread.next_id;
        thread.next_id += 1;
        thread.messages.push(Message {
            id,
            author: thread.me.clone(),
            text,
            pending: true,
        });
        let clock = self.clock.clone();
        Effect::future(async move {
            clock.sleep(Duration::from_millis(200)).await;
            ThreadAction::Delivered(id)
        })
    }
}

impl Reducer for Threading {
    type State = Thread;
    type Action = ThreadAction;

    fn reduce(&self, thread: &mut Thread, action: ThreadAction) -> Effect<ThreadAction> {
        match action {
            // The delegate gets an arm of its own, ahead of the catch-all
            // for the composer. Its reducer ignores delegates, so there's
            // nothing of the child's to run first.
            ThreadAction::Composer(ComposerAction::Delegate(ComposerDelegate::Send(text))) => {
                self.post(thread, text)
            }
            ThreadAction::Composer(action) => self.compose(thread, action),
            ThreadAction::Delivered(id) => {
                if let Some(message) = thread.messages.iter_mut().find(|m| m.id == id) {
                    message.pending = false;
                }
                Effect::none()
            }
        }
    }
}

fn thread(muted: bool) -> (TestStore<Threading>, TestClock) {
    let clock = TestClock::new();
    let state = Thread {
        me: "ana".into(),
        participants: vec!["ana".into(), "bo".into(), "cy".into()],
        muted,
        ..Thread::default()
    };
    let reducer = Threading {
        composing: Composing,
        clock: Arc::new(clock.clone()),
    };
    (TestStore::new(state, reducer), clock)
}

fn composer(action: ComposerAction) -> ThreadAction {
    ThreadAction::Composer(action)
}

#[test]
fn what_the_composer_types_is_kept_as_the_threads_draft() {
    let (mut store, _clock) = thread(false);
    store.send(composer(ComposerAction::Typed("lunch? ".into())), |s| {
        s.draft = "lunch? ".into();
    });
    store.send(composer(ComposerAction::Mention("bo".into())), |s| {
        s.draft = "lunch? @bo ".into();
    });
}

#[test]
fn the_composer_can_mention_only_others_in_the_thread() {
    let (mut store, _clock) = thread(false);
    // Not in the thread, and the person writing: neither is mentionable.
    store.send(composer(ComposerAction::Mention("dee".into())), |_| {});
    store.send(composer(ComposerAction::Mention("ana".into())), |_| {});
    store.send(composer(ComposerAction::Mention("cy".into())), |s| {
        s.draft = "@cy ".into();
    });
    // Whatever the composer did with its copy, the thread's people are
    // the thread's.
    assert_eq!(store.state().participants, ["ana", "bo", "cy"]);
}

#[test]
fn a_draft_over_the_limit_is_cut_to_it() {
    let (mut store, _clock) = thread(false);
    let long = "a".repeat(LIMIT + 10);
    store.send(composer(ComposerAction::Typed(long)), |s| {
        s.draft = "a".repeat(LIMIT);
    });
}

#[test]
fn sending_clears_the_draft_and_the_thread_posts_the_message() {
    let (mut store, clock) = thread(false);
    store.send(composer(ComposerAction::Typed("  on my way ".into())), |s| {
        s.draft = "  on my way ".into();
    });
    store.send(composer(ComposerAction::SendTapped), |s| s.draft.clear());
    store.receive(
        composer(ComposerAction::Delegate(ComposerDelegate::Send(
            "on my way".into(),
        ))),
        |s| {
            s.messages.push(Message {
                id: 0,
                author: "ana".into(),
                text: "on my way".into(),
                pending: true,
            });
            s.next_id = 1;
        },
    );
    clock.advance(Duration::from_millis(200));
    store.receive(ThreadAction::Delivered(0), |s| s.messages[0].pending = false);
}

#[test]
fn a_muted_thread_keeps_the_draft_and_posts_nothing() {
    let (mut store, _clock) = thread(true);
    store.send(composer(ComposerAction::Typed("hello".into())), |s| {
        s.draft = "hello".into();
    });
    store.send(composer(ComposerAction::SendTapped), |_| {});
    store.finish();
}

#[test]
fn an_empty_draft_sends_nothing() {
    let (mut store, _clock) = thread(false);
    store.send(composer(ComposerAction::Typed("   ".into())), |s| {
        s.draft = "   ".into();
    });
    store.send(composer(ComposerAction::SendTapped), |_| {});
    store.finish();
}

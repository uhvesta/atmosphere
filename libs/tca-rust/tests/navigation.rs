use std::time::Duration;

use tca::{
    AlertState, ButtonState, Clock, Effect, PresentationAction, PresentationState, Reducer,
    StackAction, StackElementId, StackState, TestClock, TestStore, alert, for_each_in_stack,
    if_presented,
};

/// A draft being written: it ticks while open, takes a second to save, and
/// closes itself once saved or cancelled.
#[derive(Clone, Debug, Default, PartialEq)]
struct Editor {
    text: String,
    saving: bool,
    ticks: u32,
}

#[derive(Debug, PartialEq)]
enum EditorAction {
    Type(String),
    StartTicking,
    Tick,
    Save,
    Saved,
    Cancel,
}

struct Editing {
    clock: TestClock,
}

impl Reducer for Editing {
    type State = Editor;
    type Action = EditorAction;

    fn reduce(&self, editor: &mut Editor, action: EditorAction) -> Effect<EditorAction> {
        match action {
            EditorAction::Type(text) => {
                editor.text = text;
                Effect::none()
            }
            EditorAction::StartTicking => {
                let clock = self.clock.clone();
                Effect::run(move |send| async move {
                    loop {
                        clock.sleep(Duration::from_secs(1)).await;
                        send.send(EditorAction::Tick);
                    }
                })
                .cancellable("ticks", true)
            }
            EditorAction::Tick => {
                editor.ticks += 1;
                Effect::none()
            }
            EditorAction::Save => {
                editor.saving = true;
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(1)).await;
                    EditorAction::Saved
                })
            }
            EditorAction::Saved => {
                editor.saving = false;
                Effect::dismiss()
            }
            EditorAction::Cancel => Effect::dismiss(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum AlertChoice {
    Delete,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Inbox {
    sheet: PresentationState<Editor>,
    path: StackState<Editor>,
    alert: PresentationState<AlertState<AlertChoice>>,
    saved: Vec<String>,
    deleted: bool,
}

#[derive(Debug, PartialEq)]
enum InboxAction {
    NewDraft,
    Sheet(PresentationAction<EditorAction>),
    Path(StackAction<Editor, EditorAction>),
    AskDelete,
    Alert(PresentationAction<AlertChoice>),
}

struct Inboxing {
    editor: Editing,
}

fn delete_alert() -> AlertState<AlertChoice> {
    AlertState::new("Delete the draft?")
        .button(ButtonState::destructive("Delete", AlertChoice::Delete))
        .button(ButtonState::cancel("Keep"))
}

impl Reducer for Inboxing {
    type State = Inbox;
    type Action = InboxAction;

    fn reduce(&self, inbox: &mut Inbox, action: InboxAction) -> Effect<InboxAction> {
        match action {
            InboxAction::NewDraft => {
                inbox.sheet.present(Editor::default());
                Effect::none()
            }
            InboxAction::Sheet(action) => {
                // The parent takes what was saved before the sheet goes.
                if let PresentationAction::Presented(EditorAction::Saved) = &action {
                    if let Some(editor) = inbox.sheet.get() {
                        inbox.saved.push(editor.text.clone());
                    }
                }
                if_presented(&self.editor, &mut inbox.sheet, action).map(InboxAction::Sheet)
            }
            InboxAction::Path(action) => {
                for_each_in_stack(&self.editor, &mut inbox.path, action).map(InboxAction::Path)
            }
            InboxAction::AskDelete => {
                inbox.alert.present(delete_alert());
                Effect::none()
            }
            InboxAction::Alert(action) => {
                if let Some(AlertChoice::Delete) = alert(&mut inbox.alert, action) {
                    inbox.deleted = true;
                }
                Effect::none()
            }
        }
    }
}

fn inbox() -> (TestStore<Inboxing>, TestClock) {
    let clock = TestClock::new();
    let store = TestStore::new(
        Inbox::default(),
        Inboxing {
            editor: Editing {
                clock: clock.clone(),
            },
        },
    );
    (store, clock)
}

fn editing(text: &str) -> Editor {
    Editor {
        text: text.into(),
        ..Editor::default()
    }
}

#[test]
fn a_presented_sheet_takes_its_actions_and_dismissing_it_stops_its_effects() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::NewDraft, |s| s.sheet.present(Editor::default()));
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::Type("hi".into()))),
        |s| s.sheet = PresentationState::presenting(editing("hi")),
    );
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::StartTicking)),
        |_| {},
    );
    clock.advance(Duration::from_secs(1));
    store.receive(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::Tick)),
        |s| {
            if let Some(editor) = s.sheet.get_mut() {
                editor.ticks = 1;
            }
        },
    );
    store.send(InboxAction::Sheet(PresentationAction::Dismiss), |s| {
        s.sheet = PresentationState::none()
    });
    // Ticks no more: dropping the store checks nothing came or still runs.
    clock.advance(Duration::from_secs(5));
    store.finish();
}

#[test]
fn a_sheet_dismisses_itself_once_its_work_is_done() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::NewDraft, |s| s.sheet.present(Editor::default()));
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::Type("hi".into()))),
        |s| s.sheet = PresentationState::presenting(editing("hi")),
    );
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::Save)),
        |s| {
            if let Some(editor) = s.sheet.get_mut() {
                editor.saving = true;
            }
        },
    );
    clock.advance(Duration::from_secs(1));
    store.receive(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::Saved)),
        |s| {
            s.saved = vec!["hi".into()];
            if let Some(editor) = s.sheet.get_mut() {
                editor.saving = false;
            }
        },
    );
    store.receive(InboxAction::Sheet(PresentationAction::Dismiss), |s| {
        s.sheet = PresentationState::none()
    });
    store.finish();
}

#[test]
fn a_sheet_that_dismisses_itself_stops_its_effects() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::NewDraft, |s| s.sheet.present(Editor::default()));
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::StartTicking)),
        |_| {},
    );
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::Cancel)),
        |_| {},
    );
    store.receive(InboxAction::Sheet(PresentationAction::Dismiss), |s| {
        s.sheet = PresentationState::none()
    });
    clock.advance(Duration::from_secs(3));
    store.finish();
}

#[test]
fn an_action_for_a_sheet_no_longer_presented_does_nothing() {
    let (mut store, _clock) = inbox();
    store.send(
        InboxAction::Sheet(PresentationAction::Presented(EditorAction::StartTicking)),
        |_| {},
    );
    store.send(InboxAction::Sheet(PresentationAction::Dismiss), |_| {});
    store.finish();
}

#[test]
fn screens_pushed_on_a_stack_take_their_own_actions() {
    let (mut store, _clock) = inbox();
    store.send(InboxAction::Path(StackAction::Push(editing("a"))), |s| {
        s.path.push(editing("a"));
    });
    store.send(InboxAction::Path(StackAction::Push(editing("b"))), |s| {
        s.path.push(editing("b"));
    });
    assert_eq!(
        store.state().path.ids().collect::<Vec<_>>(),
        [StackElementId(0), StackElementId(1)]
    );
    store.send(
        InboxAction::Path(StackAction::Element(
            StackElementId(0),
            EditorAction::Type("a2".into()),
        )),
        |s| {
            if let Some(editor) = s.path.get_mut(StackElementId(0)) {
                editor.text = "a2".into();
            }
        },
    );
    assert_eq!(
        store
            .state()
            .path
            .get(StackElementId(1))
            .map(|e| e.text.as_str()),
        Some("b")
    );
}

#[test]
fn a_screen_that_dismisses_itself_pops_itself_and_those_above_stopping_their_effects() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::Path(StackAction::Push(editing("a"))), |s| {
        s.path.push(editing("a"));
    });
    store.send(InboxAction::Path(StackAction::Push(editing("b"))), |s| {
        s.path.push(editing("b"));
    });
    let top = StackElementId(1);
    store.send(
        InboxAction::Path(StackAction::Element(top, EditorAction::StartTicking)),
        |_| {},
    );
    clock.advance(Duration::from_secs(1));
    store.receive(
        InboxAction::Path(StackAction::Element(top, EditorAction::Tick)),
        |s| {
            if let Some(editor) = s.path.get_mut(top) {
                editor.ticks = 1;
            }
        },
    );
    let bottom = StackElementId(0);
    store.send(
        InboxAction::Path(StackAction::Element(bottom, EditorAction::Cancel)),
        |_| {},
    );
    store.receive(InboxAction::Path(StackAction::PopFrom(bottom)), |s| {
        s.path = StackState::new()
    });
    // The top screen's ticks stopped with it.
    clock.advance(Duration::from_secs(3));
    store.finish();
}

#[test]
fn the_parent_pops_a_screen_and_its_late_actions_are_dropped() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::Path(StackAction::Push(editing("a"))), |s| {
        s.path.push(editing("a"));
    });
    let id = StackElementId(0);
    store.send(
        InboxAction::Path(StackAction::Element(id, EditorAction::StartTicking)),
        |_| {},
    );
    store.send(InboxAction::Path(StackAction::PopFrom(id)), |s| {
        s.path = StackState::new()
    });
    store.send(
        InboxAction::Path(StackAction::Element(id, EditorAction::Tick)),
        |_| {},
    );
    clock.advance(Duration::from_secs(3));
    // A new screen gets a new id.
    store.send(InboxAction::Path(StackAction::Push(editing("c"))), |s| {
        s.path.push(editing("c"));
    });
    assert_eq!(store.state().path.last().map(|(id, _)| id), Some(StackElementId(1)));
    store.finish();
}

#[test]
fn an_alerts_button_sends_its_action_and_dismisses_it() {
    let (mut store, _clock) = inbox();
    store.send(InboxAction::AskDelete, |s| s.alert.present(delete_alert()));
    let tapped = store
        .state()
        .alert
        .get()
        .and_then(|alert| alert.buttons.first())
        .map(ButtonState::tapped)
        .expect("the alert has a Delete button");
    store.send(InboxAction::Alert(tapped), |s| {
        s.alert = PresentationState::none();
        s.deleted = true;
    });
}

#[test]
fn an_alerts_cancel_button_only_dismisses_it() {
    let (mut store, _clock) = inbox();
    store.send(InboxAction::AskDelete, |s| s.alert.present(delete_alert()));
    let tapped = store
        .state()
        .alert
        .get()
        .and_then(|alert| alert.buttons.last())
        .map(ButtonState::tapped)
        .expect("the alert has a Keep button");
    assert_eq!(tapped, PresentationAction::Dismiss);
    store.send(InboxAction::Alert(tapped), |s| {
        s.alert = PresentationState::none()
    });
}

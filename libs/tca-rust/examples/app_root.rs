//! The root of a reminders app. Signed out, there's no workspace at all:
//! the root holds `Option<Workspace>`, so a signed-out app can't show a
//! stale inbox. Signed in, the workspace has two tabs, an inbox of
//! reminder rows, and a sign-out confirmation kept as data in state.
//!
//! Each level talks to the one above through delegate actions: a row says
//! it's done, the inbox says what was finished, the workspace says the
//! person signed out. No level reaches into its parent.
//!
//! Patterns: an optional root with `if_let`; tabs; delegates passed up
//! three levels; an `AlertState` in a `PresentationState`, its taps
//! handled with `alert`; signing out cancels each row's effect, because
//! `if_let` drops the state but not the work.

use std::sync::Arc;
use std::time::Duration;

use tca::{
    AlertState, ButtonRole, ButtonState, Clock, Effect, Identified, IdentifiedVec,
    PresentationAction, PresentationState, Reducer, TestClock, TestStore, alert, for_each, if_let,
};

const SNOOZE_FOR: Duration = Duration::from_secs(10 * 60);

/// Every row's snooze has this id; `for_each` keeps them apart.
const SNOOZE: &str = "snooze";

// A row.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ReminderId(u32);

#[derive(Clone, Debug, PartialEq)]
struct Reminder {
    id: ReminderId,
    title: String,
    snoozed: bool,
}

impl Identified for Reminder {
    type Id = ReminderId;

    fn id(&self) -> ReminderId {
        self.id
    }
}

#[derive(Debug, PartialEq)]
enum RowAction {
    Snooze,
    Woke,
    Done,
    Delegate(RowDelegate),
}

#[derive(Debug, PartialEq)]
enum RowDelegate {
    Done,
}

struct Row {
    clock: Arc<dyn Clock>,
}

impl Reducer for Row {
    type State = Reminder;
    type Action = RowAction;

    fn reduce(&self, reminder: &mut Reminder, action: RowAction) -> Effect<RowAction> {
        match action {
            RowAction::Snooze => {
                reminder.snoozed = true;
                let wake = self.clock.sleep(SNOOZE_FOR);
                Effect::future(async move {
                    wake.await;
                    RowAction::Woke
                })
                .cancellable(SNOOZE, true)
            }
            RowAction::Woke => {
                reminder.snoozed = false;
                Effect::none()
            }
            // The row can't remove itself: it asks whoever holds it.
            RowAction::Done => Effect::send(RowAction::Delegate(RowDelegate::Done)),
            RowAction::Delegate(_) => Effect::none(),
        }
    }
}

/// The scope `for_each` gives a row's effects, needed to cancel them from
/// outside the row.
fn row_scope(id: ReminderId) -> String {
    format!("{id:?}")
}

fn cancel_row(id: ReminderId) -> Effect<WorkspaceAction> {
    Effect::cancel(SNOOZE).scoped(&row_scope(id))
}

// The workspace.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Inbox,
    Done,
}

/// What the sign-out alert's buttons can send. Cancel sends nothing: it
/// only dismisses.
#[derive(Clone, Debug, PartialEq)]
enum SignOutAlert {
    Confirm,
}

#[derive(Clone, Debug, PartialEq)]
struct Workspace {
    account: String,
    tab: Tab,
    inbox: IdentifiedVec<Reminder>,
    done: Vec<String>,
    sign_out: PresentationState<AlertState<SignOutAlert>>,
}

#[derive(Debug, PartialEq)]
enum WorkspaceAction {
    Select(Tab),
    Row(ReminderId, RowAction),
    SignOutTapped,
    SignOut(PresentationAction<SignOutAlert>),
    Delegate(WorkspaceDelegate),
}

#[derive(Debug, PartialEq)]
enum WorkspaceDelegate {
    SignedOut,
}

/// Built in the reducer rather than the view, so a test can check what
/// the person is asked and tap its buttons.
fn sign_out_alert(account: &str) -> AlertState<SignOutAlert> {
    AlertState::new("Sign out?")
        .message(format!("Reminders for {account} stay on the server."))
        .button(ButtonState::destructive("Sign out", SignOutAlert::Confirm))
        .button(ButtonState::cancel("Stay"))
}

struct Working {
    row: Row,
}

impl Reducer for Working {
    type State = Workspace;
    type Action = WorkspaceAction;

    fn reduce(&self, workspace: &mut Workspace, action: WorkspaceAction) -> Effect<WorkspaceAction> {
        match action {
            WorkspaceAction::Select(tab) => {
                workspace.tab = tab;
                Effect::none()
            }
            WorkspaceAction::Row(id, RowAction::Delegate(RowDelegate::Done)) => {
                if let Some(reminder) = workspace.inbox.remove(&id) {
                    workspace.done.push(reminder.title);
                }
                // A snoozed row that's done must not wake later.
                cancel_row(id)
            }
            WorkspaceAction::Row(id, action) => {
                for_each(&self.row, &mut workspace.inbox, &id, action)
                    .map(move |action| WorkspaceAction::Row(id, action))
            }
            WorkspaceAction::SignOutTapped => {
                workspace.sign_out.present(sign_out_alert(&workspace.account));
                Effect::none()
            }
            WorkspaceAction::SignOut(tap) => match alert(&mut workspace.sign_out, tap) {
                Some(SignOutAlert::Confirm) => {
                    // The root is about to drop this workspace, and with
                    // it the rows, but their effects would keep running.
                    let stops = workspace.inbox.ids().map(cancel_row).collect::<Vec<_>>();
                    Effect::merge(stops).and(Effect::send(WorkspaceAction::Delegate(
                        WorkspaceDelegate::SignedOut,
                    )))
                }
                None => Effect::none(),
            },
            WorkspaceAction::Delegate(_) => Effect::none(),
        }
    }
}

// The root.

#[derive(Clone, Debug, Default, PartialEq)]
struct App {
    /// None while signed out.
    workspace: Option<Workspace>,
}

#[derive(Debug, PartialEq)]
enum AppAction {
    SignedIn {
        account: String,
        reminders: Vec<(u32, String)>,
    },
    Workspace(WorkspaceAction),
}

struct Root {
    working: Working,
}

impl Reducer for Root {
    type State = App;
    type Action = AppAction;

    fn reduce(&self, app: &mut App, action: AppAction) -> Effect<AppAction> {
        match action {
            AppAction::SignedIn { account, reminders } => {
                app.workspace = Some(workspace(&account, &reminders));
                Effect::none()
            }
            AppAction::Workspace(WorkspaceAction::Delegate(WorkspaceDelegate::SignedOut)) => {
                app.workspace = None;
                Effect::none()
            }
            AppAction::Workspace(action) => {
                if_let(&self.working, &mut app.workspace, action).map(AppAction::Workspace)
            }
        }
    }
}

fn workspace(account: &str, reminders: &[(u32, String)]) -> Workspace {
    let mut inbox = IdentifiedVec::new();
    for (id, title) in reminders {
        inbox.push(Reminder {
            id: ReminderId(*id),
            title: title.clone(),
            snoozed: false,
        });
    }
    Workspace {
        account: account.into(),
        tab: Tab::Inbox,
        inbox,
        done: Vec::new(),
        sign_out: PresentationState::none(),
    }
}

fn reminders() -> Vec<(u32, String)> {
    vec![(1, "Call the plumber".into()), (2, "Renew passport".into())]
}

/// A signed-in app, for tests that start there.
fn signed_in() -> (TestStore<Root>, TestClock) {
    let clock = TestClock::new();
    let root = Root {
        working: Working {
            row: Row {
                clock: Arc::new(clock.clone()),
            },
        },
    };
    let app = App {
        workspace: Some(workspace("sam", &reminders())),
    };
    (TestStore::new(app, root), clock)
}

fn row(id: u32, action: RowAction) -> AppAction {
    AppAction::Workspace(WorkspaceAction::Row(ReminderId(id), action))
}

fn signed_in_workspace(app: &mut App) -> &mut Workspace {
    app.workspace.as_mut().expect("signed in")
}

#[test]
fn signing_in_opens_the_inbox_tab_and_tabs_switch() {
    let (mut store, _clock) = signed_in();
    store.send(AppAction::Workspace(WorkspaceAction::Select(Tab::Done)), |s| {
        signed_in_workspace(s).tab = Tab::Done;
    });
    store.send(AppAction::Workspace(WorkspaceAction::Select(Tab::Inbox)), |s| {
        signed_in_workspace(s).tab = Tab::Inbox;
    });
}

#[test]
fn a_snoozed_reminder_comes_back_after_the_snooze() {
    let (mut store, clock) = signed_in();
    store.send(row(1, RowAction::Snooze), |s| {
        signed_in_workspace(s).inbox.update(&ReminderId(1), |r| r.snoozed = true);
    });
    clock.advance(SNOOZE_FOR);
    store.receive(row(1, RowAction::Woke), |s| {
        signed_in_workspace(s).inbox.update(&ReminderId(1), |r| r.snoozed = false);
    });
}

#[test]
fn a_reminder_marked_done_moves_from_the_inbox_to_done_and_never_wakes() {
    let (mut store, clock) = signed_in();
    store.send(row(1, RowAction::Snooze), |s| {
        signed_in_workspace(s).inbox.update(&ReminderId(1), |r| r.snoozed = true);
    });
    store.send(row(1, RowAction::Done), |_| {});
    store.receive(row(1, RowAction::Delegate(RowDelegate::Done)), |s| {
        let workspace = signed_in_workspace(s);
        workspace.inbox.remove(&ReminderId(1));
        workspace.done.push("Call the plumber".into());
    });
    clock.advance(SNOOZE_FOR);
    store.finish();
}

#[test]
fn the_sign_out_alert_offers_a_destructive_sign_out_and_a_way_back() {
    let (mut store, _clock) = signed_in();
    store.send(AppAction::Workspace(WorkspaceAction::SignOutTapped), |s| {
        signed_in_workspace(s).sign_out.present(sign_out_alert("sam"));
    });
    let roles: Vec<ButtonRole> = store
        .state()
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.sign_out.get())
        .map(|alert| alert.buttons.iter().map(|button| button.role).collect())
        .unwrap_or_default();
    assert_eq!(roles, [ButtonRole::Destructive, ButtonRole::Cancel]);
}

/// Taps the alert's button with `role`, as a view would.
fn tap(store: &mut TestStore<Root>, role: ButtonRole) -> AppAction {
    let tapped = store
        .state()
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.sign_out.get())
        .and_then(|alert| alert.buttons.iter().find(|button| button.role == role))
        .map(ButtonState::tapped)
        .expect("the alert has that button");
    AppAction::Workspace(WorkspaceAction::SignOut(tapped))
}

#[test]
fn staying_closes_the_alert_and_keeps_the_workspace() {
    let (mut store, _clock) = signed_in();
    store.send(AppAction::Workspace(WorkspaceAction::SignOutTapped), |s| {
        signed_in_workspace(s).sign_out.present(sign_out_alert("sam"));
    });
    let stay = tap(&mut store, ButtonRole::Cancel);
    store.send(stay, |s| {
        signed_in_workspace(s).sign_out = PresentationState::none();
    });
    store.finish();
}

#[test]
fn signing_out_drops_the_workspace_and_stops_every_snooze() {
    let (mut store, clock) = signed_in();
    for id in [1, 2] {
        store.send(row(id, RowAction::Snooze), |s| {
            signed_in_workspace(s).inbox.update(&ReminderId(id), |r| r.snoozed = true);
        });
    }
    store.send(AppAction::Workspace(WorkspaceAction::SignOutTapped), |s| {
        signed_in_workspace(s).sign_out.present(sign_out_alert("sam"));
    });
    let sign_out = tap(&mut store, ButtonRole::Destructive);
    store.send(sign_out, |s| {
        signed_in_workspace(s).sign_out = PresentationState::none();
    });
    store.receive(
        AppAction::Workspace(WorkspaceAction::Delegate(WorkspaceDelegate::SignedOut)),
        |s| s.workspace = None,
    );
    // Without the cancels, both rows would wake into a workspace that's
    // gone, and `finish` would find them still running.
    clock.advance(SNOOZE_FOR);
    store.finish();
}

#[test]
fn signing_in_again_starts_a_fresh_workspace() {
    let (mut store, _clock) = signed_in();
    store.send(row(2, RowAction::Done), |_| {});
    store.receive(row(2, RowAction::Delegate(RowDelegate::Done)), |s| {
        let workspace = signed_in_workspace(s);
        workspace.inbox.remove(&ReminderId(2));
        workspace.done.push("Renew passport".into());
    });
    store.send(AppAction::Workspace(WorkspaceAction::SignOutTapped), |s| {
        signed_in_workspace(s).sign_out.present(sign_out_alert("sam"));
    });
    let sign_out = tap(&mut store, ButtonRole::Destructive);
    store.send(sign_out, |s| {
        signed_in_workspace(s).sign_out = PresentationState::none();
    });
    store.receive(
        AppAction::Workspace(WorkspaceAction::Delegate(WorkspaceDelegate::SignedOut)),
        |s| s.workspace = None,
    );
    // A late tap meant for the old workspace is dropped.
    store.send(row(1, RowAction::Snooze), |_| {});
    store.send(
        AppAction::SignedIn {
            account: "sam".into(),
            reminders: reminders(),
        },
        |s| s.workspace = Some(workspace("sam", &reminders())),
    );
    store.finish();
}

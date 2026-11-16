//! What the server's events mean for the agent's turn (D201): working while
//! any session on the server runs, waiting on the user while any ask is
//! open, idle when neither. Only changes are reported.
use humata2_opencode::{Activity, Asked, Event, Turn};

fn started(session: &str) -> Event {
    Event::Started { session: session.to_owned() }
}

fn ended(session: &str) -> Event {
    Event::Ended { session: session.to_owned() }
}

fn asked(id: &str, session: &str) -> Event {
    Event::Asked(Asked {
        id: id.to_owned(),
        session: session.to_owned(),
        action: "shell".to_owned(),
        resources: vec!["echo hi".to_owned()],
    })
}

fn replied(id: &str, session: &str) -> Event {
    Event::Replied { session: session.to_owned(), request: id.to_owned() }
}

#[test]
fn a_run_works_then_goes_idle() {
    let mut activity = Activity::default();
    assert_eq!(activity.apply(&started("a")), Some(Turn::Working));
    assert_eq!(activity.apply(&ended("a")), Some(Turn::Idle));
}

#[test]
fn a_child_session_still_running_keeps_it_working() {
    let mut activity = Activity::default();
    activity.apply(&started("parent"));
    assert_eq!(activity.apply(&started("child")), None);
    assert_eq!(activity.apply(&ended("parent")), None);
    assert_eq!(activity.apply(&ended("child")), Some(Turn::Idle));
}

#[test]
fn an_open_ask_waits_on_the_user_until_it_is_replied() {
    let mut activity = Activity::default();
    activity.apply(&started("a"));
    assert_eq!(activity.apply(&asked("per_1", "a")), Some(Turn::NeedsInput));
    assert_eq!(activity.apply(&replied("per_1", "a")), Some(Turn::Working));
}

#[test]
fn a_run_ending_with_an_ask_open_drops_the_ask() {
    let mut activity = Activity::default();
    activity.apply(&started("a"));
    activity.apply(&asked("per_1", "a"));
    assert_eq!(activity.apply(&ended("a")), Some(Turn::Idle));
}

#[test]
fn the_same_ask_twice_and_events_it_doesnt_use_change_nothing() {
    let mut activity = Activity::default();
    activity.apply(&started("a"));
    activity.apply(&asked("per_1", "a"));
    assert_eq!(activity.apply(&asked("per_1", "a")), None);
    assert_eq!(activity.apply(&Event::Other), None);
    assert_eq!(activity.apply(&Event::Connected), None);
}

#[test]
fn reconnecting_takes_up_what_the_server_says_now() {
    let mut activity = Activity::default();
    activity.apply(&started("a"));
    // The link was down while "a" ended and "b" asked.
    let asks = [("per_9".to_owned(), "b".to_owned())];
    assert_eq!(
        activity.reset(["b".to_owned()], asks),
        Some(Turn::NeedsInput)
    );
    assert_eq!(activity.reset(Vec::<String>::new(), Vec::new()), Some(Turn::Idle));
}

#[test]
fn a_fresh_link_reports_its_first_state() {
    let mut activity = Activity::default();
    assert_eq!(activity.reset(Vec::<String>::new(), Vec::new()), Some(Turn::Idle));
}

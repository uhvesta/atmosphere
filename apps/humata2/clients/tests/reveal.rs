//! Reveal in Finder and Open terminal here, below the menus (PL-10).

use std::sync::Arc;

use humata2_clients::model::{Agent, SessionId};
use humata2_clients::{FakeDaemon, FakeReveal, FinderReveal, Reveal, RevealError, Terminals, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

#[test]
fn the_fake_remembers_every_path_in_order() {
    let fake = FakeReveal::default();
    assert!(fake.reveal("/Users/me/a").is_ok());
    assert!(fake.reveal("/Users/me/b").is_ok());
    assert_eq!(fake.revealed(), ["/Users/me/a", "/Users/me/b"]);
}

#[test]
fn a_home_path_is_made_absolute_before_finder_gets_it() {
    let finder = FinderReveal::with_home(Some("/Users/me".to_owned()));
    assert_eq!(finder.target("~/Humata/ml").ok().as_deref(), Some("/Users/me/Humata/ml"));
    assert_eq!(finder.target("~").ok().as_deref(), Some("/Users/me"));
}

#[test]
fn an_absolute_path_is_left_as_it_is() {
    let finder = FinderReveal::with_home(None);
    assert_eq!(finder.target("/srv/work/api").ok().as_deref(), Some("/srv/work/api"));
}

#[test]
fn a_home_path_without_a_home_folder_is_refused() {
    let finder = FinderReveal::with_home(None);
    assert!(matches!(finder.target("~/Humata"), Err(RevealError::NoHome { .. })));
}

#[test]
fn a_terminal_opened_in_a_folder_is_a_new_shell_tab_every_time() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let (first, _) = block_on(daemon.open_in(&id, Agent::Shell, "/srv/work/api")).expect("a shell starts");
    let (second, _) = block_on(daemon.open_in(&id, Agent::Shell, "/srv/work/api")).expect("another shell starts");
    assert_ne!(first, second);
    assert!(first > 0, "the agent keeps tab 0");
    assert!(
        daemon
            .calls()
            .iter()
            .any(|call| call.contains("/srv/work/api") && call.contains(id.as_str())),
        "the folder reached the daemon"
    );
}

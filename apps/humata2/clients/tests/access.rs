//! Agent access rules on the fake daemon: setting a rule, the rule with no
//! setting, and the grants "Always allow" leaves behind.

use std::sync::Arc;

use humata2_clients::model::{
    Answer, Capability, ChannelId, GrantId, RequestId, RuleSetting,
};
use humata2_clients::{Daemon, DaemonError, Event, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn daemon() -> FakeDaemon {
    FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()))
}

fn setting(daemon: &FakeDaemon, channel: &str, capability: &str) -> Option<RuleSetting> {
    daemon
        .workspace()
        .rules
        .get(&ChannelId::new(channel))
        .and_then(|rules| {
            rules
                .iter()
                .find(|rule| rule.capability.as_str() == capability)
        })
        .and_then(|rule| rule.setting)
}

#[test]
fn setting_a_rule_changes_it_and_announces_the_channels_rules() {
    let daemon = daemon();
    let mut events = block_on(daemon.events()).expect("events start");
    assert_eq!(setting(&daemon, "web", "files_read_other_session"), Some(RuleSetting::Allow));
    block_on(daemon.set_rule(
        &ChannelId::new("web"),
        &Capability::new("files_read_other_session"),
        RuleSetting::Never,
    ))
    .expect("the rule changes");
    assert_eq!(setting(&daemon, "web", "files_read_other_session"), Some(RuleSetting::Never));
    assert_eq!(setting(&daemon, "api", "files_read_other_session"), Some(RuleSetting::Allow));
    let mut announced = None;
    while let Ok(event) = events.try_recv() {
        if let Ok(Event::Rules { channel, rules }) = event {
            announced = Some((channel, rules));
        }
    }
    let (channel, rules) = announced.expect("a Rules event arrived");
    assert_eq!(channel, ChannelId::new("web"));
    assert!(rules.iter().any(|rule| rule.capability.as_str() == "files_read_other_session"
        && rule.setting == Some(RuleSetting::Never)));
}

#[test]
fn the_rule_with_no_setting_is_refused() {
    let daemon = daemon();
    let result = block_on(daemon.set_rule(
        &ChannelId::new("web"),
        &Capability::new("own_clones"),
        RuleSetting::Never,
    ));
    assert!(matches!(result, Err(DaemonError::Refused { .. })));
    assert_eq!(setting(&daemon, "web", "own_clones"), None);
}

#[test]
fn always_on_a_request_adds_a_grant_and_removing_it_takes_it_away() {
    let daemon = daemon();
    let grants = |daemon: &FakeDaemon| {
        daemon
            .workspace()
            .rules
            .get(&ChannelId::new("web"))
            .and_then(|rules| {
                rules
                    .iter()
                    .find(|rule| rule.capability.as_str() == "files_read_other_session")
                    .map(|rule| rule.grants.clone())
            })
            .unwrap_or_default()
    };
    assert!(grants(&daemon).is_empty());
    block_on(daemon.answer(
        &RequestId::new("r-edit"),
        Answer::Always {
            rule: "only under src/auth".to_owned(),
        },
    ))
    .expect("the answer is recorded");
    let after = grants(&daemon);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].scope, "only under src/auth");
    assert_eq!(after[0].session.as_str(), "s-passkey");

    block_on(daemon.remove_grant(&after[0].id)).expect("the grant is removed");
    assert!(grants(&daemon).is_empty());
    let again = block_on(daemon.remove_grant(&after[0].id));
    assert!(matches!(again, Err(DaemonError::NotFound { .. })));
    let unknown = block_on(daemon.remove_grant(&GrantId::new("nope")));
    assert!(matches!(unknown, Err(DaemonError::NotFound { .. })));
}

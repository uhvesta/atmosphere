//! Agent access rules against the real daemon: the user sets a channel's
//! rules and the defaults new channels start with, a restart keeps them,
//! and no agent can change them.

use humatad_test_common as common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const CHANNEL: &str = "ch_rules";
/// A channel no rule was ever set in or asked about.
const NEW_CHANNEL: &str = "ch_fresh";
const ASKER: &str = "se_asker";
const FRESH: &str = "se_fresh";
const TOKEN_HEADER: &str = "humata-session-token";

struct Fixture {
    daemon: Daemon,
    token: String,
    _attach: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for (id, channel) in [(ASKER, CHANNEL), (FRESH, NEW_CHANNEL)] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: channel.to_owned(),
                    machine: String::new(),
                    title: id.to_owned(),
                    agent: "zsh".to_owned(),
                    folder: dir.path().to_owned(),
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(pb::SessionId {
                    value: ASKER.to_owned(),
                }),
                repo: None,
                folder: String::new(),
            })
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap();
        let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
        let file = daemon.dir.path().join("token");
        attach
            .type_text(&format!(
                "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
                file.display()
            ))
            .await;
        let token = read_when_written(&file).await;
        Self {
            daemon,
            token,
            _attach: attach,
        }
    }

    async fn access(&self) -> pb::access_service_client::AccessServiceClient<Channel> {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
    }

    async fn rules(&self, channel: &str) -> pb::ChannelRules {
        self.access()
            .await
            .get_channel_rules(pb::GetChannelRulesRequest {
                channel: Some(channel_id(channel)),
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn set(
        &self,
        channel: &str,
        capability: pb::Capability,
        decision: pb::Decision,
        expected: Option<u64>,
    ) -> Result<pb::ChannelRules, tonic::Status> {
        self.access()
            .await
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(channel_id(channel)),
                capability: capability as i32,
                decision: decision as i32,
                expected_revision: expected.map(|value| pb::Revision { value }),
                ..Default::default()
            })
            .await
            .map(tonic::Response::into_inner)
    }
}

async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "{} was never written",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn channel_id(value: &str) -> pb::ChannelId {
    pb::ChannelId {
        value: value.to_owned(),
    }
}

fn decision_of(rules: &[pb::Rule], capability: pb::Capability) -> pb::Decision {
    rules
        .iter()
        .find(|rule| rule.capability == capability as i32)
        .map(pb::Rule::decision)
        .expect("the capability has a rule")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_the_user_sets_is_listed_and_a_restart_keeps_it() {
    let mut fixture = Fixture::start().await;
    let before = fixture.rules(CHANNEL).await;
    assert_eq!(
        decision_of(&before.rules, pb::Capability::GitPush),
        pb::Decision::Ask
    );

    let set = fixture
        .set(CHANNEL, pb::Capability::GitPush, pb::Decision::Never, None)
        .await
        .unwrap();
    assert_eq!(
        decision_of(&set.rules, pb::Capability::GitPush),
        pb::Decision::Never
    );
    let shown = fixture.rules(CHANNEL).await;
    assert_eq!(
        decision_of(&shown.rules, pb::Capability::GitPush),
        pb::Decision::Never
    );
    assert_eq!(shown.revision, set.revision);

    fixture.daemon.restart().await;

    let kept = fixture.rules(CHANNEL).await;
    assert_eq!(
        decision_of(&kept.rules, pb::Capability::GitPush),
        pb::Decision::Never
    );
    assert_eq!(kept.revision, set.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_change_rules() {
    let fixture = Fixture::start().await;
    let mut request = Request::new(pb::SetChannelRuleRequest {
        channel: Some(channel_id(CHANNEL)),
        capability: pb::Capability::GitPush as i32,
        decision: pb::Decision::Allow as i32,
        ..Default::default()
    });
    request.metadata_mut().insert(
        TOKEN_HEADER,
        MetadataValue::try_from(fixture.token.as_str()).unwrap(),
    );
    let refused = fixture
        .access()
        .await
        .set_channel_rule(request)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied);
    assert!(matches!(
        error_of(&refused).kind,
        Some(pb::error::Kind::UserOnly(_))
    ));

    let still = fixture.rules(CHANNEL).await;
    assert_eq!(
        decision_of(&still.rules, pb::Capability::GitPush),
        pb::Decision::Ask
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_made_on_a_stale_revision_conflicts() {
    let fixture = Fixture::start().await;
    let first = fixture
        .set(
            CHANNEL,
            pb::Capability::GitPush,
            pb::Decision::Never,
            Some(0),
        )
        .await
        .unwrap();
    let seen = first.revision.unwrap().value;

    fixture
        .set(
            CHANNEL,
            pb::Capability::GitPush,
            pb::Decision::Allow,
            Some(seen),
        )
        .await
        .unwrap();
    let stale = fixture
        .set(
            CHANNEL,
            pb::Capability::SessionNote,
            pb::Decision::Never,
            Some(seen),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error_of(&stale).kind,
        Some(pb::error::Kind::RevisionConflict(_))
    ));
    let shown = fixture.rules(CHANNEL).await;
    assert_eq!(
        decision_of(&shown.rules, pb::Capability::SessionNote),
        pb::Decision::Ask
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_that_does_not_exist_or_a_setting_not_offered_is_refused() {
    let fixture = Fixture::start().await;
    let no_rule = fixture
        .set(
            CHANNEL,
            pb::Capability::AccessChange,
            pb::Decision::Allow,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(no_rule.code(), Code::InvalidArgument);
    let no_allow = fixture
        .set(
            CHANNEL,
            pb::Capability::NetworkReach,
            pb::Decision::Allow,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(no_allow.code(), Code::InvalidArgument);
    let unknown_channel = fixture
        .set(
            "ch_nowhere",
            pb::Capability::GitPush,
            pb::Decision::Never,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(unknown_channel.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_channels_start_with_the_default_rules_the_user_set() {
    let mut fixture = Fixture::start().await;
    let set = fixture
        .access()
        .await
        .set_default_rules(pb::SetDefaultRulesRequest {
            rules: vec![pb::Rule {
                capability: pb::Capability::GitPush as i32,
                decision: pb::Decision::Never as i32,
                grants: Vec::new(),
            }],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        decision_of(&set.rules, pb::Capability::GitPush),
        pb::Decision::Never
    );

    let started = fixture.rules(NEW_CHANNEL).await;
    assert_eq!(
        decision_of(&started.rules, pb::Capability::GitPush),
        pb::Decision::Never
    );

    fixture.daemon.restart().await;
    let defaults = fixture
        .access()
        .await
        .get_default_rules(pb::GetDefaultRulesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        decision_of(&defaults.rules, pb::Capability::GitPush),
        pb::Decision::Never
    );
    assert_eq!(defaults.revision, set.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_marks_user_only_capabilities_as_not_held_by_agents() {
    let fixture = Fixture::start().await;
    let listed = fixture
        .access()
        .await
        .list_capabilities(pb::ListCapabilitiesRequest {})
        .await
        .unwrap()
        .into_inner()
        .capabilities;
    let find = |capability: pb::Capability| {
        listed
            .iter()
            .find(|info| info.capability == capability as i32)
            .expect("the catalog lists it")
    };

    let access_change = find(pb::Capability::AccessChange);
    assert!(!access_change.agents_may_hold);
    assert!(!access_change.rule_editable);
    assert_eq!(access_change.default_decision(), pb::Decision::Unspecified);
    let git_push = find(pb::Capability::GitPush);
    assert!(git_push.agents_may_hold);
    assert!(git_push.rule_editable);
    assert_eq!(git_push.default_decision(), pb::Decision::Ask);
}

#[tokio::test(flavor = "multi_thread")]
async fn explain_follows_the_rule_as_it_stands() {
    let fixture = Fixture::start().await;
    let asked = explain_push(&fixture).await;
    assert_eq!(asked.decision(), pb::Decision::Ask);

    fixture
        .set(CHANNEL, pb::Capability::GitPush, pb::Decision::Never, None)
        .await
        .unwrap();
    let never = explain_push(&fixture).await;
    assert_eq!(never.decision(), pb::Decision::Never);
    assert_eq!(never.source(), pb::DecisionSource::Rule);
}

/// What the daemon says would happen if `ASKER` pushed `acme/api`'s `fix`.
async fn explain_push(fixture: &Fixture) -> pb::ExplainResponse {
    fixture
        .access()
        .await
        .explain(pb::ExplainRequest {
            session: Some(pb::SessionId {
                value: ASKER.to_owned(),
            }),
            capability: pb::Capability::GitPush as i32,
            target: Some(pb::GrantTarget {
                target: Some(pb::grant_target::Target::RepoBranch(pb::RepoBranch {
                    repo: Some(pb::RepoRef {
                        owner: "acme".to_owned(),
                        name: "api".to_owned(),
                    }),
                    branch: "fix".to_owned(),
                })),
            }),
        })
        .await
        .unwrap()
        .into_inner()
}

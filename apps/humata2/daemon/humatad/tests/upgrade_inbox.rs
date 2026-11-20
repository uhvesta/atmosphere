//! UP-1, D281 b: a machine at another release than the host shows as one
//! VIP "Migrate the daemon on <machine>" item in the host's Inbox. Only the
//! user answers it and only with Migrate (Approve): never Deny, Always, a
//! reply or an Approve-all. One item per thing and machine while pending,
//! showing what the machine is now; withdrawn as resolved once the machine
//! is at the host's release again.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::upgrade;
use humatad_test_common::upgrade_rig;

use common::{error_of, shell_token};
use humata2_grpc::humata::v1 as pb;
use tonic::{Code, Status};
use tonic::transport::Channel;
use upgrade_rig::{BOX, box_id};

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey { value: value.to_owned() })
}

fn answer_of(answer: pb::request_answer::Answer) -> Option<pb::RequestAnswer> {
    Some(pb::RequestAnswer { answer: Some(answer) })
}

fn answering(item: &pb::InboxRequest, label: &str, answer: pb::request_answer::Answer) -> pb::AnswerRequestRequest {
    pb::AnswerRequestRequest {
        idempotency_key: key(label),
        id: item.id.clone(),
        answer: answer_of(answer),
        expected_revision: item.revision.clone(),
        ..Default::default()
    }
}

fn the_id(item: &pb::InboxRequest) -> &pb::RequestId {
    item.id.as_ref().expect("an Inbox item has an id")
}

/// `item` once more, failing the test unless it is still waiting.
async fn still_pending(rig: &remote_rig::Rig, item: &pb::InboxRequest, after: &str) {
    let now = upgrade::request(&rig.host, the_id(item)).await;
    assert_eq!(now.state(), pb::RequestState::Pending, "after {after}: {now:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_at_another_release_is_one_vip_migrate_item_with_no_requester() {
    let (mut rig, host_release) = upgrade_rig::at_one_release().await;

    let box_release = upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;

    assert!(item.vip, "a Migrate item is always VIP: {item:?}");
    assert_eq!(item.requester, None, "no agent asked for it");
    assert_eq!(item.always, None, "a Migrate item offers no Always");
    let migrate = upgrade::migrate_of(&item).expect("the item carries its Migrate card");
    assert_eq!(migrate.thing(), pb::MigrateThing::Daemon);
    assert_eq!(migrate.machine, Some(box_id()));
    assert_eq!(migrate.phase(), pb::MigratePhase::Waiting);
    let from = migrate.from.as_ref().expect("the card names what the machine is");
    let to = migrate.to.as_ref().expect("the card names what it would become");
    assert_eq!(from.bundle, box_release);
    assert_eq!(to.bundle, host_release);
    assert_ne!(box_release, host_release);
}

/// D892: only this Mac's daemon, CLI and host tools migrate on their own. A
/// machine's daemon item waits for the owner's click, through a second
/// detection too, and the machine keeps running its own release.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_at_another_release_is_never_migrated_until_migrate_is_clicked() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    let box_release = upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;

    // A second detection: the link drops and comes back.
    rig.link_lost(BOX).await;
    rig.link_back(BOX).await;

    still_pending(&rig, &item, "the link came back").await;
    let now = upgrade::request(&rig.host, the_id(&item)).await;
    let migrate = upgrade::migrate_of(&now).expect("the item carries its Migrate card");
    assert_eq!(migrate.phase(), pb::MigratePhase::Waiting, "nothing migrates before the click: {now:?}");
    assert_eq!(
        upgrade::build_of(rig.the(BOX).daemon()).await.bundle,
        box_release,
        "gpu still runs its own release"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_changes_release_again_keeps_its_one_item_showing_what_it_is_now() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let first = upgrade_rig::daemon_item(&rig).await;

    // Away and back at the same release: no second item.
    rig.link_lost(BOX).await;
    rig.link_back(BOX).await;
    // At yet another release: the same item, showing it.
    let now = upgrade_rig::box_at_release(&mut rig, "c").await;
    let shown = upgrade::request_until(&rig.host, the_id(&first), "showed release c", |item| {
        upgrade::migrate_of(item)
            .and_then(|migrate| migrate.from.as_ref())
            .is_some_and(|from| from.bundle == now)
    })
    .await;

    assert_eq!(shown.state(), pb::RequestState::Pending, "{shown:?}");
    let pending: Vec<pb::InboxRequest> = upgrade::inbox(&rig.host)
        .await
        .into_iter()
        .filter(|item| upgrade::pending_migrate(item, pb::MigrateThing::Daemon, &box_id()))
        .collect();
    assert_eq!(pending.len(), 1, "one item per thing and machine: {pending:?}");
    assert_eq!(pending[0].id, first.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_whose_binary_is_off_its_bundle_counts_as_another_release() {
    let (mut rig, host_release) = upgrade_rig::at_one_release().await;

    // The same bundle as the host's but for the release build's hash: the
    // machine's own humatad2 doesn't match it, so it names no release.
    upgrade_rig::box_at(&mut rig, &upgrade::manifest(&upgrade::release_sha(), "a")).await;
    let item = upgrade_rig::daemon_item(&rig).await;

    let migrate = upgrade::migrate_of(&item).expect("the item carries its Migrate card");
    let from = migrate.from.as_ref().expect("the card names what the machine is");
    assert_eq!(from.bundle, "");
    assert_eq!(from.own_sha256, upgrade::admin_sha());
    assert_eq!(migrate.to.as_ref().map(|to| to.bundle.as_str()), Some(host_release.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_migrate_item_takes_no_answer_but_migrate() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;
    let mut inbox = rig.host.inbox_service().await;

    use pb::request_answer::Answer;
    let others = [
        ("deny", Answer::Deny(pb::Deny::default())),
        ("deny with a note", Answer::Deny(pb::Deny { note: "not now".to_owned() })),
        ("always", Answer::Always(pb::AlwaysLet::default())),
        ("reply", Answer::Reply(pb::Reply { text: "later".to_owned(), ..Default::default() })),
    ];
    for (label, answer) in others {
        let answered = inbox.answer_request(answering(&item, label, answer)).await;
        assert!(answered.is_err(), "{label} answered a Migrate item: {answered:?}");
        still_pending(&rig, &item, label).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_all_leaves_a_migrate_item_waiting() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;
    let mut inbox = rig.host.inbox_service().await;

    let everywhere = Some(pb::InboxScope {
        scope: Some(pb::inbox_scope::Scope::Global(pb::GlobalInbox {})),
    });
    for (label, approve, ids) in [
        ("deny all", false, vec![]),
        ("approve all", true, vec![]),
        ("deny these", false, vec![the_id(&item).clone()]),
        ("approve these", true, vec![the_id(&item).clone()]),
    ] {
        let answered = inbox
            .answer_all(pb::AnswerAllRequest {
                idempotency_key: key(label),
                scope: everywhere.clone(),
                ids,
                approve,
                ..Default::default()
            })
            .await;
        if let Ok(response) = answered {
            let response = response.into_inner();
            assert!(
                response.answered.iter().all(|answered| answered.id != item.id),
                "{label} answered the Migrate item: {response:?}"
            );
        }
        still_pending(&rig, &item, label).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_answer_a_migrate_item() {
    let (mut rig, host_release) = upgrade_rig::at_one_release().await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[host.clone(), box_id()]).await;
    let on_host = rig.start_on(&channel, &host, "On this mac").await;
    let (token, _shell) = shell_token(&rig.host, &on_host).await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;

    // The agent's CLI is at the host's release: refused for being an agent,
    // not for its release.
    let asked = upgrade::with_headers(
        answering(&item, "agent-approves", pb::request_answer::Answer::Approve(pb::Approve {})),
        &[("humata-session-token", token.as_str()), (upgrade::BUNDLE_HEADER, host_release.as_str())],
    );
    let refused: Status = rig
        .host
        .inbox_service()
        .await
        .answer_request(asked)
        .await
        .expect_err("an agent can't answer a Migrate item");

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))), "{refused:?}");
    still_pending(&rig, &item, "the agent's approve").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_back_at_the_hosts_release_withdraws_its_item_as_resolved() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;

    upgrade_rig::box_at_release(&mut rig, "a").await;
    let done = upgrade::request_until(&rig.host, the_id(&item), "was withdrawn", |item| {
        item.state() == pb::RequestState::Withdrawn
    })
    .await;

    assert_eq!(done.withdraw_reason(), pb::WithdrawReason::Resolved, "{done:?}");
    assert!(
        !upgrade::inbox(&rig.host).await.iter().any(upgrade_rig::pending_on_box),
        "nothing is left to migrate on {BOX}"
    );
}

async fn machines(rig: &remote_rig::Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

/// `item` once it is withdrawn, failing the test unless that was because
/// its machine was removed.
async fn withdrawn_as_removed(rig: &remote_rig::Rig, item: &pb::InboxRequest) {
    let done = upgrade::request_until(&rig.host, the_id(item), "was withdrawn", |item| {
        item.state() == pb::RequestState::Withdrawn
    })
    .await;
    assert_eq!(done.withdraw_reason(), pb::WithdrawReason::MachineRemoved, "{done:?}");
}

/// D783: removing the machine withdraws its Migrate item, so a machine
/// added later under its name starts with none.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_machine_withdraws_its_migrate_item() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;

    let plan = machines(&rig)
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: Some(box_id()) })
        .await
        .expect("the removal is planned")
        .into_inner();
    let started = machines(&rig)
        .await
        .remove_machine(pb::RemoveMachineRequest { id: Some(box_id()), plan: plan.token, ..Default::default() })
        .await
        .expect("the removal starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    withdrawn_as_removed(&rig, &item).await;
}

/// D783: forgetting a machine that is gone withdraws its Migrate item the
/// same way.
#[tokio::test(flavor = "multi_thread")]
async fn forgetting_a_machine_withdraws_its_migrate_item() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;
    rig.the_mut(BOX).kill();
    rig.link_lost(BOX).await;

    let plan = machines(&rig)
        .await
        .plan_forget_machine(pb::PlanForgetMachineRequest { id: Some(box_id()) })
        .await
        .expect("the forget is planned")
        .into_inner();
    machines(&rig)
        .await
        .forget_machine(pb::ForgetMachineRequest {
            idempotency_key: key("forget"),
            id: Some(box_id()),
            plan: plan.token,
            presence: None,
        })
        .await
        .expect("the machine is forgotten");

    withdrawn_as_removed(&rig, &item).await;
}

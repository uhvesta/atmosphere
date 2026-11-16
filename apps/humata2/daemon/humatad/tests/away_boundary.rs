//! The AW-7 table as a boundary suite (AWAY T9): what each kind of call does
//! on a machine whose host is away, and, where it is cheap, while the host
//! is linked. A real role-machine daemon gets its policy copy over a real
//! `serve --stdio` link; the test is the agent (a shell tab's session
//! token), the app, and a person at a terminal logged in with the approval
//! passphrase. Assertions are on typed error kinds and status codes, never
//! on wording.
//!
//! Rows 2 and 3 as they go for the board (a read from the last copy, a
//! journaled board write) are in `tests/away_board_journal.rs`.
//! Row 7's last-copy read of the rules is D220's.

mod common;
#[path = "common/away.rs"]
mod away;
#[path = "common/away_machine.rs"]
mod away_machine;

use std::time::Duration;

use away::{Host, OK, PASSPHRASE, Terminal, host_reachable_becomes};
use away_machine::{
    ANSWER, ASKER, AwayMachine, CHANNEL, Link, OTHER, REQUEST, carried, files_of, kind, other_files,
    other_session, policy, push_main,
};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

/// `humata`'s exit code for `host_unreachable` (CLI.md §7).
const UNREACHABLE: i32 = 8;

fn is_pending(response: &pb::WaitResponse) -> bool {
    response.state() == pb::RequestState::Pending
}

// ----- Row 1: its own session's files, on the machine. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_1_own_session_files_are_read_and_written_on_the_machine() {
    for link in [Link::Away, Link::Reachable] {
        let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), link).await;
        machine
            .files()
            .await
            .create_folder(machine.as_agent(pb::CreateFolderRequest {
                idempotency_key: None,
                location: Some(files_of(ASKER, "made")),
            }))
            .await
            .unwrap_or_else(|status| panic!("{link:?}: its own clone wasn't written: {status:?}"));
        assert!(machine.clone.join("made").is_dir(), "{link:?}: the folder is on this machine's disk");

        let listed = machine
            .files()
            .await
            .list_directory(machine.as_agent(pb::ListDirectoryRequest {
                location: Some(files_of(ASKER, "")),
                page: None,
            }))
            .await
            .unwrap_or_else(|status| panic!("{link:?}: its own clone wasn't read: {status:?}"))
            .into_inner();
        assert!(
            listed
                .entries
                .iter()
                .any(|entry| entry.location.as_ref().is_some_and(|at| at.path.ends_with("made"))),
            "{link:?}: the folder it made is listed: {listed:?}"
        );
    }
}

// ----- Row 3: a question to the user is journaled while away. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_3_a_question_to_the_user_is_kept_and_journaled_while_the_host_is_away() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let asked = machine
        .inbox()
        .await
        .ask_user(machine.as_agent(pb::AskUserRequest {
            question: "which schema do you want?".to_owned(),
            ..Default::default()
        }))
        .await
        .expect("a question to the user is taken while the host is away")
        .into_inner();
    assert!(is_pending(&asked));

    let entries = machine.journal(2).await;
    assert_eq!(entries.len(), 1, "only the question is journaled: {entries:?}");
    assert_eq!(entries[0].operation, REQUEST);
    assert_eq!(carried(&entries[0]).id, asked.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn row_3_a_question_while_the_host_is_linked_is_sent_as_it_is_made() {
    let mut machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Reachable).await;
    let asked = machine
        .inbox()
        .await
        .ask_user(machine.as_agent(pb::AskUserRequest {
            question: "which schema do you want?".to_owned(),
            ..Default::default()
        }))
        .await
        .expect("the question is taken")
        .into_inner();
    assert!(is_pending(&asked));
    let live = machine.host.as_mut().expect("linked").journal(1).await;
    assert_eq!(live[0].operation, REQUEST);
    assert_eq!(carried(&live[0]).id, asked.id);
}

// ----- Row 4: Never is denied, wherever the host is. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_4_a_call_the_rules_say_never_is_denied_and_nothing_is_kept() {
    let never = [(pb::Capability::FilesReadOtherSession, pb::Decision::Never)];
    for link in [Link::Away, Link::Reachable] {
        let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &never), link).await;

        let asked = machine
            .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
            .await
            .expect_err("asking ahead for what is Never was taken");
        assert_eq!(asked.code(), Code::PermissionDenied, "{link:?}: {asked:?}");
        assert!(is_never(&asked), "{link:?}: {asked:?}");

        let read = machine
            .files()
            .await
            .list_directory(machine.as_agent(pb::ListDirectoryRequest {
                location: Some(files_of(OTHER, "")),
                page: None,
            }))
            .await
            .expect_err("another session's files were read under a Never rule");
        assert_eq!(read.code(), Code::PermissionDenied, "{link:?}: {read:?}");
        assert!(is_never(&read), "{link:?}: {read:?}");

        if link == Link::Away {
            assert!(machine.journal(2).await.is_empty(), "a denied call leaves nothing for the host");
        }
    }
}

fn is_never(status: &tonic::Status) -> bool {
    matches!(
        kind(status),
        Some(pb::error::Kind::Denied(denied)) if denied.source() == pb::denied::Source::NeverRule
    )
}

// ----- Row 5: an Ask on something on this machine. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_5_under_wait_an_ask_on_this_machine_is_held_in_its_inbox() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let files = machine
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
        .await
        .expect("the request is held in the machine's Inbox");
    assert!(is_pending(&files));
    let typing = machine
        .ask_ahead(pb::Capability::SessionType, other_session())
        .await
        .expect("typing into a session here is held too");
    assert!(is_pending(&typing));

    let entries = machine.journal(2).await;
    let held: Vec<_> = entries.iter().map(|entry| (entry.operation.as_str(), carried(entry).id)).collect();
    assert_eq!(held, [(REQUEST, files.id), (REQUEST, typing.id)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn row_5_under_skip_an_ask_on_this_machine_is_denied_saying_the_user_is_away() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Skip, &[]), Link::Away).await;
    for (capability, target) in [
        (pb::Capability::FilesReadOtherSession, other_files("src")),
        (pb::Capability::SessionType, other_session()),
    ] {
        let refused = machine
            .ask_ahead(capability, target)
            .await
            .expect_err("a Skip rule held a request");
        assert_eq!(refused.code(), Code::Unavailable, "{capability:?}: {refused:?}");
        assert!(
            matches!(kind(&refused), Some(pb::error::Kind::UserAway(_))),
            "{capability:?}: {refused:?}"
        );
    }
    assert!(machine.journal(2).await.is_empty(), "nothing was asked, so nothing is journaled");
}

#[tokio::test(flavor = "multi_thread")]
async fn row_5_while_the_host_is_linked_an_ask_is_a_request_whatever_the_away_rule() {
    for mode in [pb::AwayMode::Wait, pb::AwayMode::Skip] {
        let machine = AwayMachine::start(policy(mode, &[]), Link::Reachable).await;
        let asked = machine
            .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
            .await
            .unwrap_or_else(|status| panic!("{mode:?}: the host is linked, so the user is asked: {status:?}"));
        assert!(is_pending(&asked), "{mode:?}");
    }
}

// ----- Row 6: anything that needs another machine or GitHub. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_6_under_skip_a_push_is_denied_saying_the_host_is_away() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Skip, &[]), Link::Away).await;
    let refused = machine
        .ask_ahead(pb::Capability::GitPush, push_main())
        .await
        .expect_err("a push was taken while the host is away");
    assert_eq!(refused.code(), Code::Unavailable, "{refused:?}");
    assert!(matches!(kind(&refused), Some(pb::error::Kind::HostUnreachable(_))), "{refused:?}");
    assert!(machine.journal(2).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn row_6_while_the_host_is_linked_a_push_is_asked_of_the_user() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Skip, &[]), Link::Reachable).await;
    let asked = machine
        .ask_ahead(pb::Capability::GitPush, push_main())
        .await
        .expect("with the host linked the push is asked");
    assert!(is_pending(&asked));
}

#[tokio::test(flavor = "multi_thread")]
async fn row_6_under_wait_a_push_waits_for_the_host_and_is_then_asked() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let mut inbox = machine.inbox().await;
    let request = machine.as_agent(pb::RequestAccessRequest {
        capability: pb::Capability::GitPush as i32,
        target: Some(push_main()),
        reason: "AW-7".to_owned(),
        ..Default::default()
    });
    let mut waiting = tokio::spawn(async move { inbox.request_access(request).await });
    // A call refused at once would have ended by now; one held is still going.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut waiting)
            .await
            .is_err(),
        "the call should wait for the host"
    );

    // The host comes back: the call is judged again and is now an ordinary
    // request in the host's Inbox.
    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy(pb::AwayMode::Wait, &[])).await;
    host_reachable_becomes(&machine.daemon, true).await;
    let asked = waiting
        .await
        .expect("the call's task ended")
        .expect("a call that needs the host waited for it, and was not refused")
        .into_inner();
    assert_eq!(asked.state(), pb::RequestState::Pending);

    let entries = host.journal(1).await;
    assert_eq!(entries[0].operation, REQUEST);
    assert_eq!(carried(&entries[0]).id, asked.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn row_6_a_held_call_whose_deadline_passes_while_away_is_host_unreachable_and_keeps_nothing() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let mut request = machine.as_agent(pb::RequestAccessRequest {
        capability: pb::Capability::GitPush as i32,
        target: Some(push_main()),
        reason: "AW-7".to_owned(),
        ..Default::default()
    });
    request.set_timeout(Duration::from_secs(1));
    let refused = machine
        .inbox()
        .await
        .request_access(request)
        .await
        .expect_err("a call that outlasted its wait for the host went on");
    assert_eq!(refused.code(), Code::Unavailable, "{refused:?}");
    assert!(matches!(kind(&refused), Some(pb::error::Kind::HostUnreachable(_))), "{refused:?}");
    assert!(
        common::error_of(&refused).retry.is_some_and(|retry| retry.same_key),
        "{refused:?}: the call can be repeated with the same key"
    );
    assert!(machine.journal(2).await.is_empty(), "nothing was kept for the host");
}

// ----- Row 7: user-only calls. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_7_while_away_only_answering_with_the_passphrase_and_reading_the_copy_are_open_to_the_user() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let held = machine
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
        .await
        .expect("the request is held");
    let id = held.id.expect("a held request has an id");

    // The app on a machine has no passphrase login: it can't answer.
    let answer = pb::AnswerRequestRequest {
        id: Some(id.clone()),
        answer: Some(pb::RequestAnswer {
            answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
        }),
        ..Default::default()
    };
    let refused = machine
        .inbox()
        .await
        .answer_request(answer)
        .await
        .expect_err("a machine's Inbox was answered without the passphrase");
    assert!(
        matches!(kind(&refused), Some(pb::error::Kind::PresenceRequired(_))),
        "{refused:?}"
    );

    // Everything else user-only is refused as needing the host, even with
    // the passphrase login.
    let mut terminal = Terminal::open(&machine.daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    assert_eq!(
        terminal
            .exit_code(r#""$HUMATA" --json access revoke gr_none > /dev/null 2>&1"#, None)
            .await,
        UNREACHABLE
    );
    let changed = machine
        .access()
        .await
        .set_default_rules(pb::SetDefaultRulesRequest {
            away: Some(pb::AwayRule {
                mode: pb::AwayMode::Skip as i32,
                low_risk: Vec::new(),
            }),
            ..Default::default()
        })
        .await
        .expect_err("a machine changed its copy of the rules");
    assert!(matches!(kind(&changed), Some(pb::error::Kind::HostUnreachable(_))), "{changed:?}");

    // The rules it judges by are readable from the copy (D220).
    let rules = machine
        .access()
        .await
        .get_channel_rules(pb::GetChannelRulesRequest {
            channel: Some(pb::ChannelId {
                value: CHANNEL.to_owned(),
            }),
        })
        .await
        .expect("the rules are read from the copy")
        .into_inner();
    assert_eq!(rules.revision, None, "a copy offers no revision to write from");
    assert_eq!(rules.copy.map(|stamp| stamp.revision), Some(1));

    // The passphrase login answers the held request, and the answer is
    // journaled after the request.
    let approve = format!(r#""$HUMATA" --json inbox approve {} > /dev/null 2>&1"#, id.value);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);
    let entries = machine.journal(2).await;
    let kinds: Vec<&str> = entries.iter().map(|entry| entry.operation.as_str()).collect();
    assert_eq!(kinds, [REQUEST, ANSWER]);
    assert_eq!(carried(&entries[1]).state(), pb::RequestState::Approved);
}

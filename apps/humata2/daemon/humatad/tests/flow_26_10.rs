//! Flow §26.10, working away from the Mac (AWAY T9). The first test runs it
//! on the remote rig: devbox-1 is a real role-machine daemon behind a real
//! link to a real host, the link is cut as a closed Mac does, a person logs
//! in with the approval passphrase at a terminal on devbox-1 and lists,
//! attaches, types and detaches, an agent's request waits in devbox-1's
//! Inbox under Wait, the person approves it with `humata inbox`, and when
//! the link comes back the host's own Inbox shows the request answered by
//! the user at devbox-1's CLI. The second test is the same Inbox steps on
//! the away harness with a request to read another session's files, which
//! a `humata` in a tab can't express (the CLI's ask-ahead targets are
//! sessions and networks), and reads the journal as the host would.

mod common;
#[path = "common/away.rs"]
mod away;
#[path = "common/away_machine.rs"]
mod away_machine;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::time::{Duration, Instant};

use away::{OK, PASSPHRASE, Terminal};
use away_machine::{ANSWER, AwayMachine, Link, REQUEST, carried, other_files, policy};
use common::{Attach, binary};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");
const TITLE: &str = "api-rate-limiter";
const BOX: &str = "devbox-1";
/// Ctrl-], which leaves `humata attach`.
const DETACH: u8 = 0x1d;

/// A shell tab of `session`, opened through the host while the link is up:
/// its processes stay devbox-1's after the link is cut.
async fn shell_tab(rig: &Rig, session: &pb::SessionId) -> Attach {
    let tab = rig
        .host
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session.clone()),
            repo: None,
        })
        .await
        .expect("a shell tab opens on devbox-1")
        .into_inner()
        .id
        .expect("the tab has an id");
    let open = pb::AttachOpen {
        session: Some(session.clone()),
        ..common::open(&tab, 0)
    };
    Attach::start(&rig.host, open).await
}

/// Waits until `ready` holds, reading state every 20 ms for at most
/// [`LONG`].
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + LONG;
    while !ready() {
        assert!(Instant::now() < deadline, "{what} never happened within {LONG:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The id of the request `session` has in `remote`'s own Inbox, once it is
/// there.
async fn held_request(remote: &remote_rig::Remote, session: &pb::SessionId) -> String {
    let mut found = None;
    until("the request waiting in devbox-1's Inbox", || {
        found = remote
            .store()
            .inbox_requests()
            .unwrap()
            .into_iter()
            .find(|request| request.session == session.value)
            .map(|request| request.id);
        found.is_some()
    })
    .await;
    found.unwrap()
}

/// The host's Inbox feed. It is opened before the Mac's link comes back,
/// which is when devbox-1's journal, and the answer in it, reach the host.
async fn inbox_feed(rig: &Rig) -> tonic::Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Inbox as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Inbox feed opens")
        .into_inner()
}

/// The host's own Inbox entry for `id`, once it is answered. The Inbox is
/// listed, then listed again each time `feed` tells of a change.
async fn answered_on_host(
    rig: &Rig,
    id: &str,
    mut feed: tonic::Streaming<pb::Event>,
) -> pb::InboxRequest {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let listed = pb::inbox_service_client::InboxServiceClient::new(rig.channel().await)
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .expect("the host lists its Inbox")
            .into_inner()
            .requests;
        let answered = listed.into_iter().find(|request| {
            request.id.as_ref().is_some_and(|found| found.value == id)
                && request.state() == pb::RequestState::Approved
        });
        if let Some(answered) = answered {
            return answered;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!(
                "the host's Inbox feed ended ({ended:?}) before request {id} was answered on the host"
            ),
            Err(_) => panic!("the host's Inbox never showed request {id} answered within {LONG:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn flow_26_10_a_person_works_on_devbox_1_with_the_mac_closed_and_the_mac_sees_it_on_return() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let devbox = Rig::box_id(BOX);
    let on = [rig.host_id().await, devbox.clone()];
    let channel = rig.create_channel("Payments", &[REPO], &on).await;
    let working = rig.start_on(&channel, &devbox, TITLE).await;
    let other = rig.start_on(&channel, &devbox, "other-work").await;
    rig.wait_session(&working, "live", is_live).await;
    rig.wait_session(&other, "live", is_live).await;
    let remote = rig.the(BOX);

    // Before the Mac closes: the While I'm away rule is Wait, the approval
    // passphrase is set, and devbox-1 holds both (its policy copy).
    let mut access = pb::access_service_client::AccessServiceClient::new(rig.channel().await);
    access
        .set_away_rule(pb::SetAwayRuleRequest {
            channel: channel.id.clone(),
            away: Some(pb::AwayRule {
                mode: pb::AwayMode::Wait as i32,
                low_risk: Vec::new(),
            }),
            ..Default::default()
        })
        .await
        .expect("the channel's While I'm away rule is set");
    access
        .set_approval_passphrase(pb::SetApprovalPassphraseRequest {
            passphrase: PASSPHRASE.to_owned(),
            ..Default::default()
        })
        .await
        .expect("the approval passphrase is set");
    until("devbox-1 holding the approval passphrase's hash", || {
        remote
            .store()
            .setting("approval_passphrase")
            .unwrap()
            .is_some_and(|saved| !saved.value.is_empty())
    })
    .await;

    // The agent's side: a process in the session's own shell tab that asks
    // for access the moment the test lets it (a fifo it blocks on), so the
    // ask is made while the Mac is closed. The tab prints `fifo-ready` once
    // the fifo exists (the typed line shows `fifo''-ready`, which doesn't match).
    let mut tab = shell_tab(&rig, &working).await;
    let go = remote.daemon().dir.path().join("go");
    tab.type_text(&format!(
        "mkfifo '{go}'; echo fifo''-ready; ( cat '{go}' > /dev/null; HUMATA_SOCKET='{socket}' '{humata}' --json access ask-ahead session-type session:{other} --reason 'to compare the schemas' > /dev/null 2>&1 ) &\r",
        go = go.display(),
        socket = remote.daemon().socket.display(),
        humata = binary("HUMATA").display(),
        other = other.value,
    ))
    .await;
    tab.wait_for("fifo-ready").await;

    // The agent's own tab, found through the host while it is linked: the
    // session has a shell tab too, which `humata attach` mustn't pick.
    let agent_tab = common::agent_tab(&rig.host, &working).await;

    // 1. The Mac is closed. Sessions on devbox-1 keep working.
    rig.link_lost(BOX).await;

    // 2. From another computer, `humata login` with the passphrase, then
    //    `humata ls`.
    let mut terminal = Terminal::open(remote.daemon());
    // A release machine answers no one without a credential, so its view of
    // the host is read by the login itself: refused while the host is
    // reachable, accepted once the machine sees it gone.
    let deadline = Instant::now() + LONG;
    while terminal.log_in(PASSPHRASE).await != OK {
        assert!(Instant::now() < deadline, "devbox-1 never took the passphrase login within {LONG:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        terminal.exit_code(r#""$HUMATA" --json ls > /dev/null 2>&1"#, None).await,
        OK
    );

    // 3. `humata attach`, type into the agent's terminal, detach.
    let run = terminal.start(&format!(
        r#""$HUMATA" attach {TITLE} --tab {} < /dev/tty"#,
        agent_tab.value
    ));
    // Its note on the screen, once the terminal is raw and the attach is open.
    terminal.wait_for_screen("Attached. Ctrl-] detaches.").await;
    let said = "hello from the road";
    terminal.type_raw(format!("{said}\r").as_bytes());
    // The agent's echo of what it was given, shown on the screen through the attach.
    terminal.wait_for_screen(&format!("got: {said}")).await;
    terminal.type_raw(&[DETACH]);
    assert_eq!(terminal.finished(run).await, OK);

    // 4. The agent asks; the rule is Wait, so the request waits in devbox-1's Inbox.
    let release = go.clone();
    tokio::task::spawn_blocking(move || std::fs::write(release, "go\n"))
        .await
        .unwrap()
        .expect("the agent's process was released");
    let id = held_request(remote, &working).await;

    // 5. `humata inbox` shows it; `humata inbox approve` answers it.
    let listing = remote.daemon().dir.path().join("inbox.json");
    assert_eq!(
        terminal
            .exit_code(&format!(r#""$HUMATA" --json inbox > '{}' 2> /dev/null"#, listing.display()), None)
            .await,
        OK
    );
    assert!(
        std::fs::read_to_string(&listing).unwrap().contains(&id),
        "`humata inbox` lists the request that waits"
    );
    let approve = format!(r#""$HUMATA" --json inbox approve {id} > /dev/null 2>&1"#);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);

    // 6. The Mac reconnects: its Inbox shows the request answered, by the
    //    user at devbox-1's CLI. The session ran on throughout.
    let inbox = inbox_feed(&rig).await;
    rig.link_back(BOX).await;
    let answered = answered_on_host(&rig, &id, inbox).await;
    let by = answered.answered.and_then(|answered| answered.by).and_then(|actor| actor.actor);
    match by {
        Some(pb::actor::Actor::User(user)) => {
            assert_eq!(user.via(), pb::user_actor::Via::CliOnMachine);
            assert_eq!(user.machine, Some(devbox));
        }
        other => panic!("the answer is credited to the user on devbox-1, not {other:?}"),
    }
    rig.wait_session(&working, "live after the Mac came back", is_live).await;
    for held in remote.tabs() {
        assert!(common::holder_answers(&held), "tab {} kept running through the flow", held.id);
    }
    drop(tab);
}

#[tokio::test(flavor = "multi_thread")]
async fn flow_26_10_a_request_to_read_another_sessions_files_waits_and_is_answered_on_the_machine() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let held = machine
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
        .await
        .expect("the request waits in the machine's Inbox");
    let id = held.id.expect("a waiting request has an id");

    let mut terminal = Terminal::open(&machine.daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    assert_eq!(
        terminal.exit_code(r#""$HUMATA" --json ls > /dev/null 2>&1"#, None).await,
        OK
    );
    let listing = machine.daemon.dir.path().join("inbox.json");
    assert_eq!(
        terminal
            .exit_code(&format!(r#""$HUMATA" --json inbox > '{}' 2> /dev/null"#, listing.display()), None)
            .await,
        OK
    );
    assert!(std::fs::read_to_string(&listing).unwrap().contains(&id.value));
    let approve = format!(r#""$HUMATA" --json inbox approve {} > /dev/null 2>&1"#, id.value);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);

    // What the Mac is sent when it comes back: the request, then the answer.
    let entries = machine.journal(2).await;
    let kinds: Vec<&str> = entries.iter().map(|entry| entry.operation.as_str()).collect();
    assert_eq!(kinds, [REQUEST, ANSWER]);
    assert_eq!(carried(&entries[1]).id.as_ref(), Some(&id));
    assert_eq!(carried(&entries[1]).state(), pb::RequestState::Approved);
}

//! MV-4: while a session moves, it names its target (`moving_to`) and its
//! move (`running_operation`), and its terminal is read-only until the agent
//! resumes. Typing through the Type call (an agent's, a script's or the CLI's
//! way in) is refused with `session_busy` naming the move, nothing reaches
//! the agent, and once the agent resumes on the new machine Type works
//! there.
//!
//! The host holds the move at `move-started` (the session already shows
//! Moving, its agent still runs where it was) until the test opens a FIFO,
//! so the state is seen mid-move with no timing. Every other wait is on the
//! host's feeds; `LONG` is only a safety net.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};

use common::error_of;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

/// Typed during the move; must reach no agent.
const DURING: &str = "typed while moving";
/// Typed once the agent resumed on the new machine.
const AFTER: &str = "typed once it resumed";

fn text(path: &Path) -> String {
    path.display().to_string()
}

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

fn move_request(id: &pb::SessionId, to: &pb::MachineId, key: &str) -> pb::MoveSessionRequest {
    pb::MoveSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(id.clone()),
        to: Some(to.clone()),
    }
}

/// Types `line` into the session's agent through the host's Type call.
async fn type_line(rig: &Rig, id: &pb::SessionId, key: &str, line: &str) -> Result<pb::TypeResponse, tonic::Status> {
    rig.host
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
            session: Some(id.clone()),
            text: format!("{line}\n"),
            when: pb::type_request::When::Now as i32,
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

/// Restarts the host so its moves hold at `at` until the returned FIFO is
/// opened for writing.
async fn hold_moves_at(rig: &mut Rig, at: &str) -> PathBuf {
    let hold = rig.host.dir.path().join(format!("hold-{at}.fifo"));
    assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
    rig.restart_host(&[("HUMATA2_HOLD_AT", at), ("HUMATA2_HOLD_FIFO", &text(&hold))]).await;
    hold
}

/// Lets a move held at the FIFO `hold` go on.
async fn release(hold: &Path) {
    let hold = hold.to_owned();
    let opened = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(hold).map(drop));
    tokio::time::timeout(remote_rig::LONG, opened)
        .await
        .expect("the move never got to where it is held")
        .unwrap()
        .expect("the held move's FIFO opens");
}

fn is_moving(session: &pb::Session) -> bool {
    session.status == pb::SessionStatus::Moving as i32
}

/// MV-4: Type into a moving session is refused with `session_busy` naming
/// the session and the move; the line reaches no agent; once the agent
/// resumes on the new machine, Type reaches it there. Once for a session on
/// this Mac (its own tab) and once for one on box-1 (the host relays Type).
#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_moving_session_is_refused_and_works_once_it_resumes_on_the_new_machine() {
    for from in ["host", "box-1"] {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
        let host = rig.host_id().await;
        let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone(), two.clone()]).await;
        let (on, to, lands) = if from == "host" {
            (host.clone(), one.clone(), "box-1")
        } else {
            (one.clone(), two.clone(), "box-2")
        };
        let id = rig.start_on(&channel, &on, "Cache warmup").await;
        rig.wait_session(&id, "live", is_live).await;
        let hold = hold_moves_at(&mut rig, "move-started").await;
        rig.wait_session(&id, "live again", is_live).await;

        let started = sessions(&rig)
            .await
            .move_session(move_request(&id, &to, &format!("move-typed-{from}")))
            .await
            .expect("the move starts")
            .into_inner();
        let moving = rig.wait_session(&id, "moving", is_moving).await;

        let refused = type_line(&rig, &id, &format!("type-during-{from}"), DURING)
            .await
            .expect_err("typing into a moving session is refused");
        match error_of(&refused).kind {
            Some(pb::error::Kind::SessionBusy(busy)) => {
                assert_eq!(busy.session.as_ref(), Some(&id), "{from}: the refusal names the session");
                assert_eq!(busy.running, started.id, "{from}: the refusal names the move that blocks it");
            }
            other => panic!("{from}: expected session_busy, got {other:?} ({refused:?})"),
        }
        assert_eq!(moving.moving_to.as_ref(), Some(&to), "{from}: a moving session names where it moves");
        assert_eq!(moving.running_operation, started.id, "{from}: a moving session names its move");

        release(&hold).await;
        let ended = rig.ended(&started).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{from}: {:?}", ended.error);
        let resumed = rig
            .wait_session(&id, "live on the new machine", |s| {
                s.machine.as_ref() == Some(&to) && is_live(s) && !is_moving(s)
            })
            .await;
        assert_eq!(resumed.moving_to, None, "{from}: a resumed session no longer names a move target");
        assert_eq!(resumed.running_operation, None, "{from}: a resumed session runs no operation");

        // The old agent kept running until the move packed it, well after
        // the refused Type, so a line that reached it would be in the
        // conversation the new machine took over.
        let conversation = rig.the(lands).conversation_id(&id);
        let talk = std::fs::read_to_string(rig.the(lands).conversation(&conversation)).unwrap();
        assert!(!talk.contains(DURING), "{from}: the line typed during the move reached the agent");

        let tab = common::agent_tab(&rig.host, &id).await;
        let open = pb::AttachOpen {
            session: Some(id.clone()),
            ..common::open(&tab, 0)
        };
        let mut attach = common::Attach::start(&rig.host, open).await;
        attach.wait_for("agent up").await;
        let typed = type_line(&rig, &id, &format!("type-after-{from}"), AFTER)
            .await
            .expect("typing works once the agent resumed");
        assert!(typed.typed, "{from}: the line was typed, not queued");
        attach.wait_for(&format!("got: {AFTER}")).await;
        assert!(!attach.shows(&format!("got: {DURING}")), "{from}: the new agent's terminal got the line typed during the move");
    }
}

/// The text of a review comment sent into the session while it moves.
const COMMENT: &str = "review comment sent while moving";

/// A draft on README.md's first line, as the user makes it.
async fn draft_comment(rig: &Rig, id: &pb::SessionId) -> pb::Comment {
    pb::comment_service_client::CommentServiceClient::new(rig.channel().await)
        .create_comment(pb::CreateCommentRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            anchor: Some(pb::CommentAnchor {
                file: Some(pb::RepoPath {
                    repo: Some(common::repo_ref(REPO.0, REPO.1)),
                    path: "README.md".to_owned(),
                }),
                lines: Some(pb::LineRange { first: 1, last: 1 }),
                side: pb::DiffSide::New as i32,
                ..Default::default()
            }),
            text: COMMENT.to_owned(),
            destination: Some(pb::CommentDestination {
                destination: Some(pb::comment_destination::Destination::Terminal(pb::SessionTerminal {})),
            }),
        })
        .await
        .expect("the draft is made")
        .into_inner()
}

/// Sends `comment` into its session's agent, as the user's Send does.
async fn send_comment(rig: &Rig, id: &pb::SessionId, comment: &pb::Comment) -> pb::SendCommentsResponse {
    pb::comment_service_client::CommentServiceClient::new(rig.channel().await)
        .send_comments(pb::SendCommentsRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            ids: vec![comment.id.clone().expect("a draft has an id")],
        })
        .await
        .expect("the send answers")
        .into_inner()
}

/// The status the review API lists `comment` with.
async fn listed_status(rig: &Rig, id: &pb::SessionId, comment: &pb::Comment) -> pb::CommentStatus {
    let listed = pb::comment_service_client::CommentServiceClient::new(rig.channel().await)
        .list_comments(pb::ListCommentsRequest {
            session: Some(id.clone()),
            statuses: [
                pb::CommentStatus::Draft,
                pb::CommentStatus::Queued,
                pb::CommentStatus::Sent,
                pb::CommentStatus::Addressed,
                pb::CommentStatus::Resolved,
            ]
            .iter()
            .map(|status| *status as i32)
            .collect(),
            file: None,
            page: None,
            archived: false,
        })
        .await
        .expect("the comments are listed")
        .into_inner();
    let found = listed
        .comments
        .into_iter()
        .find(|listed| listed.id == comment.id)
        .expect("the comment is listed");
    pb::CommentStatus::try_from(found.status).expect("a known status")
}

/// MV-4 for review comments: comments sent to a session while it moves are
/// refused the way Type is. None of the batch is typed, and the comment
/// stays unsent. Once the agent resumes on the new machine, sending the
/// same comment types it and marks it sent. The session is on this Mac:
/// the review API answers for a session on its own machine.
#[tokio::test(flavor = "multi_thread")]
async fn comments_sent_while_moving_are_not_typed_and_send_once_it_resumes() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &host, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let hold = hold_moves_at(&mut rig, "move-started").await;
    rig.wait_session(&id, "live again", is_live).await;
    let comment = draft_comment(&rig, &id).await;

    let started = sessions(&rig)
        .await
        .move_session(move_request(&id, &one, "move-comments"))
        .await
        .expect("the move starts")
        .into_inner();
    rig.wait_session(&id, "moving", is_moving).await;

    // Sent while the session moves: the refusal types nothing, so the
    // comment is still pending.
    send_comment(&rig, &id, &comment).await;
    let pending = listed_status(&rig, &id, &comment).await;
    assert!(
        matches!(pending, pb::CommentStatus::Draft | pb::CommentStatus::Queued),
        "a comment sent to a moving session is still pending, not {pending:?}"
    );

    release(&hold).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on the new machine", |s| {
        s.machine.as_ref() == Some(&one) && is_live(s) && !is_moving(s)
    })
    .await;

    // The old agent kept running until the move packed its conversation,
    // so a comment typed during the move would be in what the new machine
    // took over.
    let lands = rig.the("box-1");
    let conversation = lands.conversation_id(&id);
    let talk = std::fs::read_to_string(lands.conversation(&conversation)).unwrap();
    assert!(!talk.contains(COMMENT), "the comment sent during the move reached the agent");

    // Sent once the agent resumed: it is typed, and the comment is sent.
    let tab = common::agent_tab(&rig.host, &id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    let mut attach = common::Attach::start(&rig.host, open).await;
    tokio::time::timeout(remote_rig::LONG, attach.wait_for("agent up"))
        .await
        .expect("the resumed agent comes up");
    send_comment(&rig, &id, &comment).await;
    assert_eq!(
        listed_status(&rig, &id, &comment).await,
        pb::CommentStatus::Sent,
        "sending the comment once the agent resumed marks it sent"
    );

    // The agent answers its lines in order, so once it has answered the
    // line typed after the comment, the comment's lines are in its
    // conversation too.
    let typed = type_line(&rig, &id, "type-after-comments", AFTER)
        .await
        .expect("typing works once the agent resumed");
    assert!(typed.typed, "the line was typed, not queued");
    tokio::time::timeout(remote_rig::LONG, attach.wait_for(&format!("got: {AFTER}")))
        .await
        .expect("the resumed agent answers the line typed after the comment");
    let lands = rig.the("box-1");
    let conversation = lands.conversation_id(&id);
    let talk = std::fs::read_to_string(lands.conversation(&conversation)).unwrap();
    assert!(talk.contains(COMMENT), "the comment sent once the agent resumed was not typed");
}

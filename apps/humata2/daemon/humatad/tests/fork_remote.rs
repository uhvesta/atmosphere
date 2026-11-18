//! Forking onto another machine (FK-8, FK-12, flow 26.5; D195): the fork
//! gets the parent's code (commit, uncommitted and untracked files) on a
//! new branch in the target machine's folder, and for code and context a
//! copy of the conversation under its own id; the parent keeps running
//! where it was, untouched. A fork that can't reach its target leaves
//! nothing.
//!
//! Waits are on the host's feeds (the rig's `wait_session`, `ended`).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/mcp_client.rs"]
mod mcp_client;

use std::path::Path;

use common::{git, holder_answers};
use humata2_grpc::humata::v1 as pb;
use mcp_client::Mcp;
use serde_json::json;
use remote_rig::{Rig, is_live};
use tonic::Code;

const REPO: (&str, &str) = ("acme", "api");
const PARENT: &str = "sessions/cache-warmup/api";
const FORK: &str = "sessions/try-it/api";

fn text(path: &Path) -> String {
    path.display().to_string()
}

fn branch_of(clone: &Path) -> String {
    git(&["-C", &text(clone), "rev-parse", "--abbrev-ref", "HEAD"])
}

/// One commit, one changed file and one untracked file in `clone`.
fn make_work(clone: &Path) -> String {
    std::fs::write(clone.join("README.md"), "changed in the parent\n").unwrap();
    std::fs::write(clone.join("login.rs"), "fn login() {}\n").unwrap();
    git(&["-C", &text(clone), "add", "login.rs"]);
    git(&["-C", &text(clone), "commit", "-m", "login"]);
    std::fs::write(clone.join("notes.txt"), "untracked\n").unwrap();
    git(&["-C", &text(clone), "rev-parse", "HEAD"])
}

async fn say(rig: &Rig, id: &pb::SessionId, line: &str) {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    let mut attach = common::Attach::start(&rig.host, open).await;
    attach.wait_for("agent up").await;
    attach.type_text(&format!("{line}\n")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

fn request(parent: &pb::SessionId, kind: pb::ForkKind, to: &pb::MachineId) -> pb::ForkSessionRequest {
    pb::ForkSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: format!("fork-{}-{}", parent.value, to.value),
        }),
        parent: Some(parent.clone()),
        kind: kind as i32,
        title: "Try it".to_owned(),
        machine: Some(to.clone()),
        ..Default::default()
    }
}

/// Forks and waits for the operation; the fork's id when it made one.
async fn fork(rig: &Rig, parent: &pb::SessionId, kind: pb::ForkKind, to: &pb::MachineId) -> (pb::Operation, Option<pb::SessionId>) {
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(request(parent, kind, to))
        .await
        .expect("the fork starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    let made = match &ended.result {
        Some(pb::operation::Result::Session(id)) => Some(id.clone()),
        _ => None,
    };
    (ended, made)
}

/// FK-8, 26.5: this Mac's session forked onto box-1 with code and context.
#[tokio::test(flavor = "multi_thread")]
async fn code_and_context_fork_onto_box_1_has_the_parents_work_on_a_new_branch_and_a_copied_conversation() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let parent = rig.start_on(&channel, &host, "Cache warmup").await;
    let before = rig.wait_session(&parent, "live", is_live).await;
    let from = rig.host_path(&channel, PARENT);
    let head = make_work(&from);
    say(&rig, &parent, "remember the retry").await;
    let holders = common::stored_tabs(&rig.host.state);

    let (ended, made) = fork(&rig, &parent, pb::ForkKind::CodeAndContext, &one).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let id = made.expect("the fork made a session");
    let child = rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
    assert_eq!(child.fork_origin.as_ref().and_then(|o| o.parent.clone()), Some(parent.clone()));
    assert_ne!(child.repos[0].branch, before.repos[0].branch);

    let to = rig.the("box-1").path(&channel, FORK);
    assert_eq!(branch_of(&to), child.repos[0].branch);
    assert!(git(&["-C", &text(&to), "merge-base", "--is-ancestor", &head, "HEAD"]).is_empty(), "the parent's commit is in the fork");
    assert_eq!(std::fs::read_to_string(to.join("README.md")).unwrap(), "changed in the parent\n");
    assert_eq!(std::fs::read_to_string(to.join("notes.txt")).unwrap(), "untracked\n");

    let box_one = rig.the("box-1");
    let theirs = box_one.conversation_id(&id);
    assert_ne!(theirs, before.conversation_id, "the fork's conversation has its own id");
    let talk = std::fs::read_to_string(box_one.conversation(&theirs)).unwrap();
    assert!(talk.contains("remember the retry"), "the conversation came along: {talk}");
    say(&rig, &id, "only in the fork").await;
    let mine = std::fs::read_to_string(rig.host_conversation(&before.conversation_id)).unwrap();
    assert!(!mine.contains("only in the fork"));

    assert_eq!(branch_of(&from), before.repos[0].branch, "the parent keeps its branch");
    assert_eq!(common::stored_tabs(&rig.host.state).len(), holders.len());
    assert!(holders.iter().all(holder_answers), "the parent keeps running");
    assert!(is_live(&rig.session(&parent).await));
}

/// FK-8 between boxes: box-1's session forked onto box-2, code only, is a
/// fresh agent with the parent's files; box-1 runs on.
#[tokio::test(flavor = "multi_thread")]
async fn code_only_fork_from_box_1_onto_box_2_is_a_fresh_agent_and_the_parent_keeps_running() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let parent = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    make_work(&rig.the("box-1").path(&channel, PARENT));
    say(&rig, &parent, "remember the retry").await;
    let holder = rig.the("box-1").tabs()[0].holder_pid;

    let (ended, made) = fork(&rig, &parent, pb::ForkKind::CodeOnly, &two).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let id = made.expect("the fork made a session");
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;

    let to = rig.the("box-2").path(&channel, FORK);
    assert_eq!(std::fs::read_to_string(to.join("notes.txt")).unwrap(), "untracked\n");
    assert_eq!(std::fs::read_to_string(to.join("README.md")).unwrap(), "changed in the parent\n");
    let theirs = rig.the("box-2").conversation_id(&id);
    let talk = std::fs::read_to_string(rig.the("box-2").conversation(&theirs)).unwrap_or_default();
    assert!(!talk.contains("remember the retry"), "a code-only fork starts a fresh conversation");

    let tabs = rig.the("box-1").tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].holder_pid, holder, "the parent's agent runs on");
    assert!(is_live(&rig.session(&parent).await));
}

/// FK-12: a fork onto an offline machine is refused before anything is
/// made, and leaves no session behind.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_onto_an_offline_box_is_refused_and_leaves_nothing() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let parent = rig.start_on(&channel, &host, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    rig.link_lost("box-1").await;

    let refused = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(request(&parent, pb::ForkKind::CodeAndContext, &one))
        .await
        .expect_err("an offline target is refused at once");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");

    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 1, "only the parent: {listed:?}");
    assert!(!rig.the("box-1").path(&channel, FORK).exists());
}

/// The token the daemon gave a shell tab of `id` on this Mac, which an
/// agent of the session holds.
async fn session_token(rig: &Rig, id: &pb::SessionId) -> String {
    let tab = rig
        .host
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = common::Attach::start(&rig.host, common::open(&tab, 0)).await;
    let file = rig.host.dir.path().join("token");
    attach
        .type_text(&format!("printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}.part && mv {0}.part {0}\r", file.display()))
        .await;
    let deadline = tokio::time::Instant::now() + remote_rig::LONG;
    loop {
        if let Ok(token) = std::fs::read_to_string(&file) {
            return token;
        }
        assert!(tokio::time::Instant::now() < deadline, "the shell never wrote its token");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Flow 26.5: the agent forks its own session onto box-1 through MCP; the
/// rule "Start or fork sessions on another machine" is Ask me, so the user
/// sees a card first, and once they approve the fork runs on box-1 with
/// the parent's work while the parent runs on.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_forks_itself_onto_another_machine_once_the_user_approves() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let parent = rig.start_on(&channel, &host, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    let head = make_work(&rig.host_path(&channel, PARENT));
    pb::access_service_client::AccessServiceClient::new(rig.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: channel.id.clone(),
            capability: pb::Capability::SessionStartOtherMachine as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();
    let token = session_token(&rig, &parent).await;
    let mut mcp = Mcp::start(&rig.host.socket, &token).await;

    let arguments = json!({
        "machine": one.value,
        "kind": "code_and_context",
        "title": "Try it",
        "reason": "benchmark on the bigger box",
        "wait_seconds": 0,
    });
    let asked = mcp.call(1, "session_fork", arguments).await;
    assert_eq!(asked["structuredContent"]["outcome"], "pending", "{asked}");
    let request = asked["structuredContent"]["request_id"].as_str().unwrap().to_owned();
    assert!(rig.session(&parent).await.forks.is_empty(), "nothing forks before the user answers");

    // TP-6, IN-10: the card shows the machine, the kind, each repo with
    // how it is copied and its size, and a usage note.
    let card = pb::inbox_service_client::InboxServiceClient::new(rig.channel().await)
        .get_request(pb::GetRequestRequest { id: Some(pb::RequestId { value: request.clone() }) })
        .await
        .unwrap()
        .into_inner();
    let card_reason = card.reason.clone();
    let Some(pb::request_preview::Preview::NewSession(shown)) = card.preview.and_then(|preview| preview.preview) else {
        panic!("the card previews no new session: {:?}", card.action_title);
    };
    assert_eq!(shown.machine.as_ref(), Some(&one), "the card names box-1");
    assert_eq!(card_reason, "benchmark on the bigger box", "the card carries the agent's reason");
    assert_eq!(shown.fork_kind, pb::ForkKind::CodeAndContext as i32);
    assert_eq!(shown.repos.len(), 1, "{shown:?}");
    let repo = &shown.repos[0];
    assert_eq!(repo.copy, pb::new_session_preview::RepoCopy::BranchAndUncommitted as i32);
    assert!(repo.clone_bytes > 0, "the card says how much is copied: {shown:?}");
    assert!(!shown.usage_note.is_empty(), "the card carries a usage note");

    pb::inbox_service_client::InboxServiceClient::new(rig.channel().await)
        .answer_request(pb::AnswerRequestRequest {
            id: Some(pb::RequestId { value: request.clone() }),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let waited = mcp.call(2, "inbox_wait", json!({ "request_id": request })).await;
    assert_eq!(waited["isError"], false, "{waited}");
    let operation = waited["structuredContent"]["result"]["operation"].as_str().expect("the fork's operation").to_owned();
    let ended = rig
        .ended(&pb::Operation {
            id: Some(pb::OperationId { value: operation }),
            ..Default::default()
        })
        .await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let Some(pb::operation::Result::Session(id)) = ended.result else {
        panic!("the fork made no session: {ended:?}");
    };
    rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
    let to = rig.the("box-1").path(&channel, FORK);
    assert!(git(&["-C", &text(&to), "merge-base", "--is-ancestor", &head, "HEAD"]).is_empty(), "the parent's commit is in the fork");
    assert!(is_live(&rig.session(&parent).await), "the parent runs on");
}

/// A typed branch git won't take is refused when a fork onto or from a box
/// is asked for, as is one this Mac's parent already uses; no operation
/// starts and nothing is copied.
#[tokio::test(flavor = "multi_thread")]
async fn a_bad_or_taken_typed_branch_is_refused_when_the_fork_is_asked_for() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let here = rig.start_on(&channel, &host, "Cache warmup").await;
    let there = rig.start_on(&channel, &one, "Retry queue").await;
    let parent = rig.wait_session(&here, "live", is_live).await;
    rig.wait_session(&there, "live", is_live).await;
    let taken = parent.repos[0].branch.clone();

    for (from, to, branch, code) in [
        (&here, &one, "bad name".to_owned(), Code::InvalidArgument),
        (&there, &host, "bad..name".to_owned(), Code::InvalidArgument),
        (&here, &one, taken, Code::AlreadyExists),
    ] {
        let refused = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
            .fork_session(pb::ForkSessionRequest { branch: branch.clone(), ..request(from, pb::ForkKind::CodeAndContext, to) })
            .await
            .expect_err("the branch is refused at once");
        assert_eq!(refused.code(), code, "{branch}: {refused:?}");
    }
    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 2, "only the parents: {listed:?}");
    assert!(!rig.the("box-1").path(&channel, FORK).exists());
}

/// D208, FK-8: a context-only fork works in its parent's clones, so it
/// runs only on its parent's machine. Asked for onto another machine, from
/// this Mac or from a box, it is refused at once and nothing is made.
#[tokio::test(flavor = "multi_thread")]
async fn a_context_only_fork_onto_another_machine_is_refused() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let here = rig.start_on(&channel, &host, "Cache warmup").await;
    let there = rig.start_on(&channel, &one, "Retry queue").await;
    rig.wait_session(&here, "live", is_live).await;
    rig.wait_session(&there, "live", is_live).await;

    for (from, to) in [(&here, &one), (&there, &host)] {
        let refused = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
            .fork_session(request(from, pb::ForkKind::ContextOnly, to))
            .await
            .expect_err("a context-only fork elsewhere is refused at once");
        assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
        let previewed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
            .preview_fork(request(from, pb::ForkKind::ContextOnly, to))
            .await
            .expect_err("its preview is refused the same way");
        assert_eq!(previewed.code(), Code::FailedPrecondition, "{previewed:?}");
    }
    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 2, "only the parents: {listed:?}");
    assert!(!rig.the("box-1").path(&channel, FORK).exists());
}

/// D187, D208: a context-only fork of box-1's session, asked for on the
/// host onto box-1, runs on box-1 in its parent's folder and clones, with a
/// copy of the conversation under its own id. The host lists it on box-1 as
/// the parent's fork, its preview names the parent's branches and copies
/// nothing, and the parent's agent runs on.
#[tokio::test(flavor = "multi_thread")]
async fn a_context_only_fork_of_a_box_session_onto_that_box_works_in_the_parents_clones_there() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let parent = rig.start_on(&channel, &one, "Cache warmup").await;
    let before = rig.wait_session(&parent, "live", is_live).await;
    say(&rig, &parent, "remember the retry").await;
    let holder = rig.the("box-1").tabs()[0].holder_pid;

    let preview = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .preview_fork(request(&parent, pb::ForkKind::ContextOnly, &one))
        .await
        .expect("the preview answers")
        .into_inner();
    assert_eq!(preview.copy_bytes, 0, "a context-only fork copies no code: {preview:?}");
    let named: Vec<&str> = preview.branches.iter().map(|named| named.branch.as_str()).collect();
    let theirs: Vec<&str> = before.repos.iter().map(|repo| repo.branch.as_str()).collect();
    assert_eq!(named, theirs, "it works on the parent's branches");

    let (ended, made) = fork(&rig, &parent, pb::ForkKind::ContextOnly, &one).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let id = made.expect("the fork made a session");
    let child = rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
    assert_eq!(child.fork_origin.as_ref().and_then(|o| o.parent.clone()), Some(parent.clone()));

    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    let shown = listed.iter().find(|session| session.id.as_ref() == Some(&id)).expect("the host lists the fork");
    assert_eq!(shown.machine.as_ref(), Some(&one), "listed on box-1");

    let store = rig.the("box-1").store();
    let paths = |session: &pb::SessionId| -> Vec<std::path::PathBuf> {
        store
            .session_detail(&session.value)
            .unwrap()
            .unwrap_or_else(|| panic!("box-1 has no start record for {}", session.value))
            .repos
            .into_iter()
            .map(|repo| repo.path)
            .collect()
    };
    assert_eq!(paths(&id), paths(&parent), "on box-1 it works in the parent's clones");
    assert_eq!(branch_of(&rig.the("box-1").path(&channel, PARENT)), before.repos[0].branch, "no new branch");
    assert!(!rig.the("box-1").path(&channel, FORK).exists(), "no copy of the code");

    let box_one = rig.the("box-1");
    let conversation = box_one.conversation_id(&id);
    assert_ne!(conversation, box_one.conversation_id(&parent), "the fork's conversation has its own id");
    let talk = std::fs::read_to_string(box_one.conversation(&conversation)).unwrap_or_default();
    assert!(talk.contains("remember the retry"), "the conversation came along: {talk}");

    let tabs = box_one.tabs();
    assert_eq!(tabs.len(), 2, "the parent's agent and the fork's: {tabs:?}");
    assert!(tabs.iter().any(|tab| tab.holder_pid == holder), "the parent's agent runs on");
    assert!(is_live(&rig.session(&parent).await));
}

/// FK-4, FK-8: the preview of a fork onto or from a box names the new
/// branch the fork's clones would get, a branch the parent isn't on, and
/// what the copy would weigh, measured on the parent's machine. Previewing
/// makes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn previewing_a_fork_onto_or_from_a_box_names_its_branch_and_size_and_makes_nothing() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let here = rig.start_on(&channel, &host, "Cache warmup").await;
    let there = rig.start_on(&channel, &one, "Retry queue").await;
    let parents = [
        rig.wait_session(&here, "live", is_live).await,
        rig.wait_session(&there, "live", is_live).await,
    ];

    for ((from, to), parent) in [(&here, &one), (&there, &host)].into_iter().zip(&parents) {
        for kind in [pb::ForkKind::CodeAndContext, pb::ForkKind::CodeOnly] {
            let preview = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
                .preview_fork(request(from, kind, to))
                .await
                .expect("the preview answers")
                .into_inner();
            assert_eq!(preview.branches.len(), parent.repos.len(), "{kind:?}: {preview:?}");
            for (named, repo) in preview.branches.iter().zip(&parent.repos) {
                assert!(!named.branch.is_empty(), "{kind:?}: {preview:?}");
                assert_ne!(named.branch, repo.branch, "a new branch: {preview:?}");
            }
            assert!(preview.copy_bytes > 0, "{kind:?}: {preview:?}");
            assert!(!preview.text.is_empty());
        }
    }
    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 2, "only the parents: {listed:?}");
    assert!(!rig.the("box-1").path(&channel, FORK).exists());
    assert!(staged_at(&rig.host_path(&channel, "")).is_empty());
}

/// What `machine`'s channel folder has staged under `.carry`.
fn staged_at(folder: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(folder.join(".carry"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// The host's fork journals left in its state.
fn fork_journals(rig: &Rig) -> usize {
    std::fs::read_dir(rig.host.state.join("carry").join("forks"))
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

/// A host killed while it forks this Mac's session onto box-1 undoes the
/// fork at its next start when the agent's record wasn't written yet: no
/// fork on box-1, nothing staged on either machine, the parent runs on.
/// Killed once the record was written, it finishes it: the fork runs on
/// box-1 with the parent's work.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_cut_short_by_a_host_restart_is_undone_or_finished() {
    for (at, finished) in [("fork-packed", false), ("fork-copied", false), ("fork-unpacked", false), ("fork-recorded", true)] {
        let mut rig = Rig::start(&["box-1"], &[REPO]).await;
        let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
        let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
        let parent = rig.start_on(&channel, &host, "Cache warmup").await;
        rig.wait_session(&parent, "live", is_live).await;
        let head = make_work(&rig.host_path(&channel, PARENT));

        rig.restart_host(&[("HUMATA2_STOP_AT", at)]).await;
        let _ = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
            .fork_session(request(&parent, pb::ForkKind::CodeAndContext, &one))
            .await;
        rig.host_stopped().await;
        rig.restart_host(&[]).await;

        if finished {
            let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
                .list_sessions(pb::ListSessionsRequest::default())
                .await
                .unwrap()
                .into_inner()
                .sessions;
            let fork = listed
                .iter()
                .filter_map(|session| session.id.clone())
                .find(|id| id != &parent)
                .unwrap_or_else(|| panic!("{at}: the fork is recorded: {listed:?}"));
            rig.wait_session(&fork, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
            let to = rig.the("box-1").path(&channel, FORK);
            assert!(git(&["-C", &text(&to), "merge-base", "--is-ancestor", &head, "HEAD"]).is_empty(), "{at}: the parent's commit is in the fork");
        } else {
            let fork_on_box = rig.the("box-1").path(&channel, FORK);
            let box_staging = rig.the("box-1").path(&channel, "");
            let host_staging = rig.host_path(&channel, "");
            rig.wait_session(&parent, "the fork undone", |s| {
                is_live(s) && !fork_on_box.exists() && staged_at(&box_staging).is_empty() && staged_at(&host_staging).is_empty()
            })
            .await;
            let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
                .list_sessions(pb::ListSessionsRequest::default())
                .await
                .unwrap()
                .into_inner()
                .sessions;
            assert_eq!(listed.len(), 1, "{at}: only the parent: {listed:?}");
            assert!(rig.the("box-1").store().sessions().unwrap().is_empty(), "{at}: box-1 keeps no record of the fork");
        }
        rig.wait_session(&parent, "the journal done with", |_| fork_journals(&rig) == 0).await;
        assert!(holder_answers(&common::stored_tabs(&rig.host.state)[0]), "{at}: the parent runs on");
    }
}

/// D218, D221: a host killed right after box-1 took a context-only fork of
/// box-1's session lists that fork at its next start, once box-1's link is
/// up: on box-1, with the parent as its fork origin, exactly once, and
/// box-1 made it only once.
#[tokio::test(flavor = "multi_thread")]
async fn a_context_only_fork_on_a_box_cut_short_by_a_host_restart_is_listed_once_the_box_is_linked() {
    let mut rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let parent = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;

    rig.restart_host(&[("HUMATA2_STOP_AT", "fork-context-asked")]).await;
    let _ = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(request(&parent, pb::ForkKind::ContextOnly, &one))
        .await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;

    let shown = rig.wait_session(&parent, "its fork listed", |s| !s.forks.is_empty()).await;
    assert_eq!(shown.forks.len(), 1, "one fork: {:?}", shown.forks);
    let id = shown.forks[0].clone();
    let child = rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
    let origin = child.fork_origin.expect("the fork has its origin");
    assert_eq!(origin.parent.as_ref(), Some(&parent));
    assert_eq!(origin.kind, pb::ForkKind::ContextOnly as i32);

    let deadline = tokio::time::Instant::now() + remote_rig::LONG;
    while fork_journals(&rig) != 0 {
        assert!(tokio::time::Instant::now() < deadline, "the fork's journal was never done with");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 2, "the parent and its fork, once: {listed:?}");
    assert_eq!(rig.the("box-1").store().sessions().unwrap().len(), 2, "box-1 made the fork once");
}

/// Forks `parent` onto `to` under a host killed at `at`, and starts the host
/// again once it has died there.
async fn fork_killed_at(rig: &mut Rig, parent: &pb::SessionId, to: &pb::MachineId, at: &str) {
    rig.restart_host(&[("HUMATA2_STOP_AT", at)]).await;
    let _ = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(request(parent, pb::ForkKind::CodeAndContext, to))
        .await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
}

/// After a restart has undone a fork cut short before its copy landed: the
/// parent runs on where it was with its work, the host lists the parent
/// alone with no fork, box-1 keeps no record, agent or folder of the fork,
/// and nothing is staged on either side.
async fn assert_fork_undone(rig: &Rig, channel: &pb::Channel, parent: &pb::SessionId, head: &str) {
    let shown = rig.session(parent).await;
    assert!(is_live(&shown), "the parent runs on: {shown:?}");
    assert!(shown.forks.is_empty(), "the parent lists no fork: {:?}", shown.forks);
    let listed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 1, "only the parent is listed: {listed:?}");
    assert_eq!(git(&["-C", &text(&rig.host_path(channel, PARENT)), "rev-parse", "HEAD"]), head, "the parent keeps its work");
    assert_eq!(common::stored_tabs(&rig.host.state).len(), 1, "the host runs only the parent's agent");
    assert!(common::stored_tabs(&rig.host.state).iter().all(holder_answers), "the parent's agent runs on");

    let box_one = rig.the("box-1");
    assert!(box_one.store().sessions().unwrap().is_empty(), "box-1 keeps no record of the fork");
    assert!(box_one.tabs().is_empty(), "box-1 runs no agent");
    assert!(!box_one.path(channel, FORK).exists(), "box-1 keeps no folder of the fork");
    assert!(staged_at(&box_one.path(channel, "")).is_empty(), "box-1 keeps nothing staged");
    assert!(staged_at(&rig.host_path(channel, "")).is_empty(), "the host keeps nothing staged");
}

/// Cuts a fork of the host's session onto box-1 short at `at`, restarts,
/// and checks the fork is undone; then forking again works.
async fn fork_cut_short_is_undone_at_restart(at: &str) {
    let mut rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let parent = rig.start_on(&channel, &host, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    let head = make_work(&rig.host_path(&channel, PARENT));

    fork_killed_at(&mut rig, &parent, &one, at).await;

    let fork_on_box = rig.the("box-1").path(&channel, FORK);
    let box_staging = rig.the("box-1").path(&channel, "");
    let host_staging = rig.host_path(&channel, "");
    rig.wait_session(&parent, "the fork undone", |s| {
        is_live(s) && !fork_on_box.exists() && staged_at(&box_staging).is_empty() && staged_at(&host_staging).is_empty()
    })
    .await;
    assert_fork_undone(&rig, &channel, &parent, &head).await;

    let mut again = request(&parent, pb::ForkKind::CodeAndContext, &one);
    again.idempotency_key = Some(pb::IdempotencyKey { value: format!("fork-again-{at}") });
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(again)
        .await
        .expect("the fork starts again")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{at}: forking again failed: {:?}", ended.error);
    let Some(pb::operation::Result::Session(id)) = ended.result else {
        panic!("{at}: the fork made no session: {ended:?}");
    };
    rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
    let to = rig.the("box-1").path(&channel, FORK);
    assert!(git(&["-C", &text(&to), "merge-base", "--is-ancestor", &head, "HEAD"]).is_empty(), "{at}: the parent's commit is in the fork");
}

/// FK-8: a host killed as a fork of this Mac's session
/// onto box-1 starts, before anything is staged, undoes the fork at its
/// next start, and forking again works.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_cut_short_as_it_starts_is_undone_at_restart() {
    fork_cut_short_is_undone_at_restart("fork-started").await;
}

/// FK-8 recovery: a host killed once the fork's files are staged and its
/// copy to box-1 has begun undoes the fork at its next start: box-1 takes
/// back whatever landed there, the staged files on this Mac are removed,
/// and the parent is untouched.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_cut_short_as_its_copy_begins_is_undone_at_restart() {
    fork_cut_short_is_undone_at_restart("fork-copying").await;
}

/// The prompts of the conversations a box's Codex or Copilot session holds.
const PROMPTS: [&str; 3] = ["Find the flaky test", "Make it deterministic", "Now delete the old sleep"];

/// A Codex rollout of [`PROMPTS`], each turn finished, for conversation `id`.
fn codex_rollout(id: &str) -> String {
    let mut lines = vec![json!({
        "timestamp": "2026-11-16T00:00:00Z",
        "type": "session_meta",
        "payload": {"id": id, "session_id": id, "timestamp": "2026-11-16T00:00:00Z", "cli_version": "0.130.0"},
    })];
    for (turn, prompt) in (1..).zip(PROMPTS) {
        let at = format!("2026-11-16T00:0{turn}:00Z");
        lines.push(json!({
            "timestamp": at,
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "turn_id": format!("t{turn}"),
                "item": {"type": "UserMessage", "id": format!("u{turn}"), "content": [{"type": "text", "text": prompt}]},
            },
        }));
        lines.push(json!({
            "timestamp": at,
            "type": "event_msg",
            "payload": {"type": "task_complete", "turn_id": format!("t{turn}"), "last_agent_message": "ok"},
        }));
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// A Copilot session state of [`PROMPTS`], each turn finished, for
/// conversation `id` working in `folder`.
fn copilot_events(id: &str, folder: &Path) -> String {
    let start = "00000000-0000-4000-8000-000000000000";
    let mut lines = vec![json!({
        "type": "session.start",
        "data": {
            "sessionId": id,
            "version": 1,
            "producer": "copilot-agent",
            "copilotVersion": "1.0.112-0",
            "startTime": "2026-11-16T00:00:00.000Z",
            "selectedModel": "gpt-4.1",
            "contextTier": null,
            "context": {"cwd": text(folder)},
            "alreadyInUse": false,
            "remoteSteerable": false,
        },
        "id": start,
        "timestamp": "2026-11-16T00:00:00.000Z",
        "parentId": null,
    })];
    let mut parent = start.to_owned();
    for (turn, prompt) in (1..).zip(PROMPTS) {
        let at = format!("2026-11-16T00:0{turn}:00.000Z");
        let records = [
            (
                "user.message",
                json!({
                    "content": prompt,
                    "hasSubmittedTask": false,
                    "transformedContent": prompt,
                    "messageId": format!("m{turn}"),
                    "supportedNativeDocumentMimeTypes": [],
                    "delivery": "immediate",
                    "interactionId": format!("i{turn}"),
                    "turnId": "0",
                    "parentAgentTaskId": null,
                }),
            ),
            ("assistant.turn_start", json!({"turnId": "0", "interactionId": format!("i{turn}"), "agentMode": "interactive"})),
            (
                "assistant.message",
                json!({
                    "messageId": format!("a{turn}"),
                    "originatingMessageId": format!("m{turn}"),
                    "model": "gpt-4.1",
                    "content": "ok",
                    "toolRequests": [],
                    "interactionId": format!("i{turn}"),
                    "turnId": "0",
                    "rte": null,
                    "apiCallId": format!("c{turn}"),
                }),
            ),
            ("assistant.turn_end", json!({"turnId": "0"})),
        ];
        for (step, (kind, data)) in records.into_iter().enumerate() {
            let record = format!("00000000-0000-4000-8000-00000000{turn:02}{step:02}");
            lines.push(json!({"type": kind, "data": data, "id": record, "timestamp": at, "parentId": parent}));
            parent = record;
        }
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// FK-3, D181 across machines: a session of `agent` on box-1, whose
/// conversation holds the three prompts, forked onto box-2 with its
/// conversation before turn 2 resumes there with turn 1 only, cut where
/// its own CLI's format says turn 2 starts. box-1's conversation is as it
/// was.
async fn box_fork_before_turn_2_carries_turn_1_only(agent: &str) {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let mut start = common::start_request(channel.id.as_ref().unwrap(), &one, "Cache warmup");
    start.agent = Some(pb::AgentId { value: agent.to_owned() });
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .start_session(start)
        .await
        .expect("the session starts")
        .into_inner();
    let ended = rig.ended(&started.operation.expect("a start has an operation")).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let parent = started.session.expect("a start names its session");
    rig.wait_session(&parent, "live", is_live).await;
    let box_one = rig.the("box-1");
    let launch = box_one.store().agent(&parent.value).unwrap().expect("box-1 has the agent").launch;
    let folder = box_one.store().session(&parent.value).unwrap().expect("box-1 has the session").folder;
    let talk = match agent {
        remote_rig::CODEX_AGENT => codex_rollout(&launch.conversation_id),
        _ => copilot_events(&launch.conversation_id, &folder),
    };
    std::fs::create_dir_all(launch.conversation_file.parent().unwrap()).unwrap();
    std::fs::write(&launch.conversation_file, &talk).unwrap();

    let mut cut = request(&parent, pb::ForkKind::CodeAndContext, &two);
    cut.before_turn = 2;
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(cut)
        .await
        .expect("the fork starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let Some(pb::operation::Result::Session(id)) = ended.result else {
        panic!("the fork made no session: {ended:?}");
    };
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;

    let theirs = rig.the("box-2").store().agent(&id.value).unwrap().expect("box-2 has the fork's agent").launch;
    let copy = std::fs::read_to_string(&theirs.conversation_file).unwrap();
    assert!(copy.contains(PROMPTS[0]), "the fork keeps turn 1:\n{copy}");
    for prompt in &PROMPTS[1..] {
        assert!(!copy.contains(prompt), "the fork stops before turn 2:\n{copy}");
    }
    assert_eq!(std::fs::read_to_string(&launch.conversation_file).unwrap(), talk);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codex_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only() {
    box_fork_before_turn_2_carries_turn_1_only(remote_rig::CODEX_AGENT).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copilot_fork_from_box_1_onto_box_2_before_turn_2_carries_turn_1_only() {
    box_fork_before_turn_2_carries_turn_1_only(remote_rig::COPILOT_AGENT).await;
}

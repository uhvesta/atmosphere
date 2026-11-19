//! A view a session's agent on a machine marks ready is offered on the host
//! (VW-12, P6). A real host and a real machine behind the rig's fake `ssh`:
//! MarkReady on the machine journals the whole view (D225), the host applies
//! it once (D226) and from then on owns its state. The user lists and answers
//! it on the host, the answer outlives a host restart, a journal entry the
//! host gets twice makes one view and leaves the answer be, and a draft stays
//! on the machine.
//!
//! Views are prepared only while the host is linked: AW-7 refuses view
//! drafts while it is away. Nothing sleeps. Each wait reads the host's views,
//! then reads them again each time the host's Views feed says something
//! changed. A view the agent marks ready after another is a sentinel: the
//! machine's journal is applied in order, so once the host shows it, every
//! entry before it was applied.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Request, Streaming};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";
const TOKEN_HEADER: &str = "humata-session-token";
/// The host's store commits a journaled view and the journal's mark here,
/// before the machine hears it was applied.
const VIEW_APPLIED: &str = "store.machine_journal.apply_view.committed";

fn as_agent<T>(token: &str, message: T) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: value.to_owned(),
    })
}

/// A host and box-1, with a live session on box-1 and its agent's token.
struct Remote {
    rig: Rig,
    session: pb::SessionId,
    token: String,
}

impl Remote {
    async fn start() -> Self {
        let rig = Rig::start_seen(&[BOX], &[REPO]).await;
        let (host, one) = (rig.host_id().await, Rig::box_id(BOX));
        let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
        let session = rig.start_on(&channel, &one, "Ledger").await;
        rig.wait_session(&session, "live", is_live).await;
        let token = rig.agent_token(BOX, &session).await;
        Self { rig, session, token }
    }

    /// The box's ViewService, which the session's agent calls.
    async fn agent_views(&self) -> pb::view_service_client::ViewServiceClient<Channel> {
        pb::view_service_client::ViewServiceClient::new(self.rig.the(BOX).daemon().channel().await)
    }

    /// The host's ViewService, which the user calls.
    async fn host_views(&self) -> pb::view_service_client::ViewServiceClient<Channel> {
        pb::view_service_client::ViewServiceClient::new(self.rig.channel().await)
    }

    /// The agent prepares a walkthrough titled `title` on the machine: a
    /// draft.
    async fn prepare(&self, title: &str) -> pb::PreparedView {
        let request = pb::PrepareViewRequest {
            idempotency_key: key(&format!("prepare-{title}")),
            title: title.to_owned(),
            summary: "Read the ledger first".to_owned(),
            content: Some(pb::prepare_view_request::Content::Review(pb::ReviewWalkthrough {
                steps: vec![pb::review_walkthrough::Step {
                    file: Some(pb::RepoPath {
                        repo: Some(pb::RepoRef {
                            owner: REPO.0.to_owned(),
                            name: REPO.1.to_owned(),
                        }),
                        path: "src/lib.rs".to_owned(),
                    }),
                    lines: Some(pb::LineRange { first: 1, last: 3 }),
                    note: "the ledger starts here".to_owned(),
                }],
            })),
        };
        let draft = self
            .agent_views()
            .await
            .prepare_view(as_agent(&self.token, request))
            .await
            .expect("the agent prepares a view on the machine")
            .into_inner();
        assert_eq!(state(&draft), pb::ViewState::Draft, "a prepared view starts as a draft");
        draft
    }

    /// The agent marks `id` ready on the machine with `key`.
    async fn mark_ready(&self, id: &pb::ViewId, key_value: &str) -> pb::PreparedView {
        self.agent_views()
            .await
            .mark_ready(as_agent(
                &self.token,
                pb::MarkReadyRequest {
                    idempotency_key: key(key_value),
                    id: Some(id.clone()),
                },
            ))
            .await
            .expect("the agent marks its view ready on the machine")
            .into_inner()
    }

    /// The agent prepares a view titled `title` and marks it ready; its id.
    async fn offer(&self, title: &str) -> pb::ViewId {
        let id = self.prepare(title).await.id.expect("a prepared view has an id");
        let ready = self.mark_ready(&id, &format!("ready-{title}")).await;
        assert_eq!(state(&ready), pb::ViewState::Waiting, "a view marked ready waits for the user");
        id
    }

    /// The views the host lists to the user: this session's, or with
    /// `session` None, every session's offered ones.
    async fn listed(&self, session: Option<&pb::SessionId>) -> Vec<pb::PreparedView> {
        self.host_views()
            .await
            .list_views(pb::ListViewsRequest {
                session: session.cloned(),
                ..pb::ListViewsRequest::default()
            })
            .await
            .expect("the host lists views")
            .into_inner()
            .views
    }

    /// The user answers view `id` on the host with `state`, under `key`.
    async fn answer(&self, id: &pb::ViewId, state: pb::ViewState, key_value: &str) -> pb::PreparedView {
        self.host_views()
            .await
            .set_view_state(pb::SetViewStateRequest {
                idempotency_key: key(key_value),
                id: Some(id.clone()),
                state: state as i32,
            })
            .await
            .expect("the user answers the view on the host")
            .into_inner()
    }

    /// The host's Views feed, opened before the change a wait is for.
    async fn views_feed(&self) -> Streaming<pb::Event> {
        pb::event_service_client::EventServiceClient::new(self.rig.channel().await)
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Views as i32],
                ..pb::SubscribeRequest::default()
            })
            .await
            .expect("the host's event feed opens")
            .into_inner()
    }

    /// Waits until the host lists this session's views so that `holds`, and
    /// returns them. They are read once, then read again each time the
    /// host's Views feed says something changed, so a change made before
    /// the first read is not missed.
    async fn until_listed(
        &self,
        feed: &mut Streaming<pb::Event>,
        what: &str,
        holds: impl Fn(&[pb::PreparedView]) -> bool,
    ) -> Vec<pb::PreparedView> {
        let deadline = tokio::time::Instant::now() + LONG;
        loop {
            let views = self.listed(Some(&self.session)).await;
            if holds(&views) {
                return views;
            }
            match tokio::time::timeout_at(deadline, feed.message()).await {
                Ok(Ok(Some(_))) => {}
                Ok(ended) => panic!("the host's Views feed ended ({ended:?}) before it listed {what}"),
                Err(_) => panic!("the host never listed {what} within {LONG:?}: {views:?}"),
            }
        }
    }
}

fn state(view: &pb::PreparedView) -> pb::ViewState {
    pb::ViewState::try_from(view.state).unwrap_or(pb::ViewState::Unspecified)
}

/// The views in `views` with id `id`.
fn with_id<'a>(views: &'a [pb::PreparedView], id: &pb::ViewId) -> Vec<&'a pb::PreparedView> {
    views.iter().filter(|view| view.id.as_ref() == Some(id)).collect()
}

/// Whether `views` holds `id`.
fn lists(views: &[pb::PreparedView], id: &pb::ViewId) -> bool {
    !with_id(views, id).is_empty()
}

/// The one view `id` in `views`, in `expected` state.
fn the_one(views: &[pb::PreparedView], id: &pb::ViewId, expected: pb::ViewState, said: &str) -> pb::PreparedView {
    let found = with_id(views, id);
    assert_eq!(found.len(), 1, "{said}: one copy of {id:?}, in {views:?}");
    assert_eq!(state(found[0]), expected, "{said}");
    found[0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_view_the_remote_agent_marks_ready_waits_on_the_host_with_its_id() {
    let remote = Remote::start().await;
    let mut feed = remote.views_feed().await;

    let id = remote.offer("Ledger walkthrough").await;

    let views = remote.until_listed(&mut feed, "the offered view", |views| lists(views, &id)).await;
    let shown = the_one(&views, &id, pb::ViewState::Waiting, "the host lists the session's offered view, waiting");
    assert_eq!(shown.session.as_ref(), Some(&remote.session), "the view is the remote session's");
    assert_eq!(shown.title, "Ledger walkthrough", "the host has what the agent prepared");
    let every = remote.listed(None).await;
    the_one(&every, &id, pb::ViewState::Waiting, "the host's list of every offered view has it, waiting");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_keeps_an_offered_view_across_its_restart() {
    let mut remote = Remote::start().await;
    let mut feed = remote.views_feed().await;
    let id = remote.offer("Ledger walkthrough").await;
    remote.until_listed(&mut feed, "the offered view", |views| lists(views, &id)).await;

    remote.rig.restart_host(&[]).await;

    let views = remote.listed(Some(&remote.session)).await;
    the_one(&views, &id, pb::ViewState::Waiting, "a restarted host still lists the offered view");
    let every = remote.listed(None).await;
    the_one(&every, &id, pb::ViewState::Waiting, "a restarted host still offers it among every view");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_on_the_host_sticks_across_its_restart_and_a_repeated_key_answers_once() {
    let mut remote = Remote::start().await;
    let mut feed = remote.views_feed().await;
    let id = remote.offer("Ledger walkthrough").await;
    remote.until_listed(&mut feed, "the offered view", |views| lists(views, &id)).await;

    let later = remote.answer(&id, pb::ViewState::Later, "answer-later").await;
    assert_eq!(state(&later), pb::ViewState::Later);
    let again = remote.answer(&id, pb::ViewState::Later, "answer-later").await;
    assert_eq!(again.revision, later.revision, "a repeated key changes nothing");
    let opened = remote.answer(&id, pb::ViewState::Opened, "answer-opened").await;
    assert_eq!(state(&opened), pb::ViewState::Opened);

    remote.rig.restart_host(&[]).await;

    let views = remote.listed(Some(&remote.session)).await;
    let kept = the_one(&views, &id, pb::ViewState::Opened, "the user's last answer outlives a host restart");
    assert_eq!(kept.revision, opened.revision, "nothing changed the view after the answer");
    let repeated = remote.answer(&id, pb::ViewState::Later, "answer-later").await;
    assert_eq!(
        (state(&repeated), repeated.revision),
        (pb::ViewState::Opened, opened.revision),
        "a key answered before the restart answers once: it doesn't set Later again"
    );
    let views = remote.listed(Some(&remote.session)).await;
    the_one(&views, &id, pb::ViewState::Opened, "the repeated key left the answer as it was");
}

/// The host applies the view's journal entry and stops before the machine
/// hears it was, so the machine sends it again when it next links. The user
/// has answered the view by then.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_relinks_and_replays_makes_no_second_copy_and_keeps_the_answer() {
    let mut remote = Remote::start().await;
    remote.rig.restart_host(&[("HUMATA2_STOP_AT", VIEW_APPLIED)]).await;
    let id = remote.offer("Ledger walkthrough").await;
    remote.rig.host_stopped().await;

    // Box-1 stays away through the host's restart, so the user answers
    // before the machine can send the entry again.
    remote.rig.the_mut(BOX).kill();
    remote.rig.restart_host(&[]).await;
    let views = remote.listed(Some(&remote.session)).await;
    the_one(&views, &id, pb::ViewState::Waiting, "the host applied the view before it stopped");
    let later = remote.answer(&id, pb::ViewState::Later, "answer-later").await;
    assert_eq!(state(&later), pb::ViewState::Later);

    let mut feed = remote.views_feed().await;
    remote.rig.the_mut(BOX).relaunch(&[]).await;
    remote.rig.link_back(BOX).await;
    remote.rig.wait_session(&remote.session, "live again", is_live).await;
    remote.token = remote.rig.agent_token(BOX, &remote.session).await;
    let sentinel = remote.offer("Sentinel").await;
    let views = remote
        .until_listed(&mut feed, "the view offered after the replay", |views| lists(views, &sentinel))
        .await;

    let kept = the_one(&views, &id, pb::ViewState::Later, "the replay made no second copy and kept the answer");
    assert_eq!(kept.revision, later.revision, "the replay changed nothing on the view");
    let every = remote.listed(None).await;
    the_one(&every, &id, pb::ViewState::Later, "every view the host offers has one copy of it, as answered");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_on_the_machine_never_shows_on_the_host() {
    let remote = Remote::start().await;
    let mut feed = remote.views_feed().await;

    let draft = remote.prepare("Still drafting").await.id.expect("a prepared view has an id");
    let sentinel = remote.offer("Sentinel").await;
    let views = remote
        .until_listed(&mut feed, "the view offered after the draft", |views| lists(views, &sentinel))
        .await;

    assert!(!lists(&views, &draft), "the host doesn't list the session's draft: {views:?}");
    let every = remote.listed(None).await;
    assert!(!lists(&every, &draft), "the host doesn't offer the draft among every view: {every:?}");
}

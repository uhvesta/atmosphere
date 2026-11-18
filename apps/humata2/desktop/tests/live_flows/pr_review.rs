//! SPEC flow 26.17 (review a pull request) on a real `humatad2` and the
//! daemon tests' fake GitHub (`daemon/humatad/tests/common/pr_github.rs`):
//! bare remotes holding acme/api#482 and acme/ledger#91, and a fake `gh`
//! that records every review posted. The agent of the review is an MCP
//! client with the session's token, as a real agent's tools would call the
//! daemon. The app is headless and drives the dialog, the follower, Submit
//! review and the new-commits notices through its own actions.
//!
//! Most flows run the review on This Mac, the host the app talks to. Step
//! 2's devbox-1 is the last test: the box rig has the same remotes (the
//! clones are made from them on the box) and the fake `gh` is the host's
//! alone, because `gh` and the fetch of a pull request's head run on the
//! host and a box never has a token. It is ignored until the daemon starts a
//! review on another machine; see that test.

use std::sync::Arc;

use humata2_app::{AppAction, Route};
use humata2_clients::comments::{
    Anchor, Author, CommentId, CommentsClient, Destination, Side, Status as CommentStatus, Which,
};
use humata2_clients::model::{Agent, SessionId};
use humata2_clients::pull_requests::{PullRequestInfo, PullRequestRef, PullRequestState, Verdict};
use humata2_clients::review_chat::Place;
use humata2_grpc::humata::v1 as pb;
use humata2_pr_review::dialog::{DialogAction, Fetch, Start};
use humata2_pr_review::follower::FollowerAction;
use humata2_pr_review::heads::HeadsAction;
use humata2_pr_review::submit::{Outcome, Step, SubmitAction};
use humata2_remote_rig::common::{get_session, wait_running};
use humata2_remote_rig::pr_github::{API, CONTEXT, Fixture, LEDGER, change_of, clone_head, ok, repo_of};
use humata2_review::ReviewAction;
use serde_json::json;
use tokio::runtime::Runtime;

use super::leftovers::case_watches;
use super::remote::RemoteWorld;
use super::world::{World, live_clients};
use super::{App, runtime};

/// The daemon's own check of the heads is left to the tests, so each says
/// when GitHub is asked.
const NEVER: &str = "3600000";
const API_LINK: &str = "https://github.com/acme/api/pull/482";
const LEDGER_LINK: &str = "acme/ledger#91";

const NOTE: &str = "This is where the rounding changed";
const EDITED: &str = "Round half-even here, once, at the end";
const MINE: &str = "Needs a test";
const LEDGER_DRAFT: &str = "Name the refunds";
const API_SUMMARY: &str = "Round once, at the end";
const LEDGER_SUMMARY: &str = "Only the ledger line to check";

/// Two commits the author pushes after the review began. They leave the
/// lines the drafts are on alone.
const TWO_NEW_COMMITS: [(&str, &[(&str, usize, &str)]); 2] = [
    ("Log refunds", &[("refund.rs", 6, "    log(signed); // refunds")]),
    ("Test rounding", &[("rounding_test.rs", 0, "#[test] fn halves() {}")]),
];

fn api_ref() -> PullRequestRef {
    PullRequestRef::new("acme/api", 482)
}

fn ledger_ref() -> PullRequestRef {
    PullRequestRef::new("acme/ledger", 91)
}

fn send(app: &mut App, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// A review of both pull requests, started through the dialog.
struct Review<'r> {
    // The app goes first: it is dropped before the daemon it talks to.
    app: App<'r>,
    fixture: Fixture,
    runtime: &'r Runtime,
    id: SessionId,
    /// The daemon's own view of the session, once its agent ran.
    session: pb::Session,
    comments: Arc<dyn CommentsClient>,
    /// What the dialog showed for each pull request before Start.
    found: Vec<PullRequestInfo>,
}

/// The repository of `path` in the session's diff, as the app holds it.
fn diff_repo(app: &App, id: &SessionId, path: &str) -> Option<String> {
    app.ui.read(|state| {
        state
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == id)
            .find(|diff| diff.files.iter().any(|file| file.path == path))
            .map(|diff| diff.repo.clone())
    })
}

/// The file the Diff panel shows for the session.
fn selected(app: &App, id: &SessionId) -> Option<(String, String)> {
    app.ui.read(|state| state.review.saved.selected.get(id).cloned())
}

fn place(repo: &str, path: &str) -> Place {
    Place {
        repo: repo.to_owned(),
        path: path.to_owned(),
        visible: None,
        selection: None,
        side: Side::New,
    }
}

/// What stops the dialog from getting to a session, said where the test
/// can show it.
fn dialog_problem(app: &App) -> Option<String> {
    app.ui.read(|state| {
        let dialog = &state.pr_dialog;
        for candidate in dialog.main.iter().chain(dialog.also.iter()) {
            if let Fetch::Failed(error) = &candidate.fetch {
                return Some(format!("{} could not be read: {error:?}", candidate.pr));
            }
        }
        match &dialog.start {
            Start::Failed(error) => Some(format!("Start failed: {error:?}")),
            _ => None,
        }
    })
}

/// The person opens `path` themselves, as the Diff panel would.
fn person_opens(app: &mut App, id: &SessionId, repo: &str, path: &str) {
    let (repo, path) = (repo.to_owned(), path.to_owned());
    app.wait_until("the person to open a file", |app| {
        send(
            app,
            AppAction::Review(ReviewAction::SelectFile {
                repo: repo.clone(),
                path: path.clone(),
            }),
        );
        selected(app, id) == Some((repo.clone(), path.clone()))
    });
    send(app, AppAction::PrFollower(FollowerAction::UserWentTo(place(&repo, &path))));
}

/// 26.17 steps 1 to 3: paste acme/api#482, add acme/ledger#91 under Also
/// review, give some context and Start. Returns once the session is open,
/// its agent runs and the app follows the review.
fn started(runtime: &Runtime) -> Review<'_> {
    let fixture = runtime.block_on(Fixture::with_env(&[("HUMATA2_PR_POLL_MS", NEVER)]));
    // The fixture is no World: its daemon's folder joins the case's watch
    // here, so a hang past the limit ends it and fails this case.
    case_watches(&[fixture.daemon.dir.path()]);
    let clients = runtime
        .block_on(live_clients(fixture.daemon.socket.clone(), false))
        .expect("the app connects to the daemon");
    let comments = clients.comments.clone();
    let mac = runtime
        .block_on(World::this_mac(&clients))
        .expect("the workspace has This Mac");
    let mut app = App::open(runtime, clients);
    app.wait_until("the workspace to load and the heads feed to be followed", |app| {
        app.ui
            .read(|state| !state.workspace.machines.is_empty() && state.pr_heads.watching)
    });

    send(
        &mut app,
        AppAction::PrDialog(DialogAction::Open {
            machine: Some(mac),
            agent: Agent::Claude,
        }),
    );
    send(&mut app, AppAction::PrDialog(DialogAction::Paste(API_LINK.to_owned())));
    send(&mut app, AppAction::PrDialog(DialogAction::AddAlso(LEDGER_LINK.to_owned())));
    send(&mut app, AppAction::PrDialog(DialogAction::SetContext(CONTEXT.to_owned())));
    app.wait_until("both pull requests to be read", |app| {
        if let Some(problem) = dialog_problem(app) {
            panic!("{problem}");
        }
        app.ui.read(|state| state.pr_dialog.blocker().is_none())
    });
    let found: Vec<PullRequestInfo> = app.ui.read(|state| {
        let dialog = &state.pr_dialog;
        dialog
            .main
            .iter()
            .chain(dialog.also.iter())
            .filter_map(|candidate| match &candidate.fetch {
                Fetch::Found(info) => Some(info.clone()),
                _ => None,
            })
            .collect()
    });

    send(&mut app, AppAction::PrDialog(DialogAction::Start));
    let mut opened = None;
    app.wait_until("the review's session to open", |app| {
        if let Some(problem) = dialog_problem(app) {
            panic!("{problem}");
        }
        opened = app.ui.read(|state| match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        });
        opened.is_some()
    });
    let id = opened.expect("the wait ended on an open session");
    app.wait_until("the session to show as a review", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                .is_some_and(|session| session.pull_request_review)
        })
    });
    app.wait_until("the follower to follow the review and its diff to show", |app| {
        let following = app
            .ui
            .read(|state| state.pr_follower.session.as_ref() == Some(&id));
        following && diff_repo(app, &id, "refund.rs").is_some() && diff_repo(app, &id, "ledger.rs").is_some()
    });

    let session = runtime.block_on(wait_running(
        &fixture.daemon,
        &pb::SessionId {
            value: id.as_str().to_owned(),
        },
    ));
    Review {
        app,
        fixture,
        runtime,
        id,
        session,
        comments,
        found,
    }
}

fn comments_of(review: &Review<'_>) -> Vec<humata2_clients::comments::Comment> {
    review
        .runtime
        .block_on(review.comments.list(&review.id, Which::Open))
        .expect("the daemon lists the review's comments")
        .comments
}

/// 26.17 steps 1 to 3: the dialog reads both pull requests, Start opens the
/// session, the clones have the heads and the agent is told the context.
#[test]
fn starting_a_review_of_two_pull_requests_clones_both_heads_and_tells_the_agent() {
    let runtime = runtime();
    let review = started(&runtime);
    let (api, ledger) = (review.fixture.prs[API].clone(), review.fixture.prs[LEDGER].clone());

    assert_eq!(review.found.len(), 2, "both pull requests were read: {:?}", review.found);
    let (read_api, read_ledger) = (&review.found[0], &review.found[1]);
    assert_eq!(read_api.pr, api_ref());
    assert_eq!(read_api.title, api.title);
    assert_eq!(read_api.author, api.author);
    assert_eq!(read_api.head_sha, api.head());
    assert_eq!(read_api.state, PullRequestState::Open);
    assert_eq!(read_ledger.pr, ledger_ref());
    assert_eq!(read_ledger.title, ledger.title);

    review.app.ui.read(|state| {
        let session = state.workspace.session(&review.id).expect("the app lists the session");
        assert!(session.pull_request_review);
        assert_eq!(state.route, Route::Session(review.id.clone()));
        assert!(!state.pr_dialog.open, "Start closes the dialog");
    });

    let pb_id = review.session.id.clone().expect("the session has an id");
    let now = review.runtime.block_on(get_session(&review.fixture.daemon, &pb_id));
    assert_eq!(clone_head(repo_of(&now, &api)), api.head());
    assert_eq!(clone_head(repo_of(&now, &ledger)), ledger.head());
    assert_eq!(repo_of(&now, &api).branch, api.head_branch);

    let said = review.runtime.block_on(review.fixture.told(
        &review.session,
        &[CONTEXT, &api.url(), &ledger.url()],
    ));
    assert!(said.contains(CONTEXT));
}

/// 26.17 step 4's view moves: the agent focuses a file and leaves a note
/// over MCP, the app shows them once the person has been still, and one
/// Undo puts the view back.
#[test]
fn the_agents_focus_and_note_reach_the_view_when_the_person_is_still_and_undo_puts_it_back() {
    let runtime = runtime();
    let mut review = started(&runtime);
    let id = review.id.clone();
    let api = review.fixture.prs[API].clone();
    let repo = diff_repo(&review.app, &id, "refund.rs").expect("the diff has refund.rs");
    let rounding = (repo.clone(), "rounding.rs".to_owned());
    let refund = (repo.clone(), "refund.rs".to_owned());
    let mut agent = runtime.block_on(review.fixture.agent(&review.session));

    // The person is reading rounding.rs. Touching the view just before the
    // agent moves it makes the follower wait for them to be still.
    person_opens(&mut review.app, &id, &repo, "rounding.rs");
    send(
        &mut review.app,
        AppAction::PrFollower(FollowerAction::Interacted(
            humata2_pr_review::follower::Interaction::Scroll,
        )),
    );

    let reply = runtime.block_on(agent.call(
        "review_focus",
        json!({ "repo": "acme/api", "path": "refund.rs", "lines": { "first": 3, "last": 3 } }),
    ));
    ok(&reply);
    review
        .app
        .wait_until("the follower to show refund.rs", |app| selected(app, &id) == Some(refund.clone()));
    let (undo_id, can_undo) = review.app.ui.read(|state| {
        let applied = state
            .pr_follower
            .applied
            .iter()
            .find(|entry| entry.change.tool == "review_focus")
            .expect("the focus is among the applied changes");
        (applied.change.undo_id.clone(), applied.can_undo())
    });
    assert!(can_undo, "an agent's focus can be undone");
    review.app.ui.read(|state| {
        let shown = state.pr_follower.shown.as_ref().expect("the follower shows a place");
        assert_eq!((shown.repo.as_str(), shown.path.as_str()), ("acme/api", "refund.rs"));
        assert!(state.pr_follower.back.is_some(), "Back returns to what the person read");
    });

    let reply = runtime.block_on(agent.call(
        "review_note",
        json!({
            "repo": "acme/api",
            "path": "refund.rs",
            "lines": { "first": 3, "last": 3 },
            "quoted": api.line("refund.rs", 3),
            "text": NOTE,
        }),
    ));
    let note = ok(&reply)["note_id"].as_str().expect("a note id").to_owned();
    review.app.wait_until("the note to show", |app| {
        app.ui
            .read(|state| state.pr_follower.notes.iter().any(|held| held.id.as_str() == note))
    });

    send(&mut review.app, AppAction::PrFollower(FollowerAction::Undo(undo_id.clone())));
    review.app.wait_until("Undo to put the view back", |app| {
        let undone = app.ui.read(|state| {
            state
                .pr_follower
                .applied
                .iter()
                .any(|entry| entry.change.undo_id == undo_id && entry.undone)
        });
        undone && selected(app, &id) == Some(rounding.clone())
    });
    review.app.ui.read(|state| {
        let shown = state.pr_follower.shown.as_ref().expect("the view is somewhere");
        assert_eq!((shown.repo.as_str(), shown.path.as_str()), ("acme/api", "rounding.rs"));
        assert!(
            state.pr_follower.notes.iter().any(|held| held.id.as_str() == note),
            "undoing the focus leaves the note"
        );
    });
}

/// 26.17 steps 5 and 7: a note becomes a draft for the pull request, the
/// person's own draft goes through the live comments client, and Submit
/// review posts one review per pull request with the verdicts and the
/// drafts' bodies.
#[test]
fn a_note_made_a_draft_and_the_persons_own_draft_post_in_one_review_per_pull_request() {
    let runtime = runtime();
    let mut review = started(&runtime);
    let id = review.id.clone();
    let (api, ledger) = (review.fixture.prs[API].clone(), review.fixture.prs[LEDGER].clone());
    let mut agent = runtime.block_on(review.fixture.agent(&review.session));

    let reply = runtime.block_on(agent.call(
        "review_note",
        json!({
            "repo": "acme/api",
            "path": "refund.rs",
            "lines": { "first": 3, "last": 3 },
            "quoted": api.line("refund.rs", 3),
            "text": NOTE,
        }),
    ));
    let note = CommentId::new(ok(&reply)["note_id"].as_str().expect("a note id"));
    let for_api = Destination::PullRequest {
        repo: "acme/api".to_owned(),
        number: 482,
    };

    // D284: the note becomes a draft in place, its author kept.
    let made = runtime
        .block_on(review.comments.make_note_a_draft(&note, Some(("acme/api", 482))))
        .expect("the daemon makes the note a draft");
    assert_eq!(made.id, note, "the same comment, not a copy");
    assert_eq!(made.status, CommentStatus::Draft);
    assert_eq!(made.destination, for_api);
    assert!(matches!(made.author, Author::Agent(_)), "{:?}", made.author);

    // The person's own draft succeeds over the live comments client.
    let mine = runtime
        .block_on(review.comments.create_for_pull_request(
            &id,
            Anchor {
                repo: "acme/api".to_owned(),
                path: "rounding.rs".to_owned(),
                side: Side::New,
                first: 1,
                last: 1,
                quoted: vec![api.line("rounding.rs", 1)],
                base: None,
                detached: false,
                outdated: false,
            },
            MINE,
            "acme/api",
            482,
            "pr-review-e2e-mine",
        ))
        .expect("the daemon takes the person's draft");
    assert_eq!(mine.status, CommentStatus::Draft);
    assert_eq!(mine.destination, for_api);
    assert_eq!(mine.author, Author::User);
    assert_ne!(mine.id, made.id);

    let ledger_draft = CommentId::new(runtime.block_on(agent.draft(&ledger, "ledger.rs", 2, 2, LEDGER_DRAFT)));
    let listed = comments_of(&review);
    for wanted in [&made.id, &mine.id, &ledger_draft] {
        assert!(
            listed.iter().any(|comment| &comment.id == wanted && comment.status == CommentStatus::Draft),
            "{wanted} is a draft: {listed:#?}"
        );
    }
    assert_eq!(review.fixture.gh.post_count(), 0, "drafting posts nothing");

    // Submit review plans, the person edits a draft, picks verdicts and posts.
    send(&mut review.app, AppAction::PrSubmit(SubmitAction::Open(id.clone())));
    review.app.wait_until("Submit review to plan", |app| {
        app.ui
            .read(|state| state.pr_submit.step == Step::Reviewing && state.pr_submit.reviews.len() == 2)
    });
    review.app.ui.read(|state| {
        let planned_for = |pr: &PullRequestRef| -> Vec<CommentId> {
            state
                .pr_submit
                .reviews
                .iter()
                .filter(|held| &held.pr == pr)
                .flat_map(|held| held.drafts.iter().map(|draft| draft.id.clone()))
                .collect()
        };
        let mut api_drafts = planned_for(&api_ref());
        api_drafts.sort();
        let mut both = vec![made.id.clone(), mine.id.clone()];
        both.sort();
        assert_eq!(api_drafts, both, "each pull request's own drafts are planned for it");
        assert_eq!(planned_for(&ledger_ref()), vec![ledger_draft.clone()]);
    });

    send(
        &mut review.app,
        AppAction::PrSubmit(SubmitAction::EditDraft {
            id: made.id.clone(),
            text: EDITED.to_owned(),
        }),
    );
    review.app.wait_until("the edit to reach the draft", |app| {
        app.ui.read(|state| {
            state
                .pr_submit
                .reviews
                .iter()
                .flat_map(|held| &held.drafts)
                .any(|draft| draft.id == made.id && draft.text == EDITED)
        })
    });
    for action in [
        SubmitAction::SetVerdict {
            pr: api_ref(),
            verdict: Verdict::RequestChanges,
        },
        SubmitAction::SetSummary {
            pr: api_ref(),
            summary: API_SUMMARY.to_owned(),
        },
        SubmitAction::SetVerdict {
            pr: ledger_ref(),
            verdict: Verdict::Comment,
        },
        SubmitAction::SetSummary {
            pr: ledger_ref(),
            summary: LEDGER_SUMMARY.to_owned(),
        },
    ] {
        send(&mut review.app, AppAction::PrSubmit(action));
    }
    assert!(review.app.ui.read(|state| state.pr_submit.can_submit()));

    send(&mut review.app, AppAction::PrSubmit(SubmitAction::Submit));
    review.app.wait_until("both reviews to post and Submit review to close", |app| {
        app.ui.read(|state| {
            for held in &state.pr_submit.reviews {
                if let Some(Outcome::Failed(error)) = &held.outcome {
                    panic!("{} did not post: {error:?}", held.pr);
                }
            }
            assert!(state.pr_submit.error.is_none(), "{:?}", state.pr_submit.error);
            state.pr_submit.step == Step::Closed
        })
    });

    let gh = &review.fixture.gh;
    assert_eq!(gh.post_count(), 2, "one review per pull request:\n{}", gh.log());
    let posted = gh.posted(&api);
    assert_eq!(posted.len(), 1, "{posted:?}");
    let body = &posted[0].1;
    assert_eq!(body["event"], "REQUEST_CHANGES", "{body}");
    assert_eq!(body["body"], API_SUMMARY, "{body}");
    let bodies: Vec<&serde_json::Value> = body["comments"]
        .as_array()
        .expect("the review carries its comments")
        .iter()
        .map(|comment| &comment["body"])
        .collect();
    assert_eq!(bodies.len(), 2, "{body}");
    assert!(bodies.contains(&&json!(EDITED)), "the edited note posts as the person left it: {body}");
    assert!(bodies.contains(&&json!(MINE)), "{body}");
    let posted = gh.posted(&ledger);
    assert_eq!(posted.len(), 1, "{posted:?}");
    let body = &posted[0].1;
    assert_eq!(body["event"], "COMMENT", "{body}");
    assert_eq!(body["body"], LEDGER_SUMMARY, "{body}");
    assert_eq!(body["comments"].as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(body["comments"][0]["body"], LEDGER_DRAFT, "{body}");

    let after = comments_of(&review);
    for each in [&made.id, &mine.id, &ledger_draft] {
        let held = after.iter().find(|comment| &comment.id == each).expect("the comment is kept");
        assert_eq!(held.status, CommentStatus::Posted, "{held:?}");
        assert!(held.posted_url.is_some(), "{held:?}");
    }
}

/// 26.17 steps 6 and 8: two new commits arrive on the feed and Update the
/// clone brings them in and tells the agent; later the pull request merges
/// and the app offers Archive, which starts the session's archive.
#[test]
fn new_commits_update_the_clone_and_a_merged_pull_request_offers_archive() {
    let runtime = runtime();
    let mut review = started(&runtime);
    let id = review.id.clone();
    let (api, ledger) = (review.fixture.prs[API].clone(), review.fixture.prs[LEDGER].clone());
    let pb_id = review.session.id.clone().expect("the session has an id");
    let before = runtime.block_on(get_session(&review.fixture.daemon, &pb_id));
    let reviewed = clone_head(repo_of(&before, &api));

    review.fixture.move_head(API, &TWO_NEW_COMMITS);
    let api = review.fixture.prs[API].clone();
    assert_ne!(api.head(), reviewed);
    let heads = runtime.block_on(review.fixture.check_heads(&review.session));
    assert_eq!(
        change_of(&heads, &api).map(|change| change.new_commits),
        Some(2),
        "{heads:?}"
    );

    review.app.wait_until("the app to hear of two new commits", |app| {
        app.ui.read(|state| {
            state
                .pr_heads
                .notice(&id)
                .iter()
                .any(|(pr, moved)| **pr == api_ref() && moved.new_commits == 2)
        })
    });
    review.app.ui.read(|state| {
        assert!(
            state.pr_heads.notice(&id).iter().all(|(pr, _)| **pr != ledger_ref()),
            "the ledger's head did not move"
        );
    });
    let still = runtime.block_on(get_session(&review.fixture.daemon, &pb_id));
    assert_eq!(clone_head(repo_of(&still, &api)), reviewed, "a notice never moves the clone");

    send(&mut review.app, AppAction::PrHeads(HeadsAction::UpdateClone(id.clone())));
    review.app.wait_until("the clone to be updated", |app| {
        app.ui
            .read(|state| state.pr_heads.notice(&id).is_empty() && state.pr_heads.updating.is_empty())
    });
    review.app.ui.read(|state| {
        assert!(state.pr_heads.error.is_none(), "{:?}", state.pr_heads.error);
    });
    let updated = runtime.block_on(get_session(&review.fixture.daemon, &pb_id));
    assert_eq!(clone_head(repo_of(&updated, &api)), api.head(), "the clone has the new head");
    assert_eq!(clone_head(repo_of(&updated, &ledger)), ledger.head());
    runtime.block_on(review.fixture.told(&review.session, &["rounding_test.rs", "refund.rs"]));

    // Days later #482 merges.
    review.fixture.prs[API].state = "MERGED";
    review.fixture.gh.publish(&review.fixture.prs);
    runtime.block_on(review.fixture.check_heads(&review.session));
    review.app.wait_until("the app to offer Archive for the merged pull request", |app| {
        app.ui
            .read(|state| state.pr_heads.archivable(&id).iter().any(|pr| **pr == api_ref()))
    });
    review.app.ui.read(|state| {
        assert!(
            state.pr_heads.archivable(&id).iter().all(|pr| **pr != ledger_ref()),
            "only the merged pull request offers it"
        );
    });

    send(&mut review.app, AppAction::PrHeads(HeadsAction::Archive(id.clone())));
    // The session's own archive: its dialog when the clones hold unpushed
    // work, else the archive itself.
    review.app.wait_until("the session's archive to start", |app| {
        app.ui.read(|state| {
            state
                .session
                .archiving
                .as_ref()
                .is_some_and(|confirm| confirm.session == id)
                || state.workspace.session(&id).is_none_or(|session| session.archived)
        })
    });
}

/// 26.17 step 2 with devbox-1 picked: the review's session runs on the box,
/// the box's clones hold each pull request's head, and a draft the person
/// makes in the app posts through the host's fake `gh`, which never gave the
/// box a token.
///
/// The host asks GitHub through its own `gh` and keeps the review's records
/// and the PR reviews channel; the start is relayed to the box with each pull
/// request's branch and head, and the box clones them from its remote with no
/// token. The heads feed, the diff and Submit review's line checks read the
/// box's clones through the link.
#[test]
fn a_review_started_on_devbox_1_runs_there_and_posts_through_the_hosts_gh() {
    const BOX: &str = "devbox-1";
    let runtime = runtime();
    let (world, github) = runtime.block_on(RemoteWorld::start_with_github(&[BOX], &[("HUMATA2_PR_POLL_MS", NEVER)]));
    let (api, ledger) = (github.prs[API].clone(), github.prs[LEDGER].clone());
    let clients = runtime
        .block_on(world.clients())
        .expect("the app connects to the host");
    let comments = clients.comments.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the workspace to load with devbox-1 and the heads feed to be followed", |app| {
        app.ui.read(|state| {
            state.workspace.machines.iter().any(|machine| machine.id.as_str() == BOX) && state.pr_heads.watching
        })
    });
    let devbox = app.ui.read(|state| {
        state
            .workspace
            .machines
            .iter()
            .find(|machine| machine.id.as_str() == BOX)
            .map(|machine| machine.id.clone())
    });
    let devbox = devbox.expect("the workspace lists devbox-1");

    send(
        &mut app,
        AppAction::PrDialog(DialogAction::Open {
            machine: None,
            agent: Agent::Claude,
        }),
    );
    send(&mut app, AppAction::PrDialog(DialogAction::PickMachine(devbox.clone())));
    send(&mut app, AppAction::PrDialog(DialogAction::Paste(API_LINK.to_owned())));
    send(&mut app, AppAction::PrDialog(DialogAction::AddAlso(LEDGER_LINK.to_owned())));
    send(&mut app, AppAction::PrDialog(DialogAction::SetContext(CONTEXT.to_owned())));
    app.wait_until("both pull requests to be read", |app| {
        if let Some(problem) = dialog_problem(app) {
            panic!("{problem}");
        }
        app.ui.read(|state| state.pr_dialog.blocker().is_none())
    });
    send(&mut app, AppAction::PrDialog(DialogAction::Start));
    let mut opened = None;
    app.wait_until("the review's session to open", |app| {
        if let Some(problem) = dialog_problem(app) {
            panic!("{problem}");
        }
        opened = app.ui.read(|state| match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        });
        opened.is_some()
    });
    let id = opened.expect("the wait ended on an open session");
    let pb_id = pb::SessionId {
        value: id.as_str().to_owned(),
    };
    app.wait_until("the session to show as a review on devbox-1", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                .is_some_and(|session| session.pull_request_review && session.machine == devbox)
        })
    });
    app.wait_until("the follower to follow the review and its diff to show", |app| {
        let following = app
            .ui
            .read(|state| state.pr_follower.session.as_ref() == Some(&id));
        following && diff_repo(app, &id, "refund.rs").is_some() && diff_repo(app, &id, "ledger.rs").is_some()
    });

    // The box's own daemon runs the session and its clones hold each head.
    let on_the_box = runtime.block_on(wait_running(world.box_daemon(BOX), &pb_id));
    assert_eq!(clone_head(repo_of(&on_the_box, &api)), api.head());
    assert_eq!(clone_head(repo_of(&on_the_box, &ledger)), ledger.head());
    assert_eq!(repo_of(&on_the_box, &api).branch, api.head_branch);

    // The person's draft posts through the host's `gh`.
    let mine = runtime
        .block_on(comments.create_for_pull_request(
            &id,
            Anchor {
                repo: "acme/api".to_owned(),
                path: "rounding.rs".to_owned(),
                side: Side::New,
                first: 1,
                last: 1,
                quoted: vec![api.line("rounding.rs", 1)],
                base: None,
                detached: false,
                outdated: false,
            },
            MINE,
            "acme/api",
            482,
            "pr-review-devbox-mine",
        ))
        .expect("the host takes the person's draft");
    assert_eq!(mine.status, CommentStatus::Draft);

    send(&mut app, AppAction::PrSubmit(SubmitAction::Open(id.clone())));
    app.wait_until("Submit review to plan", |app| {
        app.ui.read(|state| state.pr_submit.step == Step::Reviewing && !state.pr_submit.reviews.is_empty())
    });
    for action in [
        SubmitAction::SetVerdict {
            pr: api_ref(),
            verdict: Verdict::Comment,
        },
        SubmitAction::SetSummary {
            pr: api_ref(),
            summary: API_SUMMARY.to_owned(),
        },
    ] {
        send(&mut app, AppAction::PrSubmit(action));
    }
    send(&mut app, AppAction::PrSubmit(SubmitAction::Submit));
    app.wait_until("the review to post and Submit review to close", |app| {
        app.ui.read(|state| {
            for held in &state.pr_submit.reviews {
                if let Some(Outcome::Failed(error)) = &held.outcome {
                    panic!("{} did not post: {error:?}", held.pr);
                }
            }
            state.pr_submit.step == Step::Closed
        })
    });

    let gh = &github.gh;
    let posted = gh.posted(&api);
    assert_eq!(posted.len(), 1, "the host's gh posted the review:\n{}", gh.log());
    assert_eq!(posted[0].1["body"], API_SUMMARY);
    // `gh` ran on the host alone and was never handed a token, and no file
    // of the host's folder (the shared remotes too) holds the one it knows.
    gh.assert_token_stayed_with_gh(&world.host_root());
}

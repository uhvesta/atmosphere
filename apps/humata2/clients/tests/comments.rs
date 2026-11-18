//! The scripted comments client keeps comments the way the daemon does
//! (SM-7, SM-9 to SM-12, SM-15 to SM-21, DK-9).

use humata2_clients::comments::{
    Anchor, Author, Call, Comment, CommentEvent, CommentId, CommentsClient, Destination,
    FakeComments, Side, Status, Typing, Which,
};
use humata2_clients::model::SessionId;
use humata2_clients::DaemonError;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn session() -> SessionId {
    SessionId::new("s-1")
}

fn anchor(repo: &str, path: &str, first: u32, last: u32) -> Anchor {
    Anchor {
        repo: repo.to_owned(),
        path: path.to_owned(),
        side: Side::New,
        first,
        last,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    }
}

fn draft(fake: &FakeComments, key: &str, text: &str) -> Comment {
    draft_at(fake, key, anchor("api", "src/lib.rs", 7, 7), text)
}

fn draft_at(fake: &FakeComments, key: &str, anchor: Anchor, text: &str) -> Comment {
    match block_on(fake.create(&session(), anchor, text, key)) {
        Ok(comment) => comment,
        Err(error) => panic!("create works: {error}"),
    }
}

fn sent(fake: &FakeComments, ids: &[&Comment]) -> humata2_clients::comments::Sent {
    let ids: Vec<CommentId> = ids.iter().map(|c| c.id.clone()).collect();
    match block_on(fake.send(&session(), &ids)) {
        Ok(sent) => sent,
        Err(error) => panic!("send works: {error}"),
    }
}

fn listing(fake: &FakeComments, which: Which) -> humata2_clients::comments::Listing {
    match block_on(fake.list(&session(), which)) {
        Ok(listing) => listing,
        Err(error) => panic!("list works: {error}"),
    }
}

fn current(fake: &FakeComments, id: &CommentId) -> Comment {
    match fake.comments(&session()).into_iter().find(|c| &c.id == id) {
        Some(comment) => comment,
        None => panic!("{id} is held"),
    }
}

fn is_refused<T: std::fmt::Debug>(result: Result<T, DaemonError>) -> bool {
    matches!(result, Err(DaemonError::Refused { .. }))
}

#[test]
fn create_makes_a_draft_and_counts_it() {
    let fake = FakeComments::new();
    let comment = draft(&fake, "k1", "rename this");
    assert_eq!(comment.status, Status::Draft);
    assert_eq!(comment.revision, 1);
    assert_eq!(comment.id.as_str(), "s-1/c-1");
    let list = listing(&fake, Which::Open);
    assert_eq!(list.comments.len(), 1);
    assert_eq!(list.draft_count, 1);
}

#[test]
fn the_same_create_key_makes_one_comment() {
    let fake = FakeComments::new();
    let first = draft(&fake, "k1", "rename this");
    let second = draft(&fake, "k1", "rename this");
    assert_eq!(first.id, second.id);
    assert_eq!(fake.comments(&session()).len(), 1);
}

#[test]
fn a_blank_comment_is_refused() {
    let fake = FakeComments::new();
    let result = block_on(fake.create(&session(), anchor("api", "a.rs", 1, 1), "  ", "k"));
    assert!(is_refused(result));
}

#[test]
fn edit_refuses_a_stale_revision_and_a_comment_that_was_sent() {
    let fake = FakeComments::new();
    let comment = draft(&fake, "k1", "one");
    let Ok(edited) = block_on(fake.edit(&comment.id, "two", 1)) else {
        panic!("a current revision is accepted");
    };
    assert_eq!((edited.text.as_str(), edited.revision), ("two", 2));
    assert!(matches!(
        block_on(fake.edit(&comment.id, "three", 1)),
        Err(DaemonError::Conflict { .. })
    ));
    sent(&fake, &[&edited]);
    let now = current(&fake, &comment.id);
    assert!(is_refused(block_on(fake.edit(&comment.id, "four", now.revision))));
}

#[test]
fn send_types_one_message_for_a_batch_without_the_repo_when_there_is_one() {
    let fake = FakeComments::new();
    let a = draft_at(&fake, "a", anchor("api", "src/a.rs", 3, 3), "first");
    let b = draft_at(&fake, "b", anchor("api", "src/b.rs", 7, 9), "second  \n");
    let result = sent(&fake, &[&a, &b]);
    assert!(!result.queued);
    assert!(result.comments.iter().all(|c| c.status == Status::Sent));
    assert_eq!(
        fake.typed(&session()),
        vec!["[comment on src/a.rs:3]\nfirst\n\n[comment on src/b.rs:7-9]\nsecond".to_owned()]
    );
}

#[test]
fn send_leads_the_path_with_the_repo_when_there_are_two() {
    let fake = FakeComments::new();
    fake.set_repos(&session(), &["api", "web"]);
    let a = draft_at(&fake, "a", anchor("web", "src/a.rs", 3, 3), "first");
    sent(&fake, &[&a]);
    assert_eq!(fake.typed(&session()), vec!["[comment on web/src/a.rs:3]\nfirst".to_owned()]);
}

#[test]
fn a_comment_on_a_session_folder_file_names_just_its_path() {
    let fake = FakeComments::new();
    fake.set_repos(&session(), &["ledger", "api"]);
    let document = draft_at(&fake, "a", anchor("", "rfc-brin.md", 7, 7), "tighten this");
    assert_eq!(document.anchor.repo, "");
    sent(&fake, &[&document]);
    assert_eq!(fake.typed(&session()), vec!["[comment on rfc-brin.md:7]\ntighten this".to_owned()]);
    let listed = listing(&fake, Which::Open);
    assert!(listed.comments.iter().any(|c| c.anchor.repo.is_empty() && c.anchor.path == "rfc-brin.md"));
}

#[test]
fn a_diff_comment_names_side_and_base_and_quotes_its_lines() {
    let fake = FakeComments::new();
    let mut at = anchor("api", "src/a.rs", 4, 5);
    at.side = Side::Old;
    at.base = Some("origin/main".to_owned());
    at.quoted = vec!["let a = 1;".to_owned(), "let b = 2;".to_owned()];
    let comment = draft_at(&fake, "a", at, "why");
    sent(&fake, &[&comment]);
    assert_eq!(
        fake.typed(&session()),
        vec!["[comment on src/a.rs:4-5 · old · base origin/main]\n> let a = 1;\n> let b = 2;\nwhy"
            .to_owned()]
    );
}

#[test]
fn only_the_chosen_drafts_are_sent() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    let b = draft(&fake, "b", "two");
    sent(&fake, &[&a]);
    assert_eq!(current(&fake, &b.id).status, Status::Draft);
    assert!(is_refused(block_on(fake.send(&session(), &[]))));
    assert!(is_refused(block_on(fake.send(&session(), &[a.id.clone()]))));
}

#[test]
fn nothing_typed_queues_the_comments_and_take_back_returns_them_as_drafts() {
    let fake = FakeComments::new();
    fake.set_typing(&session(), Typing::NotTyped);
    let a = draft(&fake, "a", "one");
    let result = sent(&fake, &[&a]);
    assert!(result.queued);
    assert_eq!(result.comments[0].status, Status::Waiting);
    assert!(!result.comments[0].unconfirmed);
    assert!(fake.typed(&session()).is_empty());

    let Ok(back) = block_on(fake.take_back(&session(), &[a.id.clone()])) else {
        panic!("take back works");
    };
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].status, Status::Draft);
    assert_eq!(listing(&fake, Which::Open).draft_count, 1);
}

#[test]
fn take_back_refuses_a_comment_that_is_not_waiting() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    assert!(is_refused(block_on(fake.take_back(&session(), &[a.id.clone()]))));
}

#[test]
fn an_unknown_send_is_unconfirmed_and_sending_again_types_it_once() {
    let fake = FakeComments::new();
    fake.set_typing(&session(), Typing::Unknown);
    let a = draft(&fake, "a", "one");
    let result = sent(&fake, &[&a]);
    assert!(result.queued);
    assert!(result.comments[0].unconfirmed);
    assert!(fake.typed(&session()).is_empty());

    fake.set_typing(&session(), Typing::Typed);
    let again = sent(&fake, &[&a]);
    assert_eq!(again.comments[0].status, Status::Sent);
    assert!(!again.comments[0].unconfirmed);
    assert_eq!(fake.typed(&session()).len(), 1);
}

#[test]
fn taking_back_one_unconfirmed_comment_returns_its_whole_batch() {
    let fake = FakeComments::new();
    fake.set_typing(&session(), Typing::Unknown);
    let a = draft(&fake, "a", "one");
    let b = draft(&fake, "b", "two");
    let c = draft(&fake, "c", "three");
    sent(&fake, &[&a, &b]);
    let Ok(back) = block_on(fake.take_back(&session(), &[a.id.clone()])) else {
        panic!("take back works");
    };
    let mut ids: Vec<_> = back.iter().map(|x| x.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec![a.id.clone(), b.id.clone()]);
    assert!(back.iter().all(|x| x.status == Status::Draft && !x.unconfirmed));
    assert_eq!(current(&fake, &c.id).status, Status::Draft);
}

#[test]
fn an_archived_comment_is_never_sent_or_counted_and_unarchive_restores_it() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    let Ok(archived) = block_on(fake.archive(&a.id)) else { panic!("archive works") };
    assert!(archived.archived);
    assert_eq!(archived.status, Status::Draft);
    assert!(is_refused(block_on(fake.send(&session(), &[a.id.clone()]))));
    assert_eq!(listing(&fake, Which::Open).draft_count, 0);
    assert!(listing(&fake, Which::Open).comments.is_empty());
    assert_eq!(listing(&fake, Which::Archived).comments.len(), 1);

    let Ok(back) = block_on(fake.unarchive(&a.id)) else { panic!("unarchive works") };
    assert!(!back.archived);
    assert_eq!(back.status, Status::Draft);
    assert_eq!(listing(&fake, Which::Open).draft_count, 1);
}

#[test]
fn list_which_chooses_open_resolved_and_archived() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    let b = draft(&fake, "b", "two");
    let c = draft(&fake, "c", "three");
    assert!(block_on(fake.resolve(&b.id, 1)).is_ok());
    assert!(block_on(fake.archive(&c.id)).is_ok());
    let ids = |which| -> Vec<CommentId> {
        listing(&fake, which).comments.into_iter().map(|x| x.id).collect()
    };
    assert_eq!(ids(Which::Open), vec![a.id.clone()]);
    assert_eq!(ids(Which::WithResolved), vec![a.id.clone(), b.id.clone()]);
    assert_eq!(ids(Which::Archived), vec![c.id.clone()]);
}

#[test]
fn resolve_and_reopen_keep_the_replies() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    sent(&fake, &[&a]);
    let Ok(replied) = fake.agent_reply(&a.id, "looking") else { panic!("reply works") };
    let Ok(resolved) = block_on(fake.resolve(&a.id, replied.revision)) else {
        panic!("resolve works")
    };
    assert_eq!(resolved.status, Status::Resolved);
    assert_eq!(resolved.resolved_by, Some(Author::User));
    assert!(is_refused(block_on(fake.resolve(&a.id, resolved.revision))));

    let Ok(reopened) = block_on(fake.reopen(&a.id, resolved.revision)) else {
        panic!("reopen works")
    };
    assert_eq!(reopened.status, Status::Draft);
    assert_eq!(reopened.resolved_by, None);
    assert_eq!(reopened.replies.len(), 1);
    assert!(is_refused(block_on(fake.reopen(&a.id, reopened.revision))));
}

#[test]
fn the_agent_replies_only_to_comments_that_were_typed() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    assert!(is_refused(fake.agent_reply(&a.id, "hi")));
    sent(&fake, &[&a]);
    let Ok(replied) = fake.agent_reply(&a.id, "hi") else { panic!("reply works") };
    assert_eq!(replied.replies.len(), 1);
    assert_eq!(replied.replies[0].author, Author::Agent("claude".to_owned()));
}

#[test]
fn the_agent_resolves_with_a_reply_and_is_named() {
    let fake = FakeComments::new();
    fake.set_agent(&session(), "codex");
    let a = draft(&fake, "a", "one");
    sent(&fake, &[&a]);
    assert!(is_refused(fake.agent_resolve(&a.id, "")));
    let Ok(resolved) = fake.agent_resolve(&a.id, "done") else { panic!("resolve works") };
    assert_eq!(resolved.status, Status::Resolved);
    assert_eq!(resolved.resolved_by, Some(Author::Agent("codex".to_owned())));
    assert_eq!(resolved.replies.len(), 1);
    assert!(is_refused(fake.agent_resolve(&a.id, "again")));
}

#[test]
fn changed_lines_address_a_sent_comment_detach_a_draft_and_leave_a_resolved_one() {
    let fake = FakeComments::new();
    let sent_one = draft(&fake, "a", "one");
    let draft_one = draft(&fake, "b", "two");
    let resolved_one = draft(&fake, "c", "three");
    sent(&fake, &[&sent_one]);
    assert!(block_on(fake.resolve(&resolved_one.id, 1)).is_ok());

    let Ok(addressed) = fake.lines_changed(&sent_one.id, &["new line"]) else {
        panic!("lines changed works")
    };
    assert_eq!(addressed.status, Status::Addressed);
    assert_eq!(addressed.addressed, Some(vec!["new line".to_owned()]));

    let Ok(detached) = fake.lines_changed(&draft_one.id, &[]) else { panic!("works") };
    assert!(detached.anchor.detached);
    assert_eq!(detached.status, Status::Draft);

    let Ok(same) = fake.lines_changed(&resolved_one.id, &["x"]) else { panic!("works") };
    assert_eq!(same.status, Status::Resolved);
    assert!(!same.anchor.detached);
}

#[test]
fn reattach_clears_detached_and_moves_the_anchor() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    let Ok(detached) = fake.lines_changed(&a.id, &[]) else { panic!("works") };
    assert!(detached.anchor.detached);
    let Ok(moved) =
        block_on(fake.reattach(&a.id, anchor("api", "src/b.rs", 20, 21), detached.revision))
    else {
        panic!("reattach works")
    };
    assert!(!moved.anchor.detached);
    assert_eq!((moved.anchor.path.as_str(), moved.anchor.first), ("src/b.rs", 20));
}

#[test]
fn moved_keeps_the_status() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    sent(&fake, &[&a]);
    let Ok(moved) = fake.moved(&a.id, 12, 12) else { panic!("moved works") };
    assert_eq!(moved.status, Status::Sent);
    assert_eq!(moved.anchor.first, 12);
}

#[test]
fn resolve_addressed_takes_a_file_or_the_session_and_a_repeat_resolves_none() {
    let fake = FakeComments::new();
    let a = draft_at(&fake, "a", anchor("api", "src/a.rs", 1, 1), "one");
    let b = draft_at(&fake, "b", anchor("api", "src/b.rs", 1, 1), "two");
    let c = draft_at(&fake, "c", anchor("api", "src/b.rs", 5, 5), "three");
    sent(&fake, &[&a, &b, &c]);
    for comment in [&a, &b, &c] {
        assert!(fake.lines_changed(&comment.id, &["x"]).is_ok());
    }

    let Ok(one_file) = block_on(fake.resolve_addressed(&session(), Some(("api", "src/b.rs"))))
    else {
        panic!("resolve addressed works")
    };
    assert_eq!(one_file.len(), 2);
    assert!(one_file.iter().all(|x| x.status == Status::Resolved));
    assert_eq!(current(&fake, &a.id).status, Status::Addressed);

    let Ok(rest) = block_on(fake.resolve_addressed(&session(), None)) else { panic!("works") };
    assert_eq!(rest.len(), 1);
    let Ok(none) = block_on(fake.resolve_addressed(&session(), None)) else { panic!("works") };
    assert!(none.is_empty());
}

#[test]
fn watchers_hear_changes_and_drops_and_resync() {
    let fake = FakeComments::new();
    let Ok(mut events) = block_on(fake.watch(&session())) else { panic!("watch works") };
    let a = draft(&fake, "a", "one");
    assert!(matches!(events.try_recv(), Ok(CommentEvent::Changed(c)) if c.id == a.id));
    assert!(block_on(fake.drop_comment(&a.id)).is_ok());
    assert!(matches!(events.try_recv(), Ok(CommentEvent::Dropped(id)) if id == a.id));
    assert!(fake.comments(&session()).is_empty());
    fake.drop_events();
    assert!(matches!(events.try_recv(), Ok(CommentEvent::Resync)));
}

#[test]
fn an_unknown_comment_is_not_found() {
    let fake = FakeComments::new();
    let result = block_on(fake.archive(&CommentId::new("s-1/c-9")));
    assert!(matches!(result, Err(DaemonError::NotFound { .. })));
}

#[test]
fn fail_makes_a_call_fail_until_cleared_and_calls_are_recorded() {
    let fake = FakeComments::new();
    let a = draft(&fake, "a", "one");
    let error = DaemonError::NotFound { what: "this session".to_owned() };
    fake.fail("send", error.clone());
    assert_eq!(block_on(fake.send(&session(), &[a.id.clone()])).err(), Some(error));
    assert_eq!(current(&fake, &a.id).status, Status::Draft);
    fake.clear_failure("send");
    assert!(block_on(fake.send(&session(), &[a.id.clone()])).is_ok());
    assert!(fake.calls().contains(&Call::Send(session(), vec![a.id.clone()])));
}

#[test]
fn a_new_comment_is_for_the_terminal_and_the_user_with_no_link() {
    let fake = FakeComments::new();
    let comment = draft(&fake, "k1", "rename this");
    assert_eq!(comment.destination, Destination::Terminal);
    assert_eq!(comment.author, Author::User);
    assert_eq!(comment.posted_url, None);
    assert!(!comment.post_unconfirmed);
    assert!(!comment.anchor.outdated);
}

#[test]
fn notes_and_pull_request_drafts_are_listed_with_their_destination_and_author() {
    let fake = FakeComments::new();
    let terminal = draft(&fake, "k1", "rename this");
    let note = fake.seed_note(
        &session(),
        anchor("api", "src/lib.rs", 9, 9),
        "Check this",
        Author::Agent("claude".to_owned()),
    );
    let pull_request = fake.seed_pull_request_draft(
        &session(),
        anchor("api", "src/main.rs", 2, 2),
        "Nit",
        Author::User,
        "acme/api",
        12,
    );
    assert_eq!(listing(&fake, Which::Open).comments.len(), 3);
    assert_eq!(current(&fake, &terminal.id).destination, Destination::Terminal);
    assert_eq!(current(&fake, &note.id).destination, Destination::Note);
    assert_eq!(current(&fake, &note.id).author, Author::Agent("claude".to_owned()));
    assert_eq!(
        current(&fake, &pull_request.id).destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 12 }
    );
}

#[test]
fn only_terminal_drafts_count_toward_the_badge_and_only_they_are_sent() {
    let fake = FakeComments::new();
    let terminal = draft(&fake, "k1", "rename this");
    let note = fake.seed_note(&session(), anchor("api", "a.rs", 1, 1), "Check", Author::User);
    let pull_request = fake.seed_pull_request_draft(
        &session(),
        anchor("api", "b.rs", 1, 1),
        "Nit",
        Author::User,
        "acme/api",
        12,
    );
    assert_eq!(listing(&fake, Which::Open).draft_count, 1);
    assert!(is_refused(block_on(fake.send(&session(), &[note.id.clone()]))));
    assert!(is_refused(block_on(fake.send(&session(), &[pull_request.id.clone()]))));
    assert!(fake.typed(&session()).is_empty());

    let result = sent(&fake, &[&terminal]);
    assert_eq!(result.comments[0].status, Status::Sent);
    assert_eq!(listing(&fake, Which::Open).draft_count, 0);
}

#[test]
fn making_a_note_a_draft_keeps_its_id_author_and_anchor() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 12)]);
    let note = fake.seed_note(
        &session(),
        anchor("api", "src/lib.rs", 9, 9),
        "Check this",
        Author::Agent("claude".to_owned()),
    );
    let made = match block_on(fake.make_note_a_draft(&note.id, Some(("acme/api", 12)))) {
        Ok(comment) => comment,
        Err(error) => panic!("a note becomes a draft: {error}"),
    };
    assert_eq!(made.id, note.id);
    assert_eq!(made.author, Author::Agent("claude".to_owned()));
    assert_eq!(made.anchor, note.anchor);
    assert_eq!(made.revision, note.revision + 1);
    assert_eq!(
        made.destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 12 }
    );
    assert_eq!(listing(&fake, Which::Open).draft_count, 0);
}

#[test]
fn making_a_draft_refuses_a_terminal_comment_and_a_note_without_its_pull_request() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 12)]);
    let terminal = draft(&fake, "k1", "rename this");
    assert!(is_refused(block_on(fake.make_note_a_draft(&terminal.id, Some(("acme/api", 12))))));

    // D284: with no pull request named, it is inferred from the note's
    // repository; a note on one the review doesn't cover is refused.
    let note = fake.seed_note(&session(), anchor("docs", "a.rs", 1, 1), "Check", Author::User);
    assert!(is_refused(block_on(fake.make_note_a_draft(&note.id, None))));
    assert_eq!(current(&fake, &note.id).destination, Destination::Note);
}

#[test]
fn a_posted_pull_request_draft_keeps_its_link_and_can_no_longer_change() {
    let fake = FakeComments::new();
    let pull_request = fake.seed_pull_request_draft(
        &session(),
        anchor("api", "a.rs", 1, 1),
        "Nit",
        Author::User,
        "acme/api",
        12,
    );
    let url = "https://github.com/acme/api/pull/12#discussion_r1";
    let posted = match fake.mark_posted(&pull_request.id, url) {
        Ok(comment) => comment,
        Err(error) => panic!("a pull request draft is posted: {error}"),
    };
    assert_eq!(posted.status, Status::Posted);
    assert_eq!(posted.posted_url.as_deref(), Some(url));
    assert!(is_refused(block_on(fake.edit(&pull_request.id, "changed", posted.revision))));
    assert_eq!(listing(&fake, Which::Open).comments.len(), 1);
}

#[test]
fn marking_posted_refuses_a_comment_that_is_not_a_pull_request_draft() {
    let fake = FakeComments::new();
    let terminal = draft(&fake, "k1", "rename this");
    assert!(fake.mark_posted(&terminal.id, "https://example.test/c/1").is_err());
    assert_eq!(current(&fake, &terminal.id).posted_url, None);
}

#[test]
fn post_unconfirmed_and_outdated_are_carried_and_a_posted_draft_is_never_unconfirmed() {
    let fake = FakeComments::new();
    let pull_request = fake.seed_pull_request_draft(
        &session(),
        anchor("api", "a.rs", 1, 1),
        "Nit",
        Author::User,
        "acme/api",
        12,
    );
    let outdated = match fake.set_outdated(&pull_request.id, true) {
        Ok(comment) => comment,
        Err(error) => panic!("outdated is set: {error}"),
    };
    assert!(outdated.anchor.outdated);

    let unconfirmed = match fake.set_post_unconfirmed(&pull_request.id, true) {
        Ok(comment) => comment,
        Err(error) => panic!("post unconfirmed is set: {error}"),
    };
    assert!(unconfirmed.post_unconfirmed);

    let posted = match fake.mark_posted(&pull_request.id, "https://example.test/pr/12") {
        Ok(comment) => comment,
        Err(error) => panic!("a pull request draft is posted: {error}"),
    };
    assert!(!posted.post_unconfirmed);
    assert!(fake.set_post_unconfirmed(&pull_request.id, true).is_err());
}

fn note_on(fake: &FakeComments, repo: &str) -> Comment {
    fake.seed_note(&session(), anchor(repo, "src/lib.rs", 9, 9), "Check this", Author::User)
}

fn made_a_draft(fake: &FakeComments, id: &CommentId, pr: Option<(&str, u32)>) -> Comment {
    match block_on(fake.make_note_a_draft(id, pr)) {
        Ok(comment) => comment,
        Err(error) => panic!("the note becomes a draft: {error}"),
    }
}

#[test]
fn a_draft_with_no_pull_request_named_takes_the_only_one_in_the_notes_repository() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(
        &session(),
        vec![("acme/api".to_owned(), 12), ("acme/web".to_owned(), 3)],
    );
    // By the full name and by the folder name the note's repo shows as.
    let by_full_name = note_on(&fake, "acme/api");
    let by_folder = note_on(&fake, "web");
    assert_eq!(
        made_a_draft(&fake, &by_full_name.id, None).destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 12 }
    );
    assert_eq!(
        made_a_draft(&fake, &by_folder.id, None).destination,
        Destination::PullRequest { repo: "acme/web".to_owned(), number: 3 }
    );
}

#[test]
fn a_draft_with_no_pull_request_named_is_refused_when_none_or_several_match() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(
        &session(),
        vec![("acme/api".to_owned(), 12), ("acme/api".to_owned(), 13)],
    );
    let several = note_on(&fake, "api");
    let none = note_on(&fake, "docs");
    assert!(is_refused(block_on(fake.make_note_a_draft(&several.id, None))));
    assert!(is_refused(block_on(fake.make_note_a_draft(&none.id, None))));
    assert_eq!(current(&fake, &several.id).destination, Destination::Note);
    assert_eq!(current(&fake, &none.id).destination, Destination::Note);

    // Naming the pull request settles the several-match case.
    let named = made_a_draft(&fake, &several.id, Some(("acme/api", 13)));
    assert_eq!(
        named.destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 13 }
    );
}

#[test]
fn a_review_with_no_pull_requests_refuses_inference_and_a_named_one_that_is_not_bound() {
    let fake = FakeComments::new();
    let note = note_on(&fake, "api");
    assert!(is_refused(block_on(fake.make_note_a_draft(&note.id, None))));
    assert!(is_refused(block_on(fake.make_note_a_draft(&note.id, Some(("acme/api", 12))))));
    assert_eq!(current(&fake, &note.id).destination, Destination::Note);
}

#[test]
fn making_a_draft_of_a_pull_request_draft_returns_it_as_it_is() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 12)]);
    let note = note_on(&fake, "api");
    let first = made_a_draft(&fake, &note.id, None);
    let again = made_a_draft(&fake, &note.id, None);
    assert_eq!(again, first);
}

fn pull_request_draft(fake: &FakeComments, key: &str, repo: &str, number: u32) -> Result<Comment, DaemonError> {
    block_on(fake.create_for_pull_request(
        &session(),
        anchor(repo, "src/lib.rs", 7, 7),
        "Nit",
        repo,
        number,
        key,
    ))
}

#[test]
fn creating_a_pull_request_draft_makes_a_user_draft_for_it_that_is_not_typed() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 12)]);
    let made = match pull_request_draft(&fake, "k1", "acme/api", 12) {
        Ok(comment) => comment,
        Err(error) => panic!("a draft for a bound pull request is made: {error}"),
    };
    assert_eq!(made.status, Status::Draft);
    assert_eq!(made.author, Author::User);
    assert_eq!(made.destination, Destination::PullRequest { repo: "acme/api".to_owned(), number: 12 });
    let listed = listing(&fake, Which::Open);
    assert_eq!(listed.comments, vec![made.clone()]);
    assert_eq!(listed.draft_count, 0);
    assert!(is_refused(block_on(fake.send(&session(), &[made.id]))));
}

#[test]
fn a_pull_request_draft_is_refused_for_one_the_review_does_not_hold_or_when_blank() {
    let fake = FakeComments::new();
    assert!(is_refused(pull_request_draft(&fake, "k1", "acme/api", 12)));
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 12)]);
    assert!(is_refused(pull_request_draft(&fake, "k2", "acme/api", 99)));
    let blank = block_on(fake.create_for_pull_request(
        &session(),
        anchor("acme/api", "a.rs", 1, 1),
        "  ",
        "acme/api",
        12,
        "k3",
    ));
    assert!(is_refused(blank));
    assert!(listing(&fake, Which::Open).comments.is_empty());
}

#[test]
fn the_same_key_makes_one_pull_request_draft() {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 12)]);
    let first = pull_request_draft(&fake, "k1", "acme/api", 12);
    let again = pull_request_draft(&fake, "k1", "acme/api", 12);
    assert_eq!(first.map(|c| c.id), again.map(|c| c.id));
    assert_eq!(listing(&fake, Which::Open).comments.len(), 1);
}

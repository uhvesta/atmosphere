//! The comment ledger's rules: drafts, deliveries, restarts, resolving,
//! archiving and how passages changing affect comments.

use humata2_review_diff::{
    CommentAnchor, CommentAuthor, CommentError, CommentFilter, CommentLedger, CommentStatus,
    DeliveryState, FileRef, Side, Typed,
};

fn file(path: &str) -> FileRef {
    FileRef {
        repo: "api".to_owned(),
        path: path.to_owned(),
    }
}

fn anchor(path: &str) -> CommentAnchor {
    CommentAnchor {
        file: file(path),
        side: Side::New,
        first: 3,
        last: 4,
        quoted: vec!["a".to_owned(), "b".to_owned()],
        base: String::new(),
        blob: String::new(),
        detached: false,
    }
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

fn agent() -> CommentAuthor {
    CommentAuthor::Agent("claude".to_owned())
}

/// Two drafts c-1 and c-2.
fn two() -> CommentLedger {
    let mut l = CommentLedger::new();
    l.add(anchor("a.rs"), "first", 1);
    l.add(anchor("b.rs"), "second", 2);
    l
}

/// c-1 sent through d-1.
fn sent_one() -> CommentLedger {
    let mut l = two();
    let key = l.begin(&ids(&["c-1"]), "msg".to_owned(), 10).unwrap().key.clone();
    l.settle(&key, Typed::Yes, 11).unwrap();
    l
}

fn all(l: &CommentLedger) -> Vec<String> {
    l.list(&CommentFilter {
        statuses: &[],
        file: None,
        archived: false,
    })
    .iter()
    .map(|c| c.id.clone())
    .collect()
}

#[test]
fn new_comment_is_a_draft_and_ids_are_not_reused() {
    let mut l = CommentLedger::new();
    let c = l.add(anchor("a.rs"), "", 5);
    assert_eq!(c.id, "c-1");
    assert_eq!(c.status, CommentStatus::Draft);
    assert_eq!(c.revision, 1);
    l.remove("c-1", None).unwrap();
    assert_eq!(l.add(anchor("a.rs"), "x", 6).id, "c-2");
}

#[test]
fn edit_only_drafts_and_refuses_stale() {
    let mut l = sent_one();
    assert!(matches!(
        l.edit("c-1", "x", None),
        Err(CommentError::NotDraft { .. })
    ));
    assert!(matches!(
        l.edit("c-2", "x", Some(9)),
        Err(CommentError::Stale { .. })
    ));
    let c = l.edit("c-2", "changed", Some(1)).unwrap();
    assert_eq!((c.text.as_str(), c.revision), ("changed", 2));
}

#[test]
fn settle_yes_sends_and_removes_the_delivery() {
    let mut l = two();
    let key = l.begin(&ids(&["c-1", "c-2"]), "m".to_owned(), 3).unwrap().key.clone();
    assert_eq!(l.comment("c-1").unwrap().status, CommentStatus::Waiting);
    let touched = l.settle(&key, Typed::Yes, 4).unwrap();
    assert_eq!(touched, ids(&["c-1", "c-2"]));
    assert!(l.delivery(&key).is_none());
    let c = l.comment("c-2").unwrap();
    assert_eq!((c.status, c.sent_at), (CommentStatus::Sent, Some(4)));
}

#[test]
fn settle_no_keeps_waiting_and_resend_uses_a_new_key() {
    let mut l = two();
    let k1 = l.begin(&ids(&["c-1"]), "m".to_owned(), 3).unwrap().key.clone();
    l.settle(&k1, Typed::No, 4).unwrap();
    assert!(l.deliveries().is_empty());
    assert_eq!(l.comment("c-1").unwrap().status, CommentStatus::Waiting);
    let k2 = l.begin(&ids(&["c-1"]), "m".to_owned(), 5).unwrap().key.clone();
    assert_ne!(k1, k2);
}

#[test]
fn unknown_outcome_keeps_the_delivery_and_flags_comments() {
    let mut l = two();
    let key = l.begin(&ids(&["c-1"]), "typed text".to_owned(), 3).unwrap().key.clone();
    l.settle(&key, Typed::Unknown, 4).unwrap();
    assert_eq!(l.delivery(&key).unwrap().state, DeliveryState::Unconfirmed);
    let c = l.comment("c-1").unwrap();
    assert!(c.unconfirmed);
    assert_eq!(c.status, CommentStatus::Waiting);
    assert!(l.in_doubt().is_empty());
    // Not allowed to retry something that is still sending.
    let d = l.retry(&key).unwrap();
    assert_eq!((d.key.as_str(), d.text.as_str()), (key.as_str(), "typed text"));
    assert_eq!(d.state, DeliveryState::Sending);
    assert!(matches!(l.retry(&key), Err(CommentError::NotUnconfirmed { .. })));
}

#[test]
fn begin_is_all_or_nothing() {
    let mut l = two();
    l.add(anchor("c.rs"), "  ", 3);
    let before = l.parts();
    assert!(matches!(
        l.begin(&ids(&["c-1", "c-3"]), "m".to_owned(), 5),
        Err(CommentError::Blank { .. })
    ));
    assert!(matches!(
        l.begin(&ids(&["c-1", "c-9"]), "m".to_owned(), 5),
        Err(CommentError::Unknown { .. })
    ));
    assert!(matches!(
        l.begin(&[], "m".to_owned(), 5),
        Err(CommentError::NothingToSend)
    ));
    assert_eq!(l.parts(), before);
}

#[test]
fn begin_refuses_archived_and_comments_already_in_a_delivery() {
    let mut l = two();
    l.archive("c-2", 5).unwrap();
    assert!(matches!(
        l.begin(&ids(&["c-2"]), "m".to_owned(), 6),
        Err(CommentError::Archived { .. })
    ));
    l.begin(&ids(&["c-1"]), "m".to_owned(), 6).unwrap();
    assert!(matches!(
        l.begin(&ids(&["c-1"]), "m".to_owned(), 7),
        Err(CommentError::InDelivery { .. })
    ));
    assert!(matches!(l.remove("c-1", None), Err(CommentError::InDelivery { .. })));
}

#[test]
fn duplicate_ids_are_sent_once_in_first_position() {
    let mut l = two();
    let d = l.begin(&ids(&["c-2", "c-1", "c-2"]), "m".to_owned(), 5).unwrap();
    assert_eq!(d.comments, ids(&["c-2", "c-1"]));
}

#[test]
fn restart_mid_send_names_the_delivery_in_doubt() {
    let mut l = two();
    let key = l.begin(&ids(&["c-1"]), "m".to_owned(), 3).unwrap().key.clone();
    let restored = CommentLedger::from_parts(l.parts());
    let doubt = restored.in_doubt();
    assert_eq!(doubt.len(), 1);
    assert_eq!(doubt[0].key, key);
    assert_eq!(doubt[0].text, "m");
}

#[test]
fn the_target_of_a_delivery_survives_a_restart() {
    let mut l = two();
    let key = l.begin(&ids(&["c-1"]), "m".to_owned(), 3).unwrap().key.clone();
    assert_eq!(l.delivery(&key).unwrap().holder, 0);
    l.set_target(&key, "tab-7", 99).unwrap();
    let restored = CommentLedger::from_parts(l.parts());
    let delivery = restored.delivery(&key).unwrap();
    assert_eq!((delivery.tab.as_str(), delivery.holder), ("tab-7", 99));
    assert!(l.set_target("d-404", "t", 1).is_err());
}

#[test]
fn any_crash_point_leaves_a_state_a_restart_understands() {
    // Crash after begin: a Sending delivery, comments Waiting.
    let mut l = two();
    let key = l.begin(&ids(&["c-1", "c-2"]), "m".to_owned(), 3).unwrap().key.clone();
    let mut r = CommentLedger::from_parts(l.parts());
    assert_eq!(r, l);
    assert_eq!(r.in_doubt().len(), 1);
    assert!(r.comments().iter().all(|c| c.status == CommentStatus::Waiting));

    // The restart cannot tell: it marks the delivery unknown, then the user
    // takes it back and every comment is a draft again.
    r.settle(&key, Typed::Unknown, 9).unwrap();
    let mut r2 = CommentLedger::from_parts(r.parts());
    assert!(r2.in_doubt().is_empty());
    assert_eq!(r2.delivery(&key).unwrap().state, DeliveryState::Unconfirmed);
    r2.take_back(&ids(&["c-1"])).unwrap();
    assert!(r2.comments().iter().all(|c| c.status == CommentStatus::Draft));

    // Crash after settle: nothing in doubt, comments sent.
    l.settle(&key, Typed::Yes, 4).unwrap();
    let r3 = CommentLedger::from_parts(l.parts());
    assert!(r3.in_doubt().is_empty());
    assert!(r3.deliveries().is_empty());
    assert!(r3.comments().iter().all(|c| c.status == CommentStatus::Sent));
    // Ids keep counting after a restore.
    let mut r3 = r3;
    assert_eq!(r3.add(anchor("z.rs"), "n", 5).id, "c-3");
    assert_eq!(r3.begin(&ids(&["c-3"]), "m".to_owned(), 6).unwrap().key, "d-2");
}

#[test]
fn take_back_returns_held_comments_to_drafts() {
    let mut l = two();
    let key = l.begin(&ids(&["c-1"]), "m".to_owned(), 3).unwrap().key.clone();
    l.settle(&key, Typed::No, 4).unwrap();
    assert_eq!(l.take_back(&ids(&["c-1"])).unwrap(), ids(&["c-1"]));
    assert_eq!(l.comment("c-1").unwrap().status, CommentStatus::Draft);
    assert!(matches!(
        l.take_back(&ids(&["c-1"])),
        Err(CommentError::NotWaiting { .. })
    ));
}

#[test]
fn take_back_of_an_unconfirmed_delivery_returns_all_its_comments() {
    let mut l = two();
    let key = l.begin(&ids(&["c-1", "c-2"]), "m".to_owned(), 3).unwrap().key.clone();
    assert!(matches!(
        l.take_back(&ids(&["c-1"])),
        Err(CommentError::InDelivery { .. })
    ));
    l.settle(&key, Typed::Unknown, 4).unwrap();
    let mut changed = l.take_back(&ids(&["c-1"])).unwrap();
    changed.sort();
    assert_eq!(changed, ids(&["c-1", "c-2"]));
    assert!(l.deliveries().is_empty());
    for c in l.comments() {
        assert_eq!(c.status, CommentStatus::Draft);
        assert!(!c.unconfirmed);
    }
}

#[test]
fn user_resolve_and_reopen() {
    let mut l = two();
    let c = l.resolve("c-1", CommentAuthor::User, None, 7, None).unwrap();
    assert_eq!(c.status, CommentStatus::Resolved);
    let rev = c.revision;
    // Asking again changes nothing.
    assert_eq!(
        l.resolve("c-1", CommentAuthor::User, None, 8, None).unwrap().revision,
        rev
    );
    l.reply("c-1", CommentAuthor::User, "thanks", 9).unwrap();
    let c = l.reopen("c-1", None).unwrap();
    assert_eq!(c.status, CommentStatus::Draft);
    assert!(c.resolved.is_none() && c.sent_at.is_none() && c.addressed.is_none());
    assert_eq!(c.replies.len(), 1);
    assert!(matches!(l.reopen("c-1", None), Err(CommentError::NotResolved { .. })));
}

#[test]
fn user_cannot_resolve_a_comment_being_typed() {
    let mut l = two();
    l.begin(&ids(&["c-1"]), "m".to_owned(), 3).unwrap();
    assert!(matches!(
        l.resolve("c-1", CommentAuthor::User, None, 4, None),
        Err(CommentError::InDelivery { .. })
    ));
}

#[test]
fn agent_resolve_needs_a_reply_and_a_sent_comment() {
    let mut l = sent_one();
    assert!(matches!(
        l.resolve("c-2", agent(), Some("done"), 20, None),
        Err(CommentError::NotSent { .. })
    ));
    assert!(matches!(
        l.resolve("c-1", agent(), None, 20, None),
        Err(CommentError::ReplyNeeded { .. })
    ));
    assert!(matches!(
        l.resolve("c-1", agent(), Some("  "), 20, None),
        Err(CommentError::ReplyNeeded { .. })
    ));
    let c = l.resolve("c-1", agent(), Some("fixed"), 20, None).unwrap();
    assert_eq!(c.status, CommentStatus::Resolved);
    assert_eq!(c.replies.len(), 1);
    assert_eq!(c.replies[0].by, agent());
    assert!(matches!(
        l.resolve("c-1", agent(), Some("again"), 21, None),
        Err(CommentError::AlreadyResolved { .. })
    ));
}

#[test]
fn replies_only_on_typed_comments_and_ids_are_not_reused() {
    let mut l = sent_one();
    assert!(matches!(
        l.reply("c-2", agent(), "hi", 20),
        Err(CommentError::NotSent { .. })
    ));
    assert!(matches!(
        l.reply("c-1", agent(), " ", 20),
        Err(CommentError::Blank { .. })
    ));
    let a = l.reply("c-1", agent(), "looking", 20).unwrap().id.clone();
    let b = l.reply("c-1", CommentAuthor::User, "ok", 21).unwrap().id.clone();
    assert_ne!(a, b);
}

#[test]
fn archive_keeps_status_and_hides_the_comment() {
    let mut l = sent_one();
    l.archive("c-1", 30).unwrap();
    l.archive("c-2", 30).unwrap();
    assert_eq!(l.comment("c-1").unwrap().status, CommentStatus::Sent);
    assert_eq!(l.draft_count(), 0);
    assert!(all(&l).is_empty());
    let archived = l.list(&CommentFilter {
        statuses: &[],
        file: None,
        archived: true,
    });
    assert_eq!(archived.len(), 2);
    let rev = l.comment("c-1").unwrap().revision;
    l.archive("c-1", 31).unwrap();
    assert_eq!(l.comment("c-1").unwrap().revision, rev);
    l.unarchive("c-2").unwrap();
    assert_eq!(l.draft_count(), 1);
    assert_eq!(all(&l), ids(&["c-2"]));
}

#[test]
fn archiving_a_held_comment_makes_it_a_draft_and_a_batch_member_is_refused() {
    let mut l = two();
    let k = l.begin(&ids(&["c-1"]), "m".to_owned(), 3).unwrap().key.clone();
    assert!(matches!(l.archive("c-1", 4), Err(CommentError::InDelivery { .. })));
    l.settle(&k, Typed::No, 5).unwrap();
    assert_eq!(l.archive("c-1", 6).unwrap().status, CommentStatus::Draft);
}

#[test]
fn changed_addresses_sent_comments_and_keeps_both_passages() {
    let mut l = sent_one();
    assert!(l.changed("c-1", 7, 8, vec!["new".to_owned()], 40));
    let c = l.comment("c-1").unwrap();
    assert_eq!(c.status, CommentStatus::Addressed);
    assert_eq!((c.anchor.first, c.anchor.last), (7, 8));
    assert_eq!(c.anchor.quoted, ids(&["a", "b"]));
    assert_eq!(c.addressed.as_ref().unwrap().now, ids(&["new"]));
    // Addressed never changes again on its own.
    assert!(!l.changed("c-1", 1, 1, vec![], 41));
    assert_eq!(l.comment("c-1").unwrap().addressed.as_ref().unwrap().at, 40);
}

#[test]
fn changed_detaches_drafts_but_leaves_resolved_and_archived_alone() {
    let mut l = two();
    l.add(anchor("c.rs"), "third", 3);
    l.resolve("c-2", CommentAuthor::User, None, 4, None).unwrap();
    l.archive("c-3", 5).unwrap();
    assert!(l.changed("c-1", 3, 4, vec![], 6));
    assert!(l.comment("c-1").unwrap().anchor.detached);
    assert_eq!(l.comment("c-1").unwrap().status, CommentStatus::Draft);
    assert!(!l.changed("c-2", 3, 4, vec![], 6));
    assert!(!l.changed("c-3", 3, 4, vec![], 6));
    assert!(!l.comment("c-3").unwrap().anchor.detached);
    // A detached comment can't be sent until it is re-attached.
    assert!(matches!(
        l.begin(&ids(&["c-1"]), "m".to_owned(), 7),
        Err(CommentError::Detached { .. })
    ));
    assert!(!l.reattach("c-1", anchor("a.rs"), None).unwrap().anchor.detached);
    let k = l.begin(&ids(&["c-1"]), "m".to_owned(), 8).unwrap().key.clone();
    assert!(matches!(
        l.reattach("c-1", anchor("a.rs"), None),
        Err(CommentError::InDelivery { .. })
    ));
    l.settle(&k, Typed::No, 9).unwrap();
}

#[test]
fn moved_only_renumbers() {
    let mut l = sent_one();
    assert!(!l.moved("c-1", 3, 4));
    assert!(l.moved("c-1", 10, 11));
    let c = l.comment("c-1").unwrap();
    assert_eq!((c.anchor.first, c.anchor.last), (10, 11));
    assert_eq!(c.status, CommentStatus::Sent);
    assert!(c.addressed.is_none());
    l.changed("c-2", 3, 4, vec![], 5);
    assert!(l.comment("c-2").unwrap().anchor.detached);
    assert!(l.moved("c-2", 1, 2));
    assert!(!l.comment("c-2").unwrap().anchor.detached);
}

#[test]
fn list_hides_resolved_by_default_and_filters_by_status_and_file() {
    let mut l = sent_one();
    l.resolve("c-2", CommentAuthor::User, None, 20, None).unwrap();
    assert_eq!(all(&l), ids(&["c-1"]));
    let resolved = l.list(&CommentFilter {
        statuses: &[CommentStatus::Resolved],
        file: None,
        archived: false,
    });
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].id, "c-2");
    let both = l.list(&CommentFilter {
        statuses: &[CommentStatus::Resolved, CommentStatus::Sent],
        file: None,
        archived: false,
    });
    assert_eq!(both.len(), 2);

    let in_repo = file("a.rs");
    let any_repo = FileRef {
        repo: String::new(),
        path: "a.rs".to_owned(),
    };
    let other_repo = FileRef {
        repo: "web".to_owned(),
        path: "a.rs".to_owned(),
    };
    let count = |f: &FileRef| {
        l.list(&CommentFilter {
            statuses: &[],
            file: Some(f),
            archived: false,
        })
        .len()
    };
    assert_eq!(count(&in_repo), 1);
    assert_eq!(count(&any_repo), 1);
    assert_eq!(count(&other_repo), 0);
}

#[test]
fn an_addressed_comment_reopens_as_a_draft() {
    let mut l = sent_one();
    assert!(l.changed("c-1", 7, 8, vec!["new".to_owned()], 40));
    let c = l.reopen("c-1", None).unwrap();
    assert_eq!(c.status, CommentStatus::Draft);
    assert!(c.addressed.is_none() && c.sent_at.is_none());
}

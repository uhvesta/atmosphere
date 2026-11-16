//! Retention decisions (LD-16, LD-17, LD-18, TR-9).

use humata2_transfers::retention::{Decision, Rule, Rules, SessionFacts, SkipReason, Work, decide};

const DAY: i64 = 24 * 60 * 60 * 1000;
const NOW: i64 = 1_000 * DAY;

fn session(id: &str) -> SessionFacts {
    SessionFacts {
        id: id.to_string(),
        title: format!("title {id}"),
        archived: false,
        is_pr_review: false,
        pr_closed_unix_ms: None,
        created_unix_ms: 0,
        last_output_unix_ms: None,
        last_input_unix_ms: None,
        last_opened_unix_ms: None,
        work: Work::Safe,
    }
}

fn pr_rules(days: u32) -> Rules {
    Rules {
        archive_pr_reviews_after_days: days,
        archive_idle_sessions_after_days: 0,
    }
}

fn idle_rules(days: u32) -> Rules {
    Rules {
        archive_pr_reviews_after_days: 0,
        archive_idle_sessions_after_days: days,
    }
}

fn pr_review(id: &str, closed: Option<i64>) -> SessionFacts {
    SessionFacts {
        is_pr_review: true,
        pr_closed_unix_ms: closed,
        ..session(id)
    }
}

fn archived_ids(d: &Decision) -> Vec<&str> {
    d.archive.iter().map(|a| a.session.as_str()).collect()
}

#[test]
fn both_rules_off_decide_nothing() {
    let mut s = pr_review("a", Some(0));
    s.last_output_unix_ms = Some(0);
    let d = decide(&Rules::default(), &[s], NOW);
    assert!(d.archive.is_empty());
    assert!(d.skips.is_empty());
    assert_eq!(d.next_due_unix_ms, None);
}

#[test]
fn a_pr_review_closed_exactly_n_days_ago_is_archived() {
    let s = pr_review("a", Some(NOW - 7 * DAY));
    let d = decide(&pr_rules(7), &[s], NOW);
    assert_eq!(archived_ids(&d), ["a"]);
    assert_eq!(d.archive[0].rule, Rule::PrReviews { days: 7 });
}

#[test]
fn a_pr_review_closed_a_millisecond_short_of_n_days_is_not_archived() {
    let s = pr_review("a", Some(NOW - 7 * DAY + 1));
    let d = decide(&pr_rules(7), &[s], NOW);
    assert!(d.archive.is_empty());
    assert_eq!(d.next_due_unix_ms, Some(NOW + 1));
}

#[test]
fn an_open_pr_review_is_not_due_under_the_pr_rule() {
    let d = decide(&pr_rules(1), &[pr_review("a", None)], NOW);
    assert!(d.archive.is_empty());
    assert!(d.skips.is_empty());
    assert_eq!(d.next_due_unix_ms, None);
}

#[test]
fn a_session_that_is_not_a_pr_review_ignores_the_pr_rule() {
    let mut s = session("a");
    s.pr_closed_unix_ms = Some(0);
    let d = decide(&pr_rules(1), &[s], NOW);
    assert!(d.archive.is_empty());
}

#[test]
fn a_session_with_output_yesterday_is_not_inactive_even_if_created_long_ago() {
    let mut s = session("a");
    s.last_output_unix_ms = Some(NOW - DAY);
    let d = decide(&idle_rules(7), &[s], NOW);
    assert!(d.archive.is_empty());
    assert_eq!(d.next_due_unix_ms, Some(NOW + 6 * DAY));
}

#[test]
fn a_session_with_no_activity_for_n_days_is_archived_as_inactive() {
    let mut s = session("a");
    s.created_unix_ms = NOW - 30 * DAY;
    s.last_input_unix_ms = Some(NOW - 7 * DAY);
    let d = decide(&idle_rules(7), &[s], NOW);
    assert_eq!(archived_ids(&d), ["a"]);
    assert_eq!(d.archive[0].rule, Rule::Inactive { days: 7 });
}

#[test]
fn opening_a_session_counts_as_activity() {
    let mut s = session("a");
    s.last_opened_unix_ms = Some(NOW - DAY);
    let d = decide(&idle_rules(7), &[s], NOW);
    assert!(d.archive.is_empty());
}

#[test]
fn input_counts_as_activity() {
    let mut s = session("a");
    s.last_input_unix_ms = Some(NOW - DAY);
    let d = decide(&idle_rules(7), &[s], NOW);
    assert!(d.archive.is_empty());
}

#[test]
fn an_open_pr_review_can_still_be_due_by_inactivity() {
    let d = decide(&idle_rules(7), &[pr_review("a", None)], NOW);
    assert_eq!(archived_ids(&d), ["a"]);
    assert_eq!(d.archive[0].rule, Rule::Inactive { days: 7 });
}

#[test]
fn a_session_due_under_both_rules_is_archived_once_under_the_pr_rule() {
    let rules = Rules {
        archive_pr_reviews_after_days: 3,
        archive_idle_sessions_after_days: 7,
    };
    let d = decide(&rules, &[pr_review("a", Some(NOW - 10 * DAY))], NOW);
    assert_eq!(d.archive.len(), 1);
    assert_eq!(d.archive[0].rule, Rule::PrReviews { days: 3 });
    assert!(d.skips.is_empty());
}

#[test]
fn uncommitted_work_is_skipped_with_its_reason_and_never_archived() {
    let mut s = session("a");
    s.work = Work::Uncommitted { files: 3 };
    let d = decide(&idle_rules(7), &[s], NOW);
    assert!(d.archive.is_empty());
    assert_eq!(d.skips.len(), 1);
    assert_eq!(d.skips[0].session, "a");
    assert_eq!(d.skips[0].title, "title a");
    assert_eq!(d.skips[0].rule, Rule::Inactive { days: 7 });
    assert_eq!(d.skips[0].reason, SkipReason::Uncommitted { files: 3 });
}

#[test]
fn unpushed_work_is_skipped_with_its_branches_and_never_archived() {
    let mut s = pr_review("a", Some(0));
    s.work = Work::Unpushed {
        branches: vec!["x".into(), "y".into()],
    };
    let d = decide(&pr_rules(1), &[s], NOW);
    assert!(d.archive.is_empty());
    assert_eq!(d.skips[0].rule, Rule::PrReviews { days: 1 });
    assert_eq!(
        d.skips[0].reason,
        SkipReason::Unpushed {
            branches: vec!["x".into(), "y".into()]
        }
    );
}

#[test]
fn work_never_reported_is_skipped_as_unknown_and_never_archived() {
    let mut s = session("a");
    s.work = Work::Unknown;
    let d = decide(&idle_rules(7), &[s], NOW);
    assert!(d.archive.is_empty());
    assert_eq!(d.skips[0].reason, SkipReason::WorkUnknown);
}

#[test]
fn a_session_that_is_not_due_is_not_a_skip() {
    let mut s = session("a");
    s.last_output_unix_ms = Some(NOW);
    s.work = Work::Unknown;
    let d = decide(&idle_rules(7), &[s], NOW);
    assert!(d.skips.is_empty());
}

#[test]
fn an_archived_session_is_ignored() {
    let mut s = pr_review("a", Some(0));
    s.archived = true;
    s.work = Work::Unknown;
    let rules = Rules {
        archive_pr_reviews_after_days: 1,
        archive_idle_sessions_after_days: 1,
    };
    let d = decide(&rules, &[s], NOW);
    assert!(d.archive.is_empty());
    assert!(d.skips.is_empty());
    assert_eq!(d.next_due_unix_ms, None);
}

#[test]
fn the_earliest_due_moment_among_pending_sessions_wins() {
    let mut a = session("a");
    a.last_output_unix_ms = Some(NOW - DAY);
    let mut b = session("b");
    b.last_output_unix_ms = Some(NOW - 3 * DAY);
    let d = decide(&idle_rules(7), &[a, b], NOW);
    assert_eq!(d.next_due_unix_ms, Some(NOW + 4 * DAY));
}

#[test]
fn the_next_due_moment_considers_the_pr_rule_too() {
    let rules = Rules {
        archive_pr_reviews_after_days: 2,
        archive_idle_sessions_after_days: 30,
    };
    let mut s = pr_review("a", Some(NOW - DAY));
    s.last_output_unix_ms = Some(NOW);
    let d = decide(&rules, &[s], NOW);
    assert_eq!(d.next_due_unix_ms, Some(NOW + DAY));
}

#[test]
fn skipped_sessions_do_not_set_the_next_due_moment() {
    let mut s = session("a");
    s.work = Work::Unknown;
    let d = decide(&idle_rules(7), &[s], NOW);
    assert_eq!(d.next_due_unix_ms, None);
}

#[test]
fn nothing_pending_gives_no_next_due_moment() {
    let d = decide(&idle_rules(7), &[], NOW);
    assert_eq!(d.next_due_unix_ms, None);
}

#[test]
fn extreme_times_and_days_do_not_panic() {
    let mut s = pr_review("a", Some(i64::MAX));
    s.created_unix_ms = i64::MIN;
    s.last_output_unix_ms = Some(i64::MAX);
    let rules = Rules {
        archive_pr_reviews_after_days: u32::MAX,
        archive_idle_sessions_after_days: u32::MAX,
    };
    for now in [i64::MIN, 0, i64::MAX] {
        let _ = decide(&rules, std::slice::from_ref(&s), now);
    }
    let mut old = session("b");
    old.created_unix_ms = i64::MIN;
    let _ = decide(&idle_rules(u32::MAX), &[old], i64::MIN);
}

#[test]
fn archive_and_skips_follow_the_input_order() {
    let mut u = session("u");
    u.work = Work::Unknown;
    let sessions = [session("c"), u, session("a"), {
        let mut x = session("x");
        x.work = Work::Uncommitted { files: 1 };
        x
    }, session("b")];
    let d = decide(&idle_rules(1), &sessions, NOW);
    assert_eq!(archived_ids(&d), ["c", "a", "b"]);
    let skipped: Vec<&str> = d.skips.iter().map(|s| s.session.as_str()).collect();
    assert_eq!(skipped, ["u", "x"]);
}

#[test]
fn sentences_are_not_empty_and_differ_between_reasons_and_rules() {
    let reasons = [
        SkipReason::Uncommitted { files: 2 },
        SkipReason::Unpushed {
            branches: vec!["main".into()],
        },
        SkipReason::WorkUnknown,
    ];
    let sentences: Vec<String> = reasons.iter().map(SkipReason::sentence).collect();
    assert!(sentences.iter().all(|s| !s.is_empty()));
    assert_ne!(sentences[0], sentences[1]);
    assert_ne!(sentences[1], sentences[2]);
    assert_ne!(sentences[0], sentences[2]);
    let (a, b) = (Rule::PrReviews { days: 3 }.sentence(), Rule::Inactive { days: 3 }.sentence());
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
}

//! The approval passphrase (SEC-13, AW-5): what is accepted as one, that only
//! its hash is kept and checks it, and that five wrong tries lock the
//! machine's prompt for 15 minutes.

use std::time::{Duration, SystemTime};

use humata2_passphrase::{generate, Attempt, Lockout, Verifier};

const GOOD: &str = "correct horse battery staple orbit";

#[test]
fn fewer_than_five_words_is_refused() {
    assert!(Verifier::hash("correct horse battery staple").is_err());
    assert!(Verifier::hash("   ").is_err());
    assert!(Verifier::hash(GOOD).is_ok());
}

#[test]
fn the_hash_checks_the_passphrase_and_nothing_else() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");

    assert!(verifier.matches(GOOD));
    assert!(!verifier.matches("correct horse battery staple orbits"));
    assert!(!verifier.matches(""));
}

#[test]
fn spacing_and_case_do_not_matter_when_typed_at_a_prompt() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");

    assert!(verifier.matches("  Correct  horse battery\tstaple ORBIT \n"));
}

#[test]
fn the_kept_form_never_holds_the_passphrase_and_survives_a_round_trip() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");
    let kept = verifier.encoded().to_owned();

    assert!(!kept.contains("horse"));
    let back = Verifier::parse(&kept).expect("the kept form parses");
    assert!(back.matches(GOOD));
    assert!(Verifier::parse("not a hash").is_err());
}

#[test]
fn hashing_twice_gives_different_kept_forms() {
    let first = Verifier::hash(GOOD).expect("a verifier");
    let second = Verifier::hash(GOOD).expect("a verifier");

    assert_ne!(first.encoded(), second.encoded());
}

#[test]
fn a_generated_passphrase_has_at_least_five_words_and_is_accepted() {
    let phrase = generate();
    let other = generate();

    assert!(phrase.split_whitespace().count() >= 5);
    assert_ne!(phrase, other);
    let verifier = Verifier::hash(&phrase).expect("a generated phrase is accepted");
    assert!(verifier.matches(&phrase));
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
}

#[test]
fn five_wrong_tries_lock_for_fifteen_minutes() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");
    let mut lockout = Lockout::default();

    for second in 0..4 {
        assert_eq!(lockout.attempt(&verifier, "wrong", at(second)), Attempt::Wrong);
    }
    assert!(matches!(lockout.attempt(&verifier, "wrong", at(4)), Attempt::Locked { .. }));
    // Even the right passphrase is refused while locked.
    let Attempt::Locked { until } = lockout.attempt(&verifier, GOOD, at(60)) else {
        panic!("still locked");
    };
    assert_eq!(until, at(4) + Duration::from_secs(15 * 60));
    assert!(matches!(lockout.attempt(&verifier, GOOD, at(4 + 15 * 60 - 1)), Attempt::Locked { .. }));
    assert_eq!(lockout.attempt(&verifier, GOOD, at(4 + 15 * 60)), Attempt::Accepted);
}

#[test]
fn a_right_try_clears_the_count() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");
    let mut lockout = Lockout::default();

    for second in 0..4 {
        lockout.attempt(&verifier, "wrong", at(second));
    }
    assert_eq!(lockout.attempt(&verifier, GOOD, at(5)), Attempt::Accepted);
    for second in 6..10 {
        assert_eq!(lockout.attempt(&verifier, "wrong", at(second)), Attempt::Wrong);
    }
}

#[test]
fn after_a_lock_ends_the_count_starts_again() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");
    let mut lockout = Lockout::default();
    for second in 0..5 {
        lockout.attempt(&verifier, "wrong", at(second));
    }
    let after = 4 + 15 * 60;

    assert_eq!(lockout.attempt(&verifier, "wrong", at(after)), Attempt::Wrong);
}

#[test]
fn the_lockout_survives_being_kept_and_read_back() {
    let verifier = Verifier::hash(GOOD).expect("a verifier");
    let mut lockout = Lockout::default();
    for second in 0..5 {
        lockout.attempt(&verifier, "wrong", at(second));
    }

    let (failures, locked_until) = (lockout.failures(), lockout.locked_until());
    let mut back = Lockout::restore(failures, locked_until);
    assert!(matches!(back.attempt(&verifier, GOOD, at(10)), Attempt::Locked { .. }));
}

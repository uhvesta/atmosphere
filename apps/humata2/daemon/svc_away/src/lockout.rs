//! The approval passphrase's lockout, kept in the store so a restart keeps
//! the count and the lock (SEC-13, AW-5). A try is counted before it is
//! checked, so a crash mid-check still costs the try.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_passphrase::{Attempt, Lockout, PassphraseError, Verifier};
use humata2_store::{Store, StoreError};

use super::passphrase::verifier;

/// The settings key the count and the lock are kept under, as
/// `failures,locked_until_unix_secs` with the second part empty when
/// unlocked.
const LOCKOUT_KEY: &str = "approval_lockout";

/// Wrong tries in a row that lock the prompt (the passphrase crate's rule,
/// repeated here only to say how many are left).
const MAX_TRIES: u32 = 5;

/// Serializes tries, so parallel ones can't get around the count.
static TRIES: Mutex<()> = Mutex::new(());

/// What a typed try came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Checked {
    Accepted,
    Wrong { tries_left: u32 },
    Locked { until: SystemTime },
}

/// Why a try couldn't be checked.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error(
        "couldn't read or keep the approval passphrase's try count: {source}; \
         free disk space on this machine and try again"
    )]
    Store {
        #[source]
        source: StoreError,
    },

    #[error("this machine holds no approval passphrase; set one on the Mac and connect this machine once so it receives it")]
    NoVerifier,

    #[error(
        "the approval passphrase this machine holds can't be read: {source}; \
         set the passphrase again on the Mac and connect this machine once so it receives the new one"
    )]
    Verifier {
        #[source]
        source: PassphraseError,
    },

    #[error(
        "the kept approval passphrase try count {value:?} is damaged, so no try can be counted safely; \
         set the passphrase again on the Mac and connect this machine once, which replaces it"
    )]
    DamagedCount { value: String },
}

/// Checks `typed` against the held passphrase. Blocking (Argon2id): call it
/// from `spawn_blocking`.
pub fn check_passphrase(store: &Store, typed: &str) -> Result<Checked, CheckError> {
    check_at(store, typed, SystemTime::now())
}

fn check_at(store: &Store, typed: &str, now: SystemTime) -> Result<Checked, CheckError> {
    let _one_at_a_time = TRIES.lock().unwrap_or_else(PoisonError::into_inner);
    let encoded = verifier(store).map_err(|source| CheckError::Store { source })?;
    if encoded.is_empty() {
        return Err(CheckError::NoVerifier);
    }
    let held = Verifier::parse(&encoded).map_err(|source| CheckError::Verifier { source })?;
    let (mut lockout, revision) = load(store)?;
    let locked = lockout.locked_until().is_some_and(|until| now < until);
    if !locked {
        // The try counts even if the daemon dies before the check ends.
        let before = if lockout.locked_until().is_some() {
            0
        } else {
            lockout.failures()
        };
        save(store, before.saturating_add(1), None, revision)?;
    }
    let revision = if locked { revision } else { revision + 1 };
    let result = lockout.attempt(&held, typed, now);
    save(store, lockout.failures(), lockout.locked_until(), revision)?;
    Ok(match result {
        Attempt::Accepted => Checked::Accepted,
        Attempt::Wrong => Checked::Wrong {
            tries_left: MAX_TRIES.saturating_sub(lockout.failures()),
        },
        Attempt::Locked { until } => Checked::Locked { until },
    })
}

/// Clears the count and the lock, for a new passphrase.
pub(super) fn forget(store: &Store) -> Result<(), StoreError> {
    let revision = store.setting(LOCKOUT_KEY)?.map_or(0, |saved| saved.revision);
    store.put_setting(LOCKOUT_KEY, "0,", revision + 1)
}

fn load(store: &Store) -> Result<(Lockout, u64), CheckError> {
    let Some(saved) = store
        .setting(LOCKOUT_KEY)
        .map_err(|source| CheckError::Store { source })?
    else {
        return Ok((Lockout::default(), 0));
    };
    let damaged = || CheckError::DamagedCount {
        value: saved.value.clone(),
    };
    let (failures, until) = saved.value.split_once(',').ok_or_else(damaged)?;
    let failures: u32 = failures.parse().map_err(|_| damaged())?;
    let until = if until.is_empty() {
        None
    } else {
        let secs: u64 = until.parse().map_err(|_| damaged())?;
        Some(UNIX_EPOCH + Duration::from_secs(secs))
    };
    Ok((Lockout::restore(failures, until), saved.revision))
}

fn save(
    store: &Store,
    failures: u32,
    locked_until: Option<SystemTime>,
    revision: u64,
) -> Result<(), CheckError> {
    let until = locked_until
        .map(|at| at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().to_string())
        .unwrap_or_default();
    store
        .put_setting(LOCKOUT_KEY, &format!("{failures},{until}"), revision + 1)
        .map_err(|source| CheckError::Store { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passphrase::put_verifier;

    const RIGHT: &str = "copper lantern drifts over harbor";
    const WRONG: &str = "copper lantern drifts over meadow";

    fn held() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let encoded = Verifier::hash(RIGHT).unwrap().encoded().to_owned();
        put_verifier(&store, &encoded, 1).unwrap();
        (directory, store)
    }

    #[test]
    fn a_right_try_clears_the_count() {
        let (_directory, store) = held();
        let now = SystemTime::now();
        for _ in 0..4 {
            assert!(matches!(check_at(&store, WRONG, now).unwrap(), Checked::Wrong { .. }));
        }
        assert_eq!(check_at(&store, RIGHT, now).unwrap(), Checked::Accepted);
        // Four more wrong tries are four, not eight: none locks.
        for _ in 0..4 {
            assert!(matches!(check_at(&store, WRONG, now).unwrap(), Checked::Wrong { .. }));
        }
    }

    #[test]
    fn a_new_passphrase_from_the_host_lifts_the_lock() {
        let (_directory, store) = held();
        let now = SystemTime::now();
        save(&store, 0, Some(now + Duration::from_secs(600)), 0).unwrap();
        let other = "amber kettle hums beside window";
        let encoded = Verifier::hash(other).unwrap().encoded().to_owned();
        put_verifier(&store, &encoded, 2).unwrap();
        assert_eq!(check_at(&store, other, now).unwrap(), Checked::Accepted);
    }

    #[test]
    fn a_try_that_died_mid_check_still_counts() {
        let (_directory, store) = held();
        // What the pre-check write leaves behind when the daemon dies.
        save(&store, 4, None, 0).unwrap();
        let now = SystemTime::now();

        // The next wrong try is the fifth.
        assert!(matches!(
            check_at(&store, WRONG, now).unwrap(),
            Checked::Locked { .. }
        ));
    }

    #[test]
    fn the_fifth_wrong_try_locks() {
        let (_directory, store) = held();
        let now = SystemTime::now();

        for _ in 0..4 {
            assert!(matches!(
                check_at(&store, WRONG, now).unwrap(),
                Checked::Wrong { .. }
            ));
        }
        assert!(matches!(
            check_at(&store, WRONG, now).unwrap(),
            Checked::Locked { .. }
        ));
    }

    #[test]
    fn a_restored_lock_refuses_the_right_passphrase() {
        let (_directory, store) = held();
        let now = SystemTime::now();
        let until = now + Duration::from_secs(600);
        save(&store, 0, Some(until), 0).unwrap();

        let checked = check_at(&store, RIGHT, now).unwrap();

        assert!(matches!(checked, Checked::Locked { .. }));
        // Once the lock has passed, the right one is accepted.
        let later = until + Duration::from_secs(1);
        assert_eq!(check_at(&store, RIGHT, later).unwrap(), Checked::Accepted);
    }

    #[test]
    fn no_held_passphrase_is_an_error_not_a_refusal() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();

        assert!(matches!(
            check_at(&store, RIGHT, SystemTime::now()),
            Err(CheckError::NoVerifier)
        ));
    }
}

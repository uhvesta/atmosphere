//! The approval passphrase (SEC-13, AW-5): what counts as one, how it is kept
//! (only as an Argon2id hash), how a typed try is checked at a machine's
//! prompt, and the lockout after five wrong tries.
//!
//! Pure logic. The daemon keeps the [`Verifier`]'s PHC string and the
//! [`Lockout`] counters in its store; nothing here touches disk or network.

use std::time::{Duration, SystemTime};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::seq::IndexedRandom;

/// An approval passphrase has at least this many words.
const MIN_WORDS: usize = 5;
/// Words in a passphrase made by [`generate`].
const GENERATED_WORDS: usize = 6;
/// Wrong tries in a row that lock the prompt.
const MAX_FAILURES: u32 = 5;
/// How long the prompt stays locked after the fifth wrong try.
const LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// The only algorithm a kept hash may use.
const ARGON2ID: &str = "argon2id";
/// One lowercase English word per line.
const WORDS: &str = include_str!("words.txt");

/// Why a passphrase could not be accepted, kept or read back.
#[derive(Debug, thiserror::Error)]
pub enum PassphraseError {
    /// The passphrase has fewer than five words once spacing is normalised.
    #[error(
        "an approval passphrase needs at least five words, and this one has {words}; \
         type a longer one or let Humata generate one"
    )]
    TooFewWords {
        /// Words found in what was typed.
        words: usize,
    },

    /// Argon2id could not hash the passphrase.
    #[error(
        "could not hash the approval passphrase with Argon2id ({source}); \
         close other apps to free memory, then try again"
    )]
    Hash {
        /// The hashing failure reported by the Argon2 crate.
        #[source]
        source: argon2::password_hash::Error,
    },

    /// The stored text is not a PHC string at all.
    #[error(
        "the stored approval passphrase is not a valid PHC string ({source}), so it was damaged or written by another tool; \
         the passphrase cannot be read back from it, so set the approval passphrase again"
    )]
    MalformedHash {
        /// Why the text did not parse.
        #[source]
        source: argon2::password_hash::phc::Error,
    },

    /// The stored PHC string has no salt or no hash value.
    #[error(
        "the stored approval passphrase hash has no salt or no hash value, so it cannot be checked; \
         set the approval passphrase again"
    )]
    IncompleteHash,

    /// The stored hash uses an algorithm other than Argon2id.
    #[error(
        "the stored approval passphrase was hashed with {algorithm}, but Humata only checks Argon2id; \
         set the approval passphrase again so it is hashed with Argon2id"
    )]
    NotArgon2id {
        /// The algorithm named in the stored hash.
        algorithm: String,
    },
}

/// The kept form of an approval passphrase: its Argon2id PHC string and
/// nothing else. The plain passphrase is never held here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verifier {
    encoded: String,
}

impl Verifier {
    /// Hashes a new approval passphrase with Argon2id and a random salt.
    ///
    /// The passphrase is normalised first (see [`Verifier::matches`]), and it
    /// must have at least five words.
    pub fn hash(passphrase: &str) -> Result<Self, PassphraseError> {
        let words = passphrase.split_whitespace().count();
        if words < MIN_WORDS {
            return Err(PassphraseError::TooFewWords { words });
        }
        let hashed: PasswordHash = Argon2::default()
            .hash_password(normalise(passphrase).as_bytes())
            .map_err(|source| PassphraseError::Hash { source })?;
        Ok(Self {
            encoded: hashed.to_string(),
        })
    }

    /// Reads back a kept PHC string, which must be an Argon2id hash with a
    /// salt and a hash value.
    pub fn parse(encoded: &str) -> Result<Self, PassphraseError> {
        let parsed = PasswordHash::new(encoded)
            .map_err(|source| PassphraseError::MalformedHash { source })?;
        if parsed.algorithm.as_str() != ARGON2ID {
            return Err(PassphraseError::NotArgon2id {
                algorithm: parsed.algorithm.as_str().to_owned(),
            });
        }
        if parsed.salt.is_none() || parsed.hash.is_none() {
            return Err(PassphraseError::IncompleteHash);
        }
        Ok(Self {
            encoded: encoded.to_owned(),
        })
    }

    /// The PHC string to keep in the store.
    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    /// Whether `typed` is the approval passphrase. Spacing, tabs, line breaks
    /// and letter case are ignored, as they are when typed at a prompt.
    pub fn matches(&self, typed: &str) -> bool {
        PasswordVerifier::<str>::verify_password(
            &Argon2::default(),
            normalise(typed).as_bytes(),
            self.encoded.as_str(),
        )
        .is_ok()
    }
}

/// Words are split on whitespace, lowercased and joined with single spaces.
fn normalise(passphrase: &str) -> String {
    passphrase
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A passphrase of [`GENERATED_WORDS`] words, each picked uniformly from the
/// built-in word list with the system's cryptographic random number generator.
pub fn generate() -> String {
    let words: Vec<&str> = WORDS
        .lines()
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect();
    let mut rng = rand::rng();
    (0..GENERATED_WORDS)
        .filter_map(|_| words.choose(&mut rng).copied())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The outcome of one typed try at the prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum Attempt {
    /// The passphrase was right; the lock and the count are cleared.
    Accepted,
    /// The passphrase was wrong; the prompt is still open.
    Wrong,
    /// The prompt is locked until `until`; the typed text was not checked.
    Locked {
        /// When the prompt opens again.
        until: SystemTime,
    },
}

/// Counts wrong tries at a machine's prompt and holds the lock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lockout {
    failures: u32,
    locked_until: Option<SystemTime>,
}

impl Lockout {
    /// Puts back a count and lock that were kept in the store.
    pub fn restore(failures: u32, locked_until: Option<SystemTime>) -> Self {
        Self {
            failures,
            locked_until,
        }
    }

    /// Wrong tries counted since the last right try or lock.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// When the current lock ends, if the prompt is locked.
    pub fn locked_until(&self) -> Option<SystemTime> {
        self.locked_until
    }

    /// Checks one typed try made at `now`.
    ///
    /// While the lock holds, the typed text is not checked. Once it has
    /// passed, both the lock and the count are cleared. The fifth wrong try in
    /// a row locks the prompt for fifteen minutes.
    pub fn attempt(&mut self, verifier: &Verifier, typed: &str, now: SystemTime) -> Attempt {
        if let Some(until) = self.locked_until {
            if now < until {
                return Attempt::Locked { until };
            }
            self.locked_until = None;
            self.failures = 0;
        }
        if verifier.matches(typed) {
            self.failures = 0;
            return Attempt::Accepted;
        }
        self.failures = self.failures.saturating_add(1);
        if self.failures < MAX_FAILURES {
            return Attempt::Wrong;
        }
        let until = lock_end(now);
        self.failures = 0;
        self.locked_until = Some(until);
        Attempt::Locked { until }
    }
}

/// `now` plus the lockout. Adding 15 minutes to a wall-clock reading only
/// fails at the platform's time limit, which no real clock reaches; if it ever
/// did, the lock would end at `now` rather than panic.
fn lock_end(now: SystemTime) -> SystemTime {
    now.checked_add(LOCKOUT).unwrap_or(now)
}

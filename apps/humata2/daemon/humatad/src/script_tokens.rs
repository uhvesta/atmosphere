//! Script tokens (SEC-14): a credential for a script, limited to the
//! capabilities and channels it was made with. The daemon keeps only the
//! hash of a token's text, in memory by hash and saved in the store, so a
//! restart keeps every token. The clear token is shown once, when made.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{Capability, ChannelId, ScriptToken};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SavedScriptToken, Store};
use prost::Message;

use crate::grant_target::{capability_of, timestamp};
use crate::identity;

/// How often a token's last use is saved to the store at most.
const SAVE_EVERY_MS: i64 = 60_000;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A token's holder: who the policy treats the call as.
#[derive(Clone, Debug)]
pub(crate) struct ScriptCaller {
    /// The token's id, `st_<hex>`.
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) token: ScriptToken,
}

/// Why a script token doesn't make a call a script's.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ScriptTokenUnusable {
    #[error(
        "its script token isn't one this daemon has: it was revoked, or another daemon made it; make a new one with `humata token create`"
    )]
    Unknown,
    #[error(
        "the script token {name:?} has expired; make a new one with `humata token create` and revoke this one"
    )]
    Expired { name: String },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ScriptTokensError {
    #[error("couldn't {doing} in the daemon's state; check that its disk has room and is writable")]
    Store {
        doing: &'static str,
        #[source]
        source: humata2_store::StoreError,
    },
    #[error(
        "the saved script token {id} can't be read; the state may be damaged. Restore the state from a backup, or revoke the token by removing it and make a new one"
    )]
    Entry {
        id: String,
        #[source]
        source: prost::DecodeError,
    },
}

struct Token {
    id: String,
    name: String,
    /// The capabilities as the API names them, for listing.
    shown: Vec<i32>,
    allowed: ScriptToken,
    expires_ms: i64,
    last_used_ms: i64,
    saved_ms: i64,
}

#[derive(Default)]
struct State {
    /// By the hash of the token's text.
    by_hash: HashMap<String, Token>,
    /// The replies already given, by idempotency key; never saved, as the
    /// reply holds the clear token.
    replies: HashMap<String, pb::CreateScriptTokenResponse>,
}

pub(crate) struct ScriptTokens {
    store: Arc<Store>,
    /// How long every token lasts instead of the days it asked for: a test
    /// daemon's `--script-token-seconds`. Release builds pass `None`.
    lifetime: Option<Duration>,
    state: Mutex<State>,
}

impl ScriptTokens {
    pub(crate) fn load(store: Arc<Store>, lifetime: Option<Duration>) -> Result<Self, ScriptTokensError> {
        let saved = store.script_tokens().map_err(|source| ScriptTokensError::Store {
            doing: "read the saved script tokens",
            source,
        })?;
        let mut state = State::default();
        for row in saved {
            let entry = pb::list_script_tokens_response::Entry::decode(row.entry.as_slice())
                .map_err(|source| ScriptTokensError::Entry {
                    id: row.id.clone(),
                    source,
                })?;
            let token = Token {
                allowed: allowed(&entry.capabilities, entry.channels.iter().map(|c| c.value.as_str())),
                shown: entry.capabilities,
                name: entry.name,
                id: row.id,
                expires_ms: row.expires_unix_ms,
                last_used_ms: row.last_used_unix_ms,
                saved_ms: row.last_used_unix_ms,
            };
            state.by_hash.insert(row.hash, token);
        }
        Ok(Self {
            store,
            lifetime,
            state: Mutex::new(state),
        })
    }

    /// The script `token` names, if it is live.
    pub(crate) fn caller(&self, token: &str) -> Result<ScriptCaller, ScriptTokenUnusable> {
        self.caller_at(token, now_ms())
    }

    fn caller_at(&self, token: &str, now_ms: i64) -> Result<ScriptCaller, ScriptTokenUnusable> {
        let hash = identity::digest(token);
        let (caller, save) = {
            let mut state = lock(&self.state);
            let Some(found) = state.by_hash.get_mut(&hash) else {
                return Err(ScriptTokenUnusable::Unknown);
            };
            if found.expires_ms <= now_ms {
                return Err(ScriptTokenUnusable::Expired {
                    name: found.name.clone(),
                });
            }
            found.last_used_ms = now_ms;
            let save = now_ms.saturating_sub(found.saved_ms) >= SAVE_EVERY_MS;
            if save {
                found.saved_ms = now_ms;
            }
            let caller = ScriptCaller {
                id: found.id.clone(),
                name: found.name.clone(),
                token: found.allowed.clone(),
            };
            (caller, save)
        };
        if save && let Err(error) = self.store.touch_script_token(&caller.id, now_ms) {
            crate::log::say!(
                "couldn't save when script token {} was last used: {}",
                caller.id,
                crate::status::chain(&error)
            );
        }
        Ok(caller)
    }

    /// Makes a token, or returns the reply already given for `key`.
    pub(crate) fn create(
        &self,
        name: &str,
        capabilities: &[Capability],
        channels: &[String],
        days: u32,
        key: Option<&str>,
    ) -> Result<pb::CreateScriptTokenResponse, ScriptTokensError> {
        self.create_at(name, capabilities, channels, days, key, now_ms())
    }

    fn create_at(
        &self,
        name: &str,
        capabilities: &[Capability],
        channels: &[String],
        days: u32,
        key: Option<&str>,
        now_ms: i64,
    ) -> Result<pb::CreateScriptTokenResponse, ScriptTokensError> {
        if let Some(key) = key
            && let Some(reply) = lock(&self.state).replies.get(key)
        {
            return Ok(reply.clone());
        }
        let lasts_ms = match self.lifetime {
            Some(lifetime) => i64::try_from(lifetime.as_millis()).unwrap_or(i64::MAX),
            None => i64::from(days) * DAY_MS,
        };
        let expires_ms = now_ms.saturating_add(lasts_ms);
        let shown: Vec<i32> = capabilities
            .iter()
            .map(|capability| crate::guard::capability(*capability) as i32)
            .collect();
        let entry = pb::list_script_tokens_response::Entry {
            name: name.to_owned(),
            capabilities: shown.clone(),
            channels: channels
                .iter()
                .map(|value| pb::ChannelId {
                    value: value.clone(),
                })
                .collect(),
            ..Default::default()
        };
        let (clear, hash) = identity::new_token();
        let id = format!("st_{}", uuid::Uuid::new_v4().simple());
        self.store
            .put_script_token(&SavedScriptToken {
                id: id.clone(),
                hash: hash.clone(),
                entry: entry.encode_to_vec(),
                expires_unix_ms: expires_ms,
                last_used_unix_ms: 0,
            })
            .map_err(|source| ScriptTokensError::Store {
                doing: "save the new script token",
                source,
            })?;
        let reply = pb::CreateScriptTokenResponse {
            token_id: id.clone(),
            token: clear,
            expires_at: Some(timestamp(at(expires_ms))),
        };
        let mut state = lock(&self.state);
        state.by_hash.insert(
            hash,
            Token {
                id,
                name: name.to_owned(),
                shown,
                allowed: allowed(&entry.capabilities, channels.iter().map(String::as_str)),
                expires_ms,
                last_used_ms: 0,
                saved_ms: 0,
            },
        );
        if let Some(key) = key {
            state.replies.insert(key.to_owned(), reply.clone());
        }
        Ok(reply)
    }

    /// Every token, expired ones too, by id.
    pub(crate) fn list(&self) -> Vec<pb::list_script_tokens_response::Entry> {
        let state = lock(&self.state);
        let mut entries: Vec<_> = state
            .by_hash
            .values()
            .map(|token| pb::list_script_tokens_response::Entry {
                token_id: token.id.clone(),
                name: token.name.clone(),
                capabilities: token.shown.clone(),
                channels: token
                    .allowed
                    .channels
                    .iter()
                    .map(|channel| pb::ChannelId {
                        value: channel.as_str().to_owned(),
                    })
                    .collect(),
                expires_at: Some(timestamp(at(token.expires_ms))),
                last_used_at: (token.last_used_ms > 0).then(|| timestamp(at(token.last_used_ms))),
            })
            .collect();
        entries.sort_by(|a, b| a.token_id.cmp(&b.token_id));
        entries
    }

    /// Revokes token `id`; false when there's none.
    pub(crate) fn revoke(&self, id: &str) -> Result<bool, ScriptTokensError> {
        let went = self
            .store
            .remove_script_token(id)
            .map_err(|source| ScriptTokensError::Store {
                doing: "revoke the script token",
                source,
            })?;
        let mut state = lock(&self.state);
        let before = state.by_hash.len();
        state.by_hash.retain(|_, token| token.id != id);
        Ok(went || state.by_hash.len() != before)
    }
}

/// What a token's saved capabilities and channels let it do. A capability
/// this build no longer knows is dropped: fewer rights, never more.
fn allowed<'a>(capabilities: &[i32], channels: impl Iterator<Item = &'a str>) -> ScriptToken {
    ScriptToken {
        capabilities: capabilities
            .iter()
            .filter_map(|value| capability_of(*value))
            .collect::<BTreeSet<_>>(),
        channels: channels.map(ChannelId::new).collect(),
    }
}

fn at(unix_ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(u64::try_from(unix_ms).unwrap_or(0))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000_000;

    fn open(dir: &std::path::Path) -> (Arc<Store>, ScriptTokens) {
        let store = Arc::new(Store::open(dir).unwrap());
        let tokens = ScriptTokens::load(store.clone(), None).unwrap();
        (store, tokens)
    }

    fn make(tokens: &ScriptTokens, key: Option<&str>) -> pb::CreateScriptTokenResponse {
        tokens
            .create_at(
                "nightly",
                &[Capability::ChannelRead, Capability::GitPush],
                &["ch_a".to_owned(), "ch_b".to_owned()],
                30,
                key,
                NOW,
            )
            .unwrap()
    }

    #[test]
    fn a_made_token_is_found_with_exactly_what_it_was_made_with() {
        let dir = tempfile::tempdir().unwrap();
        let (_, tokens) = open(dir.path());
        let made = make(&tokens, None);

        let caller = tokens.caller_at(&made.token, NOW + 1).unwrap();

        assert_eq!(caller.id, made.token_id);
        assert_eq!(caller.name, "nightly");
        assert_eq!(
            caller.token.capabilities,
            BTreeSet::from([Capability::ChannelRead, Capability::GitPush])
        );
        assert_eq!(
            caller.token.channels,
            BTreeSet::from([ChannelId::new("ch_a"), ChannelId::new("ch_b")])
        );
    }

    #[test]
    fn a_made_up_token_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let (_, tokens) = open(dir.path());
        make(&tokens, None);

        assert!(matches!(
            tokens.caller_at("0000", NOW),
            Err(ScriptTokenUnusable::Unknown)
        ));
    }

    #[test]
    fn an_expired_token_is_refused_but_stays_listed() {
        let dir = tempfile::tempdir().unwrap();
        let (_, tokens) = open(dir.path());
        let made = make(&tokens, None);

        assert!(matches!(
            tokens.caller_at(&made.token, NOW + 30 * DAY_MS),
            Err(ScriptTokenUnusable::Expired { .. })
        ));
        assert_eq!(tokens.list().len(), 1);
    }

    #[test]
    fn a_revoked_token_is_refused_and_revoking_again_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (store, tokens) = open(dir.path());
        let made = make(&tokens, None);

        assert!(tokens.revoke(&made.token_id).unwrap());

        assert!(matches!(
            tokens.caller_at(&made.token, NOW + 1),
            Err(ScriptTokenUnusable::Unknown)
        ));
        assert!(!tokens.revoke(&made.token_id).unwrap());
        assert!(store.script_tokens().unwrap().is_empty());
    }

    #[test]
    fn a_token_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let made = {
            let (_, tokens) = open(dir.path());
            make(&tokens, None)
        };

        let (_, tokens) = open(dir.path());

        let caller = tokens.caller_at(&made.token, NOW + 1).unwrap();
        assert_eq!(caller.id, made.token_id);
        assert_eq!(tokens.list()[0].capabilities.len(), 2);
    }

    #[test]
    fn the_clear_token_is_never_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (store, tokens) = open(dir.path());
        let made = make(&tokens, None);

        let saved = store.script_tokens().unwrap();

        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].hash, identity::digest(&made.token));
        assert_ne!(saved[0].hash, made.token);
        assert!(
            !saved[0]
                .entry
                .windows(made.token.len())
                .any(|window| window == made.token.as_bytes())
        );
    }

    #[test]
    fn the_same_idempotency_key_gives_the_same_token() {
        let dir = tempfile::tempdir().unwrap();
        let (_, tokens) = open(dir.path());

        let first = make(&tokens, Some("k1"));
        let again = make(&tokens, Some("k1"));
        let other = make(&tokens, Some("k2"));

        assert_eq!(first, again);
        assert_ne!(first.token_id, other.token_id);
        assert_eq!(tokens.list().len(), 2);
    }

    #[test]
    fn a_test_lifetime_overrides_the_days() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let tokens = ScriptTokens::load(store, Some(Duration::from_secs(5))).unwrap();
        let made = make(&tokens, None);

        assert!(tokens.caller_at(&made.token, NOW + 4_000).is_ok());
        assert!(matches!(
            tokens.caller_at(&made.token, NOW + 5_000),
            Err(ScriptTokenUnusable::Expired { .. })
        ));
    }

    #[test]
    fn last_use_is_listed_once_the_token_was_used() {
        let dir = tempfile::tempdir().unwrap();
        let (store, tokens) = open(dir.path());
        let made = make(&tokens, None);
        assert!(tokens.list()[0].last_used_at.is_none());

        tokens.caller_at(&made.token, NOW + 120_000).unwrap();

        assert!(tokens.list()[0].last_used_at.is_some());
        assert_eq!(store.script_tokens().unwrap()[0].last_used_unix_ms, NOW + 120_000);
    }
}

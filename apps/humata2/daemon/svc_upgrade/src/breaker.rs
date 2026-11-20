//! Anti-thrash failure circuit breaker for automatic migrations.
//!
//! Persisted at `<state>/upgrade/failed.json` keyed by `(machine, to_bundle)`.
//! When a migration to `to_bundle` on `machine` fails, stalls, or rolls back
//! (`ROLL_BACK_AT_BOOTS`), the pair is recorded here so automatic migration
//! does not retry-loop on that same `to_bundle`.
//!
//! The breaker does not block:
//! - a newer target bundle (`ours.bundle` advancing to a different `to_bundle`),
//! - an explicit manual retry/approve (which clears `(machine, to_bundle)`), or
//! - a machine that reaches `to_bundle` or is removed (which clears its entries).

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const FORMAT: u64 = 1;
const FAILED_FILE: &str = "failed.json";

/// One recorded failure for `(machine, to_bundle)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureEntry {
    pub machine: String,
    pub to_bundle: String,
    pub reason: String,
    pub failed_unix_ms: i64,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    format: u64,
    entries: Vec<FailureEntry>,
}

struct Inner {
    path: Option<PathBuf>,
    entries: Mutex<HashMap<(String, String), FailureEntry>>,
}

/// Persisted circuit breaker preventing automatic retry loops on a failing
/// `(machine, to_bundle)` pair.
#[derive(Clone)]
pub struct FailedBreaker {
    inner: Arc<Inner>,
}

impl FailedBreaker {
    /// Opens `<state>/upgrade/failed.json`, loading any previously recorded
    /// failures. A missing or unreadable file starts empty (logged, never fatal).
    pub fn open(state: &Path) -> Self {
        let path = state.join("upgrade").join(FAILED_FILE);
        let entries = load_entries(&path);
        Self {
            inner: Arc::new(Inner {
                path: Some(path),
                entries: Mutex::new(entries),
            }),
        }
    }

    /// An in-memory breaker not backed by disk (for unit tests without a state folder).
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(Inner {
                path: None,
                entries: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Whether automatic migration to `to_bundle` on `machine` is currently blocked.
    pub fn is_tripped(&self, machine: &str, to_bundle: &str) -> bool {
        if machine.is_empty() || to_bundle.is_empty() {
            return false;
        }
        self.inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&(machine.to_owned(), to_bundle.to_owned()))
    }

    /// Records a failure for `(machine, to_bundle)` and persists `failed.json`.
    pub fn trip(&self, machine: &str, to_bundle: &str, reason: impl Into<String>) {
        if machine.is_empty() || to_bundle.is_empty() {
            return;
        }
        let mut guard = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.insert(
            (machine.to_owned(), to_bundle.to_owned()),
            FailureEntry {
                machine: machine.to_owned(),
                to_bundle: to_bundle.to_owned(),
                reason: reason.into(),
                failed_unix_ms: now_unix_ms(),
            },
        );
        self.save(&guard);
    }

    /// Clears a recorded failure for `(machine, to_bundle)` (e.g. on manual
    /// retry or once the machine arrives at `to_bundle`).
    pub fn clear(&self, machine: &str, to_bundle: &str) {
        if machine.is_empty() || to_bundle.is_empty() {
            return;
        }
        let mut guard = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if guard
            .remove(&(machine.to_owned(), to_bundle.to_owned()))
            .is_some()
        {
            self.save(&guard);
        }
    }

    /// Clears all recorded failures for `machine` (e.g. when removed).
    pub fn clear_machine(&self, machine: &str) {
        if machine.is_empty() {
            return;
        }
        let mut guard = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = guard.len();
        guard.retain(|(m, _), _| m != machine);
        if guard.len() != before {
            self.save(&guard);
        }
    }

    /// Snapshot of currently tripped entries, sorted by `(machine, to_bundle)`.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<FailureEntry> {
        let guard = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut list: Vec<FailureEntry> = guard.values().cloned().collect();
        list.sort_by(|a, b| (&a.machine, &a.to_bundle).cmp(&(&b.machine, &b.to_bundle)));
        list
    }

    fn save(&self, entries: &HashMap<(String, String), FailureEntry>) {
        let Some(path) = &self.inner.path else {
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(error) = humata2_durable::create_dir_all(dir, 0o700)
        {
            humata2_rpc_kit::log::say!(
                "humatad2: warning: couldn't create {}: {error}",
                dir.display()
            );
            return;
        }
        let mut list: Vec<FailureEntry> = entries.values().cloned().collect();
        list.sort_by(|a, b| (&a.machine, &a.to_bundle).cmp(&(&b.machine, &b.to_bundle)));
        let stored = Stored {
            format: FORMAT,
            entries: list,
        };
        match serde_json::to_vec_pretty(&stored) {
            Ok(bytes) => {
                if let Err(error) = humata2_store::write_atomic(path, &bytes) {
                    humata2_rpc_kit::log::say!(
                        "humatad2: warning: couldn't save {}: {}",
                        path.display(),
                        humata2_rpc_kit::helpers::chain(&error)
                    );
                }
            }
            Err(error) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: warning: couldn't encode {}: {error}",
                    path.display()
                );
            }
        }
    }
}

fn load_entries(path: &Path) -> HashMap<(String, String), FailureEntry> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return HashMap::new(),
        Err(source) => {
            humata2_rpc_kit::log::say!(
                "humatad2: warning: couldn't read {}: {source}",
                path.display()
            );
            return HashMap::new();
        }
    };
    let stored: Stored = match serde_json::from_slice(&bytes) {
        Ok(stored) => stored,
        Err(source) => {
            humata2_rpc_kit::log::say!(
                "humatad2: warning: couldn't parse {}: {source}",
                path.display()
            );
            return HashMap::new();
        }
    };
    if stored.format != FORMAT {
        humata2_rpc_kit::log::say!(
            "humatad2: warning: {} has unsupported format {}; ignoring",
            path.display(),
            stored.format
        );
        return HashMap::new();
    }
    stored
        .entries
        .into_iter()
        .filter(|e| !e.machine.is_empty() && !e.to_bundle.is_empty())
        .map(|e| ((e.machine.clone(), e.to_bundle.clone()), e))
        .collect()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tripped_entry_persists_across_reopen_and_clears_on_advance_or_explicit_clear() {
        let state = tempfile::tempdir().unwrap();
        let breaker = FailedBreaker::open(state.path());
        assert!(!breaker.is_tripped("gpu", "bundle_b"));

        breaker.trip("gpu", "bundle_b", "rolled back after 2 boots");
        assert!(breaker.is_tripped("gpu", "bundle_b"));
        assert!(!breaker.is_tripped("gpu", "bundle_c"), "newer bundle is not tripped");
        assert!(!breaker.is_tripped("tpu", "bundle_b"), "other machine is not tripped");
        assert!(state.path().join("upgrade/failed.json").is_file());

        // Reload from disk.
        let reloaded = FailedBreaker::open(state.path());
        assert!(reloaded.is_tripped("gpu", "bundle_b"));
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].reason, "rolled back after 2 boots");

        // Explicit clear persists.
        reloaded.clear("gpu", "bundle_b");
        assert!(!reloaded.is_tripped("gpu", "bundle_b"));
        let after_clear = FailedBreaker::open(state.path());
        assert!(!after_clear.is_tripped("gpu", "bundle_b"));
    }

    #[test]
    fn clear_machine_removes_only_that_machines_entries() {
        let state = tempfile::tempdir().unwrap();
        let breaker = FailedBreaker::open(state.path());
        breaker.trip("gpu", "b1", "err1");
        breaker.trip("gpu", "b2", "err2");
        breaker.trip("tpu", "b1", "err3");

        breaker.clear_machine("gpu");

        let reloaded = FailedBreaker::open(state.path());
        assert!(!reloaded.is_tripped("gpu", "b1"));
        assert!(!reloaded.is_tripped("gpu", "b2"));
        assert!(reloaded.is_tripped("tpu", "b1"));
    }
}

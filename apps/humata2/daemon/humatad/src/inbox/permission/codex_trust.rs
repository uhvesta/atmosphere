//! Codex's trust step (D202): Codex runs Humata's hooks only once the user
//! has trusted them in its `/hooks`, and keys that trust on the hook's
//! hash. The daemon keeps the fingerprint of the `-c hooks=…` value it last
//! heard a hook from, and Settings › Agents shows the step while some kind
//! carries a value it hasn't heard from.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use humata2_sessions::Agents;
use humata2_store::{Store, StoreError};
use sha2::{Digest, Sha256};

/// The store key holding the fingerprint of the hooks last heard from.
const HEARD_KEY: &str = "agents.codex_hooks.heard";

/// The hooks this daemon gives Codex, and the kinds that carry them.
struct Trust {
    fingerprint: String,
    kinds: HashSet<String>,
    /// Set once this fingerprint is saved as heard, so a turn report
    /// writes the store at most once.
    saved: Mutex<bool>,
}

static HOOKS: OnceLock<String> = OnceLock::new();
static TRUST: OnceLock<Trust> = OnceLock::new();

/// Remembers the `-c hooks=…` value Codex is given (at the daemon's start).
pub(crate) fn remember_hooks(codex: &str) {
    let _ = HOOKS.set(codex.to_owned());
}

/// Notes which kinds carry the remembered hooks. Once: a second call
/// changes nothing.
pub(crate) fn set_kinds(agents: &Agents) {
    let Some(codex) = HOOKS.get() else {
        return;
    };
    let _ = TRUST.set(Trust {
        fingerprint: fingerprint(codex),
        kinds: carrying(agents, codex),
        saved: Mutex::new(false),
    });
}

/// A hook ran in a session of `kind`: if the kind carries Codex's hooks,
/// they are trusted as they are now.
pub(crate) fn heard(store: &Store, kind: &str) -> Result<(), StoreError> {
    let Some(trust) = TRUST.get() else {
        return Ok(());
    };
    if !trust.kinds.contains(kind) {
        return Ok(());
    }
    let mut saved = match trust.saved.lock() {
        Ok(saved) => saved,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *saved {
        return Ok(());
    }
    save_heard(store, &trust.fingerprint)?;
    *saved = true;
    Ok(())
}

/// Whether Settings › Agents shows Codex's trust step.
pub(crate) fn needs_trust(store: &Store) -> Result<bool, StoreError> {
    match TRUST.get() {
        Some(trust) if !trust.kinds.is_empty() => untrusted(store, &trust.fingerprint),
        _ => Ok(false),
    }
}

fn fingerprint(codex: &str) -> String {
    Sha256::digest(codex.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The kinds whose args carry `codex`. A value that is only the argument
/// right after `--hooks` is the `humata codex` wrapper's own flag, not a
/// hook Codex runs, so it doesn't count.
fn carrying(agents: &Agents, codex: &str) -> HashSet<String> {
    agents
        .kinds()
        .iter()
        .filter(|kind| carries(&kind.start_args, codex) || carries(&kind.resume_args, codex))
        .map(|kind| kind.name.clone())
        .collect()
}

/// Whether `args` hand Codex `codex` anywhere but as the wrapper's `--hooks`
/// value.
fn carries(args: &[String], codex: &str) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        let wrapper_flag = index > 0 && args[index - 1] == "--hooks" && arg == codex;
        arg.contains(codex) && !wrapper_flag
    })
}

fn save_heard(store: &Store, fingerprint: &str) -> Result<(), StoreError> {
    store.put_setting(HEARD_KEY, fingerprint, 0)
}

/// Whether the hooks last heard from differ from `fingerprint`.
fn untrusted(store: &Store, fingerprint: &str) -> Result<bool, StoreError> {
    Ok(store
        .setting(HEARD_KEY)?
        .is_none_or(|heard| heard.value != fingerprint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &tempfile::TempDir) -> Store {
        Store::open(dir.path().join("store")).expect("a fresh store")
    }

    #[test]
    fn hooks_not_heard_need_trust_until_they_are() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(&dir);
        let now = fingerprint("hooks={ Stop = [] }");
        assert!(untrusted(&store, &now).expect("read"));
        save_heard(&store, &now).expect("saved");
        assert!(!untrusted(&store, &now).expect("read"));
    }

    #[test]
    fn a_changed_command_needs_trust_again() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(&dir);
        save_heard(&store, &fingerprint("hooks={ Stop = [] }")).expect("saved");
        let changed = fingerprint("hooks={ Stop = [], Interrupt = [] }");
        assert!(untrusted(&store, &changed).expect("read"));
    }

    #[test]
    fn only_kinds_that_carry_the_hooks_count() {
        let agents = Agents::from_json(
            br#"[
                {"name":"codex","program":"codex","start_args":["-c","hooks={codex_hooks}"],"resume_args":[],"conversation_file":"x"},
                {"name":"plain","program":"codex","start_args":[],"resume_args":[],"conversation_file":"x"}
            ]"#,
        )
        .expect("kinds")
        .with_agent_hooks("H", std::path::Path::new("/p"));
        let kinds = carrying(&agents, "H");
        assert!(kinds.contains("codex"));
        assert!(!kinds.contains("plain"));
    }

    #[test]
    fn the_wrapped_codex_needs_no_trust_step() {
        // `humata codex` reads Codex's app-server (D795) and hands plain
        // Codex the hooks only when that fails, so its kind shows no step.
        let agents = Agents::from_json(
            br#"[
                {"name":"codex","program":"/usr/bin/env","start_args":["{humata}","codex","--hooks","{codex_hooks}","--"],"resume_args":[],"conversation_file":"x"},
                {"name":"mine","program":"codex","start_args":["-c","hooks={codex_hooks}"],"resume_args":[],"conversation_file":"x"}
            ]"#,
        )
        .expect("kinds")
        .with_agent_hooks("H", std::path::Path::new("/p"));
        let kinds = carrying(&agents, "H");
        assert!(!kinds.contains("codex"));
        assert!(kinds.contains("mine"));
    }
}

//! Flat multi-agent tab cap enforcement for regular sessions (ATM-65).
//!
//! Enforces the same flat cap (`enabled_kinds + 2`, max 2 per kind including
//! plain `"shell"`, `"zsh"`, and `"terminal"`) as channel Desk sessions.

use humata2_rpc_kit::failure;
use humata2_store::{Store, TabKind};
use humata2_svc_sessions_common::pb;
use tonic::{Code, Status};

/// Maximum number of open tabs allowed for any single agent/shell kind.
pub const MAX_TABS_PER_KIND: usize = 2;

/// Total open tab cap for a session given the number of enabled kinds.
pub const fn flat_tab_cap(enabled_kinds: usize) -> usize {
    enabled_kinds + 2
}

/// Refusal returned by [`check_tab_cap`] when opening another tab would exceed
/// either the per-kind cap or the total flat tab cap.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TabCapError {
    #[error("At most {limit} {kind} tabs can be open at once. Close an existing {kind} tab first.")]
    KindLimitReached { kind: String, limit: usize },
    #[error("All {limit} tabs are open ({open} of {limit}). Close an open tab before opening another.")]
    TotalLimitReached { open: usize, limit: usize },
}

/// Checks whether opening a new tab of `next_kind` is allowed given the
/// currently open tab kinds and the number of enabled kinds.
///
/// Kind matching is trimmed and ASCII case-insensitive. Every kind, including
/// plain `"shell"` / `"zsh"` / `"terminal"`, is capped at [`MAX_TABS_PER_KIND`].
pub fn check_tab_cap(
    open_kinds: &[&str],
    next_kind: &str,
    enabled_kinds: usize,
) -> Result<(), TabCapError> {
    let trimmed = next_kind.trim();
    let normalized = trimmed.to_ascii_lowercase();

    let same_kind_count = open_kinds
        .iter()
        .filter(|kind| kind.trim().to_ascii_lowercase() == normalized)
        .count();
    if same_kind_count >= MAX_TABS_PER_KIND {
        return Err(TabCapError::KindLimitReached {
            kind: trimmed.to_owned(),
            limit: MAX_TABS_PER_KIND,
        });
    }

    let limit = flat_tab_cap(enabled_kinds);
    if open_kinds.len() >= limit {
        return Err(TabCapError::TotalLimitReached {
            open: open_kinds.len(),
            limit,
        });
    }

    Ok(())
}

/// Checks whether a regular session with `open_kinds` currently open may open
/// another tab of `next_kind`, given `enabled_kinds` enabled agent kinds.
///
/// Maps [`TabCapError`] to `Code::FailedPrecondition` via
/// [`humata2_rpc_kit::failure`].
pub fn check_regular_session_tab_cap(
    open_kinds: &[&str],
    next_kind: &str,
    enabled_kinds: usize,
) -> Result<(), Status> {
    check_tab_cap(open_kinds, next_kind, enabled_kinds).map_err(|err| {
        failure(
            Code::FailedPrecondition,
            pb::Error {
                message: err.to_string(),
                fix: "close an open tab in this session before opening another".to_owned(),
                retry: None,
                kind: None,
            },
        )
    })
}

/// Checks the flat multi-agent tab cap for `session_id` directly from `store`.
///
/// Counts tab 0 as `session_agent`, plus every open [`TabKind::Shell`] tab
/// belonging to `session_id` (resolving its kind from setting
/// `tab_agent:<tab_id>`, defaulting to `"shell"` when absent), then delegates
/// to [`check_regular_session_tab_cap`].
pub fn check_session_store_tab_cap(
    store: &Store,
    session_id: &str,
    session_agent: &str,
    next_kind: &str,
    enabled_kinds: usize,
) -> Result<(), Status> {
    let tabs = store.tabs().map_err(|err| {
        failure(
            Code::Internal,
            pb::Error {
                message: format!("failed to list open tabs for session {session_id}: {err}"),
                fix: "retry opening the tab".to_owned(),
                retry: None,
                kind: None,
            },
        )
    })?;

    let mut owned_kinds: Vec<String> = Vec::with_capacity(tabs.len() + 1);
    owned_kinds.push(session_agent.to_owned());

    for tab in tabs {
        if tab.kind != TabKind::Shell || tab.session.as_deref() != Some(session_id) {
            continue;
        }
        let key = format!("tab_agent:{}", tab.id);
        let kind = store
            .setting(&key)
            .map_err(|err| {
                failure(
                    Code::Internal,
                    pb::Error {
                        message: format!(
                            "failed to read tab agent setting for tab {}: {err}",
                            tab.id
                        ),
                        fix: "retry opening the tab".to_owned(),
                        retry: None,
                        kind: None,
                    },
                )
            })?
            .map(|saved| saved.value)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "shell".to_owned());
        owned_kinds.push(kind);
    }

    let open_refs: Vec<&str> = owned_kinds.iter().map(String::as_str).collect();
    check_regular_session_tab_cap(&open_refs, next_kind, enabled_kinds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_store::{SessionRecord, TabRecord};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "humata2_svc_sessions_local_tab_cap_{}_{}",
                std::process::id(),
                seq
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn put_test_session(store: &Store, id: &str, agent: &str) {
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: "ch_1".to_owned(),
                machine: "m_1".to_owned(),
                title: id.to_owned(),
                agent: agent.to_owned(),
                folder: PathBuf::from(format!("/tmp/{id}")),
                created_unix_ms: 1,
            })
            .unwrap();
    }

    fn put_test_tab(store: &Store, id: &str, session: Option<&str>, kind: TabKind) {
        store
            .put_tab(&TabRecord {
                id: id.to_owned(),
                session: session.map(str::to_owned),
                kind,
                title: id.to_owned(),
                holder_socket: PathBuf::from(format!("/tmp/{id}.sock")),
                holder_pid: 1,
                created_unix_ms: 1,
                holder_generation: 0,
            })
            .unwrap();
    }

    #[test]
    fn per_kind_cap_limits_agents_and_shells_to_two() {
        for kind in ["claude", "codex", "shell", "zsh", "terminal"] {
            assert!(check_regular_session_tab_cap(&[], kind, 4).is_ok());
            assert!(check_regular_session_tab_cap(&[kind], kind, 4).is_ok());
            let err = check_regular_session_tab_cap(&[kind, kind], kind, 4).unwrap_err();
            assert_eq!(err.code(), Code::FailedPrecondition);
            assert!(
                err.message().contains(&format!("At most 2 {kind} tabs")),
                "unexpected message: {}",
                err.message()
            );
        }
    }

    #[test]
    fn flat_cap_enforces_enabled_kinds_plus_two() {
        // 1 enabled kind -> flat cap 3
        assert!(check_regular_session_tab_cap(&["claude", "claude"], "shell", 1).is_ok());
        let err =
            check_regular_session_tab_cap(&["claude", "claude", "shell"], "shell", 1).unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(
            err.message().contains("All 3 tabs are open (3 of 3)"),
            "unexpected message: {}",
            err.message()
        );

        // 4 enabled kinds -> flat cap 6
        let open_six = ["claude", "claude", "codex", "codex", "shell", "shell"];
        let err = check_regular_session_tab_cap(&open_six, "opencode", 4).unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(
            err.message().contains("All 6 tabs are open (6 of 6)"),
            "unexpected message: {}",
            err.message()
        );
    }

    #[test]
    fn store_backed_tab_cap_counts_tab_zero_and_shell_tabs_with_agent_settings() {
        let dir = TempDir::new();
        let store = Store::open(&dir.0).unwrap();

        put_test_session(&store, "se_1", "claude");
        put_test_session(&store, "se_other", "claude");

        // Agent tab (tab 0) is represented by `session_agent`; TabKind::Agent rows are not double-counted.
        put_test_tab(&store, "tb_agent", Some("se_1"), TabKind::Agent);
        // Shell tab with explicit tab_agent setting -> counts as a 2nd "claude" tab.
        put_test_tab(&store, "tb_sh1", Some("se_1"), TabKind::Shell);
        store.put_setting("tab_agent:tb_sh1", "claude", 1).unwrap();
        // Shell tab in another session and a machine shell -> ignored.
        put_test_tab(&store, "tb_other", Some("se_other"), TabKind::Shell);
        put_test_tab(&store, "tb_machine", None, TabKind::Shell);

        // 2 claude tabs open (tab 0 + tb_sh1) -> 3rd claude is rejected by per-kind cap.
        let err = check_session_store_tab_cap(&store, "se_1", "claude", "claude", 1).unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(err.message().contains("At most 2 claude tabs"));

        // Plain shell tab is allowed (2 open < flat cap 3 for 1 enabled kind).
        assert!(check_session_store_tab_cap(&store, "se_1", "claude", "shell", 1).is_ok());

        // Add a plain shell tab (no tab_agent setting -> defaults to "shell").
        put_test_tab(&store, "tb_sh2", Some("se_1"), TabKind::Shell);

        // Now 3 tabs are open (claude, claude, shell) -> 4th tab rejected by flat cap 3.
        let err = check_session_store_tab_cap(&store, "se_1", "claude", "codex", 1).unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(err.message().contains("All 3 tabs are open (3 of 3)"));

        // With 2 enabled kinds (flat cap 4), a 2nd plain shell tab is allowed,
        // and a 3rd plain shell tab is rejected by the per-kind cap of 2.
        assert!(check_session_store_tab_cap(&store, "se_1", "claude", "shell", 2).is_ok());
        put_test_tab(&store, "tb_sh3", Some("se_1"), TabKind::Shell);
        let err = check_session_store_tab_cap(&store, "se_1", "claude", "shell", 3).unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(err.message().contains("At most 2 shell tabs"));
    }
}

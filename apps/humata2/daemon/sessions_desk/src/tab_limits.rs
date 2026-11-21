//! Flat multi-agent tab cap for channel Desk sessions.
//!
//! Each kind (including plain shells like `"shell"`, `"zsh"`, or `"terminal"`)
//! is capped at [`MAX_TABS_PER_KIND`] concurrent tabs, and the total open tabs
//! across all kinds are capped at [`flat_tab_cap`].

/// Maximum number of open tabs allowed for any single agent/shell kind.
pub const MAX_TABS_PER_KIND: usize = 2;

/// Total open tab cap for a channel Desk given the number of enabled kinds.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_tab_cap_scales_with_enabled_kinds() {
        assert_eq!(flat_tab_cap(1), 3);
        assert_eq!(flat_tab_cap(4), 6);
    }

    #[test]
    fn per_kind_limit_caps_agents_and_shells_at_two() {
        for kind in ["claude", "codex", "shell", "zsh", "terminal"] {
            assert_eq!(check_tab_cap(&[], kind, 4), Ok(()));
            assert_eq!(check_tab_cap(&[kind], kind, 4), Ok(()));
            assert_eq!(
                check_tab_cap(&[kind, kind], kind, 4),
                Err(TabCapError::KindLimitReached {
                    kind: kind.to_owned(),
                    limit: MAX_TABS_PER_KIND,
                })
            );
        }
    }

    #[test]
    fn per_kind_comparison_is_case_insensitive_and_trimmed() {
        let err = check_tab_cap(&["Claude", " claude "], " CLAUDE ", 4).unwrap_err();
        assert_eq!(
            err,
            TabCapError::KindLimitReached {
                kind: "CLAUDE".to_owned(),
                limit: 2,
            }
        );
    }

    #[test]
    fn total_cap_rejects_when_flat_limit_is_reached() {
        // 1 enabled kind -> cap 3
        let open = ["claude", "claude", "shell"];
        let err = check_tab_cap(&open, "shell", 1).unwrap_err();
        assert_eq!(err, TabCapError::TotalLimitReached { open: 3, limit: 3 });

        // 4 enabled kinds -> cap 6
        let open_six = ["claude", "claude", "codex", "codex", "shell", "shell"];
        let err = check_tab_cap(&open_six, "opencode", 4).unwrap_err();
        assert_eq!(err, TabCapError::TotalLimitReached { open: 6, limit: 6 });
    }
}

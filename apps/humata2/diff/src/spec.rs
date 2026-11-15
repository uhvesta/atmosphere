//! What a repo's diff compares (RV-31): the base, how much of the working
//! tree is included, path filters, and whether whitespace and renames count.
//!
//! A [`DiffSpec`] is plain data shared by the user's base picker, an
//! agent's `review_prepare` and the skills that write one. The diff view
//! shows it in each repo's header ("Comparing: origin/main...HEAD +
//! unstaged ▾"), lets the user edit it, and reports the edit as
//! [`crate::DiffEvent::SpecChanged`]; whoever computes diffs (the daemon)
//! answers with a new [`crate::Review`].

use std::fmt;

/// What a repo's changes are compared against.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    /// The merge base of `HEAD` with a branch, as GitHub shows a pull
    /// request: `origin/main...HEAD`.
    MergeBase { with: String },
    /// A branch, tag or commit directly: `main`, `abc1234`.
    Ref(String),
    /// A snapshot stored when the user marked the session reviewed
    /// (RV-28, RV-30). `label` is how the user knows it, such as
    /// `review 3, Tuesday 14:02`.
    Reviewed { number: u32, label: String },
    /// Where the session forked from its parent session.
    ForkPoint { parent: String },
}

impl Default for Base {
    fn default() -> Self {
        Self::MergeBase {
            with: "origin/main".to_owned(),
        }
    }
}

impl fmt::Display for Base {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MergeBase { with } => write!(f, "{with}...HEAD"),
            Self::Ref(name) => f.write_str(name),
            Self::Reviewed { label, .. } => write!(f, "since {label}"),
            Self::ForkPoint { parent } => write!(f, "fork point of {parent}"),
        }
    }
}

/// How much beyond the commits a diff includes. Each level includes the
/// ones before it: unstaged changes include staged ones.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Include {
    /// Committed changes only.
    Commits,
    /// Plus what is staged.
    Staged,
    /// Plus the working tree's unstaged changes.
    #[default]
    Unstaged,
    /// Plus untracked files: all of them when `paths` is empty, otherwise
    /// only those under the given paths.
    Untracked { paths: Vec<String> },
}

impl Include {
    /// The levels the spec editor offers, with untracked meaning all.
    pub const LEVELS: [Self; 4] = [
        Self::Commits,
        Self::Staged,
        Self::Unstaged,
        Self::Untracked { paths: Vec::new() },
    ];

    /// The word the spec editor shows for the level.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Commits => "Commits only",
            Self::Staged => "+ staged",
            Self::Unstaged => "+ unstaged",
            Self::Untracked { .. } => "+ untracked",
        }
    }

    /// Whether `self` is the same level as `other`, whatever their paths.
    #[must_use]
    pub fn same_level(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// What one repo's diff shows (RV-31).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffSpec {
    pub base: Base,
    pub include: Include,
    /// Only paths under these (repo-relative, `/`-separated); empty means
    /// every path.
    pub paths: Vec<String>,
    /// Whether changes in whitespace alone are left out (`git diff -w`).
    pub ignore_whitespace: bool,
    /// Whether moved files show as renames rather than a deletion and an
    /// addition.
    pub detect_renames: bool,
}

impl Default for DiffSpec {
    fn default() -> Self {
        Self {
            base: Base::default(),
            include: Include::default(),
            paths: Vec::new(),
            ignore_whitespace: false,
            detect_renames: true,
        }
    }
}

impl DiffSpec {
    /// A spec against `base` with the defaults: unstaged changes included,
    /// every path, whitespace and renames counted.
    #[must_use]
    pub fn against(base: Base) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }
}

/// The compact form the repo header shows after "Comparing:", such as
/// `origin/main...HEAD + unstaged · only migrations/`.
impl fmt::Display for DiffSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.base)?;
        match &self.include {
            Include::Commits => {}
            Include::Staged => f.write_str(" + staged")?,
            Include::Unstaged => f.write_str(" + unstaged")?,
            Include::Untracked { paths } if paths.is_empty() => {
                f.write_str(" + unstaged + untracked")?;
            }
            Include::Untracked { paths } => {
                write!(f, " + unstaged + untracked in {}", paths.join(", "))?;
            }
        }
        if !self.paths.is_empty() {
            write!(f, " · only {}", self.paths.join(", "))?;
        }
        if self.ignore_whitespace {
            f.write_str(" · ignoring whitespace")?;
        }
        if !self.detect_renames {
            f.write_str(" · no renames")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_spec_reads_as_github_shows_a_branch_plus_unstaged_changes() {
        let spec = DiffSpec::default();
        assert_eq!(spec.base, Base::MergeBase { with: "origin/main".to_owned() });
        assert_eq!(spec.include, Include::Unstaged);
        assert!(spec.detect_renames);
        assert!(!spec.ignore_whitespace);
        assert!(spec.paths.is_empty());
    }

    #[test]
    fn levels_compare_by_kind_not_paths() {
        let some = Include::Untracked {
            paths: vec!["migrations/".to_owned()],
        };
        assert!(some.same_level(&Include::Untracked { paths: Vec::new() }));
        assert!(!some.same_level(&Include::Unstaged));
    }
}

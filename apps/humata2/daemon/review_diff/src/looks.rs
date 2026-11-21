//! Looks good (D852): which files of a session the person looked at, each
//! with the fingerprint of the diff it showed. Kept next to the reviewed
//! bookmark: one blob under `refs/humata/looks/<session>` per repo, so the
//! ref moves with the clone and is replaced in one atomic `update-ref`.

use serde_json::{Value, json};

use crate::checkout::Checkout;
use crate::error::DiffError;
use crate::git::trimmed;
use crate::spec::valid_session;
use crate::stack::{REBASED_DIFF_UNCHANGED_NOTE, RestackStatus};

const LOOKS: &str = "refs/humata/looks";

/// One file the person looked at: its path in the repo and the fingerprint
/// of the diff that was shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub path: String,
    pub fingerprint: u64,
}

/// A [`Look`] reconciled against a layer's [`RestackStatus`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerLook {
    pub look: Look,
    /// `true` when the layer was rebased without changing its diff, so
    /// `Looked at` is preserved with a quiet note.
    pub rebased_diff_unchanged: bool,
}

impl LayerLook {
    /// Quiet status note when this look was preserved across a diff-unchanged restack.
    #[must_use]
    pub const fn note(&self) -> Option<&'static str> {
        if self.rebased_diff_unchanged {
            Some(REBASED_DIFF_UNCHANGED_NOTE)
        } else {
            None
        }
    }
}

/// Reconciles `looks` against a layer's [`RestackStatus`]:
/// - [`RestackStatus::Unchanged`]: keeps all looks with `rebased_diff_unchanged = false`.
/// - [`RestackStatus::RebasedDiffUnchanged`]: keeps all looks with `rebased_diff_unchanged = true`.
/// - [`RestackStatus::DiffChanged`] / [`RestackStatus::Missing`]: drops all looks.
#[must_use]
pub fn reconcile_looks_after_restack(looks: &[Look], status: RestackStatus) -> Vec<LayerLook> {
    match status {
        RestackStatus::Unchanged => looks
            .iter()
            .cloned()
            .map(|look| LayerLook {
                look,
                rebased_diff_unchanged: false,
            })
            .collect(),
        RestackStatus::RebasedDiffUnchanged => looks
            .iter()
            .cloned()
            .map(|look| LayerLook {
                look,
                rebased_diff_unchanged: true,
            })
            .collect(),
        RestackStatus::DiffChanged | RestackStatus::Missing => Vec::new(),
    }
}

impl Checkout {
    /// Replaces the session's looks of this repo with `looks`; an empty list
    /// forgets them. Touches nothing but one blob and
    /// `refs/humata/looks/<session>`.
    ///
    /// # Errors
    ///
    /// [`DiffError::BadSession`] for a session id unfit for a ref name;
    /// [`DiffError`] when git fails.
    pub fn save_looks_blocking(&self, session: &str, looks: &[Look]) -> Result<(), DiffError> {
        if !valid_session(session) {
            return Err(DiffError::BadSession {
                session: session.to_owned(),
            });
        }
        let name = format!("{LOOKS}/{session}");
        if looks.is_empty() {
            if self.git.resolve(&name)?.is_some() {
                self.git.run(["update-ref", "-d", &name])?;
            }
            return Ok(());
        }
        let files: Vec<Value> = looks
            .iter()
            .map(|look| json!([look.path, look.fingerprint]))
            .collect();
        let bytes = Value::Array(files).to_string();
        let blob = trimmed(
            &self
                .git
                .run_with_input(["hash-object", "-w", "--stdin"], bytes.as_bytes())?,
        );
        self.git.run(["update-ref", "--no-deref", &name, &blob])?;
        Ok(())
    }

    /// The session's looks of this repo; empty when none were saved.
    ///
    /// # Errors
    ///
    /// [`DiffError::BadSession`]; [`DiffError::BadLooks`] when the saved
    /// blob can't be read; [`DiffError`] when git fails.
    pub fn looks_blocking(&self, session: &str) -> Result<Vec<Look>, DiffError> {
        if !valid_session(session) {
            return Err(DiffError::BadSession {
                session: session.to_owned(),
            });
        }
        let name = format!("{LOOKS}/{session}");
        if self.git.resolve(&name)?.is_none() {
            return Ok(Vec::new());
        }
        let bytes = self.git.run(["cat-file", "blob", &name])?;
        let bad = |what: String| DiffError::BadLooks {
            repo: self.path().to_owned(),
            session: session.to_owned(),
            what,
        };
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| bad(error.to_string()))?;
        let entries = value
            .as_array()
            .ok_or_else(|| bad("not a list".to_owned()))?;
        entries
            .iter()
            .map(|entry| {
                let path = entry.get(0).and_then(Value::as_str);
                let fingerprint = entry.get(1).and_then(Value::as_u64);
                match (path, fingerprint) {
                    (Some(path), Some(fingerprint)) => Ok(Look {
                        path: path.to_owned(),
                        fingerprint,
                    }),
                    _ => Err(bad("an entry is not [path, fingerprint]".to_owned())),
                }
            })
            .collect()
    }

    /// [`Self::save_looks_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::save_looks_blocking`].
    pub async fn save_looks(&self, session: String, looks: Vec<Look>) -> Result<(), DiffError> {
        let checkout = self.clone();
        crate::blocking(move || checkout.save_looks_blocking(&session, &looks)).await
    }

    /// Reconciles the session's saved looks against `status`: preserves them
    /// when `Unchanged` or `RebasedDiffUnchanged`, and clears them from the
    /// repo when `DiffChanged` or `Missing`.
    ///
    /// # Errors
    ///
    /// As [`Self::looks_blocking`] and [`Self::save_looks_blocking`].
    pub fn reconcile_looks_blocking(
        &self,
        session: &str,
        status: RestackStatus,
    ) -> Result<Vec<LayerLook>, DiffError> {
        let looks = self.looks_blocking(session)?;
        let reconciled = reconcile_looks_after_restack(&looks, status);
        if reconciled.is_empty() && !looks.is_empty() {
            self.save_looks_blocking(session, &[])?;
        }
        Ok(reconciled)
    }

    /// [`Self::looks_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::looks_blocking`].
    pub async fn looks(&self, session: String) -> Result<Vec<Look>, DiffError> {
        let checkout = self.clone();
        crate::blocking(move || checkout.looks_blocking(&session)).await
    }
}

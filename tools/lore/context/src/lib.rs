//! Workspace topology discovery, lockfile inspection, local citation
//! verification, predicate evaluation, and code-aware tokenization for `lore`.

pub mod discover;
pub mod lockfile;
pub mod predicate;
pub mod tokenize;
pub mod verify;

pub use discover::{
    ContextFrame, EnvHints, RepoMount, canonicalize_git_url, detect, normalize_path_to_repo,
    permitted_scopes,
};
pub use lockfile::{LockfileRef, find_package_versions, matches_semver_req, normalize_pep503};
pub use predicate::{evaluate_all, evaluate_predicate};
pub use tokenize::{normalize_error_signature, tokenize_code, tokenize_code_terms};
pub use verify::{VerifierResult, compute_file_digest, verify_all, verify_one};

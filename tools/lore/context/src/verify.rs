//! Local-side citation verifier executor (`RET-2`).
//!
//! Evaluates `lore_proto::Verifier` checks against the calling machine's
//! [`ContextFrame::mounted_repos`] working tree.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use lore_proto::{
    Ecosystem, FileDigestCheck, LockfileKind, LockfileSemverCheck, PathExistsCheck,
    SymbolPresentCheck, Verifier, VerifierStatus, verifier,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::discover::{ContextFrame, RepoMount};
use crate::lockfile::{find_package_versions, matches_semver_req};

/// Outcome of executing a single [`Verifier`] locally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierResult {
    pub status: VerifierStatus,
    pub detail: String,
}

/// Evaluates all `verifiers` against `frame` and returns the aggregate
/// [`VerifierStatus`] plus per-verifier details.
///
/// Aggregation rule (`RET-2`):
/// - Empty `verifiers` -> `UnverifiedLocal`
/// - Any `Drifted` -> `Drifted`
/// - Else any `Verified` -> `Verified`
/// - Else `UnverifiedLocal`
pub fn verify_all(frame: &ContextFrame, verifiers: &[Verifier]) -> (VerifierStatus, Vec<VerifierResult>) {
    if verifiers.is_empty() {
        return (VerifierStatus::UnverifiedLocal, Vec::new());
    }
    let results: Vec<VerifierResult> = verifiers.iter().map(|v| verify_one(frame, v)).collect();
    let aggregate = if results.iter().any(|r| r.status == VerifierStatus::Drifted) {
        VerifierStatus::Drifted
    } else if results.iter().any(|r| r.status == VerifierStatus::Verified) {
        VerifierStatus::Verified
    } else {
        VerifierStatus::UnverifiedLocal
    };
    (aggregate, results)
}

/// Evaluates a single [`Verifier`] against `frame`.
pub fn verify_one(frame: &ContextFrame, v: &Verifier) -> VerifierResult {
    match &v.check {
        None => VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: "verifier has no check payload".to_string(),
        },
        Some(verifier::Check::FileDigest(check)) => eval_file_digest(frame, check),
        Some(verifier::Check::PathExists(check)) => eval_path_exists(frame, check),
        Some(verifier::Check::SymbolPresent(check)) => eval_symbol_present(frame, check),
        Some(verifier::Check::LockfileSemver(check)) => eval_lockfile_semver(frame, check),
        Some(verifier::Check::BlobRef(blob)) => VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!("evidence blob {} ({})", blob.sha256, blob.role),
        },
    }
}

/// Computes the lowercase hex SHA-256 digest of a file (when `start_line == 0`
/// and `end_line == 0`) or of the 1-based inclusive line slice
/// `[start_line..=end_line]`.
pub fn compute_file_digest(path: &Path, start_line: u32, end_line: u32) -> io::Result<String> {
    if start_line == 0 && end_line == 0 {
        let bytes = fs::read(path)?;
        return Ok(sha256_hex(&bytes));
    }

    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let start_idx = start_line.saturating_sub(1) as usize;
    let end_idx = if end_line == 0 {
        lines.len()
    } else {
        (end_line as usize).min(lines.len())
    };
    if start_idx >= lines.len() || start_idx >= end_idx {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "line range {start_line}..={end_line} out of bounds for {} lines",
                lines.len()
            ),
        ));
    }

    let slice = lines[start_idx..end_idx].join("\n");
    Ok(sha256_hex(slice.as_bytes()))
}

fn eval_file_digest(frame: &ContextFrame, check: &FileDigestCheck) -> VerifierResult {
    let Some(mount) = resolve_mount(frame, &check.repo_id) else {
        return VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!("repo '{}' is not mounted locally", check.repo_id),
        };
    };
    let Some(target) = safe_join(&mount.root_path, &check.path) else {
        return VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!("invalid repo-relative path '{}'", check.path),
        };
    };
    match compute_file_digest(&target, check.start_line, check.end_line) {
        Ok(actual) => {
            if actual.eq_ignore_ascii_case(check.sha256.trim()) {
                VerifierResult {
                    status: VerifierStatus::Verified,
                    detail: format!("{} digest matches", check.path),
                }
            } else {
                VerifierResult {
                    status: VerifierStatus::Drifted,
                    detail: format!(
                        "{} digest drifted (expected {}, got {})",
                        check.path, check.sha256, actual
                    ),
                }
            }
        }
        Err(err) => VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!("{} unreadable or out of range: {err}", check.path),
        },
    }
}

fn eval_path_exists(frame: &ContextFrame, check: &PathExistsCheck) -> VerifierResult {
    let Some(mount) = resolve_mount(frame, &check.repo_id) else {
        return VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!("repo '{}' is not mounted locally", check.repo_id),
        };
    };
    let Some(target) = safe_join(&mount.root_path, &check.path) else {
        return VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!("invalid repo-relative path '{}'", check.path),
        };
    };
    let exists = target.exists();
    if exists == check.expect_exists {
        VerifierResult {
            status: VerifierStatus::Verified,
            detail: format!("{} exists={exists} as expected", check.path),
        }
    } else {
        VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!(
                "{} exists={exists}, expected {}",
                check.path, check.expect_exists
            ),
        }
    }
}

fn eval_symbol_present(frame: &ContextFrame, check: &SymbolPresentCheck) -> VerifierResult {
    let Some(mount) = resolve_mount(frame, &check.repo_id) else {
        return VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!("repo '{}' is not mounted locally", check.repo_id),
        };
    };
    let Some(target) = safe_join(&mount.root_path, &check.path) else {
        return VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!("invalid repo-relative path '{}'", check.path),
        };
    };
    let Ok(content) = fs::read_to_string(&target) else {
        return VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!("{} is missing or unreadable", check.path),
        };
    };

    if contains_symbol(&content, &check.symbol) {
        VerifierResult {
            status: VerifierStatus::Verified,
            detail: format!("symbol '{}' found in {}", check.symbol, check.path),
        }
    } else {
        VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!("symbol '{}' not found in {}", check.symbol, check.path),
        }
    }
}

fn eval_lockfile_semver(frame: &ContextFrame, check: &LockfileSemverCheck) -> VerifierResult {
    let ecosystem = Ecosystem::try_from(check.ecosystem).unwrap_or(Ecosystem::Unspecified);
    let kind_filter = LockfileKind::try_from(check.lockfile_kind).unwrap_or(LockfileKind::Unspecified);

    if matches!(
        ecosystem,
        Ecosystem::Unspecified | Ecosystem::Other | Ecosystem::Polyglot
    ) {
        return VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!("unsupported lockfile ecosystem {:?}", ecosystem),
        };
    }

    let mut parsed_any_lockfile = false;
    let mut found_versions = Vec::new();
    for mount in &frame.mounted_repos {
        if let Some(vers) = find_package_versions(
            &mount.root_path,
            &mount.lockfiles,
            ecosystem,
            kind_filter,
            &check.package_name,
        ) {
            parsed_any_lockfile = true;
            for v in vers {
                if !found_versions.contains(&v) {
                    found_versions.push(v);
                }
            }
        }
    }

    if !parsed_any_lockfile {
        return VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!(
                "no {:?} lockfile mounted locally for {}",
                ecosystem, check.package_name
            ),
        };
    }

    if found_versions.is_empty() {
        return VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!(
                "package '{}' not found in mounted {:?} lockfile(s)",
                check.package_name, ecosystem
            ),
        };
    }

    let mut any_valid_eval = false;
    for ver in &found_versions {
        match matches_semver_req(ver, &check.semver_req) {
            Some(true) => {
                return VerifierResult {
                    status: VerifierStatus::Verified,
                    detail: format!(
                        "{} {} satisfies {}",
                        check.package_name, ver, check.semver_req
                    ),
                };
            }
            Some(false) => {
                any_valid_eval = true;
            }
            None => {}
        }
    }

    if !any_valid_eval {
        VerifierResult {
            status: VerifierStatus::UnverifiedLocal,
            detail: format!(
                "could not parse semver requirement '{}' for {}",
                check.semver_req, check.package_name
            ),
        }
    } else {
        VerifierResult {
            status: VerifierStatus::Drifted,
            detail: format!(
                "{} versions {:?} do not satisfy {}",
                check.package_name, found_versions, check.semver_req
            ),
        }
    }
}

fn resolve_mount<'a>(frame: &'a ContextFrame, repo_id: &str) -> Option<&'a RepoMount> {
    if repo_id.is_empty() {
        if frame.mounted_repos.len() == 1 {
            return frame.mounted_repos.first();
        }
        return None;
    }
    frame.mounted_repos.iter().find(|m| m.repo_id == repo_id)
}

fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return None;
    }
    let mut out = root.to_path_buf();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn contains_symbol(content: &str, symbol: &str) -> bool {
    let sym = symbol.trim();
    if sym.is_empty() {
        return false;
    }
    let bytes = content.as_bytes();
    let sym_bytes = sym.as_bytes();
    let mut offset = 0;
    while let Some(pos) = content[offset..].find(sym) {
        let abs = offset + pos;
        let left_ok = abs == 0 || !is_ident_byte(bytes[abs - 1]);
        let right_idx = abs + sym_bytes.len();
        let right_ok = right_idx >= bytes.len() || !is_ident_byte(bytes[right_idx]);
        if left_ok && right_ok {
            return true;
        }
        offset = abs + content[abs..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for &b in &digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::{EnvHints, detect};
    use lore_proto::{BlobRef, Language};
    use tempfile::tempdir;

    fn setup_repo(root: &Path) -> ContextFrame {
        fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(root.join(".git/refs/heads/main"), format!("{}\n", "a".repeat(40))).unwrap();
        fs::write(
            root.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://github.com/acme/demo.git\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() {}\n",
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            "[[package]]\nname = \"prost\"\nversion = \"0.13.5\"\n",
        )
        .unwrap();
        detect(root, &EnvHints::default())
    }

    #[test]
    fn file_digest_verified_then_drifted_after_mutation() {
        let tmp = tempdir().unwrap();
        let frame = setup_repo(tmp.path());
        let digest = compute_file_digest(&tmp.path().join("src/lib.rs"), 1, 1).unwrap();

        let v = Verifier {
            check: Some(verifier::Check::FileDigest(FileDigestCheck {
                repo_id: "github.com/acme/demo".into(),
                path: "src/lib.rs".into(),
                start_line: 1,
                end_line: 1,
                sha256: digest,
            })),
        };
        assert_eq!(verify_one(&frame, &v).status, VerifierStatus::Verified);

        // Mutate the file -> must flip to Drifted
        fs::write(
            tmp.path().join("src/lib.rs"),
            "pub fn alpha_changed() {}\npub fn beta() {}\n",
        )
        .unwrap();
        assert_eq!(verify_one(&frame, &v).status, VerifierStatus::Drifted);
    }

    #[test]
    fn symbol_path_lockfile_and_unmounted_repo_verifiers() {
        let tmp = tempdir().unwrap();
        let frame = setup_repo(tmp.path());

        let sym_ok = Verifier {
            check: Some(verifier::Check::SymbolPresent(SymbolPresentCheck {
                repo_id: "github.com/acme/demo".into(),
                path: "src/lib.rs".into(),
                symbol: "alpha".into(),
                language: Language::Rust as i32,
            })),
        };
        assert_eq!(verify_one(&frame, &sym_ok).status, VerifierStatus::Verified);

        let sym_partial = Verifier {
            check: Some(verifier::Check::SymbolPresent(SymbolPresentCheck {
                repo_id: "github.com/acme/demo".into(),
                path: "src/lib.rs".into(),
                symbol: "alp".into(),
                language: Language::Rust as i32,
            })),
        };
        assert_eq!(
            verify_one(&frame, &sym_partial).status,
            VerifierStatus::Drifted
        );

        let path_ok = Verifier {
            check: Some(verifier::Check::PathExists(PathExistsCheck {
                repo_id: "github.com/acme/demo".into(),
                path: "src/lib.rs".into(),
                expect_exists: true,
            })),
        };
        assert_eq!(verify_one(&frame, &path_ok).status, VerifierStatus::Verified);

        let lock_ok = Verifier {
            check: Some(verifier::Check::LockfileSemver(LockfileSemverCheck {
                ecosystem: Ecosystem::Cargo as i32,
                lockfile_kind: LockfileKind::CargoLock as i32,
                package_name: "prost".into(),
                semver_req: "^0.13.0".into(),
            })),
        };
        assert_eq!(verify_one(&frame, &lock_ok).status, VerifierStatus::Verified);

        let lock_drift = Verifier {
            check: Some(verifier::Check::LockfileSemver(LockfileSemverCheck {
                ecosystem: Ecosystem::Cargo as i32,
                lockfile_kind: LockfileKind::Unspecified as i32,
                package_name: "prost".into(),
                semver_req: ">=0.14.0".into(),
            })),
        };
        assert_eq!(
            verify_one(&frame, &lock_drift).status,
            VerifierStatus::Drifted
        );

        let unmounted = Verifier {
            check: Some(verifier::Check::PathExists(PathExistsCheck {
                repo_id: "github.com/other/repo".into(),
                path: "README.md".into(),
                expect_exists: true,
            })),
        };
        assert_eq!(
            verify_one(&frame, &unmounted).status,
            VerifierStatus::UnverifiedLocal
        );

        let blob = Verifier {
            check: Some(verifier::Check::BlobRef(BlobRef {
                sha256: "abc".into(),
                role: "stderr".into(),
                summary: "log".into(),
            })),
        };
        assert_eq!(
            verify_one(&frame, &blob).status,
            VerifierStatus::UnverifiedLocal
        );

        let (agg_ok, _) = verify_all(&frame, &[sym_ok.clone(), blob.clone()]);
        assert_eq!(agg_ok, VerifierStatus::Verified);

        let (agg_drift, _) = verify_all(&frame, &[sym_ok, lock_drift]);
        assert_eq!(agg_drift, VerifierStatus::Drifted);
    }

    #[test]
    fn contains_symbol_handles_non_ascii_utf8_without_panicking() {
        // First occurrence of "é" is bounded by ASCII identifier chars ("xéy"),
        // forcing the search loop to advance past a multi-byte UTF-8 char.
        assert!(!contains_symbol("let xéy = 1;", "é"));
        assert!(contains_symbol("let xéy = 1; // é", "é"));
        assert!(contains_symbol("fn α_beta() {} fn α() {}", "α"));
        assert!(!contains_symbol("fn α_beta() {}", "α"));
    }
}

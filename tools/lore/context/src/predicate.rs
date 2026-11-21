//! Tri-state `applies_when` predicate evaluator (`MOD-3`, `TOP-7`).
//!
//! Any unrecognized axis or unsupported operator evaluates to
//! [`PredicateEval::Unknown`] and never errors or panics.

use std::str::FromStr;

use lore_proto::{Ecosystem, LockfileKind, Predicate, PredicateEval, PredicateOp};

use crate::discover::ContextFrame;
use crate::lockfile::{find_package_versions, matches_semver_req};

/// Evaluates a slice of predicates against `frame` using tri-state AND
/// semantics (`MOD-3`):
/// - Any [`PredicateEval::Mismatch`] -> `Mismatch` (`G_hard = 0`)
/// - Else any [`PredicateEval::Unknown`] -> `Unknown`
/// - Else `Match` (empty slice -> `Match`)
pub fn evaluate_all(frame: &ContextFrame, predicates: &[Predicate]) -> PredicateEval {
    let mut saw_unknown = false;
    for pred in predicates {
        match evaluate_predicate(frame, pred) {
            PredicateEval::Mismatch => return PredicateEval::Mismatch,
            PredicateEval::Unknown => saw_unknown = true,
            PredicateEval::Match => {}
        }
    }
    if saw_unknown {
        PredicateEval::Unknown
    } else {
        PredicateEval::Match
    }
}

/// Evaluates a single [`Predicate`] against `frame`.
pub fn evaluate_predicate(frame: &ContextFrame, predicate: &Predicate) -> PredicateEval {
    let Ok(op) = PredicateOp::try_from(predicate.op) else {
        return PredicateEval::Unknown;
    };
    if op == PredicateOp::Unspecified {
        return PredicateEval::Unknown;
    }

    let axis = predicate.axis.trim();
    match axis {
        "os" => eval_string_value(&frame.os, op, &predicate.value),
        "arch" => eval_string_value(&frame.arch, op, &predicate.value),
        "machine" => eval_string_value(&frame.machine_id, op, &predicate.value),
        "agent" => match frame.agent_kind.as_deref() {
            Some(agent) => eval_string_value(agent, op, &predicate.value),
            None => PredicateEval::Unknown,
        },
        "branch_glob" | "branch" => {
            let branches: Vec<&str> = frame
                .mounted_repos
                .iter()
                .filter_map(|m| m.branch.as_deref())
                .collect();
            if branches.is_empty() {
                return PredicateEval::Unknown;
            }
            eval_any_string(&branches, op, &predicate.value)
        }
        "commit" => eval_commit_ancestry(frame, op, &predicate.value),
        _ if axis.starts_with("dep:") => eval_dep_axis(frame, &axis[4..], op, &predicate.value),
        _ => PredicateEval::Unknown,
    }
}

fn eval_dep_axis(
    frame: &ContextFrame,
    spec: &str,
    op: PredicateOp,
    value: &str,
) -> PredicateEval {
    let Some((eco_str, pkg_name)) = spec.split_once('/') else {
        return PredicateEval::Unknown;
    };
    let ecosystem = Ecosystem::from_str(eco_str.trim()).unwrap_or(Ecosystem::Unspecified);
    if matches!(
        ecosystem,
        Ecosystem::Unspecified | Ecosystem::Other | Ecosystem::Polyglot
    ) {
        return PredicateEval::Unknown;
    }
    let pkg_name = pkg_name.trim();
    if pkg_name.is_empty() {
        return PredicateEval::Unknown;
    }

    let mut parsed_any = false;
    let mut versions_found = Vec::new();
    for mount in &frame.mounted_repos {
        if let Some(vers) = find_package_versions(
            &mount.root_path,
            &mount.lockfiles,
            ecosystem,
            LockfileKind::Unspecified,
            pkg_name,
        ) {
            parsed_any = true;
            for v in vers {
                if !versions_found.contains(&v) {
                    versions_found.push(v);
                }
            }
        }
    }

    if !parsed_any {
        return PredicateEval::Unknown;
    }
    if versions_found.is_empty() {
        return PredicateEval::Mismatch;
    }

    match op {
        PredicateOp::SemverReq => {
            let mut valid = false;
            for v in &versions_found {
                match matches_semver_req(v, value) {
                    Some(true) => return PredicateEval::Match,
                    Some(false) => valid = true,
                    None => {}
                }
            }
            if valid {
                PredicateEval::Mismatch
            } else {
                PredicateEval::Unknown
            }
        }
        PredicateOp::Eq | PredicateOp::In | PredicateOp::Glob => {
            let refs: Vec<&str> = versions_found.iter().map(String::as_str).collect();
            eval_any_string(&refs, op, value)
        }
        PredicateOp::AncestorOf | PredicateOp::NotAncestorOf | PredicateOp::Unspecified => {
            PredicateEval::Unknown
        }
    }
}

fn eval_commit_ancestry(frame: &ContextFrame, op: PredicateOp, target_sha: &str) -> PredicateEval {
    let target = target_sha.trim().to_ascii_lowercase();
    if target.is_empty() {
        return PredicateEval::Unknown;
    }
    let heads: Vec<String> = frame
        .mounted_repos
        .iter()
        .filter_map(|m| m.head_sha.as_deref())
        .map(str::to_ascii_lowercase)
        .collect();
    if heads.is_empty() {
        return PredicateEval::Unknown;
    }

    let exact_head_match = heads
        .iter()
        .any(|h| h == &target || h.starts_with(&target) || target.starts_with(h));

    match op {
        PredicateOp::Eq => {
            if exact_head_match {
                PredicateEval::Match
            } else {
                PredicateEval::Mismatch
            }
        }
        PredicateOp::AncestorOf => {
            // A commit is always an ancestor of itself; deeper packfile
            // ancestry is not walked on the <2ms pure-Rust hot path.
            if exact_head_match {
                PredicateEval::Match
            } else {
                PredicateEval::Unknown
            }
        }
        PredicateOp::NotAncestorOf => {
            if exact_head_match {
                PredicateEval::Mismatch
            } else {
                PredicateEval::Unknown
            }
        }
        _ => PredicateEval::Unknown,
    }
}

fn eval_any_string(actuals: &[&str], op: PredicateOp, expected: &str) -> PredicateEval {
    let mut saw_unknown = false;
    for actual in actuals {
        match eval_string_value(actual, op, expected) {
            PredicateEval::Match => return PredicateEval::Match,
            PredicateEval::Unknown => saw_unknown = true,
            PredicateEval::Mismatch => {}
        }
    }
    if saw_unknown {
        PredicateEval::Unknown
    } else {
        PredicateEval::Mismatch
    }
}

fn eval_string_value(actual: &str, op: PredicateOp, expected: &str) -> PredicateEval {
    match op {
        PredicateOp::Eq => {
            if actual.eq_ignore_ascii_case(expected.trim()) {
                PredicateEval::Match
            } else {
                PredicateEval::Mismatch
            }
        }
        PredicateOp::In => {
            let matched = expected
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .any(|item| actual.eq_ignore_ascii_case(item));
            if matched {
                PredicateEval::Match
            } else {
                PredicateEval::Mismatch
            }
        }
        PredicateOp::Glob => {
            if glob_match(expected.trim(), actual) {
                PredicateEval::Match
            } else {
                PredicateEval::Mismatch
            }
        }
        PredicateOp::SemverReq => match matches_semver_req(actual, expected) {
            Some(true) => PredicateEval::Match,
            Some(false) => PredicateEval::Mismatch,
            None => PredicateEval::Unknown,
        },
        PredicateOp::AncestorOf | PredicateOp::NotAncestorOf | PredicateOp::Unspecified => {
            PredicateEval::Unknown
        }
    }
}

/// Pure-Rust glob matcher supporting `*`, `?`, and `**`.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;

    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }

    for i in 1..=p.len() {
        for j in 1..=t.len() {
            dp[i][j] = match p[i - 1] {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                ch => dp[i - 1][j - 1] && ch == t[j - 1],
            };
        }
    }

    dp[p.len()][t.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::{EnvHints, detect};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn evaluates_standard_axes_and_unknown_axis() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git/refs/heads/release")).unwrap();
        fs::write(tmp.path().join(".git/HEAD"), "ref: refs/heads/release/v2\n").unwrap();
        fs::write(
            tmp.path().join(".git/refs/heads/release/v2"),
            format!("{}\n", "c".repeat(40)),
        )
        .unwrap();
        fs::write(
            tmp.path().join("Cargo.lock"),
            "[[package]]\nname = \"prost\"\nversion = \"0.13.5\"\n",
        )
        .unwrap();

        let env = EnvHints {
            machine_id: Some("mac-studio".into()),
            agent_kind: Some("agy".into()),
            os: Some("macos".into()),
            arch: Some("aarch64".into()),
            ..Default::default()
        };
        let frame = detect(tmp.path(), &env);

        assert_eq!(
            evaluate_predicate(
                &frame,
                &Predicate {
                    axis: "os".into(),
                    op: PredicateOp::Eq as i32,
                    value: "macos".into(),
                }
            ),
            PredicateEval::Match
        );
        assert_eq!(
            evaluate_predicate(
                &frame,
                &Predicate {
                    axis: "arch".into(),
                    op: PredicateOp::In as i32,
                    value: "x86_64, aarch64".into(),
                }
            ),
            PredicateEval::Match
        );
        assert_eq!(
            evaluate_predicate(
                &frame,
                &Predicate {
                    axis: "branch_glob".into(),
                    op: PredicateOp::Glob as i32,
                    value: "release/*".into(),
                }
            ),
            PredicateEval::Match
        );
        assert_eq!(
            evaluate_predicate(
                &frame,
                &Predicate {
                    axis: "dep:cargo/prost".into(),
                    op: PredicateOp::SemverReq as i32,
                    value: "^0.13.0".into(),
                }
            ),
            PredicateEval::Match
        );
        assert_eq!(
            evaluate_predicate(
                &frame,
                &Predicate {
                    axis: "dep:cargo/prost".into(),
                    op: PredicateOp::SemverReq as i32,
                    value: ">=0.14.0".into(),
                }
            ),
            PredicateEval::Mismatch
        );
        // Unknown ecosystem -> Unknown (never error)
        assert_eq!(
            evaluate_predicate(
                &frame,
                &Predicate {
                    axis: "dep:unknown_eco/foo".into(),
                    op: PredicateOp::SemverReq as i32,
                    value: ">=1.0.0".into(),
                }
            ),
            PredicateEval::Unknown
        );
        // Unrecognized axis -> Unknown (MOD-3)
        let unknown_pred = Predicate {
            axis: "future_axis_v9".into(),
            op: PredicateOp::Eq as i32,
            value: "anything".into(),
        };
        assert_eq!(
            evaluate_predicate(&frame, &unknown_pred),
            PredicateEval::Unknown
        );

        let match_pred = Predicate {
            axis: "os".into(),
            op: PredicateOp::Eq as i32,
            value: "macos".into(),
        };
        let mismatch_pred = Predicate {
            axis: "os".into(),
            op: PredicateOp::Eq as i32,
            value: "linux".into(),
        };
        assert_eq!(evaluate_all(&frame, &[match_pred.clone()]), PredicateEval::Match);
        assert_eq!(
            evaluate_all(&frame, &[match_pred.clone(), unknown_pred.clone()]),
            PredicateEval::Unknown
        );
        assert_eq!(
            evaluate_all(&frame, &[match_pred, unknown_pred, mismatch_pred]),
            PredicateEval::Mismatch
        );
    }
}

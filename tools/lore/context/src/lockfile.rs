//! Pure-Rust language & lockfile detection, dependency version extraction, and
//! semver requirement matching across closed [`Ecosystem`] / [`LockfileKind`]
//! enums.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lore_proto::{Ecosystem, Language, LockfileKind};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Maximum number of top-level directory entries inspected for language
/// extension hints.
const MAX_LANG_SCAN_ENTRIES: usize = 64;

/// A lockfile or manifest discovered inside a mounted repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockfileRef {
    pub ecosystem: Ecosystem,
    pub kind: LockfileKind,
    pub rel_path: PathBuf,
}

/// Detects languages and lockfiles present at `repo_root` using fast root-level
/// checks (no recursive tree walk, no subprocesses).
pub(crate) fn detect_languages_and_lockfiles(repo_root: &Path) -> (Vec<Language>, Vec<LockfileRef>) {
    let mut languages = Vec::new();
    let mut lockfiles = Vec::new();

    let mut push_lang = |lang: Language| {
        if !languages.contains(&lang) {
            languages.push(lang);
        }
    };

    let mut push_lock = |ecosystem: Ecosystem, kind: LockfileKind, rel: &str| {
        lockfiles.push(LockfileRef {
            ecosystem,
            kind,
            rel_path: PathBuf::from(rel),
        });
    };

    // Rust / Cargo
    if repo_root.join("Cargo.lock").is_file() {
        push_lang(Language::Rust);
        push_lock(Ecosystem::Cargo, LockfileKind::CargoLock, "Cargo.lock");
    } else if repo_root.join("Cargo.toml").is_file() {
        push_lang(Language::Rust);
    }

    // Go
    if repo_root.join("go.mod").is_file() {
        push_lang(Language::Go);
        push_lock(Ecosystem::Go, LockfileKind::GoMod, "go.mod");
    }

    // Python
    let mut py_found = false;
    if repo_root.join("uv.lock").is_file() {
        py_found = true;
        push_lock(Ecosystem::Python, LockfileKind::PyUvLock, "uv.lock");
    }
    if repo_root.join("poetry.lock").is_file() {
        py_found = true;
        push_lock(Ecosystem::Python, LockfileKind::PyPoetryLock, "poetry.lock");
    }
    if repo_root.join("requirements.txt").is_file() {
        py_found = true;
        push_lock(
            Ecosystem::Python,
            LockfileKind::PyRequirements,
            "requirements.txt",
        );
    }
    if py_found
        || repo_root.join("pyproject.toml").is_file()
        || repo_root.join("setup.py").is_file()
    {
        push_lang(Language::Python);
    }

    // Java / Maven / Gradle
    let mut java_found = false;
    if repo_root.join("pom.xml").is_file() {
        java_found = true;
        push_lock(Ecosystem::Maven, LockfileKind::JavaPom, "pom.xml");
    }
    if repo_root.join("gradle.lockfile").is_file() {
        java_found = true;
        push_lock(
            Ecosystem::Maven,
            LockfileKind::JavaGradleLock,
            "gradle.lockfile",
        );
    }
    if java_found
        || repo_root.join("build.gradle").is_file()
        || repo_root.join("build.gradle.kts").is_file()
    {
        push_lang(Language::Java);
    }

    // Bazel / Starlark
    let mut bazel_found = false;
    if repo_root.join("MODULE.bazel.lock").is_file() {
        bazel_found = true;
        push_lock(
            Ecosystem::Bazel,
            LockfileKind::BazelModuleLock,
            "MODULE.bazel.lock",
        );
    }
    if repo_root.join("MODULE.bazel").is_file() {
        bazel_found = true;
        push_lock(
            Ecosystem::Bazel,
            LockfileKind::BazelModuleLock,
            "MODULE.bazel",
        );
    }
    if bazel_found
        || repo_root.join("BUILD.bazel").is_file()
        || repo_root.join("BUILD").is_file()
        || repo_root.join("WORKSPACE").is_file()
        || repo_root.join("WORKSPACE.bazel").is_file()
    {
        push_lang(Language::Bazel);
    }

    // Swift / SwiftPM
    let mut swift_found = false;
    if repo_root.join("Package.resolved").is_file() {
        swift_found = true;
        push_lock(
            Ecosystem::Swift,
            LockfileKind::SwiftPackageResolved,
            "Package.resolved",
        );
    }
    if swift_found || repo_root.join("Package.swift").is_file() {
        push_lang(Language::Swift);
    }

    // Bounded top-level file extension hints for repos without root manifests.
    if let Ok(entries) = fs::read_dir(repo_root) {
        for (idx, entry) in entries.enumerate() {
            if idx >= MAX_LANG_SCAN_ENTRIES {
                break;
            }
            let Ok(entry) = entry else { continue };
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_file() {
                continue;
            }
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("rs") => push_lang(Language::Rust),
                Some("go") => push_lang(Language::Go),
                Some("py") => push_lang(Language::Python),
                Some("java") => push_lang(Language::Java),
                Some("bzl") => push_lang(Language::Starlark),
                Some("swift") => push_lang(Language::Swift),
                _ => {}
            }
        }
    }

    (languages, lockfiles)
}

/// Looks up installed versions of `package_name` in `repo_root`'s lockfiles for
/// `ecosystem` (optionally narrowed by `kind_filter` when not `Unspecified`).
///
/// Returns `None` if no matching lockfile exists or could be read on disk.
/// Returns `Some(versions)` if at least one matching lockfile was parsed
/// (an empty `Vec` means the lockfile exists, but `package_name` is absent).
pub fn find_package_versions(
    repo_root: &Path,
    lockfiles: &[LockfileRef],
    ecosystem: Ecosystem,
    kind_filter: LockfileKind,
    package_name: &str,
) -> Option<Vec<String>> {
    if matches!(
        ecosystem,
        Ecosystem::Unspecified | Ecosystem::Other | Ecosystem::Polyglot
    ) {
        return None;
    }

    let mut parsed_any = false;
    let mut versions_found = Vec::new();

    for lf in lockfiles {
        if lf.ecosystem != ecosystem {
            continue;
        }
        if kind_filter != LockfileKind::Unspecified && lf.kind != kind_filter {
            continue;
        }
        let full_path = repo_root.join(&lf.rel_path);
        let Ok(content) = fs::read_to_string(&full_path) else {
            continue;
        };
        parsed_any = true;

        let extracted = match lf.kind {
            LockfileKind::CargoLock => parse_cargo_lock(&content, package_name),
            LockfileKind::GoMod => parse_go_mod(&content, package_name),
            LockfileKind::PyUvLock | LockfileKind::PyPoetryLock => {
                parse_python_toml_lock(&content, package_name)
            }
            LockfileKind::PyRequirements => parse_requirements_txt(&content, package_name),
            LockfileKind::JavaPom => parse_pom_xml(&content, package_name),
            LockfileKind::JavaGradleLock => parse_gradle_lockfile(&content, package_name),
            LockfileKind::BazelModuleLock => {
                if lf.rel_path.file_name().and_then(|n| n.to_str()) == Some("MODULE.bazel") {
                    parse_module_bazel(&content, package_name)
                } else {
                    parse_module_bazel_lock(&content, package_name)
                }
            }
            LockfileKind::SwiftPackageResolved => {
                parse_swift_package_resolved(&content, package_name)
            }
            LockfileKind::Unspecified | LockfileKind::Other => Vec::new(),
        };

        for v in extracted {
            if !versions_found.contains(&v) {
                versions_found.push(v);
            }
        }
    }

    if parsed_any {
        Some(versions_found)
    } else {
        None
    }
}

/// Evaluates whether `version_str` satisfies a comma-separated semver
/// requirement `req_str` (supporting `>=`, `<=`, `>`, `<`, `==`, `=`, `!=`,
/// `^`, `~`, and bare exact versions).
///
/// Returns `None` if either `version_str` or `req_str` cannot be parsed.
pub fn matches_semver_req(version_str: &str, req_str: &str) -> Option<bool> {
    let actual = parse_version(version_str)?;
    let trimmed = req_str.trim();
    if trimmed.is_empty() {
        return None;
    }

    for clause in trimmed.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }
        if !matches_single_clause(&actual, clause)? {
            return Some(false);
        }
    }
    Some(true)
}

/// PEP 503 Python package name normalization: lowercase and collapse runs of
/// `[-_.]+` into a single `-`.
pub fn normalize_pep503(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = false;
    for ch in name.trim().chars() {
        if ch == '-' || ch == '_' || ch == '.' {
            if !prev_sep && !out.is_empty() {
                out.push('-');
                prev_sep = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

fn normalize_cargo_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

fn parse_version(raw: &str) -> Option<versions::Version> {
    let s = raw.trim().trim_start_matches(['v', 'V']);
    if s.is_empty() {
        return None;
    }
    let v = versions::Version::new(s)?;
    // Require a leading numeric component so non-version strings like "main"
    // or "not-a-ver" are rejected.
    v.nth(0)?;
    Some(v)
}

fn matches_single_clause(actual: &versions::Version, clause: &str) -> Option<bool> {
    if let Some(rest) = clause.strip_prefix(">=") {
        let target = parse_version(rest)?;
        return Some(actual >= &target);
    }
    if let Some(rest) = clause.strip_prefix("<=") {
        let target = parse_version(rest)?;
        return Some(actual <= &target);
    }
    if let Some(rest) = clause.strip_prefix("!=") {
        let target = parse_version(rest)?;
        return Some(actual != &target);
    }
    if let Some(rest) = clause.strip_prefix("==") {
        let target = parse_version(rest)?;
        return Some(actual == &target);
    }
    if let Some(rest) = clause.strip_prefix('>') {
        let target = parse_version(rest)?;
        return Some(actual > &target);
    }
    if let Some(rest) = clause.strip_prefix('<') {
        let target = parse_version(rest)?;
        return Some(actual < &target);
    }
    if let Some(rest) = clause.strip_prefix('=') {
        let target = parse_version(rest)?;
        return Some(actual == &target);
    }
    if let Some(rest) = clause.strip_prefix('^') {
        let low = parse_version(rest)?;
        let major = low.nth(0).unwrap_or(0);
        let minor = low.nth(1).unwrap_or(0);
        let patch = low.nth(2).unwrap_or(0);
        let upper_str = if major > 0 {
            format!("{}.0.0", major + 1)
        } else if minor > 0 {
            format!("0.{}.0", minor + 1)
        } else {
            format!("0.0.{}", patch + 1)
        };
        let upper = parse_version(&upper_str)?;
        return Some(actual >= &low && actual < &upper);
    }
    if let Some(rest) = clause.strip_prefix('~') {
        let low = parse_version(rest)?;
        let major = low.nth(0).unwrap_or(0);
        let upper_str = match low.nth(1) {
            Some(minor) => format!("{}.{}.0", major, minor + 1),
            None => format!("{}.0.0", major + 1),
        };
        let upper = parse_version(&upper_str)?;
        return Some(actual >= &low && actual < &upper);
    }

    let target = parse_version(clause)?;
    Some(actual == &target)
}

fn parse_cargo_lock(content: &str, package_name: &str) -> Vec<String> {
    let want = normalize_cargo_name(package_name);
    let mut out = Vec::new();
    let mut in_pkg = false;
    let mut cur_name: Option<String> = None;
    let mut cur_ver: Option<String> = None;

    let flush = |cur_name: &mut Option<String>, cur_ver: &mut Option<String>, out: &mut Vec<String>| {
        if let (Some(n), Some(v)) = (cur_name.take(), cur_ver.take()) {
            if normalize_cargo_name(&n) == want {
                out.push(v);
            }
        }
    };

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line == "[[package]]" {
            flush(&mut cur_name, &mut cur_ver, &mut out);
            in_pkg = true;
            continue;
        }
        if line.starts_with('[') {
            flush(&mut cur_name, &mut cur_ver, &mut out);
            in_pkg = false;
            continue;
        }
        if !in_pkg {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = unquote_toml_str(v);
            if key == "name" {
                cur_name = val;
            } else if key == "version" {
                cur_ver = val;
            }
        }
    }
    flush(&mut cur_name, &mut cur_ver, &mut out);
    out
}

fn parse_go_mod(content: &str, package_name: &str) -> Vec<String> {
    let want = package_name.trim();
    let mut out = Vec::new();
    let mut in_require_block = false;

    for raw_line in content.lines() {
        let line = strip_go_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if in_require_block {
            if line == ")" {
                in_require_block = false;
                continue;
            }
            let mut parts = line.split_whitespace();
            if let (Some(mod_path), Some(ver)) = (parts.next(), parts.next()) {
                if mod_path == want {
                    out.push(ver.trim_start_matches('v').to_string());
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("require") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                in_require_block = true;
            } else {
                let mut parts = rest.split_whitespace();
                if let (Some(mod_path), Some(ver)) = (parts.next(), parts.next()) {
                    if mod_path == want {
                        out.push(ver.trim_start_matches('v').to_string());
                    }
                }
            }
        }
    }

    out
}

fn strip_go_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

fn parse_python_toml_lock(content: &str, package_name: &str) -> Vec<String> {
    let want = normalize_pep503(package_name);
    let mut out = Vec::new();
    let mut in_pkg = false;
    let mut cur_name: Option<String> = None;
    let mut cur_ver: Option<String> = None;

    let flush = |cur_name: &mut Option<String>, cur_ver: &mut Option<String>, out: &mut Vec<String>| {
        if let (Some(n), Some(v)) = (cur_name.take(), cur_ver.take()) {
            if normalize_pep503(&n) == want {
                out.push(v);
            }
        }
    };

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line == "[[package]]" {
            flush(&mut cur_name, &mut cur_ver, &mut out);
            in_pkg = true;
            continue;
        }
        if line.starts_with('[') {
            flush(&mut cur_name, &mut cur_ver, &mut out);
            in_pkg = false;
            continue;
        }
        if !in_pkg {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = unquote_toml_str(v);
            if key == "name" {
                cur_name = val;
            } else if key == "version" {
                cur_ver = val;
            }
        }
    }
    flush(&mut cur_name, &mut cur_ver, &mut out);
    out
}

fn parse_requirements_txt(content: &str, package_name: &str) -> Vec<String> {
    let want = normalize_pep503(package_name);
    let mut out = Vec::new();
    for raw_line in content.lines() {
        let line = match raw_line.find('#') {
            Some(idx) => &raw_line[..idx],
            None => raw_line,
        }
        .trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }
        // Drop environment markers (`; python_version < ...`)
        let line = line.split(';').next().unwrap_or(line).trim();
        let (lhs, rhs) = if let Some(pair) = line.split_once("===") {
            pair
        } else if let Some(pair) = line.split_once("==") {
            pair
        } else {
            continue;
        };
        // Strip extras like `requests[security]`
        let pkg = lhs.split('[').next().unwrap_or(lhs).trim();
        let ver = rhs.split_whitespace().next().unwrap_or("").trim();
        if !ver.is_empty() && normalize_pep503(pkg) == want {
            out.push(ver.to_string());
        }
    }
    out
}

fn parse_gradle_lockfile(content: &str, package_name: &str) -> Vec<String> {
    let want = package_name.trim();
    let mut out = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("empty=") {
            continue;
        }
        let coords = line.split('=').next().unwrap_or(line).trim();
        let mut parts = coords.split(':');
        if let (Some(group), Some(artifact), Some(ver)) =
            (parts.next(), parts.next(), parts.next())
        {
            if format!("{group}:{artifact}") == want && !ver.is_empty() {
                out.push(ver.to_string());
            }
        }
    }
    out
}

fn parse_pom_xml(content: &str, package_name: &str) -> Vec<String> {
    let want = package_name.trim();
    let mut props: HashMap<String, String> = HashMap::new();
    if let Some(caps) = pom_properties_re().captures(content) {
        if let Some(body) = caps.get(1) {
            for prop in xml_simple_tag_re().captures_iter(body.as_str()) {
                if let (Some(k), Some(v)) = (prop.get(1), prop.get(2)) {
                    props.insert(k.as_str().trim().to_string(), v.as_str().trim().to_string());
                }
            }
        }
    }

    let mut out = Vec::new();
    for dep in pom_dependency_re().captures_iter(content) {
        let Some(block) = dep.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let group = extract_xml_tag(block, "groupId");
        let artifact = extract_xml_tag(block, "artifactId");
        let version = extract_xml_tag(block, "version");
        if let (Some(g), Some(a), Some(mut v)) = (group, artifact, version) {
            if let Some(prop_name) = v.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
                if let Some(resolved) = props.get(prop_name) {
                    v = resolved.clone();
                }
            }
            if format!("{g}:{a}") == want && !v.is_empty() && !v.starts_with("${") {
                out.push(v);
            }
        }
    }
    out
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let val = xml[start..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn parse_module_bazel_lock(content: &str, package_name: &str) -> Vec<String> {
    let want = package_name.trim();
    let mut out = Vec::new();
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return out;
    };

    // Format 1: `moduleDepGraph` keys like `"rules_rust@0.56.0"`
    if let Some(graph) = val.get("moduleDepGraph").and_then(|v| v.as_object()) {
        for key in graph.keys() {
            if let Some((name, ver)) = key.split_once('@') {
                if name == want && !ver.is_empty() && ver != "_" && !out.contains(&ver.to_string()) {
                    out.push(ver.to_string());
                }
            }
        }
    }

    // Format 2: `registryFileHashes` URLs like `.../modules/<name>/<version>/MODULE.bazel`
    if let Some(hashes) = val.get("registryFileHashes").and_then(|v| v.as_object()) {
        let needle = format!("/modules/{want}/");
        for url in hashes.keys() {
            if let Some(idx) = url.find(&needle) {
                let rest = &url[idx + needle.len()..];
                if let Some((ver, _)) = rest.split_once('/') {
                    if !ver.is_empty() && !out.contains(&ver.to_string()) {
                        out.push(ver.to_string());
                    }
                }
            }
        }
    }

    out
}

fn parse_module_bazel(content: &str, package_name: &str) -> Vec<String> {
    let want = package_name.trim();
    let mut out = Vec::new();
    for call in bazel_dep_call_re().captures_iter(content) {
        let Some(body) = call.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let name = bazel_name_attr_re()
            .captures(body)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str());
        let ver = bazel_version_attr_re()
            .captures(body)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str());
        if let (Some(n), Some(v)) = (name, ver) {
            if n == want && !v.is_empty() {
                out.push(v.to_string());
            }
        }
    }
    out
}

fn parse_swift_package_resolved(content: &str, package_name: &str) -> Vec<String> {
    let want = package_name.trim().to_ascii_lowercase();
    let mut out = Vec::new();
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return out;
    };

    // v2/v3: top-level `pins`; v1: `object.pins`
    let pins = val
        .get("pins")
        .or_else(|| val.get("object").and_then(|o| o.get("pins")))
        .and_then(|p| p.as_array());

    let Some(pins) = pins else {
        return out;
    };

    for pin in pins {
        let identity = pin
            .get("identity")
            .or_else(|| pin.get("package"))
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase);
        let loc_basename = pin
            .get("location")
            .or_else(|| pin.get("repositoryURL"))
            .and_then(|v| v.as_str())
            .and_then(|url| {
                url.trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .map(|s| s.strip_suffix(".git").unwrap_or(s).to_ascii_lowercase())
            });

        let matched = identity.as_deref() == Some(want.as_str())
            || loc_basename.as_deref() == Some(want.as_str());
        if matched {
            if let Some(ver) = pin
                .get("state")
                .and_then(|s| s.get("version"))
                .and_then(|v| v.as_str())
            {
                if !ver.is_empty() {
                    out.push(ver.to_string());
                }
            }
        }
    }

    out
}

fn unquote_toml_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        Some(trimmed[1..trimmed.len() - 1].to_string())
    } else if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        Some(trimmed[1..trimmed.len() - 1].to_string())
    } else {
        None
    }
}

fn pom_properties_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<properties>(.*?)</properties>").unwrap())
}

fn pom_dependency_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<dependency>(.*?)</dependency>").unwrap())
}

fn xml_simple_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<([A-Za-z0-9_.-]+)>([^<]+)</[A-Za-z0-9_.-]+>").unwrap())
}

fn bazel_dep_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)bazel_dep\s*\((.*?)\)").unwrap())
}

fn bazel_name_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"name\s*=\s*"([^"]+)""#).unwrap())
}

fn bazel_version_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"version\s*=\s*"([^"]+)""#).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_six_ecosystems() {
        let cargo = r#"
[[package]]
name = "prost"
version = "0.13.5"
"#;
        assert_eq!(parse_cargo_lock(cargo, "prost"), vec!["0.13.5"]);

        let gomod = r#"
module example.com/app
require (
    github.com/spf13/cobra v1.8.1
)
"#;
        assert_eq!(parse_go_mod(gomod, "github.com/spf13/cobra"), vec!["1.8.1"]);

        let uv = r#"
[[package]]
name = "Django_REST_Framework"
version = "3.15.2"
"#;
        assert_eq!(
            parse_python_toml_lock(uv, "django-rest-framework"),
            vec!["3.15.2"]
        );

        let reqs = "Requests[security]==2.32.3 ; python_version >= '3.8'\n";
        assert_eq!(parse_requirements_txt(reqs, "requests"), vec!["2.32.3"]);

        let pom = r#"
<project>
  <properties><guava.version>33.2.0</guava.version></properties>
  <dependencies>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>${guava.version}</version>
    </dependency>
  </dependencies>
</project>
"#;
        assert_eq!(
            parse_pom_xml(pom, "com.google.guava:guava"),
            vec!["33.2.0"]
        );

        let gradle = "# lock\ncom.google.guava:guava:33.2.0=compileClasspath\n";
        assert_eq!(
            parse_gradle_lockfile(gradle, "com.google.guava:guava"),
            vec!["33.2.0"]
        );

        let bzl_lock = r#"{"moduleDepGraph":{"rules_rust@0.56.0":{}}}"#;
        assert_eq!(
            parse_module_bazel_lock(bzl_lock, "rules_rust"),
            vec!["0.56.0"]
        );

        let bzl_mod = r#"bazel_dep(name = "rules_rust", version = "0.56.0")"#;
        assert_eq!(parse_module_bazel(bzl_mod, "rules_rust"), vec!["0.56.0"]);

        let swift = r#"{"pins":[{"identity":"swift-nio","state":{"version":"2.70.0"}}],"version":2}"#;
        assert_eq!(
            parse_swift_package_resolved(swift, "swift-nio"),
            vec!["2.70.0"]
        );
    }

    #[test]
    fn semver_requirement_matching() {
        assert_eq!(matches_semver_req("0.13.5", "^0.13.0"), Some(true));
        assert_eq!(matches_semver_req("0.14.0", "^0.13.0"), Some(false));
        assert_eq!(matches_semver_req("1.8.1", ">=1.8.0, <2.0.0"), Some(true));
        assert_eq!(matches_semver_req("v1.8.1", "~1.8.0"), Some(true));
        assert_eq!(matches_semver_req("1.9.0", "~1.8.0"), Some(false));
        assert_eq!(matches_semver_req("not-a-ver", ">=1.0.0"), None);
    }
}

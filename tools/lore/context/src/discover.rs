//! Pure-Rust, `<2 ms` workspace topology discovery (`TOP-1` through `TOP-7`).
//!
//! Resolves `0`, `1`, or `N` mounted Git repositories (including sandboxed
//! worktrees whose `gitdir:` target is unreadable) without spawning `git`
//! subprocesses.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use lore_proto::Language;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::lockfile::{LockfileRef, detect_languages_and_lockfiles};

/// Maximum number of directory entries inspected during a multi-repo parent
/// scan (`TOP-4`).
const MAX_SCAN_ENTRIES: usize = 64;

/// Environment and session hints supplied to [`detect`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvHints {
    pub machine_id: Option<String>,
    pub channel_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_kind: Option<String>,
    /// Value of `HUMATA_SESSION_REPOS`: either a JSON array of objects/strings
    /// or a comma/semicolon-separated list of `repo_id` or `subpath=repo_id`.
    pub session_repos: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
}

impl EnvHints {
    /// Reads standard Humata / `lore` environment variables without failing.
    pub fn from_env() -> Self {
        Self {
            machine_id: non_empty_env("HUMATA_MACHINE_ID")
                .or_else(|| non_empty_env("LORE_MACHINE_ID"))
                .or_else(|| non_empty_env("HOSTNAME")),
            channel_id: non_empty_env("HUMATA_CHANNEL_ID")
                .or_else(|| non_empty_env("LORE_CHANNEL_ID")),
            session_id: non_empty_env("HUMATA_SESSION_ID")
                .or_else(|| non_empty_env("LORE_SESSION_ID")),
            agent_kind: non_empty_env("HUMATA_AGENT_KIND")
                .or_else(|| non_empty_env("LORE_AGENT_KIND")),
            session_repos: non_empty_env("HUMATA_SESSION_REPOS"),
            os: None,
            arch: None,
        }
    }
}

/// Snapshot of the caller's workspace topology, host environment, and session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFrame {
    pub cwd: PathBuf,
    pub machine_id: String,
    pub os: String,
    pub arch: String,
    pub channel_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_kind: Option<String>,
    pub mounted_repos: Vec<RepoMount>,
    pub cwd_dir_scope: String,
}

/// A single Git repository mounted in or beneath `cwd`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMount {
    pub repo_id: String,
    pub root_path: PathBuf,
    pub mount_subpath: PathBuf,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub sandboxed_worktree: bool,
    pub languages: Vec<Language>,
    pub lockfiles: Vec<LockfileRef>,
}

impl RepoMount {
    /// Returns [`Language::Polyglot`] when multiple languages are detected,
    /// the sole detected language when exactly one is present, or
    /// [`Language::Unspecified`] when none are detected.
    pub fn primary_language(&self) -> Language {
        match self.languages.len() {
            0 => Language::Unspecified,
            1 => self.languages[0],
            _ => Language::Polyglot,
        }
    }

    /// Returns `true` when more than one language was detected in this repo.
    pub fn is_polyglot(&self) -> bool {
        self.languages.len() > 1
    }
}

#[derive(Clone, Debug, Deserialize)]
struct SessionRepoJson {
    repo_id: String,
    #[serde(default)]
    root_path: Option<PathBuf>,
    #[serde(default)]
    mount_subpath: Option<PathBuf>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    head_sha: Option<String>,
}

#[derive(Clone, Debug)]
struct SessionRepoHint {
    repo_id: String,
    root_path: Option<PathBuf>,
    mount_subpath: Option<PathBuf>,
    branch: Option<String>,
    head_sha: Option<String>,
}

/// Detects the current [`ContextFrame`] in pure Rust without spawning `git`.
pub fn detect(cwd: &Path, env: &EnvHints) -> ContextFrame {
    let canonical_cwd = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let machine_id = env
        .machine_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("local")
        .to_string();
    let os = env
        .os
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(std::env::consts::OS)
        .to_string();
    let arch = env
        .arch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(std::env::consts::ARCH)
        .to_string();

    let cwd_dir_scope = make_dir_scope(&machine_id, &canonical_cwd);
    let session_hints = env
        .session_repos
        .as_deref()
        .map(parse_session_repos)
        .unwrap_or_default();

    let mounted_repos = if let Some(repo_root) = find_enclosing_git_root(&canonical_cwd) {
        let mount = inspect_repo_root(
            &repo_root,
            PathBuf::new(),
            &cwd_dir_scope,
            &session_hints,
        );
        vec![mount]
    } else {
        discover_child_repos(&canonical_cwd, &machine_id, &session_hints)
    };

    ContextFrame {
        cwd: canonical_cwd,
        machine_id,
        os,
        arch,
        channel_id: clean_opt(env.channel_id.as_deref()),
        session_id: clean_opt(env.session_id.as_deref()),
        agent_kind: clean_opt(env.agent_kind.as_deref()),
        mounted_repos,
        cwd_dir_scope,
    }
}

/// Maps `path` (absolute, relative to `frame.cwd`, or prefixed with a mounted
/// repo's `mount_subpath`) to `(repo_id, repo_relative_posix_path)`.
pub fn normalize_path_to_repo(frame: &ContextFrame, path: &Path) -> Option<(String, String)> {
    if frame.mounted_repos.is_empty() {
        return None;
    }

    // 1. Try matching against `mount.root_path` using a lexically normalized
    //    absolute path. Prefer the deepest matching `root_path`.
    let abs = if path.is_absolute() {
        lexical_normalize(path)
    } else {
        lexical_normalize(&frame.cwd.join(path))
    };

    let mut best: Option<(&RepoMount, PathBuf)> = None;
    for mount in &frame.mounted_repos {
        let mount_root = lexical_normalize(&mount.root_path);
        if let Ok(rel) = abs.strip_prefix(&mount_root) {
            let replace = match &best {
                None => true,
                Some((prev, _)) => mount_root.as_os_str().len() > prev.root_path.as_os_str().len(),
            };
            if replace {
                best = Some((mount, rel.to_path_buf()));
            }
        }
    }
    if let Some((mount, rel)) = best {
        return Some((mount.repo_id.clone(), to_posix_rel(&rel)));
    }

    // 2. If `path` is relative and didn't resolve via `frame.cwd`, try matching
    //    explicitly against non-empty `mount_subpath` prefixes.
    if !path.is_absolute() {
        let norm_rel = lexical_normalize(path);
        let mut best_sub: Option<(&RepoMount, PathBuf)> = None;
        for mount in &frame.mounted_repos {
            if mount.mount_subpath.as_os_str().is_empty() {
                continue;
            }
            if let Ok(rel) = norm_rel.strip_prefix(&mount.mount_subpath) {
                let replace = match &best_sub {
                    None => true,
                    Some((prev, _)) => {
                        mount.mount_subpath.as_os_str().len() > prev.mount_subpath.as_os_str().len()
                    }
                };
                if replace {
                    best_sub = Some((mount, rel.to_path_buf()));
                }
            }
        }
        if let Some((mount, rel)) = best_sub {
            return Some((mount.repo_id.clone(), to_posix_rel(&rel)));
        }
    }

    None
}

/// Returns the deduplicated list of scopes visible to queries in `frame`.
pub fn permitted_scopes(frame: &ContextFrame) -> Vec<String> {
    let mut scopes = Vec::new();
    let mut seen = HashSet::new();

    let mut push = |s: String| {
        if seen.insert(s.clone()) {
            scopes.push(s);
        }
    };

    push("global".to_string());
    push(format!("machine:{}", frame.machine_id));
    if let Some(ch) = &frame.channel_id {
        push(format!("channel:{ch}"));
    }
    if let Some(sess) = &frame.session_id {
        push(format!("session:{sess}"));
    }
    for repo in &frame.mounted_repos {
        if repo.repo_id.starts_with("dir:") {
            push(repo.repo_id.clone());
        } else {
            push(format!("repo:{}", repo.repo_id));
        }
    }
    push(frame.cwd_dir_scope.clone());

    scopes
}

/// Canonicalizes a Git remote URL (SSH, HTTPS, HTTP, `git://`, or SCP-like)
/// into a stable `<lowercase-host>/<path-without-.git>` identifier.
pub fn canonicalize_git_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');

    // Scheme-based URL: https://, ssh://, git://, http://, file://
    if let Some(scheme_pos) = trimmed.find("://") {
        let scheme = &trimmed[..scheme_pos].to_ascii_lowercase();
        let rest = &trimmed[scheme_pos + 3..];
        if scheme == "file" {
            return strip_dot_git(rest.trim_end_matches('/')).to_string();
        }
        let (authority, path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx + 1..]),
            None => (rest, ""),
        };
        let host_port = match authority.rfind('@') {
            Some(at) => &authority[at + 1..],
            None => authority,
        };
        let host_port = strip_default_port(&host_port.to_ascii_lowercase(), scheme);
        let path = strip_dot_git(path.trim_matches('/'));
        if path.is_empty() {
            return host_port;
        }
        return format!("{host_port}/{path}");
    }

    // SCP-like SSH syntax: [user@]host:owner/repo.git
    if let Some(colon_pos) = trimmed.find(':') {
        let left = &trimmed[..colon_pos];
        if !left.contains('/') && !left.is_empty() {
            let host = match left.rfind('@') {
                Some(at) => &left[at + 1..],
                None => left,
            };
            let host = host.to_ascii_lowercase();
            let path = strip_dot_git(trimmed[colon_pos + 1..].trim_matches('/'));
            if path.is_empty() {
                return host;
            }
            return format!("{host}/{path}");
        }
    }

    strip_dot_git(trimmed).to_string()
}

/// Extracts `worktree:<parent_basename>` from a `gitdir:` target string such as
/// `/path/to/my-repo/.git/worktrees/feat-1`.
pub(crate) fn parse_worktree_parent_from_gitdir(gitdir_raw: &str) -> Option<String> {
    let normalized = gitdir_raw.trim().replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    for (idx, part) in parts.iter().enumerate() {
        if *part == ".git" && idx > 0 {
            let parent = parts[idx - 1];
            if !parent.is_empty() && parent != "." && parent != ".." {
                return Some(format!("worktree:{parent}"));
            }
        }
    }
    None
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn clean_opt(val: Option<&str>) -> Option<String> {
    val.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn make_dir_scope(machine_id: &str, path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let prefix = hex_encode(&digest[..8]); // 16 hex chars
    format!("dir:{machine_id}:{prefix}")
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn strip_dot_git(s: &str) -> &str {
    s.strip_suffix(".git").unwrap_or(s)
}

fn strip_default_port(host_port: &str, scheme: &str) -> String {
    let default_port = match scheme {
        "https" => Some(":443"),
        "http" => Some(":80"),
        "ssh" => Some(":22"),
        "git" => Some(":9418"),
        _ => None,
    };
    if let Some(suffix) = default_port {
        if let Some(stripped) = host_port.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    host_port.to_string()
}

fn find_enclosing_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let dot_git = dir.join(".git");
        if fs::symlink_metadata(&dot_git).is_ok() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn inspect_repo_root(
    repo_root: &Path,
    mount_subpath: PathBuf,
    fallback_dir_scope: &str,
    session_hints: &[SessionRepoHint],
) -> RepoMount {
    let (languages, lockfiles) = detect_languages_and_lockfiles(repo_root);
    let dot_git = repo_root.join(".git");
    let meta = match fs::metadata(&dot_git) {
        Ok(m) => m,
        Err(_) => {
            // `.git` exists (e.g. broken symlink or unreadable) — fail open.
            let repo_id = match_session_hint(repo_root, &mount_subpath, session_hints)
                .map(|h| h.repo_id.clone())
                .unwrap_or_else(|| fallback_dir_scope.to_string());
            return RepoMount {
                repo_id,
                root_path: repo_root.to_path_buf(),
                mount_subpath,
                branch: None,
                head_sha: None,
                sandboxed_worktree: true,
                languages,
                lockfiles,
            };
        }
    };

    let (git_dir, common_dir, raw_gitdir, sandboxed) = if meta.is_dir() {
        (dot_git.clone(), dot_git, None, false)
    } else {
        // `.git` is a file containing `gitdir: <path>`
        let content = fs::read_to_string(&dot_git).unwrap_or_default();
        let raw_target = content
            .lines()
            .find_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("gitdir:")
                    .map(|rest| rest.trim().to_string())
            });

        match raw_target {
            None => {
                let repo_id = match_session_hint(repo_root, &mount_subpath, session_hints)
                    .map(|h| h.repo_id.clone())
                    .unwrap_or_else(|| fallback_dir_scope.to_string());
                return RepoMount {
                    repo_id,
                    root_path: repo_root.to_path_buf(),
                    mount_subpath,
                    branch: None,
                    head_sha: None,
                    sandboxed_worktree: true,
                    languages,
                    lockfiles,
                };
            }
            Some(raw) => {
                let target_path = {
                    let p = PathBuf::from(&raw);
                    if p.is_absolute() {
                        p
                    } else {
                        repo_root.join(p)
                    }
                };
                if !is_dir_readable(&target_path) {
                    // Sandboxed worktree (TOP-3): target unreadable (EPERM / ENOENT).
                    let repo_id = match_session_hint(repo_root, &mount_subpath, session_hints)
                        .map(|h| h.repo_id.clone())
                        .or_else(|| parse_worktree_parent_from_gitdir(&raw))
                        .unwrap_or_else(|| fallback_dir_scope.to_string());
                    return RepoMount {
                        repo_id,
                        root_path: repo_root.to_path_buf(),
                        mount_subpath,
                        branch: None,
                        head_sha: None,
                        sandboxed_worktree: true,
                        languages,
                        lockfiles,
                    };
                }

                // Resolve `commondir` if present (standard git worktree layout).
                let commondir_file = target_path.join("commondir");
                let (common, common_sandboxed) = match fs::read_to_string(&commondir_file) {
                    Ok(rel) => {
                        let rel_trimmed = rel.trim();
                        let resolved = {
                            let p = PathBuf::from(rel_trimmed);
                            if p.is_absolute() {
                                p
                            } else {
                                target_path.join(p)
                            }
                        };
                        if is_dir_readable(&resolved) {
                            (resolved, false)
                        } else {
                            (target_path.clone(), true)
                        }
                    }
                    Err(_) => (target_path.clone(), false),
                };

                (target_path, common, Some(raw), common_sandboxed)
            }
        }
    };

    let (branch, head_sha) = read_head(&git_dir, &common_dir);

    let repo_id = if sandboxed {
        match_session_hint(repo_root, &mount_subpath, session_hints)
            .map(|h| h.repo_id.clone())
            .or_else(|| {
                raw_gitdir
                    .as_deref()
                    .and_then(parse_worktree_parent_from_gitdir)
            })
            .unwrap_or_else(|| fallback_dir_scope.to_string())
    } else {
        read_origin_url(&common_dir)
            .map(|u| canonicalize_git_url(&u))
            .or_else(|| {
                read_root_commit_sha(&common_dir, &git_dir)
                    .or_else(|| head_sha.clone())
                    .map(|sha| format!("git:{sha}"))
            })
            .or_else(|| {
                raw_gitdir
                    .as_deref()
                    .and_then(parse_worktree_parent_from_gitdir)
            })
            .or_else(|| {
                repo_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| format!("worktree:{name}"))
            })
            .unwrap_or_else(|| fallback_dir_scope.to_string())
    };

    RepoMount {
        repo_id,
        root_path: repo_root.to_path_buf(),
        mount_subpath,
        branch,
        head_sha,
        sandboxed_worktree: sandboxed,
        languages,
        lockfiles,
    }
}

fn is_dir_readable(path: &Path) -> bool {
    fs::read_dir(path).is_ok()
}

fn read_head(git_dir: &Path, common_dir: &Path) -> (Option<String>, Option<String>) {
    let head_content = match fs::read_to_string(git_dir.join("HEAD")) {
        Ok(s) => s,
        Err(_) => return (None, None),
    };
    let trimmed = head_content.trim();
    if let Some(ref_name) = trimmed.strip_prefix("ref:") {
        let ref_name = ref_name.trim();
        let branch = ref_name
            .strip_prefix("refs/heads/")
            .map(str::to_string);
        let sha = resolve_ref(git_dir, common_dir, ref_name);
        (branch, sha)
    } else if is_hex_sha(trimmed) {
        (None, Some(trimmed.to_string()))
    } else {
        (None, None)
    }
}

fn resolve_ref(git_dir: &Path, common_dir: &Path, ref_name: &str) -> Option<String> {
    for base in [git_dir, common_dir] {
        if let Ok(content) = fs::read_to_string(base.join(ref_name)) {
            let trimmed = content.trim();
            if is_hex_sha(trimmed) {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Ok(packed) = fs::read_to_string(common_dir.join("packed-refs")) {
        for line in packed.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            let mut parts = line.split_whitespace();
            if let (Some(sha), Some(name)) = (parts.next(), parts.next()) {
                if name == ref_name && is_hex_sha(sha) {
                    return Some(sha.to_string());
                }
            }
        }
    }
    None
}

fn read_origin_url(common_dir: &Path) -> Option<String> {
    let config = fs::read_to_string(common_dir.join("config")).ok()?;
    let mut in_origin = false;
    for raw_line in config.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let header = &line[1..line.len() - 1].trim();
            in_origin = is_remote_origin_header(header);
            continue;
        }
        if in_origin {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("url") {
                    let val = v.trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

fn is_remote_origin_header(header: &str) -> bool {
    let mut parts = header.splitn(2, char::is_whitespace);
    let section = match parts.next() {
        Some(s) => s,
        None => return false,
    };
    let subsection = match parts.next() {
        Some(s) => s.trim(),
        None => return false,
    };
    section.eq_ignore_ascii_case("remote") && subsection == "\"origin\""
}

fn read_root_commit_sha(common_dir: &Path, git_dir: &Path) -> Option<String> {
    for base in [common_dir, git_dir] {
        if let Ok(content) = fs::read_to_string(base.join("logs").join("HEAD")) {
            if let Some(first_line) = content.lines().next() {
                let mut parts = first_line.split_whitespace();
                if let (Some(old_sha), Some(new_sha)) = (parts.next(), parts.next()) {
                    if is_zero_sha(old_sha) && is_hex_sha(new_sha) {
                        return Some(new_sha.to_string());
                    }
                }
            }
        }
    }
    None
}

fn is_hex_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_zero_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b == b'0')
}

fn should_skip_dir_name(name: &str) -> bool {
    name.starts_with('.')
        || name == "node_modules"
        || name == "target"
        || name.starts_with("bazel-")
}

fn discover_child_repos(
    cwd: &Path,
    machine_id: &str,
    session_hints: &[SessionRepoHint],
) -> Vec<RepoMount> {
    let mut mounts = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut inspected: usize = 0;

    // Depth-1 scan
    let mut depth1_candidates: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = read_sorted_dir(cwd) {
        for (name, path, is_dir, is_symlink) in entries {
            if inspected >= MAX_SCAN_ENTRIES {
                break;
            }
            inspected += 1;
            if is_symlink || !is_dir || should_skip_dir_name(&name) {
                continue;
            }
            if fs::symlink_metadata(path.join(".git")).is_ok() {
                let fallback = make_dir_scope(machine_id, &path);
                let mount = inspect_repo_root(
                    &path,
                    PathBuf::from(&name),
                    &fallback,
                    session_hints,
                );
                seen_roots.insert(mount.root_path.clone());
                seen_ids.insert(mount.repo_id.clone());
                mounts.push(mount);
            } else {
                depth1_candidates.push((name, path));
            }
        }
    }

    // Depth-2 scan
    for (d1_name, d1_path) in depth1_candidates {
        if inspected >= MAX_SCAN_ENTRIES {
            break;
        }
        if let Ok(entries) = read_sorted_dir(&d1_path) {
            for (d2_name, d2_path, is_dir, is_symlink) in entries {
                if inspected >= MAX_SCAN_ENTRIES {
                    break;
                }
                inspected += 1;
                if is_symlink || !is_dir || should_skip_dir_name(&d2_name) {
                    continue;
                }
                if fs::symlink_metadata(d2_path.join(".git")).is_ok() {
                    let subpath = PathBuf::from(&d1_name).join(&d2_name);
                    let fallback = make_dir_scope(machine_id, &d2_path);
                    let mount = inspect_repo_root(
                        &d2_path,
                        subpath,
                        &fallback,
                        session_hints,
                    );
                    seen_roots.insert(mount.root_path.clone());
                    seen_ids.insert(mount.repo_id.clone());
                    mounts.push(mount);
                }
            }
        }
    }

    // Merge any `HUMATA_SESSION_REPOS` hints not already discovered on disk.
    for hint in session_hints {
        let root_path = hint
            .root_path
            .clone()
            .or_else(|| hint.mount_subpath.as_ref().map(|sub| cwd.join(sub)))
            .unwrap_or_else(|| cwd.to_path_buf());
        if seen_roots.contains(&root_path) || seen_ids.contains(&hint.repo_id) {
            continue;
        }
        let mount_subpath = hint
            .mount_subpath
            .clone()
            .or_else(|| root_path.strip_prefix(cwd).ok().map(Path::to_path_buf))
            .unwrap_or_default();
        seen_roots.insert(root_path.clone());
        seen_ids.insert(hint.repo_id.clone());
        let (languages, lockfiles) = detect_languages_and_lockfiles(&root_path);
        mounts.push(RepoMount {
            repo_id: hint.repo_id.clone(),
            root_path,
            mount_subpath,
            branch: hint.branch.clone(),
            head_sha: hint.head_sha.clone(),
            sandboxed_worktree: false,
            languages,
            lockfiles,
        });
    }

    mounts
}

fn read_sorted_dir(dir: &Path) -> std::io::Result<Vec<(String, PathBuf, bool, bool)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        out.push((name, entry.path(), ft.is_dir(), ft.is_symlink()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn parse_session_repos(raw: &str) -> Vec<SessionRepoHint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if trimmed.starts_with('[') {
        if let Ok(objs) = serde_json::from_str::<Vec<SessionRepoJson>>(trimmed) {
            return objs
                .into_iter()
                .filter(|o| !o.repo_id.trim().is_empty())
                .map(|o| SessionRepoHint {
                    repo_id: o.repo_id.trim().to_string(),
                    root_path: o.root_path,
                    mount_subpath: o.mount_subpath,
                    branch: o.branch,
                    head_sha: o.head_sha,
                })
                .collect();
        }
        if let Ok(strs) = serde_json::from_str::<Vec<String>>(trimmed) {
            return strs
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .map(|s| SessionRepoHint {
                    repo_id: s.trim().to_string(),
                    root_path: None,
                    mount_subpath: None,
                    branch: None,
                    head_sha: None,
                })
                .collect();
        }
    }

    trimmed
        .split([',', ';'])
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                return None;
            }
            if let Some((sub, repo)) = item.split_once('=') {
                let sub = sub.trim();
                let repo = repo.trim();
                if repo.is_empty() {
                    return None;
                }
                Some(SessionRepoHint {
                    repo_id: repo.to_string(),
                    root_path: None,
                    mount_subpath: if sub.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(sub))
                    },
                    branch: None,
                    head_sha: None,
                })
            } else {
                Some(SessionRepoHint {
                    repo_id: item.to_string(),
                    root_path: None,
                    mount_subpath: None,
                    branch: None,
                    head_sha: None,
                })
            }
        })
        .collect()
}

fn match_session_hint<'a>(
    repo_root: &Path,
    mount_subpath: &Path,
    hints: &'a [SessionRepoHint],
) -> Option<&'a SessionRepoHint> {
    if hints.is_empty() {
        return None;
    }
    if let Some(found) = hints.iter().find(|h| {
        h.root_path
            .as_deref()
            .is_some_and(|p| lexical_normalize(p) == lexical_normalize(repo_root))
    }) {
        return Some(found);
    }
    if !mount_subpath.as_os_str().is_empty() {
        if let Some(found) = hints
            .iter()
            .find(|h| h.mount_subpath.as_deref() == Some(mount_subpath))
        {
            return Some(found);
        }
    }
    if hints.len() == 1 {
        return Some(&hints[0]);
    }
    None
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn to_posix_rel(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_fake_git_repo(root: &Path, origin_url: Option<&str>, branch: &str, sha: &str) {
        let git_dir = root.join(".git");
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        fs::write(git_dir.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
        fs::write(git_dir.join("refs/heads").join(branch), format!("{sha}\n")).unwrap();
        if let Some(url) = origin_url {
            fs::write(
                git_dir.join("config"),
                format!("[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = {url}\n"),
            )
            .unwrap();
        } else {
            fs::write(git_dir.join("config"), "[core]\n\trepositoryformatversion = 0\n").unwrap();
        }
    }

    #[test]
    fn canonicalizes_ssh_and_https_urls_identically() {
        assert_eq!(
            canonicalize_git_url("git@GitHub.com:Owner/Repo.git"),
            "github.com/Owner/Repo"
        );
        assert_eq!(
            canonicalize_git_url("https://user:token@GITHUB.COM:443/Owner/Repo.git/"),
            "github.com/Owner/Repo"
        );
        assert_eq!(
            canonicalize_git_url("ssh://git@github.com:22/Owner/Repo.git"),
            "github.com/Owner/Repo"
        );
    }

    #[test]
    fn single_repo_with_origin_and_subdir_normalization() {
        let tmp = tempdir().unwrap();
        let sha = "a".repeat(40);
        init_fake_git_repo(
            tmp.path(),
            Some("git@github.com:acme/lore.git"),
            "main",
            &sha,
        );
        let sub = tmp.path().join("src/deep");
        fs::create_dir_all(&sub).unwrap();

        let env = EnvHints {
            machine_id: Some("mac-1".into()),
            channel_id: Some("ch-1".into()),
            ..Default::default()
        };
        let frame = detect(&sub, &env);
        assert_eq!(frame.mounted_repos.len(), 1);
        let m = &frame.mounted_repos[0];
        assert_eq!(m.repo_id, "github.com/acme/lore");
        assert_eq!(m.branch.as_deref(), Some("main"));
        assert_eq!(m.head_sha.as_deref(), Some(sha.as_str()));
        assert!(!m.sandboxed_worktree);

        assert_eq!(
            normalize_path_to_repo(&frame, Path::new("mod.rs")),
            Some(("github.com/acme/lore".into(), "src/deep/mod.rs".into()))
        );
        let scopes = permitted_scopes(&frame);
        assert!(scopes.contains(&"global".to_string()));
        assert!(scopes.contains(&"machine:mac-1".to_string()));
        assert!(scopes.contains(&"channel:ch-1".to_string()));
        assert!(scopes.contains(&"repo:github.com/acme/lore".to_string()));
    }

    #[test]
    fn repo_without_origin_uses_root_commit_sha() {
        let tmp = tempdir().unwrap();
        let root_sha = "1".repeat(40);
        let head_sha = "2".repeat(40);
        init_fake_git_repo(tmp.path(), None, "main", &head_sha);
        fs::create_dir_all(tmp.path().join(".git/logs")).unwrap();
        fs::write(
            tmp.path().join(".git/logs/HEAD"),
            format!("{} {root_sha} Dev <dev@x> 1 +0000\tcommit (initial): init\n", "0".repeat(40)),
        )
        .unwrap();

        let frame = detect(tmp.path(), &EnvHints::default());
        assert_eq!(frame.mounted_repos.len(), 1);
        assert_eq!(frame.mounted_repos[0].repo_id, format!("git:{root_sha}"));
    }

    #[test]
    fn sandboxed_worktree_falls_back_gracefully() {
        let tmp = tempdir().unwrap();
        let missing_gitdir = "/nonexistent/outside-sandbox/my-cool-repo/.git/worktrees/wt-1";
        fs::write(tmp.path().join(".git"), format!("gitdir: {missing_gitdir}\n")).unwrap();

        // Without HUMATA_SESSION_REPOS -> parses worktree:<parent_basename>
        let frame = detect(tmp.path(), &EnvHints { machine_id: Some("m1".into()), ..Default::default() });
        assert_eq!(frame.mounted_repos.len(), 1);
        assert!(frame.mounted_repos[0].sandboxed_worktree);
        assert_eq!(frame.mounted_repos[0].repo_id, "worktree:my-cool-repo");

        // With HUMATA_SESSION_REPOS -> prefers session hint
        let env_with_hint = EnvHints {
            machine_id: Some("m1".into()),
            session_repos: Some("github.com/acme/my-cool-repo".into()),
            ..Default::default()
        };
        let frame_hint = detect(tmp.path(), &env_with_hint);
        assert!(frame_hint.mounted_repos[0].sandboxed_worktree);
        assert_eq!(frame_hint.mounted_repos[0].repo_id, "github.com/acme/my-cool-repo");

        // Unparseable gitdir without hint -> falls back to dir:<machine>:<hash>
        fs::write(tmp.path().join(".git"), "gitdir: /unreadable/opaque/path\n").unwrap();
        let frame_dir = detect(tmp.path(), &EnvHints { machine_id: Some("m1".into()), ..Default::default() });
        assert!(frame_dir.mounted_repos[0].sandboxed_worktree);
        assert_eq!(frame_dir.mounted_repos[0].repo_id, frame_dir.cwd_dir_scope);
        assert!(frame_dir.cwd_dir_scope.starts_with("dir:m1:"));
    }

    #[test]
    fn multi_repo_parent_dir_discovers_children_and_normalizes_paths() {
        let tmp = tempdir().unwrap();
        let sha_a = "a".repeat(40);
        let sha_b = "b".repeat(40);
        fs::create_dir_all(tmp.path().join("api")).unwrap();
        fs::create_dir_all(tmp.path().join("nested/web")).unwrap();
        fs::create_dir_all(tmp.path().join("node_modules/ignored")).unwrap();
        init_fake_git_repo(&tmp.path().join("api"), Some("https://github.com/acme/api.git"), "main", &sha_a);
        init_fake_git_repo(&tmp.path().join("nested/web"), Some("https://github.com/acme/web.git"), "dev", &sha_b);
        init_fake_git_repo(&tmp.path().join("node_modules/ignored"), Some("https://github.com/acme/ignored.git"), "main", &sha_a);

        let frame = detect(tmp.path(), &EnvHints { machine_id: Some("m1".into()), ..Default::default() });
        let ids: Vec<&str> = frame.mounted_repos.iter().map(|r| r.repo_id.as_str()).collect();
        assert_eq!(ids, vec!["github.com/acme/api", "github.com/acme/web"]);

        assert_eq!(
            normalize_path_to_repo(&frame, Path::new("api/src/lib.rs")),
            Some(("github.com/acme/api".into(), "src/lib.rs".into()))
        );
        assert_eq!(
            normalize_path_to_repo(&frame, Path::new("nested/web/app.ts")),
            Some(("github.com/acme/web".into(), "app.ts".into()))
        );
    }

    #[test]
    fn non_git_dir_has_empty_mounted_repos_and_valid_scopes() {
        let tmp = tempdir().unwrap();
        let env = EnvHints {
            machine_id: Some("desk-1".into()),
            channel_id: Some("ops".into()),
            ..Default::default()
        };
        let frame = detect(tmp.path(), &env);
        assert!(frame.mounted_repos.is_empty());
        assert!(frame.cwd_dir_scope.starts_with("dir:desk-1:"));

        let scopes = permitted_scopes(&frame);
        assert_eq!(
            scopes,
            vec![
                "global".to_string(),
                "machine:desk-1".to_string(),
                "channel:ops".to_string(),
                frame.cwd_dir_scope.clone(),
            ]
        );
    }

    #[test]
    fn detects_polyglot_languages_and_lockfiles() {
        let tmp = tempdir().unwrap();
        init_fake_git_repo(
            tmp.path(),
            Some("https://github.com/acme/poly.git"),
            "main",
            &"d".repeat(40),
        );
        fs::write(tmp.path().join("Cargo.lock"), "[[package]]\nname=\"a\"\nversion=\"1.0.0\"\n").unwrap();
        fs::write(tmp.path().join("go.mod"), "module example.com/poly\n").unwrap();
        fs::write(tmp.path().join("MODULE.bazel.lock"), "{}").unwrap();

        let frame = detect(tmp.path(), &EnvHints::default());
        assert_eq!(frame.mounted_repos.len(), 1);
        let m = &frame.mounted_repos[0];
        assert!(m.is_polyglot());
        assert_eq!(m.primary_language(), Language::Polyglot);
        assert!(m.languages.contains(&Language::Rust));
        assert!(m.languages.contains(&Language::Go));
        assert!(m.languages.contains(&Language::Bazel));
        assert_eq!(m.lockfiles.len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn sandboxed_worktree_with_chmod_000_target_never_panics() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let locked_parent = tmp.path().join("host-repo/.git/worktrees/wt-locked");
        fs::create_dir_all(&locked_parent).unwrap();
        fs::set_permissions(&locked_parent, fs::Permissions::from_mode(0o000)).unwrap();

        let wt_dir = tmp.path().join("wt-sandbox");
        fs::create_dir_all(&wt_dir).unwrap();
        fs::write(
            wt_dir.join(".git"),
            format!("gitdir: {}\n", locked_parent.display()),
        )
        .unwrap();

        let frame = detect(&wt_dir, &EnvHints { machine_id: Some("m1".into()), ..Default::default() });
        // Restore permissions before assertions so tempdir cleanup always succeeds.
        let _ = fs::set_permissions(&locked_parent, fs::Permissions::from_mode(0o755));

        assert_eq!(frame.mounted_repos.len(), 1);
        assert!(frame.mounted_repos[0].sandboxed_worktree);
        assert_eq!(frame.mounted_repos[0].repo_id, "worktree:host-repo");
    }
}

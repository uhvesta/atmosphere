//! macOS containment: resolves every path, then writes the sandbox profile
//! that `sandbox-exec` runs the command under.
//!
//! The profile allows everything by default, then denies writes everywhere,
//! reads of the hidden folders and writes to the read-only paths. Later rules
//! win, so each allowance comes after the denial that would cover it. The
//! network is denied except HTTPS, DNS and the daemon's socket. The localhost
//! denials come after the HTTPS and DNS allowances, so nothing on this machine
//! answers over TCP or UDP.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use crate::{ContainError, Containment};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const ENV: &str = "/usr/bin/env";

/// The resolver's unix socket. DNS may use it, as well as UDP port 53.
const MDNS_SOCKET: &str = "/private/var/run/mDNSResponder";

/// Writes that stay allowed for every program: the null and terminal devices
/// and the descriptors of the process. Programs break without them.
const DEVICE_WRITES: &[&str] = &[
    "(literal \"/dev/null\")",
    "(literal \"/dev/zero\")",
    "(literal \"/dev/tty\")",
    "(regex #\"^/dev/tty\")",
    "(regex #\"^/dev/fd/\")",
];

/// The characters a regex reads with a meaning of their own, plus the quote
/// that ends the `#"..."` literal. Each one is escaped in a prefix regex.
const REGEX_SPECIAL: &str = "\\.^$|?*+()[]{}\"";

/// Every path the profile names, resolved to where the kernel finds it.
struct Resolved {
    session: PathBuf,
    shared: PathBuf,
    temp: PathBuf,
    also_writable: Vec<PathBuf>,
    also_readable: Vec<PathBuf>,
    agent_state: Vec<PathBuf>,
    read_only: Vec<PathBuf>,
    hidden: Vec<PathBuf>,
    socket: PathBuf,
}

pub(crate) fn wrap(
    containment: &Containment,
    argv: &[OsString],
) -> Result<Vec<OsString>, ContainError> {
    if argv.is_empty() {
        return Err(ContainError::NoCommand);
    }
    let resolved = resolve(containment)?;
    let profile = profile(&resolved)?;

    let mut tmpdir = OsString::from("TMPDIR=");
    tmpdir.push(resolved.temp.as_os_str());
    tmpdir.push("/");

    let mut wrapped = vec![
        OsString::from(SANDBOX_EXEC),
        OsString::from("-p"),
        OsString::from(profile),
        OsString::from(ENV),
        tmpdir,
    ];
    for (name, value) in &containment.env {
        let mut assignment = name.clone();
        assignment.push("=");
        assignment.push(value);
        wrapped.push(assignment);
    }
    wrapped.extend(argv.iter().cloned());
    Ok(wrapped)
}

fn resolve(containment: &Containment) -> Result<Resolved, ContainError> {
    // The session folder is checked first, so a missing one is reported
    // rather than recreated by the temp folder inside it.
    let session = resolve_session(&containment.session_folder)?;
    let temp = make_temp(&containment.temp)?;
    let shared = resolve_folder("shared folder", &containment.shared)?;
    let also_writable = containment
        .also_writable
        .iter()
        .map(|folder| resolve_folder("also-writable folder", folder))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Resolved {
        session,
        shared,
        temp,
        also_writable,
        also_readable: resolve_all("also-readable path", &containment.also_readable)?,
        agent_state: resolve_all("agent state", &containment.agent_state)?,
        read_only: resolve_all("read-only path", &containment.read_only)?,
        hidden: resolve_all("hidden folder", &containment.hidden)?,
        socket: resolve_socket(&containment.socket)?,
    })
}

fn resolve_session(path: &Path) -> Result<PathBuf, ContainError> {
    std::fs::canonicalize(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ContainError::SessionFolderMissing {
                path: path.to_path_buf(),
                source,
            }
        } else {
            ContainError::Unresolved {
                role: "session folder",
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

/// Creates the temp folder if it is missing, then resolves it.
fn make_temp(path: &Path) -> Result<PathBuf, ContainError> {
    std::fs::create_dir_all(path).map_err(|source| ContainError::TempFolder {
        path: path.to_path_buf(),
        source,
    })?;
    resolve_folder("temp folder", path)
}

fn resolve_folder(role: &'static str, path: &Path) -> Result<PathBuf, ContainError> {
    std::fs::canonicalize(path).map_err(|source| ContainError::Unresolved {
        role,
        path: path.to_path_buf(),
        source,
    })
}

/// Resolves each path that may not exist yet, leaving out those that name
/// nothing on disk.
fn resolve_all(role: &'static str, paths: &[PathBuf]) -> Result<Vec<PathBuf>, ContainError> {
    let mut resolved = Vec::new();
    for path in paths {
        if let Some(path) = resolve_maybe_missing(role, path)? {
            resolved.push(path);
        }
    }
    Ok(resolved)
}

/// A path that doesn't exist yet resolves through its nearest existing
/// ancestor, so the rule still applies once the path appears. `None` only
/// when nothing on the path can be named at all.
fn resolve_maybe_missing(
    role: &'static str,
    path: &Path,
) -> Result<Option<PathBuf>, ContainError> {
    match std::fs::canonicalize(path) {
        Ok(resolved) => Ok(Some(resolved)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
                    Ok(resolve_maybe_missing(role, parent)?.map(|folder| folder.join(name)))
                }
                _ => Ok(None),
            }
        }
        Err(source) => Err(ContainError::Unresolved {
            role,
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The socket is resolved through its folder, since the socket file itself
/// is created by the daemon and may not be looked up before then.
fn resolve_socket(socket: &Path) -> Result<PathBuf, ContainError> {
    let Some(name) = socket.file_name() else {
        return Err(ContainError::SocketWithoutFileName {
            path: socket.to_path_buf(),
        });
    };
    let folder = match socket.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    Ok(resolve_folder("daemon socket folder", folder)?.join(name))
}

fn profile(resolved: &Resolved) -> Result<String, ContainError> {
    let mut allowed = vec![
        subpath("session folder", &resolved.session)?,
        subpath("shared folder", &resolved.shared)?,
        subpath("temp folder", &resolved.temp)?,
    ];
    for folder in &resolved.also_writable {
        allowed.push(subpath("also-writable folder", folder)?);
    }
    for path in &resolved.agent_state {
        allowed.push(regex_prefix("agent state", path)?);
    }

    let mut write_filters = allowed.clone();
    write_filters.extend(DEVICE_WRITES.iter().map(|filter| filter.to_string()));

    let mut rules = vec![
        "(version 1)".to_string(),
        "(allow default)".to_string(),
        "(deny file-write*)".to_string(),
        allow("file-write*", &write_filters),
    ];
    for folder in &resolved.hidden {
        rules.push(format!(
            "(deny file-read* {})",
            subpath("hidden folder", folder)?
        ));
    }
    let mut read_filters = allowed;
    for path in &resolved.also_readable {
        read_filters.push(subpath("also-readable path", path)?);
    }
    rules.push(allow("file-read*", &read_filters));
    for path in &resolved.read_only {
        rules.push(format!(
            "(deny file-write* {})",
            subpath("read-only path", path)?
        ));
    }

    rules.push("(deny network*)".to_string());
    rules.push("(allow network-outbound (remote tcp \"*:443\"))".to_string());
    rules.push("(allow network-outbound (remote tcp \"*:53\") (remote udp \"*:53\"))".to_string());
    rules.push(format!(
        "(allow network-outbound (remote unix-socket (path-literal {})))",
        quoted("DNS socket", Path::new(MDNS_SOCKET))?
    ));
    rules.push("(deny network-outbound (remote tcp \"localhost:*\"))".to_string());
    rules.push("(deny network-outbound (remote udp \"localhost:*\"))".to_string());
    rules.push(format!(
        "(allow network-outbound (remote unix-socket (path-literal {})))",
        quoted("daemon socket", &resolved.socket)?
    ));

    Ok(rules.join("\n") + "\n")
}

/// One allow rule with its filters, one per line.
fn allow(operation: &str, filters: &[String]) -> String {
    let mut rule = format!("(allow {operation}");
    for filter in filters {
        rule.push_str("\n  ");
        rule.push_str(filter);
    }
    rule.push(')');
    rule
}

fn subpath(role: &'static str, path: &Path) -> Result<String, ContainError> {
    Ok(format!("(subpath {})", quoted(role, path)?))
}

/// A regex that matches the path, everything under it, and its siblings that
/// add a dot-suffix to its name, so a state file's atomic-write temp files
/// (`.claude.json.tmp.123`) are covered too. A longer name such as
/// `.claude-backup` is not. Regex characters and quotes in the path are
/// escaped, so they match only themselves.
fn regex_prefix(role: &'static str, path: &Path) -> Result<String, ContainError> {
    let text = path.to_str().ok_or_else(|| ContainError::NotUtf8 {
        role,
        path: path.to_path_buf(),
    })?;
    let mut escaped = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        if REGEX_SPECIAL.contains(ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    Ok(format!("(regex #\"^{escaped}(/|\\.|$)\")"))
}

/// A path as an SBPL string literal: backslashes and quotes escaped.
fn quoted(role: &'static str, path: &Path) -> Result<String, ContainError> {
    let text = path.to_str().ok_or_else(|| ContainError::NotUtf8 {
        role,
        path: path.to_path_buf(),
    })?;
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!("\"{escaped}\""))
}

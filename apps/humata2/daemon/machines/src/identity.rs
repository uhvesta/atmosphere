//! The JSON an `identity` command prints (MP-8), checked field by field
//! (MP-11). It holds paths only; output with key material is refused whole
//! and never kept or shown.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::base64;
use crate::error::{FieldProblem, IdentityError};
use crate::name::MachineName;
use crate::platform::{Arch, Os, Platform};
use crate::provider::{parse_status_word, MachineStatus};

/// Where a machine is and how to log in to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub ssh: SshTarget,
    pub platform: Platform,
    /// Optional machine state reported alongside identity.
    pub state: Option<MachineStatus>,
    /// Optional provider-assigned instance identifier.
    pub instance_id: Option<String>,
}

/// An SSH destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub host: String,
    pub access: SshAccess,
    /// Pinned with no fingerprint prompt when given (MP-9).
    pub host_key: Option<HostKey>,
}

/// Either an ssh config file that knows `host`, or the details themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshAccess {
    Config {
        ssh_config: String,
    },
    Direct {
        port: u16,
        user: String,
        identity_file: Option<String>,
    },
}

/// A public host key, as in `known_hosts`: `ssh-ed25519 AAAA…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    algorithm: String,
    key: String,
    fingerprint: String,
}

pub(crate) const KEY_ALGORITHMS: [&str; 7] = [
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Top-level JSON keys accepted by [`parse_identity`]. Shared with `spec` so
/// the schema cannot drift from the parser.
pub(crate) const IDENTITY_KEYS: [&str; 6] = ["name", "ssh", "os", "arch", "state", "instance_id"];

/// JSON keys accepted inside `ssh` objects by [`parse_ssh_object`]. Shared
/// with `spec` so the schema cannot drift from the parser.
pub(crate) const SSH_KEYS: [&str; 6] = [
    "host",
    "ssh_config",
    "port",
    "user",
    "identity_file",
    "host_key",
];

/// Whether `bytes` holds anything that looks like a private key.
pub(crate) fn has_key_material(bytes: &[u8]) -> bool {
    const MARKERS: [&[u8]; 3] = [b"PRIVATE KEY", b"-----BEGIN", b"PuTTY-User-Key-File"];
    MARKERS
        .iter()
        .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
}

/// Reads an identity command's stdout for `machine`.
pub fn parse_identity(stdout: &[u8], machine: &MachineName) -> Result<Identity, IdentityError> {
    if has_key_material(stdout) {
        return Err(IdentityError::KeyMaterial);
    }
    let value: Value =
        serde_json::from_slice(stdout).map_err(|source| IdentityError::NotJson { source })?;
    let Value::Object(top) = value else {
        return Err(field("", "output", FieldProblem::NotObject));
    };
    known(&top, "", &IDENTITY_KEYS)?;

    let name = required(&top, "", "name")?;
    if name != machine.as_str() {
        return Err(field(
            "",
            "name",
            FieldProblem::WrongName {
                expected: machine.as_str().to_owned(),
                found: name.to_owned(),
            },
        ));
    }

    let ssh = match top.get("ssh") {
        Some(Value::Object(ssh)) => ssh,
        Some(_) => return Err(field("", "ssh", FieldProblem::NotObject)),
        None => return Err(field("", "ssh", FieldProblem::Missing)),
    };
    let ssh = parse_ssh_object(ssh, "ssh.")?;

    let os = required(&top, "", "os")?;
    let os = Os::parse(os).ok_or_else(|| {
        field(
            "",
            "os",
            FieldProblem::UnknownOs {
                value: os.to_owned(),
            },
        )
    })?;
    let arch = required(&top, "", "arch")?;
    let arch = Arch::parse(arch).ok_or_else(|| {
        field(
            "",
            "arch",
            FieldProblem::UnknownArch {
                value: arch.to_owned(),
            },
        )
    })?;
    let state = optional(&top, "", "state")?.map(parse_status_word);
    let instance_id = optional(&top, "", "instance_id")?
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);

    Ok(Identity {
        ssh,
        platform: Platform { os, arch },
        state,
        instance_id,
    })
}

/// Validates and parses an `ssh` JSON object from `identity`, `status`, or `list`.
pub(crate) fn parse_ssh_object(ssh: &Map<String, Value>, prefix: &str) -> Result<SshTarget, IdentityError> {
    known(ssh, prefix, &SSH_KEYS)?;
    let host = word(prefix, "host", required(ssh, prefix, "host")?)?;
    let access = match optional(ssh, prefix, "ssh_config")? {
        Some(ssh_config) => {
            for key in ["port", "user", "identity_file"] {
                if ssh.get(key).is_some_and(|value| !value.is_null()) {
                    return Err(field(prefix, key, FieldProblem::ConflictsWithConfig));
                }
            }
            SshAccess::Config {
                ssh_config: path(prefix, "ssh_config", ssh_config)?,
            }
        }
        None => SshAccess::Direct {
            port: port(ssh, prefix)?,
            user: word(prefix, "user", required(ssh, prefix, "user")?)?,
            identity_file: optional(ssh, prefix, "identity_file")?
                .map(|value| path(prefix, "identity_file", value))
                .transpose()?,
        },
    };
    let host_key = optional(ssh, prefix, "host_key")?
        .map(|value| {
            HostKey::parse(value).ok_or_else(|| field(prefix, "host_key", FieldProblem::BadHostKey))
        })
        .transpose()?;
    Ok(SshTarget {
        host,
        access,
        host_key,
    })
}

impl HostKey {
    /// `<algorithm> <base64>`, with an optional comment after. The base64
    /// must decode to an SSH key blob of that algorithm.
    pub fn parse(value: &str) -> Option<Self> {
        let mut words = value.split_whitespace();
        let algorithm = words.next()?;
        let key = words.next()?;
        if !KEY_ALGORITHMS.contains(&algorithm) {
            return None;
        }
        let blob = base64::decode(key)?;
        // The blob starts with its own algorithm name, length first.
        let len = usize::try_from(u32::from_be_bytes(blob.get(..4)?.try_into().ok()?)).ok()?;
        if blob.get(4..4 + len)? != algorithm.as_bytes() || blob.len() <= 4 + len {
            return None;
        }
        Some(Self {
            algorithm: algorithm.to_owned(),
            fingerprint: format!("SHA256:{}", base64::encode(&Sha256::digest(&blob))),
            key: key.to_owned(),
        })
    }

    /// Such as `ssh-ed25519`.
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// The key's base64, as in `known_hosts`.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// `SHA256:…`, as `ssh` and `ssh-keygen -l` print it, for the user to
    /// compare before trusting a key.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

fn field(prefix: &str, key: &str, problem: FieldProblem) -> IdentityError {
    IdentityError::Field {
        field: format!("{prefix}{key}"),
        problem,
    }
}

fn known(object: &Map<String, Value>, prefix: &str, keys: &[&str]) -> Result<(), IdentityError> {
    match object.keys().find(|key| !keys.contains(&key.as_str())) {
        Some(key) => Err(field(prefix, key, FieldProblem::Unknown)),
        None => Ok(()),
    }
}

fn optional<'a>(
    object: &'a Map<String, Value>,
    prefix: &str,
    key: &str,
) -> Result<Option<&'a str>, IdentityError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(field(prefix, key, FieldProblem::NotString)),
    }
}

fn required<'a>(
    object: &'a Map<String, Value>,
    prefix: &str,
    key: &str,
) -> Result<&'a str, IdentityError> {
    optional(object, prefix, key)?.ok_or_else(|| field(prefix, key, FieldProblem::Missing))
}

/// A host or user name: no spaces, no control characters, no leading `-`
/// that ssh would read as an option.
fn word(prefix: &str, key: &str, value: &str) -> Result<String, IdentityError> {
    if value.is_empty() {
        return Err(field(prefix, key, FieldProblem::Empty));
    }
    if value.starts_with('-') || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(field(prefix, key, FieldProblem::BadText));
    }
    Ok(value.to_owned())
}

/// A path: spaces allowed, no control characters (a key's newlines), no
/// leading `-`.
fn path(prefix: &str, key: &str, value: &str) -> Result<String, IdentityError> {
    if value.is_empty() {
        return Err(field(prefix, key, FieldProblem::Empty));
    }
    if value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(field(prefix, key, FieldProblem::BadText));
    }
    Ok(value.to_owned())
}

fn port(ssh: &Map<String, Value>, prefix: &str) -> Result<u16, IdentityError> {
    let port = match ssh.get("port") {
        None | Some(Value::Null) => return Ok(22),
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.parse::<u64>().ok(),
        Some(_) => None,
    };
    port.and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| field(prefix, "port", FieldProblem::NotPort))
}

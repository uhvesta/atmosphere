//! Bounded workspace records. The holder/journal formats are separate.
use super::Saved;
use crate::{Error, error::disk};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const MAX_BYTES: u64 = 16 << 20;
const MAX_SESSIONS: usize = 256;

pub(super) fn validate_startup_key(profile: &str, path: &Path) -> Result<(), Error> {
    if profile.is_empty() || profile.len() > 64 || profile.chars().any(char::is_control) {
        return Err(invalid(
            path,
            "startup profile keys must contain 1–64 bytes without control characters",
        ));
    }
    Ok(())
}

fn invalid(path: &Path, reason: &'static str) -> Error {
    Error::MetadataStructure {
        path: path.into(),
        reason,
    }
}

pub(super) fn read(path: &Path) -> Result<Option<Saved>, Error> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::MetadataRead {
                path: path.into(),
                source,
            });
        }
    };
    let info = file.metadata().map_err(|source| Error::MetadataRead {
        path: path.into(),
        source,
    })?;
    if !info.is_file() {
        return Err(Error::MetadataFileType { path: path.into() });
    }
    if info.len() > MAX_BYTES {
        return Err(Error::MetadataSize {
            path: path.into(),
            bytes: info.len(),
            limit: MAX_BYTES,
            source: None,
        });
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| Error::MetadataRead {
            path: path.into(),
            source,
        })?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::MetadataSize {
            path: path.into(),
            bytes: bytes.len() as u64,
            limit: MAX_BYTES,
            source: None,
        });
    }
    let saved = serde_json::from_slice(&bytes).map_err(|source| Error::State {
        path: path.into(),
        source,
    })?;
    validate(&saved, path)?;
    Ok(Some(saved))
}

pub(super) fn validate(saved: &Saved, path: &Path) -> Result<(), Error> {
    if !matches!(saved.version, 1 | 2 | 3)
        || saved.namespace.len() != 32
        || !saved.namespace.bytes().all(|b| b.is_ascii_hexdigit())
        || saved.next == 0
        || saved.next == u64::MAX
    {
        return Err(invalid(
            path,
            "workspace version, namespace or next identifier is invalid",
        ));
    }
    if saved.sessions.len() > MAX_SESSIONS {
        return Err(Error::MetadataSessions {
            path: path.into(),
            count: saved.sessions.len(),
            limit: MAX_SESSIONS,
        });
    }
    let mut ids = BTreeSet::new();
    let mut startup_profiles = BTreeSet::new();
    let mut uuids = BTreeSet::new();
    for session in &saved.sessions {
        crate::identity::validate_session(session, saved.version < 3)?;
        if !session.uuid.is_empty() && !uuids.insert(&session.uuid) {
            return Err(invalid(path, "terminal UUIDs are repeated"));
        }
        if session.id == 0 || session.id >= saved.next || !ids.insert(session.id) {
            return Err(invalid(
                path,
                "session identifiers are repeated or outside the valid range",
            ));
        }
        if let Some(profile) = &session.startup_profile {
            validate_startup_key(profile, path)?;
            if !startup_profiles.insert(profile) {
                return Err(invalid(path, "startup profile keys are repeated"));
            }
        }
    }
    let mut placed = BTreeSet::new();
    for group in &saved.groups {
        for id in group {
            if !ids.contains(id) || !placed.insert(*id) {
                return Err(invalid(
                    path,
                    "terminal groups contain an unknown or repeated session identifier",
                ));
            }
        }
    }
    for (group, selected) in saved.groups.iter().zip(saved.selected) {
        if selected.is_some_and(|id| !group.contains(&id)) {
            return Err(invalid(
                path,
                "a selected terminal does not belong to its group",
            ));
        }
    }
    Ok(())
}

pub(super) fn before_create(saved: &Saved, path: &Path) -> Result<(), Error> {
    if saved.sessions.len() >= MAX_SESSIONS {
        return Err(Error::MetadataSessions {
            path: path.into(),
            count: saved.sessions.len() + 1,
            limit: MAX_SESSIONS,
        });
    }
    // Advancing to MAX would write a cursor that the reader cannot reopen.
    if saved.next >= u64::MAX - 1 {
        return Err(Error::IdentifierExhausted {
            path: path.into(),
            next: saved.next,
        });
    }
    Ok(())
}

struct BoundedWriter {
    bytes: Vec<u8>,
    required: Option<u64>,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let required = (self.bytes.len() as u64).saturating_add(bytes.len() as u64);
        if required > MAX_BYTES {
            self.required = Some(required);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "terminal workspace metadata size limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode(saved: &Saved, path: &Path) -> Result<Vec<u8>, Error> {
    validate(saved, path)?;
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        required: None,
    };
    if let Err(source) = serde_json::to_writer_pretty(&mut writer, saved) {
        return Err(match writer.required {
            Some(bytes) => Error::MetadataSize {
                path: path.into(),
                bytes,
                limit: MAX_BYTES,
                source: Some(source),
            },
            None => Error::State {
                path: path.into(),
                source,
            },
        });
    }
    Ok(writer.bytes)
}

pub(super) fn check_output_file(file: &std::fs::File, path: &Path) -> Result<(), Error> {
    let info = file
        .metadata()
        .map_err(|source| disk("inspect metadata temporary file", path, source))?;
    if !info.is_file() {
        return Err(Error::MetadataFileType { path: path.into() });
    }
    if info.nlink() != 1 {
        return Err(Error::MetadataSharedTemporary {
            path: path.into(),
            links: info.nlink(),
        });
    }
    Ok(())
}

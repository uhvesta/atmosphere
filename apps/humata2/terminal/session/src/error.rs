use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Terminal UUID is invalid ({reason}). Use the canonical UUID returned by terminal metadata; numeric routes and names are not UUIDs."
    )]
    UuidFormat { reason: &'static str },
    #[error(
        "Could not parse the terminal UUID. Use the canonical UUID returned by terminal metadata."
    )]
    UuidParse {
        #[source]
        source: uuid::Error,
    },
    #[error(
        "Terminal UUID {uuid} is not in this workspace. Read the current terminal metadata instead of starting a replacement process."
    )]
    MissingUuid { uuid: String },
    #[error(
        "Terminal {id} metadata field {field} is invalid ({reason}; {actual} bytes/items, limit {limit}). Supply bounded non-secret wrapper metadata; the saved state was preserved."
    )]
    SessionMetadata {
        id: u64,
        field: &'static str,
        actual: usize,
        limit: usize,
        reason: &'static str,
    },
    #[error("Could not securely read terminal workspace metadata at {}. Use a private regular file rather than a symlink or device; preserve existing state and restore a compatible backup before retrying.",path.display())]
    MetadataRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Terminal workspace metadata at {} must be a regular file. Preserve it for inspection and use a separate private state folder; pipes, sockets and devices are not supported.",path.display())]
    MetadataFileType { path: PathBuf },
    #[error("Terminal workspace temporary file {} has {links} hard links. Preserve the linked files and use a separate private workspace; truncating this entry could damage another file.",path.display())]
    MetadataSharedTemporary { path: PathBuf, links: u64 },
    #[error("Terminal workspace metadata at {} needs at least {bytes} bytes, above its {limit}-byte limit. Preserve the existing file and use a separate workspace or reduce its metadata.",path.display())]
    MetadataSize {
        path: PathBuf,
        bytes: u64,
        limit: u64,
        #[source]
        source: Option<serde_json::Error>,
    },
    #[error("Terminal workspace metadata at {} would contain {count} sessions, above its {limit}-session limit. Preserve the workspace and create the terminal in a separate private folder.",path.display())]
    MetadataSessions {
        path: PathBuf,
        count: usize,
        limit: usize,
    },
    #[error("Terminal workspace metadata at {} is inconsistent ({reason}). Preserve it and restore a compatible backup or use a separate private workspace; it was not rewritten.",path.display())]
    MetadataStructure { path: PathBuf, reason: &'static str },
    #[error("Terminal workspace {} exhausted its identifier range at {next}. Preserve it and create new terminals in a separate private workspace.",path.display())]
    IdentifierExhausted { path: PathBuf, next: u64 },
    #[error("Could not {operation} terminal state at {}. Check ownership and free disk space; keep the existing files for recovery.", path.display())]
    Disk {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Terminal state at {} is incompatible or damaged. Keep a backup and use a separate state folder; do not overwrite it.", path.display())]
    State {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("Terminal state at {} is already open in another app. Close that app before opening this workspace.", path.display())]
    Locked {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "Terminal {id} did not answer its attachment handshake. Check the holder and SSH connection, then reconnect; no replacement process was started."
    )]
    Timeout {
        id: u64,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error(
        "Terminal {id} did not confirm shutdown before the timeout. Reconnect and inspect it before closing the SSH transport; the process may still be running."
    )]
    StopTimeout {
        id: u64,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("The output journal at {} is already attached. Detach the existing view before resuming it again.",path.display())]
    Attached {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Terminal {id} is not in this workspace. Select an existing terminal or create one.")]
    Missing { id: u64 },
    #[error("Invalid terminal configuration: {reason}. Correct it before starting the terminal.")]
    Invalid { reason: String },
    #[error(
        "Could not {operation} terminal {id}. Reconnect to the existing holder; a new shell cannot recover the old process. {source}"
    )]
    Holder {
        id: u64,
        operation: &'static str,
        #[source]
        source: humata2_holder::HolderError,
    },
    #[error(
        "Could not run SSH for {destination}. Check the SSH config, host key and key authentication, then reconnect."
    )]
    Ssh {
        destination: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "SSH to {destination} failed ({detail}). Check key authentication, host-key trust, remote holder installation and Unix-socket forwarding, then reconnect."
    )]
    Remote { destination: String, detail: String },
    #[error("Terminal journal {} is damaged at byte {offset}. Preserve it and restore from a backup; only an incomplete final record can be repaired automatically.", path.display())]
    Journal { path: PathBuf, offset: u64 },
}

pub(crate) fn disk(
    operation: &'static str,
    path: &std::path::Path,
    source: std::io::Error,
) -> Error {
    Error::Disk {
        operation,
        path: path.to_owned(),
        source,
    }
}

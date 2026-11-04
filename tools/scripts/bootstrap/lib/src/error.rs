use std::io;
use std::path::PathBuf;

use crate::download::Wait;

const RELOCK: &str = "`bazel run //tools/scripts/bootstrap/lock`";

/// A failure in the shared bootstrap code. The fields record what went wrong;
/// the message tells the user what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this tool needs the checkout's location from Bazel; run it with `bazel run {target}`")]
    NotUnderBazel { target: &'static str },

    #[error(
        "this machine is {os} {arch}; the bootstrap tools support only macOS and Linux on arm64 or x86_64"
    )]
    UnsupportedPlatform {
        os: &'static str,
        arch: &'static str,
    },

    #[error("could not read {}; check that it exists and is readable", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not write {}; check its permissions and free disk space", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "{} is invalid; it needs exactly bazelisk, python, git, gh, revup and uv, each a quoted string",
        path.display()
    )]
    InvalidVersions {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("line {line} of {} is malformed; regenerate the file with {RELOCK}", path.display())]
    MalformedLock { path: PathBuf, line: usize },

    #[error(
        "the server is rate-limiting requests from this machine ({url}); wait {wait} before retrying, since retrying sooner can extend the limit"
    )]
    RateLimited { url: String, wait: Wait },

    #[error("{url} does not exist; check that the version it was built from is published")]
    NotFound { url: String },

    #[error("{url} answered HTTP {status}; retry later, and check the URL if it keeps failing")]
    Status { url: String, status: u16 },

    #[error("could not download {url}; check your network connection and retry")]
    Download {
        url: String,
        #[source]
        source: ureq::Error,
    },

    #[error("the download of {url} was cut off; check your network connection and retry")]
    Interrupted {
        url: String,
        #[source]
        source: io::Error,
    },

    #[error(
        "{url} answered with data this tool does not understand; the service may have changed its API, so the tool needs updating"
    )]
    UnexpectedResponse {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "SHA-256 mismatch for {url}\n  pinned:     {expected}\n  downloaded: {actual}\n\
         The file is not the one versions.lock pinned. The download may be corrupted, upstream may have \
         replaced the file, or someone may have tampered with it. Nothing was installed from it.\n\
         Retry once to rule out a bad download. If it still mismatches, find out why before re-pinning: \
         check the project's release page or announcements for a re-upload. Only re-pin with {RELOCK} \
         once you trust the new file, since that accepts whatever is served now."
    )]
    HashMismatch {
        url: String,
        expected: String,
        actual: String,
    },
}

/// Prints `error` and its causes to stderr.
pub fn print_error(error: &dyn std::error::Error) {
    eprintln!("error: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}

/// Prints `error` and its causes, then exits with status 1.
pub fn exit_with(error: &dyn std::error::Error) -> ! {
    print_error(error);
    std::process::exit(1);
}

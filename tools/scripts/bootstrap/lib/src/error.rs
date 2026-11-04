use std::io;
use std::path::PathBuf;

use crate::download::Wait;

/// A failure in the shared bootstrap code. The fields record what went wrong;
/// the message tells the user what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this tool needs the checkout's location from Bazel; run it with `bazel run {target}`")]
    NotUnderBazel { target: &'static str },

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
        "{} is invalid; it needs exactly the key bazelisk, a quoted version string",
        path.display()
    )]
    InvalidVersions {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error(
        "the server is rate-limiting requests from this machine ({url}); wait {wait} before retrying, since retrying sooner can extend the limit"
    )]
    RateLimited { url: String, wait: Wait },

    #[error("{url} does not exist; check the versions in versions.toml")]
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
}

/// Prints `error` and its causes, then exits with status 1.
pub fn exit_with(error: &dyn std::error::Error) -> ! {
    eprintln!("error: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    std::process::exit(1);
}

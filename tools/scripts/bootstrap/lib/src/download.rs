//! Responses turned into data, and statuses into errors that say what to do.

use std::fmt;
use std::io::{self, Read, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::{Error, Http};

/// The body of `url` parsed as JSON, requested with `headers`.
pub fn json<T: DeserializeOwned>(
    http: &Http,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<T, Error> {
    serde_json::from_slice(&body(http, url, headers)?).map_err(|source| Error::UnexpectedResponse {
        url: url.to_owned(),
        source,
    })
}

/// SHA-256 of the body, streamed; nothing is written to disk.
pub fn sha256(http: &Http, url: &str) -> Result<String, Error> {
    copy_hashed(&mut reader(http, url, &[])?, &mut io::sink()).map_err(|error| match error {
        Copy::Read(source) | Copy::Write(source) => Error::Interrupted {
            url: url.to_owned(),
            source,
        },
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// How long a server asked us to wait before retrying, when it said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wait(Option<Duration>);

impl Wait {
    /// `retry-after` is seconds to wait; `x-ratelimit-reset` is when the
    /// limit resets, in Unix seconds.
    fn from_headers(retry_after: Option<&str>, reset: Option<&str>) -> Self {
        if let Some(seconds) = retry_after.and_then(|s| s.trim().parse().ok()) {
            return Wait(Some(Duration::from_secs(seconds)));
        }
        let until_reset = || {
            let reset: u64 = reset?.trim().parse().ok()?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some(Duration::from_secs(reset.saturating_sub(now)))
        };
        Wait(until_reset())
    }
}

impl fmt::Display for Wait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(wait) if wait.as_secs() >= 90 => {
                write!(f, "about {} minutes", wait.as_secs().div_ceil(60))
            }
            Some(wait) => write!(f, "about {} seconds", wait.as_secs().max(1)),
            None => f.write_str("a few minutes"),
        }
    }
}

fn body(http: &Http, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reader(http, url, headers)?
        .read_to_end(&mut bytes)
        .map_err(|source| Error::Interrupted {
            url: url.to_owned(),
            source,
        })?;
    Ok(bytes)
}

/// The body of a successful response.
fn reader(http: &Http, url: &str, headers: &[(&str, &str)]) -> Result<impl Read + use<>, Error> {
    let response = http.get(url, headers)?;
    check(url, response.status(), |name| response.header(name))?;
    Ok(response.into_reader())
}

/// Whether a response with `status` and these headers succeeded, and if not,
/// what went wrong.
fn check<'a>(
    url: &str,
    status: u16,
    header: impl Fn(&str) -> Option<&'a str>,
) -> Result<(), Error> {
    let url = url.to_owned();
    match status {
        200..=299 => Ok(()),
        // GitHub also signals rate limits with 403, plus these headers. A 403
        // without them is a real permission error.
        429 => Err(Error::RateLimited {
            url,
            wait: Wait::from_headers(header("retry-after"), header("x-ratelimit-reset")),
        }),
        403 if header("retry-after").is_some() || header("x-ratelimit-remaining") == Some("0") => {
            Err(Error::RateLimited {
                url,
                wait: Wait::from_headers(header("retry-after"), header("x-ratelimit-reset")),
            })
        }
        404 => Err(Error::NotFound { url }),
        status => Err(Error::Status { url, status }),
    }
}

/// Which side of a copy failed.
enum Copy {
    Read(io::Error),
    Write(io::Error),
}

/// Copies `reader` to `writer` and returns the SHA-256 of what was copied.
fn copy_hashed(reader: &mut impl Read, writer: &mut impl Write) -> Result<String, Copy> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = reader.read(&mut buffer).map_err(Copy::Read)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer.write_all(&buffer[..read]).map_err(Copy::Write)?;
    }
    writer.flush().map_err(Copy::Write)?;
    Ok(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers<'a>(pairs: &'a [(&str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> {
        move |name| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    #[test]
    fn retry_after_wins_over_reset() {
        assert_eq!(
            Wait::from_headers(Some("30"), Some("0")),
            Wait(Some(Duration::from_secs(30)))
        );
        assert_eq!(
            Wait::from_headers(None, Some("0")),
            Wait(Some(Duration::ZERO))
        );
        assert_eq!(Wait::from_headers(None, None), Wait(None));
    }

    #[test]
    fn forbidden_is_a_rate_limit_only_with_rate_limit_headers() {
        let url = "https://example.com/";
        assert!(check(url, 200, headers(&[])).is_ok());
        assert!(matches!(
            check(url, 403, headers(&[])),
            Err(Error::Status { status: 403, .. })
        ));
        assert!(matches!(
            check(url, 403, headers(&[("x-ratelimit-remaining", "0")])),
            Err(Error::RateLimited { .. })
        ));
        assert!(matches!(
            check(url, 403, headers(&[("retry-after", "30")])),
            Err(Error::RateLimited { .. })
        ));
        assert!(matches!(
            check(url, 429, headers(&[])),
            Err(Error::RateLimited {
                wait: Wait(None),
                ..
            })
        ));
        assert!(matches!(
            check(url, 404, headers(&[])),
            Err(Error::NotFound { .. })
        ));
    }
}

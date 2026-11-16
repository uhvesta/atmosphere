//! The `opencode` binary as the wrapper runs it: the URL `serve` prints, the
//! version it must be, the session it names, and where replies go.

/// The start of the line `opencode serve` prints with its URL.
const LISTENING: &str = "server listening on ";
/// The only host Humata talks to: the server is loopback-only.
const LOOPBACK: &str = "http://127.0.0.1:";
/// The oldest OpenCode with the server API Humata uses.
const MINIMUM: (u32, u32, u32) = (2, 3, 0);

/// Why `opencode --version` is not usable.
#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    /// The installed OpenCode predates the server API.
    #[error(
        "OpenCode {found} is too old. Humata needs OpenCode 2.3.0 or later. Upgrade with \
         OpenCode's own installer or package manager; after that, Humata runs its server with it."
    )]
    TooOld { found: String },
    /// The output had no version number in it.
    #[error(
        "`opencode --version` printed no version number, so the `opencode` on PATH may not be \
         OpenCode. Run `opencode --version` yourself; if it doesn't print an OpenCode version, \
         fix PATH or reinstall OpenCode with its own installer."
    )]
    Unreadable { output: String },
}

/// The URL in a `server listening on <url>` line, if it is a loopback URL with a port.
pub fn listening_url(line: &str) -> Option<String> {
    let url = line.trim().strip_prefix(LISTENING)?;
    let port = url.strip_prefix(LOOPBACK)?;
    let digits = port.chars().take_while(char::is_ascii_digit).count();
    (digits > 0).then(|| url.to_owned())
}

/// The `(major, minor, patch)` of OpenCode's version output, if it is at least 2.3.0.
///
/// The first word that starts with a version, such as `v2.3.0`, is the one read.
pub fn supported(version_output: &str) -> Result<(u32, u32, u32), VersionError> {
    let Some(found) = version_output.split_whitespace().find_map(parse_version) else {
        return Err(VersionError::Unreadable {
            output: version_output.trim().to_owned(),
        });
    };
    if found < MINIMUM {
        let (major, minor, patch) = found;
        return Err(VersionError::TooOld {
            found: format!("{major}.{minor}.{patch}"),
        });
    }
    Ok(found)
}

/// The version a word starts with, after an optional `v`.
fn parse_version(word: &str) -> Option<(u32, u32, u32)> {
    let word = word.strip_prefix('v').unwrap_or(word);
    let core = word
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .next()?;
    let mut parts = core.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// The path that answers one ask in one session.
pub fn reply_path(session: &str, request: &str) -> String {
    format!("/api/session/{session}/permission/{request}/reply")
}

/// The OpenCode session id a conversation gets: `ses_` and the first 26
/// letters and digits of the conversation id.
pub fn session_id(conversation: &str) -> String {
    let body: String = conversation
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(26)
        .collect();
    format!("ses_{body}")
}

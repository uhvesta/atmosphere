//! HistoryService.ReadMirror on a machine (D189, LD-8, LD-21): the next part
//! of a session's conversation for the host's mirror, from the offset the
//! host gives, scrubbed.
//!
//! Only the host asks, over the relay, on a connection it proved as its own
//! (D144). The only file opened is the `conversation_file` of the session's
//! agent in this machine's own store; the request names a session, an offset
//! and the file's last identity, never a path. Values are replaced here, on
//! the machine that holds them, before the bytes reach the link (SEC-1).

use humata2_chunk::{Encoder, Link};
use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{Lines, Scrubber, read_lines};
use tonic::{Code, Request, Status};

use super::Shared;
use crate::channel_service::blocking;
use crate::host_vars::{HostVars, HostVarsError};
use crate::identity::{self, Caller, UserVia};
use crate::status;

/// The link ReadMirror's codec is chosen for: the host's is always remote.
pub(super) const REMOTE: Link = Link::Remote {
    bytes_per_second: None,
};

/// Why this machine's host variable values couldn't be loaded, so a read
/// would have nothing to scrub with.
#[derive(Debug, thiserror::Error)]
pub(super) enum ScrubberError {
    #[error(
        "couldn't read this machine's host variable values, so no conversation was kept or sent: using it unscrubbed could leak a value. Fix the file named below (readable by this user, not damaged), then try again"
    )]
    Unreadable {
        #[source]
        source: HostVarsError,
    },
}

/// The scrubber for this machine's host variables. It fails when the values
/// can't be read and never falls back to an empty one (D189.3).
pub(super) fn scrubber() -> Result<Scrubber, ScrubberError> {
    scrubber_of(HostVars::installed())
}

fn scrubber_of(variables: Result<HostVars, HostVarsError>) -> Result<Scrubber, ScrubberError> {
    let values = variables
        .and_then(|variables| variables.values())
        .map_err(|source| ScrubberError::Unreadable { source })?;
    Ok(Scrubber::new(values))
}

/// Only the host reads conversations for its mirror (D189.1): the app, a CLI
/// login, an agent, a script and the peer link are all refused.
pub(super) fn require_host(caller: &Caller) -> Result<(), Status> {
    if matches!(caller, Caller::User(UserVia::Host)) {
        return Ok(());
    }
    Err(status::failure(
        Code::PermissionDenied,
        status::error(
            "only the host reads a machine's conversations for its mirror",
            "search and read sessions on the host with `humata sessions search`, the palette or `sessions_search`; the host keeps the copy",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    ))
}

pub(super) async fn read_mirror(
    shared: &std::sync::Arc<Shared>,
    request: Request<pb::ReadMirrorRequest>,
) -> Result<pb::ReadMirrorResponse, Status> {
    let caller = identity::caller(&request)?;
    require_host(&caller)?;
    let request = request.into_inner();
    let session = request
        .session
        .map(|session| session.value)
        .filter(|session| !session.is_empty())
        .ok_or_else(|| status::missing("session"))?;
    let (offset, max_bytes, source) = (request.offset, request.max_bytes, request.source);
    let reading = shared.clone();
    let lines = blocking(move || read(&reading, &session, offset, max_bytes, &source)).await??;

    let codec = shared.compression.pick(REMOTE, &request.accept_codecs);
    let names = lines.scrubbed_names;
    let chunk = Encoder::new(codec)
        .encode(lines.data)
        .map_err(|error| status::internal(status::chain(&error), ""))?;
    Ok(pb::ReadMirrorResponse {
        data: chunk.data,
        next_offset: lines.next_offset,
        more: lines.more,
        rewritten: lines.rewritten,
        source: lines.source,
        scrubbed_names: names,
        codec: chunk.header.codec.wire(),
        raw_size: chunk.header.raw_len,
        crc32: chunk.header.crc32,
    })
}

/// The scrubbed lines of `session`'s conversation file from `offset`; nothing
/// (at `offset`) when the session has no agent, no file yet or none on disk.
fn read(
    shared: &Shared,
    session: &str,
    offset: u64,
    max_bytes: u32,
    source: &str,
) -> Result<Lines, Status> {
    shared
        .learn(session)
        .map_err(|error| status::internal(status::chain(&error), ""))?;
    let agent = shared
        .store
        .agent(session)
        .map_err(|error| status::internal(status::chain(&error), ""))?;
    let nothing = || Lines {
        next_offset: offset,
        source: source.to_owned(),
        ..Lines::default()
    };
    let Some(file) = agent
        .map(|agent| agent.launch.conversation_file)
        .filter(|file| !file.as_os_str().is_empty() && file.exists())
    else {
        return Ok(nothing());
    };
    let scrubber = scrubber().map_err(|error| status::internal(status::chain(&error), ""))?;
    let lines = read_lines(&file, &shared.here(), offset, max_bytes, source, &scrubber)
        .map_err(|error| status::internal(status::chain(&error), &file.display().to_string()))?;
    for size in &lines.left_out {
        crate::log::say!(
            "humatad2: session {session}'s conversation has a line of {size} bytes that stays over 1 MiB after its long texts are cut; it is left out of the host's mirror"
        );
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use humata2_store::SessionRecord;

    use super::*;
    use std::path::PathBuf;
    use crate::identity::AgentCaller;
    use crate::script_tokens::ScriptCaller;

    fn agent() -> Caller {
        let record = SessionRecord {
            id: "se_1".to_owned(),
            channel: "ch_1".to_owned(),
            machine: String::new(),
            title: "Fix login".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::new(),
            created_unix_ms: 0,
        };
        Caller::Agent(AgentCaller::of(&record, "ma_here"))
    }

    fn script() -> Caller {
        Caller::Script(ScriptCaller {
            id: "st_1".to_owned(),
            name: "nightly".to_owned(),
            token: humata2_access::ScriptToken::default(),
        })
    }

    #[test]
    fn only_the_host_may_read_a_conversation_for_its_mirror() {
        let refused = [
            ("the app", Caller::User(UserVia::App)),
            (
                "a cli login",
                Caller::User(UserVia::CliLogin {
                    login: "login_1".to_owned(),
                    method: humata2_access::PresenceMethod::CliLogin,
                    on_host: true,
                }),
            ),
            ("an agent", agent()),
            ("a script", script()),
            ("the peer link", Caller::Peer),
        ];
        for (who, caller) in refused {
            let denied = require_host(&caller).expect_err(who);
            assert_eq!(denied.code(), Code::PermissionDenied, "{who}");
        }
        assert!(require_host(&Caller::User(UserVia::Host)).is_ok());
    }

    #[test]
    fn the_scrubber_has_this_machines_values_and_is_refused_when_they_cant_be_read() {
        let vars = crate::host_vars::MemoryStore::vars();
        vars.set("API_TOKEN", "s3cr3t-value").unwrap();

        let kept = scrubber_of(Ok(vars)).unwrap().scrub("curl -H s3cr3t-value");
        assert!(!kept.contains("s3cr3t-value"));
        assert!(kept.contains("API_TOKEN"));

        // Nothing readable: no scrubber, never an empty one.
        assert!(scrubber_of(Err(HostVarsError::NotInstalled)).is_err());
    }

    #[test]
    fn a_machine_with_no_host_variables_scrubs_nothing() {
        let kept = scrubber_of(Ok(crate::host_vars::MemoryStore::vars())).unwrap().scrub("plain text");
        assert_eq!(kept, "plain text");
    }
}

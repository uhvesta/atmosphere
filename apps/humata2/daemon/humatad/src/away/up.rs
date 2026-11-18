//! An agent's call sent up the Uplink to the host and answered with the
//! host's response (TP-8, D768). The machine and repo lookups, and the file
//! reads and transfers that reach another machine, go up this way: only the
//! host keeps machines and holds the links to them.

use humata2_grpc::humata::v1 as pb;
use prost::Message;
use tonic::{Code, Request, Response, Status};

use super::{Forwarder, Method};
use crate::status;

/// Why an agent's call couldn't be answered by the host.
#[derive(Debug, thiserror::Error)]
enum UpError {
    #[error("{method} needs the host Mac, and the host isn't linked right now, so nothing was asked or changed")]
    HostAway { method: &'static str },

    #[error("the host's answer to {method} couldn't be read")]
    Reply {
        method: &'static str,
        #[source]
        source: prost::DecodeError,
    },
}

impl From<UpError> for Status {
    fn from(error: UpError) -> Self {
        let message = status::chain(&error);
        match error {
            UpError::HostAway { .. } => status::failure(
                Code::Unavailable,
                status::error(
                    message,
                    "wait until the host Mac is awake and linked to this machine, then try again",
                    pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
                ),
            ),
            UpError::Reply { .. } => status::internal(
                format!(
                    "{message}; this machine and the host run different releases of Humata: migrate this machine from the host's Inbox"
                ),
                "",
            ),
        }
    }
}

/// Sends `session`'s `request` for `method` to the host and answers with
/// the host's response. The host runs it as that session's agent, under
/// its own rules.
pub(crate) async fn ask_host<Req: Message, Res: Message + Default>(
    forwarder: &Forwarder,
    session: &str,
    method: Method,
    request: Req,
    key: Option<pb::IdempotencyKey>,
) -> Result<Response<Res>, Status> {
    let row = method.row();
    if !forwarder.forwards() {
        return Err(UpError::HostAway { method: row.name }.into());
    }
    let bytes = forwarder.call(session, row, request.encode_to_vec(), key).await?;
    Res::decode(bytes.as_slice())
        .map(Response::new)
        .map_err(|source| UpError::Reply { method: row.name, source }.into())
}

/// The session of an agent `request` came from, when it did.
pub(crate) fn agent_session<T>(request: &Request<T>) -> Result<Option<String>, Status> {
    Ok(match crate::identity::caller(request)? {
        crate::identity::Caller::Agent(agent) => Some(agent.session.id.to_string()),
        _ => None,
    })
}

/// Whether `location` names a machine other than `here`.
pub(crate) fn names_another(here: &str, location: &pb::FileLocation) -> bool {
    location
        .machine
        .as_ref()
        .is_some_and(|machine| !machine.value.is_empty() && machine.value != here)
}

/// `location` as sent up from the machine `here`: its own machine left
/// unnamed, since the host knows it by the link's id and not by the id
/// this daemon gives itself (D768).
pub(crate) fn unnamed_here(here: &str, mut location: pb::FileLocation) -> pb::FileLocation {
    if location.machine.as_ref().is_some_and(|machine| machine.value == here) {
        location.machine = None;
    }
    location
}

/// `location` from a call `link` forwarded, on the host: one that names no
/// machine is on the machine that sent it.
pub(crate) fn on_link(link: &str, mut location: pb::FileLocation) -> pb::FileLocation {
    if location.machine.as_ref().is_none_or(|machine| machine.value.is_empty()) {
        location.machine = Some(pb::MachineId { value: link.to_owned() });
    }
    location
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(machine: Option<&str>) -> pb::FileLocation {
        pb::FileLocation {
            machine: machine.map(|value| pb::MachineId { value: value.to_owned() }),
            path: "note.txt".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_machines_own_location_goes_up_unnamed_and_the_host_reads_it_as_the_links() {
        let sent = unnamed_here("ma_self", at(Some("ma_self")));
        assert_eq!(on_link("vm-a", sent).machine.map(|machine| machine.value), Some("vm-a".to_owned()));
    }

    #[test]
    fn a_location_on_another_machine_keeps_its_machine_both_ways() {
        let sent = unnamed_here("ma_self", at(Some("ma_host")));
        assert_eq!(on_link("vm-a", sent).machine.map(|machine| machine.value), Some("ma_host".to_owned()));
    }
}

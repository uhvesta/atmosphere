//! FileService and TrashService calls for a machine other than this one,
//! relayed over that machine's link (FM-9, FM-12; D139, D144, D156).
//!
//! The host judges first: on the machine a relayed call arrives as the
//! user's, so the machine would let anything through that the host let past
//! (D144). Each call here places its locations (no disk is looked at),
//! makes the same capability check the local handler makes, and only then
//! sends a fresh request with each location's machine cleared (it is local
//! there). Nothing of the caller's metadata is copied. What comes back has
//! each location's machine set again, so the caller sees the names it asked
//! with. A status from the machine is passed on unchanged; a link that is
//! down, or went down in the call, is the machine-offline status.
//!
//! A read is the machine's stream passed through chunk by chunk, and a
//! write is the caller's stream passed on the same way, so neither is held
//! whole.
//!
//! Trash items listed from another machine come back with ids prefixed
//! `<machine>:` (and the ids MoveToTrash returns), because a restore names
//! only an item. The prefix is split off again on the way out.

use std::sync::{Arc, OnceLock};

use humata2_access::{Call, Capability, Entry, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use super::{Placed, ReadFileStream, Roots};
use crate::guard::{self, Guard};
use crate::host_relay::{Links, Relayed};
use crate::identity::{self, Caller};
use crate::status;

/// Messages of an upload held between the caller and the machine.
const WRITE_AHEAD: usize = 2;

/// The machines' links, given late (the host only); without them every
/// other machine is offline.
#[derive(Default)]
struct Reach {
    links: OnceLock<Links>,
}

impl Reach {
    fn set(&self, links: Links) {
        let _ = self.links.set(links);
    }

    /// Typed clients on `machine`'s live link, or the offline status.
    fn relayed(&self, machine: &str) -> Result<Relayed, Status> {
        self.links
            .get()
            .and_then(|links| links.relayed(machine).ok())
            .ok_or_else(|| offline(machine, false))
    }

    /// A failure of a call to `machine`: the offline status if its link is
    /// down now, else what the machine said.
    fn lost(&self, machine: &str, status: Status) -> Status {
        lost(self.links.get(), machine, status)
    }
}

fn lost(links: Option<&Links>, machine: &str, status: Status) -> Status {
    let down = links.is_none_or(|links| links.channel(machine).is_err());
    match status.code() {
        Code::Unavailable | Code::Unknown | Code::Cancelled | Code::Aborted
            if down || built_locally(&status) =>
        {
            offline(machine, true)
        }
        _ => status,
    }
}

/// Whether the client built `status` itself from a transport error. Tonic
/// gives every status made from an error a source; a status the machine
/// answered arrives over the wire with none. A broken connection reaches the
/// call at the same moment as the link supervisor, on another task, so the
/// links table may not say down yet: the status itself is the evidence.
fn built_locally(status: &Status) -> bool {
    std::error::Error::source(status).is_some()
}

/// `sent`: the call may have reached the machine before its link went down.
fn offline(machine: &str, sent: bool) -> Status {
    let (message, fix) = if sent {
        (
            format!("the link to machine {machine} went down during the call, so it may or may not have been made"),
            "wait until the machine is back (Reconnect now on its page hurries it), look at the folder, and repeat the call only if it didn't happen",
        )
    } else {
        (
            format!("machine {machine} can't be reached right now, so nothing was sent to it and nothing changed"),
            "the call works again when the machine is back: use Reconnect now on the machine's page to hurry it, then try again",
        )
    };
    status::failure(
        Code::Unavailable,
        status::error(
            message,
            fix,
            pb::error::Kind::MachineOffline(pb::MachineOffline {
                machine: Some(machine_id(machine)),
                machine_name: machine.to_owned(),
                since: None,
            }),
        ),
    )
}

fn machine_id(machine: &str) -> pb::MachineId {
    pb::MachineId {
        value: machine.to_owned(),
    }
}

/// The location as the machine knows it: its own, so no machine named.
fn there(placed: &Placed) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        ..placed.location.clone()
    }
}

/// An entry the machine returned, with its location on `machine` again.
fn back(machine: &str, entry: &mut pb::FileEntry) {
    if let Some(location) = entry.location.as_mut() {
        location.machine = Some(machine_id(machine));
    }
}

/// What the entry at `location` is judged as: the root of a folder when it
/// has no path, otherwise an entry in it.
fn entry_for(location: &pb::FileLocation) -> Entry {
    if location.path.is_empty() {
        Entry::Folder
    } else {
        Entry::File
    }
}

/// A trash item's id as the host gives it for an item on `machine`.
fn qualified(machine: &str, id: &str) -> String {
    format!("{machine}:{id}")
}

/// The machine and the id there of an id made by [`qualified`].
fn unqualified(id: &str) -> Option<(&str, &str)> {
    id.rsplit_once(':').filter(|(machine, id)| !machine.is_empty() && !id.is_empty())
}

/// File calls for other machines.
pub(crate) struct Forward {
    roots: Arc<Roots>,
    guard: Arc<Guard>,
    reach: Reach,
}

impl Forward {
    pub(crate) fn new(roots: Arc<Roots>, guard: Arc<Guard>) -> Self {
        Self {
            roots,
            guard,
            reach: Reach::default(),
        }
    }

    /// The machines' links, once they exist. The first call wins.
    pub(crate) fn use_links(&self, links: Links) {
        self.reach.set(links);
    }

    /// The machine `location` is on when that isn't this one. A location
    /// that can't be placed is left to the local handler to refuse.
    pub(crate) fn elsewhere(&self, location: Option<&pb::FileLocation>) -> Option<String> {
        let placed = self
            .roots
            .place(location.cloned(), "location", Entry::Folder)
            .ok()?;
        (placed.machine() != self.roots.machine()).then(|| placed.machine().to_owned())
    }

    /// The first machine other than this one that one of `locations` is on.
    pub(crate) fn elsewhere_any(&self, locations: &[pb::FileLocation]) -> Option<String> {
        locations
            .iter()
            .find_map(|location| self.elsewhere(Some(location)))
    }

    /// The machine a session is on, when that isn't this one.
    pub(crate) fn elsewhere_session(&self, session: Option<&pb::SessionId>) -> Option<String> {
        let id = session
            .map(|session| session.value.as_str())
            .filter(|id| !id.is_empty())?;
        let record = self.roots.store().session(id).ok()??;
        let machine = identity::session_ref(&record, self.roots.machine()).machine;
        (machine.as_str() != self.roots.machine()).then(|| machine.as_str().to_owned())
    }

    /// `location` placed and judged for reading, or for changing (which
    /// also refuses a main clone).
    fn judged(
        &self,
        caller: &Caller,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
        change: bool,
    ) -> Result<Placed, Status> {
        let placed = self.roots.place(location, field, entry)?;
        let call = if change {
            Call::WriteFiles(placed.place.clone())
        } else {
            Call::ReadFiles(placed.place.clone())
        };
        self.guard.require(caller, &call)?;
        if change {
            placed.require_writable(field)?;
        }
        Ok(placed)
    }

    pub(crate) async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let placed = self.judged(&caller, request.location, "location", Entry::Folder, false)?;
        let machine = placed.machine().to_owned();
        let mut client = self.reach.relayed(&machine)?.file();
        let sent = pb::ListDirectoryRequest {
            location: Some(there(&placed)),
            page: request.page,
        };
        let mut reply = client
            .list_directory(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        for entry in &mut reply.entries {
            back(&machine, entry);
        }
        Ok(Response::new(reply))
    }

    pub(crate) async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let placed = self.judged(&caller, request.location, "location", Entry::Folder, false)?;
        let machine = placed.machine().to_owned();
        let mut client = self.reach.relayed(&machine)?.file();
        let sent = pb::StatRequest {
            location: Some(there(&placed)),
        };
        let mut entry = client
            .stat(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        back(&machine, &mut entry);
        Ok(Response::new(entry))
    }

    pub(crate) async fn create_folder(
        &self,
        request: Request<pb::CreateFolderRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let placed = self.judged(&caller, request.location, "location", Entry::Folder, true)?;
        let machine = placed.machine().to_owned();
        let mut client = self.reach.relayed(&machine)?.file();
        let sent = pb::CreateFolderRequest {
            idempotency_key: request.idempotency_key,
            location: Some(there(&placed)),
        };
        let mut entry = client
            .create_folder(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        back(&machine, &mut entry);
        Ok(Response::new(entry))
    }

    pub(crate) async fn rename(
        &self,
        request: Request<pb::RenameRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let entry = request.location.as_ref().map_or(Entry::File, entry_for);
        let placed = self.judged(&caller, request.location, "location", entry, true)?;
        let machine = placed.machine().to_owned();
        let mut client = self.reach.relayed(&machine)?.file();
        let sent = pb::RenameRequest {
            idempotency_key: request.idempotency_key,
            location: Some(there(&placed)),
            new_name: request.new_name,
        };
        let mut entry = client
            .rename(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        back(&machine, &mut entry);
        Ok(Response::new(entry))
    }

    /// Every location must be on one machine: a request names one machine's
    /// trash. The ids it returns are qualified (see the module docs).
    pub(crate) async fn move_to_trash(
        &self,
        request: Request<pb::MoveToTrashRequest>,
    ) -> Result<Response<pb::MoveToTrashResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        if request.locations.is_empty() {
            return Err(status::missing("locations"));
        }
        let mut machine: Option<String> = None;
        let mut sent = Vec::with_capacity(request.locations.len());
        for (at, location) in request.locations.into_iter().enumerate() {
            let field = format!("locations[{at}]");
            let entry = entry_for(&location);
            let placed = self.judged(&caller, Some(location), &field, entry, true)?;
            if let Some(first) = &machine
                && first != placed.machine()
            {
                return Err(status::invalid_field(
                    &format!("{field}.machine"),
                    Problem::NotAllowedHere,
                    format!(
                        "{field} is on machine {}, but the first location is on machine {first}, and one request moves files to one machine's trash",
                        placed.machine()
                    ),
                    "send one request per machine",
                ));
            }
            if placed.is_root() {
                return Err(status::invalid_field(
                    &field,
                    Problem::NotAllowedHere,
                    "the root of a folder can't be moved to the trash",
                    "name something inside it",
                ));
            }
            machine.get_or_insert_with(|| placed.machine().to_owned());
            sent.push(there(&placed));
        }
        let machine = machine.ok_or_else(|| status::missing("locations"))?;
        let mut client = self.reach.relayed(&machine)?.file();
        let sent = pb::MoveToTrashRequest {
            idempotency_key: request.idempotency_key,
            locations: sent,
        };
        let mut reply = client
            .move_to_trash(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        for item in &mut reply.items {
            item.value = qualified(&machine, &item.value);
        }
        Ok(Response::new(reply))
    }

    /// The session's machine does the search; the host first checks that
    /// the caller may read each of the session's repos, as the local search
    /// does. A session the host knows no repos of has no matches.
    pub(crate) async fn find_files(
        &self,
        request: Request<pb::FindFilesRequest>,
    ) -> Result<Response<pb::FindFilesResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let machine = self
            .elsewhere_session(request.session.as_ref())
            .ok_or_else(|| status::missing("session"))?;
        let session = request
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .ok_or_else(|| status::missing("session"))?;
        let repos = self
            .roots
            .store()
            .session_detail(&session)
            .map_err(|error| {
                status::store(error, self.roots.machine(), "read the session's repositories")
            })?
            .map(|detail| detail.repos)
            .unwrap_or_default();
        if repos.is_empty() {
            return Ok(Response::new(pb::FindFilesResponse {
                entries: Vec::new(),
                page: Some(pb::PageResponse {
                    next_page_token: String::new(),
                }),
            }));
        }
        for repo in repos {
            let location = pb::FileLocation {
                machine: None,
                root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                    session: Some(pb::SessionId {
                        value: session.clone(),
                    }),
                    repo: Some(pb::RepoRef {
                        owner: repo.owner,
                        name: repo.name,
                    }),
                })),
                path: String::new(),
            };
            self.judged(&caller, Some(location), "session", Entry::Folder, false)?;
        }
        let mut client = self.reach.relayed(&machine)?.file();
        let mut reply = client
            .find_files(Request::new(request))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        for entry in &mut reply.entries {
            back(&machine, entry);
        }
        Ok(Response::new(reply))
    }

    /// The machine's stream, passed through chunk by chunk.
    pub(crate) async fn read_file(
        &self,
        request: Request<pb::ReadFileRequest>,
    ) -> Result<Response<ReadFileStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        // The reply is a stream that can't be held and replayed after an approval, so Ask stays refused.
        let placed = self.judged(&caller, request.location, "location", Entry::File, false)?;
        let machine = placed.machine().to_owned();
        let mut client = self.reach.relayed(&machine)?.file();
        let sent = pb::ReadFileRequest {
            location: Some(there(&placed)),
            offset: request.offset,
            max_bytes: request.max_bytes,
            reason: request.reason,
            accept_codecs: request.accept_codecs,
        };
        let chunks = client
            .read_file(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        let links = self.reach.links.get().cloned();
        let chunks = chunks.map(move |chunk| {
            chunk.map_err(|status| lost(links.as_ref(), &machine, status))
        });
        Ok(Response::new(Box::pin(chunks)))
    }

    /// The caller's stream after its header, passed on to the machine as
    /// it arrives; the header's location decides the machine.
    pub(crate) async fn write_file(
        &self,
        caller: &Caller,
        header: pb::WriteFileHeader,
        mut rest: Streaming<pb::WriteFileMessage>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        use pb::write_file_message::Message as Part;

        // The upload is a stream that can't be held and replayed after an approval, so Ask stays refused.
        let placed = self.judged(caller, header.location.clone(), "location", Entry::File, true)?;
        let machine = placed.machine().to_owned();
        let mut client = self.reach.relayed(&machine)?.file();
        let first = pb::WriteFileMessage {
            message: Some(Part::Header(pb::WriteFileHeader {
                location: Some(there(&placed)),
                ..header
            })),
        };
        let (send, outgoing) = mpsc::channel(WRITE_AHEAD);
        // Ends when the caller's stream does, or the machine stops reading.
        // A caller's stream that fails ends the machine's early, and the
        // machine writes nothing of a file that didn't arrive whole.
        tokio::spawn(async move {
            if send.send(first).await.is_err() {
                return;
            }
            while let Ok(Some(message)) = rest.message().await {
                if send.send(message).await.is_err() {
                    return;
                }
            }
        });
        let mut entry = client
            .write_file(Request::new(ReceiverStream::new(outgoing)))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        back(&machine, &mut entry);
        Ok(Response::new(entry))
    }
}

/// Trash calls for other machines' trash.
pub(crate) struct TrashForward {
    guard: Arc<Guard>,
    machine: String,
    reach: Reach,
}

impl TrashForward {
    pub(crate) fn new(guard: Arc<Guard>, machine: String) -> Self {
        Self {
            guard,
            machine,
            reach: Reach::default(),
        }
    }

    /// The machines' links, once they exist. The first call wins.
    pub(crate) fn use_links(&self, links: Links) {
        self.reach.set(links);
    }

    /// The machine named when it isn't this one.
    pub(crate) fn elsewhere(&self, machine: Option<&pb::MachineId>) -> Option<String> {
        machine
            .map(|machine| machine.value.as_str())
            .filter(|name| !name.is_empty() && *name != self.machine)
            .map(str::to_owned)
    }

    /// The machine an item id made by a forwarded listing belongs to.
    pub(crate) fn elsewhere_item(&self, id: Option<&pb::TrashItemId>) -> Option<String> {
        let (machine, _) = unqualified(&id?.value)?;
        (machine != self.machine).then(|| machine.to_owned())
    }

    /// The same check the local list, plan and empty make: the user's.
    fn require_user_only(&self, caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::TrashEmpty,
                target: Target::Global,
            },
        )
    }

    pub(crate) async fn list_trash(
        &self,
        request: Request<pb::ListTrashRequest>,
    ) -> Result<Response<pb::ListTrashResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let machine = self
            .elsewhere(request.machine.as_ref())
            .ok_or_else(|| status::missing("machine"))?;
        self.require_user_only(&caller)?;
        let mut client = self.reach.relayed(&machine)?.trash();
        let sent = pb::ListTrashRequest {
            machine: None,
            page: request.page,
        };
        let mut reply = client
            .list_trash(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        for item in &mut reply.items {
            item.machine = Some(machine_id(&machine));
            if let Some(id) = item.id.as_mut() {
                id.value = qualified(&machine, &id.value);
            }
        }
        Ok(Response::new(reply))
    }

    /// The item's id says the machine. The host can't read what the item
    /// was, so it judges the restore as the local one does an item that
    /// names no channel (an agent in its own, anyone else globally); the
    /// machine then judges it again with what it knows.
    pub(crate) async fn restore_from_trash(
        &self,
        request: Request<pb::RestoreFromTrashRequest>,
    ) -> Result<Response<pb::RestoreFromTrashResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let (machine, item) = unqualified(&id)
            .map(|(machine, item)| (machine.to_owned(), item.to_owned()))
            .ok_or_else(|| status::missing("id"))?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let target = match &caller {
            Caller::Agent(agent) => Target::Channel(agent.session.channel.clone()),
            _ => Target::Global,
        };
        self.guard.require(
            &caller,
            &Call::Use {
                capability: Capability::TrashRestore,
                target,
            },
        )?;
        let mut client = self.reach.relayed(&machine)?.trash();
        let sent = pb::RestoreFromTrashRequest {
            idempotency_key: request.idempotency_key,
            id: Some(pb::TrashItemId { value: item }),
        };
        let reply = client
            .restore_from_trash(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        Ok(Response::new(reply))
    }

    pub(crate) async fn plan_empty_trash(
        &self,
        request: Request<pb::PlanEmptyTrashRequest>,
    ) -> Result<Response<pb::EmptyTrashPlan>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let machine = self
            .elsewhere(request.machine.as_ref())
            .ok_or_else(|| status::missing("machine"))?;
        self.require_user_only(&caller)?;
        let mut client = self.reach.relayed(&machine)?.trash();
        let mut plan = client
            .plan_empty_trash(Request::new(pb::PlanEmptyTrashRequest { machine: None }))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        plan.machine = Some(machine_id(&machine));
        Ok(Response::new(plan))
    }

    /// The user only, present: judged here as the local call is. The plan
    /// token is the machine's own.
    pub(crate) async fn empty_trash(
        &self,
        request: Request<pb::EmptyTrashRequest>,
    ) -> Result<Response<pb::EmptyTrashResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let machine = self
            .elsewhere(request.machine.as_ref())
            .ok_or_else(|| status::missing("machine"))?;
        self.require_user_only(&caller)?;
        let principal = guard::principal(&caller);
        if self
            .guard
            .with_policy(|_, context| principal.present_user(context))
            .is_none()
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::TrashEmpty),
            }));
        }
        let mut client = self.reach.relayed(&machine)?.trash();
        let sent = pb::EmptyTrashRequest {
            machine: None,
            ..request
        };
        let reply = client
            .empty_trash(Request::new(sent))
            .await
            .map_err(|status| self.reach.lost(&machine, status))?
            .into_inner();
        Ok(Response::new(reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline_named(status: &Status) -> bool {
        status.code() == Code::Unavailable
            && status::error_of(status)
                .is_some_and(|error| matches!(error.kind, Some(pb::error::Kind::MachineOffline(_))))
    }

    #[tokio::test]
    async fn a_call_cut_by_a_broken_connection_is_offline_before_the_link_is_marked_down() {
        // The call hears the broken connection before the supervisor marks
        // the link down: the links still say Up.
        let links = Links::default();
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        links.up("box-1", channel);
        let broken = Status::from_error(Box::new(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "connection reset",
        )));
        assert!(offline_named(&lost(Some(&links), "box-1", broken)));
        // What the machine itself answered stays its answer.
        let said = Status::new(Code::Unknown, "the machine said so");
        let kept = lost(Some(&links), "box-1", said);
        assert_eq!(kept.code(), Code::Unknown);
        assert!(!offline_named(&kept));
    }
}

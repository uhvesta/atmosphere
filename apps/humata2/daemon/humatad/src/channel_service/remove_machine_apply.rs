//! RemoveMachine (CH-8, LD-10, MV-9): a machine leaves a channel only after
//! each of its live sessions was moved to another machine of the channel or
//! archived, as the user chose in the plan's dialog. The sessions are
//! handled one at a time, in the order given; the first one that can't be
//! handled ends the removal, the machine stays in the channel and what was
//! already done stays done. Then the machine is dropped from the channel.
//!
//! A removal is written down before its first session is touched, under
//! `<channels>/.removing-machine`, and the note goes after the last step.
//! A start that finds a note finishes the removal: the sessions still live
//! on the machine are handled as the note says, then the machine is
//! dropped. If one can't be handled, or a session started on the machine
//! since, the machine stays in the channel and the note goes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_access::{Capability, ChannelId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_store::ChannelRecord;
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::{ChannelSvc, Gated, blocking, error_of, failed, tell_channel};
use crate::host_relay::LinkState;
use crate::identity::{self, Caller, UserVia};
use crate::operations::{Progress, Started};
use crate::session_service::Removal;
use crate::restart::OperationState;
use crate::status;

/// Where removals not finished yet are written, in the channels folder.
const INTENTS: &str = ".removing-machine";

#[derive(Debug, thiserror::Error)]
enum ApplyError {
    #[error("the plan given is not one made for taking {machine} out of channel {channel}, or it was already used or has expired")]
    StalePlan { machine: String, channel: String },
    #[error("{session} is live on {machine} but the request doesn't say what to do with it")]
    Unhandled { session: String, machine: String },
    #[error("the request handles {session}, which is not a live session of {machine} in this channel")]
    NotListed { session: String, machine: String },
    #[error("the request handles {session} more than once")]
    Twice { session: String },
    #[error("{session} can't move to {target}: {why}")]
    BadTarget {
        session: String,
        target: String,
        why: &'static str,
    },
    #[error("this daemon has no way to move or archive sessions")]
    NoSessions,
    #[error("couldn't write down that {machine} is leaving channel {channel}, so no session was touched")]
    Note {
        machine: String,
        channel: String,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't take {machine} out of channel {channel} after its sessions were handled")]
    Drop {
        machine: String,
        channel: String,
        #[source]
        source: humata2_store::StoreError,
    },
}

/// One live session of the machine and what to do with it.
struct Item {
    session: String,
    title: String,
    how: Removal,
    /// "Moving X to Y" or "Archiving X", for the step.
    shown: String,
}

impl ChannelSvc {
    /// The live sessions of `machine` in channel `id`, as `(id, title)`.
    pub(super) async fn live_on(&self, id: &str, machine: &str) -> Result<Vec<(String, String)>, Status> {
        let (store, id, machine, mine) = (
            self.starter.store.clone(),
            id.to_owned(),
            machine.to_owned(),
            self.machine.clone(),
        );
        blocking(move || -> Result<Vec<(String, String)>, humata2_store::StoreError> {
            let mut live = Vec::new();
            for session in store.sessions()? {
                let on = if session.machine.is_empty() { &mine } else { &session.machine };
                if session.channel == id && *on == machine && store.archive(&session.id)?.is_none() {
                    live.push((session.id, session.title));
                }
            }
            Ok(live)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "list the machine's sessions"))
    }

    /// The live sessions of another machine, listed from what it last
    /// reported of them (SL-3): the work each has that isn't pushed.
    pub(super) async fn remote_sessions_of(
        &self,
        id: &str,
        machine: &str,
    ) -> Result<Vec<pb::SessionWork>, Status> {
        let mut listed = Vec::new();
        for (session, title) in self.live_on(id, machine).await? {
            let pointers = self
                .remote
                .as_ref()
                .and_then(|remote| remote.view(machine, &session))
                .map(|view| view.pointers)
                .unwrap_or_default();
            listed.push(pb::SessionWork {
                session: Some(pb::SessionId { value: session }),
                title,
                dirty_files: pointers.iter().map(|pointer| pointer.dirty_files).fold(0, u32::saturating_add),
                unpushed_commits: pointers
                    .iter()
                    .map(|pointer| pointer.unpushed_commits)
                    .fold(0, u32::saturating_add),
            });
        }
        Ok(listed)
    }

    /// The body of RemoveMachine; `approved` once the user said yes to it.
    pub(super) async fn remove_machine_of(
        &self,
        caller: &Caller,
        request: pb::RemoveChannelMachineRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = request
            .channel
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        let machine = request
            .machine
            .map(|machine| machine.value)
            .filter(|machine| !machine.is_empty())
            .ok_or_else(|| status::missing("machine"))?;
        let channel = self.find(&id).await?;
        if let Some(asked) = self.gate(
            caller,
            Capability::ChannelChange,
            Target::Channel(ChannelId::new(&id)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        if !channel.machines.contains(&machine) {
            return Err(failed(
                Code::FailedPrecondition,
                &NotInChannel {
                    machine: machine.clone(),
                    channel: id.clone(),
                },
                "that machine is not in the channel; list the channel's machines and pick one of them",
                None,
            ));
        }
        let sessions = self.sessions.get().cloned().ok_or_else(|| {
            failed(Code::FailedPrecondition, &ApplyError::NoSessions, "restart Humata and try again", None)
        })?;
        let live = self.live_on(&id, &machine).await?;
        let items = items_of(&channel, &machine, &live, request.dispositions)?;
        let token = request.plan.map(|plan| plan.value).unwrap_or_default();
        if !self.plans.take(&token, &id, &machine) {
            return Err(failed(
                Code::FailedPrecondition,
                &ApplyError::StalePlan {
                    machine: machine.clone(),
                    channel: id.clone(),
                },
                "plan the removal again, read what it lists, and send the new token",
                None,
            ));
        }

        let mut steps: Vec<(String, String)> = items
            .iter()
            .map(|item| (format!("session:{}", item.session), item.shown.clone()))
            .collect();
        steps.push(("remove_machine".to_owned(), format!("Taking {machine} out of {}", channel.name)));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId { value: id.clone() })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::RemoveMachine,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(self.clone().run_remove_machine(
            progress,
            sessions,
            caller.clone(),
            id,
            machine,
            items,
        ));
        Ok(Gated::Done(operation))
    }

    async fn run_remove_machine(
        self,
        progress: Progress,
        sessions: crate::session_service::SessionSvc,
        caller: Caller,
        id: String,
        machine: String,
        items: Vec<Item>,
    ) {
        let note = note_path(&self.starter.layout.root(), &id, &machine);
        let bytes = note_bytes(&id, &machine, &items);
        let written = blocking({
            let note = note.clone();
            move || write_note(&note, &bytes)
        })
        .await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(source)) => {
                for item in &items {
                    progress.set_step(&format!("session:{}", item.session), pb::StepState::Skipped, None);
                }
                progress.set_step("remove_machine", pb::StepState::Skipped, None);
                progress.fail(error_of(
                    &ApplyError::Note {
                        machine,
                        channel: id,
                        source,
                    },
                    "check that the channels folder's disk has room and can be written to, then send the removal again; nothing was changed",
                    None,
                ));
                return;
            }
            Err(refused) => {
                progress.fail(status::error_of(&refused).unwrap_or_default());
                return;
            }
        }
        // A kill here leaves only the note: the restart handles every session.
        crate::fault::point("remove_machine.intent");
        for (at, item) in items.iter().enumerate() {
            let key = format!("session:{}", item.session);
            progress.set_step(&key, pb::StepState::Running, None);
            match sessions.dispose_for_removal(&caller, &item.session, item.how.clone()).await {
                Ok(()) => {
                    progress.set_step(&key, pb::StepState::Done, None);
                    // A kill here leaves some sessions handled and the machine
                    // in the channel: the restart handles the rest.
                    crate::fault::point("remove_machine.session_handled");
                }
                Err(error) => {
                    self.end_note(note).await;
                    progress.set_step(&key, pb::StepState::Failed, Some(error.message.clone()));
                    for later in &items[at + 1..] {
                        progress.set_step(&format!("session:{}", later.session), pb::StepState::Skipped, None);
                    }
                    progress.set_step("remove_machine", pb::StepState::Skipped, None);
                    progress.fail(pb::Error {
                        message: format!("{}: {}", item.shown_failed(), error.message),
                        fix: format!(
                            "{machine} stays in the channel. {} {}",
                            error.fix,
                            if at == 0 {
                                "Nothing was changed.".to_owned()
                            } else {
                                "The sessions handled before this one stay as they are.".to_owned()
                            }
                        ),
                        retry: None,
                        kind: error.kind,
                    });
                    return;
                }
            }
        }
        crate::fault::point("remove_machine.sessions_done");
        progress.set_step("remove_machine", pb::StepState::Running, None);
        match self.drop_machine(&id, &machine).await {
            Ok(Ok(())) => {}
            Ok(Err(source)) => {
                progress.set_step("remove_machine", pb::StepState::Failed, Some(status::chain(&source)));
                progress.fail(error_of(
                    &ApplyError::Drop {
                        machine,
                        channel: id,
                        source,
                    },
                    "its sessions were handled; send the removal again to take it out of the channel",
                    None,
                ));
                return;
            }
            Err(refused) => {
                progress.fail(status::error_of(&refused).unwrap_or_default());
                return;
            }
        }
        // A kill here leaves the machine out of the channel and the note:
        // the restart finds nothing left to do and removes it.
        crate::fault::point("remove_machine.dropped");
        self.end_note(note).await;
        progress.set_step("remove_machine", pb::StepState::Done, None);
        tell_channel(
            &self.operations.feed(),
            &self.starter.store,
            &self.starter.agents,
            &id,
            pb::Change::Updated,
            Some(identity::actor(&caller)),
        );
        progress.succeed(Some(pb::operation::Result::Machine(pb::MachineId { value: machine })));
    }
}

impl ChannelSvc {
    /// Takes `machine` out of channel `id`. Safe to repeat.
    async fn drop_machine(&self, id: &str, machine: &str) -> Result<Result<(), humata2_store::StoreError>, Status> {
        let (store, key, gone) = (self.starter.store.clone(), id.to_owned(), machine.to_owned());
        blocking(move || -> Result<(), humata2_store::StoreError> {
            let Some(mut channel) = store.channel(&key)? else {
                return Ok(());
            };
            if !channel.machines.contains(&gone) {
                return Ok(());
            }
            channel.machines.retain(|named| *named != gone);
            channel.revision += 1;
            store.put_channel(&channel)
        })
        .await
    }

    /// The removal is over, finished or ended; its note goes, and the
    /// removal of the note is on disk before this returns.
    async fn end_note(&self, note: PathBuf) {
        let shown = note.clone();
        if let Ok(Err(error)) = blocking(move || remove_note(&note)).await {
            crate::log::say!("humatad2: couldn't remove the note {}: {error}", shown.display());
        }
    }

    /// Finishes the removals a run left cut short, once the start is done
    /// and the links of the machines they name are up. Never blocks the start.
    pub(crate) fn recover_removals_after(&self, started: Arc<crate::restart::Started>) {
        let me = self.clone();
        // D241: each owed removal is named pending now, before the report.
        let owed = owed_removals(&self.starter.layout.root(), started.operations());
        tokio::spawn(async move {
            started.wait().await;
            let Some(sessions) = me.sessions.get().cloned() else {
                for (_, note) in &owed {
                    started.operations().record(
                        &removal_operation(note),
                        "remove_machine",
                        "handling sessions",
                        OperationState::Stuck {
                            reason: "the session service wasn't set when the start finished; restart humatad2 to finish the removal".to_owned(),
                        },
                    );
                }
                return;
            };
            for (path, note) in owed {
                me.finish_removal(&sessions, &started, path, note).await;
            }
        });
    }

    /// Waits until the link of each machine in `named` is up; this machine's
    /// always is. Err names a machine that was forgotten meanwhile.
    async fn links_up(&self, named: &[&str]) -> Result<(), String> {
        let Some(known) = &self.known else {
            return Ok(());
        };
        let links = known.links();
        for machine in named.iter().filter(|machine| **machine != self.machine) {
            links
                .watch(machine)
                .wait_for(|now| matches!(now, humata2_svc_machines::ports::LinkState::Up { .. }))
                .await
                .map(drop)
                .map_err(|_| (*machine).to_owned())?;
        }
        Ok(())
    }

    async fn finish_removal(
        &self,
        sessions: &crate::session_service::SessionSvc,
        started: &crate::restart::Started,
        path: PathBuf,
        note: Note,
    ) {
        let operation = removal_operation(&note);
        let record = |step: &str, state: OperationState| {
            started.operations().record(&operation, "remove_machine", step, state);
        };
        let (id, machine) = (&note.channel, &note.machine);
        // Named pending already, by `owed_removals`.
        let named = machines_named(&note);
        if let Err(gone) = self.links_up(&named).await {
            record(
                "handling sessions",
                OperationState::Undone {
                    reason: format!("{gone} was forgotten before the removal could be finished"),
                },
            );
            self.end_note(path).await;
            return;
        }
        let (store, key) = (self.starter.store.clone(), id.clone());
        let channel = blocking(move || store.channel(&key)).await;
        let channel = match channel {
            Ok(Ok(Some(channel))) => channel,
            Ok(Ok(None)) => {
                record(
                    "handling sessions",
                    OperationState::Undone {
                        reason: format!("channel {id} is gone"),
                    },
                );
                self.end_note(path).await;
                return;
            }
            _ => {
                record(
                    "handling sessions",
                    OperationState::Stuck {
                        reason: format!("the channel {id} could not be read"),
                    },
                );
                return;
            }
        };
        let live = match self.live_on(id, machine).await {
            Ok(live) => live,
            Err(error) => {
                record(
                    "handling sessions",
                    OperationState::Stuck {
                        reason: error.message().to_owned(),
                    },
                );
                return;
            }
        };
        let user = Caller::User(UserVia::Host);
        for item in &note.items {
            if !live.iter().any(|(session, _)| *session == item.session) {
                continue;
            }
            if let Err(error) = sessions
                .dispose_for_removal(&user, &item.session, item.how.clone())
                .await
            {
                record(
                    "handling sessions",
                    OperationState::Undone {
                        reason: format!("{}: {}; {machine} stays in the channel", item.shown_failed(), error.message),
                    },
                );
                self.end_note(path).await;
                return;
            }
        }
        let left = self.live_on(id, machine).await.map(|left| left.len());
        if !matches!(left, Ok(0)) {
            record(
                "handling sessions",
                OperationState::Undone {
                    reason: format!("{machine} still has live sessions the removal didn't list; it stays in the channel"),
                },
            );
            self.end_note(path).await;
            return;
        }
        match self.drop_machine(id, machine).await {
            Ok(Ok(())) => {}
            other => {
                let reason = match other {
                    Ok(Err(error)) => status::chain(&error),
                    Err(status) => status.message().to_owned(),
                    Ok(Ok(())) => String::new(),
                };
                record("dropping the machine", OperationState::Stuck { reason });
                return;
            }
        }
        if channel.machines.contains(machine) {
            tell_channel(&self.operations.feed(), &self.starter.store, &self.starter.agents, id, pb::Change::Updated, None);
        }
        self.end_note(path).await;
        record("dropped the machine", OperationState::Finished);
    }
}

/// What a note of a removal in progress says.
struct Note {
    channel: String,
    machine: String,
    items: Vec<Item>,
}

/// `<channels>/.removing-machine/<channel>.<machine>.json`.
fn note_path(root: &Path, channel: &str, machine: &str) -> PathBuf {
    root.join(INTENTS).join(format!("{channel}.{machine}.json"))
}

fn note_bytes(channel: &str, machine: &str, items: &[Item]) -> Vec<u8> {
    let items: Vec<serde_json::Value> = items
        .iter()
        .map(|item| {
            let how = match &item.how {
                Removal::MoveTo(to) => serde_json::json!({ "move_to": to }),
                Removal::Archive(choice) => serde_json::json!({ "archive": *choice as i32 }),
            };
            serde_json::json!({ "session": item.session, "title": item.title, "how": how })
        })
        .collect();
    serde_json::to_vec_pretty(&serde_json::json!({ "channel": channel, "machine": machine, "items": items }))
        .unwrap_or_default()
}

fn note_of(bytes: &[u8]) -> Option<Note> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let text = |value: &serde_json::Value, key: &str| value[key].as_str().map(str::to_owned);
    let mut items = Vec::new();
    for item in value["items"].as_array()? {
        let how = if let Some(to) = item["how"]["move_to"].as_str() {
            Removal::MoveTo(to.to_owned())
        } else {
            let choice = i32::try_from(item["how"]["archive"].as_i64()?).ok()?;
            Removal::Archive(pb::ArchiveChoice::try_from(choice).ok()?)
        };
        let session = text(item, "session")?;
        items.push(Item {
            title: text(item, "title").unwrap_or_else(|| session.clone()),
            session,
            how,
            shown: String::new(),
        });
    }
    Some(Note {
        channel: text(&value, "channel")?,
        machine: text(&value, "machine")?,
        items,
    })
}

/// Writes the note, with the folder it is in, on disk.
fn write_note(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        let missing: Vec<&Path> = folder
            .ancestors()
            .take_while(|found| !found.as_os_str().is_empty() && std::fs::symlink_metadata(found).is_err())
            .collect();
        humata2_durable::create_dir_all(folder, 0o700)?;
        for found in missing {
            match found.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => humata2_durable::sync_dir(parent)?,
                _ => {}
            }
        }
    }
    humata2_store::write_atomic(path, bytes).map_err(std::io::Error::other)
}

/// Removes the note and puts the removal on disk, so a finished removal
/// can't have its note come back after a power cut and be run again.
fn remove_note(path: &Path) -> std::io::Result<()> {
    humata2_durable::remove_file(path)?;
    match path.parent() {
        Some(parent) => humata2_durable::sync_dir(parent),
        None => Ok(()),
    }
}

fn removal_operation(note: &Note) -> String {
    format!("remove_machine:{}:{}", note.channel, note.machine)
}

/// The machines a removal waits on: the one removed and each one it moves
/// sessions to.
fn machines_named(note: &Note) -> Vec<&str> {
    let mut named: Vec<&str> = vec![note.machine.as_str()];
    for item in &note.items {
        if let Removal::MoveTo(to) = &item.how {
            named.push(to.as_str());
        }
    }
    named
}

/// The removals a run left (`read_notes`), each named pending in the
/// restart report at once, which is written after this (D241).
fn owed_removals(root: &Path, operations: &crate::restart::Operations) -> Vec<(PathBuf, Note)> {
    let notes = read_notes(root);
    for (_, note) in &notes {
        operations.record(
            &removal_operation(note),
            "remove_machine",
            "handling sessions",
            OperationState::Pending {
                waits_for: format!("the start to finish, then the links of {}", machines_named(note).join(", ")),
            },
        );
    }
    notes
}

/// The notes of the removals a run left, each with its path. A note that
/// can't be read is logged and left in place.
fn read_notes(root: &Path) -> Vec<(PathBuf, Note)> {
    let Ok(entries) = std::fs::read_dir(root.join(INTENTS)) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read(&path).ok().and_then(|bytes| note_of(&bytes)) {
            Some(note) => found.push((path, note)),
            None => crate::log::say!(
                "humatad2: the note of a machine's removal at {} can't be read; the machine stays in its channel. Remove the machine again, then delete the note",
                path.display()
            ),
        }
    }
    found
}

impl Item {
    fn shown_failed(&self) -> String {
        match &self.how {
            Removal::MoveTo(to) => format!("couldn't move {} to {to}", self.title),
            Removal::Archive(_) => format!("couldn't archive {}", self.title),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("machine {machine} is not in channel {channel}")]
struct NotInChannel {
    machine: String,
    channel: String,
}

/// Checks the dispositions against the machine's live sessions: each one
/// has exactly one, and a move goes to another machine of the channel.
fn items_of(
    channel: &ChannelRecord,
    machine: &str,
    live: &[(String, String)],
    dispositions: Vec<pb::SessionDisposition>,
) -> Result<Vec<Item>, Status> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for disposition in dispositions {
        let session = disposition
            .session
            .map(|session| session.value)
            .filter(|session| !session.is_empty())
            .ok_or_else(|| status::missing("dispositions.session"))?;
        let refused = |error: ApplyError, fix: &str| {
            status::invalid_field("dispositions", Problem::UnknownValue, error.to_string(), fix)
        };
        let Some((_, title)) = live.iter().find(|(id, _)| *id == session) else {
            return Err(refused(
                ApplyError::NotListed {
                    session,
                    machine: machine.to_owned(),
                },
                "send one disposition for each session the plan listed, and no others",
            ));
        };
        if !seen.insert(session.clone()) {
            return Err(refused(ApplyError::Twice { session }, "send one disposition for each session"));
        }
        let (how, shown) = match disposition.disposition {
            Some(pb::session_disposition::Disposition::MoveTo(to)) => {
                let target = to.value;
                let why = if target == machine {
                    Some("that is the machine being removed")
                } else if !channel.machines.contains(&target) {
                    Some("it is not one of the channel's machines")
                } else {
                    None
                };
                if let Some(why) = why {
                    return Err(refused(
                        ApplyError::BadTarget {
                            session,
                            target,
                            why,
                        },
                        "move it to another machine of the channel, or archive it",
                    ));
                }
                let shown = format!("Moving {title} to {target}");
                (Removal::MoveTo(target), shown)
            }
            Some(pb::session_disposition::Disposition::Archive(choice)) => {
                match pb::ArchiveChoice::try_from(choice) {
                    Ok(
                        choice @ (pb::ArchiveChoice::Archive
                        | pb::ArchiveChoice::CommitPushPrThenArchive
                        | pb::ArchiveChoice::ArchiveAndDeleteWork),
                    ) => (Removal::Archive(choice), format!("Archiving {title}")),
                    _ => {
                        return Err(status::invalid_field(
                            "dispositions",
                            Problem::Missing,
                            format!("the request doesn't say how to archive {title}"),
                            "set the archive choice to ARCHIVE, ARCHIVE_AND_DELETE_WORK or COMMIT_PUSH_PR_THEN_ARCHIVE",
                        ));
                    }
                }
            }
            None => {
                return Err(status::invalid_field(
                    "dispositions",
                    Problem::Missing,
                    format!("the request doesn't say what to do with {title}"),
                    "set move_to or archive for it",
                ));
            }
        };
        items.push(Item {
            session,
            title: title.clone(),
            how,
            shown,
        });
    }
    for (session, _) in live {
        if !seen.contains(session) {
            return Err(status::invalid_field(
                "dispositions",
                Problem::Missing,
                ApplyError::Unhandled {
                    session: session.clone(),
                    machine: machine.to_owned(),
                }
                .to_string(),
                "send one disposition for each session the plan listed",
            ));
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GONE: &str = "ma_gone";
    const OTHER: &str = "ma_other";

    /// A removal the last run left is named pending when its note is read.
    #[test]
    fn a_removal_the_last_run_left_is_named_pending_when_read() {
        let (root, report) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir_all(root.path().join(INTENTS)).unwrap();
        std::fs::write(note_path(root.path(), "ch_1", GONE), note_bytes("ch_1", GONE, &[])).unwrap();
        let operations = crate::restart::Operations::default();
        operations.set_folder(report.path());

        let owed = owed_removals(root.path(), &operations);

        assert_eq!(owed.len(), 1);
        assert_eq!(crate::test_support::pending_in(report.path()), vec![format!("remove_machine:ch_1:{GONE}")]);
    }

    fn channel(machines: &[&str]) -> ChannelRecord {
        ChannelRecord {
            id: "ch_1".to_owned(),
            name: "Team".to_owned(),
            folder: "team".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: Vec::new(),
            machines: machines.iter().map(|machine| machine.to_string()).collect(),
            revision: 1,
            created_unix_ms: 0,
        }
    }

    /// The machine being removed and one other machine of the channel.
    fn channel_with_other() -> ChannelRecord {
        channel(&[GONE, OTHER])
    }

    fn live(sessions: &[(&str, &str)]) -> Vec<(String, String)> {
        sessions
            .iter()
            .map(|(id, title)| (id.to_string(), title.to_string()))
            .collect()
    }

    fn disposition(session: &str, what: pb::session_disposition::Disposition) -> pb::SessionDisposition {
        pb::SessionDisposition {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            disposition: Some(what),
        }
    }

    fn move_to(session: &str, machine: &str) -> pb::SessionDisposition {
        disposition(
            session,
            pb::session_disposition::Disposition::MoveTo(pb::MachineId {
                value: machine.to_owned(),
            }),
        )
    }

    fn archive(session: &str, choice: pb::ArchiveChoice) -> pb::SessionDisposition {
        disposition(
            session,
            pb::session_disposition::Disposition::Archive(choice as i32),
        )
    }

    fn accepted(result: Result<Vec<Item>, Status>) -> Vec<Item> {
        match result {
            Ok(items) => items,
            Err(status) => panic!("refused with code {:?}", status.code()),
        }
    }

    fn refused(result: Result<Vec<Item>, Status>) -> Status {
        match result {
            Ok(items) => panic!("accepted {} items", items.len()),
            Err(status) => status,
        }
    }

    /// The code, the invalid field's name and its problem of a refusal.
    fn refusal(status: &Status) -> (Code, String, i32) {
        let Some(error) = status::error_of(status) else {
            panic!("the refusal carries no humata.v1.Error");
        };
        let Some(pb::error::Kind::InvalidField(found)) = error.kind else {
            panic!("the refusal doesn't name an invalid field");
        };
        (status.code(), found.field, found.problem)
    }

    fn moved_to(how: &Removal) -> Option<&str> {
        match how {
            Removal::MoveTo(to) => Some(to.as_str()),
            Removal::Archive(_) => None,
        }
    }

    fn archived_as(how: &Removal) -> Option<pb::ArchiveChoice> {
        match how {
            Removal::Archive(choice) => Some(*choice),
            Removal::MoveTo(_) => None,
        }
    }

    #[test]
    fn a_move_to_another_machine_of_the_channel_is_accepted() {
        let items = accepted(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor")]),
            vec![move_to("se_1", OTHER)],
        ));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].session, "se_1");
        assert_eq!(items[0].title, "Refactor");
        assert_eq!(moved_to(&items[0].how), Some(OTHER));
    }

    #[test]
    fn each_archive_choice_is_accepted_as_an_archive() {
        for choice in [
            pb::ArchiveChoice::Archive,
            pb::ArchiveChoice::CommitPushPrThenArchive,
            pb::ArchiveChoice::ArchiveAndDeleteWork,
        ] {
            let items = accepted(items_of(
                &channel_with_other(),
                GONE,
                &live(&[("se_1", "Refactor")]),
                vec![archive("se_1", choice)],
            ));
            assert_eq!(items.len(), 1);
            assert_eq!(archived_as(&items[0].how), Some(choice));
        }
    }

    #[test]
    fn a_machine_with_no_live_sessions_needs_no_dispositions() {
        let items = accepted(items_of(&channel_with_other(), GONE, &[], Vec::new()));
        assert!(items.is_empty());
    }

    #[test]
    fn a_disposition_without_a_session_is_refused_as_missing() {
        for bad in [
            pb::SessionDisposition {
                session: None,
                ..move_to("se_1", OTHER)
            },
            move_to("", OTHER),
        ] {
            let status = refused(items_of(
                &channel_with_other(),
                GONE,
                &live(&[("se_1", "Refactor")]),
                vec![bad],
            ));
            assert_eq!(
                refusal(&status),
                (Code::InvalidArgument, "dispositions.session".to_owned(), Problem::Missing as i32)
            );
        }
    }

    #[test]
    fn a_session_not_live_on_the_machine_is_refused() {
        let status = refused(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor")]),
            vec![move_to("se_9", OTHER)],
        ));
        assert_eq!(
            refusal(&status),
            (Code::InvalidArgument, "dispositions".to_owned(), Problem::UnknownValue as i32)
        );
    }

    #[test]
    fn a_session_handled_twice_is_refused() {
        let status = refused(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor")]),
            vec![move_to("se_1", OTHER), archive("se_1", pb::ArchiveChoice::Archive)],
        ));
        assert_eq!(
            refusal(&status),
            (Code::InvalidArgument, "dispositions".to_owned(), Problem::UnknownValue as i32)
        );
    }

    #[test]
    fn a_move_to_the_machine_being_removed_is_refused() {
        let status = refused(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor")]),
            vec![move_to("se_1", GONE)],
        ));
        assert_eq!(
            refusal(&status),
            (Code::InvalidArgument, "dispositions".to_owned(), Problem::UnknownValue as i32)
        );
    }

    #[test]
    fn a_move_to_a_machine_outside_the_channel_is_refused() {
        let status = refused(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor")]),
            vec![move_to("se_1", "ma_stranger")],
        ));
        assert_eq!(
            refusal(&status),
            (Code::InvalidArgument, "dispositions".to_owned(), Problem::UnknownValue as i32)
        );
    }

    #[test]
    fn an_archive_without_a_usable_choice_is_refused_as_missing() {
        for choice in [0, 99] {
            let status = refused(items_of(
                &channel_with_other(),
                GONE,
                &live(&[("se_1", "Refactor")]),
                vec![disposition(
                    "se_1",
                    pb::session_disposition::Disposition::Archive(choice),
                )],
            ));
            assert_eq!(
                refusal(&status),
                (Code::InvalidArgument, "dispositions".to_owned(), Problem::Missing as i32)
            );
        }
    }

    #[test]
    fn a_disposition_with_no_action_is_refused_as_missing() {
        let status = refused(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor")]),
            vec![pb::SessionDisposition {
                disposition: None,
                ..move_to("se_1", OTHER)
            }],
        ));
        assert_eq!(
            refusal(&status),
            (Code::InvalidArgument, "dispositions".to_owned(), Problem::Missing as i32)
        );
    }

    #[test]
    fn a_live_session_left_without_a_disposition_is_refused_as_missing() {
        let status = refused(items_of(
            &channel_with_other(),
            GONE,
            &live(&[("se_1", "Refactor"), ("se_2", "Docs")]),
            vec![move_to("se_1", OTHER)],
        ));
        assert_eq!(
            refusal(&status),
            (Code::InvalidArgument, "dispositions".to_owned(), Problem::Missing as i32)
        );
    }
}

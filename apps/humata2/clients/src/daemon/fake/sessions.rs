use tokio::sync::mpsc;

use super::{FakeDaemon, Subscriber};
use crate::daemon::{
    ArchivePlan, DaemonError, EVENT_QUEUE, Event, Events, ForkOptions, Operation, PastTurn, Reply,
    UnpushedWork,
};
use crate::model::{
    Agent, Archive, ChannelId, ForkKind, ItemStatus, Line, MachineId, Scope, Session, SessionClone, SessionId,
    Speaker, Status, Turn,
};
use crate::workspace::Workspace;

pub(super) fn load(daemon: &FakeDaemon) -> Reply<Workspace> {
    daemon.reply(Some(Operation::Load), "load".to_owned(), |inner| {
        Ok(inner.workspace.clone())
    })
}

pub(super) fn events(daemon: &FakeDaemon) -> Reply<Events> {
    let gate = daemon.inner().subscribe_gate.take();
    let reply = daemon.reply(None, "events".to_owned(), |inner| {
        let (events, receiver) = mpsc::channel(EVENT_QUEUE);
        if inner.resync_on_subscribe {
            let _ = events.try_send(Ok(Event::Resync));
        }
        inner.subscribers.push(Subscriber {
            events,
            behind: false,
        });
        Ok(receiver)
    });
    Box::pin(async move {
        if let Some(gate) = gate {
            // A dropped sender is a release too.
            let _ = gate.await;
        }
        reply.await
    })
}

/// Types `text` into a shell or agent tab other than the session's own
/// agent: recorded as a call and shown in the tab's output, with no answer.
pub(super) fn send_to_tab(
    daemon: &FakeDaemon,
    session: &SessionId,
    tab: usize,
    text: &str,
    key: &str,
) -> Reply<()> {
    let id = session.clone();
    let text = text.to_owned();
    let key = key.to_owned();
    daemon.reply(
        Some(Operation::Send),
        format!("send_to_tab {session} {tab} {text:?}"),
        move |inner| {
            if inner.sent_keys.contains(&key) {
                return Ok(());
            }
            let found = inner.session_mut(&id)?;
            if tab > found.shells.len() {
                return Err(DaemonError::NotFound {
                    what: format!("Terminal {tab} of {id}"),
                });
            }
            inner.output(&id, tab, Speaker::You, &text);
            inner.sent_keys.insert(key);
            Ok(())
        },
    )
}

pub(super) fn send(daemon: &FakeDaemon, session: &SessionId, text: &str) -> Reply<()> {
    let id = session.clone();
    let text = text.to_owned();
    let shared = daemon.shared.clone();
    let step = daemon.step;
    let this = daemon.clone();
    let ack = daemon.reply(Some(Operation::Send), format!("send {session} {text:?}"), {
        let id = id.clone();
        let text = text.clone();
        move |inner| {
            let found = inner.session_mut(&id)?;
            if !found.status.accepts_input() {
                return Err(DaemonError::Refused {
                    what: format!("Typing into {id}"),
                    reason: "the session has exited. Resume it first".to_owned(),
                });
            }
            found.status = Status::Working;
            found.turn = Turn::Running;
            inner.changed(&id);
            inner.output(&id, 0, Speaker::You, &text);
            super::desk::used(inner, &id);
            Ok(())
        }
    });
    Box::pin(async move {
        ack.await?;
        for line in answer_lines(&text) {
            this.wait(step).await;
            let mut inner = match shared.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };
            let from = if line.starts_with("Read ") {
                Speaker::Tool
            } else {
                Speaker::Agent
            };
            inner.output(&id, 0, from, &line);
        }
        let mut inner = match shared.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(found) = inner
            .workspace
            .sessions
            .iter_mut()
            .find(|found| found.id == id)
        {
            found.status = Status::Idle;
            found.turn = Turn::Ended;
        }
        inner.changed(&id);
        Ok(())
    })
}

pub(super) fn resume(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let id = session.clone();
    daemon.reply(
        Some(Operation::Resume),
        format!("resume {session}"),
        move |inner| {
            let found = inner.session_mut(&id)?;
            if found.archived {
                return Err(DaemonError::Refused {
                    what: format!("Resuming session {id}"),
                    reason: format!(
                        "session {id} was ended for good, so it can't be resumed. Restore it first (Undo, or `humata session restore`); it comes back stopped, and then it can be resumed"
                    ),
                });
            }
            found.status = Status::Idle;
            found.turn = Turn::Ended;
            found.recovery = None;
            let agent = found.agent;
            inner.changed(&id);
            inner.output(
                &id,
                0,
                Speaker::System,
                &format!("{} resumed on its conversation", agent.name()),
            );
            Ok(())
        },
    )
}

pub(super) fn start_fresh(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let id = session.clone();
    daemon.reply(
        Some(Operation::StartFresh),
        format!("start fresh {session}"),
        move |inner| {
            let found = inner.session_mut(&id)?;
            found.status = Status::Idle;
            found.turn = Turn::Ended;
            found.recovery = None;
            found.resume_failed = None;
            found.summary_source = None;
            let agent = found.agent;
            inner.changed(&id);
            inner.output(
                &id,
                0,
                Speaker::System,
                &format!(
                    "{} started fresh with a summary of its last conversation",
                    agent.name()
                ),
            );
            Ok(())
        },
    )
}

pub(super) fn fork(
    daemon: &FakeDaemon,
    session: &SessionId,
    options: ForkOptions,
) -> Reply<SessionId> {
    let parent = session.clone();
    let kind = options.kind;
    let title = options.title.clone();
    daemon.reply(
        Some(Operation::Fork),
        format!("fork {session} {kind:?} {title:?}"),
        move |inner| {
            let source = inner.session_mut(&parent)?.clone();
            if title.trim().is_empty() {
                return Err(DaemonError::Refused {
                    what: "Creating the fork".into(),
                    reason: "it needs a title. Give it a title and try again".into(),
                });
            }
            let id = SessionId::new(inner.id("s-fork"));
            inner.last_fork = Some(options.clone());
            let mut fork = source.clone();
            fork.id = id.clone();
            if let Some(agent) = options.agent {
                fork.agent = agent;
            }
            if let Some(machine) = options.machine.clone() {
                fork.machine = machine;
            }
            fork.parent = Some(parent.clone());
            fork.fork = Some(kind);
            fork.title = title.clone();
            fork.status = Status::Idle;
            fork.turn = Turn::Ended;
            fork.pinned = false;
            fork.vip = false;
            fork.unread = 0;
            fork.archived = false;
            fork.started = "just now".into();
            fork.active = "just now".into();
            fork.last_used = "just now".into();
            fork.activity = if kind == ForkKind::Shared {
                "Fresh agent, no conversation".into()
            } else {
                "Conversation copied".into()
            };
            fork.branch = if kind == ForkKind::Conversation {
                source.branch.clone()
            } else if options.branch.trim().is_empty() {
                format!("humata/{id}")
            } else {
                options.branch.trim().to_owned()
            };
            fork.transcript = vec![Line {
                from: Speaker::System,
                text: match options.before_turn {
                    Some(turn) => format!(
                        "Forked from “{}” before turn {turn} ({})",
                        source.title,
                        kind.name()
                    ),
                    None => format!("Forked from “{}” at now ({})", source.title, kind.name()),
                },
            }];
            if kind == ForkKind::Shared {
                fork.transcript.push(Line {
                    from: Speaker::System,
                    text: "Fresh agent, no conversation".into(),
                });
            } else {
                fork.transcript.push(Line {
                    from: Speaker::System,
                    text: "Conversation copied".into(),
                });
                fork.transcript.extend(source.transcript.clone());
            }
            fork.shells = Vec::new();
            fork.terminal_agents.clear();
            fork.recovery = None;
            let at = inner
                .workspace
                .sessions
                .iter()
                .position(|found| found.id == parent)
                .map_or(inner.workspace.sessions.len(), |at| at + 1);
            if let Some(files) = inner.workspace.session_files.get(&parent).cloned() {
                inner.workspace.session_files.insert(id.clone(), files);
            }
            let contents: Vec<_> = inner
                .workspace
                .file_contents
                .iter()
                .filter(|((session, _, _), _)| session == &parent)
                .map(|((_, repo, path), body)| {
                    ((id.clone(), repo.clone(), path.clone()), body.clone())
                })
                .collect();
            inner.workspace.file_contents.extend(contents);
            inner.workspace.activity.insert(
                0,
                crate::model::Activity {
                    channel: source.channel.clone(),
                    session: Some(id.clone()),
                    kind: crate::model::ActivityKind::Fork,
                    text: format!("Forked “{}” → “{title}” ({})", source.title, kind.name()),
                    by: "You".into(),
                    at: "now".into(),
                },
            );
            inner.workspace.sessions.insert(at, fork);
            inner.changed(&id);
            inner.emit(Event::Resync);
            Ok(id)
        },
    )
}

pub(super) fn list_turns(daemon: &FakeDaemon, session: &SessionId) -> Reply<Vec<PastTurn>> {
    let id = session.clone();
    daemon.reply(None, format!("list turns {session}"), move |inner| {
        let found = inner.session_mut(&id)?;
        let mut turns = Vec::new();
        for line in found
            .transcript
            .iter()
            .filter(|line| line.from == Speaker::You)
        {
            let index = u32::try_from(turns.len() + 1).unwrap_or(u32::MAX);
            let text = line.text.lines().next().unwrap_or_default().to_owned();
            turns.push(PastTurn { index, text });
        }
        Ok(turns)
    })
}

/// CV-8: a new top-level session in `channel` with a copy of the session's
/// conversation, cut before `before_turn`, on the channel's first repo and
/// machine and a branch of its own. The session it came from is untouched.
pub(super) fn continue_in_channel(
    daemon: &FakeDaemon,
    session: &SessionId,
    channel: &ChannelId,
    before_turn: Option<u32>,
) -> Reply<SessionId> {
    let (from, to) = (session.clone(), channel.clone());
    daemon.reply(
        Some(Operation::ContinueInChannel),
        format!("continue {session} in {channel} before {before_turn:?}"),
        move |inner| {
            let source = inner.session_mut(&from)?.clone();
            let refuse = |reason: &str| DaemonError::Refused {
                what: "Continuing the session in another channel".into(),
                reason: reason.into(),
            };
            if crate::desk::is_desk(&source) {
                return Err(refuse(
                    "a Desk session can't go on in a channel. Move it into the channel instead",
                ));
            }
            if source.archived {
                return Err(refuse("it is archived. Restore it first, then continue it"));
            }
            if source.channel == to {
                return Err(refuse(
                    "it is already in that channel. Pick another channel, or fork it to go on here",
                ));
            }
            if before_turn.is_some_and(|turn| turn < 2) {
                return Err(refuse(
                    "nothing comes before the first turn. Continue from now or before a later turn",
                ));
            }
            let target = inner
                .workspace
                .channel(&to)
                .cloned()
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Channel {to}"),
                })?;
            let id = SessionId::new(inner.id("s-continue"));
            inner.last_continue = Some((from.clone(), to.clone(), before_turn));
            let mut copy = source.clone();
            copy.id = id.clone();
            copy.channel = to.clone();
            copy.parent = None;
            copy.fork = None;
            copy.machine = target
                .machines
                .first()
                .cloned()
                .unwrap_or_else(|| source.machine.clone());
            copy.repo = target.repos.first().cloned().unwrap_or_default();
            copy.branch = format!("humata/{id}");
            copy.status = Status::Idle;
            copy.turn = Turn::Ended;
            copy.pinned = false;
            copy.vip = false;
            copy.unread = 0;
            copy.archived = false;
            copy.started = "just now".into();
            copy.active = "just now".into();
            copy.last_used = "just now".into();
            copy.activity = format!("Continued from “{}”", source.title);
            copy.transcript = cut_before(&source.transcript, before_turn);
            copy.shells = Vec::new();
            copy.terminal_agents.clear();
            copy.recovery = None;
            copy.resume_failed = None;
            copy.moving = None;
            inner.workspace.sessions.push(copy);
            inner.changed(&id);
            inner.emit(Event::Resync);
            Ok(id)
        },
    )
}

/// The lines before turn `before`, the `before`th line typed by You; all of
/// them with `None`.
fn cut_before(transcript: &[Line], before: Option<u32>) -> Vec<Line> {
    let Some(before) = before else {
        return transcript.to_vec();
    };
    let mut turns = 0_u32;
    transcript
        .iter()
        .take_while(|line| {
            if line.from == Speaker::You {
                turns += 1;
            }
            turns < before
        })
        .cloned()
        .collect()
}

pub(super) fn move_session(daemon: &FakeDaemon, session: &SessionId, to: &MachineId) -> Reply<()> {
    let (id, to) = (session.clone(), to.clone());
    daemon.reply(
        Some(Operation::MoveSession),
        format!("move {session} to {to}"),
        move |inner| {
            let online = inner
                .workspace
                .machines
                .iter()
                .find(|machine| machine.id == to)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Machine {to}"),
                })?
                .online;
            if !online {
                return Err(DaemonError::Offline { machine: to });
            }
            let found = inner.session_mut(&id)?;
            let working = found.status == Status::Working;
            let mut shown = found.clone();
            // MV-4, MV-17: the steps one after the other, `copy` with rising
            // bytes; a handful of events, well under EVENT_QUEUE.
            let operation = crate::machines::OperationId::new(format!("move-{id}"));
            let steps: [(&str, u64, &[u64]); 4] = [
                ("Pausing the agent", 0, &[0]),
                ("Packing the session", 0, &[0]),
                ("Copying the session", 1000, &[250, 600, 1000]),
                ("Resuming the agent", 0, &[0]),
            ];
            for (at, (step, total_units, dones)) in steps.iter().copied().enumerate() {
                for done_units in dones {
                    shown.status = Status::Moving;
                    shown.turn = Turn::Ended;
                    shown.moving = Some(crate::model::MoveProgress {
                        to: to.clone(),
                        operation: operation.clone(),
                        step: step.to_owned(),
                        done_units: *done_units,
                        total_units,
                        // Earlier steps done, this one running, later ones pending.
                        steps: steps
                            .iter()
                            .enumerate()
                            .map(|(index, (title, total, _))| {
                                let (state, done) = match index.cmp(&at) {
                                    std::cmp::Ordering::Less => (crate::model::MoveStepState::Done, *total),
                                    std::cmp::Ordering::Equal => (crate::model::MoveStepState::Running, *done_units),
                                    std::cmp::Ordering::Greater => (crate::model::MoveStepState::Pending, 0),
                                };
                                crate::model::MoveStep {
                                    title: (*title).to_owned(),
                                    state,
                                    done_units: done,
                                    total_units: *total,
                                }
                            })
                            .collect(),
                    });
                    inner.emit(Event::Session(shown.clone()));
                }
            }
            let found = inner.session_mut(&id)?;
            found.machine = to.clone();
            found.moving = None;
            (found.status, found.turn) = if working {
                (Status::Working, Turn::Running)
            } else {
                (Status::Idle, Turn::Ended)
            };
            let changed = found.clone();
            inner.last_move = Some((id, to));
            inner.emit(Event::Session(changed));
            Ok(())
        },
    )
}

pub(super) fn rename_session(
    daemon: &FakeDaemon,
    session: &SessionId,
    title: &str,
    rename_branch: bool,
) -> Reply<()> {
    let (id, title) = (session.clone(), title.to_owned());
    daemon.reply(
        Some(Operation::RenameSession),
        format!("rename {session} to {title}"),
        move |inner| {
            let found = inner.session_mut(&id)?;
            if rename_branch {
                let prefix = found
                    .branch
                    .rsplit_once('/')
                    .map_or("humata", |(prefix, _)| prefix);
                found.branch = format!("{prefix}/{}", slug(&title));
            }
            found.title = title;
            let changed = found.clone();
            inner.emit(Event::Session(changed));
            Ok(())
        },
    )
}

/// `title` as a branch name: lower-case words joined by dashes.
fn slug(title: &str) -> String {
    title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub(super) fn stop_agent(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let id = session.clone();
    daemon.reply(
        Some(Operation::StopSession),
        format!("stop the agent of {session}"),
        move |inner| {
            let found = inner.session_mut(&id)?;
            // D784: a stopped agent reads as stopped, and Resume brings it
            // back; one that already exited stays exited.
            if !matches!(found.status, Status::Exited { .. }) {
                found.status = Status::AgentStopped;
                found.turn = Turn::Ended;
            }
            let changed = found.clone();
            inner.emit(Event::Session(changed));
            Ok(())
        },
    )
}

pub(super) fn plan_archive(daemon: &FakeDaemon, session: &SessionId) -> Reply<ArchivePlan> {
    let id = session.clone();
    daemon.reply(None, format!("plan archive {session}"), move |inner| {
        if inner.workspace.session(&id).is_none() {
            return Err(DaemonError::NotFound {
                what: format!("Session {id}"),
            });
        }
        let forks = inner
            .workspace
            .sessions
            .iter()
            .filter(|s| s.parent.as_ref() == Some(&id) && !s.archived)
            .count();
        Ok(ArchivePlan {
            forks_keep_running: u32::try_from(forks).unwrap_or(u32::MAX),
            unpushed: inner
                .unpushed
                .get(&id)
                .into_iter()
                .flatten()
                .map(|(repo, what)| UnpushedWork {
                    repo: repo.clone(),
                    what: what.clone(),
                })
                .collect(),
            frees: inner.archive_frees.get(&id).cloned(),
            unsubmitted_drafts: inner.unsubmitted_drafts.get(&id).copied().unwrap_or(0),
        })
    })
}

pub(super) fn archive(
    daemon: &FakeDaemon,
    session: &SessionId,
    push_first: bool,
    end: bool,
) -> Reply<Archive> {
    let id = session.clone();
    daemon.reply(
        Some(Operation::Archive),
        format!("archive {session}"),
        move |inner| {
            let snapshot =
                inner
                    .workspace
                    .session(&id)
                    .cloned()
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("Session {id}"),
                    })?;
            if snapshot.archived {
                return Err(DaemonError::Refused {
                    what: format!("Archiving {}", snapshot.title),
                    reason: "it is already archived. Restore it before archiving again".into(),
                });
            }
            let children = inner
                .workspace
                .sessions
                .iter()
                .filter(|s| s.parent.as_ref() == Some(&id))
                .map(|s| s.id.clone())
                .collect();
            inner.last_archive = Some((id.clone(), push_first));
            inner.last_archive_end = Some(end);
            if end {
                inner
                    .running
                    .retain(|holder| holder.session.as_ref() != Some(&id));
            } else {
                for holder in &mut inner.running {
                    if holder.session.as_ref() == Some(&id) {
                        holder.archived = true;
                    }
                }
            }
            inner.archived.insert(id.clone(), (snapshot, children));
            let parent = {
                let found = inner
                    .workspace
                    .sessions
                    .iter_mut()
                    .find(|found| found.id == id)
                    .ok_or_else(|| DaemonError::NotFound {
                        what: format!("Session {id}"),
                    })?;
                found.archived = true;
                found.pinned = false;
                found.parent.clone()
            };
            // FK-13: its forks move up to its parent.
            for child in inner
                .workspace
                .sessions
                .iter_mut()
                .filter(|child| child.parent.as_ref() == Some(&id))
            {
                child.parent.clone_from(&parent);
            }
            inner.emit(Event::Resync);
            Ok(Archive {
                frees: inner.archive_frees.get(&id).cloned(),
            })
        },
    )
}

pub(super) fn restore_session(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let id = session.clone();
    daemon.reply(None, format!("restore session {session}"), move |inner| {
        let (snapshot, children) =
            inner
                .archived
                .remove(&id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Archived session {id}. It may already have been restored"),
                })?;
        let restored_parent = snapshot.parent.clone();
        let found = inner
            .workspace
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Session {id}"),
            })?;
        // Restore archival metadata without overwriting output received since archive.
        found.archived = false;
        found.pinned = snapshot.pinned;
        found.parent = restored_parent.clone();
        for child in inner
            .workspace
            .sessions
            .iter_mut()
            .filter(|s| children.contains(&s.id) && s.parent == restored_parent)
        {
            child.parent = Some(id.clone());
        }
        inner.emit(Event::Resync);
        Ok(())
    })
}

pub(super) fn set_pinned(daemon: &FakeDaemon, session: &SessionId, pinned: bool) -> Reply<()> {
    let id = session.clone();
    daemon.reply(None, format!("pin {session} {pinned}"), move |inner| {
        let found = inner
            .workspace
            .sessions
            .iter_mut()
            .find(|found| found.id == id)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Session {id}"),
            })?;
        found.pinned = pinned;
        inner.changed(&id);
        Ok(())
    })
}

pub(super) fn set_vip(daemon: &FakeDaemon, session: &SessionId, vip: bool) -> Reply<()> {
    let id = session.clone();
    daemon.reply(None, format!("vip {session} {vip}"), move |inner| {
        let found = inner
            .workspace
            .sessions
            .iter_mut()
            .find(|found| found.id == id)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Session {id}"),
            })?;
        found.vip = vip;
        inner.changed(&id);
        Ok(())
    })
}

pub(super) fn mark_read(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let id = session.clone();
    daemon.reply(None, format!("read {session}"), move |inner| {
        super::desk::used(inner, &id);
        // Opening a session views it, as on the daemon (D392).
        inner.viewing = Some(id.clone());
        let noted = |recovery: &Option<crate::model::Recovery>| {
            matches!(
                recovery,
                Some(crate::model::Recovery::Reattached | crate::model::Recovery::Resumed)
            )
        };
        if let Some(found) = inner
            .workspace
            .sessions
            .iter_mut()
            .find(|found| found.id == id)
            && (found.unread > 0 || noted(&found.recovery))
        {
            found.unread = 0;
            // A session that couldn't come back keeps its card until it is
            // resumed or started fresh.
            if noted(&found.recovery) {
                found.recovery = None;
            }
            inner.changed(&id);
        }
        Ok(())
    })
}

/// The session the user views, or none (CH-16, CH-17; D392): its output
/// marks nothing, and viewing it clears its mark. Answers at once and isn't
/// logged as a call: every navigation sends it.
pub(super) fn view(daemon: &FakeDaemon, session: Option<&SessionId>) -> Reply<()> {
    let shared = daemon.shared.clone();
    let id = session.cloned();
    Box::pin(async move {
        let mut inner = match shared.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.viewing.clone_from(&id);
        if let Some(id) = id
            && let Some(found) = inner
                .workspace
                .sessions
                .iter_mut()
                .find(|found| found.id == id)
            && found.unread > 0
        {
            found.unread = 0;
            inner.changed(&id);
        }
        Ok(())
    })
}

pub(super) fn create_session(
    daemon: &FakeDaemon,
    channel: &ChannelId,
    agent: Agent,
    machine: &MachineId,
    title: &str,
    options: crate::model::SessionOptions,
) -> Reply<SessionId> {
    let channel = channel.clone();
    let machine = machine.clone();
    let title = title.to_owned();
    daemon.reply(
        Some(Operation::CreateSession),
        format!("create session {channel} {agent:?} {machine} {title:?}"),
        move |inner| {
            let online = inner
                .workspace
                .machine(&machine)
                .is_some_and(|found| found.online);
            if !online {
                return Err(DaemonError::Offline { machine });
            }
            let config=inner.workspace.channel(&channel).ok_or_else(||DaemonError::NotFound {what:format!("Channel {channel}")})?;
            if options.repos.iter().any(|repo|!config.repos.contains(repo)) {return Err(DaemonError::Refused {what:"Starting the session".into(),reason:"one of its repositories is outside this channel. Pick this channel's repositories and try again".into()});}
            if let Some(item)=&options.item {
                let found=inner.workspace.items.iter().find(|i|&i.id==item).ok_or_else(||DaemonError::NotFound {what:format!("Board item {item}")})?;
                if matches!(&found.scope,Scope::Channel(id) if id!=&channel) {return Err(DaemonError::Refused {what:"Starting an item session".into(),reason:"the item belongs to another channel. Use the item's channel".into()});}
            }
            if !inner.workspace.channel_machines(&channel).any(|m| m.id == machine) {
                return Err(DaemonError::Refused { what: "Starting the session".into(), reason: "the machine is outside this channel. Choose a connected machine or add it to the channel first".into() });
            }
            let machine_name = inner.workspace.machine(&machine).map_or_else(|| machine.to_string(), |m| m.name.clone());
            let repo=options.repos.first().cloned().unwrap_or_default();
            let id = SessionId::new(inner.id("s-new"));
            inner.workspace.sessions.push(Session {
                id: id.clone(),
                channel: channel.clone(),
                parent: None,
                fork: None,
                title: title.clone(),
                agent,
                machine,
                status: Status::Idle,
                turn: Turn::Ended,
                pinned: false,
                vip: false,
                unread: 0,
                repo,
                branch: if options.branch.is_empty(){id.to_string()}else{options.branch.clone()},
                started: "just now".to_owned(),
                active: "just now".to_owned(),
                last_used: "just now".to_owned(),
                activity: String::new(),
                transcript: vec![Line {
                    from: Speaker::System,
                    text: format!("{} started · {title}", agent.name()),
                }],
                shells: Vec::new(),
                terminal_agents: Default::default(),
                archived: false,
                recovery: None,
                resume_failed: None,
                interrupted_unix_ms: None,
                away_left_out: None,
                summary_source: None,
                model: None,
                restart_detail: None,
                permissions: Vec::new(),
                pull_request_review: false,
                push_allowed: false,
                moving: None,
                folder: format!("/Users/me/Humata/{channel}/sessions/{id}"),
                status_since_ms: None,
                clones: options
                    .repos
                    .iter()
                    .map(|each| SessionClone {
                        repo: each.clone(),
                        branch: if options.branch.is_empty() { id.to_string() } else { options.branch.clone() },
                        path: format!(
                            "/Users/me/Humata/{channel}/sessions/{id}/{}",
                            each.rsplit('/').next().unwrap_or(each)
                        ),
                    })
                    .collect(),
            });
            if !options.first_message.is_empty() {
                if let Some(session)=inner.workspace.sessions.last_mut(){session.transcript.push(Line {from:Speaker::You,text:options.first_message.clone()});session.status=Status::Working;session.turn=Turn::Running;}
            }
            if let Some(item)=&options.item {
                if let Some(item)=inner.workspace.items.iter_mut().find(|i|&i.id==item) {item.session=Some(id.clone());if matches!(item.status,ItemStatus::Backlog|ItemStatus::Todo){item.status=ItemStatus::InProgress;}}
                let working=inner.workspace.item_details.entry(item.clone()).or_default();
                if !working.sessions.contains(&id){working.sessions.push(id.clone());}
            }
            inner.workspace.activity.insert(0,crate::model::Activity {channel,session:Some(id.clone()),kind:crate::model::ActivityKind::Turn,text:format!("Started “{title}” with {} on {machine_name}",agent.name()),by:"You".into(),at:"now".into()});
            inner.workspace.session_options.insert(id.clone(),options);
            inner.emit(Event::Resync);
            inner.changed(&id);
            Ok(id)
        },
    )
}

/// The agent's answer to `text`, a line at a time.
fn answer_lines(text: &str) -> Vec<String> {
    vec![
        format!("Looking into “{}”.", text.trim()),
        "Read src/lib.rs (212 lines)".to_owned(),
        "Done. Anything else?".to_owned(),
    ]
}

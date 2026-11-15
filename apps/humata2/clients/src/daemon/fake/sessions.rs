use tokio::sync::mpsc;

use super::{FakeDaemon, Subscriber};
use crate::daemon::{DaemonError, EVENT_QUEUE, Event, Events, Operation, Reply};
use crate::model::{
    Agent, ChannelId, ForkKind, ItemStatus, Line, MachineId, Scope, Session, SessionId, Speaker,
    Status, Turn,
};
use crate::workspace::Workspace;

pub(super) fn load(daemon: &FakeDaemon) -> Reply<Workspace> {
    daemon.reply(Some(Operation::Load), "load".to_owned(), |inner| {
        Ok(inner.workspace.clone())
    })
}

pub(super) fn events(daemon: &FakeDaemon) -> Reply<Events> {
    daemon.reply(None, "events".to_owned(), |inner| {
        let (events, receiver) = mpsc::channel(EVENT_QUEUE);
        inner.subscribers.push(Subscriber {
            events,
            behind: false,
        });
        Ok(receiver)
    })
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

pub(super) fn fork(
    daemon: &FakeDaemon,
    session: &SessionId,
    kind: ForkKind,
    title: &str,
) -> Reply<SessionId> {
    let parent = session.clone();
    let title = title.to_owned();
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
            let mut fork = source.clone();
            fork.id = id.clone();
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
            fork.activity = if kind == ForkKind::Shared {
                "Fresh agent, no conversation".into()
            } else {
                "Conversation copied".into()
            };
            fork.branch = if kind == ForkKind::Conversation {
                source.branch.clone()
            } else {
                format!("humata/{id}")
            };
            fork.transcript = vec![Line {
                from: Speaker::System,
                text: format!("Forked from “{}” at now ({})", source.title, kind.name()),
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

pub(super) fn archive(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
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
            Ok(())
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
        if let Some(found) = inner
            .workspace
            .sessions
            .iter_mut()
            .find(|found| found.id == id)
            && (found.unread > 0 || found.recovery.is_some())
        {
            found.unread = 0;
            found.recovery = None;
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
                activity: String::new(),
                transcript: vec![Line {
                    from: Speaker::System,
                    text: format!("{} started · {title}", agent.name()),
                }],
                shells: Vec::new(),
                terminal_agents: Default::default(),
                archived: false,
                recovery: None,
            });
            if !options.first_message.is_empty() {
                if let Some(session)=inner.workspace.sessions.last_mut(){session.transcript.push(Line {from:Speaker::You,text:options.first_message.clone()});session.status=Status::Working;session.turn=Turn::Running;}
            }
            if let Some(item)=&options.item {
                if let Some(item)=inner.workspace.items.iter_mut().find(|i|&i.id==item) {item.session=Some(id.clone());if matches!(item.status,ItemStatus::Backlog|ItemStatus::Todo){item.status=ItemStatus::InProgress;}}
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

use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{
    Answer, AskedWhy, Grant, GrantId, ItemStatus, MigratePhase, Preview, Request, RequestId, RequestKind,
    Status,
};

impl FakeDaemon {
    /// Every answer sent, in order, including ones that failed.
    #[must_use]
    pub fn answers(&self) -> Vec<(RequestId, Answer)> {
        self.inner().answers.clone()
    }

    /// Adds a request, or replaces the one with the same id, and says so.
    pub fn put_request(&self, request: Request) {
        let mut inner = self.inner();
        match inner
            .workspace
            .requests
            .iter_mut()
            .find(|found| found.id == request.id)
        {
            Some(found) => *found = request.clone(),
            None => inner.workspace.requests.insert(0, request.clone()),
        }
        inner.emit(Event::Request(request));
    }

    /// The agent's reply to Ask why on a request, as the daemon sets it
    /// once the agent answers. Does nothing unless the owner asked why.
    pub fn reply_why(&self, id: &RequestId, reply: &str) {
        let mut inner = self.inner();
        let Some(found) = inner.workspace.requests.iter_mut().find(|found| &found.id == id) else {
            return;
        };
        let Some(asked) = &mut found.asked_why else {
            return;
        };
        asked.reply = Some(reply.to_owned());
        asked.replied = Some("now".to_owned());
        let request = found.clone();
        inner.emit(Event::Request(request));
    }

    /// Every request Ask why was sent for, in order.
    #[must_use]
    pub fn asked_why(&self) -> Vec<RequestId> {
        self.inner().asked_why.clone()
    }

    /// Removes a request, as the daemon does when it resolves by itself (a
    /// Migrate item whose machine came back at the host's release).
    pub fn withdraw_request(&self, id: &RequestId) {
        let mut inner = self.inner();
        inner.workspace.requests.retain(|found| &found.id != id);
        inner.emit(Event::RequestGone(id.clone()));
    }
}

pub(super) fn answer(daemon: &FakeDaemon, request: &RequestId, answer: Answer) -> Reply<()> {
    let id = request.clone();
    daemon.inner().answers.push((id.clone(), answer.clone()));
    daemon.reply(
        Some(Operation::Answer),
        format!("answer {request} {answer:?}"),
        move |inner| {
            let found = inner
                .workspace
                .requests
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Request {id}"),
                })?;
            if found.answer.is_some() {
                return Err(DaemonError::Refused {
                    what: format!("Answering “{}”", found.title),
                    reason: "it was already answered, perhaps from another window".to_owned(),
                });
            }
            if found.kind == RequestKind::Migrate {
                // A Migrate item takes Approve only; the daemon migrates and
                // answers it when it has arrived, so it stays pending now.
                if answer != Answer::Approved {
                    return Err(DaemonError::Refused {
                        what: format!("Answering “{}”", found.title),
                        reason: "a Migrate item can only be migrated. Click Migrate, or leave it for later".to_owned(),
                    });
                }
                if let Preview::Migrate(card) = &mut found.preview {
                    card.phase = MigratePhase::Migrating;
                }
                let request = found.clone();
                inner.emit(Event::Request(request));
                return Ok(());
            }
            if matches!(found.kind, RequestKind::BrowserRead | RequestKind::BrowserWrite) {
                if matches!(answer, Answer::Always { .. }) {
                    return Err(DaemonError::Refused {
                        what: format!("Answering “{}”", found.title),
                        reason: "browser access is never allowed always".to_owned(),
                    });
                }
                if found.kind == RequestKind::BrowserWrite
                    && matches!(answer, Answer::AllowFor30Min)
                {
                    return Err(DaemonError::Refused {
                        what: format!("Answering “{}”", found.title),
                        reason: "browser writes are asked every time and can't be allowed for 30 minutes".to_owned(),
                    });
                }
            } else if matches!(answer, Answer::AllowFor30Min) {
                return Err(DaemonError::Refused {
                    what: format!("Answering “{}”", found.title),
                    reason: "30-minute allow only applies to browser read requests".to_owned(),
                });
            }
            found.answer = Some(answer);
            let request = found.clone();
            super::desk::used(inner, &request.session);
            if let (Some(Answer::Always { rule }), Some(asked_by)) =
                (&request.answer, &request.asked_by)
            {
                let grant = Grant {
                    id: GrantId::new(inner.id("g")),
                    session: request.session.clone(),
                    scope: rule.clone(),
                    granted: "now".to_owned(),
                };
                if let Some(row) =
                    inner
                        .workspace
                        .rules
                        .get_mut(&request.channel)
                        .and_then(|rules| {
                            rules
                                .iter_mut()
                                .find(|row| row.capability == asked_by.capability)
                        })
                {
                    row.grants.push(grant);
                    let rules = inner
                        .workspace
                        .rules
                        .get(&request.channel)
                        .cloned()
                        .unwrap_or_default();
                    inner.emit(Event::Rules {
                        channel: request.channel.clone(),
                        rules,
                    });
                }
            }
            let session = request.session.clone();
            if matches!(
                request.answer,
                Some(Answer::Approved | Answer::Always { .. })
            ) {
                let changes: Vec<_> = inner
                    .workspace
                    .item_details
                    .iter()
                    .filter(|(_, detail)| detail.request.as_ref() == Some(&id))
                    .filter_map(|(item, detail)| {
                        detail.requested_status.map(|status| (item.clone(), status))
                    })
                    .collect();
                for (id, status) in changes {
                    if let Some(item) = inner.workspace.items.iter_mut().find(|item| item.id == id)
                    {
                        item.status = status;
                        let details = inner.workspace.item_details.entry(id).or_default();
                        details.done_day = (status == ItemStatus::Done).then_some(13);
                        details
                            .history
                            .insert(0, format!("Approved move to {} · now", status.name()));
                    }
                }
                inner.emit(Event::Resync);
            }
            inner.emit(Event::Request(request));
            let still = inner
                .workspace
                .waiting()
                .any(|waiting| waiting.session == session);
            if !still
                && let Some(found) = inner
                    .workspace
                    .sessions
                    .iter_mut()
                    .find(|found| found.id == session)
                && found.status == Status::NeedsYou
            {
                found.status = Status::Working;
                inner.changed(&session);
            }
            Ok(())
        },
    )
}

pub(super) fn ask_why(daemon: &FakeDaemon, request: &RequestId) -> Reply<Request> {
    let id = request.clone();
    daemon.inner().asked_why.push(id.clone());
    daemon.reply(
        Some(Operation::AskWhy),
        format!("ask why {request}"),
        move |inner| {
            let found = inner
                .workspace
                .requests
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Request {id}"),
                })?;
            if found.answer.is_some() {
                return Err(DaemonError::Refused {
                    what: format!("Asking why for “{}”", found.title),
                    reason: "it was already answered, perhaps from another window".to_owned(),
                });
            }
            if found.asked_why.is_none() {
                found.asked_why = Some(AskedWhy {
                    asked: "now".to_owned(),
                    reply: None,
                    replied: None,
                });
            }
            let request = found.clone();
            inner.emit(Event::Request(request.clone()));
            Ok(request)
        },
    )
}

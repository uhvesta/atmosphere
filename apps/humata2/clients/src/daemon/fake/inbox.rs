use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{Answer, ItemStatus, RequestId, Status};

pub(super) fn answer(daemon: &FakeDaemon, request: &RequestId, answer: Answer) -> Reply<()> {
    let id = request.clone();
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
            found.answer = Some(answer);
            let request = found.clone();
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

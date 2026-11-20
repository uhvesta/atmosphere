//! What a desktop notification says about an agent that stopped to ask
//! (frame 209). Posting it is a later milestone; this is only its content.

use humata2_clients::Workspace;
use humata2_clients::model::{Preview, Request, RequestKind, ToDoAsk};

/// A notification: what the person reads, never a way to answer. Opening it
/// opens the Agent inbox on the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// "<agent> is stopped: <ask>".
    pub title: String,
    /// What else moves or changes, then where to answer.
    pub body: String,
}

/// The notification for `request`, or none when its agent isn't held up by
/// it: a to-do ask (D846), an answered ask, or a Migrate item from Humata.
#[must_use]
pub fn notification(workspace: &Workspace, request: &Request) -> Option<Notice> {
    if request.answer.is_some()
        || request.kind == RequestKind::Migrate
        || request.is_to_do_ask()
        || request.operation.starts_with("board.")
    {
        return None;
    }
    let agent = workspace
        .session(&request.session)
        .map_or("An agent", |session| session.agent.command());
    let effect = request
        .affects
        .iter()
        .find(|effect| effect.warn)
        .or_else(|| request.affects.first())
        .map(|effect| format!("{} {}", effect.lead, effect.rest).trim().to_owned());
    let body = match effect {
        Some(effect) => format!("{effect} Open the Agent inbox to answer."),
        None => "Open the Agent inbox to answer.".to_owned(),
    };
    Some(Notice {
        title: format!("{agent} is stopped: {}", request.title),
        body,
    })
}

/// The notification for a To-do ask: an agent's suggested item or a change
/// to one. It says to answer on the To-do, never in the Agent inbox. None
/// when `request` isn't a To-do ask or is already answered.
#[must_use]
pub fn to_do_notification(workspace: &Workspace, request: &Request) -> Option<Notice> {
    let ask = request.to_do.as_ref()?;
    if request.answer.is_some() {
        return None;
    }
    let agent = workspace
        .session(&request.session)
        .map_or("An agent", |session| session.agent.command());
    let item = match &request.preview {
        Preview::Text(text) => text.lines().next().map(str::trim).filter(|l| !l.is_empty()),
        _ => None,
    }
    .unwrap_or(request.title.as_str());
    let title = match ask {
        ToDoAsk::NewItem => format!("{agent} suggests a To-do: {item}"),
        ToDoAsk::Item(_) => format!("{agent} wants to change '{item}'"),
    };
    Some(Notice { title, body: "Accept or dismiss it on the To-do.".to_owned() })
}

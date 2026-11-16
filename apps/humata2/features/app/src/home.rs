//! Home's "Needs you" group. One row per thing waiting for the person: a
//! session that needs it or has a pending Inbox request, and a pending request
//! whose session isn't listed. Its count is the number of rows; the Inbox
//! badge counts requests, which is what it means.
use humata2_clients::Workspace;
use humata2_clients::model::{Request, Session, SessionId, Status};

/// One row of Home's "Needs you" group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeedsRow<'a> {
    /// A session that needs the person, or has a pending request. Its first
    /// pending request, if any, is the ask shown on the row.
    Session {
        /// The session.
        session: &'a Session,
        /// Its pending request, if it has one.
        request: Option<&'a Request>,
    },
    /// A pending request whose session is not listed on Home.
    Request(&'a Request),
}

/// Home's "Needs you" group for a workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NeedsYou<'a> {
    /// The rows, sessions first, newest request first.
    pub rows: Vec<NeedsRow<'a>>,
}

impl NeedsYou<'_> {
    /// How many rows the group shows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether nothing needs the person.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Home's "Needs you" group for a workspace.
#[must_use]
pub fn needs_you(workspace: &Workspace) -> NeedsYou<'_> {
    let pending: Vec<&Request> = workspace
        .requests
        .iter()
        .filter(|request| request.answer.is_none())
        .collect();
    let mut listed: Vec<&SessionId> = Vec::new();
    let mut rows = Vec::new();
    for session in workspace.sessions.iter().filter(|session| !session.archived) {
        let request = pending
            .iter()
            .copied()
            .find(|request| request.session == session.id);
        if request.is_some() || matches!(session.status, Status::NeedsYou | Status::Exited { .. })
        {
            listed.push(&session.id);
            rows.push(NeedsRow::Session { session, request });
        }
    }
    rows.extend(
        pending
            .into_iter()
            .filter(|request| !listed.contains(&&request.session))
            .map(NeedsRow::Request),
    );
    NeedsYou { rows }
}

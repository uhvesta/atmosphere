//! Home's "Needs you" group (HM-1). One row per session that needs the
//! person: its agent waits on its own request (Needs you), it exited, or its
//! machine is offline. A pending request whose session isn't on Home gets a
//! row of its own. A request the agent stopped waiting on (D158.7, dot rule
//! B) leaves its session in Working or Recent; the ✉ badges and the Inbox
//! still count it. The group's count is its rows; the Inbox badge counts
//! requests.
use humata2_clients::Workspace;
use humata2_clients::model::{Request, Session, Status};

/// One row of Home's "Needs you" group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeedsRow<'a> {
    /// A session that needs the person.
    Session {
        /// The session.
        session: &'a Session,
        /// The request its agent waits on: the session's first pending
        /// request while it is Needs you, else none.
        request: Option<&'a Request>,
    },
    /// A pending request whose session is not on Home.
    Request(&'a Request),
}

/// Home's "Needs you" group for a workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NeedsYou<'a> {
    /// The rows: sessions in workspace order, then requests with no session.
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
    let shown: Vec<&Session> = workspace
        .sessions
        .iter()
        .filter(|session| !session.archived)
        .collect();
    let mut rows: Vec<NeedsRow<'_>> = shown
        .iter()
        .copied()
        .filter(|session| {
            matches!(
                session.status,
                Status::NeedsYou | Status::Exited { .. } | Status::Offline
            )
        })
        .map(|session| NeedsRow::Session {
            session,
            request: (session.status == Status::NeedsYou)
                .then(|| {
                    pending
                        .iter()
                        .copied()
                        .find(|request| request.session == session.id)
                })
                .flatten(),
        })
        .collect();
    rows.extend(
        pending
            .into_iter()
            .filter(|request| !shown.iter().any(|session| session.id == request.session))
            .map(NeedsRow::Request),
    );
    NeedsYou { rows }
}

//! Wiring adapter connecting `humata2_svc_terminal` (`RemoteTerminal`,
//! `running_holders`, `terminal_session`) to `humatad`'s concrete `Links`,
//! `SessionSvc`, `read_marks`, and `holder_end` (Wiring Wave 2).

use std::path::Path;
use std::sync::Arc;

use humata2_daemon_ports::{Caller, Gated};
use humata2_svc_terminal::ports::{
    LinkState, RelayError, RelayLinks, TerminalLifecycle, TerminalSessions,
};
use humata2_svc_terminal::RemoteTerminal;
use tokio::sync::watch;
use tonic::transport::Channel;
use tonic::Status;

use crate::host_relay::{LinkState as HostLinkState, Links};
use crate::session_service::SessionSvc;

/// Adapts `crate::host_relay::Links` to [`RelayLinks`].
pub(crate) struct LinksAdapter(pub(crate) Links);

impl RelayLinks for LinksAdapter {
    fn channel(&self, machine: &str) -> Result<Channel, RelayError> {
        self.0
            .channel(machine)
            .map_err(|_| RelayError::Offline {
                machine: machine.to_owned(),
            })
    }

    fn state(&self, machine: &str) -> LinkState {
        match *self.0.watch(machine).borrow() {
            HostLinkState::Up { generation } => LinkState::Up(generation),
            HostLinkState::Down => LinkState::Down,
        }
    }

    fn watch(&self, machine: &str) -> watch::Receiver<LinkState> {
        let mut inner = self.0.watch(machine);
        let initial = match *inner.borrow() {
            HostLinkState::Up { generation } => LinkState::Up(generation),
            HostLinkState::Down => LinkState::Down,
        };
        let (tx, rx) = watch::channel(initial);
        tokio::spawn(async move {
            while inner.changed().await.is_ok() {
                let mapped = match *inner.borrow() {
                    HostLinkState::Up { generation } => LinkState::Up(generation),
                    HostLinkState::Down => LinkState::Down,
                };
                if tx.send(mapped).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

/// D784: a stopped shell starts again through the restart doctor's repair,
/// not a session resume: its id is a tab's, and a session's resume brings
/// back an agent.
async fn resume_shell_via(svc: &SessionSvc, caller: &Caller, tab: &str) -> Result<(), Status> {
    match svc.repair_tab(caller, tab.to_owned(), true).await? {
        Gated::Done(()) => Ok(()),
        Gated::Ask(_) => Err(Status::permission_denied(
            "starting this shell again needs the user's approval; resume it from the app",
        )),
    }
}

/// Adapts `crate::session_service::SessionSvc` to [`TerminalSessions`].
pub(crate) struct SessionsAdapter(pub(crate) SessionSvc);

#[tonic::async_trait]
impl TerminalSessions for SessionsAdapter {
    fn is_moving(&self, session: &str) -> bool {
        crate::session_service::is_moving(session)
    }

    fn moving_changes(&self) -> watch::Receiver<u64> {
        crate::session_service::moving_changes()
    }

    fn paused_for_move(&self, socket: &Path, pid: u32) -> bool {
        crate::session_service::paused_for_move(socket, pid)
    }

    fn refuse_input_while_moving(&self, session: &str) -> Result<(), Status> {
        crate::session_service::refuse_input_while_moving(session)
    }

    async fn resume_shell(&self, caller: &Caller, tab: &str) -> Result<(), Status> {
        resume_shell_via(&self.0, caller, tab).await
    }
}

/// Adapts `crate::read_marks` + `crate::holder_end` to [`TerminalLifecycle`].
pub(crate) struct LifecycleAdapter;

impl TerminalLifecycle for LifecycleAdapter {
    fn is_viewed(&self, session: &str) -> bool {
        crate::read_marks::is_viewed(session)
    }

    fn tab_typed(&self, tab: &str) {
        crate::read_marks::typed(tab);
    }

    fn end_holder_at(&self, socket: &Path, owner: &str) -> Result<(), String> {
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            handle.block_on(crate::holder_end::shut_or_end(socket, 0, owner))
        })
        .map_err(|error| error.to_string())
    }

    fn shells_changed(&self, session: &str) {
        crate::session_service::shells_changed(session);
    }
}

/// Adapts a late-bound `OnceLock<SessionSvc>` to [`TerminalSessions`].
pub(crate) struct LateSessionsAdapter(pub(crate) Arc<std::sync::OnceLock<SessionSvc>>);

#[tonic::async_trait]
impl TerminalSessions for LateSessionsAdapter {
    fn is_moving(&self, session: &str) -> bool {
        crate::session_service::is_moving(session)
    }

    fn moving_changes(&self) -> watch::Receiver<u64> {
        crate::session_service::moving_changes()
    }

    fn paused_for_move(&self, socket: &Path, pid: u32) -> bool {
        crate::session_service::paused_for_move(socket, pid)
    }

    fn refuse_input_while_moving(&self, session: &str) -> Result<(), Status> {
        crate::session_service::refuse_input_while_moving(session)
    }

    async fn resume_shell(&self, caller: &Caller, tab: &str) -> Result<(), Status> {
        match self.0.get() {
            Some(s) => resume_shell_via(s, caller, tab).await,
            None => Err(Status::unavailable("session service not yet initialized")),
        }
    }
}

/// Constructs [`humata2_svc_terminal::RemoteTerminal`] wired to `humatad`'s
/// concrete `Links` and a late-bound `SessionSvc` slot.
pub(crate) fn build_remote_terminal_late(
    links: Links,
    sessions: Arc<std::sync::OnceLock<SessionSvc>>,
) -> RemoteTerminal {
    RemoteTerminal::new(
        Arc::new(LinksAdapter(links)),
        Arc::new(LateSessionsAdapter(sessions)),
    )
}

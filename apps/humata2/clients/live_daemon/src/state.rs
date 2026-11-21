//! The shared state behind every clone of [`LiveDaemon`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use humata2_clients::desk::{
    Access, ChannelDeskSession, CleanUp, CleanUpRow, DeskClient, Document, NewChannelDesk,
    NewDeskSession, Switches,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{DaemonError, Event, Reply};
use humata2_clients_live_core::map::AgentNames;
pub use humata2_clients_live_core::{
    ConnectError, DaemonFacts, LiveConnection, LiveOptions,
};
pub(crate) use humata2_clients_live_core::{
    MESSAGE_MAX_BYTES, Wire, bad_repo, daemon_error, fresh_key, key_of, not_in_this_release, said,
    status_error,
};
use humata2_clients_live_desk::LiveDesk;
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::{
    access_service_client::AccessServiceClient, agent_service_client::AgentServiceClient,
    board_service_client::BoardServiceClient,
    browser_bridge_service_client::BrowserBridgeServiceClient,
    channel_service_client::ChannelServiceClient,
    cli_auth_service_client::CliAuthServiceClient,
    daemon_service_client::DaemonServiceClient,
    event_service_client::EventServiceClient, inbox_service_client::InboxServiceClient,
    operation_service_client::OperationServiceClient,
    session_service_client::SessionServiceClient, settings_service_client::SettingsServiceClient,
    terminal_service_client::TerminalServiceClient,
};
use tonic::Status;

use crate::events::Subscriber;

#[derive(Default)]
pub(crate) struct State {
    pub(crate) subscribers: Vec<Subscriber>,
    /// Whether the one EventService subscription is running.
    pub(crate) subscribed: bool,
    /// Each session's tabs, in the app's order.
    pub(crate) tabs: BTreeMap<String, Vec<pb::Tab>>,
    /// Whether this host's restart report has been pushed or loaded.
    pub(crate) restart_known: bool,
    /// Whether a server-info read for the restart report is in flight.
    pub(crate) restart_reading: bool,
    /// Whether a cue to read it came while a read was in flight, which may
    /// have started before the daemon published the report.
    pub(crate) restart_again: bool,
    /// The channel name of each setup (operation id), for its events.
    pub(crate) setup_names: BTreeMap<String, String>,
    /// The channel each setup (operation id) makes, so a retry can name it.
    pub(crate) setup_channels: BTreeMap<String, ChannelId>,
    /// What the Inbox last showed of each request, for answering it.
    pub(crate) inbox: crate::inbox::Seen,
    /// Each moving session (by id) with the move being watched for it.
    pub(crate) moves: BTreeMap<String, crate::events::Move>,
    /// The session this client views, held open on the daemon (D392).
    pub(crate) viewing: Option<crate::read_marks::Viewing>,
}

impl State {
    /// A cue that the restart report may be there now. True when the caller
    /// should read it; a cue during a read is kept for [`Self::restart_read_ended`].
    pub(crate) fn restart_cue(&mut self) -> bool {
        if self.restart_known {
            return false;
        }
        if self.restart_reading {
            self.restart_again = true;
            return false;
        }
        self.restart_reading = true;
        true
    }

    /// A read of the report ended; `known` is whether it found the daemon's
    /// answer. True when it must be read again: it wasn't known, and a cue
    /// came while that read was in flight.
    pub(crate) fn restart_read_ended(&mut self, known: bool) -> bool {
        self.restart_known |= known;
        let again = std::mem::take(&mut self.restart_again) && !self.restart_known;
        self.restart_reading = again;
        again
    }
}

pub(crate) struct Shared {
    pub(crate) conn: LiveConnection,
    pub(crate) wire: Wire,
    pub(crate) agents: AgentNames,
    /// This machine's id as the app knows it.
    pub(crate) machine: MachineId,
    state: Mutex<State>,
}

/// The app's daemon client over one socket. Cheap to clone.
#[derive(Clone)]
pub struct LiveDaemon {
    pub(crate) shared: Arc<Shared>,
    desk: LiveDesk,
}

impl LiveDaemon {
    /// Connects to the daemon's socket and asks who it is.
    pub async fn connect(options: LiveOptions) -> Result<Self, ConnectError> {
        LiveConnection::connect(options)
            .await
            .map(Self::from_connection)
    }

    /// Builds a daemon client over an existing [`LiveConnection`].
    #[must_use]
    pub fn from_connection(conn: LiveConnection) -> Self {
        let desk = LiveDesk::new(&conn);
        let shared = Arc::new(Shared {
            wire: conn.wire(),
            agents: conn.agents().clone(),
            machine: conn.machine().clone(),
            conn: conn.clone(),
            state: Mutex::new(State::default()),
        });
        let weak = Arc::downgrade(&shared);
        conn.set_progress_sink(move |update| {
            if let Some(shared) = weak.upgrade() {
                shared.publish(Event::Operation(update));
            }
        });
        Self { shared, desk }
    }

    /// What the daemon said about itself when the app connected.
    #[must_use]
    pub fn facts(&self) -> &DaemonFacts {
        self.shared.conn.facts()
    }

    /// The underlying shared socket connection.
    #[must_use]
    pub fn connection(&self) -> &LiveConnection {
        &self.shared.conn
    }
}

impl AsRef<LiveConnection> for LiveDaemon {
    fn as_ref(&self) -> &LiveConnection {
        &self.shared.conn
    }
}

impl Shared {
    /// Runs `task` on the runtime the client connected on, whoever polls
    /// the caller.
    pub(crate) fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.conn.spawn(task)
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn daemon_info(&self) -> DaemonServiceClient<Wire> {
        DaemonServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn channels(&self) -> ChannelServiceClient<Wire> {
        ChannelServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn inbox(&self) -> InboxServiceClient<Wire> {
        InboxServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn access(&self) -> AccessServiceClient<Wire> {
        AccessServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn cli_auth(&self) -> CliAuthServiceClient<Wire> {
        CliAuthServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn agents(&self) -> AgentServiceClient<Wire> {
        AgentServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn settings(&self) -> SettingsServiceClient<Wire> {
        SettingsServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn browser_bridge(&self) -> BrowserBridgeServiceClient<Wire> {
        BrowserBridgeServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn boards(&self) -> BoardServiceClient<Wire> {
        BoardServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn sessions(&self) -> SessionServiceClient<Wire> {
        SessionServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn terminal(&self) -> TerminalServiceClient<Wire> {
        TerminalServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn event_stream(&self) -> EventServiceClient<Wire> {
        EventServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn operations(&self) -> OperationServiceClient<Wire> {
        OperationServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    /// A gRPC failure as the app's error, for a call about `what`.
    pub(crate) fn error(&self, status: &Status, what: &str) -> DaemonError {
        self.conn.error(status, what)
    }

    /// Waits for an operation to stop running; a failed or cancelled one is
    /// refused with the daemon's words.
    pub(crate) async fn finish(
        &self,
        operation: pb::Operation,
        what: &str,
    ) -> Result<pb::Operation, DaemonError> {
        self.conn.finish_operation(operation, what).await
    }
}

impl DeskClient for LiveDaemon {
    fn start(&self, new: NewDeskSession) -> Reply<SessionId> {
        self.desk.start(new)
    }

    fn close(&self, session: &SessionId) -> Reply<()> {
        self.desk.close(session)
    }

    fn reopen(&self, session: &SessionId) -> Reply<()> {
        self.desk.reopen(session)
    }

    fn keep(&self, session: &SessionId) -> Reply<()> {
        self.desk.keep(session)
    }

    fn clean_up_list(&self) -> Reply<Vec<CleanUpRow>> {
        self.desk.clean_up_list()
    }

    fn clean_up(&self, shown: Vec<CleanUpRow>, delete: Vec<SessionId>) -> Reply<CleanUp> {
        self.desk.clean_up(shown, delete)
    }

    fn copy_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Reply<()> {
        self.desk.copy_document(document, channel, machine)
    }

    fn add_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        self.desk.add_repo(session, repo)
    }

    fn remove_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        self.desk.remove_repo(session, repo)
    }

    fn move_to(&self, session: &SessionId, machine: &MachineId) -> Reply<()> {
        self.desk.move_to(session, machine)
    }

    fn list_versions(
        &self,
        session: &SessionId,
        path: &str,
    ) -> Reply<humata2_clients::docs::DocVersions> {
        self.desk.list_versions(session, path)
    }

    fn read_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<humata2_clients::docs::VersionText> {
        self.desk.read_version(session, path, id)
    }

    fn save_version(
        &self,
        session: &SessionId,
        message: &str,
    ) -> Reply<humata2_clients::docs::DocVersion> {
        self.desk.save_version(session, message)
    }

    fn restore_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<humata2_clients::docs::Restored> {
        self.desk.restore_version(session, path, id)
    }

    fn diff_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<humata2_clients::docs::VersionDiff> {
        self.desk.diff_version(session, path, id)
    }

    fn set_access(&self, session: &SessionId, access: Access) -> Reply<()> {
        self.desk.set_access(session, access)
    }

    fn channel_sessions(&self, channel: &ChannelId) -> Reply<Vec<ChannelDeskSession>> {
        self.desk.channel_sessions(channel)
    }

    fn target_sessions(&self, target: &SessionId) -> Reply<Vec<ChannelDeskSession>> {
        self.desk.target_sessions(target)
    }

    fn start_channel(&self, new: NewChannelDesk) -> Reply<SessionId> {
        self.desk.start_channel(new)
    }

    fn set_switches(&self, session: &SessionId, switches: Switches) -> Reply<()> {
        self.desk.set_switches(session, switches)
    }

    fn set_target(&self, session: &SessionId, target: Option<&SessionId>) -> Reply<()> {
        self.desk.set_target(session, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cue_during_a_read_that_found_nothing_reads_again() {
        let mut state = State::default();
        assert!(state.restart_cue());
        // An event arrives while that read is in flight; it may have begun before the daemon had the report.
        assert!(!state.restart_cue());
        assert!(
            state.restart_read_ended(false),
            "the cue that came during the read isn't lost"
        );
        assert!(
            !state.restart_read_ended(false),
            "with no new cue, it waits for the next one"
        );
        assert!(state.restart_cue());
    }

    #[test]
    fn a_known_report_is_never_read_again() {
        let mut state = State::default();
        assert!(state.restart_cue());
        assert!(!state.restart_cue());
        assert!(!state.restart_read_ended(true));
        assert!(!state.restart_cue());
    }
}

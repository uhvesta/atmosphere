//! Adapter wiring `humata2_svc_inbox` (`Inbox` + `screen_ask`) into `humatad2`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use humata2_access::{
    AnswerError, Call, Capability, Context, Grant, GrantTarget, Policy, PresentUser, Principal,
    Refusal,
};
use humata2_daemon_ports::{AgentCaller, Caller};
use humata2_daemon_runtime::agent_switches::AgentSwitches;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Agents;
use humata2_store::{SessionRecord, Store, StoreError};
use humata2_svc_away::HostLink;
use humata2_svc_inbox::{
    AgentGatePort, AskingGuardPort, EventScope, InboxFeedPort, InboxGuardPort,
    MigrateApproverPort, ScreenHoldersPort,
};
use tonic::Status;

pub(crate) use humata2_svc_inbox::inbox::Replayed;
pub(crate) use humata2_svc_inbox::{
    BoardAsk, Deferred, Inbox, MoveSwitchError, Registered, Relays, Run, ScreenAsk, permission,
};

use crate::events::{Feed, Scope};
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::holders::Holders;
use crate::status;
use crate::terminal::Writers;

pub(crate) struct FeedAdapter(pub(crate) Arc<Feed>);

impl InboxFeedPort for FeedAdapter {
    fn publish(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        actor: Option<pb::Actor>,
        scope: EventScope,
    ) {
        self.0.publish_in(
            change,
            payload,
            Scope {
                channel: scope.channel.unwrap_or_default(),
                session: scope.session.unwrap_or_default(),
            },
            actor,
        );
    }
}

pub(crate) struct GuardAdapter(pub(crate) Arc<Guard>);

impl InboxGuardPort for GuardAdapter {
    fn host_link(&self) -> Arc<HostLink> {
        self.0.host_link()
    }

    fn with_policy_mut(&self, f: &mut dyn FnMut(&mut Policy, &Context)) {
        self.0.with_policy(|policy, context| f(policy, context));
    }

    fn principal(&self, caller: &Caller) -> Principal {
        guard::principal(caller)
    }

    fn present_user(&self, caller: &Caller) -> Result<PresentUser, Status> {
        let principal = guard::principal(caller);
        self.0
            .with_policy(|_, context| principal.present_user(context))
            .ok_or_else(|| {
                status::refused(&Refusal::UserOnly {
                    capability: Some(Capability::InboxAnswer),
                })
            })
    }

    fn capability_pb(&self, capability: Capability) -> pb::Capability {
        guard::capability(capability)
    }

    fn capability_of(&self, capability: i32) -> Option<Capability> {
        grant_target::capability_of(capability)
    }

    fn grant_number(&self, grant: &pb::Grant) -> Option<u64> {
        grant_target::grant_number(grant)
    }

    fn grant_pb(&self, grant: &Grant, suspended: bool) -> pb::Grant {
        grant_target::grant(grant, suspended)
    }

    fn target_pb(&self, target: &GrantTarget) -> pb::GrantTarget {
        grant_target::target(target)
    }

    fn ask_ahead_call(
        &self,
        store: &Store,
        machine: &str,
        agent: &AgentCaller,
        capability: Capability,
        target: pb::GrantTarget,
    ) -> Result<Call, Status> {
        grant_target::ask_ahead_call(store, machine, agent, capability, target)
    }

    fn answer_failed(&self, error: &AnswerError) -> Status {
        grant_target::answer_failed(error)
    }

    fn refused_status(&self, refusal: &Refusal) -> Status {
        status::refused(refusal)
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.0.require(caller, call)
    }

    fn check_inbox_ask(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.0.check_inbox_ask(caller, call).map(|_| ())
    }

    fn hold_bound(&self, caller: &Caller, metadata: &tonic::metadata::MetadataMap) -> Duration {
        guard::hold_bound(caller, metadata).unwrap_or_default()
    }

    fn waits_for_host(&self) -> Status {
        guard::waits_for_host()
    }

    fn agent_caller(
        &self,
        record: &SessionRecord,
        machine: &str,
        store: &Store,
    ) -> Result<AgentCaller, StoreError> {
        crate::identity::resolve_fork(crate::identity::agent_caller_of(record, machine), store)
    }
}

pub(crate) struct MigrateApproverAdapter(pub(crate) crate::upgrade::items::Approver);

#[tonic::async_trait]
impl MigrateApproverPort for MigrateApproverAdapter {
    async fn approve(&self, id: &str) -> Result<pb::InboxRequest, Status> {
        self.0.approve(id).await
    }

    fn is_migrate(&self, request: &pb::InboxRequest) -> bool {
        crate::upgrade::items::is_migrate(request)
    }

    fn refuse_answer(
        &self,
        request: &pb::InboxRequest,
        answer: &pb::request_answer::Answer,
    ) -> Option<Status> {
        crate::upgrade::items::refuse_answer(request, answer)
    }

    fn finish_steps(&self, request: &mut pb::InboxRequest) {
        crate::upgrade::items::finish_steps(request);
    }
}

pub(crate) struct AskingAdapter;

impl AskingGuardPort for AskingAdapter {
    fn hold(&self, session: &str) -> Box<dyn Send + Sync> {
        Box::new(crate::session_service::Asking::hold(session))
    }
}

pub(crate) struct AgentGateAdapter;

impl AgentGatePort for AgentGateAdapter {
    fn switches(&self, agents: &Agents) -> Option<AgentSwitches> {
        crate::agent_gate::switches(agents)
    }
}

pub(crate) struct ScreenHoldersAdapter {
    pub(crate) holders: Arc<Holders>,
    pub(crate) writers: Arc<Writers>,
    pub(crate) store: Arc<Store>,
}

#[tonic::async_trait]
impl ScreenHoldersPort for ScreenHoldersAdapter {
    async fn screen_and_size_at(&self, socket: &Path) -> Result<(String, (u16, u16)), Status> {
        crate::holders::screen_and_size_at(socket)
            .await
            .map_err(|error| status::internal(error.to_string(), ""))
    }

    async fn grow_rows(&self, tab: &str, cols: u16, rows: u16) -> Result<(), Status> {
        let store = Arc::clone(&self.store);
        let id = tab.to_owned();
        let record = tokio::task::spawn_blocking(move || store.tab(&id))
            .await
            .map_err(|error| status::internal(error.to_string(), ""))?
            .map_err(|error| status::internal(error.to_string(), ""))?;
        let Some(record) = record else {
            return Ok(());
        };
        crate::attach::grow_rows(&record, &self.holders, cols, rows).await
    }

    async fn send_keys(&self, tab: &str, socket: &Path, keys: &[u8]) -> Result<(), Status> {
        let link = self
            .holders
            .link(tab, socket)
            .await
            .map_err(|error| status::internal(error.to_string(), ""))?;
        link.type_text(keys)
            .await
            .map_err(|error| status::internal(error.to_string(), ""))
    }

    fn writer_count(&self, tab: &str) -> u32 {
        self.writers.count(tab)
    }
}

/// Constructs [`Inbox`] wired to `humatad2`'s `Guard`, `Feed`, `Asking`, and `agent_gate`.
pub(crate) fn new_inbox(
    guard: Arc<Guard>,
    store: Arc<Store>,
    machine: String,
    feed: Arc<Feed>,
) -> Arc<Inbox> {
    let inbox = Arc::new(
        Inbox::new(
            Arc::new(GuardAdapter(guard)),
            store,
            machine,
            Arc::new(FeedAdapter(feed)),
        )
        .with_asking(Arc::new(AskingAdapter)),
    );
    inbox.use_gate(Arc::new(AgentGateAdapter));
    inbox
}

/// Wraps `upgrade::items::Approver` as a [`MigrateApproverPort`] for `inbox.migrating(...)`.
pub(crate) fn wrap_migrator(
    approver: crate::upgrade::items::Approver,
) -> Arc<dyn MigrateApproverPort> {
    Arc::new(MigrateApproverAdapter(approver))
}

/// Starts `humata2_svc_inbox::screen_ask` wired to `humatad2`'s `Holders`, `Writers`,
/// `Asking`, and `start_ready`.
pub(crate) fn start_screen_ask(
    holders: Arc<Holders>,
    writers: Arc<Writers>,
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
) {
    humata2_svc_inbox::screen_ask::start(
        Arc::new(ScreenHoldersAdapter {
            holders,
            writers,
            store: store.clone(),
        }),
        Arc::new(AskingAdapter),
        inbox,
        store,
        machine,
        Arc::new(crate::start_ready::is_heard),
        Arc::new(crate::start_ready::forget),
    );
}

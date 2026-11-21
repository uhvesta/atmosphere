//! ATM-54 SESSIONBOOT: live daemon launch wiring for deterministic TUI startup
//! authorization and prompt handling.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata2_holder::Event;
use humata2_store::Store;
use humata2_svc_inbox::startup_auth::{
    ApprovalReviewSummary, KeyInput, OwnerConsentScope, StartupAuthError, StartupAuthReport,
    StartupAuthRunner, StartupConfig, StartupStatus, StartupTerminalPort, seed_for_session,
};

use crate::identity::Caller;
use crate::inbox::Inbox;
use crate::inbox::permission::Question;
use crate::session_service::Asking;
use crate::status;

/// Returns `true` when `agent` is a supported TUI agent kind for startup boot
/// automation.
pub(crate) fn is_supported_tui_agent(agent: &str) -> bool {
    matches!(agent, "agy" | "claude" | "codex" | "copilot")
}

/// Terminal port backed by a live holder Unix socket.
pub(crate) struct HolderStartupTerminalPort {
    socket: PathBuf,
    last_screen: Mutex<Option<(u64, String)>>,
}

impl HolderStartupTerminalPort {
    pub(crate) fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            last_screen: Mutex::new(None),
        }
    }
}

#[tonic::async_trait]
impl StartupTerminalPort for HolderStartupTerminalPort {
    async fn capture_screen(&self) -> Result<Option<(u64, String)>, StartupAuthError> {
        let last_end = {
            let guard = self
                .last_screen
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            guard.as_ref().map(|(end, _)| *end)
        };
        let captured = crate::holders::screen_at_if_changed(&self.socket, last_end)
            .await
            .map_err(|err| StartupAuthError::ScreenRead {
                detail: status::chain(&err),
            })?;
        match captured {
            Some((end, screen)) => {
                let mut guard = self
                    .last_screen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                *guard = Some((end, screen.clone()));
                Ok(Some((end, screen)))
            }
            None => {
                let guard = self
                    .last_screen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                Ok(guard.clone())
            }
        }
    }

    async fn send_keys(&self, keys: &[KeyInput]) -> Result<(), StartupAuthError> {
        const HOLDER_IO_TIMEOUT: Duration = Duration::from_secs(5);
        if keys.is_empty() {
            return Ok(());
        }
        let mut attachment = tokio::time::timeout(
            HOLDER_IO_TIMEOUT,
            humata2_holder::attach(&self.socket, u64::MAX),
        )
        .await
        .map_err(|_| StartupAuthError::KeySend {
            detail: "timed out attaching to holder socket".to_owned(),
        })?
        .map_err(|err| StartupAuthError::KeySend {
            detail: status::chain(&err),
        })?;
        for (idx, key) in keys.iter().enumerate() {
            let bytes = key.to_bytes();
            if bytes.is_empty() {
                continue;
            }
            let seq = (idx as u64) + 1;
            attachment
                .control
                .input(seq, &bytes)
                .await
                .map_err(|err| StartupAuthError::KeySend {
                    detail: status::chain(&err),
                })?;
            loop {
                match tokio::time::timeout(HOLDER_IO_TIMEOUT, attachment.events.next()).await {
                    Ok(Ok(Some(Event::InputWritten { seq: written }))) if written == seq => break,
                    Ok(Ok(Some(Event::Refused { message }))) => {
                        return Err(StartupAuthError::KeySend { detail: message });
                    }
                    Ok(Ok(Some(Event::Exited { code }))) => {
                        return Err(StartupAuthError::KeySend {
                            detail: format!("holder exited with code {code} before key write"),
                        });
                    }
                    Ok(Ok(Some(_))) => {}
                    Ok(Ok(None)) => {
                        return Err(StartupAuthError::KeySend {
                            detail: "holder closed connection before confirming key write"
                                .to_owned(),
                        });
                    }
                    Ok(Err(err)) => {
                        return Err(StartupAuthError::KeySend {
                            detail: status::chain(&err),
                        });
                    }
                    Err(_) => {
                        return Err(StartupAuthError::KeySend {
                            detail: "timed out waiting for holder key write confirmation"
                                .to_owned(),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

/// Runs the bounded startup authorization loop, logs a structured audit summary,
/// and surfaces any `PausedForReview` boundary cleanly into `Inbox` without ever
/// auto-approving.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_startup_boot(
    terminal: &dyn StartupTerminalPort,
    config: StartupConfig,
    scope: OwnerConsentScope,
    store: Arc<Store>,
    inbox: Arc<Inbox>,
    machine: &str,
    session_id: &str,
    agent_kind: &str,
) -> Result<StartupAuthReport, StartupAuthError> {
    let runner = StartupAuthRunner::new(session_id, agent_kind, config)?;
    let report = runner.run(terminal, &scope).await;

    crate::log::say!(
        "humatad2: startup_boot session={} agent={} seed={} attempts={} keys_sent={} status={:?}",
        report.audit.session_id,
        report.audit.agent_kind,
        report.audit.seed,
        report.audit.entries.len(),
        report.total_keys_sent,
        match &report.status {
            StartupStatus::Ready => "Ready",
            StartupStatus::PausedForReview(_) => "PausedForReview",
        }
    );

    if let StartupStatus::PausedForReview(summary) = &report.status {
        crate::log::say!(
            "humatad2: startup_boot paused_for_review session={} agent={} reason={:?} action={:?} target={:?} scope={:?} reversibility={:?} changed_risk_factors={:?}",
            session_id,
            agent_kind,
            summary.reason,
            summary.action,
            summary.target,
            summary.scope,
            summary.reversibility,
            summary.changed_risk_factors,
        );
        raise_pause_review(&store, &inbox, machine, session_id, agent_kind, summary).await;
    }

    Ok(report)
}

async fn raise_pause_review(
    store: &Arc<Store>,
    inbox: &Arc<Inbox>,
    machine: &str,
    session_id: &str,
    agent_kind: &str,
    summary: &ApprovalReviewSummary,
) {
    let store = store.clone();
    let id = session_id.to_owned();
    let found = tokio::task::spawn_blocking(move || store.session(&id)).await;
    let Ok(Ok(Some(record))) = found else {
        crate::log::say!(
            "humatad2: startup_boot session={session_id} paused for review, but session record couldn't be loaded to raise an Inbox request"
        );
        return;
    };
    let agent = crate::identity::agent_caller_of(&record, machine);
    let caller = Caller::Agent(agent.clone());
    let card = Question {
        question: pause_review_question(session_id, agent_kind, summary),
        choices: vec!["Reviewed in terminal".to_owned(), "Dismiss".to_owned()],
        key: format!("startup-boot:{session_id}"),
        only_choices: false,
    };
    match inbox.raise_question(&caller, &agent, card).await {
        Ok(mut wait) => {
            let asking = Asking::hold(session_id);
            tokio::spawn(async move {
                let _asking = asking;
                let _ = wait.decided().await;
            });
        }
        Err(error) => {
            crate::log::say!(
                "humatad2: startup_boot couldn't raise Inbox review request for session {session_id}: {}",
                status::chain(&error)
            );
        }
    }
}

fn pause_review_question(
    session_id: &str,
    agent_kind: &str,
    summary: &ApprovalReviewSummary,
) -> String {
    let risk_factors = if summary.changed_risk_factors.is_empty() {
        "none".to_owned()
    } else {
        summary.changed_risk_factors.join(", ")
    };
    format!(
        "Startup authorization for session {session_id} ({agent_kind}) paused for review.\n\
         Reason: {:?}\n\
         Action: {}\n\
         Target: {}\n\
         Scope: {}\n\
         Duration: {}\n\
         Reversibility: {:?}\n\
         Risk factors: {}\n\n\
         Open the session terminal to review and respond directly.",
        summary.reason,
        summary.action,
        summary.target,
        summary.scope,
        summary.duration,
        summary.reversibility,
        risk_factors,
    )
}

/// Spawns a bounded, non-blocking startup boot task for supported TUI agents.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_for_session(
    store: Arc<Store>,
    inbox: Arc<Inbox>,
    machine: String,
    session_id: String,
    agent_kind: String,
    session_folder: PathBuf,
    holder_socket: PathBuf,
    holder_pid: u32,
) {
    if !is_supported_tui_agent(&agent_kind) {
        return;
    }
    let config = StartupConfig {
        seed: seed_for_session(&session_id, holder_pid),
        ..StartupConfig::default()
    };
    let scope = OwnerConsentScope::local_workspace(session_folder);
    let terminal = HolderStartupTerminalPort::new(holder_socket);
    tokio::spawn(async move {
        if let Err(error) = run_startup_boot(
            &terminal,
            config,
            scope,
            store,
            inbox,
            &machine,
            &session_id,
            &agent_kind,
        )
        .await
        {
            crate::log::say!(
                "humatad2: startup_boot failed for session {session_id} ({agent_kind}): {error}"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use humata2_grpc::humata::v1 as pb;
    use humata2_grpc::humata::v1::inbox_service_server::InboxService;
    use humata2_store::{SessionRecord, Store};
    use humata2_svc_inbox::startup_auth::{
        KeyInput, OwnerConsentScope, PauseReason, StartupAuthError, StartupConfig, StartupStatus,
        StartupTerminalPort,
    };

    use super::{is_supported_tui_agent, run_startup_boot};
    use crate::events::Feed;
    use crate::guard::Guard;
    use crate::identity::{Caller, UserVia};
    use crate::inbox::{Inbox, new_inbox};

    struct MockTerminal {
        screens: Mutex<VecDeque<Option<(u64, String)>>>,
        sent_keys: Mutex<Vec<Vec<KeyInput>>>,
    }

    impl MockTerminal {
        fn new(screens: Vec<Option<(u64, String)>>) -> Self {
            Self {
                screens: Mutex::new(screens.into_iter().collect()),
                sent_keys: Mutex::new(Vec::new()),
            }
        }

        fn sent(&self) -> Vec<Vec<KeyInput>> {
            self.sent_keys.lock().unwrap().clone()
        }
    }

    #[tonic::async_trait]
    impl StartupTerminalPort for MockTerminal {
        async fn capture_screen(&self) -> Result<Option<(u64, String)>, StartupAuthError> {
            let mut guard = self.screens.lock().unwrap();
            Ok(guard.pop_front().flatten())
        }

        async fn send_keys(&self, keys: &[KeyInput]) -> Result<(), StartupAuthError> {
            self.sent_keys.lock().unwrap().push(keys.to_vec());
            Ok(())
        }
    }

    fn zero_delay_config() -> StartupConfig {
        StartupConfig {
            initial_wait: Duration::ZERO,
            base_retry_delay: Duration::ZERO,
            max_retry_delay: Duration::ZERO,
            max_total_wait: Duration::from_secs(5),
            max_attempts: 5,
            jitter_ratio_bps: 0,
            seed: 42,
        }
    }

    fn user<T>(message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request.extensions_mut().insert(Caller::User(UserVia::App));
        request
    }

    async fn list_requests(inbox: &Inbox) -> Vec<pb::InboxRequest> {
        inbox
            .list_requests(user(pb::ListRequestsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .requests
    }

    fn test_session(id: &str, machine: &str, agent: &str, folder: PathBuf) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "ch_test".to_owned(),
            machine: machine.to_owned(),
            title: "Startup boot test".to_owned(),
            agent: agent.to_owned(),
            folder,
            created_unix_ms: 0,
        }
    }

    #[test]
    fn supported_tui_agent_filter() {
        for agent in ["agy", "claude", "codex", "copilot"] {
            assert!(
                is_supported_tui_agent(agent),
                "{agent} should be supported"
            );
        }
        assert!(!is_supported_tui_agent("zsh"));
        assert!(!is_supported_tui_agent("bash"));
        assert!(!is_supported_tui_agent(""));
    }

    #[tokio::test]
    async fn ready_path_approves_matching_folder_trust_and_raises_no_inbox_requests() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let workspace = PathBuf::from("/workspace/project-alpha");
        store
            .put_session(&test_session("se_ready", "ma_host", "copilot", workspace.clone()))
            .unwrap();
        let guard = Arc::new(Guard::new("ma_host", pb::DaemonRole::Host));
        let inbox = new_inbox(guard, store.clone(), "ma_host".to_owned(), Arc::new(Feed::default()));

        let trust_screen = "\
Do you trust the files in this folder?
/workspace/project-alpha
> 1. Yes, proceed
  2. Yes, and remember this folder
  3. No, exit";
        let ready_screen = "Copilot CLI ready.\n> ";
        let terminal = MockTerminal::new(vec![
            Some((100, trust_screen.to_owned())),
            Some((200, ready_screen.to_owned())),
        ]);

        let report = run_startup_boot(
            &terminal,
            zero_delay_config(),
            OwnerConsentScope::local_workspace(workspace),
            store,
            inbox.clone(),
            "ma_host",
            "se_ready",
            "copilot",
        )
        .await
        .unwrap();

        assert_eq!(report.status, StartupStatus::Ready);
        assert!(report.total_keys_sent > 0);
        assert!(!terminal.sent().is_empty());
        assert!(list_requests(&inbox).await.is_empty());
    }

    #[tokio::test]
    async fn paused_for_review_on_destructive_prompt_sends_no_keys_and_raises_inbox_request() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let workspace = PathBuf::from("/workspace/project-beta");
        store
            .put_session(&test_session("se_pause", "ma_host", "claude", workspace.clone()))
            .unwrap();
        let guard = Arc::new(Guard::new("ma_host", pb::DaemonRole::Host));
        let inbox = new_inbox(guard, store.clone(), "ma_host".to_owned(), Arc::new(Feed::default()));

        let destructive_screen = "\
Confirm destructive command: rm -rf /workspace/project-beta/build? [y/N]";
        let terminal = MockTerminal::new(vec![Some((100, destructive_screen.to_owned()))]);

        let report = run_startup_boot(
            &terminal,
            zero_delay_config(),
            OwnerConsentScope::local_workspace(workspace),
            store,
            inbox.clone(),
            "ma_host",
            "se_pause",
            "claude",
        )
        .await
        .unwrap();

        let StartupStatus::PausedForReview(summary) = report.status else {
            panic!("expected PausedForReview, got {:?}", report.status);
        };
        assert_eq!(summary.reason, PauseReason::DestructiveOrExternalSignal);
        assert_eq!(report.total_keys_sent, 0);
        assert!(terminal.sent().is_empty());

        let requests = list_requests(&inbox).await;
        assert_eq!(requests.len(), 1);
        let question_text = match requests[0]
            .preview
            .as_ref()
            .and_then(|p| p.preview.as_ref())
        {
            Some(pb::request_preview::Preview::Question(q)) => q.question.as_str(),
            other => panic!("expected Question preview, got {other:?}"),
        };
        assert!(question_text.contains(&summary.action));
        assert!(question_text.contains("DestructiveOrExternalSignal"));
        assert!(question_text.contains("rm -rf"));
    }
}

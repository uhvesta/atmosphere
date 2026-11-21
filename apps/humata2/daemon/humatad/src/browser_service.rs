//! `BrowserBridgeService` (BROWSER, A3): connects the native messaging host
//! (`humata2-browser-host`), dispatches approved browser actions with 30 s
//! `request_id` oneshot correlation, and handles user-only Settings › Browser
//! host discovery / install / uninstall.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata2_access::Refusal;
use humata2_browser_copilot::{
    ActionClass, BrowserAction, BrowserDetector, BrowserFamily, BrowserKind, BrowserReply,
    BrowserRequest, ManifestConfig, ManifestInstaller, NATIVE_HOST_NAME, Platform, SupportedAgent,
};
use humata2_grpc::humata::v1 as pb;
use prost::Message;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::away::{Forwarder, Method};
use crate::identity::{self, Caller, UserVia};
use crate::inbox::Inbox;
use crate::status;

/// Maximum time `ExecuteBrowserAction` waits for the browser extension to reply.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Default Chromium extension origin used when no explicit override is set.
/// Matches the unpacked Chromium extension's deterministic key/id or can be
/// overridden via `HUMATA2_BROWSER_CHROMIUM_ORIGINS` (comma-separated).
const DEFAULT_CHROMIUM_ORIGIN: &str = "chrome-extension://humata2browsercopilothost0000000/";

/// Filesystem layout used for browser detection and native messaging host
/// installation. Injectable for tests via `HUMATA2_BROWSER_HOME` so tests
/// never touch real browser profile folders or `~/.humata2/bin`.
#[derive(Clone, Debug)]
pub(crate) struct BrowserHostLayout {
    pub(crate) platform: Platform,
    pub(crate) fs_root: PathBuf,
    pub(crate) home_dir: PathBuf,
    pub(crate) bin_dir: PathBuf,
    pub(crate) own_exe: PathBuf,
    pub(crate) extension_root: PathBuf,
    pub(crate) chromium_origins: Vec<String>,
}

impl BrowserHostLayout {
    pub(crate) fn from_env(state: &Path, own_exe: &Path) -> Self {
        let env_home = std::env::var_os("HUMATA2_BROWSER_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let (fs_root, home_dir, bin_dir) = match env_home {
            Some(fake_home) => {
                let fs_root = std::env::var_os("HUMATA2_BROWSER_FS_ROOT")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| fake_home.clone());
                let bin_dir = fake_home.join(".humata2/bin");
                (fs_root, fake_home, bin_dir)
            }
            None => {
                let home_dir = std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| state.to_path_buf());
                let bin_dir = home_dir.join(".humata2/bin");
                (PathBuf::from("/"), home_dir, bin_dir)
            }
        };
        let extension_root = own_exe
            .parent()
            .map(|p| p.join("browser_extension"))
            .unwrap_or_else(|| PathBuf::from("apps/humata2/browser_extension"));
        let chromium_origins = std::env::var("HUMATA2_BROWSER_CHROMIUM_ORIGINS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![DEFAULT_CHROMIUM_ORIGIN.to_owned()]);
        Self {
            platform: Platform::current(),
            fs_root,
            home_dir,
            bin_dir,
            own_exe: own_exe.to_path_buf(),
            extension_root,
            chromium_origins,
        }
    }

    pub(crate) fn host_link_path(&self) -> PathBuf {
        self.bin_dir.join("humata2-browser-host")
    }

    fn manifest_config(&self) -> ManifestConfig {
        ManifestConfig::new(self.host_link_path(), self.chromium_origins.clone())
    }
}

struct ConnectedHost {
    id: u64,
    extension_origin: String,
    host_pid: u32,
    family: BrowserFamily,
    tx: mpsc::Sender<Result<pb::BrowserDaemonFrame, Status>>,
}

struct PendingAction {
    host_id: u64,
    tx: oneshot::Sender<Result<String, Status>>,
}

#[derive(Default)]
struct BridgeState {
    hosts: Vec<ConnectedHost>,
    pending: HashMap<String, PendingAction>,
}

/// Shared state for `BrowserBridgeService` on the host Mac.
#[derive(Clone)]
pub struct BrowserBridgeSvc {
    inbox: Arc<Inbox>,
    layout: BrowserHostLayout,
    state: Arc<Mutex<BridgeState>>,
    next_host_id: Arc<AtomicU64>,
    timeout: Duration,
}

impl BrowserBridgeSvc {
    pub(crate) fn new(inbox: Arc<Inbox>, layout: BrowserHostLayout) -> Self {
        Self {
            inbox,
            layout,
            state: Arc::new(Mutex::new(BridgeState::default())),
            next_host_id: Arc::new(AtomicU64::new(1)),
            timeout: ACTION_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BridgeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn remove_host(&self, host_id: u64) {
        let mut state = self.lock();
        state.hosts.retain(|h| h.id != host_id);
        let dropped: Vec<String> = state
            .pending
            .iter()
            .filter(|(_, p)| p.host_id == host_id)
            .map(|(k, _)| k.clone())
            .collect();
        for key in dropped {
            if let Some(pending) = state.pending.remove(&key) {
                let _ = pending.tx.send(Err(Status::unavailable(
                    "the browser native host disconnected before replying; open Settings › Browser to reconnect",
                )));
            }
        }
    }

    fn deliver_reply(&self, raw_json: String) {
        let request_id = serde_json::from_str::<BrowserReply>(&raw_json)
            .ok()
            .map(|r| r.request_id)
            .or_else(|| {
                serde_json::from_str::<serde_json::Value>(&raw_json)
                    .ok()
                    .and_then(|v| v.get("request_id")?.as_str().map(str::to_owned))
            });
        let Some(request_id) = request_id else {
            return;
        };
        let pending = self.lock().pending.remove(&request_id);
        if let Some(pending) = pending {
            let _ = pending.tx.send(Ok(raw_json));
        }
    }

    fn list_browsers(&self) -> pb::ListDetectedBrowsersResponse {
        let detector = BrowserDetector::new(
            self.layout.platform,
            &self.layout.fs_root,
            &self.layout.home_dir,
        );
        let detected = detector.detect_all();
        let expected_link = self.layout.host_link_path();
        let link_matches_exe = symlink_points_to(&expected_link, &self.layout.own_exe);
        let connected_families: Vec<BrowserFamily> = self
            .lock()
            .hosts
            .iter()
            .map(|h| h.family)
            .collect();

        let browsers = detected
            .into_iter()
            .map(|b| {
                let manifest_path =
                    ManifestInstaller::manifest_path(&b.native_messaging_dir, NATIVE_HOST_NAME);
                let host_installed = manifest_path.is_file();
                let host_matches_current = host_installed
                    && link_matches_exe
                    && manifest_points_to(&manifest_path, &expected_link);
                let extension_subdir = match b.family {
                    BrowserFamily::Chromium => "chromium",
                    BrowserFamily::Firefox => "firefox",
                };
                pb::DetectedBrowser {
                    kind: pb_browser_kind(b.kind) as i32,
                    family: pb_browser_family(b.family) as i32,
                    display_name: b.kind.display_name().to_owned(),
                    binary_path: b.binary_path.to_string_lossy().into_owned(),
                    native_messaging_dir: b.native_messaging_dir.to_string_lossy().into_owned(),
                    manifest_path: manifest_path.to_string_lossy().into_owned(),
                    host_installed,
                    host_matches_current,
                    connected: connected_families.contains(&b.family),
                    extension_path: self
                        .layout
                        .extension_root
                        .join(extension_subdir)
                        .to_string_lossy()
                        .into_owned(),
                }
            })
            .collect();

        pb::ListDetectedBrowsersResponse { browsers }
    }
}

#[tonic::async_trait]
impl pb::browser_bridge_service_server::BrowserBridgeService for BrowserBridgeSvc {
    type ConnectBrowserHostStream = ReceiverStream<Result<pb::BrowserDaemonFrame, Status>>;

    async fn connect_browser_host(
        &self,
        request: Request<Streaming<pb::BrowserHostFrame>>,
    ) -> Result<Response<Self::ConnectBrowserHostStream>, Status> {
        if !identity::is_browser_host(&request) {
            return Err(Status::permission_denied(
                "only the local humata2-browser-host carrying the browser host token outside every session may connect here",
            ));
        }
        let mut inbound = request.into_inner();
        let first = inbound
            .next()
            .await
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("browser host stream closed before hello"))?;
        let hello = match first.frame {
            Some(pb::browser_host_frame::Frame::Hello(hello)) => hello,
            _ => {
                return Err(Status::invalid_argument(
                    "first frame on ConnectBrowserHost must be BrowserHostHello",
                ));
            }
        };

        let family = infer_family(&hello.extension_origin);
        let host_id = self.next_host_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(32);
        {
            let mut state = self.lock();
            state.hosts.push(ConnectedHost {
                id: host_id,
                extension_origin: hello.extension_origin,
                host_pid: hello.host_pid,
                family,
                tx,
            });
        }

        let bridge = self.clone();
        tokio::spawn(async move {
            while let Some(frame) = inbound.next().await {
                match frame {
                    Ok(pb::BrowserHostFrame {
                        frame: Some(pb::browser_host_frame::Frame::ReplyJson(raw)),
                    }) => bridge.deliver_reply(raw),
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            bridge.remove_host(host_id);
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn execute_browser_action(
        &self,
        request: Request<pb::ExecuteBrowserActionRequest>,
    ) -> Result<Response<pb::ExecuteBrowserActionResponse>, Status> {
        let caller = identity::caller(&request)?;
        let req = request.into_inner();
        let action = BrowserAction::from_json(&req.action_json).map_err(|error| {
            status::invalid_field(
                "action_json",
                pb::invalid_field::Problem::BadFormat,
                format!("browser action JSON isn't valid: {error}"),
                "send a supported browser action JSON object",
            )
        })?;
        let is_write = ActionClass::classify(&action) == ActionClass::Write;
        let action_key = action.action_key();

        match &caller {
            Caller::Agent(agent) => {
                if SupportedAgent::parse(&agent.agent).is_err() {
                    return Err(Status::permission_denied(format!(
                        "agent `{}` is not permitted to use the browser bridge (only claude, codex, copilot and agy are supported)",
                        agent.agent
                    )));
                }
                let approval_id = req
                    .approval
                    .as_ref()
                    .map(|id| id.value.as_str())
                    .filter(|s| !s.is_empty());
                self.inbox
                    .consume_browser_approval(
                        agent.session.id.as_str(),
                        approval_id,
                        &action_key,
                        is_write,
                    )
                    .await?;
            }
            Caller::User(_) => {}
            Caller::Script(_) | Caller::Peer => {
                return Err(Status::permission_denied(
                    "only supported session agents (with Inbox approval) or the user may execute browser actions",
                ));
            }
        }

        let request_id = format!("br_{}", uuid::Uuid::new_v4().simple());
        let wire_request = BrowserRequest {
            request_id: request_id.clone(),
            action,
        };
        let request_json = serde_json::to_string(&wire_request)
            .map_err(|e| status::internal(format!("couldn't encode browser request: {e}"), ""))?;

        let (reply_tx, reply_rx) = oneshot::channel();
        let host_tx = {
            let mut state = self.lock();
            let Some(host) = state.hosts.last() else {
                return Err(Status::unavailable(
                    "no browser is connected; open Settings › Browser in Humata to set up the native host and load the extension",
                ));
            };
            let host_id = host.id;
            let host_tx = host.tx.clone();
            state.pending.insert(
                request_id.clone(),
                PendingAction {
                    host_id,
                    tx: reply_tx,
                },
            );
            host_tx
        };

        if host_tx
            .send(Ok(pb::BrowserDaemonFrame { request_json }))
            .await
            .is_err()
        {
            self.lock().pending.remove(&request_id);
            return Err(Status::unavailable(
                "the connected browser host closed before accepting the request; open Settings › Browser to reconnect",
            ));
        }

        match tokio::time::timeout(self.timeout, reply_rx).await {
            Ok(Ok(Ok(reply_json))) => Ok(Response::new(pb::ExecuteBrowserActionResponse {
                reply_json,
            })),
            Ok(Ok(Err(status))) => Err(status),
            Ok(Err(_)) => Err(Status::unavailable(
                "the browser host disconnected before replying",
            )),
            Err(_) => {
                self.lock().pending.remove(&request_id);
                Err(Status::deadline_exceeded(format!(
                    "the browser didn't reply within {} seconds; check that the extension is loaded and responsive",
                    self.timeout.as_secs()
                )))
            }
        }
    }

    async fn get_browser_bridge_status(
        &self,
        request: Request<pb::GetBrowserBridgeStatusRequest>,
    ) -> Result<Response<pb::GetBrowserBridgeStatusResponse>, Status> {
        let caller = identity::caller(&request)?;
        match &caller {
            Caller::User(_) => {}
            Caller::Agent(agent) => {
                if SupportedAgent::parse(&agent.agent).is_err() {
                    return Err(Status::permission_denied(format!(
                        "agent `{}` is not permitted to query browser bridge status",
                        agent.agent
                    )));
                }
            }
            Caller::Script(_) | Caller::Peer => {
                return Err(Status::permission_denied(
                    "only the user or a supported session agent may check browser bridge status",
                ));
            }
        }

        let hosts: Vec<pb::ConnectedBrowserHost> = self
            .lock()
            .hosts
            .iter()
            .map(|h| pb::ConnectedBrowserHost {
                extension_origin: h.extension_origin.clone(),
                host_pid: h.host_pid,
                family: pb_browser_family(h.family) as i32,
            })
            .collect();

        Ok(Response::new(pb::GetBrowserBridgeStatusResponse {
            connected: !hosts.is_empty(),
            hosts,
        }))
    }

    async fn list_detected_browsers(
        &self,
        request: Request<pb::ListDetectedBrowsersRequest>,
    ) -> Result<Response<pb::ListDetectedBrowsersResponse>, Status> {
        let caller = identity::caller(&request)?;
        if !matches!(caller, Caller::User(_)) {
            return Err(user_only_error());
        }
        let svc = self.clone();
        let response = tokio::task::spawn_blocking(move || svc.list_browsers())
            .await
            .map_err(|e| status::internal(format!("browser detection task failed: {e}"), ""))?;
        Ok(Response::new(response))
    }

    async fn install_browser_host(
        &self,
        request: Request<pb::InstallBrowserHostRequest>,
    ) -> Result<Response<pb::ListDetectedBrowsersResponse>, Status> {
        let caller = identity::caller(&request)?;
        require_user_mutation(&caller)?;
        let req = request.into_inner();
        let wanted = parse_browser_kinds(&req.kinds)?;
        let svc = self.clone();
        let response = tokio::task::spawn_blocking(move || -> Result<_, Status> {
            install_symlink(&svc.layout.host_link_path(), &svc.layout.own_exe)?;
            let detector = BrowserDetector::new(
                svc.layout.platform,
                &svc.layout.fs_root,
                &svc.layout.home_dir,
            );
            let config = svc.layout.manifest_config();
            for browser in detector.detect_all() {
                if !wanted.is_empty() && !wanted.contains(&browser.kind) {
                    continue;
                }
                ManifestInstaller::install_for_browser(&browser, &config).map_err(|e| {
                    status::internal(
                        format!(
                            "couldn't install browser host manifest for {}: {e}",
                            browser.kind.display_name()
                        ),
                        "check that the browser's NativeMessagingHosts folder is writable",
                    )
                })?;
            }
            Ok(svc.list_browsers())
        })
        .await
        .map_err(|e| status::internal(format!("browser host install task failed: {e}"), ""))??;
        Ok(Response::new(response))
    }

    async fn uninstall_browser_host(
        &self,
        request: Request<pb::UninstallBrowserHostRequest>,
    ) -> Result<Response<pb::ListDetectedBrowsersResponse>, Status> {
        let caller = identity::caller(&request)?;
        require_user_mutation(&caller)?;
        let req = request.into_inner();
        let wanted = parse_browser_kinds(&req.kinds)?;
        let svc = self.clone();
        let response = tokio::task::spawn_blocking(move || -> Result<_, Status> {
            let detector = BrowserDetector::new(
                svc.layout.platform,
                &svc.layout.fs_root,
                &svc.layout.home_dir,
            );
            for browser in detector.detect_all() {
                if !wanted.is_empty() && !wanted.contains(&browser.kind) {
                    continue;
                }
                ManifestInstaller::uninstall_for_browser(&browser, NATIVE_HOST_NAME).map_err(
                    |e| {
                        status::internal(
                            format!(
                                "couldn't remove browser host manifest for {}: {e}",
                                browser.kind.display_name()
                            ),
                            "check that the browser's NativeMessagingHosts folder is writable",
                        )
                    },
                )?;
            }
            Ok(svc.list_browsers())
        })
        .await
        .map_err(|e| status::internal(format!("browser host uninstall task failed: {e}"), ""))??;
        Ok(Response::new(response))
    }
}

// ---------------------------------------------------------------------------
// Machine-role BrowserBridgeService (D891): forwards status & approved action
// execution to the host Mac over the peer Uplink; host-only management RPCs
// are refused with FailedPrecondition.
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum OnMachineError {
    #[error(
        "the host Mac is away, and browser access needs it; it answers again when the host is back"
    )]
    HostAway,

    #[error("{method} runs on the host Mac: this machine has no browser bridge of its own")]
    HostOnly { method: &'static str },

    #[error("the host's answer to {method} couldn't be read")]
    Reply {
        method: &'static str,
        #[source]
        source: prost::DecodeError,
    },
}

impl From<OnMachineError> for Status {
    fn from(error: OnMachineError) -> Self {
        let message = status::chain(&error);
        match error {
            OnMachineError::HostAway => status::failure(
                Code::Unavailable,
                status::error(
                    message,
                    "wait until the host Mac is awake and linked, then try again",
                    pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
                ),
            ),
            OnMachineError::HostOnly { .. } => status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message,
                    fix: "manage Settings › Browser and the browser native host on the host Mac"
                        .to_owned(),
                    retry: None,
                    kind: None,
                },
            ),
            OnMachineError::Reply { .. } => status::internal(
                format!(
                    "{message}; update Humata on this machine so both ends speak the same version"
                ),
                "",
            ),
        }
    }
}

/// `BrowserBridgeService` on a remote machine daemon (D891).
///
/// Agents on a remote machine get the same 4-agent scope, the same 30-minute
/// read grants and strict per-action write approvals (raised in this machine's
/// Inbox and mirrored to the host Mac with this machine's name on the card),
/// and forward `GetBrowserBridgeStatus` and `ExecuteBrowserAction` to the host
/// Mac over the peer Uplink. Host setup RPCs stay host-only.
pub(crate) struct BrowserBridgeOnMachine {
    inbox: Arc<Inbox>,
    forwarder: Arc<Forwarder>,
}

impl BrowserBridgeOnMachine {
    pub(crate) fn new(inbox: Arc<Inbox>, forwarder: Arc<Forwarder>) -> Self {
        Self { inbox, forwarder }
    }

    fn host_only<T>(method: &'static str) -> Result<Response<T>, Status> {
        Err(OnMachineError::HostOnly { method }.into())
    }
}

#[tonic::async_trait]
impl pb::browser_bridge_service_server::BrowserBridgeService for BrowserBridgeOnMachine {
    type ConnectBrowserHostStream = ReceiverStream<Result<pb::BrowserDaemonFrame, Status>>;

    async fn connect_browser_host(
        &self,
        _request: Request<Streaming<pb::BrowserHostFrame>>,
    ) -> Result<Response<Self::ConnectBrowserHostStream>, Status> {
        Self::host_only("/humata.v1.BrowserBridgeService/ConnectBrowserHost")
    }

    async fn execute_browser_action(
        &self,
        request: Request<pb::ExecuteBrowserActionRequest>,
    ) -> Result<Response<pb::ExecuteBrowserActionResponse>, Status> {
        let row = Method::ExecuteBrowserAction.row();
        let agent = match identity::caller(&request)? {
            Caller::Agent(agent) => agent,
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) | Caller::Script(_) => return Self::host_only(row.name),
        };
        if SupportedAgent::parse(&agent.agent).is_err() {
            return Err(Status::permission_denied(format!(
                "agent `{}` is not permitted to use the browser bridge (only claude, codex, copilot and agy are supported)",
                agent.agent
            )));
        }
        let req = request.into_inner();
        let action = BrowserAction::from_json(&req.action_json).map_err(|error| {
            status::invalid_field(
                "action_json",
                pb::invalid_field::Problem::BadFormat,
                format!("browser action JSON isn't valid: {error}"),
                "send a supported browser action JSON object",
            )
        })?;
        let is_write = ActionClass::classify(&action) == ActionClass::Write;
        let action_key = action.action_key();

        // Check host reachability before consuming a single-use approval so an
        // unlinked host doesn't burn the approval.
        if !self.forwarder.forwards() {
            return Err(OnMachineError::HostAway.into());
        }

        let approval_id = req
            .approval
            .as_ref()
            .map(|id| id.value.as_str())
            .filter(|s| !s.is_empty());
        self.inbox
            .consume_browser_approval(
                agent.session.id.as_str(),
                approval_id,
                &action_key,
                is_write,
            )
            .await?;

        let bytes = self
            .forwarder
            .call(
                agent.session.id.as_str(),
                row,
                req.encode_to_vec(),
                None,
            )
            .await?;
        pb::ExecuteBrowserActionResponse::decode(bytes.as_slice())
            .map(Response::new)
            .map_err(|source| OnMachineError::Reply { method: row.name, source }.into())
    }

    async fn get_browser_bridge_status(
        &self,
        request: Request<pb::GetBrowserBridgeStatusRequest>,
    ) -> Result<Response<pb::GetBrowserBridgeStatusResponse>, Status> {
        let row = Method::GetBrowserBridgeStatus.row();
        let agent = match identity::caller(&request)? {
            Caller::Agent(agent) => agent,
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) | Caller::Script(_) => return Self::host_only(row.name),
        };
        if SupportedAgent::parse(&agent.agent).is_err() {
            return Err(Status::permission_denied(format!(
                "agent `{}` is not permitted to query browser bridge status",
                agent.agent
            )));
        }
        if !self.forwarder.forwards() {
            return Err(OnMachineError::HostAway.into());
        }
        let bytes = self
            .forwarder
            .call(
                agent.session.id.as_str(),
                row,
                request.into_inner().encode_to_vec(),
                None,
            )
            .await?;
        pb::GetBrowserBridgeStatusResponse::decode(bytes.as_slice())
            .map(Response::new)
            .map_err(|source| OnMachineError::Reply { method: row.name, source }.into())
    }

    async fn list_detected_browsers(
        &self,
        _request: Request<pb::ListDetectedBrowsersRequest>,
    ) -> Result<Response<pb::ListDetectedBrowsersResponse>, Status> {
        Self::host_only("/humata.v1.BrowserBridgeService/ListDetectedBrowsers")
    }

    async fn install_browser_host(
        &self,
        _request: Request<pb::InstallBrowserHostRequest>,
    ) -> Result<Response<pb::ListDetectedBrowsersResponse>, Status> {
        Self::host_only("/humata.v1.BrowserBridgeService/InstallBrowserHost")
    }

    async fn uninstall_browser_host(
        &self,
        _request: Request<pb::UninstallBrowserHostRequest>,
    ) -> Result<Response<pb::ListDetectedBrowsersResponse>, Status> {
        Self::host_only("/humata.v1.BrowserBridgeService/UninstallBrowserHost")
    }
}

fn require_user_mutation(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(UserVia::LocalReadOnly { .. }) => Err(Status::permission_denied(
            "installing or uninstalling the browser host requires a full user session (the app or `humata login`)",
        )),
        Caller::User(_) => Ok(()),
        _ => Err(user_only_error()),
    }
}

fn user_only_error() -> Status {
    status::failure(
        Code::PermissionDenied,
        status::error(
            "Settings › Browser is user-only; agents cannot list, install or uninstall the browser host",
            "manage browser host setup in Humata Settings › Browser",
            pb::error::Kind::UserOnly(pb::UserOnly { capability: 0 }),
        ),
    )
}

fn infer_family(origin: &str) -> BrowserFamily {
    if origin.starts_with("chrome-extension://") {
        BrowserFamily::Chromium
    } else {
        BrowserFamily::Firefox
    }
}

/// Atomically creates or updates the symlink `link -> target`.
fn install_symlink(link: &Path, target: &Path) -> Result<(), Status> {
    let parent = link.parent().ok_or_else(|| {
        status::internal(
            format!("invalid browser host symlink path {}", link.display()),
            "",
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        status::internal(
            format!("couldn't create {}: {e}", parent.display()),
            "check permissions on ~/.humata2/bin",
        )
    })?;
    let tmp = parent.join(format!(
        ".humata2-browser-host.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    std::os::unix::fs::symlink(target, &tmp).map_err(|e| {
        status::internal(
            format!("couldn't create browser host symlink {}: {e}", tmp.display()),
            "check permissions on ~/.humata2/bin",
        )
    })?;
    if let Err(e) = std::fs::rename(&tmp, link) {
        let _ = std::fs::remove_file(&tmp);
        return Err(status::internal(
            format!("couldn't install browser host symlink {}: {e}", link.display()),
            "check permissions on ~/.humata2/bin",
        ));
    }
    Ok(())
}

fn symlink_points_to(link: &Path, expected_target: &Path) -> bool {
    let Ok(actual) = std::fs::read_link(link) else {
        return false;
    };
    if actual == expected_target {
        return true;
    }
    match (
        std::fs::canonicalize(&actual),
        std::fs::canonicalize(expected_target),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn manifest_points_to(manifest_path: &Path, expected_host_path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|p| Path::new(p) == expected_host_path)
}

fn pb_browser_kind(kind: BrowserKind) -> pb::BrowserKind {
    match kind {
        BrowserKind::Chrome => pb::BrowserKind::Chrome,
        BrowserKind::Brave => pb::BrowserKind::Brave,
        BrowserKind::Edge => pb::BrowserKind::Edge,
        BrowserKind::Chromium => pb::BrowserKind::Chromium,
        BrowserKind::Arc => pb::BrowserKind::Arc,
        BrowserKind::Vivaldi => pb::BrowserKind::Vivaldi,
        BrowserKind::Firefox => pb::BrowserKind::Firefox,
    }
}

fn pb_browser_family(family: BrowserFamily) -> pb::BrowserFamily {
    match family {
        BrowserFamily::Chromium => pb::BrowserFamily::Chromium,
        BrowserFamily::Firefox => pb::BrowserFamily::Firefox,
    }
}

fn parse_browser_kinds(raw: &[i32]) -> Result<Vec<BrowserKind>, Status> {
    let mut out = Vec::with_capacity(raw.len());
    for &value in raw {
        let kind = match pb::BrowserKind::try_from(value) {
            Ok(pb::BrowserKind::Chrome) => BrowserKind::Chrome,
            Ok(pb::BrowserKind::Brave) => BrowserKind::Brave,
            Ok(pb::BrowserKind::Edge) => BrowserKind::Edge,
            Ok(pb::BrowserKind::Chromium) => BrowserKind::Chromium,
            Ok(pb::BrowserKind::Arc) => BrowserKind::Arc,
            Ok(pb::BrowserKind::Vivaldi) => BrowserKind::Vivaldi,
            Ok(pb::BrowserKind::Firefox) => BrowserKind::Firefox,
            _ => {
                return Err(status::invalid_field(
                    "kinds",
                    pb::invalid_field::Problem::UnknownValue,
                    format!("unknown browser kind {value}"),
                    "choose from the browsers returned by ListDetectedBrowsers",
                ));
            }
        };
        out.push(kind);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use humata2_browser_copilot::{BrowserReplyPayload, TabInfo};
    use humata2_grpc::humata::v1::browser_bridge_service_server::BrowserBridgeService;
    use humata2_store::Store;

    use super::*;
    use crate::events::Feed;
    use crate::guard::Guard;

    fn test_svc(tmp: &tempfile::TempDir) -> BrowserBridgeSvc {
        let state = tmp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let store = Arc::new(Store::open(state.join("store")).unwrap());
        let guard = Arc::new(Guard::new("local", pb::DaemonRole::Host));
        let feed = Arc::new(Feed::default());
        let inbox = crate::wire_inbox::new_inbox(guard, store, "local".to_owned(), feed);
        let fake_exe = tmp.path().join("fake_humatad2");
        std::fs::write(&fake_exe, b"#!/bin/sh\n").unwrap();
        let layout = BrowserHostLayout {
            platform: Platform::MacOs,
            fs_root: tmp.path().join("root"),
            home_dir: tmp.path().join("home"),
            bin_dir: tmp.path().join("home/.humata2/bin"),
            own_exe: fake_exe,
            extension_root: tmp.path().join("ext"),
            chromium_origins: vec![DEFAULT_CHROMIUM_ORIGIN.to_owned()],
        };
        BrowserBridgeSvc::new(inbox, layout)
    }

    fn user_req<T>(msg: T) -> Request<T> {
        let mut req = Request::new(msg);
        req.extensions_mut().insert(Caller::User(UserVia::App));
        req
    }

    #[tokio::test]
    async fn install_and_uninstall_in_fake_home_creates_symlink_and_manifests_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = test_svc(&tmp);
        std::fs::create_dir_all(tmp.path().join("root/Applications/Google Chrome.app")).unwrap();
        std::fs::create_dir_all(tmp.path().join("root/Applications/Firefox.app")).unwrap();

        let before = svc
            .list_detected_browsers(user_req(pb::ListDetectedBrowsersRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(before.browsers.len(), 2);
        assert!(before.browsers.iter().all(|b| !b.host_installed));

        let installed = svc
            .install_browser_host(user_req(pb::InstallBrowserHostRequest {
                idempotency_key: None,
                kinds: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(installed.browsers.len(), 2);
        assert!(
            installed
                .browsers
                .iter()
                .all(|b| b.host_installed && b.host_matches_current)
        );
        assert!(svc.layout.host_link_path().is_symlink());

        let after_uninstall = svc
            .uninstall_browser_host(user_req(pb::UninstallBrowserHostRequest {
                idempotency_key: None,
                kinds: vec![pb::BrowserKind::Chrome as i32],
            }))
            .await
            .unwrap()
            .into_inner();
        let chrome = after_uninstall
            .browsers
            .iter()
            .find(|b| b.kind == pb::BrowserKind::Chrome as i32)
            .unwrap();
        let firefox = after_uninstall
            .browsers
            .iter()
            .find(|b| b.kind == pb::BrowserKind::Firefox as i32)
            .unwrap();
        assert!(!chrome.host_installed);
        assert!(firefox.host_installed);
    }

    #[tokio::test]
    async fn execute_correlates_request_id_and_times_out_when_host_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = test_svc(&tmp).with_timeout(Duration::from_millis(80));

        // Disconnected returns Unavailable.
        let err = svc
            .execute_browser_action(user_req(pb::ExecuteBrowserActionRequest {
                approval: None,
                action_json: serde_json::to_string(&BrowserAction::ListTabs).unwrap(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::Unavailable);

        // Attach a fake connected host channel.
        let (tx, mut rx) = mpsc::channel(4);
        svc.lock().hosts.push(ConnectedHost {
            id: 1,
            extension_origin: DEFAULT_CHROMIUM_ORIGIN.to_owned(),
            host_pid: 1234,
            family: BrowserFamily::Chromium,
            tx,
        });

        // Happy path: host replies with matching request_id.
        let responder = svc.clone();
        let respond_task = tokio::spawn(async move {
            let frame = rx.recv().await.unwrap().unwrap();
            let req: BrowserRequest = serde_json::from_str(&frame.request_json).unwrap();
            let reply = BrowserReply::ok(
                req.request_id,
                BrowserReplyPayload::Tabs {
                    tabs: vec![TabInfo {
                        tab_id: 1,
                        window_id: 1,
                        title: "Docs".to_owned(),
                        url: "https://example.com".to_owned(),
                        active: true,
                        pinned: false,
                    }],
                },
            );
            responder.deliver_reply(serde_json::to_string(&reply).unwrap());
            // Keep rx alive for the timeout check below.
            let _silent = rx.recv().await;
        });

        let ok = svc
            .execute_browser_action(user_req(pb::ExecuteBrowserActionRequest {
                approval: None,
                action_json: serde_json::to_string(&BrowserAction::ListTabs).unwrap(),
            }))
            .await
            .unwrap()
            .into_inner();
        let decoded: BrowserReply = serde_json::from_str(&ok.reply_json).unwrap();
        assert!(decoded.ok);

        // Silent host hits timeout -> DeadlineExceeded.
        let timed_out = svc
            .execute_browser_action(user_req(pb::ExecuteBrowserActionRequest {
                approval: None,
                action_json: serde_json::to_string(&BrowserAction::ListTabs).unwrap(),
            }))
            .await
            .unwrap_err();
        assert_eq!(timed_out.code(), Code::DeadlineExceeded);
        respond_task.abort();
    }
}

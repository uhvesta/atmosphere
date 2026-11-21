//! What an action is refused for when it would run across releases (UP-1,
//! D281 e, D290 l, m, o). `machine_ready` holds the actions a machine's
//! daemon takes to the host's release: starting a session on it, typing
//! into one, and a writable attach. `cli_ready` and `CliRelease` hold a CLI
//! to this daemon's release. Reads and ServerInfo are never refused here.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::pb;
use humata2_rpc_kit::helpers as status;
use tonic::codegen::http;
use tonic::{Code, Status};

use super::build::{self, BUNDLE_HEADER, Release};
use super::items::Migrations;
use crate::ports::{LOGIN_HEADER, SCRIPT_HEADER, TOKEN_HEADER};

/// Refuses an action on `machine` while its daemon is at another release
/// than this host's. The machine's Migrate item for the daemon says what it
/// would become, and the refusal points to that item (D281 e).
pub fn machine_ready(migrations: &Migrations, machine: &pb::MachineId) -> Result<(), Status> {
    match migrations.mismatched(machine) {
        Some((pb::MigrateThing::Daemon, request, from, to)) => Err(Refused::Daemon {
            machine: machine.value.clone(),
            name: migrations.name_of(machine),
            request,
            from,
            to,
        }
        .status()),
        // Another thing waiting to migrate doesn't stop the daemon's actions.
        _ => Ok(()),
    }
}

/// Holds a CLI to this daemon's release (D290 m). A call is a CLI when it
/// carries a CLI credential or `humata-bundle`; a CLI must name this
/// daemon's release, and one naming none is refused. A daemon with no
/// release refuses no CLI (D290 l), and a call that isn't a CLI passes.
pub fn cli_ready(
    release: &Release,
    machine: &pb::MachineId,
    name: &str,
    header: Option<&str>,
    is_cli: bool,
) -> Result<(), Status> {
    let bundle = release.bundle();
    if bundle.is_empty() || !is_cli || header == Some(bundle) {
        return Ok(());
    }
    let to = release.info();
    let refused = match header.filter(|named| !named.is_empty()) {
        Some(named) => Refused::CliAnother {
            machine: machine.value.clone(),
            name: name.to_owned(),
            from: pb::BuildInfo {
                bundle: named.to_owned(),
                ..Default::default()
            },
            to,
        },
        None => Refused::CliUnnamed {
            machine: machine.value.clone(),
            name: name.to_owned(),
            to,
        },
    };
    Err(refused.status())
}

/// What a call that would run across releases is refused for (UP-1, D290 o).
#[derive(Debug, thiserror::Error)]
enum Refused {
    #[error(
        "Migrate the daemon on {name} first: it is at {}, and this host is at {}, so nothing runs across releases until it is migrated",
        release_name(.from),
        release_name(.to)
    )]
    Daemon {
        /// The machine's id, which `migration_needed` carries.
        machine: String,
        /// What a person calls it (D500).
        name: String,
        request: pb::RequestId,
        from: pb::BuildInfo,
        to: pb::BuildInfo,
    },

    #[error(
        "this humata is at {}, but the daemon on {name} is at {}, so the daemon refuses it",
        release_name(.from),
        release_name(.to)
    )]
    CliAnother {
        machine: String,
        name: String,
        from: pb::BuildInfo,
        to: pb::BuildInfo,
    },

    #[error(
        "this humata names no release, so the daemon on {name} (at {}) refuses it",
        release_name(.to)
    )]
    CliUnnamed {
        machine: String,
        name: String,
        to: pb::BuildInfo,
    },
}

impl Refused {
    /// The status a refused call ends with: `migration_needed` names the
    /// thing, the machine, the Inbox item when there is one, and both builds.
    fn status(&self) -> Status {
        let (needed, fix) = match self {
            Self::Daemon {
                machine,
                name,
                request,
                from,
                to,
            } => (
                needed(pb::MigrateThing::Daemon, machine, Some(request.clone()), from, to),
                format!("Open “Migrate the daemon on {name}” in the Inbox and click Migrate."),
            ),
            Self::CliAnother { machine, from, to, .. } => (
                needed(pb::MigrateThing::Cli, machine, None, from, to),
                "run the humata that comes with this daemon's release, the one in ~/.humata2/bin with its bundle.json beside it, or click Migrate on the CLI in the Inbox if it is waiting there. Replacing only the humata binary isn't enough: the bundle.json beside it names the release, so it must match too".to_owned(),
            ),
            Self::CliUnnamed { machine, to, .. } => (
                needed(
                    pb::MigrateThing::Cli,
                    machine,
                    None,
                    &pb::BuildInfo::default(),
                    to,
                ),
                "install the Humata release this daemon is at and run the humata it puts in ~/.humata2/bin (its bundle.json beside it names the release), or click Migrate on the item in the Inbox to bring this machine to your Humata's release. A humata built by hand names no release, and a daemon at a release refuses it, login included".to_owned(),
            ),
        };
        status::failure(
            Code::FailedPrecondition,
            status::error(
                self.to_string(),
                fix,
                pb::error::Kind::MigrationNeeded(needed),
            ),
        )
    }
}

fn needed(
    thing: pb::MigrateThing,
    machine: &str,
    request: Option<pb::RequestId>,
    from: &pb::BuildInfo,
    to: &pb::BuildInfo,
) -> pb::MigrationNeeded {
    pb::MigrationNeeded {
        thing: thing as i32,
        machine: Some(pb::MachineId {
            value: machine.to_owned(),
        }),
        request,
        from: Some(from.clone()),
        to: Some(to.clone()),
    }
}

/// A build as a message names it: its version and the short id of its release.
fn release_name(build: &pb::BuildInfo) -> String {
    let bundle = build::short(&build.bundle);
    match (build.version.is_empty(), bundle.is_empty()) {
        (_, true) => "an unnamed release".to_owned(),
        (true, false) => format!("release {bundle}"),
        (false, false) => format!("version {} (release {bundle})", build.version),
    }
}

/// The layer that holds every call but ServerInfo to the CLI rule (D290 m).
/// It sits inside `Authenticate`, so a call's credential is checked first.
#[derive(Clone)]
pub struct CliRelease {
    release: Release,
    machine: pb::MachineId,
    name: String,
}

impl CliRelease {
    pub fn new(release: Release, machine: &str) -> Self {
        Self {
            release,
            machine: pb::MachineId {
                value: machine.to_owned(),
            },
            name: machine.to_owned(),
        }
    }

    /// What a refusal calls this daemon's machine (D500); its id until told.
    pub fn named(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }
}

impl<S> tower::Layer<S> for CliRelease {
    type Service = CliReleaseCheck<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CliReleaseCheck {
            inner,
            release: self.release.clone(),
            machine: self.machine.clone(),
            name: self.name.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CliReleaseCheck<S> {
    inner: S,
    release: Release,
    machine: pb::MachineId,
    name: String,
}

impl<S, B, ResB> tower::Service<http::Request<B>> for CliReleaseCheck<S>
where
    S: tower::Service<http::Request<B>, Response = http::Response<ResB>> + 'static,
    S::Future: Send + 'static,
    S::Error: 'static,
    B: 'static,
    ResB: Default + 'static,
{
    type Response = http::Response<ResB>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<http::Response<ResB>, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        match cli_check(&self.release, &self.machine, &self.name, &request) {
            Ok(()) => Box::pin(self.inner.call(request)),
            // The refusal is a whole reply with the status in its headers,
            // as a unary call's error is.
            Err(status) => Box::pin(async move {
                Ok::<http::Response<ResB>, S::Error>(status.into_http())
            }),
        }
    }
}

/// Whether the call `request` may go on. ServerInfo always may (D290 l): it
/// is how a CLI learns which release to be. So may Upgrade (D281 e): it is
/// how a daemon at another release is brought to this one.
fn cli_check<B>(
    release: &Release,
    machine: &pb::MachineId,
    name: &str,
    request: &http::Request<B>,
) -> Result<(), Status> {
    if is_exempt(request.uri().path()) {
        return Ok(());
    }
    let headers = request.headers();
    let is_cli = [TOKEN_HEADER, LOGIN_HEADER, SCRIPT_HEADER, BUNDLE_HEADER]
        .iter()
        .any(|name| headers.contains_key(*name));
    let header = headers
        .get(BUNDLE_HEADER)
        .map(|value| value.to_str().unwrap_or(""));
    cli_ready(release, machine, name, header, is_cli)
}

fn is_exempt(path: &str) -> bool {
    path.strip_prefix('/')
        .and_then(|route| route.split_once('/'))
        .is_some_and(|(service, method)| {
            service == pb::daemon_service_server::SERVICE_NAME
                && ["GetServerInfo", "Upgrade"].contains(&method)
        })
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use humata2_rpc_kit::helpers::status_error_of;
    use sha2::{Digest, Sha256};
    use tonic::codegen::http;
    use tower::{Layer, ServiceExt};

    use super::build::{BUNDLE_HEADER, Release};
    use super::*;

    const GPU: &str = "gpu";
    const LIST_TABS: &str = "/humata.v1.TerminalService/ListTabs";
    const SERVER_INFO: &str = "/humata.v1.DaemonService/GetServerInfo";
    const UPGRADE: &str = "/humata.v1.DaemonService/Upgrade";

    fn gpu() -> pb::MachineId {
        pb::MachineId {
            value: GPU.to_owned(),
        }
    }

    /// A bundle id nothing in these tests has.
    fn another_release() -> String {
        "f".repeat(64)
    }

    fn platform() -> &'static str {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "darwin-arm64",
            ("macos", "x86_64") => "darwin-x86_64",
            ("linux", "aarch64") => "linux-aarch64",
            ("linux", "x86_64") => "linux-x86_64",
            (os, arch) => panic!("Humata ships no bundle for {os}-{arch}"),
        }
    }

    /// A daemon's release in a folder of its own: with a `bundle.json` in
    /// its `bin` folder that names the daemon's file, or with none.
    fn release(with_bundle: bool) -> (tempfile::TempDir, Release) {
        let dir = tempfile::tempdir().expect("a temp folder");
        let daemon = dir.path().join("humatad2");
        std::fs::write(&daemon, b"a daemon").expect("the daemon's file is written");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("the bin folder is made");
        if with_bundle {
            let sha = Sha256::digest(b"a daemon").iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            let entry = serde_json::json!({
                "humatad2": {"path": "humatad2", "sha256": sha},
                "humata2_holder": {"path": "holder", "sha256": sha},
                "humata": {"path": "humata", "sha256": sha},
            });
            let mut platforms = serde_json::Map::new();
            platforms.insert(platform().to_owned(), entry);
            let manifest = serde_json::json!({ "platforms": platforms });
            std::fs::write(bin.join("bundle.json"), manifest.to_string())
                .expect("bundle.json is written");
        }
        let release = Release::read(&bin, &daemon, "2.0.5", 3);
        (dir, release)
    }

    fn refusal_of(status: &Status) -> pb::MigrationNeeded {
        match status_error_of(status).and_then(|error| error.kind) {
            Some(pb::error::Kind::MigrationNeeded(needed)) => needed,
            other => panic!("expected migration_needed, got {other:?}"),
        }
    }

    /// The reply a call on `path` with `headers` gets through the layer,
    /// over a service that answers every call with nothing.
    async fn through(release: Release, path: &str, headers: &[(&str, &str)]) -> http::Response<String> {
        let mut request = http::Request::builder().uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = request.body(()).expect("a request");
        let answers = tower::service_fn(|_: http::Request<()>| async {
            Ok::<_, Infallible>(http::Response::new(String::new()))
        });
        CliRelease::new(release, GPU)
            .layer(answers)
            .oneshot(request)
            .await
            .expect("the layer never fails")
    }

    #[test]
    fn a_daemon_with_no_release_refuses_no_cli() {
        let (_dir, release) = release(false);

        let served = cli_ready(&release, &gpu(), GPU, Some(&another_release()), true);

        assert!(served.is_ok(), "{served:?}");
    }

    #[test]
    fn a_cli_naming_this_release_is_served() {
        let (_dir, release) = release(true);
        let bundle = release.bundle().to_owned();

        let served = cli_ready(&release, &gpu(), GPU, Some(&bundle), true);

        assert!(served.is_ok(), "{served:?}");
    }

    #[test]
    fn a_cli_naming_another_release_is_refused_for_the_cli() {
        let (_dir, release) = release(true);

        let refused = cli_ready(&release, &gpu(), GPU, Some(&another_release()), true)
            .expect_err("a CLI at another release is refused");

        assert_eq!(refused.code(), Code::FailedPrecondition);
        assert_eq!(refusal_of(&refused).thing(), pb::MigrateThing::Cli);
    }

    #[test]
    fn a_cli_that_names_no_release_is_refused() {
        let (_dir, release) = release(true);

        let refused = cli_ready(&release, &gpu(), GPU, None, true).expect_err("an unnamed CLI is refused");

        assert_eq!(refusal_of(&refused).thing(), pb::MigrateThing::Cli);
    }

    #[test]
    fn a_refusal_calls_the_machine_by_its_name_and_carries_its_id() {
        let (_dir, release) = release(true);

        let refused = cli_ready(&release, &gpu(), "This Mac", None, true).expect_err("an unnamed CLI is refused");

        assert!(refused.message().contains("This Mac"), "{}", refused.message());
        assert!(!refused.message().contains(GPU), "{}", refused.message());
        assert_eq!(refusal_of(&refused).machine, Some(gpu()));
    }

    #[test]
    fn a_call_that_is_no_cli_is_served_whatever_it_names() {
        let (_dir, release) = release(true);

        let served = cli_ready(&release, &gpu(), GPU, Some(&another_release()), false);

        assert!(served.is_ok(), "{served:?}");
    }

    #[tokio::test]
    async fn a_cli_at_another_release_is_refused_on_the_wire_as_migration_needed() {
        let (_dir, release) = release(true);

        let response = through(release, LIST_TABS, &[(BUNDLE_HEADER, another_release().as_str())]).await;

        let status = Status::from_header_map(response.headers()).expect("the call is refused");
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert_eq!(refusal_of(&status).thing(), pb::MigrateThing::Cli);
    }

    #[tokio::test]
    async fn server_info_is_served_to_a_cli_at_another_release() {
        let (_dir, release) = release(true);

        let response = through(release, SERVER_INFO, &[(BUNDLE_HEADER, another_release().as_str())]).await;

        assert!(Status::from_header_map(response.headers()).is_none());
    }

    #[tokio::test]
    async fn upgrade_is_served_to_a_cli_at_another_release_and_the_daemon_decides_who_may_call_it() {
        let (_dir, release) = release(true);

        let response = through(release, UPGRADE, &[(BUNDLE_HEADER, another_release().as_str())]).await;

        assert!(Status::from_header_map(response.headers()).is_none());
    }

    #[tokio::test]
    async fn a_call_with_no_credential_and_no_release_is_served() {
        let (_dir, release) = release(true);

        let response = through(release, LIST_TABS, &[]).await;

        assert!(Status::from_header_map(response.headers()).is_none());
    }
}

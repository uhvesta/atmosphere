//! `humatad2 browser-host` / `humata2-browser-host` (BROWSER, A4): WebExtension
//! Native Messaging stdio host.
//!
//! Browsers invoke the native messaging manifest's `"path"` directly with no
//! custom arguments array, passing `<extension-origin>` (or the manifest path
//! on Firefox) as `argv[1]`. `InstallBrowserHost` links
//! `~/.humata2/bin/humata2-browser-host -> humatad2`; `main.rs` detects an
//! `argv[0]` basename of `humata2-browser-host` before `Cli::parse()` and
//! enters this module on a lightweight current-thread tokio runtime.

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

use humata2_browser_copilot::{ProtocolError, read_frame, write_frame};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Endpoint, Uri};

use crate::error::DaemonError;
use crate::{identity, serve};

/// The symlink basename browsers launch for the native messaging host.
pub const ARGV0_NAME: &str = "humata2-browser-host";

/// Filename inside `<state_dir>` holding the `0o600` browser host token.
pub const TOKEN_FILE: &str = "browser-host.token";

#[derive(clap::Args, Debug, Clone)]
pub struct BrowserHostArgs {
    /// The daemon's socket. Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    pub socket: Option<PathBuf>,
    /// The browser host token file. Default: ~/.humata2/state/browser-host.token.
    #[arg(long)]
    pub token_file: Option<PathBuf>,
    /// Positional arguments passed by the browser (e.g. `chrome-extension://<id>/`).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub extra: Vec<OsString>,
}

/// Returns `true` when this process was launched via the `humata2-browser-host`
/// symlink (`argv[0]` basename matches [`ARGV0_NAME`]).
pub fn invoked_as_browser_host() -> bool {
    std::env::args_os()
        .next()
        .is_some_and(|arg0| Path::new(&arg0).file_name().and_then(|n| n.to_str()) == Some(ARGV0_NAME))
}

/// Builds [`BrowserHostArgs`] when invoked via `argv[0] == humata2-browser-host`.
/// Honors `HUMATA_SOCKET` and `HUMATA_BROWSER_HOST_TOKEN_FILE` so tests can
/// point the real symlink at a test daemon without touching `~/.humata2`.
pub fn args_from_argv0() -> BrowserHostArgs {
    let extra: Vec<OsString> = std::env::args_os().skip(1).collect();
    let socket = std::env::var_os("HUMATA_SOCKET")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let token_file = std::env::var_os("HUMATA_BROWSER_HOST_TOKEN_FILE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    BrowserHostArgs {
        socket,
        token_file,
        extra,
    }
}

fn default_token_file() -> Result<PathBuf, DaemonError> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".humata2/state").join(TOKEN_FILE))
        .ok_or(DaemonError::NoHome)
}

/// Connects to `humatad2` over UDS, authenticates with `humata-browser-host-token`,
/// and pumps 4-byte native-endian JSON frames between stdio and
/// `BrowserBridgeService.ConnectBrowserHost` until either side closes.
pub async fn run(args: BrowserHostArgs) -> Result<(), DaemonError> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => serve::default_socket()?,
    };
    let token_file = match args.token_file {
        Some(path) => path,
        None => default_token_file()?,
    };
    let token = std::fs::read_to_string(&token_file)
        .map_err(|source| DaemonError::BrowserHostToken {
            path: token_file.clone(),
            source,
        })?
        .trim()
        .to_owned();

    let extension_origin = args
        .extra
        .first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .unwrap_or_default();

    let dial_socket = socket.clone();
    let channel = Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = dial_socket.clone();
            async move { Ok::<_, io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        }))
        .await
        .map_err(|source| DaemonError::BrowserHostConnect {
            socket: socket.clone(),
            source,
        })?;

    let mut client =
        pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(channel);
    let (to_daemon_tx, to_daemon_rx) = mpsc::channel::<pb::BrowserHostFrame>(32);

    let hello = pb::BrowserHostFrame {
        frame: Some(pb::browser_host_frame::Frame::Hello(pb::BrowserHostHello {
            extension_origin,
            host_pid: std::process::id(),
        })),
    };
    let _ = to_daemon_tx.send(hello).await;

    let mut request = Request::new(ReceiverStream::new(to_daemon_rx));
    let mut header = MetadataValue::try_from(token.as_str()).map_err(|_| {
        DaemonError::BrowserHostToken {
            path: token_file,
            source: io::Error::new(io::ErrorKind::InvalidData, "token contains invalid header characters"),
        }
    })?;
    header.set_sensitive(true);
    request
        .metadata_mut()
        .insert(identity::BROWSER_HOST_HEADER, header);

    let mut from_daemon = client
        .connect_browser_host(request)
        .await
        .map_err(|source| DaemonError::BrowserHostStream { source })?
        .into_inner();

    // Pump stdin -> daemon on a dedicated blocking thread so stdin reads don't
    // stall the current-thread runtime. Moving `to_daemon_tx` into this task
    // ensures stdin EOF closes the gRPC request stream cleanly.
    let stdin_task = tokio::task::spawn_blocking(move || {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        loop {
            match read_frame::<_, serde_json::Value>(&mut reader) {
                Ok(value) => {
                    let Ok(reply_json) = serde_json::to_string(&value) else {
                        break;
                    };
                    let frame = pb::BrowserHostFrame {
                        frame: Some(pb::browser_host_frame::Frame::ReplyJson(reply_json)),
                    };
                    if to_daemon_tx.blocking_send(frame).is_err() {
                        break;
                    }
                }
                Err(ProtocolError::UnexpectedEof) => break,
                Err(_) => break,
            }
        }
    });

    // Pump daemon -> stdout until either side closes.
    let mut stdout = BufWriter::new(io::stdout());
    loop {
        match from_daemon.message().await {
            Ok(Some(frame)) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&frame.request_json)
                else {
                    continue;
                };
                if write_frame(&mut stdout, &value).is_err() {
                    break;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    stdin_task.abort();
    Ok(())
}

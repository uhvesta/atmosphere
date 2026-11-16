use std::{net::SocketAddr, path::PathBuf, process::ExitCode};
use terminal_host::{Config, Error, Host, Profile, serve};
use terminal_session::{Endpoint, SessionStore};
use terminal_wire::HistoryPolicy;
struct Options {
    state: PathBuf,
    holder: PathBuf,
    assets: PathBuf,
    listen: SocketAddr,
    profiles: Option<PathBuf>,
    starts: Vec<String>,
    history: HistoryPolicy,
}
fn options() -> Result<Options, Error> {
    let mut state = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".humata-terminal-host");
    let executable = std::env::current_exe().map_err(|source| Error::Disk {
        operation: "locate host executable",
        path: PathBuf::new(),
        source,
    })?;
    let mut runfiles = executable.into_os_string();
    runfiles.push(".runfiles");
    let mut holder =
        PathBuf::from(runfiles).join("_main/apps/humata2/daemon/holder/humata2_holder");
    let mut assets = None;
    let mut listen = "127.0.0.1:8741"
        .parse::<SocketAddr>()
        .map_err(|_| Error::Invalid {
            reason: "default address could not be parsed",
        })?;
    let mut profiles = None;
    let mut starts = Vec::new();
    let mut off = false;
    let mut lines = 10000;
    let mut bytes = 4 * 1024 * 1024;
    let mut seconds = 30;
    let mut args = std::env::args_os().skip(1);
    while let Some(option) = args.next() {
        if option == "--no-history" {
            off = true;
            continue;
        }
        let value=args.next().ok_or(Error::Invalid{reason:"an option is missing its value; use --state DIR --holder BINARY --static-dir DIR --listen 127.0.0.1:PORT [--profiles FILE] [--start NAME] [--no-history]"})?;
        match option.to_str() {
            Some("--state") => state = value.into(),
            Some("--holder") => holder = value.into(),
            Some("--static-dir") => assets = Some(PathBuf::from(value)),
            Some("--listen") => {
                listen = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or(Error::Invalid {
                        reason: "--listen must be a loopback IP address and port",
                    })?
            }
            Some("--profiles") => profiles = Some(PathBuf::from(value)),
            Some("--start") => starts.push(value.to_string_lossy().into_owned()),
            Some("--history-lines") => {
                lines = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or(Error::Invalid {
                        reason: "--history-lines must be a positive integer",
                    })?
            }
            Some("--checkpoint-bytes") => {
                bytes = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or(Error::Invalid {
                        reason: "--checkpoint-bytes must be a positive integer",
                    })?
            }
            Some("--checkpoint-seconds") => {
                seconds = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or(Error::Invalid {
                        reason: "--checkpoint-seconds must be a positive integer",
                    })?
            }
            _ => {
                return Err(Error::Invalid {
                    reason: "unknown option; use --state DIR --holder BINARY --static-dir DIR --listen 127.0.0.1:PORT [--profiles FILE] [--start NAME] [--no-history]",
                });
            }
        }
    }
    Ok(Options {
        state,
        holder,
        assets: assets.ok_or(Error::Invalid {
            reason: "pass --static-dir with the built shared GPUI browser assets",
        })?,
        listen,
        profiles,
        starts,
        history: if off {
            HistoryPolicy::Off
        } else {
            HistoryPolicy::Recent {
                max_lines: lines,
                max_bytes: bytes,
                interval_seconds: seconds,
            }
        },
    })
}
async fn run() -> Result<(), Error> {
    let options = options()?;
    let profiles = if let Some(path) = &options.profiles {
        let bytes = std::fs::read(path).map_err(|source| Error::Disk {
            operation: "read trusted launch profiles",
            path: path.clone(),
            source,
        })?;
        serde_json::from_slice::<Vec<Profile>>(&bytes).map_err(|source| Error::Decode {
            path: path.clone(),
            source,
        })?
    } else {
        vec![Profile {
            name: "shell".into(),
            title: "Shell".into(),
            program: std::env::var_os("SHELL")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/bin/sh".into()),
            args: vec!["-l".into()],
            cwd: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/tmp".into()),
            endpoint: Endpoint::Local,
            recovery: None,
            history: options.history.clone(),
        }]
    };
    let store = SessionStore::open_memory_only(&options.state, options.holder)?;
    let token = options.state.join("browser.token");
    let state = std::fs::canonicalize(&options.state).map_err(|source| Error::Disk {
        operation: "locate private state",
        path: options.state.clone(),
        source,
    })?;
    let assets = std::fs::canonicalize(&options.assets).map_err(|source| Error::Disk {
        operation: "locate browser assets",
        path: options.assets.clone(),
        source,
    })?;
    if state.starts_with(&assets) {
        return Err(Error::Invalid {
            reason: "browser assets must not contain the private host state directory",
        });
    }
    let host = Host::start(
        Config {
            state: options.state,
            label: "Terminal host".into(),
            start_profiles: options.starts,
            default_history: options.history,
        },
        store,
        profiles,
    )
    .await?;
    serve(host, options.listen, assets, token).await
}
fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Could not start the terminal runtime. Close unused processes and retry.");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

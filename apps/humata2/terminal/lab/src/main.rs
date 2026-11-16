use gpui_kit::{prelude::*, *};
use humata2_engine::Resources;
use std::{path::PathBuf, process::ExitCode};
use terminal_lab::{Assets, Lab};
use terminal_session::{Endpoint, SessionStore};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(
        "Terminal Lab options: {reason}. Use --data FOLDER --holder BINARY; optional --ssh-config FILE --ssh-host ALIAS --remote-holder PATH --remote-root FOLDER."
    )]
    Args { reason: String },
    #[error(
        "Could not locate Terminal Lab on disk. Rebuild the app or pass a valid --holder path."
    )]
    Executable {
        #[source]
        source: std::io::Error,
    },
    #[error("Could not start the terminal runtime. Close other apps and retry.")]
    Runtime {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    State(#[from] terminal_session::Error),
}
gpui_kit::actions!(terminal_lab, [Quit]);
fn run() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let mut data = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into())
        .join(".humata-terminal-lab");
    let executable = std::env::current_exe().map_err(|source| Error::Executable { source })?;
    let mut runfiles = executable.into_os_string();
    runfiles.push(".runfiles");
    let mut holder =
        PathBuf::from(runfiles).join("_main/apps/humata2/daemon/holder/humata2_holder");
    let mut config = None;
    let mut host = "lima-h2-terminal-c1".to_owned();
    let mut remote_holder = PathBuf::from("/tmp/h2-terminal-holder");
    let mut remote_root = "/var/tmp/terminal-lab".to_owned();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| Error::Args {
            reason: format!("missing value for {arg}"),
        })?;
        match arg.as_str() {
            "--data" => data = value.into(),
            "--holder" => holder = value.into(),
            "--ssh-config" => config = Some(PathBuf::from(value)),
            "--ssh-host" => host = value,
            "--remote-holder" => remote_holder = value.into(),
            "--remote-root" => remote_root = value,
            _ => {
                return Err(Error::Args {
                    reason: format!("unknown option {arg}"),
                });
            }
        }
    }
    let holder =
        std::fs::canonicalize(&holder).map_err(|source| terminal_session::Error::Disk {
            operation: "locate the holder (pass --holder explicitly)",
            path: holder,
            source,
        })?;
    let store = SessionStore::open(&data, holder)?;
    let ssh = config.map(|config| Endpoint::Ssh {
        config,
        destination: host,
        holder: remote_holder,
        root: remote_root,
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|source| Error::Runtime { source })?;
    let handle = runtime.handle().clone();
    let resources = Resources::new(Assets);
    humata2_engine::application(&resources).run(move |cx| {
        if let Err(e) = humata2_engine::init(cx, &resources) {
            eprintln!("{e}");
            cx.quit();
            return;
        }
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1200.), px(760.)),
                cx,
            ))),
            titlebar: None,
            ..Default::default()
        };
        if let Err(e) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Lab::new(store, handle, ssh, window, cx))
        }) {
            eprintln!("Could not open Terminal Lab: {e}");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
    drop(runtime);
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

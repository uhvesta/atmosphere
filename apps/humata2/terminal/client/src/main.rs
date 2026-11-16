mod native;
use gpui_kit::{prelude::*, *};
use std::process::ExitCode;
use terminal_client::{Options, RemoteWorkspace, resources};
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Invalid viewer options: {reason}. Use --url ws://HOST:PORT/ws --token-file PATH.")]
    Args { reason: String },
    #[error(
        "Could not read the terminal authentication token from {path}. Check that the server is running and this account can read its private token file."
    )]
    TokenFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not start the viewer's network runtime. Close other applications and retry.")]
    Runtime {
        #[source]
        source: std::io::Error,
    },
}
gpui_kit::actions!(terminal_remote_client, [Quit]);
fn run() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let mut url = "ws://127.0.0.1:8741/ws".to_string();
    let mut token = None;
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| Error::Args {
            reason: format!("missing value for {arg}"),
        })?;
        match arg.as_str() {
            "--url" => url = value,
            "--token" => token = Some(value),
            "--token-file" => {
                token = Some(
                    std::fs::read_to_string(&value)
                        .map_err(|source| Error::TokenFile {
                            path: value,
                            source,
                        })?
                        .trim()
                        .to_string(),
                )
            }
            _ => {
                return Err(Error::Args {
                    reason: format!("unknown option {arg}"),
                });
            }
        }
    }
    let token = token.filter(|s| !s.is_empty()).ok_or_else(|| Error::Args {
        reason: "missing authentication token".into(),
    })?;
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err(Error::Args {
            reason: "the URL must use ws:// or wss://".into(),
        });
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|source| Error::Runtime { source })?;
    let transport = native::NativeSocket::new(url, runtime.handle());
    let resources = resources();
    humata2_engine::application(&resources).run(move |cx| {
        if let Err(error) = humata2_engine::init(cx, &resources) {
            eprintln!("Could not initialize the terminal viewer: {error}");
            cx.quit();
            return;
        }
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.), px(800.)),
                cx,
            ))),
            titlebar: None,
            ..Default::default()
        };
        if let Err(error) = gpui_kit::open_window(options, cx, |w, cx| {
            cx.new(|cx| {
                RemoteWorkspace::new(
                    transport,
                    Options {
                        token,
                        title: "Humata · Remote terminal".into(),
                        diagnostics: None,
                    },
                    w,
                    cx,
                )
            })
        }) {
            eprintln!("Could not open the terminal viewer: {error}");
            cx.quit();
        }
    });
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

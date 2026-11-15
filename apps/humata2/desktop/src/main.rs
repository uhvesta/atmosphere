//! Humata 2's desktop app, on fake clients until the real daemon is wired
//! in: the window, its menus, and where the app keeps what it saves.
//!
//! ```text
//! humata2 [--data <folder>]
//! ```
//!
//! `--data` is where the window's state is kept between runs; by default
//! `~/Library/Application Support/Humata 2 (fakes)`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use gpui_kit::{
    App, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
};
use humata2_clients::Clients;
use humata2_engine::Resources;
use humata2_ui::bridge::Ui;
use humata2_ui::keys;
use humata2_ui::record::Recorder;
use tca::{FileStorage, SystemClock};

gpui_kit::actions!(humata_desktop, [
    /// ⌘Q: quit.
    Quit
]);

/// What went wrong reading the command line.
#[derive(Debug, thiserror::Error)]
enum ArgsError {
    /// An option this app doesn't know.
    #[error("humata2 doesn't know the option `{option}`. Run it with no options, or with `--data <folder>`.")]
    Unknown {
        /// The option given.
        option: String,
    },
    /// An option that needs a value had none.
    #[error("`{option}` needs a folder after it, such as `{option} /tmp/humata2`.")]
    Missing {
        /// The option given.
        option: &'static str,
    },
    /// No `--data` and no home folder to put the default in.
    #[error("There is no HOME to keep the app's state in. Set HOME, or pass `--data <folder>`.")]
    NoHome {
        /// Why HOME couldn't be read.
        #[source]
        source: std::env::VarError,
    },
}

/// What went wrong starting the app.
#[derive(Debug, thiserror::Error)]
enum StartError {
    /// The command line.
    #[error(transparent)]
    Args(#[from] ArgsError),
    /// The runtime effects run on.
    #[error("The app couldn't start its background runtime; the system may be out of threads. Close other apps and try again.")]
    Runtime {
        /// What tokio said.
        #[source]
        source: std::io::Error,
    },
}

struct Args {
    data: PathBuf,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, ArgsError> {
        let mut data = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => {
                    data = Some(PathBuf::from(
                        args.next().ok_or(ArgsError::Missing { option: "--data" })?,
                    ));
                }
                _ => return Err(ArgsError::Unknown { option: arg }),
            }
        }
        let data = match data {
            Some(data) => data,
            None => PathBuf::from(std::env::var("HOME").map_err(|source| ArgsError::NoHome { source })?)
                .join("Library/Application Support/Humata 2 (fakes)"),
        };
        Ok(Self { data })
    }
}

fn menus(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus([
        Menu::new("Humata").items([MenuItem::action("Quit Humata", Quit)]),
        Menu::new("File").items([
            MenuItem::action("New Session", keys::NewSession),
            MenuItem::action("New Channel", keys::NewChannel),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette", keys::OpenPalette),
            MenuItem::action("Show or Hide Sidebar", keys::ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Home", keys::GoHome),
            MenuItem::action("Inbox", keys::GoInbox),
            MenuItem::action("Board", keys::GoBoard),
        ]),
        Menu::new("Help").items([MenuItem::action("Keyboard Shortcuts", keys::ShowShortcuts)]),
    ]);
}

fn run() -> Result<(), StartError> {
    let args = Args::parse(std::env::args().skip(1))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|source| StartError::Runtime { source })?;
    let handle = runtime.handle().clone();
    let resources = Resources::new(humata2_ui::Assets);
    humata2_engine::application(&resources).run(move |cx| {
        if let Err(error) = humata2_engine::init(cx, &resources) {
            eprintln!("{error}");
            cx.quit();
            return;
        }
        humata2_ui::init(cx);
        menus(cx);
        let storage = Arc::new(FileStorage::new(args.data));
        let (clients, _daemon) = Clients::demo(Arc::new(SystemClock::new()), storage);
        let ui = Ui::new(&handle, clients, Recorder::new());
        let bounds = Bounds::centered(None, size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Humata".into()),
                ..TitlebarOptions::default()
            }),
            ..WindowOptions::default()
        };
        if let Err(error) = humata2_ui::open(cx, &ui, options) {
            eprintln!("Humata couldn't open its window: {error}. Check the system's display settings and try again.");
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
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

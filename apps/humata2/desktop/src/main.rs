//! Humata 2's desktop app: the window, its menus, where the app keeps what
//! it saves, and which clients it runs on.
//!
//! ```text
//! humata2 [--data <folder>] [--live] [--socket <path>]
//! ```
//!
//! From an app bundle, or with `--live` or `--socket`, the app installs the
//! host tools it carries (from a bundle), waits for This Mac's daemon and
//! runs on it; its state is kept in `~/Library/Application Support/Humata
//! 2`. Otherwise it runs on the demo workspace's fakes and keeps its state in
//! `~/Library/Application Support/Humata 2 (fakes)`. `--data` overrides
//! either folder; `--socket` the daemon's socket, `~/.humata2/humatad.sock`.

mod live;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, px, size};
use humata2_clients::Clients;
use humata2_engine::Resources;
use humata2_install::Installed;
use humata2_ui::bridge::Ui;
use humata2_ui::keys;
use humata2_ui::record::Recorder;
use tca::{FileStorage, SystemClock};

gpui_kit::actions!(
    humata_desktop,
    [
        /// ⌘Q: quit.
        Quit
    ]
);

/// What went wrong reading the command line.
#[derive(Debug, thiserror::Error)]
enum ArgsError {
    /// An option this app doesn't know.
    #[error(
        "humata2 doesn't know the option `{option}`. Run it with no options, or with `--data <folder>`, `--live` or `--socket <path>`."
    )]
    Unknown {
        /// The option given.
        option: String,
    },
    /// An option that needs a value had none.
    #[error("`{option}` needs a path after it, such as `{option} {example}`.")]
    Missing {
        /// The option given.
        option: &'static str,
        /// A value it could take.
        example: &'static str,
    },
    /// No home folder to put the default state folder in, or to find This
    /// Mac's daemon in.
    #[error(
        "There is no HOME to keep the app's state in or to find the daemon under. Set HOME; on the demo workspace, passing `--data <folder>` also works."
    )]
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
    #[error(
        "The app couldn't start its background runtime; the system may be out of threads. Close other apps and try again."
    )]
    Runtime {
        /// What tokio said.
        #[source]
        source: std::io::Error,
    },
}

struct Args {
    data: PathBuf,
    backend: live::Backend,
    /// HOME, or empty on the demo workspace with `--data` and no HOME.
    home: PathBuf,
}

impl Args {
    fn parse(
        mut args: impl Iterator<Item = String>,
        home: Result<PathBuf, std::env::VarError>,
        exe: &Path,
    ) -> Result<Self, ArgsError> {
        let mut data = None;
        let mut live_flag = false;
        let mut socket = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => {
                    data = Some(PathBuf::from(args.next().ok_or(ArgsError::Missing {
                        option: "--data",
                        example: "/tmp/humata2",
                    })?));
                }
                "--live" => live_flag = true,
                "--socket" => {
                    socket = Some(PathBuf::from(args.next().ok_or(ArgsError::Missing {
                        option: "--socket",
                        example: "/tmp/humatad.sock",
                    })?));
                }
                _ => return Err(ArgsError::Unknown { option: arg }),
            }
        }
        let (home, no_home) = match home {
            Ok(home) => (home, None),
            Err(source) => (PathBuf::new(), Some(source)),
        };
        let backend = live::backend(live_flag, socket, exe, &home);
        let needs_home = data.is_none() || backend != live::Backend::Fakes;
        if let Some(source) = no_home
            && needs_home
        {
            return Err(ArgsError::NoHome { source });
        }
        let data = data.unwrap_or_else(|| {
            let folder = match backend {
                live::Backend::Fakes => "Humata 2 (fakes)",
                live::Backend::Live { .. } => "Humata 2",
            };
            home.join("Library/Application Support").join(folder)
        });
        Ok(Self {
            data,
            backend,
            home,
        })
    }
}

/// Tells the person, once per launch, what the install left them to do
/// (D137). Runs their login shell, so it runs off the main thread.
fn tell_about_install(installed: Installed) {
    if let Some(clash) = &installed.path_clash {
        eprintln!(
            "{} already exists and isn't Humata 2's, so it was left alone and `humata` in a terminal runs it. To use Humata 2's, move it aside, then reopen Humata.",
            clash.display()
        );
    }
    let Some(hint) = installed.path_hint else {
        return;
    };
    std::thread::spawn(move || {
        let shell = std::env::var_os("SHELL").map_or_else(|| PathBuf::from("/bin/zsh"), PathBuf::from);
        // The app's own PATH is launchd's; the login shell's is the one
        // that matters.
        if live::shell_finds_humata(&shell) != Some(true) {
            eprintln!("{hint}");
        }
    });
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
        Menu::new("Help").items([
            MenuItem::action("Keyboard Shortcuts", keys::ShowShortcuts),
            MenuItem::submenu(Menu::new("Report a Bug").items([
                MenuItem::action("Start or Stop Recording", keys::ToggleRecording),
                MenuItem::action("Take Snapshot", keys::TakeSnapshot),
            ])),
        ]),
    ]);
}

fn run() -> Result<(), StartError> {
    let exe = std::env::current_exe().unwrap_or_default();
    let args = Args::parse(std::env::args().skip(1), std::env::var("HOME").map(PathBuf::from), &exe)?;
    let bundle = live::bundled_tools(&exe);
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
        let reports = args.data.join("reports");
        let storage = Arc::new(FileStorage::new(args.data));
        let (clients, _daemon) = match &args.backend {
            live::Backend::Fakes => {
                let (clients, daemon) = Clients::demo(Arc::new(SystemClock::new()), storage);
                (clients, Some(daemon))
            }
            live::Backend::Live { socket } => {
                let started = handle.block_on(live::start_live(
                    socket,
                    bundle.as_ref(),
                    &args.home,
                    storage,
                    Duration::from_secs(10),
                ));
                match started {
                    Ok((clients, installed)) => {
                        if let Some(installed) = installed {
                            tell_about_install(installed);
                        }
                        (clients, None)
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        cx.quit();
                        return;
                    }
                }
            }
        };
        let ui = Ui::new(&handle, clients, Recorder::new().saving_to(reports));
        let bounds = Bounds::centered(None, size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            ..WindowOptions::default()
        };
        let window = match humata2_ui::open(cx, &ui, options) {
            Ok((window, _)) => window,
            Err(error) => {
                eprintln!("Humata couldn't open its window: {error}. Check the system's display settings and try again.");
                cx.quit();
                return;
            }
        };
        // `humata report` reaches the running app here. Another app already
        // answering is not fatal: this one just can't take its commands.
        if let Err(error) = humata2_ui::report_view::serve_default(cx, &ui, window, &handle) {
            eprintln!("{error}");
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

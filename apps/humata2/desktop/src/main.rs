//! Humata 2's desktop app: the window, its menus, where the app keeps what
//! it saves, and which clients it runs on.
//!
//! ```text
//! humata2 [--data <folder>] [--live] [--socket <path>]
//! humata2 --version
//! ```
//!
//! From an app bundle, or with `--live` or `--socket`, the app installs the
//! host tools it carries (from a bundle), waits for This Mac's daemon and
//! runs on it; its state is kept in `~/Library/Application Support/Humata
//! 2`. Otherwise it runs on the demo workspace's fakes and keeps its state in
//! `~/Library/Application Support/Humata 2 (fakes)`. Off a Mac both live in
//! the XDG data folder instead, as `humata2` and `humata2-fakes` (D400).
//! `--data` overrides either folder; `--socket` the daemon's socket, `~/.humata2/humatad.sock`.
//! `--version` prints the bundle's version and touches nothing else, so a
//! release check can run the signed app without installing anything.

mod live;
mod starting;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input;
use gpui_kit::{
    App, Bounds, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, WindowBounds, WindowOptions,
    px, size,
};
use humata2_app::AppAction;
use humata2_clients::Clients;
use humata2_clients::model::{MachineId, Release};
use humata2_clients_live::DaemonFacts;
use humata2_engine::Resources;
use humata2_inbox::{InboxAction, OldLocalDaemon};
use humata2_install::Installed;
use humata2_ui::bridge::Ui;
use humata2_ui::keys;
use humata2_ui::record::Recorder;
use tca::{FileStorage, SystemClock};

gpui_kit::actions!(
    humata_desktop,
    [
        /// ⌘Q: quit.
        Quit,
        /// ⌘H: hide the app.
        Hide,
        /// ⌥⌘H: hide the other apps.
        HideOthers,
        /// Show the other apps again.
        ShowAll,
        /// ⌘M: minimize the window.
        Minimize,
        /// Zoom the window.
        Zoom,
        /// ⌃⌘F: the window full screen, or back.
        ToggleFullScreen
    ]
);

/// What went wrong reading the command line.
#[derive(Debug, thiserror::Error)]
enum ArgsError {
    /// An option this app doesn't know.
    #[error(
        "humata2 doesn't know the option `{option}`. Run it with no options, with `--data <folder>`, `--live` or `--socket <path>`, or with `--version` alone."
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
        // The "Humata 2 (fakes)" app (//apps/humata2/app:app_fakes) runs on
        // the made-up workspace even from a bundle.
        let backend = if option_env!("HUMATA2_BACKEND") == Some("fakes") {
            live::Backend::Fakes
        } else {
            live::backend(live_flag, socket, exe, &home)
        };
        let needs_home = data.is_none() || backend != live::Backend::Fakes;
        if let Some(source) = no_home
            && needs_home
        {
            return Err(ArgsError::NoHome { source });
        }
        let data = data.unwrap_or_else(|| {
            default_data_dir(
                &home,
                backend == live::Backend::Fakes,
                cfg!(target_os = "macos"),
                std::env::var_os("XDG_DATA_HOME").as_deref(),
            )
        });
        Ok(Self {
            data,
            backend,
            home,
        })
    }
}

/// The app's own "Migrate the daemon on This Mac" item (D374): held when
/// the local daemon predates UPGRADE (its ServerInfo names no build, so it
/// can't hold a Migrate item) and the install staged the app's release
/// beside it. A daemon that names its build posts the item itself.
fn old_local_daemon(facts: &DaemonFacts, staged: Option<&str>) -> Option<OldLocalDaemon> {
    let staged = staged.filter(|staged| !staged.is_empty())?;
    (!facts.reports_build).then(|| OldLocalDaemon {
        machine: MachineId::new(facts.machine.as_str()),
        running: facts.version.clone(),
        // The manifest names no version, only the bundle id (D370).
        staged: Release { version: String::new(), bundle: staged.to_owned() },
    })
}

/// The folder the app keeps its state in when `--data` isn't given. On
/// macOS it's in Application Support. Elsewhere it's in `$XDG_DATA_HOME`, or
/// `~/.local/share` when that is unset, empty or relative, as the XDG Base
/// Directory spec says (D400). The fakes never share the live folder.
fn default_data_dir(
    home: &Path,
    fakes: bool,
    macos: bool,
    xdg_data_home: Option<&std::ffi::OsStr>,
) -> PathBuf {
    if macos {
        let folder = if fakes { "Humata 2 (fakes)" } else { "Humata 2" };
        return home.join("Library/Application Support").join(folder);
    }
    let folder = if fakes { "humata2-fakes" } else { "humata2" };
    xdg_data_home
        .map(Path::new)
        .filter(|dir| dir.is_absolute())
        .map_or_else(|| home.join(".local/share"), Path::to_path_buf)
        .join(folder)
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
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            // A window that closed between the key and here has nothing to minimize.
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.zoom_window());
        }
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.toggle_fullscreen());
        }
    });
    cx.bind_keys([
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    cx.set_menus([
        Menu::new("Humata").items([
            MenuItem::action("Settings…", keys::OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Humata", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Humata", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Session", keys::NewSession),
            MenuItem::action("New Channel", keys::NewChannel),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette", keys::OpenPalette),
            MenuItem::action("Show or Hide Sidebar", keys::ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Home", keys::GoHome),
            MenuItem::action("Inbox", keys::GoInbox),
            MenuItem::action("Board", keys::GoBoard),
            MenuItem::separator(),
            MenuItem::action("Enter Full Screen", ToggleFullScreen),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
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

/// What `--version` prints: the version the release wrote into the app
/// bundle's Info.plist.
fn version_line(exe: &Path) -> String {
    match bundle_version(exe) {
        Some(version) => format!("Humata 2 {version}"),
        None => "Humata 2 (a development build: only an app bundle has a version)".to_owned(),
    }
}

fn bundle_version(exe: &Path) -> Option<String> {
    live::bundled_tools(exe)?;
    let contents = exe.parent()?.parent()?;
    let plist = std::fs::read_to_string(contents.join("Info.plist")).ok()?;
    let (_, after) = plist.split_once("<key>CFBundleShortVersionString</key>")?;
    let (value, _) = after.trim_start().strip_prefix("<string>")?.split_once("</string>")?;
    Some(value.trim().to_owned()).filter(|value| !value.is_empty())
}

fn run() -> Result<(), StartError> {
    let exe = std::env::current_exe().unwrap_or_default();
    // Before anything else: it must not install tools or open a window.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{}", version_line(&exe));
        return Ok(());
    }
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
        let recorder = Recorder::new().saving_to(args.data.join("reports"));
        let storage = Arc::new(FileStorage::new(args.data));
        match &args.backend {
            live::Backend::Fakes => {
                let (clients, _daemon) = Clients::demo(Arc::new(SystemClock::new()), storage);
                open_main(cx, &handle, clients, recorder, None);
            }
            live::Backend::Live { socket } => {
                // Installing the tools and waiting for the daemon can take
                // seconds, so they run on the runtime while a small window
                // shows progress; the main window opens when they're done.
                let socket = socket.clone();
                let home = args.home.clone();
                let calls = calls_into(&recorder);
                let first = live::first_phase(bundle.is_some());
                let bundle = bundle.clone();
                // The starting window reads the saved theme; the job keeps `storage`.
                let saved_ui = storage.clone();
                starting::show_while(
                    cx,
                    &handle.clone(),
                    saved_ui.as_ref(),
                    first,
                    move |progress| async move {
                        live::start_live(
                            &socket,
                            bundle.as_ref(),
                            &home,
                            storage,
                            Duration::from_secs(10),
                            Some(calls),
                            |phase| progress(phase),
                        )
                        .await
                    },
                    move |cx, (clients, installed, facts)| {
                        let old = installed
                            .as_ref()
                            .and_then(|(_, staged)| old_local_daemon(&facts, staged.as_deref()));
                        if let Some((installed, _)) = installed {
                            tell_about_install(installed);
                        }
                        open_main(cx, &handle, clients, recorder, old);
                    },
                );
            }
        }
    });
    drop(runtime);
    Ok(())
}

/// Opens the main window on `clients`, wires `humata report` to it and quits
/// the app when a window closes. On failure it says why and quits. With an
/// `old` local daemon the Inbox holds the app's own Migrate item for it.
fn open_main(
    cx: &mut App,
    handle: &tokio::runtime::Handle,
    clients: Clients,
    recorder: Recorder,
    old: Option<OldLocalDaemon>,
) {
    let ui = Ui::new(handle, clients, recorder);
    let bounds = Bounds::centered(None, size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)), cx);
    let options = main_window_options(bounds);
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
    if let Err(error) = humata2_ui::report_view::serve_default(cx, &ui, window, handle) {
        eprintln!("{error}");
    }
    cx.on_window_closed(|cx, _| cx.quit()).detach();
    if old.is_some() {
        ui.send(AppAction::Inbox(InboxAction::OldLocalDaemon(old)), cx);
    }
    cx.activate(true);
}

/// The main window at `bounds`. On macOS GPUI gives a window without
/// titlebar options neither the Resizable nor the Miniaturizable style, so it
/// couldn't be resized, zoomed or made full screen. A transparent titlebar
/// with no title keeps the look a missing one had: content runs under it and
/// the traffic lights sit where they always did.
fn main_window_options(bounds: Bounds<gpui_kit::Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: None,
        }),
        ..WindowOptions::default()
    }
}

/// The live client's calls, into the bug-report recorder while it records
/// (D159).
fn calls_into(recorder: &Recorder) -> humata2_clients_live::Observer {
    let feed = recorder.rpc_feed();
    std::sync::Arc::new(move |call: humata2_clients_live::Call| {
        feed.push(humata2_ui::record::RpcCall {
            method: call.method,
            started: call.started,
            duration: call.duration,
            status: call.status,
        });
    })
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

#[cfg(test)]
mod window_tests {
    use gpui_kit::{Bounds, point, px, size};

    use super::main_window_options;

    /// On macOS GPUI gives a window without titlebar options neither the
    /// Resizable nor the Miniaturizable style, so it can't be resized, zoomed
    /// or made full screen.
    #[test]
    fn the_main_window_can_be_resized_zoomed_and_made_full_screen() {
        let options = main_window_options(Bounds { origin: point(px(0.), px(0.)), size: size(px(1200.), px(800.)) });
        assert!(options.is_resizable && options.is_minimizable);
        let titlebar = options.titlebar.expect("titlebar options, so macOS gets the Resizable style");
        // Today's look: the content runs under a transparent titlebar, no title shown.
        assert!(titlebar.appears_transparent);
        assert!(titlebar.title.is_none());
    }
}

#[cfg(test)]
mod data_dir_tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use super::default_data_dir;

    const HOME: &str = "/home/someone";

    #[test]
    fn macos_keeps_state_in_application_support() {
        let live = default_data_dir(Path::new(HOME), false, true, None);
        let fakes = default_data_dir(Path::new(HOME), true, true, None);
        let support = Path::new("/home/someone/Library/Application Support");
        assert_eq!(live, support.join("Humata 2"));
        assert_eq!(fakes, support.join("Humata 2 (fakes)"));
    }

    #[test]
    fn elsewhere_state_follows_xdg_data_home() {
        let xdg = Some(OsStr::new("/data/xdg"));
        let live = default_data_dir(Path::new(HOME), false, false, xdg);
        assert_eq!(live, PathBuf::from("/data/xdg/humata2"));
    }

    #[test]
    fn elsewhere_without_xdg_data_home_state_is_in_local_share() {
        let live = default_data_dir(Path::new(HOME), false, false, None);
        assert_eq!(live, PathBuf::from("/home/someone/.local/share/humata2"));
    }

    #[test]
    fn a_relative_or_empty_xdg_data_home_is_ignored() {
        // The XDG spec says relative values are invalid and must be ignored.
        for value in ["relative/dir", ""] {
            let xdg = Some(OsStr::new(value));
            let live = default_data_dir(Path::new(HOME), false, false, xdg);
            assert_eq!(live, PathBuf::from("/home/someone/.local/share/humata2"));
        }
    }

    #[test]
    fn fakes_never_share_the_live_folder() {
        for macos in [true, false] {
            let live = default_data_dir(Path::new(HOME), false, macos, None);
            let fakes = default_data_dir(Path::new(HOME), true, macos, None);
            assert_ne!(live, fakes);
        }
    }
}

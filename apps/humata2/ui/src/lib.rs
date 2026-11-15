//! Humata 2's desktop UI in GPUI, drawn from the app's tca-rust store.
//!
//! [`bridge`] is the one place the store meets GPUI. The views read the
//! state, send actions, and hold nothing the store doesn't except GPUI's
//! own handles: text fields' editing state, focus, scroll offsets. Every
//! control carries a role and a label, so flow tests, recordings and screen
//! readers find it by what it says.

pub mod bridge;
pub mod keys;
pub mod record;

mod board;
mod channel;
mod chrome;
mod code_wrap;
mod hosts;
mod lists;
mod overlays;
mod pages;
mod panels;
mod session_tabs;
mod settings;
mod shell;
mod sidebar;
mod terminals;
mod theme;
mod widgets;

use std::borrow::Cow;
use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{prelude::*, *};

pub use shell::Shell;

use crate::bridge::Ui;

gpui_kit::assets::icon_assets!(
    Icons,
    [
        Search,
        Plus,
        ChevronDown,
        ChevronRight,
        ChevronLeft,
        PanelLeft,
        PanelRight,
        Terminal,
        GitBranch,
        GitFork,
        GitPullRequest,
        Files,
        File,
        Folder,
        Settings,
        Inbox,
        LayoutDashboard,
        House,
        Server,
        Check,
        X,
        ArrowUp,
        ArrowRight,
        Ellipsis,
        Circle,
        Pin,
        Sun,
        Moon,
        Maximize,
        MessageSquare,
        MessageSquareText,
        Columns2,
        Square,
        FileCode,
        FileDiff,
        FolderGit2,
        FolderTree,
        SquareKanban,
        BookOpenText,
        Info,
        Maximize2,
        Minimize2,
        ChevronsUpDown,
        PanelLeftOpen,
        Sparkles,
        FolderOpen,
        FileText,
        ShieldCheck,
        Bot,
        SquarePen,
        SquareTerminal,
        Archive,
        HardDrive,
        Bell,
        Keyboard,
        SquareTerminal,
        WifiOff,
    ]
);

/// The app's icons, for the engine's `Resources`. Text uses the
/// platform's fonts.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        Icons.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Icons.list(path)
    }
}

/// Sets the theme and binds the shortcuts. Call once, after the engine's
/// own init.
pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::from(cx.window_appearance()), None, cx);
    Theme::update(cx, |theme| {
        theme.font_size = px(13.);
        theme.mono_font_size = px(12.);
        theme.focus_ring = true;
    });
    keys::bind(cx);
}

/// The window's size when nothing was saved.
pub const WINDOW: (f32, f32) = (1440., 900.);

/// Opens the app's window on `ui` with `options`, starts the store and
/// launches the app.
///
/// # Errors
///
/// What GPUI says when the window can't open.
pub fn open(
    cx: &mut App,
    ui: &Rc<Ui>,
    options: WindowOptions,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Shell>)> {
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let ui = ui.clone();
        cx.new(|cx| Shell::new(ui, window, cx))
    })?;
    ui.start(cx);
    Ok(opened)
}

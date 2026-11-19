//! Shared GPUI primitives, theme, keybindings, widgets, and store bridge for
//! Humata 2's UI crates.

pub mod action_menu;
pub mod bridge;
pub mod chrome;
pub mod code_wrap;
pub mod full_text;
pub mod keys;
pub mod repo_search_field;
pub mod theme;
pub mod widgets;

use std::borrow::Cow;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{prelude::*, *};

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
        ArrowUpRight,
        Ellipsis,
        Circle,
        CircleX,
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

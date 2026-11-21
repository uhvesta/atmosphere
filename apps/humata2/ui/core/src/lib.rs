//! Shared GPUI primitives, theme, keybindings, widgets, and store bridge for
//! Humata 2's UI crates.

pub mod action_menu;
pub mod bridge;
pub mod chrome;
pub mod clock;
pub mod code_wrap;
pub mod full_text;
pub mod keys;
pub mod repo_search_field;
pub mod theme;
pub mod widgets;

use std::borrow::Cow;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

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
        Globe,
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

/// Syncs Humata 2's [`theme::Look`] tokens into `gpui-kit`'s global [`Theme`]
/// so kit buttons, inputs, popovers, and focus rings match the app's palette
/// in both light and dark appearances.
pub fn sync_theme(cx: &mut App) {
    let look = theme::Look::of(cx);
    let dark = look.is_dark();
    let primary = Hsla::from(rgb(look.accent));
    let (primary_hover, primary_active) = if dark {
        (
            Hsla {
                l: (primary.l + 0.04).min(1.0),
                ..primary
            },
            Hsla {
                l: (primary.l - 0.05).max(0.0),
                ..primary
            },
        )
    } else {
        (
            Hsla {
                l: (primary.l - 0.04).max(0.0),
                ..primary
            },
            Hsla {
                l: (primary.l - 0.09).max(0.0),
                ..primary
            },
        )
    };
    let (secondary, secondary_hover, secondary_active) = if dark {
        (
            Hsla::from(rgb(look.fill)),
            Hsla::from(rgb(0x3c3c40)),
            Hsla::from(rgb(0x45454a)),
        )
    } else {
        (
            Hsla::from(rgb(look.card)),
            Hsla::from(rgb(look.fill)),
            Hsla::from(rgb(look.line)),
        )
    };
    let ring = Hsla { a: 0.45, ..primary };
    Theme::update(cx, |theme| {
        theme.colors.primary = primary;
        theme.colors.primary_hover = primary_hover;
        theme.colors.primary_active = primary_active;
        theme.colors.secondary = secondary;
        theme.colors.secondary_hover = secondary_hover;
        theme.colors.secondary_active = secondary_active;
        theme.colors.border = Hsla::from(rgb(look.line));
        theme.colors.input = Hsla::from(look.control_border);
        theme.colors.popover = Hsla::from(rgb(look.card));
        theme.colors.foreground = Hsla::from(rgb(look.text));
        theme.colors.muted_foreground = Hsla::from(rgb(look.secondary));
        theme.colors.ring = ring;
    });
}

/// Sets the theme and binds the shortcuts. Call once, after the engine's
/// own init.
pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::from(cx.window_appearance()), None, cx);
    let mono_font = theme::preferred_mono_font(&cx.text_system().all_font_names());
    Theme::update(cx, |theme| {
        theme.font_size = px(13.);
        theme.mono_font_size = px(12.);
        if let Some(family) = mono_font {
            theme.mono_font_family = family;
        }
        theme.focus_ring = true;
    });
    sync_theme(cx);
    keys::bind(cx);
}

/// The window's size when nothing was saved.
pub const WINDOW: (f32, f32) = (1440., 900.);

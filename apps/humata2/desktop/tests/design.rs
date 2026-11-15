//! The design goldens: what Humata 2's UI should look like. Every screen
//! is drawn headless from made-up content, in look A (light) and look B
//! (dark), which follow the system. They're the target, not the app: when
//! you build or restyle a screen, match its frame here, and change the frame
//! here first (with the owner's say) when the design changes. See
//! `apps/humata2/OVERVIEW.md` § Design goldens and DECISIONS.md D76.
//!
//! The owner's picks (2026-11-15): the middle of a session is its embedded
//! terminal; sidebar, terminal and right panel are columns resized by
//! dragging, and the terminal or the panel can go full screen. A labelled
//! strip down the right edge picks the panel. Forks indent under their
//! parent in the sidebar. A channel opens on its Sessions tree. Home and the
//! Inbox are top-to-bottom lists, what needs you first. A session splits
//! only into its own agents' terminals, one per agent; another copy of the
//! work is a fork. New session runs any agent's TUI or a plain terminal.

#[path = "design/desk.rs"]
mod desk;
#[path = "design/board.rs"]
mod board;
#[path = "design/channel.rs"]
mod channel;
#[path = "design/kit.rs"]
mod kit;
#[path = "design/overlays.rs"]
mod overlays;
#[path = "design/pages.rs"]
mod pages;
#[path = "design/session.rs"]
mod session;
#[path = "design/sidebar.rs"]
mod sidebar;

use gpui_kit::{prelude::*, *};
use humata2_engine::Resources;
use humata2_snapshot::{Tolerance, snapshots};

use kit::{DARK, LIGHT, Look, SYSTEM_FONT, c};
use session::{Fill, Panel, Screen, Term};
use sidebar::Side;

// The mocks draw more icons than the app bundles today.
gpui_kit::assets::icon_assets!(
    MockIcons,
    [
        Archive,
        Bell,
        BookOpenText,
        Bot,
        Check,
        ChevronDown,
        ChevronLeft,
        ChevronRight,
        ChevronsUpDown,
        CircleX,
        Columns2,
        Ellipsis,
        FileCode,
        FileDiff,
        FileText,
        Folder,
        FolderGit2,
        FolderOpen,
        FolderTree,
        GitFork,
        GitPullRequest,
        HardDrive,
        House,
        Inbox,
        Info,
        Keyboard,
        Maximize2,
        MessageSquare,
        MessageSquareText,
        Minimize2,
        PanelLeft,
        PanelLeftOpen,
        PanelRight,
        Pin,
        Plus,
        Search,
        Server,
        Settings,
        ShieldCheck,
        Sparkles,
        SquareKanban,
        SquarePen,
        SquareTerminal,
        WifiOff,
        X,
    ]
);

struct Mock {
    look: Look,
    draw: fn(Look) -> Div,
}

impl Render for Mock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        (self.draw)(self.look)
            .font_family(SYSTEM_FONT)
            .text_size(px(13.))
            .text_color(c(self.look.text))
            .bg(c(self.look.content))
    }
}

fn with(screen: Screen, change: impl FnOnce(&mut Screen)) -> Screen {
    let mut screen = screen;
    change(&mut screen);
    screen
}

type Draw = fn(Look) -> Div;

const FRAMES: &[(&str, Look, Draw)] = &[
    ("01-session-working-light", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Diff), Term::Working))),
    ("02-session-working-dark", DARK, |look| session::session(look, Screen::new(Some(Panel::Diff), Term::Working))),
    ("03-session-needs-you-light", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Inbox), Term::NeedsYou))),
    ("04-session-needs-you-dark", DARK, |look| session::session(look, Screen::new(Some(Panel::Inbox), Term::NeedsYou))),
    ("05-session-exited", DARK, |look| session::session(look, Screen::new(Some(Panel::Info), Term::Exited))),
    ("06-session-offline", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Files), Term::Offline))),
    ("07-session-board-panel", DARK, |look| session::session(look, Screen::new(Some(Panel::Board), Term::Working))),
    ("08-session-journal-panel", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Journal), Term::Working))),
    ("09-session-no-panel", DARK, |look| session::session(look, Screen::new(None, Term::Working))),
    ("10-terminal-full-screen", DARK, |look| {
        session::session(look, with(Screen::new(None, Term::Working), |s| s.fill = Fill::TerminalFull))
    }),
    ("11-panel-full-screen-diff", LIGHT, |look| {
        session::session(look, with(Screen::new(Some(Panel::Diff), Term::Working), |s| s.fill = Fill::PanelFull))
    }),
    ("12-panel-full-screen-board", DARK, |look| {
        session::session(look, with(Screen::new(Some(Panel::Board), Term::Working), |s| s.fill = Fill::PanelFull))
    }),
    ("13-columns-collapsed", DARK, |look| {
        session::session(look, with(Screen::new(None, Term::Working), |s| s.fill = Fill::Collapsed))
    }),
    ("14-two-agents-one-session", DARK, session::split),
    ("15-sidebar-pinned", LIGHT, |look| {
        session::session(
            look,
            with(Screen::new(Some(Panel::Diff), Term::NeedsYou), |s| {
                s.side = Side { pinned: true, ..Side::at("Ledger schema migration") };
            }),
        )
    }),
    ("16-sidebar-fork-selected", LIGHT, |look| {
        session::session(look, with(Screen::new(Some(Panel::Diff), Term::Working), |s| s.side = Side::at("Partial BRIN on posted_at")))
    }),
    ("17-sidebar-channels-folded", DARK, |look| {
        session::session(
            look,
            with(Screen::new(Some(Panel::Diff), Term::Working), |s| {
                s.side = Side { folded: true, ..Side::at("Ledger schema migration") };
            }),
        )
    }),
    ("18-channel-sessions", LIGHT, channel::sessions),
    ("19-channel-activity", DARK, channel::activity),
    ("20-channel-repos-machines", LIGHT, channel::repos),
    ("21-board-kanban", LIGHT, board::kanban),
    ("22-board-item-open", DARK, board::drawer),
    ("23-board-table", LIGHT, board::table),
    ("24-board-dashboard", DARK, board::dashboard),
    ("25-home", LIGHT, pages::home_list),
    ("26-inbox", LIGHT, pages::inbox_split),
    ("27-inbox-dark", DARK, pages::inbox_split),
    ("28-hosts", LIGHT, pages::hosts),
    ("29-settings", DARK, pages::settings),
    ("30-palette", DARK, |look| overlays::palette(look, session::session(look, Screen::new(Some(Panel::Diff), Term::Working)))),
    ("31-new-session", LIGHT, |look| overlays::new_session(look, board::kanban(look))),
    ("32-toast", LIGHT, |look| overlays::toast(look, pages::home_list(look), "Archived “Currency codes” and freed 1.4 GB", "Undo")),
    ("33-first-run", DARK, pages::first_run),
    // Drafts for the owner's review (2026-11-15), not yet signed off.
    ("34-draft-empty-home", LIGHT, desk::empty_home),
    ("35-draft-empty-channel", DARK, desk::empty_channel),
    ("36-draft-desk", LIGHT, desk::desk),
    ("37-draft-desk-board", DARK, desk::board_session),
    ("38-draft-desk-move", DARK, desk::move_session),
    ("39-draft-desk-document", LIGHT, desk::doc_session),
    ("40-draft-new-desk-session", LIGHT, desk::new_desk),
    ("41-draft-review-beside-terminal", DARK, desk::split_review),
    ("42-draft-review-markdown-source", LIGHT, desk::source_review),
    ("43-draft-review-terminal-source-page", DARK, desk::terminal_source_page),
];

// The glob import brings in gpui's own `test` attribute, so name std's.
#[::core::prelude::v1::test]
fn design_goldens() {
    let mut cx = humata2_engine::headless(&Resources::new(MockIcons)).expect("the headless context starts");
    cx.update(humata2_ui::init);
    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    for (name, look, draw) in FRAMES {
        let (look, draw) = (*look, *draw);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(760.))))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let window: AnyWindowHandle = cx
            .update(|cx| cx.open_window(options, |_, cx| cx.new(|_| Mock { look, draw })))
            .expect("the window opens")
            .into();
        cx.run_until_parked();
        let image = cx.capture_screenshot(window).expect("the window draws");
        snapshots.assert_image(name, &image, Tolerance::default());
    }
}

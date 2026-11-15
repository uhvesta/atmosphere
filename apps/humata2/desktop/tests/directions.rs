//! N1: the macOS-native look, drawn headless as still mocks so the owner can
//! settle the direction before the real screens are restyled. Nothing here
//! is the app: every frame is a picture built from made-up content.
//!
//! Owner direction (2026-11-15): looks A (light) and B (dark) follow the
//! system. The middle of a session is its embedded terminal, not a chat. The
//! sidebar, the terminal and the right panel are columns resized by
//! dragging the handles between them, and the terminal or the right panel
//! can each go full screen. The frames show every main screen, the states a
//! session can be in, and options where the layout is still open:
//! `opt-a`/`opt-b`/`opt-c` in a name mark the alternatives.

#[path = "directions/board.rs"]
mod board;
#[path = "directions/channel.rs"]
mod channel;
#[path = "directions/kit.rs"]
mod kit;
#[path = "directions/overlays.rs"]
mod overlays;
#[path = "directions/pages.rs"]
mod pages;
#[path = "directions/session.rs"]
mod session;
#[path = "directions/sidebar.rs"]
mod sidebar;

use gpui_kit::{prelude::*, *};
use humata2_engine::Resources;
use humata2_snapshot::{Tolerance, snapshots};

use kit::{DARK, LIGHT, Look, SYSTEM_FONT, c};
use session::{Fill, Panel, Screen, Switch, Term};
use sidebar::{Side, Tree};

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
    ("03-panel-switch-opt-a-tabs", DARK, |look| session::session(look, Screen::new(Some(Panel::Inbox), Term::NeedsYou))),
    ("04-panel-switch-opt-b-strip", DARK, |look| {
        session::session(look, with(Screen::new(Some(Panel::Inbox), Term::NeedsYou), |s| s.switch = Switch::Strip))
    }),
    ("05-panel-switch-opt-c-panes", DARK, |look| {
        session::session(look, with(Screen::new(Some(Panel::Inbox), Term::NeedsYou), |s| s.switch = Switch::Panes))
    }),
    ("06-session-needs-you-light", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Inbox), Term::NeedsYou))),
    ("07-session-exited", DARK, |look| session::session(look, Screen::new(Some(Panel::Info), Term::Exited))),
    ("08-session-offline", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Files), Term::Offline))),
    ("09-session-board-panel", DARK, |look| session::session(look, Screen::new(Some(Panel::Board), Term::Working))),
    ("10-session-journal-panel", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Journal), Term::Working))),
    ("11-session-no-panel", DARK, |look| session::session(look, Screen::new(None, Term::Working))),
    ("12-terminal-full-screen", DARK, |look| {
        session::session(look, with(Screen::new(None, Term::Working), |s| s.fill = Fill::TerminalFull))
    }),
    ("13-panel-full-screen", LIGHT, |look| {
        session::session(look, with(Screen::new(Some(Panel::Diff), Term::Working), |s| s.fill = Fill::PanelFull))
    }),
    ("14-columns-collapsed", DARK, |look| {
        session::session(look, with(Screen::new(None, Term::Working), |s| s.fill = Fill::Collapsed))
    }),
    ("15-split-two-terminals", DARK, session::split),
    ("16-sidebar-pinned", LIGHT, |look| {
        session::session(
            look,
            with(Screen::new(Some(Panel::Diff), Term::NeedsYou), |s| {
                s.side = Side { pinned: true, ..Side::at("Ledger schema migration") };
            }),
        )
    }),
    ("17-tree-opt-a-indent", LIGHT, |look| {
        session::session(look, with(Screen::new(Some(Panel::Diff), Term::Working), |s| s.side = Side::at("Partial BRIN on posted_at")))
    }),
    ("18-tree-opt-b-lines", LIGHT, |look| {
        session::session(
            look,
            with(Screen::new(Some(Panel::Diff), Term::Working), |s| {
                s.side = Side { tree: Tree::Lines, ..Side::at("Partial BRIN on posted_at") };
            }),
        )
    }),
    ("19-channels-folded", DARK, |look| {
        session::session(
            look,
            with(Screen::new(Some(Panel::Diff), Term::Working), |s| {
                s.side = Side { folded: true, ..Side::at("Ledger schema migration") };
            }),
        )
    }),
    ("20-channel-opt-a-sessions", LIGHT, channel::sessions),
    ("21-channel-opt-b-overview", DARK, channel::overview),
    ("22-channel-activity", LIGHT, channel::activity),
    ("23-channel-repos-machines", DARK, channel::repos),
    ("24-board-kanban", LIGHT, board::kanban),
    ("25-board-item-open", DARK, board::drawer),
    ("26-board-opt-b-table", LIGHT, board::table),
    ("27-board-opt-c-dashboard", DARK, board::dashboard),
    ("28-home-opt-a-list", LIGHT, pages::home_list),
    ("29-home-opt-b-columns", DARK, pages::home_columns),
    ("30-inbox-opt-a-split", LIGHT, pages::inbox_split),
    ("31-inbox-opt-b-stream", DARK, pages::inbox_stream),
    ("32-hosts", LIGHT, pages::hosts),
    ("33-settings", DARK, pages::settings),
    ("34-palette", DARK, |look| overlays::palette(look, session::session(look, Screen::new(Some(Panel::Diff), Term::Working)))),
    ("35-new-session", LIGHT, |look| overlays::new_session(look, board::kanban(look))),
    ("36-toast", LIGHT, |look| overlays::toast(look, pages::home_list(look), "Archived “Currency codes” and freed 1.4 GB", "Undo")),
    ("37-first-run", DARK, pages::first_run),
];

// The glob import brings in gpui's own `test` attribute, so name std's.
#[::core::prelude::v1::test]
fn native_directions() {
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

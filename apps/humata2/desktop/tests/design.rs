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

#[path = "design/agent_asks.rs"]
mod agent_asks;
#[path = "design/desk.rs"]
mod desk;
#[path = "design/board.rs"]
mod board;
#[path = "design/channel.rs"]
mod channel;
#[path = "design/channel_tabs.rs"]
mod channel_tabs;
#[path = "design/cleanup.rs"]
mod cleanup;
#[path = "design/comments.rs"]
mod comments;
#[path = "design/kit.rs"]
mod kit;
#[path = "design/new_channel.rs"]
mod new_channel;
#[path = "design/overlays.rs"]
mod overlays;
#[path = "design/pages.rs"]
mod pages;
#[path = "design/resume.rs"]
mod resume;
#[path = "design/roam.rs"]
mod roam;
#[path = "design/session.rs"]
mod session;
#[path = "design/sidebar.rs"]
mod sidebar;
#[path = "design/archive.rs"]
mod archive;
#[path = "design/files.rs"]
mod files;
#[path = "design/fork_move.rs"]
mod fork_move;
#[path = "design/machine_page.rs"]
mod machine_page;
#[path = "design/machines.rs"]
mod machines;
#[path = "design/pr_review.rs"]
mod pr_review;
#[path = "design/views.rs"]
mod views;
#[path = "design/diff_targets.rs"]
mod diff_targets;
#[path = "design/screen_question.rs"]
mod screen_question;
#[path = "design/stop.rs"]
mod stop;
#[path = "design/upgrade.rs"]
mod upgrade;

use gpui_kit::{prelude::*, *};
use humata2_engine::Resources;
use humata2_snapshot::{Mode, Tolerance, snapshots};

use kit::{DARK, LIGHT, Look, SYSTEM_FONT, c};
use session::{Fill, Panel, Screen, Term};
use sidebar::Side;

// The mocks draw more icons than the app bundles today.
gpui_kit::assets::icon_assets!(
    MockIcons,
    [
        Archive,
        ArrowRight,
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
        GitBranch,
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
    ("27-inbox-dark", DARK, pages::inbox_all),
    ("28-hosts", LIGHT, pages::hosts),
    ("29-settings", DARK, pages::settings),
    ("30-palette", DARK, |look| overlays::palette(look, session::session(look, Screen::new(Some(Panel::Diff), Term::Working)))),
    ("31-new-session", LIGHT, |look| overlays::new_session(look, board::kanban(look))),
    ("32-toast", LIGHT, |look| overlays::toast(look, pages::home_list(look), "Archived “Currency codes”. Frees 1.4 GB when this closes.", "Undo")),
    // Empty states, the Desk and markdown review (owner sign-off 2026-11-15, D100).
    ("34-empty-home", LIGHT, desk::empty_home),
    ("35-empty-channel", DARK, desk::empty_channel),
    ("36-desk", LIGHT, desk::desk),
    ("37-desk-board", DARK, desk::board_session),
    ("38-desk-move", DARK, desk::move_session),
    ("39-desk-document", LIGHT, desk::doc_session),
    ("40-new-desk-session", LIGHT, desk::new_desk),
    ("41-review-beside-terminal", DARK, desk::split_review),
    ("42-review-markdown-source", LIGHT, desk::source_review),
    ("43-review-terminal-source-page", DARK, desk::terminal_source_page),
    // Drafts (minion FRAMES1, 2026-11-15): stay drafts until the owner
    // signs them off (milestone FRAMES1).
    ("44-new-channel", LIGHT, new_channel::create),
    ("45-new-channel-cloning", DARK, new_channel::cloning),
    ("46-session-couldnt-resume", LIGHT, |look| session::session(look, Screen::new(Some(Panel::Info), Term::CouldntResume))),
    ("47-home-restart-report", DARK, resume::home_after_restart),
    ("48-channel-inbox", LIGHT, channel_tabs::inbox),
    ("49-channel-agent-access", DARK, channel_tabs::access),
    ("50-diff-comment-box", LIGHT, comments::comment_box),
    ("51-comment-addressed", DARK, comments::addressed),
    ("52-comment-resolved-by-agent", LIGHT, comments::resolved),
    ("53-comment-archived-toast", DARK, comments::archived),
    ("54-info-archived", LIGHT, comments::info_archived),
    ("55-desk-clean-up", LIGHT, cleanup::clean_up),
    // Drafts (minion FRAMES2, 2026-11-15): stay drafts until the owner
    // signs them off (milestone FRAMES2).
    ("56-add-machine", LIGHT, machines::add_machine),
    ("57-add-machine-login", DARK, machines::add_machine_login),
    ("58-add-machine-unattended", LIGHT, machines::add_machine_unattended),
    ("59-machine-page", LIGHT, machine_page::machine),
    ("60-machine-storage", DARK, machine_page::storage),
    ("61-file-manager", LIGHT, files::file_manager),
    ("62-fork-dialog", LIGHT, fork_move::fork),
    ("63-move-dialog", DARK, fork_move::move_dialog),
    ("64-session-moving", DARK, fork_move::moving),
    ("65-archive-unpushed", LIGHT, archive::archive_unpushed),
    ("66-archived-session", DARK, archive::archived),
    ("67-pr-review-dialog", LIGHT, pr_review::review_dialog),
    ("68-pr-review-session", DARK, pr_review::review_session),
    ("69-pr-submit-review", LIGHT, pr_review::submit_review),
    ("70-view-tab-showing", LIGHT, views::view_tab),
    ("71-delete-channel", DARK, archive::delete_channel),
    // Drafts (STOP, 2026-11-16): stay drafts until the owner signs them off
    // (milestone STOP).
    ("72-stop-confirm", LIGHT, stop::stop_confirm),
    ("73-machine-sessions-stopped", DARK, stop::machine_stopped),
    ("74-hosts-sessions-stopped", LIGHT, stop::hosts_stopped),
    ("75-start-confirm", DARK, stop::start_confirm),
    ("76-stop-not-delivered", LIGHT, stop::stop_not_delivered),
    // Drafts (UPGRADE, 2026-11-16): stay drafts until the owner signs them off
    // (milestone UPGRADE).
    ("77-inbox-migrate", LIGHT, upgrade::inbox_migrate),
    ("78-migrate-details", DARK, upgrade::details),
    ("79-migrate-migrating", LIGHT, upgrade::migrating),
    ("79-machine-migrating", DARK, upgrade::machine_migrating),
    ("80-migrate-failed", DARK, upgrade::failed),
    ("80-machine-read-only", LIGHT, upgrade::machine_read_only),
    ("81-migrate-migrated", LIGHT, upgrade::migrated),
    ("81-machine-migrated", DARK, upgrade::machine_migrated),
    ("82-migrate-first", LIGHT, upgrade::migrate_first),
    ("83-migrate-holders", DARK, upgrade::holders),
    // Drafts (VIEWS A1, 2026-11-16): stay drafts until the owner signs them
    // off (milestone VIEWS).
    ("84-diff-per-repo-beside-terminal", LIGHT, diff_targets::beside_terminal),
    ("85-diff-per-repo-full-screen", DARK, diff_targets::full_screen),
    ("86-diff-repo-base-picker", LIGHT, diff_targets::base_picker),
    ("98-diff-no-default-branch", LIGHT, diff_targets::no_default_branch),
    // Drafts (ROAM, 2026-11-16): stay drafts until the owner signs them off (milestone ROAM).
    ("87-inbox-repo-proposal", LIGHT, roam::inbox_repo_proposal),
    ("88-inbox-desk-move-to-channel", DARK, roam::inbox_desk_move),
    ("89-desk-move-into-channel-dialog", LIGHT, roam::move_into_channel),
    ("90-move-refused-unknown-format", DARK, roam::move_refused),
    ("91-channel-desk-tab", LIGHT, roam::channel_desk_tab),
    ("92-new-channel-desk-session", LIGHT, roam::new_channel_desk),
    ("93-channel-desk-info-switches", DARK, roam::desk_info_switches),
    ("94-channel-desk-close", LIGHT, roam::desk_close),
    ("95-sidebar-channel-desk", DARK, roam::sidebar_channel_desk),
    ("97-desk-moving-copy-progress", LIGHT, roam::moving_copy_progress),
    // R2 (CV-8): no frame drew this dialog; built from frames 62 and 63.
    ("113-continue-in-channel", LIGHT, fork_move::continue_in_channel),
    // D782 (2026-11-17): each agent's "Ask in the Inbox" switch; Fable
    // accepted the native capture on 2026-11-17.
    ("117-settings-agents-ask-in-inbox", LIGHT, agent_asks::ask_in_inbox),
    ("118-machine-running-menu", LIGHT, machine_page::running_menu),
    // D781 (2026-11-17): a question on its agent's screen with its live
    // terminal under the card; Fable approved them with its fixes.
    ("114-inbox-screen-question", LIGHT, screen_question::unfocused),
    ("115-inbox-screen-question-dark", DARK, screen_question::focused),
    ("116-inbox-screen-question-no-terminal", LIGHT, screen_question::offline_screen),
    // PROGRESS: the shared progress line and run time.
    ("122-new-session-cloning", LIGHT, |look| overlays::new_session_cloning(look, board::kanban(look))),
    ("123-session-moving-unmeasured-step", LIGHT, fork_move::moving_unmeasured),
    ("124-session-working-elapsed", LIGHT, session::working_elapsed),
];

/// Frames not yet approved: drawn into the test's undeclared outputs
/// (`candidates/<name>.png`) for review, never checked against a golden.
/// Move one into [`FRAMES`] and record it once it is approved.
const CANDIDATES: &[(&str, Look, Draw)] = &[];

// The glob import brings in gpui's own `test` attribute, so name std's.
#[::core::prelude::v1::test]
fn design_goldens() {
    let mut cx = humata2_engine::headless(&Resources::new(MockIcons)).expect("the headless context starts");
    cx.update(humata2_ui::init);
    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    // Recording must keep even a one-glyph change (a badge count) that fits
    // within the verify tolerance, so the approved golden is the current mock.
    let tolerance = if snapshots.mode() == Mode::Record {
        Tolerance::EXACT
    } else {
        Tolerance::default()
    };
    let mut failed = Vec::new();
    let candidates = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR")
        .map(|folder| std::path::PathBuf::from(folder).join("candidates"));
    for (name, look, draw) in FRAMES.iter().chain(CANDIDATES) {
        let (look, draw) = (*look, *draw);
        let candidate = CANDIDATES.iter().any(|(each, _, _)| each == name);
        if candidate && candidates.is_none() {
            continue;
        }
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
        if let (true, Some(folder)) = (candidate, candidates.as_ref()) {
            std::fs::create_dir_all(folder).expect("the candidates folder is made");
            image.save(folder.join(format!("{name}.png"))).expect("the candidate is written");
            continue;
        }
        // Each failing frame still writes its .new.png, so one run shows them all.
        if let Err(error) = snapshots.check_image(name, &image, tolerance) {
            failed.push(format!("{name}: {error}"));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} design frames failed; look at each .new.png beside its golden, then record the ones the owner approves:\n\n{}",
        failed.len(),
        FRAMES.len(),
        failed.join("\n\n")
    );
}

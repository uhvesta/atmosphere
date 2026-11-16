//! KP-1: every action in the app is reachable from the palette. The list
//! of actions comes from where they are defined, not from a list kept here:
//! the keyboard bindings in ui/src/keys.rs (KP-2: each shows its shortcut),
//! every route the app can show, every Settings section and row, and every
//! palette command. Each walk below is an exhaustive match with no
//! wildcard, so a new route, section, row or command does not compile
//! until it says how the palette reaches it.
//!
//! Lives in features/app because Route is there and the palette can't see
//! it; keys.rs is read as text since the ui crate depends on this one.
use std::sync::Arc;

use humata2_app::Route;
use humata2_clients::Workspace;
use humata2_clients::fixture;
use humata2_clients::model::{ChannelId, MachineId, Scope, SessionId};
use humata2_palette::{Command, Marked, Palette, PaletteAction as A, PaletteState, PullRef, ReportStatus};
use humata2_settings::{Row, Section};
use tca::Reducer;

/// The keyboard map, as written. Its path is from tests/, where this file
/// runs once it leaves pending/.
const KEYS: &str = include_str!("../../../ui/src/keys.rs");

/// The session the palette opens over: a channel session on devbox-1 in
/// payments-rewrite, which also lists This Mac.
fn here() -> SessionId {
    SessionId::new("ledger")
}

fn workspace() -> Workspace {
    fixture::design()
}

/// A palette opened over [`here`], with a bug report saved so its entry
/// shows.
fn opened(workspace: &Arc<Workspace>) -> PaletteState {
    let mut state = PaletteState {
        workspace: workspace.clone(),
        context: Some(here()),
        report: ReportStatus {
            recording: false,
            last: true,
        },
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: true });
    state
}

/// A palette opened over the page of `channel`, with no session beneath it.
fn channel_page(workspace: &Arc<Workspace>, channel: &ChannelId) -> PaletteState {
    let mut state = PaletteState {
        workspace: workspace.clone(),
        channel: Some(channel.clone()),
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: false });
    state
}

fn query(state: &mut PaletteState, text: &str) {
    let _ = Palette.reduce(state, A::Query(text.to_owned()));
}

fn offers<'a>(state: &'a PaletteState, command: &Command) -> Option<&'a humata2_palette::Entry> {
    state.entries.iter().find(|entry| &entry.command == command)
}

/// How a command is reached.
enum Reach {
    /// It is listed, and typing its title finds it.
    Typing,
    /// Pasting this text offers it.
    Pasting(String),
    /// Running this command first asks for it inline (KP-5).
    After(Command),
    /// Opened over this channel's page, with no session beneath it.
    OnChannelPage(ChannelId),
}

/// Whether `command` is reached as `reach` says; why not, if it isn't.
fn reached(workspace: &Arc<Workspace>, command: &Command, reach: &Reach) -> Result<(), String> {
    let mut state = match reach {
        Reach::OnChannelPage(channel) => channel_page(workspace, channel),
        Reach::Typing | Reach::Pasting(_) | Reach::After(_) => opened(workspace),
    };
    match reach {
        Reach::Typing | Reach::OnChannelPage(_) => {
            let Some(title) = offers(&state, command).map(|e| e.title.clone()) else {
                return Err(format!("{command:?} is not listed in the palette"));
            };
            query(&mut state, &title);
            if offers(&state, command).is_none() {
                return Err(format!("typing “{title}” doesn't find {command:?}"));
            }
        }
        Reach::Pasting(text) => {
            query(&mut state, text);
            if offers(&state, command).is_none() {
                return Err(format!("pasting “{text}” doesn't offer {command:?}"));
            }
        }
        Reach::After(first) => {
            reached(workspace, first, &Reach::Typing)?;
            let at = state.entries.iter().position(|e| &e.command == first);
            let Some(at) = at else {
                return Err(format!("{first:?} is not listed in the palette"));
            };
            let _ = Palette.reduce(&mut state, A::Pick(at));
            if !state.open {
                return Err(format!("running {first:?} closes the palette instead of asking"));
            }
            if offers(&state, command).is_none() {
                return Err(format!("running {first:?} doesn't ask with {command:?}"));
            }
        }
    }
    Ok(())
}

fn check(failures: &mut Vec<String>, workspace: &Arc<Workspace>, command: &Command, reach: &Reach) {
    if let Err(failure) = reached(workspace, command, reach) {
        failures.push(failure);
    }
}

fn report(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "not reachable from the palette (KP-1):\n{}",
        failures.join("\n")
    );
}

// Every palette command.

struct Samples {
    channel: ChannelId,
    session: SessionId,
    machine: MachineId,
    file: (String, String),
}

fn samples(workspace: &Workspace) -> Samples {
    let file = workspace
        .session_files
        .get(&here())
        .and_then(|files| files.iter().find(|file| !file.folder))
        .map(|file| (file.repo.clone(), file.path.clone()))
        .unwrap_or_else(|| panic!("the design workspace has files in {}", here()));
    let session = workspace
        .sessions
        .iter()
        .find(|s| !s.archived && s.id != here())
        .map(|s| s.id.clone())
        .unwrap_or_else(|| panic!("the design workspace has another session"));
    Samples {
        channel: workspace
            .channels
            .first()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| panic!("the design workspace has a channel")),
        session,
        machine: MachineId::new("this-mac"),
        file,
    }
}

/// The command after `command`, walking every variant once, and how the
/// palette reaches it. No wildcard: a new variant must be added here.
#[allow(clippy::too_many_lines)]
fn next_command(at: Option<&Command>, s: &Samples) -> Option<Command> {
    Some(match at {
        None => Command::Home,
        Some(Command::Home) => Command::Inbox,
        Some(Command::Inbox) => Command::Board,
        Some(Command::Board) => Command::Hosts,
        Some(Command::Hosts) => Command::Files,
        Some(Command::Files) => Command::Desk,
        Some(Command::Desk) => Command::NewDeskSession,
        Some(Command::NewDeskSession) => Command::Settings,
        Some(Command::Settings) => Command::NewSession,
        Some(Command::NewSession) => Command::NewChannel,
        Some(Command::NewChannel) => Command::ExpandAll,
        Some(Command::ExpandAll) => Command::CollapseAll,
        Some(Command::CollapseAll) => Command::ToggleSidebar,
        Some(Command::ToggleSidebar) => Command::Fork,
        Some(Command::Fork) => Command::Archive,
        Some(Command::Archive) => Command::Channel(s.channel.clone()),
        Some(Command::Channel(_)) => Command::Session(s.session.clone()),
        Some(Command::Session(_)) => Command::Pull(482),
        Some(Command::Pull(_)) => Command::File {
            session: here(),
            repo: s.file.0.clone(),
            path: s.file.1.clone(),
        },
        Some(Command::File { .. }) => Command::TerminalFullScreen,
        Some(Command::TerminalFullScreen) => Command::Record,
        Some(Command::Record) => Command::Snapshot,
        Some(Command::Snapshot) => Command::ReportBug,
        Some(Command::ReportBug) => Command::OpenLastReport,
        Some(Command::OpenLastReport) => Command::TogglePanel,
        Some(Command::TogglePanel) => Command::Back,
        Some(Command::Back) => Command::ShowShortcuts,
        Some(Command::ShowShortcuts) => Command::Views,
        Some(Command::Views) => Command::Machine(s.machine.clone()),
        Some(Command::Machine(_)) => Command::ChannelBoard(s.channel.clone()),
        Some(Command::ChannelBoard(_)) => Command::MoveSession,
        Some(Command::MoveSession) => Command::MoveTo {
            session: here(),
            machine: s.machine.clone(),
        },
        Some(Command::MoveTo { .. }) => Command::ReviewPull(pull()),
        Some(Command::ReviewPull(_)) => Command::Pin {
            thing: Marked::Session(here()),
            on: true,
        },
        Some(Command::Pin {
            thing: Marked::Session(_),
            ..
        }) => Command::Vip {
            thing: Marked::Session(here()),
            // The ledger session is VIP in the design workspace, so the
            // palette offers clearing the mark, not setting it.
            on: false,
        },
        Some(Command::Vip {
            thing: Marked::Session(_),
            ..
        }) => Command::Pin {
            thing: Marked::Channel(s.channel.clone()),
            on: true,
        },
        Some(Command::Pin {
            thing: Marked::Channel(_),
            ..
        }) => Command::Vip {
            thing: Marked::Channel(s.channel.clone()),
            on: true,
        },
        Some(Command::Vip {
            thing: Marked::Channel(_),
            ..
        }) => Command::SettingsSection(Section::General),
        Some(Command::SettingsSection(_)) => Command::SettingsRow(Row::LongLines),
        Some(Command::SettingsRow(_)) => return None,
    })
}

fn pull() -> PullRef {
    PullRef {
        owner: "acme".to_owned(),
        repo: "api".to_owned(),
        number: 482,
    }
}

/// How the palette reaches `command`. No wildcard.
fn reach(command: &Command) -> Reach {
    match command {
        Command::Pull(number) => Reach::Pasting(format!("#{number}")),
        Command::ReviewPull(pull) => Reach::Pasting(format!(
            "https://github.com/{}/{}/pull/{}",
            pull.owner, pull.repo, pull.number
        )),
        Command::MoveTo { .. } => Reach::After(Command::MoveSession),
        Command::Pin {
            thing: Marked::Channel(channel),
            ..
        }
        | Command::Vip {
            thing: Marked::Channel(channel),
            ..
        } => Reach::OnChannelPage(channel.clone()),
        Command::Pin {
            thing: Marked::Session(_),
            ..
        }
        | Command::Vip {
            thing: Marked::Session(_),
            ..
        } => Reach::Typing,
        Command::Home
        | Command::Inbox
        | Command::Board
        | Command::Hosts
        | Command::Files
        | Command::Desk
        | Command::NewDeskSession
        | Command::Settings
        | Command::NewSession
        | Command::NewChannel
        | Command::ExpandAll
        | Command::CollapseAll
        | Command::ToggleSidebar
        | Command::Fork
        | Command::Archive
        | Command::Channel(_)
        | Command::Session(_)
        | Command::File { .. }
        | Command::TerminalFullScreen
        | Command::Record
        | Command::Snapshot
        | Command::ReportBug
        | Command::OpenLastReport
        | Command::TogglePanel
        | Command::Back
        | Command::ShowShortcuts
        | Command::Views
        | Command::Machine(_)
        | Command::ChannelBoard(_)
        | Command::MoveSession
        | Command::SettingsSection(_)
        | Command::SettingsRow(_) => Reach::Typing,
    }
}

#[test]
fn every_palette_command_is_reachable() {
    let workspace = Arc::new(workspace());
    let s = samples(&workspace);
    let mut failures = Vec::new();
    let mut at = next_command(None, &s);
    while let Some(command) = at {
        check(&mut failures, &workspace, &command, &reach(&command));
        at = next_command(Some(&command), &s);
    }
    report(&failures);
}

// Every route.

/// The route after `route`, walking every variant once. No wildcard.
fn next_route(at: Option<&Route>, s: &Samples) -> Option<Route> {
    Some(match at {
        None => Route::Home,
        Some(Route::Home) => Route::Inbox,
        Some(Route::Inbox) => Route::Board(Scope::Global),
        Some(Route::Board(Scope::Global)) => Route::Board(Scope::Channel(s.channel.clone())),
        Some(Route::Board(Scope::Channel(_) | Scope::Session { .. })) => Route::Channel(s.channel.clone()),
        Some(Route::Channel(_)) => Route::Session(s.session.clone()),
        Some(Route::Session(_)) => Route::Hosts,
        Some(Route::Hosts) => Route::Files,
        Some(Route::Files) => Route::Views,
        Some(Route::Views) => Route::Desk,
        Some(Route::Desk) => Route::Settings,
        Some(Route::Settings) => Route::Machine(s.machine.clone()),
        Some(Route::Machine(_)) => return None,
    })
}

/// The palette command that shows `route`. No wildcard.
fn route_command(route: &Route) -> Command {
    match route {
        Route::Home => Command::Home,
        Route::Inbox => Command::Inbox,
        Route::Board(Scope::Global) => Command::Board,
        Route::Board(Scope::Channel(id) | Scope::Session { channel: id, .. }) => {
            Command::ChannelBoard(id.clone())
        }
        Route::Channel(id) => Command::Channel(id.clone()),
        Route::Session(id) => Command::Session(id.clone()),
        Route::Hosts => Command::Hosts,
        Route::Files => Command::Files,
        Route::Views => Command::Views,
        Route::Desk => Command::Desk,
        Route::Settings => Command::Settings,
        Route::Machine(id) => Command::Machine(id.clone()),
    }
}

#[test]
fn every_page_is_reachable() {
    let workspace = Arc::new(workspace());
    let s = samples(&workspace);
    let mut failures = Vec::new();
    let mut at = next_route(None, &s);
    while let Some(route) = at {
        let command = route_command(&route);
        if let Err(failure) = reached(&workspace, &command, &Reach::Typing) {
            failures.push(format!("{route:?}: {failure}"));
        }
        at = next_route(Some(&route), &s);
    }
    // Every channel, session and machine, not only the first.
    for channel in &workspace.channels {
        check(&mut failures, &workspace, &Command::Channel(channel.id.clone()), &Reach::Typing);
    }
    for session in workspace.sessions.iter().filter(|s| !s.archived) {
        check(&mut failures, &workspace, &Command::Session(session.id.clone()), &Reach::Typing);
    }
    for machine in &workspace.machines {
        check(&mut failures, &workspace, &Command::Machine(machine.id.clone()), &Reach::Typing);
    }
    report(&failures);
}

// Every Settings section and row.

/// The section after `section`, walking every variant once. No wildcard.
fn next_section(at: Option<Section>) -> Option<Section> {
    Some(match at {
        None => Section::General,
        Some(Section::General) => Section::Agents,
        Some(Section::Agents) => Section::Machines,
        Some(Section::Machines) => Section::Layout,
        Some(Section::Layout) => Section::Access,
        Some(Section::Access) => Section::Storage,
        Some(Section::Storage) => Section::Notifications,
        Some(Section::Notifications) => Section::Keyboard,
        Some(Section::Keyboard) => return None,
    })
}

#[test]
fn every_settings_section_and_row_is_reachable() {
    let workspace = Arc::new(workspace());
    let mut failures = Vec::new();
    let mut at = next_section(None);
    while let Some(section) = at {
        check(&mut failures, &workspace, &Command::SettingsSection(section), &Reach::Typing);
        at = next_section(Some(section));
    }
    assert!(!Row::ALL.is_empty());
    for row in Row::ALL {
        check(&mut failures, &workspace, &Command::SettingsRow(row), &Reach::Typing);
    }
    report(&failures);
}

// Every keyboard binding.

/// What a bound key does, as the palette sees it.
enum Key {
    /// It runs this command, which the palette offers too.
    Runs(Command),
    /// It is not an app action the palette could run.
    Not,
}

/// What the action `name` in keys.rs does. `None` for an action this
/// audit doesn't know yet: map it, and give it a palette entry.
fn key(name: &str) -> Option<Key> {
    Some(match name {
        // ⌘K itself, Escape, and moving focus or the selection in the
        // focused sidebar.
        "OpenPalette" | "Dismiss" | "FocusSidebar" | "Up" | "Down" | "Out" | "In" | "First"
        | "Last" | "Open" => Key::Not,
        "NewSession" => Key::Runs(Command::NewSession),
        "NewChannel" => Key::Runs(Command::NewChannel),
        "Back" => Key::Runs(Command::Back),
        "ShowShortcuts" => Key::Runs(Command::ShowShortcuts),
        "ExpandAll" => Key::Runs(Command::ExpandAll),
        "CollapseAll" => Key::Runs(Command::CollapseAll),
        "ToggleSidebar" => Key::Runs(Command::ToggleSidebar),
        "GoHome" => Key::Runs(Command::Home),
        "GoInbox" => Key::Runs(Command::Inbox),
        "GoBoard" => Key::Runs(Command::Board),
        "OpenSettings" => Key::Runs(Command::Settings),
        "TogglePanel" => Key::Runs(Command::TogglePanel),
        "ToggleRecording" => Key::Runs(Command::Record),
        "TakeSnapshot" => Key::Runs(Command::Snapshot),
        "MoveSession" => Key::Runs(Command::MoveSession),
        _ => return None,
    })
}

/// Every `KeyBinding::new("keys", Action, context)` in keys.rs, as the
/// keys and the action's name.
fn bindings() -> Vec<(String, String)> {
    KEYS.split("KeyBinding::new(")
        .skip(1)
        .filter_map(|rest| {
            // The keys are a quoted string that may hold a comma (`cmd-,`).
            let (keys, after) = rest.trim_start().strip_prefix('"')?.split_once('"')?;
            let (args, _) = after.split_once(')')?;
            let action = args.trim_start().strip_prefix(',')?.split(',').next()?.trim().to_owned();
            Some((keys.to_owned(), action))
        })
        .collect()
}

/// `shift-cmd-n` as people read it: ⇧⌘N.
fn glyphs(keys: &str) -> String {
    keys.split('-')
        .map(|part| match part {
            "ctrl" => "⌃".to_owned(),
            "alt" => "⌥".to_owned(),
            "shift" => "⇧".to_owned(),
            "cmd" => "⌘".to_owned(),
            "right" => "→".to_owned(),
            "left" => "←".to_owned(),
            "up" => "↑".to_owned(),
            "down" => "↓".to_owned(),
            "enter" => "↩".to_owned(),
            "escape" => "Esc".to_owned(),
            other => other.to_uppercase(),
        })
        .collect()
}

#[test]
fn every_key_bound_action_is_in_the_palette_with_its_shortcut() {
    let workspace = Arc::new(workspace());
    let bound = bindings();
    assert!(bound.len() > 10, "keys.rs is read: {bound:?}");
    let mut failures = Vec::new();
    for (keys, action) in &bound {
        match key(action) {
            None => failures.push(format!(
                "keys.rs binds {action} ({keys}): give it a palette entry and map it in this audit"
            )),
            Some(Key::Not) => {}
            Some(Key::Runs(command)) => {
                check(&mut failures, &workspace, &command, &Reach::Typing);
                let state = opened(&workspace);
                if let Some(entry) = offers(&state, &command) {
                    let shown = glyphs(keys);
                    if entry.hint != shown {
                        failures.push(format!(
                            "{command:?} shows “{}”, not its shortcut {shown} (KP-2)",
                            entry.hint
                        ));
                    }
                }
            }
        }
    }
    report(&failures);
}

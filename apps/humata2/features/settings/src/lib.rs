//! Settings (ST-1–9): how terminals and diffs look, the agent new sessions
//! start with, and keys kept in the keychain. Saved between launches.

mod access;
// AGENTSET (D725): Settings › Agents › Agents.
mod agents;

pub use access::{AccessAction, AccessState};
pub use agents::{AgentsAction, AgentsState};
// REPOS (ST-21): Settings › General › Repositories.
mod repositories;

pub use repositories::{RepositoriesAction, RepositoriesState};
// PROVIDERS: Settings › Machines › Providers.
mod providers;

pub use providers::{ImportDraft, ProvidersAction, ProvidersState, Secret};

use humata2_clients::machines::{
    MachineScripts, SCRIPT_RULES_PATH, SCRIPT_RULES_STARTER, SCRIPTS_FOLDER,
};
use humata2_clients::model::Agent;
use humata2_clients::{Clients, DaemonError, KeychainError};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// The app's colors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Follow the window system.
    #[default]
    System,
    /// Dark.
    Dark,
    /// Light.
    Light,
}

/// What the person set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Colors.
    pub theme: Theme,
    /// Terminal and diff text size, in points.
    pub font_size: u8,
    /// Long diff lines wrap (RV-9).
    pub wrap_diffs: bool,
    /// The agent new sessions start with.
    pub default_agent: Agent,
    /// Show pinned rows at the top of the sidebar.
    pub show_pinned: bool,
    /// Ask before Approve all when something can't be undone.
    pub confirm_destructive: bool,
    /// Show extra app alerts when an agent requests permission.
    pub permission_request_alerts: bool,
    /// A terminal tab that stays hidden for 30 seconds gives its screen
    /// memory back, and shows a fresh copy when opened again.
    pub free_hidden_terminals: bool,
    /// On a Mac keyboard, ⌥ keys send Esc and the key instead of the
    /// character the layout gives (D420).
    pub option_as_alt: bool,
    /// FAST (D787): trade small comforts for less CPU and fewer redraws.
    /// Terminal cursors stop blinking, so an idle terminal draws nothing.
    pub fast_mode: bool,
    /// At most how many Desk sessions the sidebar lists under Desk (D793).
    pub desk_rows: DeskRows,
}

/// How many Desk sessions the sidebar lists under Desk (D793).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeskRows {
    /// At most this many.
    Count(u8),
    /// Every open one.
    All,
}

/// The Desk rows' compiled-in default (D793).
const DESK_ROWS: DeskRows = DeskRows::Count(5);

/// The fewest and most Desk rows a saved count gives, so a hand-edited
/// value still makes a usable sidebar.
const DESK_ROW_COUNTS: std::ops::RangeInclusive<u8> = 1..=20;

impl Default for DeskRows {
    fn default() -> Self {
        DESK_ROWS
    }
}

impl DeskRows {
    /// The pop-up's choices, in order.
    pub const OPTIONS: [Self; 5] = [
        Self::Count(3),
        Self::Count(4),
        Self::Count(5),
        Self::Count(8),
        Self::All,
    ];

    /// The cap the sidebar applies, in range; `None` lists them all.
    #[must_use]
    pub fn limit(self) -> Option<usize> {
        match self {
            Self::Count(count) => Some(usize::from(
                count.clamp(*DESK_ROW_COUNTS.start(), *DESK_ROW_COUNTS.end()),
            )),
            Self::All => None,
        }
    }

    /// How the pop-up names it.
    #[must_use]
    pub fn label(self) -> String {
        match self.limit() {
            Some(count) => count.to_string(),
            None => "All".to_owned(),
        }
    }
}

/// Fast mode's compiled-in default: off.
const FAST_MODE: bool = false;

/// The environment variable that sets fast mode's default, so a test run
/// can have it on (`--test_env=HUMATA2_FAST_MODE=1`) or off. A saved
/// setting wins over it.
pub const FAST_MODE_VAR: &str = "HUMATA2_FAST_MODE";

/// Fast mode's default given `HUMATA2_FAST_MODE`'s value: `1`, `true` or
/// `on` turn it on, `0`, `false` or `off` turn it off, and anything else,
/// or no value, keeps the compiled-in default.
#[must_use]
pub fn fast_mode_default(var: Option<&str>) -> bool {
    match var.map(|value| value.trim().to_ascii_lowercase()).as_deref() {
        Some("1" | "true" | "on") => true,
        Some("0" | "false" | "off") => false,
        _ => FAST_MODE,
    }
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: 12,
            wrap_diffs: true,
            default_agent: Agent::Claude,
            confirm_destructive: true,
            show_pinned: true,
            permission_request_alerts: false,
            free_hidden_terminals: true,
            option_as_alt: false,
            fast_mode: fast_mode_default(std::env::var(FAST_MODE_VAR).ok().as_deref()),
            desk_rows: DESK_ROWS,
        }
    }
}

/// The smallest and largest text sizes.
pub const FONT_SIZES: std::ops::RangeInclusive<u8> = 10..=20;

/// Sections in the native settings sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Section {
    General,
    Agents,
    Machines,
    #[default]
    Layout,
    Access,
    Storage,
    Notifications,
    Keyboard,
}

impl Section {
    /// Every section, in the order the sidebar lists them.
    pub const ALL: [Self; 8] = [
        Self::General,
        Self::Agents,
        Self::Machines,
        Self::Layout,
        Self::Access,
        Self::Storage,
        Self::Notifications,
        Self::Keyboard,
    ];

    /// Its name in the sidebar.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Agents => "Agents",
            Self::Machines => "Machines",
            Self::Layout => "Layout",
            Self::Access => "Agent access",
            Self::Storage => "Storage",
            Self::Notifications => "Notifications",
            Self::Keyboard => "Keyboard",
        }
    }
}

/// A setting on the Settings page, so the palette can open at it (KP-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Row {
    /// Colors.
    Appearance,
    /// The terminal's text size.
    TerminalFont,
    /// Pinned rows at the top of the sidebar.
    SidebarShowsPinned,
    /// How many Desk sessions the sidebar lists (D793).
    DeskRowsInSidebar,
    /// Long diff lines wrap.
    LongLines,
    /// Ask before risky bulk answers.
    ConfirmBulkApprovals,
    /// The agent new sessions start with.
    DefaultAgent,
    /// Extra app alerts for permission requests.
    PermissionRequestAlerts,
    /// Hidden terminals give their screen memory back.
    FreeHiddenTerminals,
    /// ⌥ keys send Esc and the key (D420).
    OptionAsAlt,
    /// Less CPU and fewer redraws (D787).
    FastMode,
}

impl Row {
    /// Every row, in the order the page shows them.
    pub const ALL: [Self; 11] = [
        Self::Appearance,
        Self::TerminalFont,
        Self::SidebarShowsPinned,
        Self::DeskRowsInSidebar,
        Self::LongLines,
        Self::ConfirmBulkApprovals,
        Self::DefaultAgent,
        Self::PermissionRequestAlerts,
        Self::FreeHiddenTerminals,
        Self::OptionAsAlt,
        Self::FastMode,
    ];

    /// Its label on the page.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::TerminalFont => "Terminal font",
            Self::SidebarShowsPinned => "Sidebar shows Pinned",
            Self::DeskRowsInSidebar => "Desk sessions in sidebar",
            Self::LongLines => "Long lines",
            Self::ConfirmBulkApprovals => "Confirm bulk approvals",
            Self::DefaultAgent => "Default agent",

            Self::PermissionRequestAlerts => "Permission request alerts",
            Self::FreeHiddenTerminals => "Free memory of hidden terminals",
            Self::OptionAsAlt => "Option acts as Alt",
            Self::FastMode => "Fast mode",
        }
    }

    /// The section it is in.
    #[must_use]
    pub const fn section(self) -> Section {
        match self {
            Self::Appearance
            | Self::TerminalFont
            | Self::SidebarShowsPinned
            | Self::DeskRowsInSidebar
            | Self::LongLines => Section::Layout,
            Self::ConfirmBulkApprovals | Self::FreeHiddenTerminals | Self::FastMode => {
                Section::General
            }
            Self::DefaultAgent => Section::Agents,
            Self::PermissionRequestAlerts => Section::Notifications,
            Self::OptionAsAlt => Section::Keyboard,
        }
    }
}

/// The settings page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsState {
    /// The selected settings section.
    pub section: Section,
    /// What is set.
    pub preferences: Preferences,
    /// An API key being typed, before it is saved.
    pub key: String,
    /// Whether a key is saved.
    pub has_key: bool,
    /// What went wrong last.
    pub error: Option<String>,
    /// The channel Agent access tab's in-flight changes.
    pub access: AccessState,
    /// REPOS (ST-21): the Repositories section of General.
    pub repos: RepositoriesState,
    /// AGENTSET (D725): which agents Humata offers.
    pub agents: AgentsState,
    /// Whether the terminal font size list is open.
    pub font_menu: bool,
    /// Whether the Desk sessions in sidebar list is open (D793).
    pub desk_rows_menu: bool,
    /// The row the palette opened the page at.
    pub revealed: Option<Row>,

    /// Whether Codex's hooks still need trusting in its /hooks (D202).
    pub codex_needs_trust: bool,
    /// The owner's machine scripts, as the Machines section last read them (MACHINESCRIPTS).
    pub scripts: Option<MachineScripts>,
    /// Why reading the scripts, or opening the rules or folder, failed, as text.
    pub scripts_error: Option<String>,
    /// PROVIDERS: the Providers block of Machines.
    pub providers: ProvidersState,
}

/// What happens on the settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Open a settings section.
    Section(Section),
    /// Open the page at a row, in its section (the palette does this, KP-1).
    Reveal(Row),
    /// Show or hide pinned rows.
    TogglePinned,
    /// Saved preferences came back at launch.
    Restore(Preferences),
    /// Pick colors.
    SetTheme(Theme),
    /// Open or close the terminal font size list.
    FontMenu,
    /// Pick the terminal font size and close the list.
    SetFontSize(u8),
    /// Open or close the Desk sessions in sidebar list (D793).
    DeskRowsMenu,
    /// Pick how many Desk sessions the sidebar lists and close the list.
    SetDeskRows(DeskRows),
    /// Wrap diffs or not.
    ToggleWrap,
    /// Ask before risky bulk answers or not.
    ToggleConfirm,
    /// Enable or disable extra permission-request alerts.
    TogglePermissionRequestAlerts,
    /// Free the memory of terminals hidden for 30 seconds, or keep every screen.
    ToggleFreeHiddenTerminals,
    /// Make ⌥ act as Alt in terminals, or type the layout's characters.
    ToggleOptionAsAlt,
    /// Turn fast mode on or off (D787).
    ToggleFastMode,

    /// The daemon said whether Codex's hooks still need trusting.
    CodexTrustShown(Result<bool, DaemonError>),
    /// Pick the default agent.
    SetAgent(Agent),
    /// The daemon answered for the default agent.
    DefaultAgentSaved(Result<(), DaemonError>),
    /// The key box changed.
    Key(String),
    /// Save the key in the keychain.
    SaveKey,
    /// The channel Agent access tab.
    Access(AccessAction),
    /// REPOS (ST-21): Settings › General › Repositories.
    Repos(RepositoriesAction),
    /// AGENTSET (D725): Settings › Agents › Agents.
    Agents(AgentsAction),
    /// MACHINESCRIPTS: the host answered the read of the rules and script names, or couldn't.
    ScriptsShown(Result<MachineScripts, DaemonError>),
    /// Edit rules: open the rules file in the editor, making it first when it isn't there.
    EditScriptRules,
    /// Reveal scripts: show the scripts folder in Finder, making it first when it isn't there.
    RevealScripts,
    /// PROVIDERS: Settings › Machines › Providers.
    Providers(ProvidersAction),
}

/// The settings page's reducer.
#[derive(Clone)]
pub struct Settings {
    clients: Clients,
}

/// The keychain item an OpenCode key is kept in.
pub const KEY_NAME: &str = "opencode-api-key";

impl SettingsState {
    /// The built-in agents pickers list, in menu order: the enabled ones, or
    /// all of them until the daemon has said (AGENTSET). Never the shell:
    /// a picker adds Terminal itself where it offers one.
    #[must_use]
    pub fn offered_agents(&self) -> Vec<Agent> {
        self.agents.offered()
    }
}

impl Settings {
    /// A settings page that keeps keys with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }

    /// Reads the owner's machine scripts (MACHINESCRIPTS), as the Machines section opens.
    fn read_scripts(&self) -> Effect<SettingsAction> {
        let call = self.clients.machines.scripts(None);
        Effect::future(async move { SettingsAction::ScriptsShown(call.await) })
    }

    /// Reads whether Codex's hooks need trusting, then the agents' switches
    /// (with where each one asks, D782), as the Agents section opens.
    fn read_asks(&self) -> Effect<SettingsAction> {
        let trust = self.clients.daemon.clone();
        let switches = self.clients.daemon.clone();
        Effect::concat([
            Effect::future(async move {
                SettingsAction::CodexTrustShown(trust.codex_needs_trust().await)
            }),
            Effect::future(async move {
                SettingsAction::Agents(AgentsAction::Loaded(switches.agent_switches().await))
            }),
        ])
    }
}

impl Reducer for Settings {
    type State = SettingsState;
    type Action = SettingsAction;

    fn reduce(&self, state: &mut SettingsState, action: SettingsAction) -> Effect<SettingsAction> {
        if let SettingsAction::Access(action) = action {
            return access::reduce(&self.clients, &mut state.access, action)
                .map(SettingsAction::Access);
        }
        if let SettingsAction::Repos(action) = action {
            return repositories::reduce(&self.clients, &mut state.repos, action)
                .map(SettingsAction::Repos);
        }
        if let SettingsAction::Providers(action) = action {
            return providers::reduce(&self.clients, &mut state.providers, action)
                .map(SettingsAction::Providers);
        }
        if let SettingsAction::Agents(action) = action {
            return agents::reduce(&self.clients, &mut state.agents, &mut state.error, action)
                .map(SettingsAction::Agents);
        }
        let preferences = &mut state.preferences;
        match action {
            SettingsAction::Restore(restored) => {
                state.preferences = restored;
                state.has_key = self.clients.keychain.has(KEY_NAME).unwrap_or(false);
            }
            SettingsAction::Section(section) => {
                state.section = section;
                state.font_menu = false;
                state.desk_rows_menu = false;
                state.revealed = None;
                if section == Section::Agents {
                    return self.read_asks();
                }
                // SEAM (MACHINESCRIPTS): the Scripts block of Machines reads the rules,
                // and the Providers block above it the providers (PROVIDERS).
                if section == Section::Machines {
                    return Effect::merge([
                        self.read_scripts(),
                        providers::open(&self.clients).map(SettingsAction::Providers),
                    ]);
                }
            }
            SettingsAction::Reveal(row) => {
                let section = row.section();
                state.section = section;
                state.revealed = Some(row);
                state.font_menu = false;
                state.desk_rows_menu = false;
                if section == Section::Agents {
                    return self.read_asks();
                }
            }

            SettingsAction::CodexTrustShown(result) => match result {
                Ok(needs) => state.codex_needs_trust = needs,
                Err(error) => state.error = Some(error.to_string()),
            },
            SettingsAction::FontMenu => state.font_menu = !state.font_menu,
            SettingsAction::SetFontSize(size) => {
                preferences.font_size = size.clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
                state.font_menu = false;
            }
            SettingsAction::DeskRowsMenu => state.desk_rows_menu = !state.desk_rows_menu,
            SettingsAction::SetDeskRows(rows) => {
                preferences.desk_rows = rows;
                state.desk_rows_menu = false;
            }
            SettingsAction::TogglePinned => preferences.show_pinned = !preferences.show_pinned,
            SettingsAction::SetTheme(theme) => preferences.theme = theme,
            SettingsAction::ToggleWrap => preferences.wrap_diffs = !preferences.wrap_diffs,
            SettingsAction::ToggleConfirm => {
                preferences.confirm_destructive = !preferences.confirm_destructive;
            }
            SettingsAction::TogglePermissionRequestAlerts => {
                preferences.permission_request_alerts = !preferences.permission_request_alerts;
            }
            SettingsAction::ToggleFreeHiddenTerminals => {
                preferences.free_hidden_terminals = !preferences.free_hidden_terminals;
            }
            SettingsAction::ToggleOptionAsAlt => {
                preferences.option_as_alt = !preferences.option_as_alt;
            }
            SettingsAction::ToggleFastMode => preferences.fast_mode = !preferences.fast_mode,
            SettingsAction::SetAgent(agent) => {
                preferences.default_agent = agent;
                // The daemon keeps Settings' default agent too (ST-15, D770),
                // so each channel can say which agent a start takes there.
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    SettingsAction::DefaultAgentSaved(daemon.set_default_agent(agent).await)
                });
            }
            SettingsAction::DefaultAgentSaved(result) => {
                if let Err(error) = result {
                    state.error = Some(error.to_string());
                }
            }
            SettingsAction::Access(_) => {}
            SettingsAction::Repos(_) => {}
            SettingsAction::Agents(_) => {}
            SettingsAction::Providers(_) => {}
            SettingsAction::ScriptsShown(result) => match result {
                Ok(scripts) => {
                    state.scripts = Some(scripts);
                    state.scripts_error = None;
                }
                // Keep what was read: only this read failed.
                Err(error) => state.scripts_error = Some(error.to_string()),
            },
            SettingsAction::EditScriptRules => {
                state.scripts_error = self
                    .clients
                    .reveal
                    .edit(SCRIPT_RULES_PATH, SCRIPT_RULES_STARTER)
                    .err()
                    .map(|error| error.to_string());
            }
            SettingsAction::RevealScripts => {
                state.scripts_error = self
                    .clients
                    .reveal
                    .reveal_folder(SCRIPTS_FOLDER)
                    .err()
                    .map(|error| error.to_string());
            }
            SettingsAction::Key(key) => state.key = key,
            SettingsAction::SaveKey => {
                let key = std::mem::take(&mut state.key);
                if key.trim().is_empty() {
                    return Effect::none();
                }
                match self.clients.keychain.set(KEY_NAME, key.trim()) {
                    Ok(()) => {
                        state.has_key = true;
                        state.error = None;
                    }
                    Err(error @ KeychainError::Denied { .. }) => {
                        state.error = Some(error.to_string())
                    }
                }
            }
        }
        Effect::none()
    }
}

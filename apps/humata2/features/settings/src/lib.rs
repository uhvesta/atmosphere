//! Settings (ST-1–9): how terminals and diffs look, the agent new sessions
//! start with, and keys kept in the keychain. Saved between launches.

mod access;

pub use access::{AccessAction, AccessState};

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
    /// Long diff lines wrap.
    LongLines,
    /// Ask before risky bulk answers.
    ConfirmBulkApprovals,
    /// The agent new sessions start with.
    DefaultAgent,
    /// Agents ask for permission in the Inbox (D199).
    AsksInInbox,
    /// Extra app alerts for permission requests.
    PermissionRequestAlerts,
}

impl Row {
    /// Every row, in the order the page shows them.
    pub const ALL: [Self; 8] = [
        Self::Appearance,
        Self::TerminalFont,
        Self::SidebarShowsPinned,
        Self::LongLines,
        Self::ConfirmBulkApprovals,
        Self::DefaultAgent,
        Self::AsksInInbox,
        Self::PermissionRequestAlerts,
    ];

    /// Its label on the page.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::TerminalFont => "Terminal font",
            Self::SidebarShowsPinned => "Sidebar shows Pinned",
            Self::LongLines => "Long lines",
            Self::ConfirmBulkApprovals => "Confirm bulk approvals",
            Self::DefaultAgent => "Default agent",
            Self::AsksInInbox => "Ask for permission in the Inbox",
            Self::PermissionRequestAlerts => "Permission request alerts",
        }
    }

    /// The section it is in.
    #[must_use]
    pub const fn section(self) -> Section {
        match self {
            Self::Appearance | Self::TerminalFont | Self::SidebarShowsPinned | Self::LongLines => {
                Section::Layout
            }
            Self::ConfirmBulkApprovals => Section::General,
            Self::DefaultAgent | Self::AsksInInbox => Section::Agents,
            Self::PermissionRequestAlerts => Section::Notifications,
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
    /// Whether the terminal font size list is open.
    pub font_menu: bool,
    /// The row the palette opened the page at.
    pub revealed: Option<Row>,
    /// Whether agents' permission asks wait in the Inbox (D199); None until
    /// the daemon says.
    pub asks_in_inbox: Option<bool>,
    /// Whether the Inbox setting is being saved.
    pub asks_saving: bool,
    /// Whether Codex's hooks still need trusting in its /hooks (D202).
    pub codex_needs_trust: bool,
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
    /// Wrap diffs or not.
    ToggleWrap,
    /// Ask before risky bulk answers or not.
    ToggleConfirm,
    /// Enable or disable extra permission-request alerts.
    TogglePermissionRequestAlerts,
    /// Send agents' permission asks to the Inbox, or to their own prompt (D199).
    ToggleAsksInInbox,
    /// The daemon saved the Inbox setting, or refused it. `wanted` is what
    /// was asked for.
    AsksSaved {
        /// Whether asks were to go to the Inbox.
        wanted: bool,
        /// Where the daemon keeps them, or why it refused.
        result: Result<bool, DaemonError>,
    },
    /// The daemon said where asks go, or couldn't.
    AsksShown(Result<bool, DaemonError>),
    /// The daemon said whether Codex's hooks still need trusting.
    CodexTrustShown(Result<bool, DaemonError>),
    /// Pick the default agent.
    SetAgent(Agent),
    /// The key box changed.
    Key(String),
    /// Save the key in the keychain.
    SaveKey,
    /// The channel Agent access tab.
    Access(AccessAction),
}

/// The settings page's reducer.
#[derive(Clone)]
pub struct Settings {
    clients: Clients,
}

/// The keychain item an OpenCode key is kept in.
pub const KEY_NAME: &str = "opencode-api-key";

impl Settings {
    /// A settings page that keeps keys with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }

    /// Reads where the daemon sends permission asks, then whether Codex's
    /// hooks need trusting, as the Agents section opens.
    fn read_asks(&self) -> Effect<SettingsAction> {
        let asks = self.clients.daemon.clone();
        let trust = self.clients.daemon.clone();
        Effect::concat([
            Effect::future(async move { SettingsAction::AsksShown(asks.asks_in_inbox().await) }),
            Effect::future(async move {
                SettingsAction::CodexTrustShown(trust.codex_needs_trust().await)
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
        let preferences = &mut state.preferences;
        match action {
            SettingsAction::Restore(restored) => {
                state.preferences = restored;
                state.has_key = self.clients.keychain.has(KEY_NAME).unwrap_or(false);
            }
            SettingsAction::Section(section) => {
                state.section = section;
                state.font_menu = false;
                state.revealed = None;
                if section == Section::Agents {
                    return self.read_asks();
                }
            }
            SettingsAction::Reveal(row) => {
                let section = row.section();
                state.section = section;
                state.revealed = Some(row);
                state.font_menu = false;
                if section == Section::Agents {
                    return self.read_asks();
                }
            }
            SettingsAction::ToggleAsksInInbox => {
                if state.asks_saving {
                    return Effect::none();
                }
                let wanted = !state.asks_in_inbox.unwrap_or(true);
                state.asks_in_inbox = Some(wanted);
                state.asks_saving = true;
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    let result = daemon.set_asks_in_inbox(wanted).await;
                    SettingsAction::AsksSaved { wanted, result }
                });
            }
            SettingsAction::AsksSaved { wanted, result } => {
                state.asks_saving = false;
                match result {
                    Ok(inbox) => state.asks_in_inbox = Some(inbox),
                    Err(error) => {
                        state.asks_in_inbox = Some(!wanted);
                        state.error = Some(error.to_string());
                    }
                }
            }
            SettingsAction::AsksShown(result) => match result {
                Ok(inbox) => state.asks_in_inbox = Some(inbox),
                Err(error) => state.error = Some(error.to_string()),
            },
            SettingsAction::CodexTrustShown(result) => match result {
                Ok(needs) => state.codex_needs_trust = needs,
                Err(error) => state.error = Some(error.to_string()),
            },
            SettingsAction::FontMenu => state.font_menu = !state.font_menu,
            SettingsAction::SetFontSize(size) => {
                preferences.font_size = size.clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
                state.font_menu = false;
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
            SettingsAction::SetAgent(agent) => preferences.default_agent = agent,
            SettingsAction::Access(_) => {}
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

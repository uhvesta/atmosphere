//! Settings (ST-1–9): how terminals and diffs look, the agent new sessions
//! start with, and keys kept in the keychain. Saved between launches.

mod access;

pub use access::{AccessAction, AccessState};

use humata2_clients::model::Agent;
use humata2_clients::{Clients, KeychainError};
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
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: 12,
            wrap_diffs: false,
            default_agent: Agent::Claude,
            confirm_destructive: true,
            show_pinned: true,
        }
    }
}

/// The smallest and largest text sizes.
pub const FONT_SIZES: std::ops::RangeInclusive<u8> = 10..=20;

/// Sections in the native settings sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

/// What happens on the settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Open a settings section.
    Section(Section),
    /// Show or hide pinned rows.
    TogglePinned,
    /// Saved preferences came back at launch.
    Restore(Preferences),
    /// Pick colors.
    SetTheme(Theme),
    /// Make text bigger or smaller by `delta` points.
    FontSize(i8),
    /// Wrap diffs or not.
    ToggleWrap,
    /// Ask before risky bulk answers or not.
    ToggleConfirm,
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
            SettingsAction::Section(section) => state.section = section,
            SettingsAction::TogglePinned => preferences.show_pinned = !preferences.show_pinned,
            SettingsAction::SetTheme(theme) => preferences.theme = theme,
            SettingsAction::FontSize(delta) => {
                let size = preferences.font_size.saturating_add_signed(delta);
                preferences.font_size = size.clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
            }
            SettingsAction::ToggleWrap => preferences.wrap_diffs = !preferences.wrap_diffs,
            SettingsAction::ToggleConfirm => {
                preferences.confirm_destructive = !preferences.confirm_destructive;
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

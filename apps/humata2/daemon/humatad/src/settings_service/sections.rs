//! The Settings sections kept as portable config (D770): default agent,
//! terminal, retention defaults, long lines and notifications in the humata
//! home's `config/settings.toml`, skills in the state folder's
//! `skills-settings.json` that skills sync reads (SK-3, SK-6), and a
//! channel's own default agent and retention in the store. Every section
//! reads its compiled-in default until it is set.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Agents;
use humata2_store::{Store, StoreError};
use serde::{Deserialize, Serialize};
use tonic::{Code, Status};

use crate::status;

/// The store key whose revision is the config file's last save.
pub(super) const REVISION_KEY: &str = "settings.sections";
/// The holder's scrollback, shown until the terminal section is set.
const SCROLLBACK_LINES: u32 = 10_000;
/// The app's terminal font size, shown until the terminal section is set.
const FONT_SIZE: f32 = 12.0;
const FONT_SIZES: std::ops::RangeInclusive<f32> = 6.0..=72.0;
const MOST_SCROLLBACK: u32 = 1_000_000;
/// Ten years: more is a typo, not a rule.
const MOST_DAYS: u32 = 3_650;

/// Where the settings file is, for a start that reads the default agent
/// (ST-15). Set once by the Settings service.
static CONFIG: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
pub(super) enum SectionsError {
    #[error("couldn't read the settings file {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the settings file {path} isn't valid settings TOML")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("the skills settings file {path} isn't valid JSON")]
    Skills {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("couldn't save the settings file {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl SectionsError {
    pub(super) fn status(&self) -> Status {
        let message = status::chain(self);
        match self {
            Self::Read { .. } | Self::Write { .. } => status::internal(
                format!("{message}. Fix the file's permissions or free some disk space, then save again"),
                "",
            ),
            Self::Parse { path, .. } | Self::Skills { path, .. } => status::failure(
                Code::FailedPrecondition,
                status::error(
                    message,
                    format!(
                        "fix {} by hand, or delete it to go back to the defaults (that loses what it holds); nothing was saved",
                        path.display()
                    ),
                    pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "settings".to_owned(),
                        problem: pb::invalid_field::Problem::BadFormat as i32,
                    }),
                ),
            ),
        }
    }
}

/// `config/settings.toml`. A section not named keeps its default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct File {
    // Plain values before tables, as TOML writes them.
    #[serde(skip_serializing_if = "Option::is_none")]
    fusefs_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    long_lines: Option<LongLines>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_agent: Option<Agent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal: Option<Terminal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retention: Option<Retention>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notifications: Option<Notifications>,
    #[serde(skip_serializing_if = "Option::is_none")]
    todo_automation: Option<TodoAutomation>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TodoMode {
    #[default]
    Ask,
    Auto,
}

/// The To-do automation settings, in the settings file and, as JSON, in a
/// channel's own override.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
struct TodoAutomation {
    on_pr_merged: TodoMode,
    on_todo_closed_archive_sessions: TodoMode,
    auto_wake_on_ci_fail: bool,
    auto_wake_on_review_comments: bool,
    /// 0 reads as the board's default.
    idle_nudge_days: u32,
}

impl Default for TodoAutomation {
    fn default() -> Self {
        Self {
            on_pr_merged: TodoMode::Ask,
            on_todo_closed_archive_sessions: TodoMode::Ask,
            auto_wake_on_ci_fail: false,
            auto_wake_on_review_comments: false,
            idle_nudge_days: 0,
        }
    }
}

fn todo_mode_of(mode: i32) -> TodoMode {
    match pb::TodoSyncMode::try_from(mode) {
        Ok(pb::TodoSyncMode::Auto) => TodoMode::Auto,
        _ => TodoMode::Ask,
    }
}

impl From<&pb::TodoAutomationSettings> for TodoAutomation {
    fn from(settings: &pb::TodoAutomationSettings) -> Self {
        Self {
            on_pr_merged: todo_mode_of(settings.on_pr_merged),
            on_todo_closed_archive_sessions: todo_mode_of(settings.on_todo_closed_archive_sessions),
            auto_wake_on_ci_fail: settings.auto_wake_on_ci_fail,
            auto_wake_on_review_comments: settings.auto_wake_on_review_comments,
            idle_nudge_days: settings.idle_nudge_days,
        }
    }
}

impl From<TodoAutomation> for pb::TodoAutomationSettings {
    fn from(settings: TodoAutomation) -> Self {
        let mode = |mode| match mode {
            TodoMode::Ask => pb::TodoSyncMode::Ask,
            TodoMode::Auto => pb::TodoSyncMode::Auto,
        } as i32;
        Self {
            on_pr_merged: mode(settings.on_pr_merged),
            on_todo_closed_archive_sessions: mode(settings.on_todo_closed_archive_sessions),
            auto_wake_on_ci_fail: settings.auto_wake_on_ci_fail,
            auto_wake_on_review_comments: settings.auto_wake_on_review_comments,
            idle_nudge_days: settings.idle_nudge_days,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Agent {
    agent: String,
    model: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Terminal {
    font: String,
    font_size: f32,
    theme: String,
    scrollback_lines: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
struct Retention {
    archive_pr_reviews_after_days: u32,
    archive_idle_sessions_after_days: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LongLines {
    Wrap,
    Scroll,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Notifications {
    quiet_from: String,
    quiet_until: String,
    vip_breaks_quiet_hours: bool,
}

/// `skills-settings.json`, as skills sync reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Skills {
    auto_sync: bool,
    auto_off: Vec<String>,
    excludes: Vec<String>,
}

impl Default for Skills {
    fn default() -> Self {
        Self { auto_sync: true, auto_off: Vec::new(), excludes: Vec::new() }
    }
}

/// Where the sections live for one daemon.
#[derive(Debug, Clone)]
pub(super) struct Places {
    config: PathBuf,
    skills: PathBuf,
    agents: Agents,
    /// The daemon's own default agent, shown until Settings names one.
    default_agent: String,
}

impl Places {
    pub(super) fn new(state: &Path, agents: Agents, default_agent: String) -> Self {
        let home = state.parent().unwrap_or(state);
        let config = home.join("config").join("settings.toml");
        let _ = CONFIG.set(config.clone());
        Self { config, skills: state.join("skills-settings.json"), agents, default_agent }
    }

    pub(super) fn read(&self) -> Result<File, SectionsError> {
        read_file(&self.config)
    }

    fn read_skills(&self) -> Result<Skills, SectionsError> {
        match std::fs::read_to_string(&self.skills) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|source| SectionsError::Skills { path: self.skills.clone(), source }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Skills::default()),
            Err(source) => Err(SectionsError::Read { path: self.skills.clone(), source }),
        }
    }

    /// Fills `shown` with every section: the saved one, else its default. A
    /// file that can't be read shows the defaults and is logged; saving
    /// names it.
    pub(super) fn show(&self, shown: &mut pb::Settings) {
        let file = self.read().unwrap_or_else(|error| {
            crate::log::say!("humatad2: showing default settings: {}", status::chain(&error));
            File::default()
        });
        let skills = self.read_skills().unwrap_or_else(|error| {
            crate::log::say!("humatad2: showing default skills settings: {}", status::chain(&error));
            Skills::default()
        });
        let agent = file.default_agent.unwrap_or_else(|| Agent { agent: self.default_agent.clone(), model: String::new() });
        shown.default_agent = Some(default_agent_of(&agent.agent, &agent.model));
        let terminal = file.terminal.unwrap_or_default();
        shown.terminal = Some(pb::TerminalSettings {
            font: terminal.font,
            font_size: if terminal.font_size > 0.0 { terminal.font_size } else { FONT_SIZE },
            theme: terminal.theme,
            scrollback_lines: if terminal.scrollback_lines > 0 { terminal.scrollback_lines } else { SCROLLBACK_LINES },
        });
        shown.retention_defaults = Some(retention_of(file.retention.unwrap_or_default()));
        // D895: the FUSE mount is off until Settings turns it on.
        shown.fusefs_enabled = file.fusefs_enabled.unwrap_or(false);
        shown.long_lines = match file.long_lines {
            Some(LongLines::Scroll) => pb::LongLines::Scroll,
            Some(LongLines::Wrap) | None => pb::LongLines::Wrap,
        } as i32;
        shown.todo_automation = Some(file.todo_automation.unwrap_or_default().into());
        shown.skills = Some(pb::SkillsSettings {
            auto_sync: skills.auto_sync,
            opted_out: skills.auto_off.into_iter().map(|value| pb::MachineId { value }).collect(),
            exclude_patterns: skills.excludes,
        });
        let notifications = file.notifications.unwrap_or_default();
        shown.notifications = Some(pb::NotificationSettings {
            quiet_from: notifications.quiet_from,
            quiet_until: notifications.quiet_until,
            vip_breaks_quiet_hours: notifications.vip_breaks_quiet_hours,
        });
    }

    /// The retention defaults in Settings, for a channel without its own.
    pub(super) fn retention_defaults(&self) -> pb::RetentionRules {
        retention_of(self.read().ok().and_then(|file| file.retention).unwrap_or_default())
    }

    /// Checks every section `request` sets; the first bad field fails.
    pub(super) fn check(&self, request: &pb::UpdateSettingsRequest) -> Result<(), Status> {
        if let Some(agent) = &request.default_agent {
            self.check_agent(agent)?;
        }
        if let Some(terminal) = &request.terminal {
            if terminal.font_size != 0.0 && !FONT_SIZES.contains(&terminal.font_size) {
                return Err(out_of_range(
                    "terminal.font_size",
                    format!("a terminal font size of {} points can't be shown", terminal.font_size),
                    "pick a size from 6 to 72 points, or 0 for the default",
                ));
            }
            if terminal.scrollback_lines > MOST_SCROLLBACK {
                return Err(out_of_range(
                    "terminal.scrollback_lines",
                    format!("{} lines of scrollback is more than a terminal keeps", terminal.scrollback_lines),
                    "pick at most 1000000 lines, or 0 for the default",
                ));
            }
        }
        if let Some(retention) = &request.retention_defaults {
            check_retention("retention_defaults", retention)?;
        }
        if let Some(long_lines) = request.long_lines
            && !matches!(pb::LongLines::try_from(long_lines), Ok(pb::LongLines::Wrap | pb::LongLines::Scroll))
        {
            return Err(status::invalid_field(
                "long_lines",
                pb::invalid_field::Problem::UnknownValue,
                "long lines must either wrap or scroll",
                "pick Wrap or Scroll",
            ));
        }
        if let Some(skills) = &request.skills {
            if skills.opted_out.iter().any(|machine| machine.value.trim().is_empty()) {
                return Err(status::invalid_field(
                    "skills.opted_out",
                    pb::invalid_field::Problem::Missing,
                    "a machine opted out of skill sync has no name",
                    "pick the machines from the list",
                ));
            }
            if skills.exclude_patterns.iter().any(|pattern| pattern.trim().is_empty()) {
                return Err(status::invalid_field(
                    "skills.exclude_patterns",
                    pb::invalid_field::Problem::BadFormat,
                    "an empty pattern would leave out no skill",
                    "remove the empty pattern, or write one such as \"*.bak\"",
                ));
            }
        }
        if let Some(notifications) = &request.notifications {
            check_quiet_hours(notifications)?;
        }
        if let Some(todo) = &request.todo_automation {
            check_todo_automation("todo_automation", todo)?;
        }
        Ok(())
    }

    /// `agent` names a known agent.
    pub(super) fn check_agent(&self, agent: &pb::DefaultAgent) -> Result<(), Status> {
        let name = agent.agent.as_ref().map(|agent| agent.value.as_str()).unwrap_or_default();
        self.agents.get(name).map(|_| ()).map_err(|error| {
            status::invalid_field(
                "default_agent.agent",
                pb::invalid_field::Problem::UnknownValue,
                format!("no agent {name:?} is set up on this Mac: {}", status::chain(&error)),
                "pick one of the agents Settings › Agents lists",
            )
        })
    }

    /// Saves the sections `request` sets, leaving the others as they are.
    pub(super) fn save(&self, request: &pb::UpdateSettingsRequest) -> Result<(), SectionsError> {
        if let Some(skills) = &request.skills {
            let saved = Skills {
                auto_sync: skills.auto_sync,
                auto_off: skills.opted_out.iter().map(|machine| machine.value.clone()).collect(),
                excludes: skills.exclude_patterns.clone(),
            };
            let text = serde_json::to_string_pretty(&saved).unwrap_or_default();
            write_whole(&self.skills, &text)?;
        }
        if !sets_file(request) {
            return Ok(());
        }
        let mut file = self.read()?;
        if let Some(agent) = &request.default_agent {
            file.default_agent = Some(Agent {
                agent: agent.agent.as_ref().map(|agent| agent.value.clone()).unwrap_or_default(),
                model: agent.model.clone(),
            });
        }
        if let Some(terminal) = &request.terminal {
            file.terminal = Some(Terminal {
                font: terminal.font.clone(),
                font_size: terminal.font_size,
                theme: terminal.theme.clone(),
                scrollback_lines: terminal.scrollback_lines,
            });
        }
        if let Some(retention) = request.retention_defaults {
            file.retention = Some(Retention {
                archive_pr_reviews_after_days: retention.archive_pr_reviews_after_days,
                archive_idle_sessions_after_days: retention.archive_idle_sessions_after_days,
            });
        }
        if let Some(enabled) = request.fusefs_enabled {
            file.fusefs_enabled = Some(enabled);
        }
        if let Some(long_lines) = request.long_lines {
            file.long_lines = Some(match pb::LongLines::try_from(long_lines) {
                Ok(pb::LongLines::Scroll) => LongLines::Scroll,
                _ => LongLines::Wrap,
            });
        }
        if let Some(notifications) = &request.notifications {
            file.notifications = Some(Notifications {
                quiet_from: notifications.quiet_from.clone(),
                quiet_until: notifications.quiet_until.clone(),
                vip_breaks_quiet_hours: notifications.vip_breaks_quiet_hours,
            });
        }
        if let Some(todo) = &request.todo_automation {
            file.todo_automation = Some(todo.into());
        }
        let text = toml::to_string(&file).unwrap_or_default();
        write_whole(&self.config, &text)
    }
}

/// Whether `request` sets any section this module keeps.
pub(super) fn sets_any(request: &pb::UpdateSettingsRequest) -> bool {
    request.skills.is_some() || sets_file(request)
}

fn sets_file(request: &pb::UpdateSettingsRequest) -> bool {
    request.default_agent.is_some()
        || request.terminal.is_some()
        || request.retention_defaults.is_some()
        || request.long_lines.is_some()
        || request.notifications.is_some()
        || request.fusefs_enabled.is_some()
        || request.todo_automation.is_some()
}

/// Whether the FUSE mount is saved as on in the settings file under the
/// humata home `state` is in. False when unset or unreadable (D895).
pub(crate) fn fusefs_enabled_in(state: &Path) -> bool {
    let config = state.parent().unwrap_or(state).join("config").join("settings.toml");
    read_file(&config).ok().and_then(|file| file.fusefs_enabled).unwrap_or(false)
}

fn read_file(path: &Path) -> Result<File, SectionsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| SectionsError::Parse { path: path.to_owned(), source }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
        Err(source) => Err(SectionsError::Read { path: path.to_owned(), source }),
    }
}

/// Replaces `path` whole through a temporary file beside it.
fn write_whole(path: &Path, text: &str) -> Result<(), SectionsError> {
    let failed = |source| SectionsError::Write { path: path.to_owned(), source };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(failed)?;
    }
    let temporary = path.with_extension(format!("{}.new", uuid::Uuid::new_v4().simple()));
    let written = std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map_err(failed)
}

pub(super) fn default_agent_of(agent: &str, model: &str) -> pb::DefaultAgent {
    pb::DefaultAgent {
        agent: Some(pb::AgentId { value: agent.to_owned() }),
        model: model.to_owned(),
    }
}

fn retention_of(retention: Retention) -> pb::RetentionRules {
    pb::RetentionRules {
        archive_pr_reviews_after_days: retention.archive_pr_reviews_after_days,
        archive_idle_sessions_after_days: retention.archive_idle_sessions_after_days,
    }
}

fn out_of_range(field: &str, message: String, fix: &str) -> Status {
    status::invalid_field(field, pb::invalid_field::Problem::OutOfRange, message, fix)
}

pub(super) fn check_retention(section: &str, retention: &pb::RetentionRules) -> Result<(), Status> {
    for (field, days) in [
        ("archive_pr_reviews_after_days", retention.archive_pr_reviews_after_days),
        ("archive_idle_sessions_after_days", retention.archive_idle_sessions_after_days),
    ] {
        if days > MOST_DAYS {
            return Err(out_of_range(
                &format!("{section}.{field}"),
                format!("{days} days is longer than Humata keeps anything waiting"),
                "pick at most 3650 days, or 0 to keep it until you archive it",
            ));
        }
    }
    Ok(())
}

/// Both sync modes are ones Settings offers, and the nudge is a sane number of days.
pub(super) fn check_todo_automation(section: &str, todo: &pb::TodoAutomationSettings) -> Result<(), Status> {
    for (field, mode) in [
        ("on_pr_merged", todo.on_pr_merged),
        ("on_todo_closed_archive_sessions", todo.on_todo_closed_archive_sessions),
    ] {
        if pb::TodoSyncMode::try_from(mode).is_err() {
            return Err(status::invalid_field(
                &format!("{section}.{field}"),
                pb::invalid_field::Problem::UnknownValue,
                format!("{mode} isn't a way for the To-do list to follow a change"),
                "pick Ask or Auto",
            ));
        }
    }
    if todo.idle_nudge_days > MOST_DAYS {
        return Err(out_of_range(
            &format!("{section}.idle_nudge_days"),
            format!("{} days is longer than Humata keeps anything waiting", todo.idle_nudge_days),
            "pick at most 3650 days, or 0 for the default",
        ));
    }
    Ok(())
}

/// Quiet hours are both HH:MM on a 24-hour clock, or both empty.
fn check_quiet_hours(notifications: &pb::NotificationSettings) -> Result<(), Status> {
    let (from, until) = (notifications.quiet_from.as_str(), notifications.quiet_until.as_str());
    if from.is_empty() && until.is_empty() {
        return Ok(());
    }
    for (field, time, other) in [("quiet_from", from, "quiet_until"), ("quiet_until", until, "quiet_from")] {
        if time.is_empty() {
            return Err(status::invalid_field(
                &format!("notifications.{field}"),
                pb::invalid_field::Problem::Missing,
                format!("quiet hours need both ends, and {field} is empty"),
                format!("set {field} as HH:MM, or clear {other} too to turn quiet hours off"),
            ));
        }
        if !is_clock_time(time) {
            return Err(status::invalid_field(
                &format!("notifications.{field}"),
                pb::invalid_field::Problem::BadFormat,
                format!("{time:?} isn't a time of day"),
                "write it as HH:MM on a 24-hour clock, such as 22:00",
            ));
        }
    }
    Ok(())
}

fn is_clock_time(time: &str) -> bool {
    let Some((hours, minutes)) = time.split_once(':') else {
        return false;
    };
    let two_digits = |part: &str| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit());
    two_digits(hours)
        && two_digits(minutes)
        && hours.parse::<u8>().is_ok_and(|hours| hours < 24)
        && minutes.parse::<u8>().is_ok_and(|minutes| minutes < 60)
}

pub(super) fn channel_agent_key(channel: &str) -> String {
    format!("channel.{channel}.default_agent")
}

pub(super) fn channel_retention_key(channel: &str) -> String {
    format!("channel.{channel}.retention")
}

pub(super) fn channel_todo_key(channel: &str) -> String {
    format!("channel.{channel}.todo_automation")
}

/// A channel's own To-do automation, saved as JSON; empty when cleared.
pub(super) fn todo_value(todo: &pb::TodoAutomationSettings) -> String {
    serde_json::to_string(&TodoAutomation::from(todo)).unwrap_or_default()
}

pub(super) fn parse_todo(value: &str) -> Option<pb::TodoAutomationSettings> {
    serde_json::from_str::<TodoAutomation>(value).ok().map(Into::into)
}

/// A channel's own retention, saved as `pr,idle` days.
pub(super) fn parse_retention(value: &str) -> Option<pb::RetentionRules> {
    let (pr, idle) = value.split_once(',')?;
    Some(pb::RetentionRules {
        archive_pr_reviews_after_days: pr.parse().ok()?,
        archive_idle_sessions_after_days: idle.parse().ok()?,
    })
}

pub(super) fn retention_value(retention: &pb::RetentionRules) -> String {
    format!("{},{}", retention.archive_pr_reviews_after_days, retention.archive_idle_sessions_after_days)
}

/// ST-15: the agent a start that names none runs in `channel`: the
/// channel's own default agent, else the one set in Settings. None leaves
/// the channel's record to say.
pub(crate) fn default_agent_for(store: &Store, channel: &str) -> Result<Option<String>, StoreError> {
    Ok(default_agent_chain(store, channel)?.into_iter().next())
}

/// The agents a start that names none prefers in `channel`, best first: the
/// channel's own saved default, then the one set in Settings (each only when
/// set). The caller picks the first that is turned on.
pub(crate) fn default_agent_chain(store: &Store, channel: &str) -> Result<Vec<String>, StoreError> {
    let mut chain = Vec::new();
    if let Some(saved) = store.setting(&channel_agent_key(channel))?
        && !saved.value.is_empty()
    {
        chain.push(saved.value);
    }
    if let Some(agent) = CONFIG
        .get()
        .and_then(|path| read_file(path).ok())
        .and_then(|file| file.default_agent)
        .map(|agent| agent.agent)
        .filter(|agent| !agent.is_empty())
    {
        chain.push(agent);
    }
    Ok(chain)
}

/// The To-do automation settings in effect for `channel`: its own override,
/// else Settings, else the defaults (every mode Ask, no auto-wake, nudge 0).
/// Reads the settings file on each call; it is small and the board asks only
/// when an event arrives, so nothing is cached and a change applies at once.
pub fn todo_automation_for(store: &Store, channel: Option<&str>) -> pb::TodoAutomationSettings {
    if let Some(channel) = channel
        && let Ok(Some(saved)) = store.setting(&channel_todo_key(channel))
        && let Some(own) = parse_todo(&saved.value)
    {
        return own;
    }
    CONFIG
        .get()
        .and_then(|path| read_file(path).ok())
        .and_then(|file| file.todo_automation)
        .unwrap_or_default()
        .into()
}

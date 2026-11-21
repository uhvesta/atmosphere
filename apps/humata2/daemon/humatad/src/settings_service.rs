//! SettingsService (ST-13, FC-12, D196, D199, D770, D782): the branch
//! prefix and Contain agents, kept in the store; where agents ask, read
//! from and written to every kind's `asks_in_inbox` in agents.toml (D782:
//! `permissions_in_agent` is true iff every kind keeps its asks);
//! the default agent, terminal, retention defaults, long lines, skills and
//! notifications, kept as portable config (see `sections`); and a
//! channel's own prefix, default agent, retention and Contain agents. Each
//! section reads its default until it is set. Writes are the user's: agents are refused as
//! user-only, and the user must be at the Mac. Every change is told on the
//! Settings feed with the settings as they are after it.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use humata2_access::{Call, Capability, ChannelId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{Agents, BRANCH_PREFIX_KEY, default_prefix, set_default_prefix, valid_prefix};
use humata2_store::Store;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::{Gated, answer_gated_showing, blocking, gate};
use crate::config_sync::{self, ConfigSync};
use crate::events::Feed;
use crate::guard::Guard;
use crate::identity::{self, Caller};
use crate::inbox::{BoardAsk, Inbox, permission};
use crate::status;
use crate::agent_switches::OnChange;

mod gh_login;
mod sections;

pub(crate) use sections::{default_agent_chain, default_agent_for, fusefs_enabled_in, web_todo_enabled_in};
pub use sections::todo_automation_for;

#[derive(Clone)]
pub(crate) struct SettingsSvc {
    guard: Arc<Guard>,
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
    feed: Arc<Feed>,
    /// A `gh` login question is in flight.
    asking: Arc<AtomicBool>,
    /// Where the portable sections are kept.
    sections: Arc<sections::Places>,
    /// The agent kinds and their agents.toml, for where they ask (D782).
    agents: Agents,
    /// Told when that file is written, so machines get the new copy.
    agents_changed: OnChange,
    /// The sync settings and the sessions that follow them, once wired.
    config_sync: Option<Arc<ConfigSync>>,
    /// Told when Settings turns the FUSE mount on or off, after it is saved.
    fusefs_switch: Arc<dyn Fn(bool) + Send + Sync>,
    /// What the FUSE mount is doing now, shown as `fusefs_status` (D908).
    fusefs_status: Arc<dyn Fn() -> pb::FuseMountStatus + Send + Sync>,
    /// Told when Settings turns the loopback To-do web UI on or off (D905).
    web_todo_switch: Arc<dyn Fn(bool) + Send + Sync>,
    /// What the To-do web server is doing now, shown as `web_todo_status` (D905).
    web_todo_status: Arc<dyn Fn() -> pb::WebTodoStatus + Send + Sync>,
}

impl SettingsSvc {
    /// `state` is the daemon's state folder, inside the humata home;
    /// `default_agent` is shown until Settings names one.
    pub(crate) fn new(
        guard: Arc<Guard>,
        inbox: Arc<Inbox>,
        store: Arc<Store>,
        machine: String,
        feed: Arc<Feed>,
        state: &Path,
        agents: Agents,
        default_agent: String,
    ) -> Self {
        let sections = Arc::new(sections::Places::new(state, agents.clone(), default_agent));
        Self {
            guard,
            inbox,
            store,
            machine,
            feed,
            asking: Arc::default(),
            sections,
            agents,
            agents_changed: OnChange::default(),
            config_sync: None,
            fusefs_switch: Arc::new(|_| {}),
            fusefs_status: Arc::new(pb::FuseMountStatus::default),
            web_todo_switch: Arc::new(|_| {}),
            web_todo_status: Arc::new(pb::WebTodoStatus::default),
        }
    }

    /// Keeps the sync settings in `sync`, and re-composes its sessions when
    /// they change.
    pub(crate) fn with_config_sync(mut self, sync: Arc<ConfigSync>) -> Self {
        self.config_sync = Some(sync);
        self
    }

    /// Calls `switch` with the new value each time Settings saves
    /// `fusefs_enabled`.
    pub(crate) fn with_fusefs_switch(mut self, switch: Arc<dyn Fn(bool) + Send + Sync>) -> Self {
        self.fusefs_switch = switch;
        self
    }

    /// Fills `fusefs_status` from `status` each time the settings are shown.
    pub(crate) fn with_fusefs_status(mut self, status: Arc<dyn Fn() -> pb::FuseMountStatus + Send + Sync>) -> Self {
        self.fusefs_status = status;
        self
    }

    /// Calls `switch` with the new value each time Settings saves `web_todo_enabled` (D905).
    pub(crate) fn with_web_todo_switch(mut self, switch: Arc<dyn Fn(bool) + Send + Sync>) -> Self {
        self.web_todo_switch = switch;
        self
    }

    /// Fills `web_todo_status` from `status` each time the settings are shown (D905).
    pub(crate) fn with_web_todo_status(mut self, status: Arc<dyn Fn() -> pb::WebTodoStatus + Send + Sync>) -> Self {
        self.web_todo_status = status;
        self
    }

    /// Tells the Settings feed what the settings are now, from any thread.
    /// The mount state calls this when it moves on its own (D908).
    pub(crate) fn tell_in_background(&self, runtime: tokio::runtime::Handle) {
        let this = self.clone();
        runtime.spawn(async move { this.tell(None).await });
    }

    /// Tells `changed` each time the compatibility switch writes agents.toml.
    pub(crate) fn with_agents_changed(mut self, changed: OnChange) -> Self {
        self.agents_changed = changed;
        self
    }

    /// Learns the host's `gh` login in the background, once, and tells the
    /// Settings feed when the default prefix it gives changes. A start never
    /// waits on this, and a login that can't be read leaves the default as
    /// it was. It is asked again, one ask at a time, whenever the Settings
    /// page reads the settings while the prefix is the default, and when any
    /// read finds the login still unknown: no timer runs.
    pub(crate) fn follow_gh_login(&self) {
        self.refresh_gh_login();
    }

    fn refresh_gh_login(&self) {
        if self.asking.swap(true, Ordering::AcqRel) {
            return;
        }
        let this = self.clone();
        let runtime = tokio::runtime::Handle::current();
        // A thread of its own: `gh` is slow, and no timer is armed to wait on it.
        std::thread::spawn(move || {
            let asked = gh_login::login();
            // Free for the next ask before telling: a read that follows the
            // event must be able to ask again.
            this.asking.store(false, Ordering::Release);
            match asked {
                Ok(login) => {
                    if set_default_prefix(&gh_login::prefix_of(&login)) {
                        runtime.block_on(this.tell(None));
                    }
                }
                Err(error) => crate::log::say!("humatad2: {}", status::chain(&error)),
            }
        });
    }

    /// Tells the Settings feed what the settings are now.
    async fn tell(&self, by: Option<pb::Actor>) {
        match self.shown().await {
            Ok(shown) => self.feed.publish(pb::Change::Updated, pb::event::Payload::Settings(shown), by),
            Err(error) => crate::log::say!("humatad2: couldn't tell the Settings feed what changed: {}", error.message()),
        }
    }

    /// Checks `SettingsChange` access for `caller` on `target` (`AskNoAllow`:
    /// user at host is allowed immediately; agents get an Inbox card with no
    /// "Always allow"; `Never` refuses; peers are refused).
    fn gate_settings(
        &self,
        caller: &Caller,
        target: Target,
        approved: bool,
    ) -> Result<Option<Box<humata2_access::AccessRequest>>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        gate(
            &self.guard,
            caller,
            &Call::Use {
                capability: Capability::SettingsChange,
                target,
            },
            approved,
        )
    }

    async fn update_settings_of(
        &self,
        caller: &Caller,
        request: pb::UpdateSettingsRequest,
        approved: bool,
    ) -> Result<Gated<pb::Settings>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        if request.branch_prefix.is_none()
            && request.agents.is_none()
            && request.security.is_none()
            && request.sync.is_none()
            && !sections::sets_any(&request)
        {
            return self.shown().await.map(Gated::Done);
        }
        if let Some(prefix) = &request.branch_prefix
            && !valid_prefix(prefix)
        {
            return Err(bad_prefix("branch_prefix"));
        }
        // Every section is checked before any is saved or queued in the Inbox.
        self.sections.check(&request)?;
        let wanted_sync = request.sync.as_ref().map(config_sync::wanted).transpose()?;
        let current = self.shown().await?.revision.map_or(0, |revision| revision.value);
        if let Some(expected) = request.expected_revision
            && expected.value != current
        {
            return Err(changed_since("settings", "expected_revision"));
        }
        if let Some(asked) = self.gate_settings(caller, Target::Global, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let (Some(wanted), Some(sync)) = (wanted_sync, &self.config_sync) {
            // In place: running sessions follow at once, and no holder restarts.
            // A sync file the user broke is named, and nothing is saved.
            let sync = sync.clone();
            blocking(move || sync.apply(wanted)).await?.map_err(|error| error.status())?;
        }
        if sections::sets_any(&request) {
            // A file the user broke by hand is named, not overwritten.
            self.sections.read().map_err(|error| error.status())?;
            let (places, wanted) = (self.sections.clone(), request.clone());
            blocking(move || places.save(&wanted)).await?.map_err(|error| error.status())?;
        }
        let saves_sections = sections::sets_any(&request);
        // D782: the old one switch sets every agent's own switch.
        if let Some(wanted) = &request.agents {
            let (agents, in_agent) = (self.agents.clone(), wanted.permissions_in_agent);
            let written = blocking(move || permission::set_every_kind(&agents, in_agent)).await?;
            // Even a half-done write changed the file for some kinds.
            self.agents_changed.tell();
            written.map_err(|error| {
                status::failure(
                    Code::FailedPrecondition,
                    pb::Error {
                        message: status::chain(&error),
                        fix: "fix or remove ~/.humata2/config/agents.toml, then change the switch again".to_owned(),
                        retry: None,
                        kind: None,
                    },
                )
            })?;
        }
        let saves_sync = request.sync.is_some();
        let (store, prefix, saves_agents, security) =
            (self.store.clone(), request.branch_prefix, request.agents.is_some(), request.security);
        blocking(move || -> Result<(), humata2_store::StoreError> {
            // The agents' switches live in agents.toml; this key keeps the
            // revision moving when only they change.
            if saves_sections || saves_agents || saves_sync {
                store.put_setting(sections::REVISION_KEY, "", current + 1)?;
            }
            if let Some(security) = security {
                crate::contained::put_global(&store, security.contain_agents, current + 1)?;
            }
            if let Some(prefix) = prefix {
                store.put_setting(BRANCH_PREFIX_KEY, &prefix, current + 1)?;
            }
            Ok(())
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "save the settings"))?;
        // AW-8: machines get Contain agents with the next policy copy.
        self.guard.policy_changed();
        if let Some(enabled) = request.fusefs_enabled {
            let switch = self.fusefs_switch.clone();
            blocking(move || switch(enabled)).await?;
        }
        if let Some(enabled) = request.web_todo_enabled {
            let switch = self.web_todo_switch.clone();
            blocking(move || switch(enabled)).await?;
        }
        let shown = self.shown().await?;
        self.feed.publish(
            pb::Change::Updated,
            pb::event::Payload::Settings(shown.clone()),
            Some(identity::actor(caller)),
        );
        if saves_sections {
            // D770: every channel's shown default agent may have changed.
            let (feed, store, agents, by) = (self.feed.clone(), self.store.clone(), self.agents.clone(), identity::actor(caller));
            let _ = blocking(move || {
                crate::channel_service::tell_every_channel(&feed, &store, &agents, Some(&by));
            })
            .await;
        }
        Ok(Gated::Done(shown))
    }

    async fn update_channel_settings_of(
        &self,
        caller: &Caller,
        request: pb::UpdateChannelSettingsRequest,
        approved: bool,
    ) -> Result<Gated<pb::ChannelSettings>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let id = request
            .channel
            .clone()
            .map(|channel| channel.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        // ST-15: the channel's own default agent, or back to Settings when cleared.
        let agent = match (request.clear_default_agent, &request.default_agent) {
            (true, _) => Some(String::new()),
            (false, Some(agent)) => {
                self.sections.check_agent(agent)?;
                Some(agent.agent.as_ref().map(|agent| agent.value.clone()).unwrap_or_default())
            }
            (false, None) => None,
        };
        let retention = match &request.retention {
            Some(retention) => {
                sections::check_retention("retention", retention)?;
                Some(sections::retention_value(retention))
            }
            None => None,
        };
        let todo = match (request.clear_todo_automation, &request.todo_automation) {
            (true, _) => Some(String::new()),
            (false, Some(todo)) => {
                sections::check_todo_automation("todo_automation", todo)?;
                Some(sections::todo_value(todo))
            }
            (false, None) => None,
        };
        let shown = self.channel_settings(&id).await?;
        let prefix = match (request.clear_branch_prefix, request.branch_prefix) {
            (true, _) => Some(None),
            (false, Some(prefix)) => {
                if !valid_prefix(&prefix) {
                    return Err(bad_prefix("branch_prefix"));
                }
                Some(Some(prefix))
            }
            (false, None) => None,
        };
        // SEC-6: the channel's own choice, or back to Settings when cleared.
        let contain = match (request.clear_contain_agents, request.contain_agents) {
            (true, _) => Some(None),
            (false, Some(on)) => Some(Some(on)),
            (false, None) => None,
        };
        if prefix.is_none() && contain.is_none() && agent.is_none() && retention.is_none() && todo.is_none() {
            return Ok(Gated::Done(shown));
        }
        let current = shown.revision.map_or(0, |revision| revision.value);
        if let Some(expected) = request.expected_revision
            && expected.value != current
        {
            return Err(changed_since("channel's settings", "expected_revision"));
        }
        if let Some(asked) =
            self.gate_settings(caller, Target::Channel(ChannelId::new(id.clone())), approved)?
        {
            return Ok(Gated::Ask(asked));
        }
        let (store, key) = (self.store.clone(), id.clone());
        blocking(move || -> Result<(), humata2_store::StoreError> {
            let Some(mut channel) = store.channel(&key)? else {
                return Ok(());
            };
            if let Some(prefix) = prefix {
                channel.branch_prefix = prefix;
            }
            channel.revision += 1;
            if let Some(on) = contain {
                crate::contained::put_channel(&store, &key, on, channel.revision)?;
            }
            if let Some(agent) = agent {
                store.put_setting(&sections::channel_agent_key(&key), &agent, channel.revision)?;
            }
            if let Some(retention) = retention {
                store.put_setting(&sections::channel_retention_key(&key), &retention, channel.revision)?;
            }
            if let Some(todo) = todo {
                store.put_setting(&sections::channel_todo_key(&key), &todo, channel.revision)?;
            }
            store.put_channel(&channel)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "save the channel's settings"))?;
        // AW-8: machines get the channel's Contain agents with the next copy.
        self.guard.policy_changed();
        // D770: the channel's shown default agent may have changed.
        let (feed, store, agents, key, by) = (self.feed.clone(), self.store.clone(), self.agents.clone(), id.clone(), identity::actor(caller));
        let _ = blocking(move || {
            crate::channel_service::tell_channel(&feed, &store, &agents, &key, pb::Change::Updated, Some(by));
        })
        .await;
        self.channel_settings(&id).await.map(Gated::Done)
    }

    async fn shown(&self) -> Result<pb::Settings, Status> {
        let (store, agents) = (self.store.clone(), self.agents.clone());
        let (saved, in_agent, codex_needs_trust, (contain, contain_revision), sections_revision) =
            blocking(move || -> Result<_, humata2_store::StoreError> {
                Ok((
                    store.setting(BRANCH_PREFIX_KEY)?,
                    permission::every_kind_in_agent(&agents, &crate::wire_inbox::AgentGateAdapter),
                    permission::codex_trust::needs_trust(&store)?,
                    crate::contained::global(&store)?,
                    store.setting(sections::REVISION_KEY)?.map_or(0, |saved| saved.revision),
                ))
            })
        .await?
        .map_err(|error| status::store(error, &self.machine, "read the settings"))?;
        // One revision for every section: the newest of their saves.
        if saved.is_none() && default_prefix().is_empty() {
            self.refresh_gh_login();
        }
        let revision = saved
            .as_ref()
            .map_or(0, |saved| saved.revision)
            .max(contain_revision)
            .max(sections_revision);
        let mut shown = pb::Settings {
            branch_prefix_is_default: saved.is_none(),
            // The default is the host's gh login and "/", or none until it is known.
            branch_prefix: saved.map_or_else(default_prefix, |saved| saved.value),
            revision: Some(pb::Revision { value: revision }),
            agents: Some(pb::AgentSettings {
                permissions_in_agent: in_agent,
                codex_hooks_need_trust: codex_needs_trust,
            }),
            security: Some(pb::SecuritySettings { contain_agents: contain }),
            ..Default::default()
        };
        self.sections.show(&mut shown);
        shown.fusefs_status = Some((self.fusefs_status)());
        shown.web_todo_status = Some((self.web_todo_status)());
        shown.sync = Some(config_sync::shown(&self.config_sync.as_ref().map(|sync| sync.config()).unwrap_or_default()));
        Ok(shown)
    }

    async fn channel_settings(&self, id: &str) -> Result<pb::ChannelSettings, Status> {
        let (store, key) = (self.store.clone(), id.to_owned());
        let (channel, contain, agent, retention, todo) = blocking(move || -> Result<_, humata2_store::StoreError> {
            Ok((
                store.channel(&key)?,
                crate::contained::channel(&store, &key)?,
                store.setting(&sections::channel_agent_key(&key))?,
                store.setting(&sections::channel_retention_key(&key))?,
                store.setting(&sections::channel_todo_key(&key))?,
            ))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "read the channel"))?;
        let channel = channel.ok_or_else(|| no_channel(id))?;
        Ok(pb::ChannelSettings {
            channel: Some(pb::ChannelId { value: channel.id }),
            branch_prefix: channel.branch_prefix,
            revision: Some(pb::Revision { value: channel.revision }),
            contain_agents: contain,
            // Unset: the default agent in Settings.
            default_agent: agent
                .filter(|saved| !saved.value.is_empty())
                .map(|saved| sections::default_agent_of(&saved.value, "")),
            // Unset: the retention defaults in Settings.
            retention: Some(
                retention
                    .and_then(|saved| sections::parse_retention(&saved.value))
                    .unwrap_or_else(|| self.sections.retention_defaults()),
            ),
            // Unset: the To-do automation in Settings.
            todo_automation: todo.and_then(|saved| sections::parse_todo(&saved.value)),
        })
    }
}

fn no_channel(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Channel,
        id,
        format!("no channel {id} is on this machine"),
        "list the channels to see their ids",
    )
}

fn bad_prefix(field: &str) -> Status {
    status::invalid_field(
        field,
        pb::invalid_field::Problem::BadFormat,
        "that prefix can't begin a branch name: git refuses spaces, `..`, `~`, `^`, `:`, `?`, `*`, `[` and `\\`",
        "use letters, digits, `-`, `_`, `.` and `/`, such as \"me/\"",
    )
}

fn changed_since(what: &str, field: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!("the {what} changed since you opened them, so nothing was saved"),
            "read them again, merge your change, and save with the new revision",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: pb::invalid_field::Problem::OutOfRange as i32,
            }),
        ),
    )
}

const CONTAIN_AGENTS_OFF_WARNING: &str = "WARNING: Turning Contain agents off lets agents in sessions access files outside their Humata folders.";

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn format_default_agent(agent: Option<&pb::DefaultAgent>) -> String {
    match agent {
        Some(agent) => {
            let id = agent.agent.as_ref().map(|id| id.value.as_str()).unwrap_or("");
            if agent.model.is_empty() {
                id.to_owned()
            } else {
                format!("{id} ({})", agent.model)
            }
        }
        None => "inherit".to_owned(),
    }
}

fn global_settings_card(before: &pb::Settings, request: &pb::UpdateSettingsRequest) -> BoardAsk {
    let mut lines = Vec::new();
    let mut contain_turning_off = false;
    if let Some(prefix) = &request.branch_prefix {
        lines.push(format!("branch_prefix: {:?} -> {:?}", before.branch_prefix, prefix));
    }
    if let Some(agent) = &request.default_agent {
        lines.push(format!(
            "default_agent: {} -> {}",
            format_default_agent(before.default_agent.as_ref()),
            format_default_agent(Some(agent))
        ));
    }
    if let Some(security) = request.security {
        let was = before.security.is_some_and(|s| s.contain_agents);
        if was && !security.contain_agents {
            contain_turning_off = true;
        }
        lines.push(format!(
            "security.contain_agents: {} -> {}",
            on_off(was),
            on_off(security.contain_agents)
        ));
    }
    if let Some(agents) = &request.agents {
        let was = before.agents.as_ref().is_some_and(|a| a.permissions_in_agent);
        lines.push(format!(
            "agents.permissions_in_agent: {} -> {}",
            on_off(was),
            on_off(agents.permissions_in_agent)
        ));
    }
    if let Some(enabled) = request.fusefs_enabled {
        lines.push(format!(
            "fusefs_enabled: {} -> {}",
            on_off(before.fusefs_enabled),
            on_off(enabled)
        ));
    }
    if let Some(enabled) = request.web_todo_enabled {
        lines.push(format!(
            "web_todo_enabled: {} -> {}",
            on_off(before.web_todo_enabled),
            on_off(enabled)
        ));
    }
    if let Some(long_lines) = request.long_lines {
        let label = |v: i32| match pb::LongLines::try_from(v) {
            Ok(pb::LongLines::Scroll) => "scroll",
            _ => "wrap",
        };
        lines.push(format!(
            "long_lines: {} -> {}",
            label(before.long_lines),
            label(long_lines)
        ));
    }
    if let Some(terminal) = &request.terminal {
        let old = before.terminal.as_ref();
        lines.push(format!(
            "terminal: font={:?} size={} theme={:?} scrollback={} -> font={:?} size={} theme={:?} scrollback={}",
            old.map(|t| t.font.as_str()).unwrap_or(""),
            old.map(|t| t.font_size).unwrap_or(0.0),
            old.map(|t| t.theme.as_str()).unwrap_or(""),
            old.map(|t| t.scrollback_lines).unwrap_or(0),
            terminal.font,
            terminal.font_size,
            terminal.theme,
            terminal.scrollback_lines,
        ));
    }
    if let Some(retention) = request.retention_defaults {
        let old = before.retention_defaults;
        lines.push(format!(
            "retention_defaults: pr={}d idle={}d -> pr={}d idle={}d",
            old.map(|r| r.archive_pr_reviews_after_days).unwrap_or(0),
            old.map(|r| r.archive_idle_sessions_after_days).unwrap_or(0),
            retention.archive_pr_reviews_after_days,
            retention.archive_idle_sessions_after_days,
        ));
    }
    if let Some(notifications) = &request.notifications {
        let old = before.notifications.as_ref();
        lines.push(format!(
            "notifications: quiet={:?}..{:?} vip={} -> quiet={:?}..{:?} vip={}",
            old.map(|n| n.quiet_from.as_str()).unwrap_or(""),
            old.map(|n| n.quiet_until.as_str()).unwrap_or(""),
            on_off(old.is_some_and(|n| n.vip_breaks_quiet_hours)),
            notifications.quiet_from,
            notifications.quiet_until,
            on_off(notifications.vip_breaks_quiet_hours),
        ));
    }
    if let Some(skills) = &request.skills {
        let old = before.skills.as_ref();
        lines.push(format!(
            "skills.auto_sync: {} -> {}",
            on_off(old.is_none_or(|s| s.auto_sync)),
            on_off(skills.auto_sync)
        ));
    }
    if let Some(todo) = &request.todo_automation {
        lines.push(format!(
            "todo_automation: idle_nudge_days={} auto_wake_ci={} auto_wake_reviews={}",
            todo.idle_nudge_days,
            on_off(todo.auto_wake_on_ci_fail),
            on_off(todo.auto_wake_on_review_comments),
        ));
    }
    if let Some(sync) = &request.sync {
        let was = before.sync.as_ref().is_some_and(|s| s.sync_enabled);
        lines.push(format!(
            "sync.sync_enabled: {} -> {}",
            on_off(was),
            on_off(sync.sync_enabled)
        ));
    }
    if contain_turning_off {
        lines.push(CONTAIN_AGENTS_OFF_WARNING.to_owned());
    }
    let reason = if contain_turning_off {
        CONTAIN_AGENTS_OFF_WARNING.to_owned()
    } else {
        "Change global Settings".to_owned()
    };
    BoardAsk {
        preview: pb::request_preview::Preview::Command(pb::CommandPreview {
            machine: None,
            command: "Global settings".to_owned(),
            notes: lines.join("\n"),
        }),
        reason,
    }
}

fn channel_settings_card(
    channel_id: &str,
    before: &pb::ChannelSettings,
    request: &pb::UpdateChannelSettingsRequest,
) -> BoardAsk {
    let mut lines = Vec::new();
    let mut contain_turning_off = false;
    if request.clear_branch_prefix {
        lines.push(format!(
            "branch_prefix: {:?} -> inherit",
            before.branch_prefix.as_deref().unwrap_or("inherit")
        ));
    } else if let Some(prefix) = &request.branch_prefix {
        lines.push(format!(
            "branch_prefix: {:?} -> {:?}",
            before.branch_prefix.as_deref().unwrap_or("inherit"),
            prefix
        ));
    }
    if request.clear_contain_agents {
        let was = before.contain_agents.map(on_off).unwrap_or("inherit");
        lines.push(format!("contain_agents: {was} -> inherit"));
    } else if let Some(on) = request.contain_agents {
        let was = before.contain_agents.map(on_off).unwrap_or("inherit");
        if !on {
            contain_turning_off = true;
        }
        lines.push(format!("contain_agents: {was} -> {}", on_off(on)));
    }
    if request.clear_default_agent {
        lines.push(format!(
            "default_agent: {} -> inherit",
            format_default_agent(before.default_agent.as_ref())
        ));
    } else if let Some(agent) = &request.default_agent {
        lines.push(format!(
            "default_agent: {} -> {}",
            format_default_agent(before.default_agent.as_ref()),
            format_default_agent(Some(agent))
        ));
    }
    if let Some(retention) = request.retention {
        let old = before.retention;
        lines.push(format!(
            "retention: pr={}d idle={}d -> pr={}d idle={}d",
            old.map(|r| r.archive_pr_reviews_after_days).unwrap_or(0),
            old.map(|r| r.archive_idle_sessions_after_days).unwrap_or(0),
            retention.archive_pr_reviews_after_days,
            retention.archive_idle_sessions_after_days,
        ));
    }
    if request.clear_todo_automation {
        lines.push("todo_automation: custom -> inherit".to_owned());
    } else if let Some(todo) = &request.todo_automation {
        lines.push(format!(
            "todo_automation: idle_nudge_days={} auto_wake_ci={} auto_wake_reviews={}",
            todo.idle_nudge_days,
            on_off(todo.auto_wake_on_ci_fail),
            on_off(todo.auto_wake_on_review_comments),
        ));
    }
    if contain_turning_off {
        lines.push(CONTAIN_AGENTS_OFF_WARNING.to_owned());
    }
    let reason = if contain_turning_off {
        CONTAIN_AGENTS_OFF_WARNING.to_owned()
    } else {
        format!("Change channel settings for {channel_id}")
    };
    BoardAsk {
        preview: pb::request_preview::Preview::Command(pb::CommandPreview {
            machine: None,
            command: format!("Channel settings ({channel_id})"),
            notes: lines.join("\n"),
        }),
        reason,
    }
}

#[tonic::async_trait]
impl pb::settings_service_server::SettingsService for SettingsSvc {
    async fn get_settings(&self, request: Request<pb::GetSettingsRequest>) -> Result<Response<pb::Settings>, Status> {
        if matches!(identity::caller(&request)?, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let shown = self.shown().await?;
        // The host's own check facts are read when the page opens, as a
        // machine page's health is (HO-3): the default prefix follows a
        // `gh` login that changed since. One ask at a time; no timer.
        if shown.branch_prefix_is_default {
            self.refresh_gh_login();
        }
        Ok(Response::new(shown))
    }

    async fn update_settings(&self, request: Request<pb::UpdateSettingsRequest>) -> Result<Response<pb::Settings>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|k| k.value.clone());
        let first = self.update_settings_of(&caller, request.clone(), false).await;
        let card = if matches!(first, Ok(Gated::Ask(_))) {
            let before = self.shown().await?;
            Some(global_settings_card(&before, &request))
        } else {
            None
        };
        let (me, who) = (self.clone(), caller.clone());
        answer_gated_showing(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Settings",
            first,
            card,
            move || async move { me.update_settings_of(&who, request, true).await },
        )
        .await
    }

    async fn get_channel_settings(
        &self,
        request: Request<pb::GetChannelSettingsRequest>,
    ) -> Result<Response<pb::ChannelSettings>, Status> {
        if matches!(identity::caller(&request)?, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let id = request.into_inner().channel.map(|channel| channel.value).unwrap_or_default();
        self.channel_settings(&id).await.map(Response::new)
    }

    async fn update_channel_settings(
        &self,
        request: Request<pb::UpdateChannelSettingsRequest>,
    ) -> Result<Response<pb::ChannelSettings>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|k| k.value.clone());
        let first = self
            .update_channel_settings_of(&caller, request.clone(), false)
            .await;
        let card = if matches!(first, Ok(Gated::Ask(_))) {
            let id = request
                .channel
                .as_ref()
                .map(|c| c.value.as_str())
                .unwrap_or_default();
            let before = self.channel_settings(id).await?;
            Some(channel_settings_card(id, &before, &request))
        } else {
            None
        };
        let (me, who) = (self.clone(), caller.clone());
        answer_gated_showing(
            &self.inbox,
            &caller,
            key,
            "humata.v1.ChannelSettings",
            first,
            card,
            move || async move { me.update_channel_settings_of(&who, request, true).await },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use humata2_store::SessionRecord;
    use pb::inbox_service_server::InboxService;
    use pb::settings_service_server::SettingsService;

    use super::*;
    use crate::identity::UserVia;

    struct Harness {
        svc: SettingsSvc,
        inbox: Arc<Inbox>,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let store = Arc::new(Store::open(dir.path().join("store")).expect("store"));
            store
                .put_session(&SessionRecord {
                    id: "se_1".to_owned(),
                    channel: "ch_1".to_owned(),
                    machine: "m1".to_owned(),
                    title: "worker".to_owned(),
                    agent: "claude".to_owned(),
                    folder: dir.path().to_path_buf(),
                    created_unix_ms: 0,
                })
                .expect("put_session");
            let guard = Arc::new(Guard::new("m1", pb::DaemonRole::Host));
            let feed = Arc::new(Feed::default());
            let inbox = crate::wire_inbox::new_inbox(
                guard.clone(),
                store.clone(),
                "m1".to_owned(),
                feed.clone(),
            );
            let svc = SettingsSvc::new(
                guard,
                inbox.clone(),
                store,
                "m1".to_owned(),
                feed,
                dir.path(),
                Agents::builtin(),
                "claude".to_owned(),
            );
            Self {
                svc,
                inbox,
                _dir: dir,
            }
        }

        fn agent_caller() -> Caller {
            let record = SessionRecord {
                id: "se_1".to_owned(),
                channel: "ch_1".to_owned(),
                machine: "m1".to_owned(),
                title: "worker".to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::new(),
                created_unix_ms: 0,
            };
            Caller::Agent(identity::agent_caller_of(&record, "m1"))
        }

        fn user_request<T>(body: T) -> Request<T> {
            let mut req = Request::new(body);
            req.extensions_mut().insert(Caller::User(UserVia::App));
            req
        }

        fn agent_request<T>(body: T) -> Request<T> {
            let mut req = Request::new(body);
            req.extensions_mut().insert(Self::agent_caller());
            req
        }

        async fn list_pending(&self) -> Vec<pb::InboxRequest> {
            self.inbox
                .list_requests(Self::user_request(pb::ListRequestsRequest {
                    agent_inbox: true,
                    ..Default::default()
                }))
                .await
                .expect("list_requests")
                .into_inner()
                .requests
        }

        async fn answer(&self, id: pb::RequestId, approve: bool) {
            let answer = if approve {
                pb::request_answer::Answer::Approve(pb::Approve {})
            } else {
                pb::request_answer::Answer::Deny(pb::Deny {
                    note: "no".to_owned(),
                })
            };
            self.inbox
                .answer_request(Self::user_request(pb::AnswerRequestRequest {
                    id: Some(id.clone()),
                    answer: Some(pb::RequestAnswer {
                        answer: Some(answer),
                    }),
                    ..Default::default()
                }))
                .await
                .expect("answer_request");
            // Wait for the spawned deferred call to settle on Inbox's watch channel.
            let mut rx = self
                .inbox
                .wait_for_answer(Self::agent_request(pb::WaitForAnswerRequest {
                    id: Some(id),
                    wait_seconds: 5,
                }))
                .await
                .expect("wait_for_answer")
                .into_inner()
                .into_inner();
            while let Some(msg) = rx.recv().await {
                if !msg.expect("wait reply").keepalive {
                    break;
                }
            }
        }
    }

    #[tokio::test]
    async fn agent_update_settings_creates_ask_no_allow_card_and_approve_applies_while_deny_keeps_old_value() {
        let h = Harness::new();

        // Agent proposes changing branch_prefix to "bot/".
        let pending = h
            .svc
            .update_settings(Harness::agent_request(pb::UpdateSettingsRequest {
                branch_prefix: Some("bot/".to_owned()),
                ..Default::default()
            }))
            .await
            .expect_err("agent update must wait in Inbox");
        let err = status::error_of(&pending).expect("typed error");
        assert!(matches!(err.kind, Some(pb::error::Kind::ApprovalPending(_))));

        // Inbox card exists in Agent inbox, never offers Always allow, and shows diff.
        let cards = h.list_pending().await;
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert!(card.always.is_none(), "AskNoAllow must never offer Always allow");
        let Some(pb::request_preview::Preview::Command(cmd)) =
            card.preview.as_ref().and_then(|p| p.preview.as_ref())
        else {
            panic!("expected CommandPreview on settings card: {:?}", card.preview);
        };
        assert!(cmd.command.contains("Global settings"));
        assert!(cmd.notes.contains("branch_prefix") && cmd.notes.contains("bot/"));

        // Approving applies the setting.
        h.answer(card.id.clone().unwrap(), true).await;
        let after_approve = h
            .svc
            .get_settings(Harness::user_request(pb::GetSettingsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(after_approve.branch_prefix, "bot/");

        // Second proposal: agent proposes "other/", user denies -> value stays "bot/".
        let _ = h
            .svc
            .update_settings(Harness::agent_request(pb::UpdateSettingsRequest {
                branch_prefix: Some("other/".to_owned()),
                ..Default::default()
            }))
            .await
            .expect_err("second agent update must also wait in Inbox");
        let cards = h.list_pending().await;
        assert_eq!(cards.len(), 1);
        h.answer(cards[0].id.clone().unwrap(), false).await;
        let after_deny = h
            .svc
            .get_settings(Harness::user_request(pb::GetSettingsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(after_deny.branch_prefix, "bot/");
    }

    #[tokio::test]
    async fn turning_contain_agents_off_warns_clearly_on_the_inbox_card() {
        let h = Harness::new();
        // User turns contain_agents on first.
        h.svc
            .update_settings(Harness::user_request(pb::UpdateSettingsRequest {
                security: Some(pb::SecuritySettings {
                    contain_agents: true,
                }),
                ..Default::default()
            }))
            .await
            .unwrap();

        // Agent proposes turning contain_agents off.
        let _ = h
            .svc
            .update_settings(Harness::agent_request(pb::UpdateSettingsRequest {
                security: Some(pb::SecuritySettings {
                    contain_agents: false,
                }),
                ..Default::default()
            }))
            .await
            .expect_err("agent turning contain_agents off must ask in Inbox");

        let cards = h.list_pending().await;
        assert_eq!(cards.len(), 1);
        assert!(cards[0].always.is_none());
        assert!(cards[0].reason.contains("WARNING"));
        let Some(pb::request_preview::Preview::Command(cmd)) =
            cards[0].preview.as_ref().and_then(|p| p.preview.as_ref())
        else {
            panic!("expected CommandPreview");
        };
        assert!(cmd.notes.contains("contain_agents: on -> off"));
        assert!(cmd.notes.contains("WARNING"));
    }
}

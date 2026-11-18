//! SettingsService (ST-13, FC-12, D196, D199, D770): the branch prefix,
//! where agents ask for permission and Contain agents, kept in the store;
//! the default agent, terminal, retention defaults, long lines, skills and
//! notifications, kept as portable config (see `sections`); and a
//! channel's own prefix, default agent, retention and Contain agents. Each
//! section reads its default until it is set. Writes are the user's: agents are refused as
//! user-only, and the user must be at the Mac. Every change is told on the
//! Settings feed with the settings as they are after it.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use humata2_access::{Call, Capability, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{Agents, BRANCH_PREFIX_KEY, default_prefix, set_default_prefix, valid_prefix};
use humata2_store::Store;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::blocking;
use crate::events::Feed;
use crate::guard::{self, Guard};
use crate::identity::{self, Caller};
use crate::inbox::permission;
use crate::status;

mod gh_login;
mod sections;

pub(crate) use sections::default_agent_for;

#[derive(Clone)]
pub(crate) struct SettingsSvc {
    guard: Arc<Guard>,
    store: Arc<Store>,
    machine: String,
    feed: Arc<Feed>,
    /// A `gh` login question is in flight.
    asking: Arc<AtomicBool>,
    /// Where the portable sections are kept.
    sections: Arc<sections::Places>,
}

impl SettingsSvc {
    /// `state` is the daemon's state folder, inside the humata home;
    /// `default_agent` is shown until Settings names one.
    pub(crate) fn new(
        guard: Arc<Guard>,
        store: Arc<Store>,
        machine: String,
        feed: Arc<Feed>,
        state: &Path,
        agents: Agents,
        default_agent: String,
    ) -> Self {
        let sections = Arc::new(sections::Places::new(state, agents, default_agent));
        Self { guard, store, machine, feed, asking: Arc::default(), sections }
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

    /// The user, at the Mac: Settings is theirs to change.
    fn require_user(&self, caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::SettingsChange,
                target: Target::Global,
            },
        )?;
        let principal = guard::principal(caller);
        if self.guard.with_policy(|_, context| principal.present_user(context)).is_none() {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::SettingsChange),
            }));
        }
        Ok(())
    }

    async fn shown(&self) -> Result<pb::Settings, Status> {
        let store = self.store.clone();
        let (saved, (in_agent, agents_revision), codex_needs_trust, (contain, contain_revision), sections_revision) =
            blocking(move || -> Result<_, humata2_store::StoreError> {
                Ok((
                    store.setting(BRANCH_PREFIX_KEY)?,
                    permission::in_agent(&store)?,
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
            .max(agents_revision)
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
        Ok(shown)
    }

    async fn channel_settings(&self, id: &str) -> Result<pb::ChannelSettings, Status> {
        let (store, key) = (self.store.clone(), id.to_owned());
        let (channel, contain, agent, retention) = blocking(move || -> Result<_, humata2_store::StoreError> {
            Ok((
                store.channel(&key)?,
                crate::contained::channel(&store, &key)?,
                store.setting(&sections::channel_agent_key(&key))?,
                store.setting(&sections::channel_retention_key(&key))?,
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
        self.require_user(&caller)?;
        let request = request.into_inner();
        if request.branch_prefix.is_none()
            && request.agents.is_none()
            && request.security.is_none()
            && !sections::sets_any(&request)
        {
            return self.shown().await.map(Response::new);
        }
        if let Some(prefix) = &request.branch_prefix
            && !valid_prefix(prefix)
        {
            return Err(bad_prefix("branch_prefix"));
        }
        // Every section is checked before any is saved.
        self.sections.check(&request)?;
        let current = self.shown().await?.revision.map_or(0, |revision| revision.value);
        if let Some(expected) = request.expected_revision
            && expected.value != current
        {
            return Err(changed_since("settings", "expected_revision"));
        }
        if sections::sets_any(&request) {
            // A file the user broke by hand is named, not overwritten.
            self.sections.read().map_err(|error| error.status())?;
            let (places, wanted) = (self.sections.clone(), request.clone());
            blocking(move || places.save(&wanted)).await?.map_err(|error| error.status())?;
        }
        let saves_sections = sections::sets_any(&request);
        let (store, prefix, agents, security) =
            (self.store.clone(), request.branch_prefix, request.agents, request.security);
        blocking(move || -> Result<(), humata2_store::StoreError> {
            if saves_sections {
                store.put_setting(sections::REVISION_KEY, "", current + 1)?;
            }
            if let Some(security) = security {
                crate::contained::put_global(&store, security.contain_agents, current + 1)?;
            }
            if let Some(agents) = agents {
                permission::put_in_agent(&store, agents.permissions_in_agent, current + 1)?;
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
        let shown = self.shown().await?;
        self.feed.publish(
            pb::Change::Updated,
            pb::event::Payload::Settings(shown.clone()),
            Some(identity::actor(&caller)),
        );
        if saves_sections {
            // D770: every channel's shown default agent may have changed.
            let (feed, store, by) = (self.feed.clone(), self.store.clone(), identity::actor(&caller));
            let _ = blocking(move || {
                for channel in store.channels().unwrap_or_default() {
                    crate::channel_service::tell_channel(&feed, &store, &channel.id, pb::Change::Updated, Some(by.clone()));
                }
            })
            .await;
        }
        Ok(Response::new(shown))
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
        self.require_user(&caller)?;
        let request = request.into_inner();
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
        if prefix.is_none() && contain.is_none() && agent.is_none() && retention.is_none() {
            return Ok(Response::new(shown));
        }
        let current = shown.revision.map_or(0, |revision| revision.value);
        if let Some(expected) = request.expected_revision
            && expected.value != current
        {
            return Err(changed_since("channel's settings", "expected_revision"));
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
            store.put_channel(&channel)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "save the channel's settings"))?;
        // AW-8: machines get the channel's Contain agents with the next copy.
        self.guard.policy_changed();
        // D770: the channel's shown default agent may have changed.
        let (feed, store, key, by) = (self.feed.clone(), self.store.clone(), id.clone(), identity::actor(&caller));
        let _ = blocking(move || {
            crate::channel_service::tell_channel(&feed, &store, &key, pb::Change::Updated, Some(by));
        })
        .await;
        self.channel_settings(&id).await.map(Response::new)
    }
}
